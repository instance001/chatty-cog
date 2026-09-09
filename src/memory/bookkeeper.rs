use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};

use crate::cloud_ai::{CloudProviderAdapter, ResolvedCloudTarget, build_adapter};
use crate::app_paths::read_gguf_architecture;
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
    DraftSemanticSort {
        prompt: String,
        max_tokens: usize,
        reply: Sender<Result<String, String>>,
    },
    DraftColdAtlas {
        prompt: String,
        max_tokens: usize,
        reply: Sender<Result<String, String>>,
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
            if let Err(e) = run_local_bookkeeper(&runtime_dir, &model_path, &data_dir, config, rx) {
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

    pub fn draft_semantic_sort(&self, prompt: String, max_tokens: usize) -> Result<String> {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BookkeeperCmd::DraftSemanticSort {
                prompt,
                max_tokens,
                reply: rtx,
            })
            .map_err(|e| anyhow!("{e}"))?;
        rrx.recv()
            .map_err(|e| anyhow!("{e}"))?
            .map_err(|e| anyhow!("{e}"))
    }

    pub fn draft_cold_atlas(&self, prompt: String, max_tokens: usize) -> Result<String> {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        self.tx
            .send(BookkeeperCmd::DraftColdAtlas {
                prompt,
                max_tokens,
                reply: rtx,
            })
            .map_err(|e| anyhow!("{e}"))?;
        rrx.recv()
            .map_err(|e| anyhow!("{e}"))?
            .map_err(|e| anyhow!("{e}"))
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
    let gpt_oss_json_mode = read_gguf_architecture(model_path)
        .ok()
        .flatten()
        .is_some_and(|architecture| architecture.eq_ignore_ascii_case("gpt-oss"));

    let mut store = Store::load(data_dir)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let generation_top_k = config.top_k;
    let mut lukewarm = LukeWarm::new(config, data_dir);
    let mut dept_status =
        DepartmentStatus::load(data_dir).unwrap_or_else(|_| DepartmentStatus::new(data_dir));
    if lukewarm.summary.is_empty() {
        if let Ok(s) = std::fs::read_to_string(&lukewarm.summary_path) {
            let s = sanitize_lukewarm_output(&s);
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
            BookkeeperCmd::DraftSemanticSort {
                prompt,
                max_tokens,
                reply,
            } => {
                let mut out = String::new();
                let draft_prompt = local_json_draft_prompt(&prompt, gpt_oss_json_mode);
                let result = llama
                    .generate_chat(
                        model_path,
                        "You are the Bookkeeper semantic sorter. Return compact JSON only.",
                        &draft_prompt,
                        max_tokens.max(256),
                        0.1,
                        0.95,
                        generation_top_k,
                        &cancel,
                        |t| out.push_str(t),
                    )
                    .and_then(|_| sanitize_required_json_candidate(&out))
                    .map_err(|err| err.to_string());
                let _ = reply.send(result);
            }
            BookkeeperCmd::DraftColdAtlas {
                prompt,
                max_tokens,
                reply,
            } => {
                let mut out = String::new();
                let draft_prompt = local_json_draft_prompt(&prompt, gpt_oss_json_mode);
                let result = llama
                    .generate_chat(
                        model_path,
                        "You are the Bookkeeper cold-atlas drafter. Return compact JSON only.",
                        &draft_prompt,
                        max_tokens.max(256),
                        0.1,
                        0.95,
                        generation_top_k,
                        &cancel,
                        |t| out.push_str(t),
                    )
                    .and_then(|_| sanitize_required_json_candidate(&out))
                    .map_err(|err| err.to_string());
                let _ = reply.send(result);
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
            let s = sanitize_lukewarm_output(&s);
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
            BookkeeperCmd::DraftSemanticSort {
                prompt,
                max_tokens,
                reply,
            } => {
                let system = "You are the Bookkeeper semantic sorter. Return compact JSON only.";
                let result = client
                    .chat_completion(system, &prompt, max_tokens.max(256), 0.1, 0.95)
                    .and_then(|out| sanitize_required_json_candidate(&out))
                    .map_err(|err| err.to_string());
                let _ = reply.send(result);
            }
            BookkeeperCmd::DraftColdAtlas {
                prompt,
                max_tokens,
                reply,
            } => {
                let system = "You are the Bookkeeper cold-atlas drafter. Return compact JSON only.";
                let result = client
                    .chat_completion(system, &prompt, max_tokens.max(256), 0.1, 0.95)
                    .and_then(|out| sanitize_required_json_candidate(&out))
                    .map_err(|err| err.to_string());
                let _ = reply.send(result);
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

fn sanitize_json_candidate(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```JSON"))
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|body| body.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);

    if serde_json::from_str::<serde_json::Value>(unfenced).is_ok() {
        return unfenced.to_string();
    }

    // Some local models prefix an otherwise-valid JSON response with reasoning
    // or their chat-protocol markers. Parse from each object boundary so a
    // valid JSON value can be recovered without trusting surrounding text.
    for (start, _) in unfenced.match_indices('{') {
        let mut values = serde_json::Deserializer::from_str(&unfenced[start..])
            .into_iter::<serde_json::Value>();
        if let Some(Ok(value)) = values.next()
            && value.is_object()
        {
            // Keep the model's JSON bytes rather than serializing `value` again:
            // downstream review can preserve its field order and whitespace.
            return unfenced[start..start + values.byte_offset()].trim().to_string();
        }
    }

    unfenced.to_string()
}

fn local_json_draft_prompt(prompt: &str, gpt_oss_json_mode: bool) -> String {
    if !gpt_oss_json_mode {
        return prompt.to_string();
    }

    // GPT-OSS can spend an entire small structured-output budget in its analysis
    // channel. Start its final constrained JSON channel explicitly; ordinary GGUF
    // chat models continue to receive the portable plain-language prompt above.
    format!(
        "{}\n\n<|start|>assistant<|channel|>final <|constrain|>JSON<|message|>",
        prompt.trim_end()
    )
}

fn sanitize_required_json_candidate(raw: &str) -> Result<String> {
    let candidate = sanitize_json_candidate(raw);
    if candidate.trim().is_empty() {
        anyhow::bail!(
            "Bookkeeper produced no visible structured output; no draft was accepted or written"
        );
    }
    Ok(candidate)
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

    let mut s = gpt_oss_final_visible_or_empty(raw).unwrap_or(raw).trim().to_string();
    if s.is_empty() {
        return String::new();
    }

    s = s
        .replace("<bullet>", "")
        .replace("<paragraph>", "")
        .replace("```", "")
        .replace("<|start|>", "")
        .replace("<|end|>", "")
        .replace("<|return|>", "");

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
    let mut usable_lines = 0usize;
    for line in &lines {
        let normalized = line
            .trim_start_matches(['-', '*', '•', ' '])
            .trim()
            .to_string();
        if looks_like_lukewarm_scaffolding(&normalized)
            || looks_like_lukewarm_instruction_echo(&normalized)
        {
            continue;
        }
        usable_lines += 1;
        if bullet.is_none() && !normalized.is_empty() {
            bullet = Some(format!("- {normalized}"));
        } else if !normalized.is_empty() {
            para_parts.push(normalized);
        }
    }

    if usable_lines == 0 {
        return String::new();
    }

    let bullet = bullet.unwrap_or_else(|| "- Recent activity updated.".to_string());
    let para = if para_parts.is_empty() {
        "Current context is available in the recent activity summary.".to_string()
    } else {
        para_parts.join(" ")
    };

    let bullet = clamp_chars(bullet.trim(), 260);
    let para = clamp_chars(para.trim(), 900);
    let out = format!("{bullet}\n{para}");
    if looks_like_lukewarm_non_summary(&out) {
        String::new()
    } else {
        out
    }
}

/// Returns the final channel from a GPT-OSS protocol completion. An analysis
/// channel without a final channel is explicitly an answerless completion, so
/// callers must not repurpose the preceding scratch work as user-facing text.
fn gpt_oss_final_visible_or_empty(raw: &str) -> Option<&str> {
    let lower = raw.to_ascii_lowercase();
    const ANALYSIS: &str = "<|channel|>analysis<|message|>";
    const FINAL: &str = "<|channel|>final<|message|>";

    if let Some(idx) = lower.rfind(FINAL) {
        Some(&raw[idx + FINAL.len()..])
    } else if lower.contains(ANALYSIS) {
        Some("")
    } else {
        None
    }
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

fn looks_like_lukewarm_non_summary(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    [
        "i'm ready to help",
        "i am ready to help",
        "could you share",
        "please share the recent activity",
        "ask for activity",
    ]
    .iter()
    .any(|pattern| normalized.contains(pattern))
}

fn looks_like_lukewarm_instruction_echo(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    let direct_echoes = [
        "we need to produce a bullet",
        "we need to produce one bullet",
        "we need to produce:",
        "we need to summarize this chat",
        "one bullet and one short paragraph",
        "one bullet point, one paragraph",
        "the user wants a bullet",
        "the user wants a paragraph",
        "summarizing the recent activity",
        "output format",
        "the instruction specifies",
        "single bullet point followed by a paragraph",
        "this format ensures",
        "paragraph should be max",
        "bullet should be concise",
        "we need to keep it short",
        "let's count",
        "that's 30 words",
    ];
    direct_echoes.iter().any(|pattern| normalized.contains(pattern))
        || count_non_overlapping(&normalized, "the user wants") >= 3
        || count_non_overlapping(&normalized, "we need to") >= 3
}

fn count_non_overlapping(text: &str, needle: &str) -> usize {
    text.match_indices(needle).count()
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
        _llama: &Llama,
        _model_path: &Path,
        _cancel: &AtomicBool,
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

        self.maybe_summarize_host(false);
    }

    fn push_event_cloud(
        &mut self,
        _client: &dyn CloudProviderAdapter,
        _cancel: &AtomicBool,
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

        self.maybe_summarize_host(false);
    }

    fn tick(&mut self, _llama: &Llama, _model_path: &Path, _cancel: &AtomicBool) {
        self.maybe_summarize_host(true);
    }

    fn tick_cloud(&mut self, _client: &dyn CloudProviderAdapter, _cancel: &AtomicBool) {
        self.maybe_summarize_host(true);
    }

    fn maybe_summarize_host(&mut self, force_by_time: bool) {
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

        if let Some(summary) = build_host_lukewarm_summary(&self.buf) {
            self.summary = summary;
            self.last_summary_at = Some(now);
            self.prune_buffer_after_success();
            let _ = std::fs::write(&self.summary_path, &self.summary);
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

/// Luke Warm is intentionally host-built.  It is prompt context for the main
/// chat, so letting a small model summarize its own output creates a feedback
/// loop where malformed text can be re-injected on every turn.  Preserve only
/// the most recent user-authored activity and never model prose.
fn build_host_lukewarm_summary(lines: &[String]) -> Option<String> {
    let mut items = Vec::new();
    for line in lines {
        if !line.starts_with("[user]") {
            continue;
        }
        let Some((_, text)) = line.split_once(") ") else {
            continue;
        };
        let cleaned = text
            .chars()
            .filter(|ch| !ch.is_control())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if cleaned.is_empty() || items.last().is_some_and(|last: &String| last == &cleaned) {
            continue;
        }
        items.push(cleaned);
    }

    let latest = items.last()?;
    let latest = clamp_chars_ellipsis(latest, 260);
    let earlier = items
        .iter()
        .rev()
        .skip(1)
        .take(2)
        .rev()
        .map(|item| clamp_chars_ellipsis(item, 140))
        .collect::<Vec<_>>();

    let mut summary = format!("- Recent user activity: {latest}");
    if earlier.is_empty() {
        summary.push_str("\nCurrent user activity is recorded in the cold log.");
    } else {
        summary.push_str("\nEarlier user activity: ");
        summary.push_str(&earlier.join(" | "));
    }
    Some(summary)
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
        let body =
            "The assistant and user inspected the bookkeeper rolling summary path. ".repeat(8);
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
    fn host_lukewarm_summary_uses_only_recent_user_activity() {
        let lines = vec![
            "[assistant] (-/message) Invented workflow details.".to_string(),
            "[user] (-/message) Create Project Alpha with a short description.".to_string(),
            "[assistant] (-/message) More model prose.".to_string(),
            "[user] (-/message) Track the project in the cold log.".to_string(),
        ];

        let summary = build_host_lukewarm_summary(&lines).expect("user activity summary");

        assert!(summary.contains("Track the project in the cold log"));
        assert!(summary.contains("Create Project Alpha"));
        assert!(!summary.contains("Invented workflow"));
        assert!(!summary.contains("More model prose"));
    }

    #[test]
    fn lukewarm_sanitizer_rejects_incomplete_gpt_oss_protocol_output() {
        let raw = "We need to ask for activity.<|end|><|start|>assistant<|channel|>analysis<|message|>No activity is available.";

        assert!(sanitize_lukewarm_output(raw).is_empty());
    }

    #[test]
    fn lukewarm_sanitizer_keeps_only_gpt_oss_final_channel() {
        let raw = "Scratch work.<|start|>assistant<|channel|>analysis<|message|>Plan.\
<|end|><|start|>assistant<|channel|>final<|message|>- Summary refreshed.\nCurrent work remains available.<|end|>";

        assert_eq!(
            sanitize_lukewarm_output(raw),
            "- Summary refreshed.\nCurrent work remains available."
        );
    }

    #[test]
    fn lukewarm_sanitizer_rejects_repeated_summary_instruction_echoes() {
        let raw = "- We need to produce a bullet and a paragraph summarizing the recent activity. \
The user wants a bullet and a paragraph. The user wants a bullet and a paragraph.";

        assert!(sanitize_lukewarm_output(raw).is_empty());
    }

    #[test]
    fn lukewarm_sanitizer_rejects_output_format_echoes() {
        let raw = "- The output format requires a single bullet point followed by a paragraph. \
The instruction specifies that the assistant should provide a concise bullet. This format ensures clarity and brevity.";

        assert!(sanitize_lukewarm_output(raw).is_empty());
    }

    #[test]
    fn lukewarm_sanitizer_rejects_max_word_count_prompt_echoes() {
        let raw = "We need to summarize this chat. We need to produce: One bullet point, one paragraph. \
The paragraph should be max 80 words. The bullet should be concise.";

        assert!(sanitize_lukewarm_output(raw).is_empty());
    }

    #[test]
    fn lukewarm_sanitizer_rejects_self_counted_prompt_echoes() {
        let raw = "We need to keep it short. Let's count: that is 30 words.\n\
The user wants to know the workflow again.";

        assert!(sanitize_lukewarm_output(raw).is_empty());
    }

    #[test]
    fn semantic_sort_json_sanitizer_strips_markdown_fences() {
        let raw = "```json\n{\"items\":[]}\n```";

        assert_eq!(sanitize_json_candidate(raw), "{\"items\":[]}");
    }

    #[test]
    fn semantic_sort_json_sanitizer_extracts_valid_json_from_prose() {
        let raw = "Here is the JSON:\n{\"items\":[{\"evidence_id\":\"e1\",\"decisions\":[]}]}";

        assert_eq!(
            sanitize_json_candidate(raw),
            "{\"items\":[{\"evidence_id\":\"e1\",\"decisions\":[]}]}"
        );
    }

    #[test]
    fn semantic_sort_json_sanitizer_extracts_json_after_gpt_oss_protocol_text() {
        let raw = concat!(
            "I should classify this carefully.\n",
            "<|end|><|start|>assistant<|channel|>final <|constrain|>JSON<|message|>",
            "{\"items\":[{\"evidence_id\":\"e1\",\"decisions\":[]}]}"
        );

        assert_eq!(
            sanitize_json_candidate(raw),
            "{\"items\":[{\"evidence_id\":\"e1\",\"decisions\":[]}]}"
        );
    }

    #[test]
    fn semantic_sort_json_sanitizer_leaves_invalid_output_visible() {
        let raw = "I cannot sort this evidence.";

        assert_eq!(sanitize_json_candidate(raw), raw);
    }

    #[test]
    fn semantic_sort_required_json_reports_an_empty_model_draft() {
        let err = sanitize_required_json_candidate(" \n\t ").expect_err("empty draft must fail");

        assert!(err.to_string().contains("no visible structured output"));
    }

    #[test]
    fn local_json_draft_prompt_uses_gpt_oss_final_json_channel_only_when_needed() {
        let plain = "Return JSON only.";

        assert_eq!(local_json_draft_prompt(plain, false), plain);
        assert!(local_json_draft_prompt(plain, true)
            .ends_with("<|channel|>final <|constrain|>JSON<|message|>"));
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
