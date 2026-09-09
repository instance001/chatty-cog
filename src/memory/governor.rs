use std::collections::{BTreeSet, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::memory::bookkeeper::MemoryHit;
use crate::memory::cold_atlas::ColdAtlasHit;
use crate::memory::semantic_index::{ColdEvidenceRef, ColdEvidenceSourceKind, ColdSemanticHit};

pub const REMEMBER_ATTACHMENTS_JSONL: &str = "remember_attachments.jsonl";
pub const REMEMBER_RECALLS_JSONL: &str = "remember_recalls.jsonl";
pub const REMEMBER_FEEDBACK_JSONL: &str = "remember_feedback.jsonl";

#[derive(Debug, Clone)]
pub struct ColdMemoryQuery {
    pub query: String,
    pub max_candidates: usize,
    pub surrounding_exchanges: usize,
}

impl ColdMemoryQuery {
    pub fn remember_current_input(input: impl Into<String>) -> Self {
        Self {
            query: input.into(),
            max_candidates: 8,
            surrounding_exchanges: 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdInteractionEvidenceQuery {
    pub after_event_index: Option<usize>,
    pub max_items: usize,
}

impl ColdInteractionEvidenceQuery {
    pub fn oldest(max_items: usize) -> Self {
        Self {
            after_event_index: None,
            max_items,
        }
    }

    pub fn latest(max_items: usize) -> Self {
        Self {
            after_event_index: None,
            max_items,
        }
    }

    pub fn after(after_event_index: usize, max_items: usize) -> Self {
        Self {
            after_event_index: Some(after_event_index),
            max_items,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdInteractionEvidence {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub exchange_block: String,
    pub evidence_ref: ColdEvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdMemorySourceRef {
    pub log_path: PathBuf,
    pub matched_event_index: usize,
    pub context_start_event_index: usize,
    pub context_end_event_index: usize,
    pub matched_ts_unix_ms: i64,
    pub context_start_ts_unix_ms: Option<i64>,
    pub context_end_ts_unix_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ColdMemoryCandidate {
    pub id: String,
    pub score: f32,
    pub title: String,
    pub preview: String,
    pub recall_trace: ColdMemoryRecallTrace,
    pub matched_exchange: String,
    pub context_block: String,
    pub provenance: ColdMemorySourceRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdMemoryRecallTrace {
    pub lane: ColdMemoryRecallLane,
    pub reason: String,
    pub bucket_id: Option<String>,
    pub bucket_label: Option<String>,
    pub placement_id: Option<String>,
    #[serde(default)]
    pub atlas: Option<ColdAtlasRecallMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdAtlasRecallMetadata {
    pub atlas_id: String,
    pub title: String,
    pub summary: String,
    pub scope: String,
    pub status: String,
    pub confidence_score: f32,
    pub confidence_label: String,
    pub evidence_index: usize,
}

impl ColdMemoryRecallTrace {
    pub fn bookkeeper() -> Self {
        Self {
            lane: ColdMemoryRecallLane::Bookkeeper,
            reason: "Bookkeeper search matched this cold-log exchange.".to_string(),
            bucket_id: None,
            bucket_label: None,
            placement_id: None,
            atlas: None,
        }
    }

    pub fn semantic(hit: &ColdSemanticHit) -> Self {
        Self {
            lane: ColdMemoryRecallLane::SemanticBucket,
            reason: format!(
                "Semantic bucket '{}' surfaced this placement as {:?} fit: {}",
                hit.bucket_label, hit.placement_fit, hit.placement_reason
            ),
            bucket_id: Some(hit.bucket_id.clone()),
            bucket_label: Some(hit.bucket_label.clone()),
            placement_id: Some(hit.placement_id.clone()),
            atlas: None,
        }
    }

    pub fn atlas(hit: &ColdAtlasHit) -> Self {
        Self {
            lane: ColdMemoryRecallLane::ColdAtlas,
            reason: format!(
                "Cold atlas '{}' surfaced evidence ref {} as broad historical shape memory.",
                hit.title,
                hit.evidence_index + 1
            ),
            bucket_id: Some(hit.atlas_id.clone()),
            bucket_label: Some(hit.title.clone()),
            placement_id: None,
            atlas: Some(ColdAtlasRecallMetadata {
                atlas_id: hit.atlas_id.clone(),
                title: hit.title.clone(),
                summary: hit.summary.clone(),
                scope: hit.scope.clone(),
                status: hit.status.as_str().to_string(),
                confidence_score: hit.confidence.score,
                confidence_label: hit.confidence.label.clone(),
                evidence_index: hit.evidence_index,
            }),
        }
    }

    pub fn ui_label(&self) -> String {
        let lane = self.lane.display_label();
        format!("{lane}: {}", self.reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdMemoryRecallLane {
    Bookkeeper,
    KeywordFallback,
    SemanticBucket,
    ColdAtlas,
}

impl ColdMemoryRecallLane {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bookkeeper => "bookkeeper",
            Self::KeywordFallback => "keyword_fallback",
            Self::SemanticBucket => "semantic_bucket",
            Self::ColdAtlas => "cold_atlas",
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Bookkeeper => "Bookkeeper",
            Self::KeywordFallback => "Keyword fallback",
            Self::SemanticBucket => "Semantic bucket",
            Self::ColdAtlas => "Cold atlas",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ColdMemoryAttachment {
    pub recall_id: Option<String>,
    pub attached_at_unix_ms: i64,
    pub query: String,
    pub candidates: Vec<ColdMemoryCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdMemoryAttachmentReceipt {
    #[serde(default)]
    pub recall_id: Option<String>,
    pub attached_at_unix_ms: i64,
    pub query: String,
    pub candidate_count: usize,
    pub candidates: Vec<ColdMemoryAttachedCandidateReceipt>,
    pub note: String,
}

impl ColdMemoryAttachmentReceipt {
    pub fn from_attachment(attachment: &ColdMemoryAttachment) -> Self {
        Self {
            recall_id: attachment.recall_id.clone(),
            attached_at_unix_ms: attachment.attached_at_unix_ms,
            query: attachment.query.clone(),
            candidate_count: attachment.candidates.len(),
            candidates: attachment
                .candidates
                .iter()
                .map(ColdMemoryAttachedCandidateReceipt::from_candidate)
                .collect(),
            note: "User selected these cold-log evidence refs for one Main AI turn; this receipt is audit metadata only and does not promote memory.".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdMemoryAttachedCandidateReceipt {
    pub candidate_id: String,
    pub title: String,
    pub recall_trace: ColdMemoryRecallTrace,
    pub source_log_path: String,
    pub matched_event_index: usize,
    pub context_start_event_index: usize,
    pub context_end_event_index: usize,
    pub matched_ts_unix_ms: i64,
    pub context_start_ts_unix_ms: Option<i64>,
    pub context_end_ts_unix_ms: Option<i64>,
}

impl ColdMemoryAttachedCandidateReceipt {
    fn from_candidate(candidate: &ColdMemoryCandidate) -> Self {
        Self {
            candidate_id: candidate.id.clone(),
            title: candidate.title.clone(),
            recall_trace: candidate.recall_trace.clone(),
            source_log_path: candidate.provenance.log_path.display().to_string(),
            matched_event_index: candidate.provenance.matched_event_index,
            context_start_event_index: candidate.provenance.context_start_event_index,
            context_end_event_index: candidate.provenance.context_end_event_index,
            matched_ts_unix_ms: candidate.provenance.matched_ts_unix_ms,
            context_start_ts_unix_ms: candidate.provenance.context_start_ts_unix_ms,
            context_end_ts_unix_ms: candidate.provenance.context_end_ts_unix_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdMemoryRecallReceipt {
    #[serde(default)]
    pub recall_id: String,
    pub recalled_at_unix_ms: i64,
    pub query: String,
    pub outcome: ColdMemoryRecallOutcome,
    pub candidate_count: usize,
    pub selected_candidate_count: usize,
    pub candidates: Vec<ColdMemoryAttachedCandidateReceipt>,
    #[serde(default)]
    pub selected_candidate_ids: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdMemoryFeedbackReceipt {
    #[serde(default)]
    pub recall_id: Option<String>,
    pub marked_at_unix_ms: i64,
    pub query: String,
    pub feedback: ColdMemoryCandidateFeedback,
    pub candidate: ColdMemoryAttachedCandidateReceipt,
    pub note: String,
}

impl ColdMemoryFeedbackReceipt {
    pub fn from_candidate(
        recall_id: Option<String>,
        query: impl Into<String>,
        marked_at_unix_ms: i64,
        feedback: ColdMemoryCandidateFeedback,
        candidate: &ColdMemoryCandidate,
    ) -> Self {
        Self {
            recall_id,
            marked_at_unix_ms,
            query: query.into(),
            feedback,
            candidate: ColdMemoryAttachedCandidateReceipt::from_candidate(candidate),
            note: "User marked Remember result quality; this receipt is audit metadata only and does not mutate cold logs, semantic buckets, atlas records, or current context.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdMemoryCandidateFeedback {
    Stale,
    Misleading,
    Noise,
}

impl ColdMemoryCandidateFeedback {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Misleading => "misleading",
            Self::Noise => "noise",
        }
    }

    pub fn display_label(&self) -> &'static str {
        match self {
            Self::Stale => "Stale",
            Self::Misleading => "Misleading",
            Self::Noise => "Noise",
        }
    }
}

impl ColdMemoryRecallReceipt {
    pub fn from_candidates(
        recall_id: impl Into<String>,
        query: impl Into<String>,
        recalled_at_unix_ms: i64,
        outcome: ColdMemoryRecallOutcome,
        candidates: &[ColdMemoryCandidate],
        selected_candidate_ids: &HashSet<String>,
    ) -> Self {
        let mut selected_candidate_ids = selected_candidate_ids
            .iter()
            .filter(|id| candidates.iter().any(|candidate| candidate.id == **id))
            .cloned()
            .collect::<Vec<_>>();
        selected_candidate_ids.sort();
        Self {
            recall_id: recall_id.into(),
            recalled_at_unix_ms,
            query: query.into(),
            outcome,
            candidate_count: candidates.len(),
            selected_candidate_count: selected_candidate_ids.len(),
            candidates: candidates
                .iter()
                .map(ColdMemoryAttachedCandidateReceipt::from_candidate)
                .collect(),
            selected_candidate_ids,
            note: "Remember recall receipt: search and user outcome metadata only; transcript payloads remain in cold logs and are not promoted.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdMemoryRecallOutcome {
    Searched,
    CancelledNoInjection,
    AttachedSelected,
}

impl ColdMemoryRecallOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Searched => "searched",
            Self::CancelledNoInjection => "cancelled_no_injection",
            Self::AttachedSelected => "attached_selected",
        }
    }
}

impl ColdMemoryAttachment {
    pub fn from_selected_ids(
        query: impl Into<String>,
        attached_at_unix_ms: i64,
        candidates: &[ColdMemoryCandidate],
        selected_ids: &HashSet<String>,
    ) -> Option<Self> {
        let selected = candidates
            .iter()
            .filter(|candidate| selected_ids.contains(&candidate.id))
            .cloned()
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return None;
        }

        Some(Self {
            recall_id: None,
            attached_at_unix_ms,
            query: query.into(),
            candidates: selected,
        })
    }

    pub fn prompt_block(&self) -> String {
        render_user_selected_cold_context(&self.candidates)
    }

    pub fn host_prompt_block(&self) -> String {
        render_host_selected_cold_context_attachment(self, 600, 12_000)
    }
}

#[derive(Debug, Clone)]
struct ColdLogEvent {
    index: usize,
    ts_unix_ms: i64,
    cat: String,
    source: String,
    module_id: Option<String>,
    event_type: Option<String>,
    summary: String,
    tags: Vec<String>,
    payload_json: Option<String>,
}

#[derive(Debug, Clone)]
struct ChatExchange {
    start_event_index: usize,
    end_event_index: usize,
    user: Option<ColdLogEvent>,
    assistant: Option<ColdLogEvent>,
}

pub fn query_cold_memory(
    logs_dir: Option<&Path>,
    query: &ColdMemoryQuery,
    ranked_hits: &[MemoryHit],
) -> Result<Vec<ColdMemoryCandidate>> {
    let Some(logs_dir) = logs_dir else {
        return Ok(Vec::new());
    };
    let log_path = logs_dir.join("cold_log.jsonl");
    if !log_path.is_file() {
        return Ok(Vec::new());
    }

    let events = read_cold_log_events(&log_path)?;
    let exchanges = build_chat_exchanges(&events);
    if exchanges.is_empty() {
        return Ok(Vec::new());
    }

    let mut matched_indices = Vec::new();
    for hit in ranked_hits {
        if let Some(index) = find_event_index_for_hit(&events, hit) {
            matched_indices.push((
                index,
                hit.score.max(0.001),
                ColdMemoryRecallTrace::bookkeeper(),
            ));
        }
    }

    if matched_indices.is_empty() {
        matched_indices = keyword_event_matches(&events, &query.query)
            .into_iter()
            .map(|(index, score)| {
                (
                    index,
                    score,
                    ColdMemoryRecallTrace {
                        lane: ColdMemoryRecallLane::KeywordFallback,
                        reason: "Deterministic keyword fallback matched this cold-log exchange."
                            .to_string(),
                        bucket_id: None,
                        bucket_label: None,
                        placement_id: None,
                        atlas: None,
                    },
                )
            })
            .collect();
    }

    let mut seen_contexts = HashSet::new();
    let mut candidates = Vec::new();
    for (matched_event_index, score, recall_trace) in matched_indices {
        let Some(candidate) = build_candidate_from_match(
            &log_path,
            &events,
            &exchanges,
            matched_event_index,
            score,
            query.surrounding_exchanges,
            &mut seen_contexts,
            recall_trace,
        ) else {
            continue;
        };

        candidates.push(candidate);

        if candidates.len() >= query.max_candidates.max(1) {
            break;
        }
    }

    Ok(candidates)
}

pub fn collect_cold_interaction_evidence(
    logs_dir: Option<&Path>,
    query: &ColdInteractionEvidenceQuery,
) -> Result<Vec<ColdInteractionEvidence>> {
    let Some(logs_dir) = logs_dir else {
        return Ok(Vec::new());
    };
    let log_path = logs_dir.join("cold_log.jsonl");
    if !log_path.is_file() {
        return Ok(Vec::new());
    }

    let events = read_cold_log_events(&log_path)?;
    let mut exchanges = build_chat_exchanges(&events)
        .into_iter()
        .filter(|exchange| {
            query
                .after_event_index
                .is_none_or(|after| exchange.end_event_index > after)
        })
        .collect::<Vec<_>>();
    exchanges.sort_by(|left, right| left.start_event_index.cmp(&right.start_event_index));
    let take_count = query.max_items.max(1);
    let skip_count = exchanges.len().saturating_sub(take_count);

    let mut evidence = Vec::new();
    for exchange in exchanges.into_iter().skip(skip_count) {
        let start_ts = exchange_first_ts(&exchange);
        let end_ts = exchange_last_ts(&exchange);
        let mut evidence_ref = ColdEvidenceRef::interaction_log_range(
            "cold_log.jsonl",
            exchange.start_event_index,
            exchange.end_event_index,
        );
        evidence_ref.start_ts_unix_ms = start_ts;
        evidence_ref.end_ts_unix_ms = end_ts;
        let exchange_block = render_exchange(&exchange);
        let title = format!(
            "interaction events {}..={} | {}",
            exchange.start_event_index,
            exchange.end_event_index,
            format_optional_ts_for_payload(start_ts)
        );
        let preview = one_line(&exchange_block, 220);
        let id = format!(
            "interaction:{}:{}",
            exchange.start_event_index, exchange.end_event_index
        );
        evidence.push(ColdInteractionEvidence {
            id,
            title,
            preview,
            exchange_block,
            evidence_ref,
        });
    }

    Ok(evidence)
}

pub fn hydrate_semantic_cold_hits(
    logs_dir: Option<&Path>,
    query: &ColdMemoryQuery,
    semantic_hits: &[ColdSemanticHit],
) -> Result<Vec<ColdMemoryCandidate>> {
    let Some(logs_dir) = logs_dir else {
        return Ok(Vec::new());
    };
    if semantic_hits.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    let mut seen_contexts = HashSet::new();
    for hit in semantic_hits {
        if hit.evidence_ref.source_kind != ColdEvidenceSourceKind::InteractionLog {
            continue;
        }
        let log_path = resolve_cold_log_path(logs_dir, &hit.evidence_ref.source_log_path);
        if !log_path.is_file() {
            continue;
        }
        let events = read_cold_log_events(&log_path)?;
        let exchanges = build_chat_exchanges(&events);
        if exchanges.is_empty() {
            continue;
        }

        let Some(matched_event_index) =
            find_event_index_for_evidence_ref(&events, &hit.evidence_ref)
        else {
            continue;
        };
        let Some(candidate) = build_candidate_from_match(
            &log_path,
            &events,
            &exchanges,
            matched_event_index,
            hit.score.max(0.001),
            query.surrounding_exchanges,
            &mut seen_contexts,
            ColdMemoryRecallTrace::semantic(hit),
        ) else {
            continue;
        };

        candidates.push(candidate);
        if candidates.len() >= query.max_candidates.max(1) {
            break;
        }
    }

    Ok(candidates)
}

pub fn hydrate_atlas_cold_hits(
    logs_dir: Option<&Path>,
    query: &ColdMemoryQuery,
    atlas_hits: &[ColdAtlasHit],
) -> Result<Vec<ColdMemoryCandidate>> {
    let Some(logs_dir) = logs_dir else {
        return Ok(Vec::new());
    };
    if atlas_hits.is_empty() {
        return Ok(Vec::new());
    }

    let mut candidates = Vec::new();
    let mut seen_contexts = HashSet::new();
    for hit in atlas_hits {
        if hit.evidence_ref.source_kind != ColdEvidenceSourceKind::InteractionLog {
            continue;
        }
        let log_path = resolve_cold_log_path(logs_dir, &hit.evidence_ref.source_log_path);
        if !log_path.is_file() {
            continue;
        }
        let events = read_cold_log_events(&log_path)?;
        let exchanges = build_chat_exchanges(&events);
        if exchanges.is_empty() {
            continue;
        }

        let Some(matched_event_index) =
            find_event_index_for_evidence_ref(&events, &hit.evidence_ref)
        else {
            continue;
        };
        let Some(candidate) = build_candidate_from_match(
            &log_path,
            &events,
            &exchanges,
            matched_event_index,
            hit.score.max(0.001),
            query.surrounding_exchanges,
            &mut seen_contexts,
            ColdMemoryRecallTrace::atlas(hit),
        ) else {
            continue;
        };

        candidates.push(candidate);
        if candidates.len() >= query.max_candidates.max(1) {
            break;
        }
    }

    Ok(candidates)
}

pub fn render_recall_route_preview(
    query: &ColdMemoryQuery,
    atlas_hits: &[ColdAtlasHit],
    semantic_hits: &[ColdSemanticHit],
    cold_candidates: &[ColdMemoryCandidate],
    merged_candidates: &[ColdMemoryCandidate],
) -> String {
    let mut out = String::new();
    out.push_str("Recall route preview\n");
    out.push_str("Preview shows what Remember lanes would surface before user selection. It does not attach selected context, promote memory, or persist recall receipts.\n\n");
    out.push_str(&format!("query: {}\n\n", query.query.trim()));

    out.push_str(&format!("Cold atlas shape hits: {}\n", atlas_hits.len()));
    if atlas_hits.is_empty() {
        out.push_str("  - none\n");
    } else {
        for (index, hit) in atlas_hits.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} [{}] score={:.3} status={} evidence_ref={} source={} events {}..{}\n",
                index + 1,
                hit.title,
                hit.atlas_id,
                hit.score,
                hit.status.as_str(),
                hit.evidence_index + 1,
                hit.evidence_ref.source_log_path,
                hit.evidence_ref.start_event_index,
                hit.evidence_ref.end_event_index
            ));
        }
    }

    out.push_str(&format!(
        "\nSemantic bucket evidence hits: {}\n",
        semantic_hits.len()
    ));
    if semantic_hits.is_empty() {
        out.push_str("  - none\n");
    } else {
        for (index, hit) in semantic_hits.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} [{}] score={:.3} kind={} fit={} placement={} source={} events {}..{}\n",
                index + 1,
                hit.bucket_label,
                hit.bucket_id,
                hit.score,
                hit.bucket_kind.as_str(),
                hit.placement_fit.as_str(),
                hit.placement_id,
                hit.evidence_ref.source_log_path,
                hit.evidence_ref.start_event_index,
                hit.evidence_ref.end_event_index
            ));
        }
    }

    out.push_str(&format!(
        "\nCold-log direct candidates: {}\n",
        cold_candidates.len()
    ));
    if cold_candidates.is_empty() {
        out.push_str("  - none\n");
    } else {
        for (index, candidate) in cold_candidates.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} score={:.3} lane={} source={} events {}..{} matched={} preview={}\n",
                index + 1,
                candidate.title,
                candidate.score,
                candidate.recall_trace.lane.as_str(),
                candidate.provenance.log_path.display(),
                candidate.provenance.context_start_event_index,
                candidate.provenance.context_end_event_index,
                candidate.provenance.matched_event_index,
                candidate.preview
            ));
        }
    }

    out.push_str(&format!(
        "\nRemember post-merge candidate order: {}\n",
        merged_candidates.len()
    ));
    if merged_candidates.is_empty() {
        out.push_str("  - none\n");
    } else {
        for (index, candidate) in merged_candidates.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} score={:.3} lane={} source={} events {}..{} matched={} preview={}\n",
                index + 1,
                candidate.title,
                candidate.score,
                candidate.recall_trace.lane.as_str(),
                candidate.provenance.log_path.display(),
                candidate.provenance.context_start_event_index,
                candidate.provenance.context_end_event_index,
                candidate.provenance.matched_event_index,
                candidate.preview
            ));
        }
    }

    out
}

pub fn hydrate_cold_evidence_ref(
    logs_dir: Option<&Path>,
    evidence_ref: &ColdEvidenceRef,
    surrounding_exchanges: usize,
    recall_trace: ColdMemoryRecallTrace,
) -> Result<Option<ColdMemoryCandidate>> {
    let Some(logs_dir) = logs_dir else {
        return Ok(None);
    };
    if evidence_ref.source_kind != ColdEvidenceSourceKind::InteractionLog {
        return Ok(None);
    }

    let log_path = resolve_cold_log_path(logs_dir, &evidence_ref.source_log_path);
    if !log_path.is_file() {
        return Ok(None);
    }

    let events = read_cold_log_events(&log_path)?;
    let exchanges = build_chat_exchanges(&events);
    if exchanges.is_empty() {
        return Ok(None);
    }

    let Some(matched_event_index) = find_event_index_for_evidence_ref(&events, evidence_ref) else {
        return Ok(None);
    };
    let mut seen_contexts = HashSet::new();
    Ok(build_candidate_from_match(
        &log_path,
        &events,
        &exchanges,
        matched_event_index,
        1.0,
        surrounding_exchanges,
        &mut seen_contexts,
        recall_trace,
    ))
}

pub fn render_user_selected_cold_context(candidates: &[ColdMemoryCandidate]) -> String {
    if candidates.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("USER-SELECTED COLD LOG CONTEXT\n");
    out.push_str("================================\n");
    out.push_str("The user explicitly selected these bounded historical transcript blocks. They are cold-log specifics brought forward for the current task, not automatic belief. Treat them as historical evidence, not current truth. Do not promote them into hot memory, Luke Warm memory, or durable memory unless the user explicitly asks.\n\n");
    for (idx, candidate) in candidates.iter().enumerate() {
        out.push_str(&format!("## Selected cold context {}\n", idx + 1));
        out.push_str(&candidate.context_block);
        if !candidate.context_block.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

pub fn render_host_selected_cold_context_attachment(
    attachment: &ColdMemoryAttachment,
    max_query_chars: usize,
    max_context_chars: usize,
) -> String {
    let remember_block = attachment.prompt_block();
    if remember_block.trim().is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("### USER-SELECTED COLD LOG CONTEXT\n");
    out.push_str("The host is injecting this only because the user selected it in Remember. It is historical evidence, not automatically current truth.\n");
    out.push_str(&format!(
        "Remember query: {}\nAttached at unix ms: {}\n\n",
        truncate_chars(&attachment.query, max_query_chars),
        attachment.attached_at_unix_ms
    ));
    out.push_str(&truncate_chars(remember_block.trim(), max_context_chars));
    out.push('\n');
    out
}

pub fn remember_attachment_receipts_path(logs_dir: impl AsRef<Path>) -> PathBuf {
    logs_dir.as_ref().join(REMEMBER_ATTACHMENTS_JSONL)
}

pub fn remember_recall_receipts_path(logs_dir: impl AsRef<Path>) -> PathBuf {
    logs_dir.as_ref().join(REMEMBER_RECALLS_JSONL)
}

pub fn remember_feedback_receipts_path(logs_dir: impl AsRef<Path>) -> PathBuf {
    logs_dir.as_ref().join(REMEMBER_FEEDBACK_JSONL)
}

pub fn append_memory_attachment_receipt_to_store(
    logs_dir: impl AsRef<Path>,
    attachment: &ColdMemoryAttachment,
) -> Result<ColdMemoryAttachmentReceipt> {
    let receipt = ColdMemoryAttachmentReceipt::from_attachment(attachment);
    append_memory_attachment_receipt_jsonl(remember_attachment_receipts_path(logs_dir), &receipt)?;
    Ok(receipt)
}

pub fn append_memory_attachment_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdMemoryAttachmentReceipt,
) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create remember receipt dir {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open remember attachment receipt {}", path.display()))?;
    let json = serde_json::to_string(receipt).context("serialize remember attachment receipt")?;
    writeln!(file, "{json}")
        .with_context(|| format!("append remember attachment receipt {}", path.display()))?;
    Ok(())
}

pub fn read_memory_attachment_receipts_jsonl(
    path: impl AsRef<Path>,
) -> Result<Vec<ColdMemoryAttachmentReceipt>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)
        .with_context(|| format!("open remember attachment receipts {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut receipts = Vec::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "read remember attachment receipt line {} from {}",
                line_number + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let receipt =
            serde_json::from_str::<ColdMemoryAttachmentReceipt>(&line).with_context(|| {
                format!(
                    "parse remember attachment receipt line {} from {}",
                    line_number + 1,
                    path.display()
                )
            })?;
        receipts.push(receipt);
    }
    Ok(receipts)
}

pub fn append_memory_recall_receipt_to_store(
    logs_dir: impl AsRef<Path>,
    recall_id: impl Into<String>,
    query: impl Into<String>,
    recalled_at_unix_ms: i64,
    outcome: ColdMemoryRecallOutcome,
    candidates: &[ColdMemoryCandidate],
    selected_candidate_ids: &HashSet<String>,
) -> Result<ColdMemoryRecallReceipt> {
    let receipt = ColdMemoryRecallReceipt::from_candidates(
        recall_id,
        query,
        recalled_at_unix_ms,
        outcome,
        candidates,
        selected_candidate_ids,
    );
    append_memory_recall_receipt_jsonl(remember_recall_receipts_path(logs_dir), &receipt)?;
    Ok(receipt)
}

pub fn append_memory_recall_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdMemoryRecallReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "remember recall receipt")
}

pub fn read_memory_recall_receipts_jsonl(
    path: impl AsRef<Path>,
) -> Result<Vec<ColdMemoryRecallReceipt>> {
    read_jsonl_records(path, "remember recall receipt")
}

pub fn append_memory_feedback_receipt_to_store(
    logs_dir: impl AsRef<Path>,
    recall_id: Option<String>,
    query: impl Into<String>,
    marked_at_unix_ms: i64,
    feedback: ColdMemoryCandidateFeedback,
    candidate: &ColdMemoryCandidate,
) -> Result<ColdMemoryFeedbackReceipt> {
    let receipt = ColdMemoryFeedbackReceipt::from_candidate(
        recall_id,
        query,
        marked_at_unix_ms,
        feedback,
        candidate,
    );
    append_memory_feedback_receipt_jsonl(remember_feedback_receipts_path(logs_dir), &receipt)?;
    Ok(receipt)
}

pub fn append_memory_feedback_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdMemoryFeedbackReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "remember feedback receipt")
}

pub fn read_memory_feedback_receipts_jsonl(
    path: impl AsRef<Path>,
) -> Result<Vec<ColdMemoryFeedbackReceipt>> {
    read_jsonl_records(path, "remember feedback receipt")
}

pub fn summarize_prior_feedback_for_candidate(
    candidate: &ColdMemoryCandidate,
    receipts: &[ColdMemoryFeedbackReceipt],
) -> Option<String> {
    let mut stale = 0usize;
    let mut misleading = 0usize;
    let mut noise = 0usize;
    for receipt in receipts {
        if !feedback_receipt_matches_candidate(receipt, candidate) {
            continue;
        }
        match receipt.feedback {
            ColdMemoryCandidateFeedback::Stale => stale += 1,
            ColdMemoryCandidateFeedback::Misleading => misleading += 1,
            ColdMemoryCandidateFeedback::Noise => noise += 1,
        }
    }
    if stale == 0 && misleading == 0 && noise == 0 {
        return None;
    }

    let mut parts = Vec::new();
    if stale > 0 {
        parts.push(format!("stale={stale}"));
    }
    if misleading > 0 {
        parts.push(format!("misleading={misleading}"));
    }
    if noise > 0 {
        parts.push(format!("noise={noise}"));
    }
    Some(format!(
        "Prior feedback for this bounded evidence: {}. Review carefully before attaching.",
        parts.join(", ")
    ))
}

fn feedback_receipt_matches_candidate(
    receipt: &ColdMemoryFeedbackReceipt,
    candidate: &ColdMemoryCandidate,
) -> bool {
    receipt.candidate.source_log_path == candidate.provenance.log_path.display().to_string()
        && receipt.candidate.context_start_event_index
            == candidate.provenance.context_start_event_index
        && receipt.candidate.context_end_event_index == candidate.provenance.context_end_event_index
        && receipt.candidate.recall_trace.lane == candidate.recall_trace.lane
}

pub fn render_memory_attachment_receipts_overview(
    receipts: &[ColdMemoryAttachmentReceipt],
) -> String {
    let mut out = String::new();
    out.push_str("Remember attachment receipts\n");
    out.push_str("Receipts are audit metadata only; transcript payloads stay in cold logs and selected one-turn prompt context.\n\n");
    out.push_str(&format!(
        "remember_attachment_receipts: {}\n\n",
        receipts.len()
    ));

    if receipts.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for receipt in receipts.iter().rev() {
        out.push_str(&format!(
            "  - recall_id={} attached_at={} candidates={} query={}\n",
            receipt.recall_id.as_deref().unwrap_or("-"),
            receipt.attached_at_unix_ms,
            receipt.candidate_count,
            truncate_chars(&receipt.query, 160)
        ));
        for candidate in &receipt.candidates {
            out.push_str(&format!(
                "      * {} lane={} source={} events {}..={} matched={} bucket={} placement={}\n",
                truncate_chars(&candidate.candidate_id, 96),
                candidate.recall_trace.lane.as_str(),
                candidate.source_log_path,
                candidate.context_start_event_index,
                candidate.context_end_event_index,
                candidate.matched_event_index,
                candidate.recall_trace.bucket_id.as_deref().unwrap_or("-"),
                candidate
                    .recall_trace
                    .placement_id
                    .as_deref()
                    .map(|id| truncate_chars(id, 96))
                    .unwrap_or_else(|| "-".to_string())
            ));
        }
    }

    out
}

pub fn render_memory_recall_receipts_overview(receipts: &[ColdMemoryRecallReceipt]) -> String {
    let mut out = String::new();
    out.push_str("Remember recall receipts\n");
    out.push_str("Recall receipts record search/outcome metadata only; they do not attach or promote memory.\n\n");
    out.push_str(&format!("remember_recall_receipts: {}\n\n", receipts.len()));

    if receipts.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for receipt in receipts.iter().rev() {
        out.push_str(&format!(
            "  - recall_id={} recalled_at={} outcome={} candidates={} selected={} query={}\n",
            receipt.recall_id,
            receipt.recalled_at_unix_ms,
            receipt.outcome.as_str(),
            receipt.candidate_count,
            receipt.selected_candidate_count,
            truncate_chars(&receipt.query, 160)
        ));
        for candidate in &receipt.candidates {
            let selected = receipt
                .selected_candidate_ids
                .iter()
                .any(|id| id == &candidate.candidate_id);
            out.push_str(&format!(
                "      * {} selected={} lane={} source={} events {}..={} matched={} bucket={}\n",
                truncate_chars(&candidate.candidate_id, 96),
                selected,
                candidate.recall_trace.lane.as_str(),
                candidate.source_log_path,
                candidate.context_start_event_index,
                candidate.context_end_event_index,
                candidate.matched_event_index,
                candidate.recall_trace.bucket_id.as_deref().unwrap_or("-")
            ));
        }
    }

    out
}

pub fn render_memory_feedback_receipts_overview(receipts: &[ColdMemoryFeedbackReceipt]) -> String {
    let mut out = String::new();
    out.push_str("Remember feedback receipts\n");
    out.push_str("Feedback marks recall quality only; it does not rewrite cold logs, buckets, atlas records, or current context.\n\n");
    out.push_str(&format!(
        "remember_feedback_receipts: {}\n\n",
        receipts.len()
    ));

    if receipts.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for receipt in receipts.iter().rev() {
        out.push_str(&format!(
            "  - recall_id={} feedback={} at={} candidate={} lane={} source={} events {}..={} query={}\n",
            receipt.recall_id.as_deref().unwrap_or("-"),
            receipt.feedback.as_str(),
            receipt.marked_at_unix_ms,
            receipt.candidate.candidate_id,
            receipt.candidate.recall_trace.lane.as_str(),
            receipt.candidate.source_log_path,
            receipt.candidate.context_start_event_index,
            receipt.candidate.context_end_event_index,
            one_line(&receipt.query, 140)
        ));
    }

    out
}

pub fn render_memory_remember_session_receipts_overview(
    recall_receipts: &[ColdMemoryRecallReceipt],
    attachment_receipts: &[ColdMemoryAttachmentReceipt],
) -> String {
    let mut recall_ids = BTreeSet::new();
    for receipt in recall_receipts {
        recall_ids.insert(if receipt.recall_id.trim().is_empty() {
            "-".to_string()
        } else {
            receipt.recall_id.clone()
        });
    }
    for receipt in attachment_receipts {
        recall_ids.insert(
            receipt
                .recall_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or("-")
                .to_string(),
        );
    }

    let mut out = String::new();
    out.push_str("Remember session receipts\n");
    out.push_str("Grouped by recall_id. These records audit deliberate recall and user outcomes; transcript payloads stay in cold logs.\n\n");
    out.push_str(&format!(
        "remember_sessions: {}\nremember_recall_receipts: {}\nremember_attachment_receipts: {}\n\n",
        recall_ids.len(),
        recall_receipts.len(),
        attachment_receipts.len()
    ));

    if recall_ids.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for recall_id in recall_ids.iter().rev() {
        let recalls = recall_receipts
            .iter()
            .filter(|receipt| normalized_recall_id(&receipt.recall_id) == *recall_id)
            .collect::<Vec<_>>();
        let attachments = attachment_receipts
            .iter()
            .filter(|receipt| {
                normalized_optional_recall_id(receipt.recall_id.as_deref()) == *recall_id
            })
            .collect::<Vec<_>>();
        out.push_str(&format!(
            "  - recall_id={} recall_events={} attachment_events={}\n",
            recall_id,
            recalls.len(),
            attachments.len()
        ));
        for receipt in recalls {
            out.push_str(&format!(
                "      recall outcome={} at={} candidates={} selected={} query={}\n",
                receipt.outcome.as_str(),
                receipt.recalled_at_unix_ms,
                receipt.candidate_count,
                receipt.selected_candidate_count,
                truncate_chars(&receipt.query, 160)
            ));
        }
        for receipt in attachments {
            out.push_str(&format!(
                "      attachment at={} candidates={} query={}\n",
                receipt.attached_at_unix_ms,
                receipt.candidate_count,
                truncate_chars(&receipt.query, 160)
            ));
            for candidate in &receipt.candidates {
                out.push_str(&format!(
                    "          * {} lane={} source={} events {}..={} matched={} bucket={} placement={}\n",
                    truncate_chars(&candidate.candidate_id, 96),
                    candidate.recall_trace.lane.as_str(),
                    candidate.source_log_path,
                    candidate.context_start_event_index,
                    candidate.context_end_event_index,
                    candidate.matched_event_index,
                    candidate.recall_trace.bucket_id.as_deref().unwrap_or("-"),
                    candidate
                        .recall_trace
                        .placement_id
                        .as_deref()
                        .map(|id| truncate_chars(id, 96))
                        .unwrap_or_else(|| "-".to_string())
                ));
            }
        }
    }

    out
}

fn normalized_recall_id(recall_id: &str) -> String {
    if recall_id.trim().is_empty() {
        "-".to_string()
    } else {
        recall_id.to_string()
    }
}

fn normalized_optional_recall_id(recall_id: Option<&str>) -> String {
    recall_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or("-")
        .to_string()
}

fn append_jsonl_record<T>(path: impl AsRef<Path>, record: &T, record_label: &str) -> Result<()>
where
    T: Serialize,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {record_label} dir {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {record_label} {}", path.display()))?;
    let json =
        serde_json::to_string(record).with_context(|| format!("serialize {record_label}"))?;
    writeln!(file, "{json}")
        .with_context(|| format!("append {record_label} {}", path.display()))?;
    Ok(())
}

fn read_jsonl_records<T>(path: impl AsRef<Path>, record_label: &str) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file =
        File::open(path).with_context(|| format!("open {record_label}s {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "read {record_label} line {} from {}",
                line_number + 1,
                path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<T>(&line).with_context(|| {
            format!(
                "parse {record_label} line {} from {}",
                line_number + 1,
                path.display()
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    let mut out = text.chars().take(keep).collect::<String>();
    out.push_str("...");
    out
}

fn read_cold_log_events(log_path: &Path) -> Result<Vec<ColdLogEvent>> {
    let file = File::open(log_path).with_context(|| format!("open {}", log_path.display()))?;
    let mut events = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("read {}", log_path.display()))?;
        if let Some(event) = parse_cold_log_event(index, &line) {
            events.push(event);
        }
    }
    Ok(events)
}

fn parse_cold_log_event(index: usize, line: &str) -> Option<ColdLogEvent> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let get_str = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let ts_unix_ms = value.get("ts").and_then(|v| v.as_i64()).unwrap_or_default();
    let cat = get_str("cat").unwrap_or_else(|| "chat".to_string());
    let source = get_str("source")?;
    let module_id = get_str("module_id")
        .or_else(|| get_str("module"))
        .and_then(non_empty);
    let event_type = get_str("event_type")
        .or_else(|| get_str("type"))
        .and_then(non_empty);
    let summary = get_str("summary").or_else(|| get_str("text"))?;
    let tags = value
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let payload_json = get_str("payload_json").and_then(non_empty);

    Some(ColdLogEvent {
        index,
        ts_unix_ms,
        cat,
        source,
        module_id,
        event_type,
        summary,
        tags,
        payload_json,
    })
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn build_chat_exchanges(events: &[ColdLogEvent]) -> Vec<ChatExchange> {
    let mut exchanges = Vec::new();
    let mut pending_user: Option<ColdLogEvent> = None;

    for event in events.iter().filter(|event| is_chat_message(event)) {
        if event.source.eq_ignore_ascii_case("user") {
            if let Some(user) = pending_user.take() {
                exchanges.push(ChatExchange {
                    start_event_index: user.index,
                    end_event_index: user.index,
                    user: Some(user),
                    assistant: None,
                });
            }
            pending_user = Some(event.clone());
        } else if event.source.eq_ignore_ascii_case("assistant") {
            if let Some(user) = pending_user.take() {
                exchanges.push(ChatExchange {
                    start_event_index: user.index,
                    end_event_index: event.index,
                    user: Some(user),
                    assistant: Some(event.clone()),
                });
            } else {
                exchanges.push(ChatExchange {
                    start_event_index: event.index,
                    end_event_index: event.index,
                    user: None,
                    assistant: Some(event.clone()),
                });
            }
        }
    }

    if let Some(user) = pending_user {
        exchanges.push(ChatExchange {
            start_event_index: user.index,
            end_event_index: user.index,
            user: Some(user),
            assistant: None,
        });
    }

    exchanges
}

fn is_chat_message(event: &ColdLogEvent) -> bool {
    event.cat == "chat"
        && event.module_id.is_none()
        && event.event_type.as_deref().unwrap_or_default() == "message"
        && (event.source.eq_ignore_ascii_case("user")
            || event.source.eq_ignore_ascii_case("assistant"))
}

fn find_event_index_for_hit(events: &[ColdLogEvent], hit: &MemoryHit) -> Option<usize> {
    events
        .iter()
        .find(|event| {
            event.ts_unix_ms == hit.ts_unix_ms
                && event.source == hit.source
                && event.module_id == hit.module
                && event.event_type == hit.event_type
                && event.summary == hit.text
        })
        .map(|event| event.index)
        .or_else(|| {
            events
                .iter()
                .find(|event| {
                    event.source == hit.source
                        && event.module_id == hit.module
                        && event.event_type == hit.event_type
                        && event.summary == hit.text
                })
                .map(|event| event.index)
        })
}

fn keyword_event_matches(events: &[ColdLogEvent], query: &str) -> Vec<(usize, f32)> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for event in events.iter().filter(|event| is_chat_message(event)) {
        let hay = searchable_event_text(event);
        let mut matched = 0usize;
        for term in &terms {
            if hay.contains(term) {
                matched += 1;
            }
        }
        if matched > 0 {
            out.push((event.index, matched as f32 / terms.len().max(1) as f32));
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn build_candidate_from_match(
    log_path: &Path,
    events: &[ColdLogEvent],
    exchanges: &[ChatExchange],
    matched_event_index: usize,
    score: f32,
    surrounding_exchanges: usize,
    seen_contexts: &mut HashSet<(PathBuf, usize, usize)>,
    recall_trace: ColdMemoryRecallTrace,
) -> Option<ColdMemoryCandidate> {
    let matched_event = events
        .iter()
        .find(|event| event.index == matched_event_index)?;
    let exchange_index = exchanges.iter().position(|exchange| {
        matched_event_index >= exchange.start_event_index
            && matched_event_index <= exchange.end_event_index
    })?;

    let start_exchange = exchange_index.saturating_sub(surrounding_exchanges);
    let end_exchange =
        (exchange_index + surrounding_exchanges).min(exchanges.len().saturating_sub(1));
    let context_start_event_index = exchanges[start_exchange].start_event_index;
    let context_end_event_index = exchanges[end_exchange].end_event_index;
    let context_start_ts_unix_ms = exchange_first_ts(&exchanges[start_exchange]);
    let context_end_ts_unix_ms = exchange_last_ts(&exchanges[end_exchange]);
    if !seen_contexts.insert((
        log_path.to_path_buf(),
        context_start_event_index,
        context_end_event_index,
    )) {
        return None;
    }

    let context_exchanges = &exchanges[start_exchange..=end_exchange];
    let matched_exchange = render_exchange(&exchanges[exchange_index]);
    let mut context_block = render_context_block(
        log_path,
        matched_event_index,
        context_start_event_index,
        context_end_event_index,
        matched_event.ts_unix_ms,
        context_start_ts_unix_ms,
        context_end_ts_unix_ms,
        context_exchanges,
    );
    let mut title = format_candidate_title(matched_event);
    let mut id = format!(
        "cold:{}:{}:{}",
        matched_event_index, context_start_event_index, context_end_event_index
    );
    if recall_trace.lane == ColdMemoryRecallLane::SemanticBucket {
        let bucket_label = recall_trace
            .bucket_label
            .as_deref()
            .unwrap_or("unknown bucket");
        title = format!("{} | semantic bucket: {}", title, bucket_label);
        id = format!(
            "semantic:{}:{}:{}:{}",
            recall_trace.bucket_id.as_deref().unwrap_or("unknown"),
            matched_event_index,
            context_start_event_index,
            context_end_event_index
        );
        context_block = format!(
            "Recall provenance: {}\nBucket id: {}\nPlacement id: {}\n\n{}",
            recall_trace.reason,
            recall_trace.bucket_id.as_deref().unwrap_or("unknown"),
            recall_trace.placement_id.as_deref().unwrap_or("unknown"),
            context_block
        );
    } else if recall_trace.lane == ColdMemoryRecallLane::ColdAtlas {
        let atlas_title = recall_trace
            .bucket_label
            .as_deref()
            .unwrap_or("unknown atlas record");
        title = format!("{} | cold atlas: {}", title, atlas_title);
        id = format!(
            "atlas:{}:{}:{}:{}",
            recall_trace.bucket_id.as_deref().unwrap_or("unknown"),
            matched_event_index,
            context_start_event_index,
            context_end_event_index
        );
        context_block = format!(
            "Recall provenance: {}\nAtlas id: {}\n\n{}",
            recall_trace.reason,
            recall_trace.bucket_id.as_deref().unwrap_or("unknown"),
            context_block
        );
    } else {
        context_block = format!(
            "Recall provenance: {}\n\n{}",
            recall_trace.reason, context_block
        );
    }
    let preview = one_line(&matched_exchange, 220);

    Some(ColdMemoryCandidate {
        id,
        score,
        title,
        preview,
        recall_trace,
        matched_exchange,
        context_block,
        provenance: ColdMemorySourceRef {
            log_path: log_path.to_path_buf(),
            matched_event_index,
            context_start_event_index,
            context_end_event_index,
            matched_ts_unix_ms: matched_event.ts_unix_ms,
            context_start_ts_unix_ms,
            context_end_ts_unix_ms,
        },
    })
}

fn resolve_cold_log_path(logs_dir: &Path, source_log_path: &str) -> PathBuf {
    let source = PathBuf::from(source_log_path);
    if source.is_absolute() {
        return source;
    }

    let direct = logs_dir.join(&source);
    if direct.is_file() {
        return direct;
    }

    if source
        .file_name()
        .is_some_and(|name| name == "cold_log.jsonl")
    {
        return logs_dir.join("cold_log.jsonl");
    }

    direct
}

fn find_event_index_for_evidence_ref(
    events: &[ColdLogEvent],
    evidence_ref: &ColdEvidenceRef,
) -> Option<usize> {
    events
        .iter()
        .filter(|event| {
            event.index >= evidence_ref.start_event_index
                && event.index <= evidence_ref.end_event_index
                && is_chat_message(event)
        })
        .max_by_key(|event| event.ts_unix_ms)
        .map(|event| event.index)
        .or_else(|| {
            events
                .iter()
                .find(|event| event.index == evidence_ref.start_event_index)
                .map(|event| event.index)
        })
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = BTreeSet::new();
    for raw in query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|term| term.trim().to_ascii_lowercase())
    {
        if raw.len() >= 4 {
            terms.insert(raw);
        }
    }
    terms.into_iter().take(24).collect()
}

fn searchable_event_text(event: &ColdLogEvent) -> String {
    [
        event.source.as_str(),
        event.module_id.as_deref().unwrap_or_default(),
        event.event_type.as_deref().unwrap_or_default(),
        event.summary.as_str(),
        &event.tags.join(" "),
        event.payload_json.as_deref().unwrap_or_default(),
    ]
    .join("\n")
    .to_ascii_lowercase()
}

fn render_context_block(
    log_path: &Path,
    matched_event_index: usize,
    context_start_event_index: usize,
    context_end_event_index: usize,
    matched_ts_unix_ms: i64,
    context_start_ts_unix_ms: Option<i64>,
    context_end_ts_unix_ms: Option<i64>,
    exchanges: &[ChatExchange],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Source: {}\nMatched event index: {}\nMatched timestamp: {}\nContext event range: {}..={}\nContext timestamp range: {} to {}\n\n",
        log_path.display(),
        matched_event_index,
        format_unix_ms_for_payload(matched_ts_unix_ms),
        context_start_event_index,
        context_end_event_index,
        format_optional_ts_for_payload(context_start_ts_unix_ms),
        format_optional_ts_for_payload(context_end_ts_unix_ms)
    ));
    for (idx, exchange) in exchanges.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&render_exchange(exchange));
    }
    out
}

fn format_optional_ts_for_payload(ts: Option<i64>) -> String {
    ts.map(format_unix_ms_for_payload)
        .unwrap_or_else(|| "unknown".to_string())
}

fn exchange_first_ts(exchange: &ChatExchange) -> Option<i64> {
    exchange
        .user
        .as_ref()
        .or(exchange.assistant.as_ref())
        .map(|event| event.ts_unix_ms)
        .filter(|ts| *ts > 0)
}

fn exchange_last_ts(exchange: &ChatExchange) -> Option<i64> {
    exchange
        .assistant
        .as_ref()
        .or(exchange.user.as_ref())
        .map(|event| event.ts_unix_ms)
        .filter(|ts| *ts > 0)
}

fn render_exchange(exchange: &ChatExchange) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "[exchange events {}..={}]\n",
        exchange.start_event_index, exchange.end_event_index
    ));
    if let Some(user) = &exchange.user {
        out.push_str(&format_event_for_prompt("User", user));
    }
    if let Some(assistant) = &exchange.assistant {
        out.push_str(&format_event_for_prompt("Assistant", assistant));
    }
    out.trim_end().to_string()
}

fn format_event_for_prompt(role: &str, event: &ColdLogEvent) -> String {
    format!(
        "{} [ts={} source={} event_index={}]:\n{}\n",
        role,
        event.ts_unix_ms,
        event.source,
        event.index,
        event.summary.trim()
    )
}

fn format_candidate_title(event: &ColdLogEvent) -> String {
    format!(
        "{} | {} | event {}",
        format_unix_ms_for_title(event.ts_unix_ms),
        event.source,
        event.index
    )
}

fn format_unix_ms_for_title(ts_unix_ms: i64) -> String {
    if ts_unix_ms <= 0 {
        return "unknown time".to_string();
    }
    format_unix_ms_utc(ts_unix_ms)
}

fn format_unix_ms_for_payload(ts_unix_ms: i64) -> String {
    if ts_unix_ms <= 0 {
        return "unknown".to_string();
    }
    format!(
        "{} (unix ms {})",
        format_unix_ms_utc(ts_unix_ms),
        ts_unix_ms
    )
}

fn format_unix_ms_utc(ts_unix_ms: i64) -> String {
    let seconds = ts_unix_ms.div_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096).div_euclid(365);
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2).div_euclid(153);
    let d = doy - (153 * mp + 2).div_euclid(5) + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut out = collapsed.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::bookkeeper::MemoryHit;
    use crate::memory::cold_atlas::{ColdAtlasConfidence, ColdAtlasHit, ColdAtlasRecordStatus};
    use crate::memory::semantic_index::{
        ColdBucketFit, ColdBucketPlacementAuthor, ColdBucketPlacementDecision, ColdEvidenceRef,
        ColdEvidenceSourceKind, ColdSemanticBucketKind, ColdSemanticIndexPaths, ColdSemanticQuery,
        append_semantic_placement_proposals_to_store, apply_placement_review_decisions_to_store,
        default_interaction_bucket_map, query_semantic_index_store, read_placements_jsonl,
        write_bucket_map_json,
    };

    fn write_test_log(lines: &[&str]) -> tempfile_path::TempLog {
        let temp = tempfile_path::TempLog::new();
        std::fs::create_dir_all(&temp.dir).unwrap();
        std::fs::write(temp.dir.join("cold_log.jsonl"), lines.join("\n")).unwrap();
        temp
    }

    mod tempfile_path {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicUsize, Ordering};

        static TEMP_LOG_COUNTER: AtomicUsize = AtomicUsize::new(0);

        pub struct TempLog {
            pub dir: PathBuf,
        }

        impl TempLog {
            pub fn new() -> Self {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock should be after unix epoch")
                    .as_nanos();
                let counter = TEMP_LOG_COUNTER.fetch_add(1, Ordering::Relaxed);
                let unique = format!(
                    "chattycog-cold-governor-test-{}-{}-{}",
                    std::process::id(),
                    nonce,
                    counter
                );
                Self {
                    dir: std::env::temp_dir().join(unique),
                }
            }
        }

        impl Drop for TempLog {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }

    fn line(ts: i64, source: &str, text: &str) -> String {
        serde_json::json!({
            "ts": ts,
            "kind": "cold",
            "cat": "chat",
            "source": source,
            "module_id": "",
            "event_type": "message",
            "summary": text,
            "tags": [],
            "entities": [],
            "payload_json": "",
            "emb_len": 0
        })
        .to_string()
    }

    #[test]
    fn extracts_previous_matched_and_following_exchange() {
        let lines = vec![
            line(1, "user", "first user"),
            line(2, "assistant", "first assistant"),
            line(3, "user", "needle user"),
            line(4, "assistant", "needle assistant"),
            line(5, "user", "third user"),
            line(6, "assistant", "third assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let hit = MemoryHit {
            score: 0.9,
            source: "assistant".to_string(),
            module: None,
            event_type: Some("message".to_string()),
            text: "needle assistant".to_string(),
            tags: Vec::new(),
            payload_json: None,
            ts_unix_ms: 4,
        };

        let candidates = query_cold_memory(
            Some(&temp.dir),
            &ColdMemoryQuery {
                query: "needle".to_string(),
                max_candidates: 4,
                surrounding_exchanges: 1,
            },
            &[hit],
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        let block = &candidates[0].context_block;
        assert!(block.contains("first user"));
        assert!(block.contains("needle assistant"));
        assert!(block.contains("third assistant"));
        assert_eq!(candidates[0].provenance.matched_event_index, 3);
        assert_eq!(candidates[0].provenance.context_start_event_index, 0);
        assert_eq!(candidates[0].provenance.context_end_event_index, 5);
        assert_eq!(candidates[0].provenance.matched_ts_unix_ms, 4);
        assert_eq!(candidates[0].provenance.context_start_ts_unix_ms, Some(1));
        assert_eq!(candidates[0].provenance.context_end_ts_unix_ms, Some(6));
    }

    #[test]
    fn remember_v1_ignores_non_interaction_log_events() {
        let module_line = serde_json::json!({
            "ts": 10,
            "cat": "module",
            "source": "module",
            "module_id": "sandbox",
            "event_type": "message",
            "summary": "needle from module event",
            "tags": [],
            "payload_json": ""
        })
        .to_string();
        let sandbox_line = serde_json::json!({
            "ts": 11,
            "cat": "chat",
            "source": "user",
            "module_id": "sandbox",
            "event_type": "message",
            "summary": "needle from scoped sandbox chat",
            "tags": [],
            "payload_json": ""
        })
        .to_string();
        let lines = vec![
            module_line,
            sandbox_line,
            line(12, "user", "ordinary needle user"),
            line(13, "assistant", "ordinary needle assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);

        let candidates = query_cold_memory(
            Some(&temp.dir),
            &ColdMemoryQuery {
                query: "needle".to_string(),
                max_candidates: 8,
                surrounding_exchanges: 1,
            },
            &[],
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].context_block.contains("ordinary needle user"));
        assert!(!candidates[0].context_block.contains("module event"));
        assert!(!candidates[0].context_block.contains("scoped sandbox"));
    }

    #[test]
    fn collects_interaction_evidence_refs_without_module_events() {
        let module_line = serde_json::json!({
            "ts": 10,
            "cat": "module",
            "source": "module",
            "module_id": "sandbox",
            "event_type": "message",
            "summary": "module filing input",
            "tags": [],
            "payload_json": ""
        })
        .to_string();
        let lines = vec![
            module_line,
            line(11, "user", "first filing user"),
            line(12, "assistant", "first filing assistant"),
            line(13, "user", "second filing user"),
            line(14, "assistant", "second filing assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);

        let evidence = collect_cold_interaction_evidence(
            Some(&temp.dir),
            &ColdInteractionEvidenceQuery::latest(8),
        )
        .unwrap();

        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0].evidence_ref.source_log_path, "cold_log.jsonl");
        assert_eq!(
            evidence[0].evidence_ref.source_kind,
            ColdEvidenceSourceKind::InteractionLog
        );
        assert_eq!(evidence[0].evidence_ref.start_event_index, 1);
        assert_eq!(evidence[0].evidence_ref.end_event_index, 2);
        assert_eq!(evidence[0].evidence_ref.start_ts_unix_ms, Some(11));
        assert_eq!(evidence[0].evidence_ref.end_ts_unix_ms, Some(12));
        assert!(evidence[0].exchange_block.contains("first filing user"));
        assert!(!evidence[0].exchange_block.contains("module filing input"));
    }

    #[test]
    fn collects_interaction_evidence_after_event_index_for_incremental_sorting() {
        let lines = vec![
            line(1, "user", "old user"),
            line(2, "assistant", "old assistant"),
            line(3, "user", "new user"),
            line(4, "assistant", "new assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);

        let evidence = collect_cold_interaction_evidence(
            Some(&temp.dir),
            &ColdInteractionEvidenceQuery::after(1, 8),
        )
        .unwrap();

        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_ref.start_event_index, 2);

        let evidence = collect_cold_interaction_evidence(
            Some(&temp.dir),
            &ColdInteractionEvidenceQuery::after(2, 8),
        )
        .unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_ref.start_event_index, 2);
    }

    #[test]
    fn collected_interaction_evidence_can_be_hydrated_after_semantic_placement() {
        let lines = vec![
            line(1, "user", "miso filing user"),
            line(2, "assistant", "miso filing assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let evidence = collect_cold_interaction_evidence(
            Some(&temp.dir),
            &ColdInteractionEvidenceQuery::latest(1),
        )
        .unwrap();
        let hit = ColdSemanticHit {
            score: 1.0,
            bucket_id: "entity:miso".to_string(),
            bucket_label: "Miso".to_string(),
            bucket_kind: ColdSemanticBucketKind::Positive,
            placement_id: "placement:entity:miso:test".to_string(),
            placement_fit: ColdBucketFit::Strong,
            placement_reason: "Sorter filed this as Miso context.".to_string(),
            evidence_ref: evidence[0].evidence_ref.clone(),
        };

        let candidates = hydrate_semantic_cold_hits(
            Some(&temp.dir),
            &ColdMemoryQuery::remember_current_input("miso"),
            &[hit],
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(
            candidates[0]
                .context_block
                .contains("miso filing assistant")
        );
    }

    #[test]
    fn evidence_ref_hydration_extracts_bounded_context_for_review_inspection() {
        let lines = vec![
            line(1, "user", "previous user"),
            line(2, "assistant", "previous assistant"),
            line(3, "user", "miso review user"),
            line(4, "assistant", "miso review assistant"),
            line(5, "user", "following user"),
            line(6, "assistant", "following assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let evidence_ref = ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 2, 3);

        let candidate = hydrate_cold_evidence_ref(
            Some(&temp.dir),
            &evidence_ref,
            1,
            ColdMemoryRecallTrace {
                lane: ColdMemoryRecallLane::SemanticBucket,
                reason: "Placement review inspection only; no approval or injection.".to_string(),
                bucket_id: Some("entity:miso".to_string()),
                bucket_label: Some("Miso".to_string()),
                placement_id: Some("placement:entity:miso:test".to_string()),
                atlas: None,
            },
        )
        .unwrap()
        .expect("evidence ref should hydrate");

        assert!(candidate.context_block.contains("previous assistant"));
        assert!(candidate.context_block.contains("miso review assistant"));
        assert!(candidate.context_block.contains("following assistant"));
        assert!(
            candidate
                .context_block
                .contains("Placement review inspection only")
        );
        assert_eq!(candidate.provenance.context_start_event_index, 0);
        assert_eq!(candidate.provenance.context_end_event_index, 5);
    }

    #[test]
    fn semantic_hits_hydrate_bounded_cold_log_candidates() {
        let lines = vec![
            line(1, "user", "first user"),
            line(2, "assistant", "first assistant"),
            line(3, "user", "miso user"),
            line(4, "assistant", "miso assistant"),
            line(5, "user", "third user"),
            line(6, "assistant", "third assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let hit = ColdSemanticHit {
            score: 1.2,
            bucket_id: "entity:miso".to_string(),
            bucket_label: "Miso".to_string(),
            bucket_kind: ColdSemanticBucketKind::Positive,
            placement_id: "placement:entity:miso:test".to_string(),
            placement_fit: ColdBucketFit::Strong,
            placement_reason: "Miso care details.".to_string(),
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 2, 3),
        };

        let candidates = hydrate_semantic_cold_hits(
            Some(&temp.dir),
            &ColdMemoryQuery {
                query: "miso".to_string(),
                max_candidates: 4,
                surrounding_exchanges: 1,
            },
            &[hit],
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].id.starts_with("semantic:entity:miso"));
        assert!(candidates[0].title.contains("semantic bucket: Miso"));
        assert!(
            candidates[0]
                .context_block
                .contains("Recall provenance: Semantic bucket 'Miso'")
        );
        assert_eq!(
            candidates[0].recall_trace.lane,
            ColdMemoryRecallLane::SemanticBucket
        );
        assert_eq!(
            candidates[0].recall_trace.bucket_id.as_deref(),
            Some("entity:miso")
        );
        assert!(
            candidates[0]
                .recall_trace
                .ui_label()
                .contains("Miso care details")
        );
        assert!(candidates[0].context_block.contains("first user"));
        assert!(candidates[0].context_block.contains("miso assistant"));
        assert!(candidates[0].context_block.contains("third assistant"));
        assert_eq!(candidates[0].provenance.matched_event_index, 3);
        assert_eq!(candidates[0].provenance.context_start_event_index, 0);
        assert_eq!(candidates[0].provenance.context_end_event_index, 5);
    }

    #[test]
    fn atlas_hits_hydrate_bounded_cold_log_candidates() {
        let lines = vec![
            line(1, "user", "first user"),
            line(2, "assistant", "first assistant"),
            line(3, "user", "miso user"),
            line(4, "assistant", "miso assistant"),
            line(5, "user", "third user"),
            line(6, "assistant", "third assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let hit = ColdAtlasHit {
            score: 1.4,
            atlas_id: "atlas:miso:v1".to_string(),
            title: "Miso care shape".to_string(),
            summary: "Miso recall often concerns care routines and anxiety.".to_string(),
            scope: "pet care".to_string(),
            status: ColdAtlasRecordStatus::HistoricalShape,
            confidence: ColdAtlasConfidence::new(0.8, "supported"),
            evidence_index: 0,
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 2, 3),
        };

        let candidates = hydrate_atlas_cold_hits(
            Some(&temp.dir),
            &ColdMemoryQuery {
                query: "miso".to_string(),
                max_candidates: 4,
                surrounding_exchanges: 1,
            },
            &[hit],
        )
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].id.starts_with("atlas:atlas:miso:v1"));
        assert!(candidates[0].title.contains("cold atlas: Miso care shape"));
        assert!(
            candidates[0]
                .context_block
                .contains("Recall provenance: Cold atlas 'Miso care shape'")
        );
        assert!(
            !candidates[0]
                .context_block
                .contains("Miso recall often concerns care routines and anxiety.")
        );
        assert_eq!(
            candidates[0].recall_trace.lane,
            ColdMemoryRecallLane::ColdAtlas
        );
        assert_eq!(
            candidates[0].recall_trace.bucket_id.as_deref(),
            Some("atlas:miso:v1")
        );
        let atlas = candidates[0]
            .recall_trace
            .atlas
            .as_ref()
            .expect("atlas recall should retain orientation metadata");
        assert_eq!(atlas.title, "Miso care shape");
        assert_eq!(
            atlas.summary,
            "Miso recall often concerns care routines and anxiety."
        );
        assert_eq!(atlas.status, "historical_shape");
        assert_eq!(atlas.confidence_label, "supported");
        assert!(candidates[0].context_block.contains("first user"));
        assert!(candidates[0].context_block.contains("miso assistant"));
        assert!(candidates[0].context_block.contains("third assistant"));
        assert_eq!(candidates[0].provenance.matched_event_index, 3);
    }

    #[test]
    fn semantic_hydration_ignores_non_interaction_evidence_refs() {
        let lines = vec![
            line(1, "user", "miso user"),
            line(2, "assistant", "miso assistant"),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let mut evidence_ref = ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 0, 1);
        evidence_ref.source_kind = ColdEvidenceSourceKind::SandboxEvent;
        let hit = ColdSemanticHit {
            score: 1.2,
            bucket_id: "entity:miso".to_string(),
            bucket_label: "Miso".to_string(),
            bucket_kind: ColdSemanticBucketKind::Positive,
            placement_id: "placement:entity:miso:test".to_string(),
            placement_fit: ColdBucketFit::Strong,
            placement_reason: "Miso care details.".to_string(),
            evidence_ref,
        };

        let candidates = hydrate_semantic_cold_hits(
            Some(&temp.dir),
            &ColdMemoryQuery::remember_current_input("miso"),
            &[hit],
        )
        .unwrap();

        assert!(candidates.is_empty());
    }

    #[test]
    fn approved_semantic_memory_round_trip_hydrates_exact_user_selected_context() {
        let lines = vec![
            line(
                1,
                "user",
                "I prefer concise implementation notes before broad theory.",
            ),
            line(
                2,
                "assistant",
                "Got it. I will keep the working note tight first.",
            ),
            line(
                3,
                "user",
                "Remember that Miso gets anxious at the vet unless the carrier has the blue blanket.",
            ),
            line(
                4,
                "assistant",
                "Miso vet context: blue blanket in the carrier helps with anxiety.",
            ),
            line(
                5,
                "user",
                "Now switch back to the Rust tests for the memory bridge.",
            ),
            line(
                6,
                "assistant",
                "I will verify the memory bridge tests next.",
            ),
        ];
        let line_refs = lines.iter().map(String::as_str).collect::<Vec<_>>();
        let temp = write_test_log(&line_refs);
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        write_bucket_map_json(&paths.bucket_map_json, &default_interaction_bucket_map(10)).unwrap();

        let evidence = collect_cold_interaction_evidence(
            Some(&temp.dir),
            &ColdInteractionEvidenceQuery::latest(8),
        )
        .unwrap();
        let miso_evidence = evidence
            .iter()
            .find(|item| item.exchange_block.contains("Miso gets anxious"))
            .expect("fixture should contain the Miso exchange");
        let decisions = vec![ColdBucketPlacementDecision::new(
            "entity:people_places_pets",
            ColdBucketFit::Strong,
            "Miso vet anxiety and carrier blanket details.",
            ColdBucketPlacementAuthor::Sorter,
        )];
        let proposed = append_semantic_placement_proposals_to_store(
            &paths,
            &miso_evidence.evidence_ref,
            &decisions,
        )
        .unwrap();

        let proposed_hits = query_semantic_index_store(
            &paths,
            &ColdSemanticQuery::remember_current_input("Miso vet carrier"),
        )
        .unwrap();
        assert!(
            proposed_hits.is_empty(),
            "proposed semantic placements must stay inert until user approval"
        );

        let mut approved = HashSet::new();
        approved.insert(proposed[0].placement_id.clone());
        apply_placement_review_decisions_to_store(&paths, &approved, &HashSet::new(), 20).unwrap();
        let placements = read_placements_jsonl(&paths.placements_jsonl).unwrap();
        assert_eq!(placements.len(), 1);

        let semantic_hits = query_semantic_index_store(
            &paths,
            &ColdSemanticQuery::remember_current_input("Miso vet carrier"),
        )
        .unwrap();
        assert_eq!(semantic_hits.len(), 1);
        assert_eq!(semantic_hits[0].bucket_id, "entity:people_places_pets");

        let candidates = hydrate_semantic_cold_hits(
            Some(&temp.dir),
            &ColdMemoryQuery::remember_current_input("What was Miso's vet anxiety setup?"),
            &semantic_hits,
        )
        .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].recall_trace.lane,
            ColdMemoryRecallLane::SemanticBucket
        );
        assert!(
            candidates[0]
                .context_block
                .contains("concise implementation notes")
        );
        assert!(
            candidates[0]
                .context_block
                .contains("Miso gets anxious at the vet")
        );
        assert!(
            candidates[0]
                .context_block
                .contains("Rust tests for the memory bridge")
        );

        let mut selected = HashSet::new();
        selected.insert(candidates[0].id.clone());
        let mut attachment = ColdMemoryAttachment::from_selected_ids(
            "What was Miso's vet anxiety setup?",
            30,
            &candidates,
            &selected,
        )
        .expect("explicit user selection should create an attachment receipt");
        attachment.recall_id = Some("remember-recall-test".to_string());
        let host_block = attachment.host_prompt_block();

        assert!(host_block.contains("### USER-SELECTED COLD LOG CONTEXT"));
        assert!(host_block.contains("only because the user selected it in Remember"));
        assert!(host_block.contains("historical evidence, not automatically current truth"));
        assert!(host_block.contains("Miso vet context: blue blanket"));
        assert!(
            host_block.contains(
                "Do not promote them into hot memory, Luke Warm memory, or durable memory"
            )
        );

        let receipt = append_memory_attachment_receipt_to_store(&temp.dir, &attachment).unwrap();
        let receipt_path = remember_attachment_receipts_path(&temp.dir);
        let receipts = read_memory_attachment_receipts_jsonl(&receipt_path).unwrap();
        assert_eq!(receipts, vec![receipt]);
        assert_eq!(
            receipts[0].recall_id.as_deref(),
            Some("remember-recall-test")
        );
        assert_eq!(receipts[0].candidate_count, 1);
        assert_eq!(
            receipts[0].candidates[0].recall_trace.lane,
            ColdMemoryRecallLane::SemanticBucket
        );
        assert_eq!(
            receipts[0].candidates[0].source_log_path,
            temp.dir.join("cold_log.jsonl").display().to_string()
        );

        let raw_receipt = std::fs::read_to_string(receipt_path).unwrap();
        assert!(raw_receipt.contains("remember-recall-test"));
        assert!(raw_receipt.contains("entity:people_places_pets"));
        assert!(!raw_receipt.contains("Got it. I will keep the working note tight first."));
        assert!(!raw_receipt.contains("Miso vet context: blue blanket"));
        assert!(!raw_receipt.contains("I will verify the memory bridge tests next."));

        let overview = render_memory_attachment_receipts_overview(&receipts);
        assert!(overview.contains("Remember attachment receipts"));
        assert!(overview.contains("remember_attachment_receipts: 1"));
        assert!(overview.contains("recall_id=remember-recall-test"));
        assert!(overview.contains("lane=semantic_bucket"));
        assert!(overview.contains("entity:people_places_pets"));
        assert!(!overview.contains("Miso vet context: blue blanket"));

        let recall_receipt = append_memory_recall_receipt_to_store(
            &temp.dir,
            "remember-recall-test",
            "What was Miso's vet anxiety setup?",
            40,
            ColdMemoryRecallOutcome::AttachedSelected,
            &candidates,
            &selected,
        )
        .unwrap();
        let recall_receipt_path = remember_recall_receipts_path(&temp.dir);
        let recall_receipts = read_memory_recall_receipts_jsonl(&recall_receipt_path).unwrap();
        assert_eq!(recall_receipts, vec![recall_receipt]);
        assert_eq!(recall_receipts[0].recall_id, "remember-recall-test");
        assert_eq!(
            recall_receipts[0].outcome,
            ColdMemoryRecallOutcome::AttachedSelected
        );
        assert_eq!(recall_receipts[0].candidate_count, 1);
        assert_eq!(recall_receipts[0].selected_candidate_count, 1);

        let raw_recall_receipt = std::fs::read_to_string(recall_receipt_path).unwrap();
        assert!(raw_recall_receipt.contains("attached_selected"));
        assert!(raw_recall_receipt.contains("entity:people_places_pets"));
        assert!(!raw_recall_receipt.contains("Miso vet context: blue blanket"));

        let recall_overview = render_memory_recall_receipts_overview(&recall_receipts);
        assert!(recall_overview.contains("Remember recall receipts"));
        assert!(recall_overview.contains("remember_recall_receipts: 1"));
        assert!(recall_overview.contains("recall_id=remember-recall-test"));
        assert!(recall_overview.contains("outcome=attached_selected"));
        assert!(recall_overview.contains("selected=true"));
        assert!(recall_overview.contains("lane=semantic_bucket"));
        assert!(!recall_overview.contains("Miso vet context: blue blanket"));

        let session_overview =
            render_memory_remember_session_receipts_overview(&recall_receipts, &receipts);
        assert!(session_overview.contains("Remember session receipts"));
        assert!(session_overview.contains("remember_sessions: 1"));
        assert!(session_overview.contains("recall_id=remember-recall-test"));
        assert!(session_overview.contains("recall outcome=attached_selected"));
        assert!(session_overview.contains("attachment at=30"));
        assert!(session_overview.contains("lane=semantic_bucket"));
        assert!(!session_overview.contains("Miso vet context: blue blanket"));

        let feedback_receipt = append_memory_feedback_receipt_to_store(
            &temp.dir,
            Some("remember-recall-test".to_string()),
            "What was Miso's vet anxiety setup?",
            50,
            ColdMemoryCandidateFeedback::Stale,
            &candidates[0],
        )
        .unwrap();
        let feedback_receipt_path = remember_feedback_receipts_path(&temp.dir);
        let feedback_receipts =
            read_memory_feedback_receipts_jsonl(&feedback_receipt_path).unwrap();
        assert_eq!(feedback_receipts, vec![feedback_receipt]);
        assert_eq!(
            feedback_receipts[0].feedback,
            ColdMemoryCandidateFeedback::Stale
        );
        assert_eq!(
            feedback_receipts[0].recall_id.as_deref(),
            Some("remember-recall-test")
        );

        let raw_feedback_receipt = std::fs::read_to_string(feedback_receipt_path).unwrap();
        assert!(raw_feedback_receipt.contains("stale"));
        assert!(raw_feedback_receipt.contains("entity:people_places_pets"));
        assert!(!raw_feedback_receipt.contains("Miso vet context: blue blanket"));

        let feedback_overview = render_memory_feedback_receipts_overview(&feedback_receipts);
        assert!(feedback_overview.contains("Remember feedback receipts"));
        assert!(feedback_overview.contains("feedback=stale"));
        assert!(feedback_overview.contains("lane=semantic_bucket"));
        assert!(!feedback_overview.contains("Miso vet context: blue blanket"));

        let feedback_summary =
            summarize_prior_feedback_for_candidate(&candidates[0], &feedback_receipts)
                .expect("matching prior feedback should be summarized");
        assert!(feedback_summary.contains("stale=1"));
        assert!(feedback_summary.contains("Review carefully before attaching"));
    }

    #[test]
    fn recall_route_preview_summarizes_lanes_without_context_payloads() {
        let evidence_ref = ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 2, 3);
        let atlas_hit = ColdAtlasHit {
            score: 1.2,
            atlas_id: "atlas:miso:v1".to_string(),
            title: "Miso care shape".to_string(),
            summary: "Broad Miso care context.".to_string(),
            scope: "pet care".to_string(),
            status: ColdAtlasRecordStatus::HistoricalShape,
            confidence: ColdAtlasConfidence::new(0.8, "supported"),
            evidence_index: 0,
            evidence_ref: evidence_ref.clone(),
        };
        let semantic_hit = ColdSemanticHit {
            score: 2.4,
            bucket_id: "entity:miso".to_string(),
            bucket_label: "Miso".to_string(),
            bucket_kind: crate::memory::semantic_index::ColdSemanticBucketKind::Positive,
            placement_id: "placement:miso".to_string(),
            placement_fit: crate::memory::semantic_index::ColdBucketFit::Strong,
            placement_reason: "Miso care details.".to_string(),
            evidence_ref: evidence_ref.clone(),
        };
        let cold_candidate = ColdMemoryCandidate {
            id: "cold:1:0:2".to_string(),
            score: 0.5,
            title: "cold-log match".to_string(),
            preview: "User asked about Miso".to_string(),
            recall_trace: ColdMemoryRecallTrace::bookkeeper(),
            matched_exchange: "User: old".to_string(),
            context_block: "Source: cold_log.jsonl\nUser: old\nAssistant: exact answer".to_string(),
            provenance: ColdMemorySourceRef {
                log_path: PathBuf::from("cold_log.jsonl"),
                matched_event_index: 2,
                context_start_event_index: 0,
                context_end_event_index: 3,
                matched_ts_unix_ms: 10,
                context_start_ts_unix_ms: Some(9),
                context_end_ts_unix_ms: Some(11),
            },
        };
        let query = ColdMemoryQuery::remember_current_input("miso care");

        let preview = render_recall_route_preview(
            &query,
            &[atlas_hit],
            &[semantic_hit],
            &[cold_candidate.clone()],
            &[cold_candidate],
        );

        assert!(preview.contains("Recall route preview"));
        assert!(preview.contains("Cold atlas shape hits: 1"));
        assert!(preview.contains("Semantic bucket evidence hits: 1"));
        assert!(preview.contains("Cold-log direct candidates: 1"));
        assert!(preview.contains("Remember post-merge candidate order: 1"));
        assert!(preview.contains("Miso care shape [atlas:miso:v1]"));
        assert!(preview.contains("Miso [entity:miso]"));
        assert!(preview.contains("preview=User asked about Miso"));
        assert!(!preview.contains("Assistant: exact answer"));
    }

    #[test]
    fn selected_context_renderer_marks_history_as_user_selected_evidence() {
        let candidate = ColdMemoryCandidate {
            id: "cold:1:0:2".to_string(),
            score: 0.5,
            title: "title".to_string(),
            preview: "preview".to_string(),
            recall_trace: ColdMemoryRecallTrace::bookkeeper(),
            matched_exchange: "User: old".to_string(),
            context_block: "Source: cold_log.jsonl\nUser: old\nAssistant: answer".to_string(),
            provenance: ColdMemorySourceRef {
                log_path: PathBuf::from("cold_log.jsonl"),
                matched_event_index: 1,
                context_start_event_index: 0,
                context_end_event_index: 2,
                matched_ts_unix_ms: 10,
                context_start_ts_unix_ms: Some(9),
                context_end_ts_unix_ms: Some(11),
            },
        };

        let block = render_user_selected_cold_context(&[candidate]);

        assert!(block.contains("USER-SELECTED COLD LOG CONTEXT"));
        assert!(block.contains("historical evidence, not current truth"));
        assert!(
            block.contains(
                "Do not promote them into hot memory, Luke Warm memory, or durable memory"
            )
        );
        assert!(block.contains("User: old"));
    }

    #[test]
    fn attachment_receipt_requires_explicit_selected_ids() {
        let candidate = ColdMemoryCandidate {
            id: "cold:7:6:8".to_string(),
            score: 0.5,
            title: "title".to_string(),
            preview: "preview".to_string(),
            recall_trace: ColdMemoryRecallTrace::bookkeeper(),
            matched_exchange: "User: old".to_string(),
            context_block: "Source: cold_log.jsonl\nUser: old\nAssistant: answer".to_string(),
            provenance: ColdMemorySourceRef {
                log_path: PathBuf::from("cold_log.jsonl"),
                matched_event_index: 7,
                context_start_event_index: 6,
                context_end_event_index: 8,
                matched_ts_unix_ms: 70,
                context_start_ts_unix_ms: Some(60),
                context_end_ts_unix_ms: Some(80),
            },
        };
        let candidates = vec![candidate.clone()];

        let empty = HashSet::new();
        assert!(
            ColdMemoryAttachment::from_selected_ids("query", 123, &candidates, &empty).is_none()
        );

        let mut selected = HashSet::new();
        selected.insert(candidate.id.clone());
        let receipt =
            ColdMemoryAttachment::from_selected_ids("query", 123, &candidates, &selected).unwrap();

        assert_eq!(receipt.attached_at_unix_ms, 123);
        assert_eq!(receipt.query, "query");
        assert_eq!(receipt.candidates.len(), 1);
        assert_eq!(receipt.candidates[0].id, candidate.id);
        assert!(
            receipt
                .prompt_block()
                .contains("USER-SELECTED COLD LOG CONTEXT")
        );
    }

    #[test]
    fn remember_attachment_receipt_overview_renders_empty_audit_trail() {
        let overview = render_memory_attachment_receipts_overview(&[]);

        assert!(overview.contains("Remember attachment receipts"));
        assert!(overview.contains("remember_attachment_receipts: 0"));
        assert!(overview.contains("  - empty"));
    }

    #[test]
    fn remember_recall_receipt_overview_renders_empty_audit_trail() {
        let overview = render_memory_recall_receipts_overview(&[]);

        assert!(overview.contains("Remember recall receipts"));
        assert!(overview.contains("remember_recall_receipts: 0"));
        assert!(overview.contains("  - empty"));
    }

    #[test]
    fn remember_session_receipt_overview_groups_empty_audit_trail() {
        let overview = render_memory_remember_session_receipts_overview(&[], &[]);

        assert!(overview.contains("Remember session receipts"));
        assert!(overview.contains("remember_sessions: 0"));
        assert!(overview.contains("  - empty"));
    }

    #[test]
    fn unix_ms_formatting_is_stable_utc() {
        assert_eq!(format_unix_ms_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            format_unix_ms_utc(1_700_000_000_000),
            "2023-11-14 22:13:20 UTC"
        );
        assert_eq!(
            format_unix_ms_for_payload(1_700_000_000_000),
            "2023-11-14 22:13:20 UTC (unix ms 1700000000000)"
        );
    }
}
