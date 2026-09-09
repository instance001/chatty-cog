use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::memory::governor::{ColdMemoryCandidateFeedback, ColdMemoryFeedbackReceipt};

pub const COLD_SEMANTIC_DIR: &str = "cold_semantic";
pub const BUCKET_MAP_JSON: &str = "bucket_map.json";
pub const PLACEMENTS_JSONL: &str = "placements.jsonl";
pub const SORT_RECEIPTS_JSONL: &str = "sort_receipts.jsonl";
pub const PLACEMENT_REVIEW_RECEIPTS_JSONL: &str = "placement_review_receipts.jsonl";
pub const SIFT_PROPOSALS_JSONL: &str = "sift_proposals.jsonl";
pub const SIFT_RECEIPTS_JSONL: &str = "sift_receipts.jsonl";
pub const DEFAULT_MISC_BUCKET_ID: &str = "misc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdSemanticIndexPaths {
    pub root_dir: PathBuf,
    pub bucket_map_json: PathBuf,
    pub placements_jsonl: PathBuf,
    pub sort_receipts_jsonl: PathBuf,
    pub placement_review_receipts_jsonl: PathBuf,
    pub sift_proposals_jsonl: PathBuf,
    pub sift_receipts_jsonl: PathBuf,
}

impl ColdSemanticIndexPaths {
    pub fn under_memory_dir(memory_dir: impl AsRef<Path>) -> Self {
        let root_dir = memory_dir.as_ref().join(COLD_SEMANTIC_DIR);
        let bucket_map_json = root_dir.join(BUCKET_MAP_JSON);
        let placements_jsonl = root_dir.join(PLACEMENTS_JSONL);
        let sort_receipts_jsonl = root_dir.join(SORT_RECEIPTS_JSONL);
        let placement_review_receipts_jsonl = root_dir.join(PLACEMENT_REVIEW_RECEIPTS_JSONL);
        let sift_proposals_jsonl = root_dir.join(SIFT_PROPOSALS_JSONL);
        let sift_receipts_jsonl = root_dir.join(SIFT_RECEIPTS_JSONL);
        Self {
            root_dir,
            bucket_map_json,
            placements_jsonl,
            sort_receipts_jsonl,
            placement_review_receipts_jsonl,
            sift_proposals_jsonl,
            sift_receipts_jsonl,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdSemanticIndexSnapshot {
    pub bucket_map: Option<ColdSemanticBucketMap>,
    #[serde(default)]
    pub placements: Vec<ColdBucketPlacement>,
}

impl ColdSemanticIndexSnapshot {
    pub fn empty() -> Self {
        Self {
            bucket_map: None,
            placements: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let Some(bucket_map) = &self.bucket_map else {
            if self.placements.is_empty() {
                return Ok(());
            }
            anyhow::bail!("semantic placements exist without a semantic bucket map");
        };

        validate_placements_against_bucket_map(bucket_map, &self.placements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdSemanticQuery {
    pub query: String,
    pub max_hits: usize,
    pub include_misc: bool,
}

impl ColdSemanticQuery {
    pub fn remember_current_input(input: impl Into<String>) -> Self {
        Self {
            query: input.into(),
            max_hits: 16,
            include_misc: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColdSemanticHit {
    pub score: f32,
    pub bucket_id: String,
    pub bucket_label: String,
    pub bucket_kind: ColdSemanticBucketKind,
    pub placement_id: String,
    pub placement_fit: ColdBucketFit,
    pub placement_reason: String,
    pub evidence_ref: ColdEvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdEvidenceRef {
    pub source_log_path: String,
    pub source_session_id: Option<String>,
    pub start_event_index: usize,
    pub end_event_index: usize,
    pub start_ts_unix_ms: Option<i64>,
    pub end_ts_unix_ms: Option<i64>,
    pub source_kind: ColdEvidenceSourceKind,
    pub source_hash: Option<String>,
}

impl ColdEvidenceRef {
    pub fn interaction_log_range(
        source_log_path: impl Into<String>,
        start_event_index: usize,
        end_event_index: usize,
    ) -> Self {
        Self {
            source_log_path: source_log_path.into(),
            source_session_id: None,
            start_event_index,
            end_event_index,
            start_ts_unix_ms: None,
            end_ts_unix_ms: None,
            source_kind: ColdEvidenceSourceKind::InteractionLog,
            source_hash: None,
        }
    }

    pub fn stable_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.source_kind.as_str(),
            self.source_log_path,
            self.start_event_index,
            self.end_event_index
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdEvidenceSourceKind {
    InteractionLog,
    ModuleLog,
    SandboxEvent,
    ImportedConversation,
}

impl ColdEvidenceSourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InteractionLog => "interaction_log",
            Self::ModuleLog => "module_log",
            Self::SandboxEvent => "sandbox_event",
            Self::ImportedConversation => "imported_conversation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdSemanticBucket {
    pub bucket_id: String,
    pub label: String,
    pub description: String,
    pub sort_intent: String,
    pub kind: ColdSemanticBucketKind,
    pub status: ColdSemanticBucketStatus,
    #[serde(default)]
    pub evidence_refs: Vec<ColdEvidenceRef>,
    #[serde(default)]
    pub weak_fit_refs: Vec<ColdEvidenceRef>,
    #[serde(default)]
    pub overflow_refs: Vec<ColdEvidenceRef>,
}

impl ColdSemanticBucket {
    pub fn positive(
        bucket_id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        sort_intent: impl Into<String>,
    ) -> Self {
        Self {
            bucket_id: bucket_id.into(),
            label: label.into(),
            description: description.into(),
            sort_intent: sort_intent.into(),
            kind: ColdSemanticBucketKind::Positive,
            status: ColdSemanticBucketStatus::Active,
            evidence_refs: Vec::new(),
            weak_fit_refs: Vec::new(),
            overflow_refs: Vec::new(),
        }
    }

    pub fn misc(sort_intent: impl Into<String>) -> Self {
        Self {
            bucket_id: DEFAULT_MISC_BUCKET_ID.to_string(),
            label: "Misc / weak fit".to_string(),
            description: "Evidence that does not fit a positive semantic bucket cleanly yet."
                .to_string(),
            sort_intent: sort_intent.into(),
            kind: ColdSemanticBucketKind::Misc,
            status: ColdSemanticBucketStatus::Active,
            evidence_refs: Vec::new(),
            weak_fit_refs: Vec::new(),
            overflow_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdSemanticBucketMap {
    pub schema_version: u32,
    pub map_id: String,
    pub created_at_unix_ms: i64,
    pub positive_bucket_count: usize,
    pub bucket_count: usize,
    pub misc_bucket_id: String,
    pub sort_intent: String,
    pub created_by: ColdBucketPlacementAuthor,
    #[serde(default)]
    pub buckets: Vec<ColdSemanticBucket>,
}

impl ColdSemanticBucketMap {
    pub fn new(
        map_id: impl Into<String>,
        created_at_unix_ms: i64,
        sort_intent: impl Into<String>,
        created_by: ColdBucketPlacementAuthor,
        buckets: Vec<ColdSemanticBucket>,
    ) -> Self {
        let positive_bucket_count = buckets
            .iter()
            .filter(|bucket| bucket.kind == ColdSemanticBucketKind::Positive)
            .count();
        let bucket_count = buckets.len();
        let misc_bucket_id = buckets
            .iter()
            .find(|bucket| bucket.kind == ColdSemanticBucketKind::Misc)
            .map(|bucket| bucket.bucket_id.clone())
            .unwrap_or_else(|| DEFAULT_MISC_BUCKET_ID.to_string());
        Self {
            schema_version: 1,
            map_id: map_id.into(),
            created_at_unix_ms,
            positive_bucket_count,
            bucket_count,
            misc_bucket_id,
            sort_intent: sort_intent.into(),
            created_by,
            buckets,
        }
    }

    pub fn bucket(&self, bucket_id: &str) -> Option<&ColdSemanticBucket> {
        self.buckets
            .iter()
            .find(|bucket| bucket.bucket_id == bucket_id)
    }

    pub fn misc_bucket(&self) -> Option<&ColdSemanticBucket> {
        self.bucket(&self.misc_bucket_id)
    }

    pub fn validate(&self) -> Result<()> {
        if self.bucket_count != self.buckets.len() {
            anyhow::bail!(
                "semantic bucket map bucket_count {} does not match {} buckets",
                self.bucket_count,
                self.buckets.len()
            );
        }

        let positive_count = self
            .buckets
            .iter()
            .filter(|bucket| bucket.kind == ColdSemanticBucketKind::Positive)
            .count();
        if self.positive_bucket_count != positive_count {
            anyhow::bail!(
                "semantic bucket map positive_bucket_count {} does not match {} positive buckets",
                self.positive_bucket_count,
                positive_count
            );
        }

        let misc_count = self
            .buckets
            .iter()
            .filter(|bucket| bucket.kind == ColdSemanticBucketKind::Misc)
            .count();
        if misc_count != 1 {
            anyhow::bail!("semantic bucket map must contain exactly one misc bucket");
        }

        let Some(misc_bucket) = self.misc_bucket() else {
            anyhow::bail!(
                "semantic bucket map misc_bucket_id '{}' does not resolve",
                self.misc_bucket_id
            );
        };
        if misc_bucket.kind != ColdSemanticBucketKind::Misc {
            anyhow::bail!(
                "semantic bucket map misc_bucket_id '{}' points to a non-misc bucket",
                self.misc_bucket_id
            );
        }

        Ok(())
    }
}

pub fn default_interaction_bucket_map(created_at_unix_ms: i64) -> ColdSemanticBucketMap {
    ColdSemanticBucketMap::new(
        "interaction-bucket-map-v1",
        created_at_unix_ms,
        "Sort bounded cold-log interaction exchanges into a small frozen v1 map: broad positive buckets for deliberate recall, plus misc for weak or unclear fits.",
        ColdBucketPlacementAuthor::User,
        vec![
            ColdSemanticBucket::positive(
                "entity:people_places_pets",
                "People, places, and pets",
                "Recurring real-world entities, relationships, care context, locations, and personal references.",
                "Use when the exchange is primarily about a named person, pet, place, household entity, relationship, or recurring real-world referent.",
            ),
            ColdSemanticBucket::positive(
                "topic:projects_code",
                "Projects and code",
                "Repository work, implementation decisions, architecture notes, technical plans, debugging, and tests.",
                "Use when the exchange is primarily about software projects, repo structure, coding tasks, architecture, bugs, tests, or implementation plans.",
            ),
            ColdSemanticBucket::positive(
                "topic:preferences_workflow",
                "Preferences and workflow",
                "User preferences, interaction style, working agreements, process choices, and collaboration norms.",
                "Use when the exchange records how the user wants Chatty-Cog, Codex, or collaborators to behave or work.",
            ),
            ColdSemanticBucket::positive(
                "topic:open_threads",
                "Open threads and commitments",
                "Unfinished decisions, future follow-ups, pending tasks, promises, and things explicitly marked to revisit.",
                "Use when the exchange creates, changes, or resolves a remembered open loop or future commitment.",
            ),
            ColdSemanticBucket::misc(
                "Use only when an exchange does not fit a positive bucket cleanly, is too weak/noisy to place, or needs a future bucket-map revision.",
            ),
        ],
    )
}

pub fn create_default_interaction_bucket_map_if_missing(
    path: impl AsRef<Path>,
    created_at_unix_ms: i64,
) -> Result<Option<ColdSemanticBucketMap>> {
    if read_bucket_map_json(path.as_ref())?.is_some() {
        return Ok(None);
    }

    let map = default_interaction_bucket_map(created_at_unix_ms);
    map.validate()?;
    write_bucket_map_json(path, &map)?;
    Ok(Some(map))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdSemanticBucketKind {
    Positive,
    Misc,
}

impl ColdSemanticBucketKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Misc => "misc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdSemanticBucketStatus {
    Active,
    NeedsReview,
    Stale,
    Retired,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketPlacement {
    pub placement_id: String,
    pub bucket_id: String,
    pub evidence_ref: ColdEvidenceRef,
    pub fit: ColdBucketFit,
    pub reason: String,
    pub created_by: ColdBucketPlacementAuthor,
    pub review_status: ColdBucketReviewStatus,
}

impl ColdBucketPlacement {
    pub fn new(
        bucket_id: impl Into<String>,
        evidence_ref: ColdEvidenceRef,
        fit: ColdBucketFit,
        reason: impl Into<String>,
        created_by: ColdBucketPlacementAuthor,
    ) -> Self {
        let bucket_id = bucket_id.into();
        let placement_id = format!("placement:{}:{}", bucket_id, evidence_ref.stable_key());
        Self {
            placement_id,
            bucket_id,
            evidence_ref,
            fit,
            reason: reason.into(),
            created_by,
            review_status: ColdBucketReviewStatus::Proposed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketPlacementDecision {
    pub bucket_id: String,
    pub fit: ColdBucketFit,
    pub reason: String,
    pub proposed_by: ColdBucketPlacementAuthor,
}

impl ColdBucketPlacementDecision {
    pub fn new(
        bucket_id: impl Into<String>,
        fit: ColdBucketFit,
        reason: impl Into<String>,
        proposed_by: ColdBucketPlacementAuthor,
    ) -> Self {
        Self {
            bucket_id: bucket_id.into(),
            fit,
            reason: reason.into(),
            proposed_by,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdSemanticSortReceipt {
    pub receipt_id: String,
    pub sorted_at_unix_ms: i64,
    pub source_log_path: String,
    pub last_seen_event_index: Option<usize>,
    pub evidence_ref_count: usize,
    #[serde(default)]
    pub proposed_placement_ids: Vec<String>,
    #[serde(default)]
    pub outcome: ColdSemanticSortOutcome,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColdSemanticSortOutcome {
    #[default]
    Committed,
    Skipped,
}

impl ColdSemanticSortOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::Skipped => "skipped",
        }
    }
}

impl ColdSemanticSortReceipt {
    pub fn from_run(
        receipt_id: impl Into<String>,
        sorted_at_unix_ms: i64,
        source_log_path: impl Into<String>,
        evidence_refs: &[ColdEvidenceRef],
        proposed_placements: &[ColdBucketPlacement],
        note: impl Into<String>,
    ) -> Self {
        let source_log_path = source_log_path.into();
        let last_seen_event_index = evidence_refs
            .iter()
            .filter(|evidence_ref| evidence_ref.source_log_path == source_log_path)
            .map(|evidence_ref| evidence_ref.end_event_index)
            .max();
        Self {
            receipt_id: receipt_id.into(),
            sorted_at_unix_ms,
            source_log_path,
            last_seen_event_index,
            evidence_ref_count: evidence_refs.len(),
            proposed_placement_ids: proposed_placements
                .iter()
                .map(|placement| placement.placement_id.clone())
                .collect(),
            outcome: ColdSemanticSortOutcome::Committed,
            note: note.into(),
        }
    }

    pub fn skipped(
        receipt_id: impl Into<String>,
        sorted_at_unix_ms: i64,
        source_log_path: impl Into<String>,
        evidence_refs: &[ColdEvidenceRef],
        note: impl Into<String>,
    ) -> Self {
        let mut receipt = Self::from_run(
            receipt_id,
            sorted_at_unix_ms,
            source_log_path,
            evidence_refs,
            &[],
            note,
        );
        receipt.outcome = ColdSemanticSortOutcome::Skipped;
        receipt
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketPlacementReviewReceipt {
    pub reviewed_at_unix_ms: i64,
    #[serde(default)]
    pub approved_placement_ids: Vec<String>,
    #[serde(default)]
    pub rejected_placement_ids: Vec<String>,
    pub remaining_proposed_count: usize,
    #[serde(default)]
    pub remaining_needs_review_count: usize,
    #[serde(default)]
    pub transitions: Vec<ColdBucketPlacementReviewTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColdBucketPlacementReviewTransition {
    pub placement_id: String,
    pub from_status: ColdBucketReviewStatus,
    pub to_status: ColdBucketReviewStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdSemanticSortItem {
    pub evidence_ref: ColdEvidenceRef,
    #[serde(default)]
    pub decisions: Vec<ColdBucketPlacementDecision>,
}

impl ColdSemanticSortItem {
    pub fn new(evidence_ref: ColdEvidenceRef, decisions: Vec<ColdBucketPlacementDecision>) -> Self {
        Self {
            evidence_ref,
            decisions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdBucketFit {
    Strong,
    Medium,
    Weak,
    Overflow,
}

impl ColdBucketFit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strong => "strong",
            Self::Medium => "medium",
            Self::Weak => "weak",
            Self::Overflow => "overflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdBucketPlacementAuthor {
    Host,
    Bookkeeper,
    Sorter,
    User,
    Imported,
}

impl ColdBucketPlacementAuthor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Bookkeeper => "bookkeeper",
            Self::Sorter => "sorter",
            Self::User => "user",
            Self::Imported => "imported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdBucketReviewStatus {
    Proposed,
    Approved,
    Rejected,
    NeedsReview,
}

impl ColdBucketReviewStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::NeedsReview => "needs_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketSiftProposal {
    pub proposal_id: String,
    pub bucket_id: String,
    pub proposed_at_unix_ms: i64,
    pub rationale: String,
    #[serde(default)]
    pub changes: Vec<ColdBucketSiftChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketSiftChange {
    pub change_id: String,
    pub action: ColdBucketSiftAction,
    #[serde(default)]
    pub target_placement_id: Option<String>,
    #[serde(default)]
    pub target_placement_index: Option<usize>,
    pub evidence_ref: ColdEvidenceRef,
    pub from_bucket_id: Option<String>,
    pub to_bucket_id: Option<String>,
    pub before: String,
    pub after: String,
    pub rationale: String,
    pub selected_for_apply: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdBucketSiftAction {
    MergeDuplicate,
    MarkStale,
    MoveToBucket,
    MoveToOverflow,
    PreserveTemporalRef,
    RetirePlacement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdBucketSiftReceipt {
    pub receipt_id: String,
    pub proposal_id: String,
    pub applied_at_unix_ms: i64,
    #[serde(default)]
    pub applied_change_ids: Vec<String>,
    #[serde(default)]
    pub rejected_change_ids: Vec<String>,
}

pub fn read_placements_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdBucketPlacement>> {
    read_jsonl_records(path, "semantic placement")
}

pub fn read_sort_receipts_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdSemanticSortReceipt>> {
    read_jsonl_records(path, "semantic sort receipt")
}

pub fn read_placement_review_receipts_jsonl(
    path: impl AsRef<Path>,
) -> Result<Vec<ColdBucketPlacementReviewReceipt>> {
    read_jsonl_records(path, "semantic placement review receipt")
}

pub fn read_sift_proposals_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdBucketSiftProposal>> {
    read_jsonl_records(path, "semantic sift proposal")
}

pub fn read_sift_receipts_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdBucketSiftReceipt>> {
    read_jsonl_records(path, "semantic sift receipt")
}

fn read_jsonl_records<T>(path: impl AsRef<Path>, record_label: &str) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let path = path.as_ref();
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut records = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line.with_context(|| format!("read {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<T>(&line).with_context(|| {
            format!(
                "parse {record_label} line {} in {}",
                index + 1,
                path.display()
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

pub fn read_bucket_map_json(path: impl AsRef<Path>) -> Result<Option<ColdSemanticBucketMap>> {
    let path = path.as_ref();
    if !path.is_file() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let map = serde_json::from_str::<ColdSemanticBucketMap>(&json)
        .with_context(|| format!("parse {}", path.display()))?;
    map.validate()
        .with_context(|| format!("validate {}", path.display()))?;
    Ok(Some(map))
}

pub fn write_bucket_map_json(path: impl AsRef<Path>, map: &ColdSemanticBucketMap) -> Result<()> {
    let path = path.as_ref();
    map.validate().context("validate semantic bucket map")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(map).context("serialize semantic bucket map")?;
    std::fs::write(path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn load_semantic_index_snapshot(
    paths: &ColdSemanticIndexPaths,
) -> Result<ColdSemanticIndexSnapshot> {
    let snapshot = ColdSemanticIndexSnapshot {
        bucket_map: read_bucket_map_json(&paths.bucket_map_json)?,
        placements: read_placements_jsonl(&paths.placements_jsonl)?,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

pub fn append_placement_jsonl(
    path: impl AsRef<Path>,
    placement: &ColdBucketPlacement,
) -> Result<()> {
    append_jsonl_record(path, placement, "semantic placement")
}

pub fn write_placements_jsonl(
    path: impl AsRef<Path>,
    placements: &[ColdBucketPlacement],
) -> Result<()> {
    write_jsonl_records(path, placements, "semantic placement")
}

pub fn append_sift_proposal_jsonl(
    path: impl AsRef<Path>,
    proposal: &ColdBucketSiftProposal,
) -> Result<()> {
    append_jsonl_record(path, proposal, "semantic sift proposal")
}

pub fn append_sort_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdSemanticSortReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "semantic sort receipt")
}

pub fn append_placement_review_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdBucketPlacementReviewReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "semantic placement review receipt")
}

pub fn append_sift_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdBucketSiftReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "semantic sift receipt")
}

fn append_jsonl_record<T>(path: impl AsRef<Path>, record: &T, record_label: &str) -> Result<()>
where
    T: Serialize,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let line =
        serde_json::to_string(record).with_context(|| format!("serialize {record_label}"))?;
    writeln!(file, "{line}").with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn write_jsonl_records<T>(path: impl AsRef<Path>, records: &[T], record_label: &str) -> Result<()>
where
    T: Serialize,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let mut file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    for record in records {
        let line =
            serde_json::to_string(record).with_context(|| format!("serialize {record_label}"))?;
        writeln!(file, "{line}").with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

pub fn append_validated_placement_jsonl(
    paths: &ColdSemanticIndexPaths,
    placement: &ColdBucketPlacement,
) -> Result<()> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot append semantic placement without bucket_map.json");
    };
    validate_placements_against_bucket_map(&bucket_map, std::slice::from_ref(placement))?;
    append_placement_jsonl(&paths.placements_jsonl, placement)
}

pub fn build_semantic_placement_proposals(
    map: &ColdSemanticBucketMap,
    evidence_ref: &ColdEvidenceRef,
    decisions: &[ColdBucketPlacementDecision],
) -> Result<Vec<ColdBucketPlacement>> {
    map.validate()?;
    let mut placements = Vec::new();
    for decision in decisions {
        let Some(bucket) = map.bucket(&decision.bucket_id) else {
            anyhow::bail!(
                "semantic placement decision references missing bucket '{}'",
                decision.bucket_id
            );
        };
        if bucket.kind == ColdSemanticBucketKind::Misc && decision.fit != ColdBucketFit::Overflow {
            anyhow::bail!(
                "semantic placement decision for misc bucket '{}' must use overflow fit",
                decision.bucket_id
            );
        }
        if decision.fit == ColdBucketFit::Overflow && bucket.kind != ColdSemanticBucketKind::Misc {
            anyhow::bail!(
                "overflow semantic placement decision must target misc bucket '{}'",
                map.misc_bucket_id
            );
        }

        let placement = ColdBucketPlacement::new(
            decision.bucket_id.clone(),
            evidence_ref.clone(),
            decision.fit,
            decision.reason.clone(),
            decision.proposed_by,
        );
        validate_placements_against_bucket_map(map, std::slice::from_ref(&placement))?;
        placements.push(placement);
    }
    Ok(placements)
}

pub fn append_semantic_placement_proposals_to_store(
    paths: &ColdSemanticIndexPaths,
    evidence_ref: &ColdEvidenceRef,
    decisions: &[ColdBucketPlacementDecision],
) -> Result<Vec<ColdBucketPlacement>> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot append semantic placement proposals without bucket_map.json");
    };
    let existing = read_placements_jsonl(&paths.placements_jsonl)?;
    let mut seen_existing = existing
        .iter()
        .filter(|placement| placement.review_status != ColdBucketReviewStatus::Rejected)
        .map(|placement| {
            (
                placement.bucket_id.clone(),
                placement.evidence_ref.stable_key(),
            )
        })
        .collect::<HashSet<_>>();
    let proposed = build_semantic_placement_proposals(&bucket_map, evidence_ref, decisions)?;
    let mut appended = Vec::new();
    for placement in proposed {
        let key = (
            placement.bucket_id.clone(),
            placement.evidence_ref.stable_key(),
        );
        if !seen_existing.insert(key) {
            continue;
        }
        append_validated_placement_jsonl(paths, &placement)?;
        appended.push(placement);
    }
    Ok(appended)
}

pub fn latest_sort_cursor_for_source(
    receipts: &[ColdSemanticSortReceipt],
    source_log_path: &str,
) -> Option<usize> {
    receipts
        .iter()
        .filter(|receipt| receipt.source_log_path == source_log_path)
        .filter_map(|receipt| receipt.last_seen_event_index)
        .max()
}

pub fn latest_sort_cursor_for_store(
    paths: &ColdSemanticIndexPaths,
    source_log_path: &str,
) -> Result<Option<usize>> {
    let receipts = read_sort_receipts_jsonl(&paths.sort_receipts_jsonl)?;
    Ok(latest_sort_cursor_for_source(&receipts, source_log_path))
}

pub fn append_semantic_sort_receipt_to_store(
    paths: &ColdSemanticIndexPaths,
    receipt: &ColdSemanticSortReceipt,
) -> Result<()> {
    append_sort_receipt_jsonl(&paths.sort_receipts_jsonl, receipt)
}

pub fn commit_semantic_sort_run_to_store(
    paths: &ColdSemanticIndexPaths,
    receipt_id: impl Into<String>,
    sorted_at_unix_ms: i64,
    source_log_path: impl Into<String>,
    items: &[ColdSemanticSortItem],
    note: impl Into<String>,
) -> Result<ColdSemanticSortReceipt> {
    let source_log_path = source_log_path.into();
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot commit semantic sort run without bucket_map.json");
    };
    let existing = read_placements_jsonl(&paths.placements_jsonl)?;
    let mut seen_existing = existing
        .iter()
        .filter(|placement| placement.review_status != ColdBucketReviewStatus::Rejected)
        .map(|placement| {
            (
                placement.bucket_id.clone(),
                placement.evidence_ref.stable_key(),
            )
        })
        .collect::<HashSet<_>>();

    let mut evidence_refs = Vec::new();
    let mut proposed_to_append = Vec::new();
    for item in items {
        if item.evidence_ref.source_log_path != source_log_path {
            anyhow::bail!(
                "semantic sort item source '{}' does not match run source '{}'",
                item.evidence_ref.source_log_path,
                source_log_path
            );
        }
        evidence_refs.push(item.evidence_ref.clone());
        let proposed =
            build_semantic_placement_proposals(&bucket_map, &item.evidence_ref, &item.decisions)?;
        for placement in proposed {
            let key = (
                placement.bucket_id.clone(),
                placement.evidence_ref.stable_key(),
            );
            if !seen_existing.insert(key) {
                continue;
            }
            proposed_to_append.push(placement);
        }
    }

    for placement in &proposed_to_append {
        append_validated_placement_jsonl(paths, placement)?;
    }

    let receipt = ColdSemanticSortReceipt::from_run(
        receipt_id,
        sorted_at_unix_ms,
        source_log_path,
        &evidence_refs,
        &proposed_to_append,
        note,
    );
    append_semantic_sort_receipt_to_store(paths, &receipt)?;
    Ok(receipt)
}

pub fn skip_semantic_sort_run_to_store(
    paths: &ColdSemanticIndexPaths,
    receipt_id: impl Into<String>,
    sorted_at_unix_ms: i64,
    source_log_path: impl Into<String>,
    evidence_refs: &[ColdEvidenceRef],
    note: impl Into<String>,
) -> Result<ColdSemanticSortReceipt> {
    let source_log_path = source_log_path.into();
    if evidence_refs.is_empty() {
        anyhow::bail!("cannot skip a semantic sort run without reviewed evidence");
    }
    if evidence_refs
        .iter()
        .any(|evidence_ref| evidence_ref.source_log_path != source_log_path)
    {
        anyhow::bail!("semantic skip evidence source does not match the run source");
    }
    let receipt = ColdSemanticSortReceipt::skipped(
        receipt_id,
        sorted_at_unix_ms,
        source_log_path,
        evidence_refs,
        note,
    );
    append_semantic_sort_receipt_to_store(paths, &receipt)?;
    Ok(receipt)
}

pub fn apply_selected_sift_changes_to_store(
    paths: &ColdSemanticIndexPaths,
    proposal: &ColdBucketSiftProposal,
    applied_at_unix_ms: i64,
) -> Result<ColdBucketSiftReceipt> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot apply semantic sift proposal without bucket_map.json");
    };
    let placements = read_placements_jsonl(&paths.placements_jsonl)?;
    let (updated, receipt) =
        apply_selected_sift_changes(&bucket_map, &placements, proposal, applied_at_unix_ms)?;

    write_placements_jsonl(&paths.placements_jsonl, &updated)?;
    append_sift_receipt_jsonl(&paths.sift_receipts_jsonl, &receipt)?;
    Ok(receipt)
}

pub fn apply_placement_review_decisions_to_store(
    paths: &ColdSemanticIndexPaths,
    approved_placement_ids: &HashSet<String>,
    rejected_placement_ids: &HashSet<String>,
    reviewed_at_unix_ms: i64,
) -> Result<ColdBucketPlacementReviewReceipt> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot review semantic placements without bucket_map.json");
    };
    let placements = read_placements_jsonl(&paths.placements_jsonl)?;
    let (updated, receipt) = apply_placement_review_decisions(
        &bucket_map,
        &placements,
        approved_placement_ids,
        rejected_placement_ids,
        reviewed_at_unix_ms,
    )?;
    write_placements_jsonl(&paths.placements_jsonl, &updated)?;
    append_placement_review_receipt_jsonl(&paths.placement_review_receipts_jsonl, &receipt)?;
    Ok(receipt)
}

pub fn apply_placement_review_decisions(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
    approved_placement_ids: &HashSet<String>,
    rejected_placement_ids: &HashSet<String>,
    reviewed_at_unix_ms: i64,
) -> Result<(Vec<ColdBucketPlacement>, ColdBucketPlacementReviewReceipt)> {
    validate_placements_against_bucket_map(map, placements)?;
    if approved_placement_ids.is_empty() && rejected_placement_ids.is_empty() {
        anyhow::bail!("select at least one semantic placement to approve or reject");
    }
    if let Some(overlap) = approved_placement_ids
        .intersection(rejected_placement_ids)
        .next()
    {
        anyhow::bail!("semantic placement '{overlap}' cannot be both approved and rejected");
    }

    let known_ids = placements
        .iter()
        .map(|placement| placement.placement_id.clone())
        .collect::<HashSet<_>>();
    for id in approved_placement_ids.iter().chain(rejected_placement_ids) {
        if !known_ids.contains(id) {
            anyhow::bail!("semantic placement review referenced unknown placement '{id}'");
        }
    }

    let mut updated = placements.to_vec();
    let mut approved = Vec::new();
    let mut rejected = Vec::new();
    let mut transitions = Vec::new();
    for placement in &mut updated {
        if approved_placement_ids.contains(&placement.placement_id) {
            let from_status = placement.review_status;
            placement.review_status = ColdBucketReviewStatus::Approved;
            approved.push(placement.placement_id.clone());
            transitions.push(ColdBucketPlacementReviewTransition {
                placement_id: placement.placement_id.clone(),
                from_status,
                to_status: ColdBucketReviewStatus::Approved,
            });
        } else if rejected_placement_ids.contains(&placement.placement_id) {
            let from_status = placement.review_status;
            placement.review_status = ColdBucketReviewStatus::Rejected;
            rejected.push(placement.placement_id.clone());
            transitions.push(ColdBucketPlacementReviewTransition {
                placement_id: placement.placement_id.clone(),
                from_status,
                to_status: ColdBucketReviewStatus::Rejected,
            });
        }
    }
    let remaining_proposed_count = updated
        .iter()
        .filter(|placement| placement.review_status == ColdBucketReviewStatus::Proposed)
        .count();
    let remaining_needs_review_count = updated
        .iter()
        .filter(|placement| placement.review_status == ColdBucketReviewStatus::NeedsReview)
        .count();

    Ok((
        updated,
        ColdBucketPlacementReviewReceipt {
            reviewed_at_unix_ms,
            approved_placement_ids: approved,
            rejected_placement_ids: rejected,
            remaining_proposed_count,
            remaining_needs_review_count,
            transitions,
        },
    ))
}

pub fn query_semantic_index(
    snapshot: &ColdSemanticIndexSnapshot,
    query: &ColdSemanticQuery,
) -> Result<Vec<ColdSemanticHit>> {
    snapshot.validate()?;
    let Some(bucket_map) = &snapshot.bucket_map else {
        return Ok(Vec::new());
    };
    let terms = query_terms(&query.query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut seen = HashSet::new();
    let mut hits = Vec::new();
    for placement in snapshot
        .placements
        .iter()
        .filter(|placement| placement.review_status == ColdBucketReviewStatus::Approved)
    {
        let Some(bucket) = bucket_map.bucket(&placement.bucket_id) else {
            continue;
        };
        if bucket.kind == ColdSemanticBucketKind::Misc && !query.include_misc {
            continue;
        }

        let bucket_score = score_terms(
            &terms,
            &[
                bucket.bucket_id.as_str(),
                bucket.label.as_str(),
                bucket.description.as_str(),
                bucket.sort_intent.as_str(),
            ],
        );
        let placement_score = score_terms(&terms, &[placement.reason.as_str()]);
        let fit_weight = match placement.fit {
            ColdBucketFit::Strong => 0.30,
            ColdBucketFit::Medium => 0.20,
            ColdBucketFit::Weak => 0.05,
            ColdBucketFit::Overflow => 0.01,
        };
        let score = (bucket_score * 2.0) + placement_score + fit_weight;
        if score <= 0.0 {
            continue;
        }

        let seen_key = format!(
            "{}:{}",
            bucket.bucket_id,
            placement.evidence_ref.stable_key()
        );
        if !seen.insert(seen_key) {
            continue;
        }

        hits.push(ColdSemanticHit {
            score,
            bucket_id: bucket.bucket_id.clone(),
            bucket_label: bucket.label.clone(),
            bucket_kind: bucket.kind,
            placement_id: placement.placement_id.clone(),
            placement_fit: placement.fit,
            placement_reason: placement.reason.clone(),
            evidence_ref: placement.evidence_ref.clone(),
        });
    }

    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.bucket_id.cmp(&right.bucket_id))
            .then_with(|| {
                left.evidence_ref
                    .stable_key()
                    .cmp(&right.evidence_ref.stable_key())
            })
    });
    hits.truncate(query.max_hits.max(1));
    Ok(hits)
}

pub fn query_semantic_index_store(
    paths: &ColdSemanticIndexPaths,
    query: &ColdSemanticQuery,
) -> Result<Vec<ColdSemanticHit>> {
    let snapshot = load_semantic_index_snapshot(paths)?;
    query_semantic_index(&snapshot, query)
}

pub fn render_semantic_query_preview(
    query: &ColdSemanticQuery,
    hits: &[ColdSemanticHit],
) -> String {
    let mut out = String::new();
    out.push_str("Semantic bucket query preview\n");
    out.push_str("Preview uses approved semantic placements only. It does not hydrate cold-log context, inject into Main AI, or promote memory.\n\n");
    out.push_str(&format!("query: {}\n", query.query.trim()));
    out.push_str(&format!(
        "include_misc: {}\nhits: {}\n\n",
        query.include_misc,
        hits.len()
    ));

    if hits.is_empty() {
        out.push_str("  - no semantic bucket hits\n");
        return out;
    }

    for (index, hit) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} [{}] score={:.3} kind={} fit={} placement={}\n",
            index + 1,
            hit.bucket_label,
            hit.bucket_id,
            hit.score,
            hit.bucket_kind.as_str(),
            hit.placement_fit.as_str(),
            hit.placement_id
        ));
        out.push_str(&format!("   reason: {}\n", hit.placement_reason));
        out.push_str(&format!(
            "   source: {} events {}..{}\n",
            hit.evidence_ref.source_log_path,
            hit.evidence_ref.start_event_index,
            hit.evidence_ref.end_event_index
        ));
    }

    out
}

pub fn render_semantic_receipts_overview(
    sort_receipts: &[ColdSemanticSortReceipt],
    placement_review_receipts: &[ColdBucketPlacementReviewReceipt],
    sift_receipts: &[ColdBucketSiftReceipt],
) -> String {
    let mut out = String::new();
    out.push_str("Semantic governance receipts\n");
    out.push_str(
        "Receipts are audit records only; they do not replace cold logs or current truth.\n\n",
    );
    out.push_str(&format!(
        "sort_receipts: {}\nplacement_review_receipts: {}\nsift_receipts: {}\n\n",
        sort_receipts.len(),
        placement_review_receipts.len(),
        sift_receipts.len()
    ));

    out.push_str("Sort receipts:\n");
    if sort_receipts.is_empty() {
        out.push_str("  - empty\n");
    } else {
        for receipt in sort_receipts.iter().rev() {
            out.push_str(&format!(
                "  - {} at {} outcome={} source={} cursor={:?} evidence={} proposed={} note={}\n",
                receipt.receipt_id,
                receipt.sorted_at_unix_ms,
                receipt.outcome.as_str(),
                receipt.source_log_path,
                receipt.last_seen_event_index,
                receipt.evidence_ref_count,
                receipt.proposed_placement_ids.len(),
                receipt.note
            ));
        }
    }

    out.push_str("\nPlacement review receipts:\n");
    if placement_review_receipts.is_empty() {
        out.push_str("  - empty\n");
    } else {
        for receipt in placement_review_receipts.iter().rev() {
            out.push_str(&format!(
                "  - reviewed_at={} approved={} rejected={} remaining_proposed={} remaining_needs_review={} transitions={}\n",
                receipt.reviewed_at_unix_ms,
                receipt.approved_placement_ids.len(),
                receipt.rejected_placement_ids.len(),
                receipt.remaining_proposed_count,
                receipt.remaining_needs_review_count,
                receipt.transitions.len()
            ));
            for transition in &receipt.transitions {
                out.push_str(&format!(
                    "      * {}: {} -> {}\n",
                    transition.placement_id,
                    transition.from_status.as_str(),
                    transition.to_status.as_str()
                ));
            }
        }
    }

    out.push_str("\nSift receipts:\n");
    if sift_receipts.is_empty() {
        out.push_str("  - empty\n");
    } else {
        for receipt in sift_receipts.iter().rev() {
            out.push_str(&format!(
                "  - {} proposal={} applied_at={} applied={} rejected={}\n",
                receipt.receipt_id,
                receipt.proposal_id,
                receipt.applied_at_unix_ms,
                receipt.applied_change_ids.len(),
                receipt.rejected_change_ids.len()
            ));
        }
    }

    out
}

pub fn render_semantic_bucket_placement_overview(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
) -> Result<String> {
    validate_placements_against_bucket_map(map, placements)?;

    let mut out = String::new();
    out.push_str("Semantic bucket placements\n");
    out.push_str("Index stores evidence refs and reasons only; transcript text is hydrated from cold logs.\n\n");
    out.push_str(&format!(
        "total_placements: {}\nactive_for_recall: {}\n\n",
        placements.len(),
        placements
            .iter()
            .filter(|placement| placement.review_status == ColdBucketReviewStatus::Approved)
            .count()
    ));

    for bucket in &map.buckets {
        let bucket_placements = placements_for_bucket(placements, &bucket.bucket_id);
        out.push_str(&format!(
            "{} [{}] - {} placement(s)\n",
            bucket.label,
            bucket.bucket_id,
            bucket_placements.len()
        ));
        if bucket_placements.is_empty() {
            out.push_str("  - empty\n\n");
            continue;
        }
        for placement in bucket_placements {
            out.push_str(&format!(
                "  - {} / {} / {} events {}..{}: {}\n",
                placement.review_status.as_str(),
                placement.fit.as_str(),
                placement.created_by.as_str(),
                placement.evidence_ref.start_event_index,
                placement.evidence_ref.end_event_index,
                placement.reason
            ));
        }
        out.push('\n');
    }

    Ok(out)
}

pub fn placements_for_bucket<'a>(
    placements: &'a [ColdBucketPlacement],
    bucket_id: &str,
) -> Vec<&'a ColdBucketPlacement> {
    placements
        .iter()
        .filter(|placement| placement.bucket_id == bucket_id)
        .collect()
}

pub fn placements_for_evidence_range<'a>(
    placements: &'a [ColdBucketPlacement],
    source_log_path: &str,
    start_event_index: usize,
    end_event_index: usize,
) -> Vec<&'a ColdBucketPlacement> {
    placements
        .iter()
        .filter(|placement| {
            placement.evidence_ref.source_log_path == source_log_path
                && placement.evidence_ref.start_event_index <= end_event_index
                && placement.evidence_ref.end_event_index >= start_event_index
        })
        .collect()
}

pub fn filter_semantic_review_placements(
    placements: &[ColdBucketPlacement],
    bucket_filter: &str,
    fit_filter: &str,
) -> Vec<ColdBucketPlacement> {
    let bucket_filter = bucket_filter.trim().to_ascii_lowercase();
    let fit_filter = fit_filter.trim().to_ascii_lowercase();
    placements
        .iter()
        .filter(|placement| {
            bucket_filter.is_empty()
                || placement
                    .bucket_id
                    .to_ascii_lowercase()
                    .contains(bucket_filter.as_str())
        })
        .filter(|placement| fit_filter.is_empty() || placement.fit.as_str() == fit_filter)
        .cloned()
        .collect()
}

pub fn semantic_review_queue_placements(
    placements: &[ColdBucketPlacement],
) -> Vec<ColdBucketPlacement> {
    placements
        .iter()
        .filter(|placement| is_semantic_review_queue_status(placement.review_status))
        .cloned()
        .collect()
}

pub fn is_semantic_review_queue_status(status: ColdBucketReviewStatus) -> bool {
    matches!(
        status,
        ColdBucketReviewStatus::Proposed | ColdBucketReviewStatus::NeedsReview
    )
}

pub fn build_bucket_sift_proposal(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
    bucket_id: &str,
    proposed_at_unix_ms: i64,
) -> Result<Option<ColdBucketSiftProposal>> {
    build_bucket_sift_proposal_with_feedback(map, placements, bucket_id, proposed_at_unix_ms, &[])
}

pub fn build_bucket_sift_proposal_with_feedback(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
    bucket_id: &str,
    proposed_at_unix_ms: i64,
    feedback_receipts: &[ColdMemoryFeedbackReceipt],
) -> Result<Option<ColdBucketSiftProposal>> {
    map.validate()?;
    validate_placements_against_bucket_map(map, placements)?;
    let Some(bucket) = map.bucket(bucket_id) else {
        anyhow::bail!("cannot sift missing semantic bucket '{bucket_id}'");
    };

    let mut seen_evidence = HashSet::new();
    let mut changed_placement_ids = HashSet::new();
    let mut changes = Vec::new();
    for (placement_index, placement) in placements
        .iter()
        .enumerate()
        .filter(|(_, placement)| placement.bucket_id == bucket_id)
        .filter(|(_, placement)| placement.review_status != ColdBucketReviewStatus::Rejected)
    {
        let evidence_key = placement.evidence_ref.stable_key();
        if !seen_evidence.insert(evidence_key.clone()) {
            changes.push(ColdBucketSiftChange {
                change_id: format!("change:{}:duplicate:{}", bucket_id, changes.len() + 1),
                action: ColdBucketSiftAction::RetirePlacement,
                target_placement_id: Some(placement.placement_id.clone()),
                target_placement_index: Some(placement_index),
                evidence_ref: placement.evidence_ref.clone(),
                from_bucket_id: Some(bucket_id.to_string()),
                to_bucket_id: None,
                before: format!("{bucket_id} contains duplicate evidence ref {evidence_key}."),
                after: format!(
                    "Retire duplicate placement {}; keep the earlier evidence ref.",
                    placement.placement_id
                ),
                rationale:
                    "Duplicate semantic placement adds bucket bloat without improving recall."
                        .to_string(),
                selected_for_apply: false,
            });
            changed_placement_ids.insert(placement.placement_id.clone());
            continue;
        }

        if bucket.kind == ColdSemanticBucketKind::Positive && placement.fit == ColdBucketFit::Weak {
            changes.push(ColdBucketSiftChange {
                change_id: format!("change:{}:weak_fit:{}", bucket_id, changes.len() + 1),
                action: ColdBucketSiftAction::MoveToOverflow,
                target_placement_id: Some(placement.placement_id.clone()),
                target_placement_index: Some(placement_index),
                evidence_ref: placement.evidence_ref.clone(),
                from_bucket_id: Some(bucket_id.to_string()),
                to_bucket_id: Some(map.misc_bucket_id.clone()),
                before: format!("{bucket_id} keeps weak-fit evidence ref {evidence_key}."),
                after: format!(
                    "{} receives weak-fit evidence ref {evidence_key} as overflow.",
                    map.misc_bucket_id
                ),
                rationale:
                    "Weak-fit evidence should stay findable without making a positive bucket noisy."
                        .to_string(),
                selected_for_apply: false,
            });
            changed_placement_ids.insert(placement.placement_id.clone());
            continue;
        }

        let feedback_summary = feedback_summary_for_placement(placement, feedback_receipts);
        if feedback_summary.noise + feedback_summary.misleading > 0
            && !changed_placement_ids.contains(&placement.placement_id)
        {
            changes.push(ColdBucketSiftChange {
                change_id: format!("change:{}:feedback:{}", bucket_id, changes.len() + 1),
                action: ColdBucketSiftAction::MarkStale,
                target_placement_id: Some(placement.placement_id.clone()),
                target_placement_index: Some(placement_index),
                evidence_ref: placement.evidence_ref.clone(),
                from_bucket_id: Some(bucket_id.to_string()),
                to_bucket_id: None,
                before: format!(
                    "{bucket_id} keeps evidence ref {evidence_key} active despite prior feedback."
                ),
                after: format!(
                    "Mark placement {} as needs_review; cold evidence remains intact.",
                    placement.placement_id
                ),
                rationale: format!(
                    "Prior Remember feedback for this bounded evidence: misleading={}, noise={}. User review is required before the placement changes.",
                    feedback_summary.misleading, feedback_summary.noise
                ),
                selected_for_apply: false,
            });
            changed_placement_ids.insert(placement.placement_id.clone());
        }
    }

    if changes.is_empty() {
        return Ok(None);
    }

    Ok(Some(ColdBucketSiftProposal {
        proposal_id: format!(
            "sift:{}:{}:{}",
            bucket_id,
            proposed_at_unix_ms,
            changes.len()
        ),
        bucket_id: bucket_id.to_string(),
        proposed_at_unix_ms,
        rationale: format!(
            "Review {bucket_id} for weak-fit, duplicate, or prior-feedback semantic placement issues. This proposal is inspectable and requires user approval before anything changes."
        ),
        changes,
    }))
}

#[derive(Default)]
struct PlacementFeedbackSummary {
    misleading: usize,
    noise: usize,
}

fn feedback_summary_for_placement(
    placement: &ColdBucketPlacement,
    feedback_receipts: &[ColdMemoryFeedbackReceipt],
) -> PlacementFeedbackSummary {
    let mut summary = PlacementFeedbackSummary::default();
    for receipt in feedback_receipts {
        if !feedback_receipt_matches_placement(receipt, placement) {
            continue;
        }
        match receipt.feedback {
            ColdMemoryCandidateFeedback::Misleading => summary.misleading += 1,
            ColdMemoryCandidateFeedback::Noise => summary.noise += 1,
            ColdMemoryCandidateFeedback::Stale => {}
        }
    }
    summary
}

fn feedback_receipt_matches_placement(
    receipt: &ColdMemoryFeedbackReceipt,
    placement: &ColdBucketPlacement,
) -> bool {
    feedback_source_matches_evidence_ref(
        &receipt.candidate.source_log_path,
        &placement.evidence_ref.source_log_path,
    ) && placement.evidence_ref.start_event_index >= receipt.candidate.context_start_event_index
        && placement.evidence_ref.end_event_index <= receipt.candidate.context_end_event_index
}

fn feedback_source_matches_evidence_ref(feedback_source: &str, evidence_source: &str) -> bool {
    if feedback_source == evidence_source {
        return true;
    }
    let normalized_feedback = feedback_source.replace('\\', "/");
    let normalized_evidence = evidence_source.replace('\\', "/");
    normalized_feedback.ends_with(&format!("/{}", normalized_evidence))
}

pub fn apply_selected_sift_changes(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
    proposal: &ColdBucketSiftProposal,
    applied_at_unix_ms: i64,
) -> Result<(Vec<ColdBucketPlacement>, ColdBucketSiftReceipt)> {
    map.validate()?;
    validate_placements_against_bucket_map(map, placements)?;

    let mut updated = placements.to_vec();
    let mut applied_change_ids = Vec::new();
    let mut rejected_change_ids = Vec::new();

    for change in &proposal.changes {
        if !change.selected_for_apply {
            rejected_change_ids.push(change.change_id.clone());
            continue;
        }

        let applied = match change.action {
            ColdBucketSiftAction::MoveToOverflow => {
                apply_move_to_overflow(map, &mut updated, change)?
            }
            ColdBucketSiftAction::RetirePlacement => retire_target_placement(&mut updated, change),
            ColdBucketSiftAction::MarkStale => {
                mark_target_placement_needs_review(&mut updated, change)
            }
            ColdBucketSiftAction::MergeDuplicate
            | ColdBucketSiftAction::MoveToBucket
            | ColdBucketSiftAction::PreserveTemporalRef => false,
        };

        if applied {
            applied_change_ids.push(change.change_id.clone());
        } else {
            rejected_change_ids.push(change.change_id.clone());
        }
    }

    validate_placements_against_bucket_map(map, &updated)?;
    Ok((
        updated,
        ColdBucketSiftReceipt {
            receipt_id: format!(
                "receipt:{}:{}:{}",
                proposal.proposal_id,
                applied_at_unix_ms,
                applied_change_ids.len()
            ),
            proposal_id: proposal.proposal_id.clone(),
            applied_at_unix_ms,
            applied_change_ids,
            rejected_change_ids,
        },
    ))
}

fn apply_move_to_overflow(
    map: &ColdSemanticBucketMap,
    placements: &mut Vec<ColdBucketPlacement>,
    change: &ColdBucketSiftChange,
) -> Result<bool> {
    let Some(to_bucket_id) = change.to_bucket_id.as_deref() else {
        anyhow::bail!(
            "move-to-overflow change '{}' has no target bucket",
            change.change_id
        );
    };
    if to_bucket_id != map.misc_bucket_id {
        anyhow::bail!(
            "move-to-overflow change '{}' targets '{}' instead of misc bucket '{}'",
            change.change_id,
            to_bucket_id,
            map.misc_bucket_id
        );
    }

    let Some(source_index) = find_target_placement_index(placements, change) else {
        return Ok(false);
    };
    placements[source_index].review_status = ColdBucketReviewStatus::Rejected;

    if let Some(existing_index) = placements.iter().position(|placement| {
        placement.bucket_id == to_bucket_id
            && placement.evidence_ref == change.evidence_ref
            && placement.review_status != ColdBucketReviewStatus::Rejected
    }) {
        placements[existing_index].fit = ColdBucketFit::Overflow;
        placements[existing_index].review_status = ColdBucketReviewStatus::Approved;
        return Ok(true);
    }

    let mut overflow = ColdBucketPlacement::new(
        to_bucket_id,
        change.evidence_ref.clone(),
        ColdBucketFit::Overflow,
        change.rationale.clone(),
        ColdBucketPlacementAuthor::Host,
    );
    overflow.review_status = ColdBucketReviewStatus::Approved;
    placements.push(overflow);
    Ok(true)
}

fn retire_target_placement(
    placements: &mut [ColdBucketPlacement],
    change: &ColdBucketSiftChange,
) -> bool {
    let Some(index) = find_target_placement_index(placements, change) else {
        return false;
    };
    placements[index].review_status = ColdBucketReviewStatus::Rejected;
    true
}

fn mark_target_placement_needs_review(
    placements: &mut [ColdBucketPlacement],
    change: &ColdBucketSiftChange,
) -> bool {
    let Some(index) = find_target_placement_index(placements, change) else {
        return false;
    };
    placements[index].review_status = ColdBucketReviewStatus::NeedsReview;
    true
}

fn find_target_placement_index(
    placements: &[ColdBucketPlacement],
    change: &ColdBucketSiftChange,
) -> Option<usize> {
    if let Some(target_placement_index) = change.target_placement_index {
        let placement = placements.get(target_placement_index)?;
        if placement.evidence_ref == change.evidence_ref
            && change
                .from_bucket_id
                .as_deref()
                .is_none_or(|bucket_id| placement.bucket_id == bucket_id)
            && change
                .target_placement_id
                .as_deref()
                .is_none_or(|placement_id| placement.placement_id == placement_id)
        {
            return Some(target_placement_index);
        }
    }

    if let Some(target_placement_id) = change.target_placement_id.as_deref() {
        return placements
            .iter()
            .position(|placement| placement.placement_id == target_placement_id);
    }

    placements.iter().position(|placement| {
        placement.evidence_ref == change.evidence_ref
            && change
                .from_bucket_id
                .as_deref()
                .is_none_or(|bucket_id| placement.bucket_id == bucket_id)
    })
}

fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|term| {
            let term = term.trim().to_ascii_lowercase();
            if term.len() < 2 || !seen.insert(term.clone()) {
                return None;
            }
            Some(term)
        })
        .collect()
}

fn score_terms(terms: &[String], fields: &[&str]) -> f32 {
    let searchable = fields
        .iter()
        .map(|field| field.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    terms
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count() as f32
        / terms.len().max(1) as f32
}

pub fn validate_placements_against_bucket_map(
    map: &ColdSemanticBucketMap,
    placements: &[ColdBucketPlacement],
) -> Result<()> {
    map.validate()?;
    for placement in placements {
        let Some(bucket) = map.bucket(&placement.bucket_id) else {
            anyhow::bail!(
                "semantic placement '{}' references missing bucket '{}'",
                placement.placement_id,
                placement.bucket_id
            );
        };
        if placement.fit == ColdBucketFit::Overflow && bucket.kind != ColdSemanticBucketKind::Misc {
            anyhow::bail!(
                "overflow semantic placement '{}' must target misc bucket '{}'",
                placement.placement_id,
                map.misc_bucket_id
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::governor::{ColdMemoryAttachedCandidateReceipt, ColdMemoryRecallTrace};

    mod tempfile_path {
        use std::path::PathBuf;

        pub struct TempStore {
            pub dir: PathBuf,
        }

        impl TempStore {
            pub fn new() -> Self {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock should be after unix epoch")
                    .as_nanos();
                Self {
                    dir: std::env::temp_dir().join(format!(
                        "chattycog-semantic-index-test-{}-{nonce}",
                        std::process::id()
                    )),
                }
            }
        }

        impl Drop for TempStore {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }

    #[test]
    fn evidence_ref_stable_key_points_to_log_range_not_text() {
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 12, 18);

        assert_eq!(
            evidence.stable_key(),
            "interaction_log:memory/cold_log.jsonl:12:18"
        );
        assert_eq!(evidence.source_kind, ColdEvidenceSourceKind::InteractionLog);
    }

    #[test]
    fn placement_defaults_to_proposed_review_status() {
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 40, 42);

        let placement = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Strong,
            "The exchange explicitly mentions Miso.",
            ColdBucketPlacementAuthor::Sorter,
        );

        assert_eq!(placement.bucket_id, "entity:miso");
        assert_eq!(placement.evidence_ref, evidence);
        assert_eq!(placement.review_status, ColdBucketReviewStatus::Proposed);
        assert!(placement.placement_id.contains("entity:miso"));
    }

    fn bucket(bucket_id: &str, label: &str) -> ColdSemanticBucket {
        ColdSemanticBucket::positive(
            bucket_id,
            label,
            format!("{label} bucket"),
            "test sort intent",
        )
    }

    fn feedback_receipt_for(
        placement: &ColdBucketPlacement,
        source_log_path: &str,
        feedback: ColdMemoryCandidateFeedback,
    ) -> ColdMemoryFeedbackReceipt {
        ColdMemoryFeedbackReceipt {
            recall_id: Some("remember-recall-test".to_string()),
            marked_at_unix_ms: 123,
            query: "miso".to_string(),
            feedback,
            candidate: ColdMemoryAttachedCandidateReceipt {
                candidate_id: "candidate".to_string(),
                title: "candidate".to_string(),
                recall_trace: ColdMemoryRecallTrace::semantic(&ColdSemanticHit {
                    score: 1.0,
                    bucket_id: placement.bucket_id.clone(),
                    bucket_label: placement.bucket_id.clone(),
                    bucket_kind: ColdSemanticBucketKind::Positive,
                    placement_id: placement.placement_id.clone(),
                    placement_fit: placement.fit,
                    placement_reason: placement.reason.clone(),
                    evidence_ref: placement.evidence_ref.clone(),
                }),
                source_log_path: source_log_path.to_string(),
                matched_event_index: placement.evidence_ref.end_event_index,
                context_start_event_index: placement
                    .evidence_ref
                    .start_event_index
                    .saturating_sub(2),
                context_end_event_index: placement.evidence_ref.end_event_index + 2,
                matched_ts_unix_ms: 123,
                context_start_ts_unix_ms: Some(120),
                context_end_ts_unix_ms: Some(130),
            },
            note: "test feedback".to_string(),
        }
    }

    #[test]
    fn sift_change_is_explicitly_user_selectable() {
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 9, 11);
        let change = ColdBucketSiftChange {
            change_id: "change-1".to_string(),
            action: ColdBucketSiftAction::MoveToOverflow,
            target_placement_id: None,
            target_placement_index: None,
            evidence_ref: evidence,
            from_bucket_id: Some("topic:pet_health".to_string()),
            to_bucket_id: Some("overflow".to_string()),
            before: "topic:pet_health contains a weak weather aside".to_string(),
            after: "overflow contains the weak weather aside for review".to_string(),
            rationale: "The placement is weak and not directly about pet health.".to_string(),
            selected_for_apply: false,
        };

        assert!(!change.selected_for_apply);
        assert_eq!(change.action, ColdBucketSiftAction::MoveToOverflow);
    }

    #[test]
    fn semantic_index_paths_live_under_cold_semantic_folder() {
        let paths = ColdSemanticIndexPaths::under_memory_dir("memory");

        assert_eq!(
            paths.root_dir,
            PathBuf::from("memory").join("cold_semantic")
        );
        assert_eq!(
            paths.bucket_map_json,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("bucket_map.json")
        );
        assert_eq!(
            paths.placements_jsonl,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("placements.jsonl")
        );
        assert_eq!(
            paths.sort_receipts_jsonl,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("sort_receipts.jsonl")
        );
        assert_eq!(
            paths.placement_review_receipts_jsonl,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("placement_review_receipts.jsonl")
        );
        assert_eq!(
            paths.sift_proposals_jsonl,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("sift_proposals.jsonl")
        );
        assert_eq!(
            paths.sift_receipts_jsonl,
            PathBuf::from("memory")
                .join("cold_semantic")
                .join("sift_receipts.jsonl")
        );
    }

    #[test]
    fn placement_jsonl_roundtrip_preserves_refs_without_cold_log_text() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104);
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            evidence,
            ColdBucketFit::Strong,
            "The exchange is about Miso.",
            ColdBucketPlacementAuthor::User,
        );

        append_placement_jsonl(&paths.placements_jsonl, &placement).unwrap();
        let placements = read_placements_jsonl(&paths.placements_jsonl).unwrap();

        assert_eq!(placements, vec![placement]);
        let raw = std::fs::read_to_string(&paths.placements_jsonl).unwrap();
        assert!(raw.contains("\"bucket_id\":\"entity:miso\""));
        assert!(!raw.contains("Exact historical transcript"));
    }

    #[test]
    fn write_placements_jsonl_replaces_current_snapshot() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let first = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "The exchange is about Miso.",
            ColdBucketPlacementAuthor::User,
        );
        let replacement = ColdBucketPlacement::new(
            "topic:pet_health",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 120, 124),
            ColdBucketFit::Medium,
            "The exchange is about pet health.",
            ColdBucketPlacementAuthor::Sorter,
        );

        write_placements_jsonl(&paths.placements_jsonl, std::slice::from_ref(&first)).unwrap();
        write_placements_jsonl(&paths.placements_jsonl, std::slice::from_ref(&replacement))
            .unwrap();

        assert_eq!(
            read_placements_jsonl(&paths.placements_jsonl).unwrap(),
            vec![replacement]
        );
    }

    #[test]
    fn bucket_map_json_roundtrip_preserves_frozen_bucket_count() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        assert!(
            read_bucket_map_json(&paths.bucket_map_json)
                .unwrap()
                .is_none()
        );

        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs into entity and topic buckets",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                bucket("topic:pet_health", "Pet health"),
                ColdSemanticBucket::misc("test sort intent"),
            ],
        );

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        let restored = read_bucket_map_json(&paths.bucket_map_json)
            .unwrap()
            .unwrap();

        assert_eq!(restored, map);
        assert_eq!(restored.positive_bucket_count, 2);
        assert_eq!(restored.bucket_count, 3);
        assert!(restored.bucket("entity:miso").is_some());
        assert!(restored.misc_bucket().is_some());
        assert!(restored.bucket("missing").is_none());
    }

    #[test]
    fn default_interaction_bucket_map_is_small_frozen_positive_set_plus_misc() {
        let map = default_interaction_bucket_map(123);

        map.validate().unwrap();
        assert_eq!(map.map_id, "interaction-bucket-map-v1");
        assert_eq!(map.created_at_unix_ms, 123);
        assert_eq!(map.positive_bucket_count, 4);
        assert_eq!(map.bucket_count, 5);
        assert_eq!(map.misc_bucket_id, DEFAULT_MISC_BUCKET_ID);
        assert!(map.bucket("entity:people_places_pets").is_some());
        assert!(map.bucket("topic:projects_code").is_some());
        assert!(map.bucket("topic:preferences_workflow").is_some());
        assert!(map.bucket("topic:open_threads").is_some());
        assert_eq!(
            map.misc_bucket().map(|bucket| bucket.kind),
            Some(ColdSemanticBucketKind::Misc)
        );
    }

    #[test]
    fn create_default_interaction_bucket_map_if_missing_never_overwrites_existing_map() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);

        let created = create_default_interaction_bucket_map_if_missing(&paths.bucket_map_json, 123)
            .unwrap()
            .expect("starter map should be created");
        assert_eq!(created.map_id, "interaction-bucket-map-v1");

        let existing = ColdSemanticBucketMap::new(
            "custom-map",
            456,
            "custom frozen user map",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("topic:custom", "Custom"),
                ColdSemanticBucket::misc("custom misc"),
            ],
        );
        write_bucket_map_json(&paths.bucket_map_json, &existing).unwrap();

        let second =
            create_default_interaction_bucket_map_if_missing(&paths.bucket_map_json, 789).unwrap();
        assert!(second.is_none());
        let restored = read_bucket_map_json(&paths.bucket_map_json)
            .unwrap()
            .unwrap();
        assert_eq!(restored, existing);
    }

    #[test]
    fn semantic_index_snapshot_loads_empty_missing_store() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);

        let snapshot = load_semantic_index_snapshot(&paths).unwrap();

        assert_eq!(snapshot, ColdSemanticIndexSnapshot::empty());
        assert!(snapshot.validate().is_ok());
    }

    #[test]
    fn semantic_index_snapshot_rejects_orphan_placements() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 1, 2),
            ColdBucketFit::Strong,
            "Miso is directly named.",
            ColdBucketPlacementAuthor::User,
        );

        append_placement_jsonl(&paths.placements_jsonl, &placement).unwrap();

        assert!(load_semantic_index_snapshot(&paths).is_err());
    }

    #[test]
    fn semantic_index_snapshot_validates_map_and_placements_together() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 1, 2),
            ColdBucketFit::Strong,
            "Miso is directly named.",
            ColdBucketPlacementAuthor::User,
        );

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        append_placement_jsonl(&paths.placements_jsonl, &placement).unwrap();
        let snapshot = load_semantic_index_snapshot(&paths).unwrap();

        assert_eq!(snapshot.bucket_map, Some(map));
        assert_eq!(snapshot.placements, vec![placement]);
    }

    #[test]
    fn validated_append_requires_bucket_map_and_known_bucket() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 1, 2);
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Strong,
            "Miso is directly named.",
            ColdBucketPlacementAuthor::User,
        );

        assert!(append_validated_placement_jsonl(&paths, &placement).is_err());

        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        append_validated_placement_jsonl(&paths, &placement).unwrap();

        let missing_bucket = ColdBucketPlacement::new(
            "missing",
            evidence,
            ColdBucketFit::Strong,
            "Unknown bucket.",
            ColdBucketPlacementAuthor::User,
        );
        assert!(append_validated_placement_jsonl(&paths, &missing_bucket).is_err());

        let placements = read_placements_jsonl(&paths.placements_jsonl).unwrap();
        assert_eq!(placements, vec![placement]);
    }

    #[test]
    fn placement_proposal_builder_creates_proposed_validated_placements() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 10, 12);
        let decisions = vec![ColdBucketPlacementDecision::new(
            "entity:miso",
            ColdBucketFit::Strong,
            "Sorter identified a direct Miso care exchange.",
            ColdBucketPlacementAuthor::Sorter,
        )];

        let placements = build_semantic_placement_proposals(&map, &evidence, &decisions).unwrap();

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].bucket_id, "entity:miso");
        assert_eq!(placements[0].evidence_ref, evidence);
        assert_eq!(placements[0].fit, ColdBucketFit::Strong);
        assert_eq!(placements[0].created_by, ColdBucketPlacementAuthor::Sorter);
        assert_eq!(
            placements[0].review_status,
            ColdBucketReviewStatus::Proposed
        );
    }

    #[test]
    fn placement_proposal_builder_rejects_missing_bucket_and_bad_overflow_targets() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 10, 12);

        let missing = vec![ColdBucketPlacementDecision::new(
            "missing",
            ColdBucketFit::Strong,
            "Unknown bucket.",
            ColdBucketPlacementAuthor::Sorter,
        )];
        assert!(build_semantic_placement_proposals(&map, &evidence, &missing).is_err());

        let wrong_overflow = vec![ColdBucketPlacementDecision::new(
            "entity:miso",
            ColdBucketFit::Overflow,
            "Overflow cannot target a positive bucket.",
            ColdBucketPlacementAuthor::Sorter,
        )];
        assert!(build_semantic_placement_proposals(&map, &evidence, &wrong_overflow).is_err());

        let misc_not_overflow = vec![ColdBucketPlacementDecision::new(
            DEFAULT_MISC_BUCKET_ID,
            ColdBucketFit::Weak,
            "Misc placements should be overflow.",
            ColdBucketPlacementAuthor::Sorter,
        )];
        assert!(build_semantic_placement_proposals(&map, &evidence, &misc_not_overflow).is_err());
    }

    #[test]
    fn append_placement_proposals_to_store_dedupes_active_bucket_evidence_refs() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 10, 12);
        let decisions = vec![ColdBucketPlacementDecision::new(
            "entity:miso",
            ColdBucketFit::Strong,
            "Sorter identified a direct Miso care exchange.",
            ColdBucketPlacementAuthor::Sorter,
        )];

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        let first =
            append_semantic_placement_proposals_to_store(&paths, &evidence, &decisions).unwrap();
        let second =
            append_semantic_placement_proposals_to_store(&paths, &evidence, &decisions).unwrap();

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
        assert_eq!(
            read_placements_jsonl(&paths.placements_jsonl).unwrap(),
            first
        );
    }

    #[test]
    fn semantic_sort_receipt_records_cursor_and_proposed_placement_ids_without_text() {
        let evidence = vec![
            ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 10, 12),
            ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 13, 14),
        ];
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            evidence[0].clone(),
            ColdBucketFit::Strong,
            "Sorter identified a Miso exchange.",
            ColdBucketPlacementAuthor::Sorter,
        );

        let receipt = ColdSemanticSortReceipt::from_run(
            "sort-1",
            123,
            "cold_log.jsonl",
            &evidence,
            std::slice::from_ref(&placement),
            "Filed recent interaction exchanges.",
        );

        assert_eq!(receipt.last_seen_event_index, Some(14));
        assert_eq!(receipt.evidence_ref_count, 2);
        assert_eq!(receipt.proposed_placement_ids, vec![placement.placement_id]);
        assert!(
            !serde_json::to_string(&receipt)
                .unwrap()
                .contains("Exact historical transcript")
        );
    }

    #[test]
    fn semantic_receipts_overview_renders_empty_audit_trail() {
        let overview = render_semantic_receipts_overview(&[], &[], &[]);

        assert!(overview.contains("Semantic governance receipts"));
        assert!(overview.contains("sort_receipts: 0"));
        assert!(overview.contains("Placement review receipts:\n  - empty"));
        assert!(overview.contains("Sift receipts:\n  - empty"));
    }

    #[test]
    fn semantic_receipts_overview_renders_receipt_counts_without_transcript_text() {
        let evidence = vec![ColdEvidenceRef::interaction_log_range(
            "cold_log.jsonl",
            10,
            12,
        )];
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            evidence[0].clone(),
            ColdBucketFit::Strong,
            "Sorter identified a Miso exchange.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let sort_receipt = ColdSemanticSortReceipt::from_run(
            "sort-1",
            123,
            "cold_log.jsonl",
            &evidence,
            std::slice::from_ref(&placement),
            "Filed recent interaction exchanges.",
        );
        let review_receipt = ColdBucketPlacementReviewReceipt {
            reviewed_at_unix_ms: 456,
            approved_placement_ids: vec![placement.placement_id],
            rejected_placement_ids: Vec::new(),
            remaining_proposed_count: 2,
            remaining_needs_review_count: 1,
            transitions: Vec::new(),
        };
        let sift_receipt = ColdBucketSiftReceipt {
            receipt_id: "sift-1".to_string(),
            proposal_id: "proposal-1".to_string(),
            applied_at_unix_ms: 789,
            applied_change_ids: vec!["change-1".to_string()],
            rejected_change_ids: vec!["change-2".to_string()],
        };

        let overview =
            render_semantic_receipts_overview(&[sort_receipt], &[review_receipt], &[sift_receipt]);

        assert!(overview.contains("sort_receipts: 1"));
        assert!(overview.contains("placement_review_receipts: 1"));
        assert!(overview.contains("sift_receipts: 1"));
        assert!(overview.contains("sort-1 at 123 outcome=committed source=cold_log.jsonl"));
        assert!(overview.contains("reviewed_at=456 approved=1 rejected=0"));
        assert!(overview.contains("remaining_needs_review=1"));
        assert!(overview.contains("sift-1 proposal=proposal-1 applied_at=789"));
        assert!(!overview.contains("User:"));
        assert!(!overview.contains("Assistant:"));
    }

    #[test]
    fn semantic_sort_receipt_jsonl_roundtrip_and_cursor_lookup() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let first = ColdSemanticSortReceipt::from_run(
            "sort-1",
            100,
            "cold_log.jsonl",
            &[ColdEvidenceRef::interaction_log_range(
                "cold_log.jsonl",
                1,
                4,
            )],
            &[],
            "First pass.",
        );
        let second = ColdSemanticSortReceipt::from_run(
            "sort-2",
            200,
            "cold_log.jsonl",
            &[ColdEvidenceRef::interaction_log_range(
                "cold_log.jsonl",
                5,
                9,
            )],
            &[],
            "Second pass.",
        );
        let other = ColdSemanticSortReceipt::from_run(
            "sort-other",
            300,
            "other_cold_log.jsonl",
            &[ColdEvidenceRef::interaction_log_range(
                "other_cold_log.jsonl",
                100,
                104,
            )],
            &[],
            "Other source.",
        );

        append_semantic_sort_receipt_to_store(&paths, &first).unwrap();
        append_semantic_sort_receipt_to_store(&paths, &second).unwrap();
        append_semantic_sort_receipt_to_store(&paths, &other).unwrap();
        let receipts = read_sort_receipts_jsonl(&paths.sort_receipts_jsonl).unwrap();

        assert_eq!(receipts, vec![first, second, other]);
        assert_eq!(
            latest_sort_cursor_for_source(&receipts, "cold_log.jsonl"),
            Some(9)
        );
        assert_eq!(
            latest_sort_cursor_for_store(&paths, "cold_log.jsonl").unwrap(),
            Some(9)
        );
        assert_eq!(
            latest_sort_cursor_for_store(&paths, "missing.jsonl").unwrap(),
            None
        );
    }

    #[test]
    fn commit_semantic_sort_run_appends_proposals_and_receipt_with_cursor() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();

        let items = vec![
            ColdSemanticSortItem::new(
                ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 10, 12),
                vec![ColdBucketPlacementDecision::new(
                    "entity:miso",
                    ColdBucketFit::Strong,
                    "Sorter identified a direct Miso care exchange.",
                    ColdBucketPlacementAuthor::Sorter,
                )],
            ),
            ColdSemanticSortItem::new(
                ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 13, 14),
                Vec::new(),
            ),
        ];

        let receipt = commit_semantic_sort_run_to_store(
            &paths,
            "sort-commit-1",
            999,
            "cold_log.jsonl",
            &items,
            "Committed sorter proposals.",
        )
        .unwrap();

        let placements = read_placements_jsonl(&paths.placements_jsonl).unwrap();
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].bucket_id, "entity:miso");
        assert_eq!(
            placements[0].review_status,
            ColdBucketReviewStatus::Proposed
        );
        assert_eq!(receipt.last_seen_event_index, Some(14));
        assert_eq!(receipt.evidence_ref_count, 2);
        assert_eq!(
            receipt.proposed_placement_ids,
            vec![placements[0].placement_id.clone()]
        );
        assert_eq!(
            read_sort_receipts_jsonl(&paths.sort_receipts_jsonl).unwrap(),
            vec![receipt]
        );
    }

    #[test]
    fn commit_semantic_sort_run_dedupes_repeated_sorter_passes_but_records_cursor() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();

        let items = vec![ColdSemanticSortItem::new(
            ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 10, 12),
            vec![ColdBucketPlacementDecision::new(
                "entity:miso",
                ColdBucketFit::Strong,
                "Sorter identified a direct Miso care exchange.",
                ColdBucketPlacementAuthor::Sorter,
            )],
        )];

        let first = commit_semantic_sort_run_to_store(
            &paths,
            "sort-commit-1",
            999,
            "cold_log.jsonl",
            &items,
            "First pass.",
        )
        .unwrap();
        let second = commit_semantic_sort_run_to_store(
            &paths,
            "sort-commit-2",
            1000,
            "cold_log.jsonl",
            &items,
            "Second pass.",
        )
        .unwrap();

        assert_eq!(
            read_placements_jsonl(&paths.placements_jsonl)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(first.proposed_placement_ids.len(), 1);
        assert!(second.proposed_placement_ids.is_empty());
        assert_eq!(
            latest_sort_cursor_for_store(&paths, "cold_log.jsonl").unwrap(),
            Some(12)
        );
    }

    #[test]
    fn commit_semantic_sort_run_prevalidates_source_before_writing() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();

        let items = vec![ColdSemanticSortItem::new(
            ColdEvidenceRef::interaction_log_range("other_cold_log.jsonl", 10, 12),
            vec![ColdBucketPlacementDecision::new(
                "entity:miso",
                ColdBucketFit::Strong,
                "Wrong source.",
                ColdBucketPlacementAuthor::Sorter,
            )],
        )];

        assert!(
            commit_semantic_sort_run_to_store(
                &paths,
                "sort-bad",
                999,
                "cold_log.jsonl",
                &items,
                "Bad source.",
            )
            .is_err()
        );
        assert!(
            read_placements_jsonl(&paths.placements_jsonl)
                .unwrap()
                .is_empty()
        );
        assert!(
            read_sort_receipts_jsonl(&paths.sort_receipts_jsonl)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn bucket_map_requires_exactly_one_misc_bucket() {
        let no_misc = ColdSemanticBucketMap::new(
            "map-no-misc",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![bucket("entity:miso", "Miso")],
        );
        assert!(no_misc.validate().is_err());

        let two_misc = ColdSemanticBucketMap::new(
            "map-two-misc",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
                ColdSemanticBucket {
                    bucket_id: "misc-2".to_string(),
                    ..ColdSemanticBucket::misc("sort interaction logs")
                },
            ],
        );
        assert!(two_misc.validate().is_err());
    }

    #[test]
    fn placement_queries_filter_by_bucket_and_overlapping_evidence_range() {
        let miso_evidence =
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104);
        let vet_evidence =
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 103, 108);
        let other_log_evidence =
            ColdEvidenceRef::interaction_log_range("memory/other_cold_log.jsonl", 100, 104);
        let placements = vec![
            ColdBucketPlacement::new(
                "entity:miso",
                miso_evidence,
                ColdBucketFit::Strong,
                "Miso is directly named.",
                ColdBucketPlacementAuthor::User,
            ),
            ColdBucketPlacement::new(
                "topic:pet_health",
                vet_evidence,
                ColdBucketFit::Medium,
                "Vet context is discussed.",
                ColdBucketPlacementAuthor::Sorter,
            ),
            ColdBucketPlacement::new(
                "entity:miso",
                other_log_evidence,
                ColdBucketFit::Weak,
                "Different imported log also mentions Miso.",
                ColdBucketPlacementAuthor::Imported,
            ),
        ];

        let miso = placements_for_bucket(&placements, "entity:miso");
        assert_eq!(miso.len(), 2);

        let overlapping =
            placements_for_evidence_range(&placements, "memory/cold_log.jsonl", 104, 105);
        assert_eq!(overlapping.len(), 2);
        assert!(overlapping.iter().any(|p| p.bucket_id == "entity:miso"));
        assert!(
            overlapping
                .iter()
                .any(|p| p.bucket_id == "topic:pet_health")
        );

        let exact_other =
            placements_for_evidence_range(&placements, "memory/other_cold_log.jsonl", 100, 104);
        assert_eq!(exact_other.len(), 1);
        assert_eq!(
            exact_other[0].created_by,
            ColdBucketPlacementAuthor::Imported
        );
    }

    #[test]
    fn review_placement_filter_matches_bucket_and_fit() {
        let placements = vec![
            ColdBucketPlacement::new(
                "entity:miso",
                ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
                ColdBucketFit::Strong,
                "Miso care.",
                ColdBucketPlacementAuthor::Sorter,
            ),
            ColdBucketPlacement::new(
                "topic:build",
                ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 200, 204),
                ColdBucketFit::Weak,
                "Build aside.",
                ColdBucketPlacementAuthor::Sorter,
            ),
            ColdBucketPlacement::new(
                DEFAULT_MISC_BUCKET_ID,
                ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 300, 304),
                ColdBucketFit::Overflow,
                "Weak fit.",
                ColdBucketPlacementAuthor::Sorter,
            ),
        ];

        let miso = filter_semantic_review_placements(&placements, "miso", "");
        assert_eq!(miso.len(), 1);
        assert_eq!(miso[0].bucket_id, "entity:miso");

        let weak = filter_semantic_review_placements(&placements, "", "weak");
        assert_eq!(weak.len(), 1);
        assert_eq!(weak[0].bucket_id, "topic:build");

        let none = filter_semantic_review_placements(&placements, "miso", "overflow");
        assert!(none.is_empty());
    }

    #[test]
    fn bucket_placement_overview_groups_refs_without_transcript_text() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc bucket"),
            ],
        );
        let mut rejected = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 20, 22),
            ColdBucketFit::Weak,
            "Duplicate stale placement.",
            ColdBucketPlacementAuthor::Sorter,
        );
        rejected.review_status = ColdBucketReviewStatus::Rejected;
        let mut approved = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 10, 12),
            ColdBucketFit::Strong,
            "Direct Miso care context.",
            ColdBucketPlacementAuthor::Sorter,
        );
        approved.review_status = ColdBucketReviewStatus::Approved;
        let placements = vec![approved, rejected];

        let overview = render_semantic_bucket_placement_overview(&map, &placements).unwrap();

        assert!(overview.contains("total_placements: 2"));
        assert!(overview.contains("active_for_recall: 1"));
        assert!(overview.contains("Miso [entity:miso] - 2 placement(s)"));
        assert!(overview.contains("approved / strong / sorter events 10..12"));
        assert!(overview.contains("rejected / weak / sorter events 20..22"));
        assert!(!overview.contains("User:"));
        assert!(!overview.contains("Assistant:"));
    }

    #[test]
    fn bucket_placement_overview_rejects_unknown_bucket_refs() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc"),
            ],
        );
        let placement = ColdBucketPlacement::new(
            "missing",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 10, 12),
            ColdBucketFit::Strong,
            "Bad bucket ref.",
            ColdBucketPlacementAuthor::Sorter,
        );

        assert!(render_semantic_bucket_placement_overview(&map, &[placement]).is_err());
    }

    #[test]
    fn query_semantic_index_returns_ranked_evidence_refs_without_transcript_text() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                ColdSemanticBucket::positive(
                    "entity:miso",
                    "Miso",
                    "Cat identity and care context",
                    "sort interaction logs",
                ),
                ColdSemanticBucket::positive(
                    "topic:build",
                    "Build Work",
                    "Implementation details and code tasks",
                    "sort interaction logs",
                ),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let mut miso = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso vet notes and care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        miso.review_status = ColdBucketReviewStatus::Approved;
        let mut build = ColdBucketPlacement::new(
            "topic:build",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 200, 204),
            ColdBucketFit::Strong,
            "Rust implementation detail.",
            ColdBucketPlacementAuthor::Sorter,
        );
        build.review_status = ColdBucketReviewStatus::Approved;
        let placements = vec![miso, build];
        let snapshot = ColdSemanticIndexSnapshot {
            bucket_map: Some(map),
            placements,
        };

        let hits = query_semantic_index(
            &snapshot,
            &ColdSemanticQuery {
                query: "miso cat vet".to_string(),
                max_hits: 8,
                include_misc: true,
            },
        )
        .unwrap();

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].bucket_id, "entity:miso");
        assert_eq!(hits[0].bucket_label, "Miso");
        assert_eq!(hits[0].evidence_ref.start_event_index, 100);
        assert!(
            !hits[0]
                .placement_reason
                .contains("Exact historical transcript")
        );
    }

    #[test]
    fn semantic_query_preview_renders_policy_without_hydrating_context() {
        let hit = ColdSemanticHit {
            score: 2.31,
            bucket_id: "entity:miso".to_string(),
            bucket_label: "Miso".to_string(),
            bucket_kind: ColdSemanticBucketKind::Positive,
            placement_id: "placement:one".to_string(),
            placement_fit: ColdBucketFit::Strong,
            placement_reason: "Miso vet notes and care details.".to_string(),
            evidence_ref: ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
        };
        let query = ColdSemanticQuery::remember_current_input("miso vet");

        let preview = render_semantic_query_preview(&query, &[hit]);

        assert!(preview.contains("Semantic bucket query preview"));
        assert!(preview.contains("approved semantic placements only"));
        assert!(preview.contains("does not hydrate cold-log context"));
        assert!(preview.contains("include_misc: true"));
        assert!(preview.contains("Miso [entity:miso]"));
        assert!(preview.contains("fit=strong"));
        assert!(preview.contains("source: memory/cold_log.jsonl events 100..104"));
        assert!(!preview.contains("USER:"));
        assert!(!preview.contains("ASSISTANT:"));
    }

    #[test]
    fn semantic_query_preview_renders_empty_hits_and_misc_policy() {
        let mut query = ColdSemanticQuery::remember_current_input("nothing");
        query.include_misc = false;

        let preview = render_semantic_query_preview(&query, &[]);

        assert!(preview.contains("include_misc: false"));
        assert!(preview.contains("hits: 0"));
        assert!(preview.contains("no semantic bucket hits"));
    }

    #[test]
    fn query_semantic_index_ignores_proposed_until_user_approval() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc"),
            ],
        );
        let proposed = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let snapshot = ColdSemanticIndexSnapshot {
            bucket_map: Some(map),
            placements: vec![proposed],
        };

        let hits = query_semantic_index(
            &snapshot,
            &ColdSemanticQuery {
                query: "miso".to_string(),
                max_hits: 8,
                include_misc: true,
            },
        )
        .unwrap();

        assert!(hits.is_empty());
    }

    #[test]
    fn placement_review_decisions_approve_and_reject_selected_proposals() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc"),
            ],
        );
        let approve = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let reject = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 200, 204),
            ColdBucketFit::Weak,
            "Weak duplicate.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let keep_proposed = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 300, 304),
            ColdBucketFit::Medium,
            "Needs later review.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let mut reapprove = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 400, 404),
            ColdBucketFit::Strong,
            "Previously flagged but still valid.",
            ColdBucketPlacementAuthor::Sorter,
        );
        reapprove.review_status = ColdBucketReviewStatus::NeedsReview;
        let mut approve_ids = HashSet::new();
        approve_ids.insert(approve.placement_id.clone());
        approve_ids.insert(reapprove.placement_id.clone());
        let mut reject_ids = HashSet::new();
        reject_ids.insert(reject.placement_id.clone());

        let (updated, receipt) = apply_placement_review_decisions(
            &map,
            &[approve, reject, keep_proposed, reapprove],
            &approve_ids,
            &reject_ids,
            1234,
        )
        .unwrap();

        assert_eq!(receipt.reviewed_at_unix_ms, 1234);
        assert_eq!(receipt.approved_placement_ids.len(), 2);
        assert_eq!(receipt.rejected_placement_ids.len(), 1);
        assert_eq!(receipt.remaining_proposed_count, 1);
        assert_eq!(receipt.remaining_needs_review_count, 0);
        assert!(receipt.transitions.iter().any(|transition| {
            transition.from_status == ColdBucketReviewStatus::NeedsReview
                && transition.to_status == ColdBucketReviewStatus::Approved
        }));
        assert_eq!(updated[0].review_status, ColdBucketReviewStatus::Approved);
        assert_eq!(updated[1].review_status, ColdBucketReviewStatus::Rejected);
        assert_eq!(updated[2].review_status, ColdBucketReviewStatus::Proposed);
        assert_eq!(updated[3].review_status, ColdBucketReviewStatus::Approved);
    }

    #[test]
    fn placement_review_decisions_to_store_rewrites_placements_and_appends_receipt() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc"),
            ],
        );
        let approve = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let reject = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 200, 204),
            ColdBucketFit::Weak,
            "Weak duplicate.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let mut approve_ids = HashSet::new();
        approve_ids.insert(approve.placement_id.clone());
        let mut reject_ids = HashSet::new();
        reject_ids.insert(reject.placement_id.clone());

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        write_placements_jsonl(&paths.placements_jsonl, &[approve, reject]).unwrap();

        let receipt =
            apply_placement_review_decisions_to_store(&paths, &approve_ids, &reject_ids, 1234)
                .unwrap();

        assert_eq!(receipt.approved_placement_ids.len(), 1);
        assert_eq!(receipt.rejected_placement_ids.len(), 1);
        let updated = read_placements_jsonl(&paths.placements_jsonl).unwrap();
        assert_eq!(updated[0].review_status, ColdBucketReviewStatus::Approved);
        assert_eq!(updated[1].review_status, ColdBucketReviewStatus::Rejected);
        assert_eq!(
            read_placement_review_receipts_jsonl(&paths.placement_review_receipts_jsonl).unwrap(),
            vec![receipt]
        );
    }

    #[test]
    fn placement_review_decisions_reject_overlap_and_unknown_ids() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("misc"),
            ],
        );
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let mut approve_ids = HashSet::new();
        approve_ids.insert(placement.placement_id.clone());
        let mut reject_ids = HashSet::new();
        reject_ids.insert(placement.placement_id.clone());

        assert!(
            apply_placement_review_decisions(
                &map,
                std::slice::from_ref(&placement),
                &approve_ids,
                &reject_ids,
                1234
            )
            .is_err()
        );

        reject_ids.clear();
        approve_ids.clear();
        approve_ids.insert("missing".to_string());
        assert!(
            apply_placement_review_decisions(&map, &[placement], &approve_ids, &reject_ids, 1234)
                .is_err()
        );
    }

    #[test]
    fn query_semantic_index_excludes_rejected_and_can_skip_misc() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let mut rejected = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        rejected.review_status = ColdBucketReviewStatus::Rejected;
        let mut misc = ColdBucketPlacement::new(
            DEFAULT_MISC_BUCKET_ID,
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 300, 304),
            ColdBucketFit::Overflow,
            "Miso aside with weak fit.",
            ColdBucketPlacementAuthor::Host,
        );
        misc.review_status = ColdBucketReviewStatus::Approved;
        let snapshot = ColdSemanticIndexSnapshot {
            bucket_map: Some(map),
            placements: vec![rejected, misc],
        };

        let no_misc = query_semantic_index(
            &snapshot,
            &ColdSemanticQuery {
                query: "miso".to_string(),
                max_hits: 8,
                include_misc: false,
            },
        )
        .unwrap();
        let with_misc = query_semantic_index(
            &snapshot,
            &ColdSemanticQuery {
                query: "miso".to_string(),
                max_hits: 8,
                include_misc: true,
            },
        )
        .unwrap();

        assert!(no_misc.is_empty());
        assert_eq!(with_misc.len(), 1);
        assert_eq!(with_misc[0].bucket_kind, ColdSemanticBucketKind::Misc);
    }

    #[test]
    fn query_semantic_index_excludes_needs_review_until_reapproved() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let mut needs_review = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        needs_review.review_status = ColdBucketReviewStatus::NeedsReview;
        let snapshot = ColdSemanticIndexSnapshot {
            bucket_map: Some(map.clone()),
            placements: vec![needs_review.clone()],
        };

        let inactive_hits = query_semantic_index(
            &snapshot,
            &ColdSemanticQuery::remember_current_input("miso"),
        )
        .unwrap();
        assert!(inactive_hits.is_empty());

        let mut approve_ids = HashSet::new();
        approve_ids.insert(needs_review.placement_id.clone());
        let (updated, _) = apply_placement_review_decisions(
            &map,
            &[needs_review],
            &approve_ids,
            &HashSet::new(),
            9,
        )
        .unwrap();
        let active_hits = query_semantic_index(
            &ColdSemanticIndexSnapshot {
                bucket_map: Some(map),
                placements: updated,
            },
            &ColdSemanticQuery::remember_current_input("miso"),
        )
        .unwrap();

        assert_eq!(active_hits.len(), 1);
    }

    #[test]
    fn query_semantic_index_store_loads_snapshot_before_searching() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let mut placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 100, 104),
            ColdBucketFit::Strong,
            "Miso care details.",
            ColdBucketPlacementAuthor::Sorter,
        );
        placement.review_status = ColdBucketReviewStatus::Approved;

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        write_placements_jsonl(&paths.placements_jsonl, &[placement]).unwrap();

        let hits = query_semantic_index_store(
            &paths,
            &ColdSemanticQuery {
                query: "miso".to_string(),
                max_hits: 8,
                include_misc: true,
            },
        )
        .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bucket_id, "entity:miso");
    }

    #[test]
    fn placement_validation_rejects_missing_bucket_and_non_misc_overflow() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 1, 2);
        let valid = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Strong,
            "Miso is directly named.",
            ColdBucketPlacementAuthor::User,
        );
        assert!(validate_placements_against_bucket_map(&map, &[valid]).is_ok());

        let missing = ColdBucketPlacement::new(
            "missing",
            evidence.clone(),
            ColdBucketFit::Strong,
            "Bad bucket.",
            ColdBucketPlacementAuthor::User,
        );
        assert!(validate_placements_against_bucket_map(&map, &[missing]).is_err());

        let wrong_overflow = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Overflow,
            "Overflow must go to misc.",
            ColdBucketPlacementAuthor::Sorter,
        );
        assert!(validate_placements_against_bucket_map(&map, &[wrong_overflow]).is_err());

        let misc_overflow = ColdBucketPlacement::new(
            DEFAULT_MISC_BUCKET_ID,
            evidence,
            ColdBucketFit::Overflow,
            "Does not fit a positive bucket cleanly.",
            ColdBucketPlacementAuthor::Sorter,
        );
        assert!(validate_placements_against_bucket_map(&map, &[misc_overflow]).is_ok());
    }

    #[test]
    fn bucket_sift_proposal_moves_weak_positive_fit_to_misc_without_applying() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let weak = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 50, 54),
            ColdBucketFit::Weak,
            "Miso is only tangentially mentioned.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let placements = vec![weak.clone()];

        let proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999)
            .unwrap()
            .unwrap();

        assert_eq!(proposal.bucket_id, "entity:miso");
        assert_eq!(proposal.changes.len(), 1);
        assert_eq!(
            proposal.changes[0].action,
            ColdBucketSiftAction::MoveToOverflow
        );
        assert_eq!(
            proposal.changes[0].to_bucket_id,
            Some(DEFAULT_MISC_BUCKET_ID.to_string())
        );
        assert!(!proposal.changes[0].selected_for_apply);
        assert_eq!(placements, vec![weak]);
    }

    #[test]
    fn bucket_sift_proposal_marks_duplicate_evidence_for_retirement() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 70, 74);
        let first = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Strong,
            "Miso is directly discussed.",
            ColdBucketPlacementAuthor::User,
        );
        let duplicate = ColdBucketPlacement::new(
            "entity:miso",
            evidence,
            ColdBucketFit::Medium,
            "Duplicate route from sorter.",
            ColdBucketPlacementAuthor::Sorter,
        );

        let proposal = build_bucket_sift_proposal(&map, &[first, duplicate], "entity:miso", 999)
            .unwrap()
            .unwrap();

        assert_eq!(proposal.changes.len(), 1);
        assert_eq!(
            proposal.changes[0].action,
            ColdBucketSiftAction::RetirePlacement
        );
        assert_eq!(
            proposal.changes[0].from_bucket_id,
            Some("entity:miso".to_string())
        );
        assert_eq!(proposal.changes[0].to_bucket_id, None);
        assert!(!proposal.changes[0].selected_for_apply);
    }

    #[test]
    fn bucket_sift_proposal_returns_none_for_clean_bucket() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let placements = vec![ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 80, 84),
            ColdBucketFit::Strong,
            "Miso is directly discussed.",
            ColdBucketPlacementAuthor::User,
        )];

        let proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999).unwrap();

        assert!(proposal.is_none());
    }

    #[test]
    fn semantic_review_queue_includes_proposed_and_needs_review_only() {
        let mut proposed = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 1, 2),
            ColdBucketFit::Strong,
            "Proposed.",
            ColdBucketPlacementAuthor::Sorter,
        );
        proposed.review_status = ColdBucketReviewStatus::Proposed;
        let mut needs_review = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 3, 4),
            ColdBucketFit::Strong,
            "Needs review.",
            ColdBucketPlacementAuthor::Sorter,
        );
        needs_review.review_status = ColdBucketReviewStatus::NeedsReview;
        let mut approved = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 5, 6),
            ColdBucketFit::Strong,
            "Approved.",
            ColdBucketPlacementAuthor::Sorter,
        );
        approved.review_status = ColdBucketReviewStatus::Approved;

        let queue = semantic_review_queue_placements(&[proposed, needs_review, approved]);

        assert_eq!(queue.len(), 2);
        assert!(
            queue
                .iter()
                .any(|placement| placement.review_status == ColdBucketReviewStatus::Proposed)
        );
        assert!(
            queue
                .iter()
                .any(|placement| placement.review_status == ColdBucketReviewStatus::NeedsReview)
        );
    }

    #[test]
    fn bucket_sift_proposal_uses_noise_feedback_as_needs_review_hint() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 80, 84),
            ColdBucketFit::Strong,
            "Miso is directly discussed.",
            ColdBucketPlacementAuthor::User,
        );
        let feedback = feedback_receipt_for(
            &placement,
            "C:\\memory\\cold_log.jsonl",
            ColdMemoryCandidateFeedback::Noise,
        );

        let proposal = build_bucket_sift_proposal_with_feedback(
            &map,
            std::slice::from_ref(&placement),
            "entity:miso",
            999,
            &[feedback],
        )
        .unwrap()
        .unwrap();

        assert_eq!(proposal.changes.len(), 1);
        assert_eq!(proposal.changes[0].action, ColdBucketSiftAction::MarkStale);
        assert_eq!(
            proposal.changes[0].target_placement_id.as_deref(),
            Some(placement.placement_id.as_str())
        );
        assert!(proposal.changes[0].rationale.contains("noise=1"));
        assert!(!proposal.changes[0].selected_for_apply);
    }

    #[test]
    fn selected_feedback_sift_marks_placement_needs_review_not_rejected() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let placement = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 80, 84),
            ColdBucketFit::Strong,
            "Miso is directly discussed.",
            ColdBucketPlacementAuthor::User,
        );
        let feedback = feedback_receipt_for(
            &placement,
            "C:\\memory\\cold_log.jsonl",
            ColdMemoryCandidateFeedback::Misleading,
        );
        let placements = vec![placement];
        let mut proposal = build_bucket_sift_proposal_with_feedback(
            &map,
            &placements,
            "entity:miso",
            999,
            &[feedback],
        )
        .unwrap()
        .unwrap();
        proposal.changes[0].selected_for_apply = true;

        let (updated, receipt) =
            apply_selected_sift_changes(&map, &placements, &proposal, 1000).unwrap();

        assert_eq!(
            receipt.applied_change_ids,
            vec![proposal.changes[0].change_id.clone()]
        );
        assert!(receipt.rejected_change_ids.is_empty());
        assert_eq!(updated.len(), 1);
        assert_eq!(
            updated[0].review_status,
            ColdBucketReviewStatus::NeedsReview
        );
    }

    #[test]
    fn apply_selected_sift_changes_moves_weak_fit_to_misc_and_records_receipt() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let weak = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 50, 54),
            ColdBucketFit::Weak,
            "Miso is only tangentially mentioned.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let placements = vec![weak];
        let mut proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999)
            .unwrap()
            .unwrap();
        proposal.changes[0].selected_for_apply = true;

        let (updated, receipt) =
            apply_selected_sift_changes(&map, &placements, &proposal, 1000).unwrap();

        assert_eq!(
            receipt.applied_change_ids,
            vec![proposal.changes[0].change_id.clone()]
        );
        assert!(receipt.rejected_change_ids.is_empty());
        assert_eq!(updated.len(), 2);
        assert_eq!(updated[0].bucket_id, "entity:miso");
        assert_eq!(updated[0].review_status, ColdBucketReviewStatus::Rejected);
        assert_eq!(updated[1].bucket_id, DEFAULT_MISC_BUCKET_ID);
        assert_eq!(updated[1].fit, ColdBucketFit::Overflow);
        assert_eq!(updated[1].review_status, ColdBucketReviewStatus::Approved);
    }

    #[test]
    fn apply_selected_sift_changes_retains_unselected_changes_as_rejected_receipt_only() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let weak = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 50, 54),
            ColdBucketFit::Weak,
            "Miso is only tangentially mentioned.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let placements = vec![weak.clone()];
        let proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999)
            .unwrap()
            .unwrap();

        let (updated, receipt) =
            apply_selected_sift_changes(&map, &placements, &proposal, 1000).unwrap();

        assert!(receipt.applied_change_ids.is_empty());
        assert_eq!(
            receipt.rejected_change_ids,
            vec![proposal.changes[0].change_id.clone()]
        );
        assert_eq!(updated, vec![weak]);
    }

    #[test]
    fn apply_selected_sift_changes_retires_target_duplicate_not_first_match() {
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 70, 74);
        let first = ColdBucketPlacement::new(
            "entity:miso",
            evidence.clone(),
            ColdBucketFit::Strong,
            "Miso is directly discussed.",
            ColdBucketPlacementAuthor::User,
        );
        let duplicate = ColdBucketPlacement::new(
            "entity:miso",
            evidence,
            ColdBucketFit::Medium,
            "Duplicate route from sorter.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let placements = vec![first, duplicate];
        let mut proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999)
            .unwrap()
            .unwrap();
        proposal.changes[0].selected_for_apply = true;

        let (updated, receipt) =
            apply_selected_sift_changes(&map, &placements, &proposal, 1000).unwrap();

        assert_eq!(
            receipt.applied_change_ids,
            vec![proposal.changes[0].change_id.clone()]
        );
        assert_eq!(updated[0].review_status, ColdBucketReviewStatus::Proposed);
        assert_eq!(updated[1].review_status, ColdBucketReviewStatus::Rejected);
    }

    #[test]
    fn apply_selected_sift_changes_to_store_rewrites_placements_and_appends_receipt() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let map = ColdSemanticBucketMap::new(
            "map-test",
            123,
            "sort interaction logs",
            ColdBucketPlacementAuthor::User,
            vec![
                bucket("entity:miso", "Miso"),
                ColdSemanticBucket::misc("sort interaction logs"),
            ],
        );
        let weak = ColdBucketPlacement::new(
            "entity:miso",
            ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 50, 54),
            ColdBucketFit::Weak,
            "Miso is only tangentially mentioned.",
            ColdBucketPlacementAuthor::Sorter,
        );
        let placements = vec![weak];

        write_bucket_map_json(&paths.bucket_map_json, &map).unwrap();
        write_placements_jsonl(&paths.placements_jsonl, &placements).unwrap();
        let mut proposal = build_bucket_sift_proposal(&map, &placements, "entity:miso", 999)
            .unwrap()
            .unwrap();
        proposal.changes[0].selected_for_apply = true;
        append_sift_proposal_jsonl(&paths.sift_proposals_jsonl, &proposal).unwrap();

        let receipt = apply_selected_sift_changes_to_store(&paths, &proposal, 1000).unwrap();

        let updated = read_placements_jsonl(&paths.placements_jsonl).unwrap();
        assert_eq!(updated.len(), 2);
        assert_eq!(updated[0].review_status, ColdBucketReviewStatus::Rejected);
        assert_eq!(updated[1].bucket_id, DEFAULT_MISC_BUCKET_ID);
        assert_eq!(updated[1].review_status, ColdBucketReviewStatus::Approved);
        assert_eq!(
            read_sift_receipts_jsonl(&paths.sift_receipts_jsonl).unwrap(),
            vec![receipt]
        );
    }

    #[test]
    fn sift_proposal_and_receipt_jsonl_roundtrip_without_applying_changes() {
        let temp = tempfile_path::TempStore::new();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        let evidence = ColdEvidenceRef::interaction_log_range("memory/cold_log.jsonl", 20, 24);
        let proposal = ColdBucketSiftProposal {
            proposal_id: "sift-1".to_string(),
            bucket_id: "topic:pet_health".to_string(),
            proposed_at_unix_ms: 456,
            rationale: "Move weak weather aside into misc overflow.".to_string(),
            changes: vec![ColdBucketSiftChange {
                change_id: "change-1".to_string(),
                action: ColdBucketSiftAction::MoveToOverflow,
                target_placement_id: None,
                target_placement_index: None,
                evidence_ref: evidence,
                from_bucket_id: Some("topic:pet_health".to_string()),
                to_bucket_id: Some(DEFAULT_MISC_BUCKET_ID.to_string()),
                before: "topic:pet_health contains a weak weather aside.".to_string(),
                after: "misc contains the weak weather aside as overflow.".to_string(),
                rationale: "The exchange is weakly fit and should not bloat pet health."
                    .to_string(),
                selected_for_apply: false,
            }],
        };
        let receipt = ColdBucketSiftReceipt {
            receipt_id: "receipt-1".to_string(),
            proposal_id: proposal.proposal_id.clone(),
            applied_at_unix_ms: 789,
            applied_change_ids: Vec::new(),
            rejected_change_ids: vec!["change-1".to_string()],
        };

        append_sift_proposal_jsonl(&paths.sift_proposals_jsonl, &proposal).unwrap();
        append_sift_receipt_jsonl(&paths.sift_receipts_jsonl, &receipt).unwrap();

        assert_eq!(
            read_sift_proposals_jsonl(&paths.sift_proposals_jsonl).unwrap(),
            vec![proposal]
        );
        assert_eq!(
            read_sift_receipts_jsonl(&paths.sift_receipts_jsonl).unwrap(),
            vec![receipt]
        );
        assert!(
            read_placements_jsonl(&paths.placements_jsonl)
                .unwrap()
                .is_empty()
        );
    }
}
