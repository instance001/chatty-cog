use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};

use crate::cloud_ai::{CloudProviderAdapter, ResolvedCloudTarget, build_adapter};
use crate::llama_dyn::Llama;

#[derive(Debug, Clone)]
pub struct BookkeeperConfig {
    pub lukewarm_token_window: usize,
    pub lukewarm_max_tokens: usize,
    pub temp: f32,
    pub top_p: f32,
    pub top_k: i32,
}

impl Default for BookkeeperConfig {
    fn default() -> Self {
        Self {
            lukewarm_token_window: 1500,
            lukewarm_max_tokens: 160,
            temp: 0.2,
            top_p: 0.9,
            top_k: 40,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCategory {
    Chat,
    Module,
}

#[derive(Debug, Clone)]
pub struct MemoryEvent {
    pub ts_unix_ms: i64,
    pub kind: MemoryKind,
    pub category: EventCategory,
    pub source: String,
    pub module: Option<String>, // module_id / department
    pub event_type: Option<String>,
    pub text: String, // summary
    pub tags: Vec<String>,
    pub entities: Vec<String>,
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub score: f32,
    pub source: String,
    pub module: Option<String>,
    pub event_type: Option<String>,
    pub text: String,
    pub tags: Vec<String>,
    pub payload_json: Option<String>,
    pub ts_unix_ms: i64,
}

#[derive(Debug)]
pub enum BookkeeperCmd {
    Append(MemoryEvent),
    SummarizeModuleRundown {
        module_id: String,
        input: String,
        reply: Sender<String>,
    },
    Search {
        query: String,
        module: Option<String>,
        tag: Option<String>,
        k: usize,
        reply: Sender<Vec<MemoryHit>>,
    },
    GetLukeWarm {
        reply: Sender<String>,
    },
    Shutdown,
}

pub struct BookkeeperHandle {
    tx: Sender<BookkeeperCmd>,
}

impl Clone for BookkeeperHandle {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

impl BookkeeperHandle {
    pub fn start_local(
        runtime_dir: PathBuf,
        model_path: PathBuf,
        data_dir: PathBuf,
        config: BookkeeperConfig,
    ) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("create {}", data_dir.display()))?;

        let (tx, rx) = crossbeam_channel::unbounded::<BookkeeperCmd>();
        std::thread::spawn(move || {
            if let Err(e) =
                run_local_bookkeeper(&runtime_dir, &model_path, &data_dir, config, rx)
            {
                eprintln!("bookkeeper error: {e:#}");
            }
        });

        Ok(Self { tx })
    }

    pub fn start_cloud(
        target: ResolvedCloudTarget,
        data_dir: PathBuf,
        config: BookkeeperConfig,
    ) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("create {}", data_dir.display()))?;

        let (tx, rx) = crossbeam_channel::unbounded::<BookkeeperCmd>();
        std::thread::spawn(move || {
            if let Err(e) = run_cloud_bookkeeper(target, &data_dir, config, rx) {
                eprintln!("bookkeeper error: {e:#}");
            }
        });

        Ok(Self { tx })
    }

    pub fn append(&self, ev: MemoryEvent) {
        let _ = self.tx.send(BookkeeperCmd::Append(ev));
    }

    pub fn append_module_event(
        &self,
        module_id: impl Into<String>,
        event_type: impl Into<String>,
        summary: impl Into<String>,
        tags: Vec<String>,
        payload_json: Option<String>,
    ) {
        self.append(MemoryEvent {
            ts_unix_ms: crate::memory::time::now_unix_ms(),
            kind: MemoryKind::Cold,
            category: EventCategory::Module,
            source: "module".to_string(),
            module: Some(module_id.into()),
            event_type: Some(event_type.into()),
            text: summary.into(),
            tags,
            entities: Vec::new(),
            payload_json,
        });
    }

    pub fn search(
        &self,
        query: String,
        module: Option<String>,
        tag: Option<String>,
        k: usize,
    ) -> Result<Vec<MemoryHit>> {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BookkeeperCmd::Search {
                query,
                module,
                tag,
                k,
                reply: rtx,
            })
            .map_err(|e| anyhow!("{e}"))?;
        rrx.recv().map_err(|e| anyhow!("{e}"))
    }

    pub fn get_lukewarm(&self) -> Result<String> {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BookkeeperCmd::GetLukeWarm { reply: rtx })
            .map_err(|e| anyhow!("{e}"))?;
        rrx.recv().map_err(|e| anyhow!("{e}"))
    }

    pub fn summarize_module_rundown(&self, module_id: String, input: String) -> Result<String> {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BookkeeperCmd::SummarizeModuleRundown {
                module_id,
                input,
                reply: rtx,
            })
            .map_err(|e| anyhow!("{e}"))?;
        rrx.recv().map_err(|e| anyhow!("{e}"))
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(BookkeeperCmd::Shutdown);
    }
}

#[derive(Debug, Clone)]
struct Entry {
    ts_unix_ms: i64,
    source: String,
    module: Option<String>,
    event_type: Option<String>,
    text: String,
    tags: Vec<String>,
    payload_json: Option<String>,
    emb: Vec<f32>,
}

fn run_local_bookkeeper(
    runtime_dir: &Path,
    model_path: &Path,
    data_dir: &Path,
    config: BookkeeperConfig,
    rx: Receiver<BookkeeperCmd>,
) -> Result<()> {
    let llama = Llama::load(runtime_dir)?;

    let mut store = Store::load(data_dir)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut lukewarm = LukeWarm::new(config, data_dir);
    let mut dept_status =
        DepartmentStatus::load(data_dir).unwrap_or_else(|_| DepartmentStatus::new(data_dir));
    if lukewarm.summary.is_empty() {
        if let Ok(s) = std::fs::read_to_string(&lukewarm.summary_path) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                lukewarm.summary = s;
            }
        }
    }

    while let Ok(cmd) = rx.recv() {
        match cmd {
            BookkeeperCmd::Append(ev) => {
                let mut text_for_emb = String::new();
                text_for_emb.push_str(ev.text.trim());
                if !ev.tags.is_empty() {
                    text_for_emb.push_str("\nTags: ");
                    text_for_emb.push_str(&ev.tags.join(", "));
                }
                if let Some(p) = &ev.payload_json {
                    let p = p.trim();
                    if !p.is_empty() {
                        text_for_emb.push_str("\nPayload: ");
                        // Keep embeddings stable/fast: don't try to embed huge JSON blobs.
                        text_for_emb.push_str(&clamp_chars_ellipsis(p, 800));
                    }
                }
                text_for_emb.push_str("\n[");
                text_for_emb.push_str(&ev.module.clone().unwrap_or_default());
                text_for_emb.push(':');
                text_for_emb.push_str(&ev.event_type.clone().unwrap_or_default());
                text_for_emb.push(']');

                let text_for_emb = clamp_chars_ellipsis(&text_for_emb, 2400);
                let emb = llama
                    .embed_text_cpu_only(model_path, &text_for_emb)
                    .unwrap_or_default();
                store.append(&ev, emb)?;
                lukewarm.push_event(&llama, model_path, &cancel, &ev);
                dept_status.update_from_event(&ev);
            }
            BookkeeperCmd::SummarizeModuleRundown {
                module_id,
                input,
                reply,
            } => {
                let summary =
                    summarize_module_rundown(&llama, model_path, &cancel, &module_id, &input);
                let _ = reply.send(summary);
            }
            BookkeeperCmd::Search {
                query,
                module,
                tag,
                k,
                reply,
            } => {
                let qemb = llama
                    .embed_text_cpu_only(model_path, &query)
                    .unwrap_or_default();
                let hits = store.search(&query, module.as_deref(), tag.as_deref(), &qemb, k);
                let _ = reply.send(hits);
            }
            BookkeeperCmd::GetLukeWarm { reply } => {
                lukewarm.tick(&llama, model_path, &cancel);
                let _ = reply.send(lukewarm.summary.clone());
            }
            BookkeeperCmd::Shutdown => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
    }

    Ok(())
}

fn run_cloud_bookkeeper(
    target: ResolvedCloudTarget,
    data_dir: &Path,
    config: BookkeeperConfig,
    rx: Receiver<BookkeeperCmd>,
) -> Result<()> {
    let client = build_adapter(target)?;

    let mut store = Store::load(data_dir)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let mut lukewarm = LukeWarm::new(config, data_dir);
    let mut dept_status =
        DepartmentStatus::load(data_dir).unwrap_or_else(|_| DepartmentStatus::new(data_dir));
    if lukewarm.summary.is_empty() {
        if let Ok(s) = std::fs::read_to_string(&lukewarm.summary_path) {
            let s = s.trim().to_string();
            if !s.is_empty() {
                lukewarm.summary = s;
            }
        }
    }

    while let Ok(cmd) = rx.recv() {
        match cmd {
            BookkeeperCmd::Append(ev) => {
                let mut text_for_emb = String::new();
                text_for_emb.push_str(ev.text.trim());
                if !ev.tags.is_empty() {
                    text_for_emb.push_str("\nTags: ");
                    text_for_emb.push_str(&ev.tags.join(", "));
                }
                if let Some(p) = &ev.payload_json {
                    let p = p.trim();
                    if !p.is_empty() {
                        text_for_emb.push_str("\nPayload: ");
                        text_for_emb.push_str(&clamp_chars_ellipsis(p, 800));
                    }
                }
                text_for_emb.push_str("\n[");
                text_for_emb.push_str(&ev.module.clone().unwrap_or_default());
                text_for_emb.push(':');
                text_for_emb.push_str(&ev.event_type.clone().unwrap_or_default());
                text_for_emb.push(']');

                let text_for_emb = clamp_chars_ellipsis(&text_for_emb, 2400);
                let emb = client.embed_text(&text_for_emb).unwrap_or_default();
                store.append(&ev, emb)?;
                lukewarm.push_event_cloud(&*client, &cancel, &ev);
                dept_status.update_from_event(&ev);
            }
            BookkeeperCmd::SummarizeModuleRundown {
                module_id,
                input,
                reply,
            } => {
                let summary = summarize_module_rundown_cloud(&*client, &module_id, &input);
                let _ = reply.send(summary);
            }
            BookkeeperCmd::Search {
                query,
                module,
                tag,
                k,
                reply,
            } => {
                let qemb = client.embed_text(&query).unwrap_or_default();
                let hits = store.search(&query, module.as_deref(), tag.as_deref(), &qemb, k);
                let _ = reply.send(hits);
            }
            BookkeeperCmd::GetLukeWarm { reply } => {
                lukewarm.tick_cloud(&*client, &cancel);
                let _ = reply.send(lukewarm.summary.clone());
            }
            BookkeeperCmd::Shutdown => {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        }
    }

    Ok(())
}

fn summarize_module_rundown(
    llama: &Llama,
    model_path: &Path,
    cancel: &AtomicBool,
    module_id: &str,
    input: &str,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("You are the Bookkeeper for a modular research lab UI.\n");
    prompt.push_str("Task: Write a short suspend rundown for the module below.\n");
    prompt.push_str("Output rules:\n");
    prompt.push_str("- Output EXACTLY two lines.\n");
    prompt.push_str("- Line 1: starts with '- ' and is one sentence.\n");
    prompt.push_str("- Line 2: one short paragraph.\n");
    prompt.push_str("- Max ~80 words total.\n");
    prompt.push_str(
        "- Include: current status, what changed, next action, and any key artifact paths.\n",
    );
    prompt.push_str("- Be factual. If missing info, say what's missing.\n");
    prompt.push_str("- Do NOT output placeholder text like '<bullet>' or '<paragraph>'.\n");
    prompt.push_str("- No headings.\n\n");
    prompt.push_str("Module: ");
    prompt.push_str(module_id);
    prompt.push_str("\n\nContext:\n");
    prompt.push_str(input);
    prompt.push_str("\n\nRundown:\n");

    let mut out = String::new();
    let _ = llama.generate_text_cpu_only(model_path, &prompt, 140, 0.2, 0.9, 40, cancel, |t| {
        out.push_str(t)
    });
    sanitize_rundown_output(module_id, &out)
}

fn summarize_module_rundown_cloud(
    client: &dyn CloudProviderAdapter,
    module_id: &str,
    input: &str,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("Task: Write a short suspend rundown for the module below.\n");
    prompt.push_str("Output rules:\n");
    prompt.push_str("- Output EXACTLY two lines.\n");
    prompt.push_str("- Line 1: starts with '- ' and is one sentence.\n");
    prompt.push_str("- Line 2: one short paragraph.\n");
    prompt.push_str("- Max ~80 words total.\n");
    prompt.push_str(
        "- Include: current status, what changed, next action, and any key artifact paths.\n",
    );
    prompt.push_str("- Be factual. If missing info, say what's missing.\n");
    prompt.push_str("- No headings.\n\n");
    prompt.push_str("Module: ");
    prompt.push_str(module_id);
    prompt.push_str("\n\nContext:\n");
    prompt.push_str(input);

    let system =
        "You are the Bookkeeper for a modular research lab UI. Keep outputs compact and factual.";
    match client.chat_completion(system, &prompt, 140, 0.2, 0.9) {
        Ok(out) => sanitize_rundown_output(module_id, &out),
        Err(_) => String::new(),
    }
}

fn clamp_chars_ellipsis(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

fn sanitize_rundown_output(module_id: &str, raw: &str) -> String {
    fn clamp_chars(s: &str, max_chars: usize) -> String {
        if s.chars().count() <= max_chars {
            return s.to_string();
        }
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("...");
        out
    }

    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return String::new();
    }

    // Defensive: strip common placeholder patterns if a model echoes instructions.
    s = s.replace("<bullet>", "").replace("<paragraph>", "");

    let lines: Vec<String> = s
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    if lines.is_empty() {
        return String::new();
    }

    let mut bullet: Option<String> = None;
    let mut para_parts: Vec<String> = Vec::new();
    for l in &lines {
        if bullet.is_none() && l.starts_with('-') {
            bullet = Some(l.to_string());
        } else {
            para_parts.push(l.to_string());
        }
    }

    let bullet = bullet.unwrap_or_else(|| {
        let first = lines.first().cloned().unwrap_or_default();
        if first.starts_with('-') {
            first
        } else {
            format!("- {first}")
        }
    });

    let bullet = if bullet.trim() == "-" || bullet.trim().is_empty() {
        format!("- {module_id}: suspend rundown")
    } else {
        bullet
    };

    let para = para_parts.join(" ");
    let para = if para.trim().is_empty() {
        "Next: review the module state and continue.".to_string()
    } else {
        para
    };

    let bullet = clamp_chars(bullet.trim(), 180);
    let para = clamp_chars(para.trim(), 520);
    format!("{bullet}\n{para}")
}

fn sanitize_lukewarm_output(raw: &str) -> String {
    fn clamp_chars(s: &str, max_chars: usize) -> String {
        if s.chars().count() <= max_chars {
            return s.to_string();
        }
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("...");
        out
    }

    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return String::new();
    }

    s = s
        .replace("<bullet>", "")
        .replace("<paragraph>", "")
        .replace("```", "");

    let lines: Vec<String> = s
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect();

    if lines.is_empty() {
        return String::new();
    }

    let mut bullet: Option<String> = None;
    let mut para_parts = Vec::new();
    for line in &lines {
        let normalized = line
            .trim_start_matches(['-', '*', '•', ' '])
            .trim()
            .to_string();
        if looks_like_lukewarm_scaffolding(&normalized) {
            continue;
        }
        if bullet.is_none() && !normalized.is_empty() {
            bullet = Some(format!("- {normalized}"));
        } else if !normalized.is_empty() {
            para_parts.push(normalized);
        }
    }

    let bullet = bullet.unwrap_or_else(|| "- Recent activity updated.".to_string());
    let para = if para_parts.is_empty() {
        "Current context is available in the recent activity summary.".to_string()
    } else {
        para_parts.join(" ")
    };

    let bullet = clamp_chars(bullet.trim(), 260);
    let para = clamp_chars(para.trim(), 900);
    format!("{bullet}\n{para}")
}

fn looks_like_lukewarm_scaffolding(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    [
        "okay, let's",
        "ok, let's",
        "first, i need",
        "i need to parse",
        "looking at the activity",
        "the key points",
        "the bullet should",
        "the paragraph needs",
        "the user wants",
    ]
    .iter()
    .any(|pattern| normalized.starts_with(pattern))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DepartmentStatusItem {
    module_id: String,
    #[serde(default)]
    display_name: Option<String>,
    ts_unix_ms: i64,
    summary: String,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct DepartmentStatus {
    items: Vec<DepartmentStatusItem>,
    #[serde(skip, default)]
    path_json: PathBuf,
    #[serde(skip, default)]
    path_md: PathBuf,
}

impl DepartmentStatus {
    fn new(data_dir: &Path) -> Self {
        Self {
            items: Vec::new(),
            path_json: data_dir.join("departments.json"),
            path_md: data_dir.join("departments.md"),
        }
    }

    fn load(data_dir: &Path) -> Result<Self> {
        let path_json = data_dir.join("departments.json");
        let path_md = data_dir.join("departments.md");

        if path_json.is_file() {
            let bytes = std::fs::read(&path_json)
                .with_context(|| format!("read {}", path_json.display()))?;
            let mut s: DepartmentStatus = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse {}", path_json.display()))?;
            // Defensive cleanup: keep summaries small and strip placeholder echoes from older runs.
            s.items.retain_mut(|it| {
                it.summary = sanitize_rundown_output(&it.module_id, &it.summary);
                !it.summary.trim().is_empty()
            });
            s.path_json = path_json;
            s.path_md = path_md;
            let _ = s.persist();
            Ok(s)
        } else {
            Ok(Self {
                items: Vec::new(),
                path_json,
                path_md,
            })
        }
    }

    fn update_from_event(&mut self, ev: &MemoryEvent) {
        if ev.category != EventCategory::Module {
            return;
        }
        if ev.event_type.as_deref() != Some("suspend_rundown") {
            return;
        }
        let Some(module_id) = ev.module.as_deref() else {
            return;
        };
        if module_id.trim().is_empty() {
            return;
        }
        let summary = ev.text.trim();
        if summary.is_empty() {
            return;
        }
        // Keep this file small and safe to inject into prompts.
        let summary = sanitize_rundown_output(module_id, summary);
        if summary.is_empty() {
            return;
        }

        let display_name = ev
            .payload_json
            .as_deref()
            .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok())
            .and_then(|v| {
                v.get("display_name")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string())
            });

        // Upsert
        self.items.retain(|it| it.module_id != module_id);
        self.items.push(DepartmentStatusItem {
            module_id: module_id.to_string(),
            display_name,
            ts_unix_ms: ev.ts_unix_ms,
            summary,
        });
        self.items.sort_by(|a, b| {
            a.display_name
                .as_deref()
                .unwrap_or(&a.module_id)
                .to_lowercase()
                .cmp(
                    &b.display_name
                        .as_deref()
                        .unwrap_or(&b.module_id)
                        .to_lowercase(),
                )
        });

        let _ = self.persist();
    }

    fn persist(&self) -> Result<()> {
        // JSON store
        if let Some(dir) = self.path_json.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(self).context("serialize departments.json")?;
        std::fs::write(&self.path_json, bytes)
            .with_context(|| format!("write {}", self.path_json.display()))?;

        // Human-readable markdown (for orchestrator injection)
        let mut md = String::new();
        md.push_str("Department Status Updates\n");
        md.push_str("=========================\n\n");
        for it in &self.items {
            let name = it.display_name.as_deref().unwrap_or(&it.module_id);
            md.push_str("## ");
            md.push_str(name);
            md.push_str(" (");
            md.push_str(&it.module_id);
            md.push_str(")\n\n");
            md.push_str(it.summary.trim());
            md.push_str("\n\n");
        }
        std::fs::write(&self.path_md, md)
            .with_context(|| format!("write {}", self.path_md.display()))?;
        Ok(())
    }
}

struct LukeWarm {
    cfg: BookkeeperConfig,
    summary_path: PathBuf,
    last_summary_at: Option<std::time::Instant>,
    buf: Vec<String>,
    token_est: usize,
    summary: String,
}

impl LukeWarm {
    fn new(cfg: BookkeeperConfig, data_dir: &Path) -> Self {
        Self {
            cfg,
            summary_path: data_dir.join("lukewarm.txt"),
            last_summary_at: None,
            buf: Vec::new(),
            token_est: 0,
            summary: String::new(),
        }
    }

    fn push_event(
        &mut self,
        llama: &Llama,
        model_path: &Path,
        cancel: &AtomicBool,
        ev: &MemoryEvent,
    ) {
        // Estimate tokens as ~4 chars/token (good enough for a rolling window).
        let line = format!(
            "[{}] ({}/{}) {}",
            ev.source,
            ev.module.clone().unwrap_or_else(|| "-".to_string()),
            ev.event_type.clone().unwrap_or_else(|| "-".to_string()),
            ev.text
        );
        self.token_est += (line.len().max(1) + 3) / 4;
        self.buf.push(line);

        // Keep the most recent summary around as a compact continuity anchor once
        // raw events start aging out of the active rolling buffer.
        while self.token_est > self.cfg.lukewarm_token_window && !self.buf.is_empty() {
            let removed = self.buf.remove(0);
            self.token_est = self
                .token_est
                .saturating_sub((removed.len().max(1) + 3) / 4);
        }

        self.maybe_summarize(llama, model_path, cancel, false);
    }

    fn push_event_cloud(
        &mut self,
        client: &dyn CloudProviderAdapter,
        cancel: &AtomicBool,
        ev: &MemoryEvent,
    ) {
        let line = format!(
            "[{}] ({}/{}) {}",
            ev.source,
            ev.module.clone().unwrap_or_else(|| "-".to_string()),
            ev.event_type.clone().unwrap_or_else(|| "-".to_string()),
            ev.text
        );
        self.token_est += (line.len().max(1) + 3) / 4;
        self.buf.push(line);

        while self.token_est > self.cfg.lukewarm_token_window && !self.buf.is_empty() {
            let removed = self.buf.remove(0);
            self.token_est = self
                .token_est
                .saturating_sub((removed.len().max(1) + 3) / 4);
        }

        self.maybe_summarize_cloud(client, cancel, false);
    }

    fn tick(&mut self, llama: &Llama, model_path: &Path, cancel: &AtomicBool) {
        self.maybe_summarize(llama, model_path, cancel, true);
    }

    fn tick_cloud(&mut self, client: &dyn CloudProviderAdapter, cancel: &AtomicBool) {
        self.maybe_summarize_cloud(client, cancel, true);
    }

    fn maybe_summarize(
        &mut self,
        llama: &Llama,
        model_path: &Path,
        cancel: &AtomicBool,
        force_by_time: bool,
    ) {
        // Periodic update gate (default 30s) so summary stays fresh even if token window isn't full.
        let now = std::time::Instant::now();
        let due_by_time = self
            .last_summary_at
            .map(|t| now.duration_since(t) >= std::time::Duration::from_secs(30))
            .unwrap_or(true);

        let due_by_size = self.token_est >= self.cfg.lukewarm_token_window.saturating_sub(200);
        if !(due_by_size || (force_by_time && due_by_time)) {
            return;
        }

        let mut prompt = String::new();
        if !self.summary.trim().is_empty() {
            prompt.push_str("Existing rolling summary:\n");
            prompt.push_str(self.summary.trim());
            prompt.push_str(
                "\n\nUpdate that summary using the recent activity below. Preserve still-relevant context, drop stale detail, and keep the result compact.\n\n",
            );
        }
        prompt.push_str("Summarize the recent activity below into ONE bullet point and ONE short paragraph (max ~80 words). ");
        prompt
            .push_str("Focus on what happened, key decisions, and what is currently in progress. ");
        prompt.push_str("Do not include more than one bullet.\n\nActivity:\n");
        for l in &self.buf {
            prompt.push_str("- ");
            prompt.push_str(l);
            prompt.push('\n');
        }
        prompt.push_str("\nOutput format:\n- <bullet>\n<paragraph>\n");

        let mut out = String::new();
        let _ = llama.generate_text_cpu_only(
            model_path,
            &prompt,
            self.cfg.lukewarm_max_tokens,
            self.cfg.temp,
            self.cfg.top_p,
            self.cfg.top_k,
            cancel,
            |t| out.push_str(t),
        );
        let out = sanitize_lukewarm_output(out.trim());
        if !out.is_empty() {
            self.summary = out;
            self.last_summary_at = Some(now);
            self.prune_buffer_after_success();
            let _ = std::fs::write(&self.summary_path, &self.summary);
        }
    }

    fn maybe_summarize_cloud(
        &mut self,
        client: &dyn CloudProviderAdapter,
        cancel: &AtomicBool,
        force_by_time: bool,
    ) {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let now = std::time::Instant::now();
        let due_by_time = self
            .last_summary_at
            .map(|t| now.duration_since(t) >= std::time::Duration::from_secs(30))
            .unwrap_or(true);

        let due_by_size = self.token_est >= self.cfg.lukewarm_token_window.saturating_sub(200);
        if !(due_by_size || (force_by_time && due_by_time)) {
            return;
        }

        let mut prompt = String::new();
        if !self.summary.trim().is_empty() {
            prompt.push_str("Existing rolling summary:\n");
            prompt.push_str(self.summary.trim());
            prompt.push_str(
                "\n\nUpdate that summary using the recent activity below. Preserve still-relevant context, drop stale detail, and keep the result compact.\n\n",
            );
        }
        prompt.push_str("Summarize the recent activity below into ONE bullet point and ONE short paragraph (max ~80 words). ");
        prompt
            .push_str("Focus on what happened, key decisions, and what is currently in progress. ");
        prompt.push_str("Do not include more than one bullet.\n\nActivity:\n");
        for l in &self.buf {
            prompt.push_str("- ");
            prompt.push_str(l);
            prompt.push('\n');
        }

        let system =
            "You are the Bookkeeper for a modular research lab UI. Keep summaries compact and factual.";
        if let Ok(out) = client.chat_completion(
            system,
            &prompt,
            self.cfg.lukewarm_max_tokens,
            self.cfg.temp,
            self.cfg.top_p,
        ) {
            let out = sanitize_lukewarm_output(out.trim());
            if !out.is_empty() {
                self.summary = out;
                self.last_summary_at = Some(now);
                self.prune_buffer_after_success();
                let _ = std::fs::write(&self.summary_path, &self.summary);
            }
        }
    }

    fn prune_buffer_after_success(&mut self) {
        let target = (self.cfg.lukewarm_token_window / 3).max(200);
        while self.token_est > target && !self.buf.is_empty() {
            let removed = self.buf.remove(0);
            self.token_est = self
                .token_est
                .saturating_sub((removed.len().max(1) + 3) / 4);
        }
    }
}

struct Store {
    log_path: PathBuf,
    entries: Vec<Entry>,
}

impl Store {
    fn load(data_dir: &Path) -> Result<Self> {
        let log_path = data_dir.join("cold_log.jsonl");
        let mut entries = Vec::new();

        if log_path.is_file() {
            let f =
                File::open(&log_path).with_context(|| format!("open {}", log_path.display()))?;
            for line in BufReader::new(f).lines().flatten() {
                if let Ok(ev) = parse_event_line(&line) {
                    entries.push(Entry {
                        ts_unix_ms: ev.ts_unix_ms,
                        source: ev.source,
                        module: ev.module,
                        event_type: ev.event_type,
                        text: ev.text,
                        tags: ev.tags,
                        payload_json: ev.payload_json,
                        emb: Vec::new(),
                    });
                }
            }
        }

        Ok(Self { log_path, entries })
    }

    fn append(&mut self, ev: &MemoryEvent, emb: Vec<f32>) -> Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .with_context(|| format!("open {}", self.log_path.display()))?;
        writeln!(f, "{}", format_event_line(ev, emb.len()))?;
        self.entries.push(Entry {
            ts_unix_ms: ev.ts_unix_ms,
            source: ev.source.clone(),
            module: ev.module.clone(),
            event_type: ev.event_type.clone(),
            text: ev.text.clone(),
            tags: ev.tags.clone(),
            payload_json: ev.payload_json.clone(),
            emb,
        });
        Ok(())
    }

    fn search(
        &self,
        query: &str,
        module: Option<&str>,
        tag: Option<&str>,
        qemb: &[f32],
        k: usize,
    ) -> Vec<MemoryHit> {
        let mut scored = Vec::new();
        let module = module.and_then(|s| if s.trim().is_empty() { None } else { Some(s) });
        let tag = tag.and_then(|s| if s.trim().is_empty() { None } else { Some(s) });

        for e in &self.entries {
            if let Some(m) = module {
                if e.module.as_deref() != Some(m) {
                    continue;
                }
            }
            if let Some(t) = tag {
                if !e.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                    continue;
                }
            }

            let mut score = 0.0f32;
            if !qemb.is_empty() && !e.emb.is_empty() && qemb.len() == e.emb.len() {
                score = cosine_sim(qemb, &e.emb);
            } else if !query.trim().is_empty() {
                let q = query.to_lowercase();
                let hay = format!(
                    "{}\n{}\n{}\n{}\n{}\n{}",
                    e.source,
                    e.module.clone().unwrap_or_default(),
                    e.event_type.clone().unwrap_or_default(),
                    e.text,
                    e.tags.join(", "),
                    e.payload_json.clone().unwrap_or_default()
                )
                .to_lowercase();
                if hay.contains(&q) {
                    score = 0.25;
                }
            }
            if score > 0.0 {
                scored.push(MemoryHit {
                    score,
                    source: e.source.clone(),
                    module: e.module.clone(),
                    event_type: e.event_type.clone(),
                    text: e.text.clone(),
                    tags: e.tags.clone(),
                    payload_json: e.payload_json.clone(),
                    ts_unix_ms: e.ts_unix_ms,
                });
            }
        }

        scored.sort_by(|a, b| b.score.total_cmp(&a.score));
        scored.truncate(k.max(1));
        scored
    }
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn format_event_line(ev: &MemoryEvent, emb_len: usize) -> String {
    // Minimal JSONL without pulling in serde yet.
    // Note: text/source are escaped to keep one-line JSON.
    let module = ev.module.as_deref().unwrap_or("");
    let event_type = ev.event_type.as_deref().unwrap_or("");
    let category = match ev.category {
        EventCategory::Chat => "chat",
        EventCategory::Module => "module",
    };
    let tags_json = format_json_string_array(&ev.tags);
    let entities_json = format_json_string_array(&ev.entities);
    let payload = ev.payload_json.as_deref().unwrap_or("");
    format!(
        "{{\"ts\":{},\"kind\":\"cold\",\"cat\":\"{}\",\"source\":\"{}\",\"module_id\":\"{}\",\"module\":\"{}\",\"event_type\":\"{}\",\"type\":\"{}\",\"summary\":\"{}\",\"text\":\"{}\",\"tags\":{},\"entities\":{},\"payload_json\":\"{}\",\"emb_len\":{}}}",
        ev.ts_unix_ms,
        category,
        escape_json(&ev.source),
        escape_json(module),
        escape_json(module),
        escape_json(event_type),
        escape_json(event_type),
        escape_json(&ev.text),
        escape_json(&ev.text),
        tags_json,
        entities_json,
        escape_json(payload),
        emb_len
    )
}

fn parse_event_line(line: &str) -> Result<MemoryEvent> {
    // Best-effort parser for the fields we wrote.
    fn get_str<'a>(s: &'a str, key: &str) -> Option<String> {
        let pat = format!("\"{key}\":\"");
        let i = s.find(&pat)? + pat.len();
        let rest = &s[i..];
        let j = rest.find('"')?;
        Some(unescape_json(&rest[..j]))
    }
    fn get_i64(s: &str, key: &str) -> Option<i64> {
        let pat = format!("\"{key}\":");
        let i = s.find(&pat)? + pat.len();
        let rest = &s[i..];
        let j = rest.find(',').or_else(|| rest.find('}'))?;
        rest[..j].trim().parse().ok()
    }
    fn get_array(s: &str, key: &str) -> Vec<String> {
        let pat = format!("\"{key}\":[");
        let Some(i0) = s.find(&pat) else {
            return Vec::new();
        };
        let mut i = i0 + pat.len();
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        while i < bytes.len() {
            let ch = bytes[i] as char;
            if ch == ']' {
                break;
            }
            if ch == '"' {
                i += 1;
                let start = i;
                while i < bytes.len() && (bytes[i] as char) != '"' {
                    if (bytes[i] as char) == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i <= bytes.len() {
                    let raw = &s[start..i];
                    out.push(unescape_json(raw));
                }
            }
            i += 1;
        }
        out
    }

    let cat = get_str(line, "cat").unwrap_or_else(|| "chat".to_string());
    let category = if cat == "module" {
        EventCategory::Module
    } else {
        EventCategory::Chat
    };

    let module = get_str(line, "module_id")
        .or_else(|| get_str(line, "module"))
        .and_then(|s| if s.is_empty() { None } else { Some(s) });
    let event_type = get_str(line, "event_type")
        .or_else(|| get_str(line, "type"))
        .and_then(|s| if s.is_empty() { None } else { Some(s) });
    let text = get_str(line, "summary")
        .or_else(|| get_str(line, "text"))
        .ok_or_else(|| anyhow!("missing text"))?;
    let tags = get_array(line, "tags");
    let entities = get_array(line, "entities");
    let payload_json =
        get_str(line, "payload_json").and_then(|s| if s.is_empty() { None } else { Some(s) });

    Ok(MemoryEvent {
        ts_unix_ms: get_i64(line, "ts").ok_or_else(|| anyhow!("missing ts"))?,
        kind: MemoryKind::Cold,
        category,
        source: get_str(line, "source").ok_or_else(|| anyhow!("missing source"))?,
        module,
        event_type,
        text,
        tags,
        entities,
        payload_json,
    })
}

fn format_json_string_array(v: &[String]) -> String {
    let mut out = String::from("[");
    for (i, s) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('\"');
        out.push_str(&escape_json(s));
        out.push('\"');
    }
    out.push(']');
    out
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(ch) = it.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match it.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lukewarm_sanitizer_keeps_useful_body_text() {
        let body = "The assistant and user inspected the bookkeeper rolling summary path. "
            .repeat(8);
        let raw = format!(
            "- Bookkeeper rolling summary update investigated.\n{body}Next action is to keep the summary fresh without repeatedly reprocessing the same full buffer."
        );

        let sanitized = sanitize_lukewarm_output(&raw);

        assert!(sanitized.contains("Bookkeeper rolling summary update investigated"));
        assert!(sanitized.contains("Next action"));
        assert!(
            sanitized.chars().count() > 500,
            "summary was unexpectedly over-truncated: {sanitized}"
        );
    }

    #[test]
    fn successful_lukewarm_update_prunes_raw_buffer() {
        let data_dir = std::env::temp_dir().join(format!(
            "chattycog-lukewarm-prune-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&data_dir);

        let mut lukewarm = LukeWarm::new(
            BookkeeperConfig {
                lukewarm_token_window: 900,
                ..BookkeeperConfig::default()
            },
            &data_dir,
        );
        for i in 0..12 {
            let line = format!("event {i}: {}", "recent work item ".repeat(16));
            lukewarm.token_est += (line.len().max(1) + 3) / 4;
            lukewarm.buf.push(line);
        }
        assert!(lukewarm.token_est > 300);

        lukewarm.prune_buffer_after_success();

        assert!(lukewarm.token_est <= 300);
        assert!(!lukewarm.buf.is_empty());
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
