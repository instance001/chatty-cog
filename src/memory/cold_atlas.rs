use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::semantic_index::{
    ColdBucketReviewStatus, ColdEvidenceRef, ColdSemanticIndexPaths, read_bucket_map_json,
    read_placements_jsonl,
};

pub const COLD_ATLAS_DIR: &str = "cold_atlas";
pub const ATLAS_RECORDS_JSONL: &str = "atlas_records.jsonl";
pub const ATLAS_RECEIPTS_JSONL: &str = "atlas_receipts.jsonl";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdAtlasPaths {
    pub root_dir: PathBuf,
    pub records_jsonl: PathBuf,
    pub receipts_jsonl: PathBuf,
}

impl ColdAtlasPaths {
    pub fn under_memory_dir(memory_dir: impl AsRef<Path>) -> Self {
        let root_dir = memory_dir.as_ref().join(COLD_ATLAS_DIR);
        let records_jsonl = root_dir.join(ATLAS_RECORDS_JSONL);
        let receipts_jsonl = root_dir.join(ATLAS_RECEIPTS_JSONL);
        Self {
            root_dir,
            records_jsonl,
            receipts_jsonl,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdAtlasRecord {
    pub schema_version: u32,
    pub atlas_id: String,
    pub title: String,
    pub summary: String,
    pub scope: String,
    pub status: ColdAtlasRecordStatus,
    pub confidence: ColdAtlasConfidence,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<ColdEvidenceRef>,
    pub note: String,
}

impl ColdAtlasRecord {
    pub fn new(
        atlas_id: impl Into<String>,
        title: impl Into<String>,
        summary: impl Into<String>,
        scope: impl Into<String>,
        confidence: ColdAtlasConfidence,
        recorded_at_unix_ms: i64,
        evidence_refs: Vec<ColdEvidenceRef>,
        note: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            atlas_id: atlas_id.into(),
            title: title.into(),
            summary: summary.into(),
            scope: scope.into(),
            status: ColdAtlasRecordStatus::HistoricalShape,
            confidence,
            created_at_unix_ms: recorded_at_unix_ms,
            updated_at_unix_ms: recorded_at_unix_ms,
            tags: Vec::new(),
            evidence_refs,
            note: note.into(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            anyhow::bail!("cold atlas record schema_version must be 1");
        }
        if self.atlas_id.trim().is_empty() {
            anyhow::bail!("cold atlas record requires atlas_id");
        }
        if self.title.trim().is_empty() {
            anyhow::bail!("cold atlas record '{}' requires title", self.atlas_id);
        }
        if self.summary.trim().is_empty() {
            anyhow::bail!("cold atlas record '{}' requires summary", self.atlas_id);
        }
        if self.scope.trim().is_empty() {
            anyhow::bail!("cold atlas record '{}' requires scope", self.atlas_id);
        }
        if self.evidence_refs.is_empty() {
            anyhow::bail!(
                "cold atlas record '{}' requires at least one evidence ref",
                self.atlas_id
            );
        }
        self.confidence.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdAtlasRecordStatus {
    HistoricalShape,
    NeedsReview,
    Stale,
    Superseded,
    Retired,
}

impl ColdAtlasRecordStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::HistoricalShape => "historical_shape",
            Self::NeedsReview => "needs_review",
            Self::Stale => "stale",
            Self::Superseded => "superseded",
            Self::Retired => "retired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdAtlasConfidence {
    pub score: f32,
    pub label: String,
}

impl ColdAtlasConfidence {
    pub fn new(score: f32, label: impl Into<String>) -> Self {
        Self {
            score,
            label: label.into(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.score) {
            anyhow::bail!("cold atlas confidence score must be between 0.0 and 1.0");
        }
        if self.label.trim().is_empty() {
            anyhow::bail!("cold atlas confidence requires a label");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColdAtlasReceipt {
    pub receipt_id: String,
    pub recorded_at_unix_ms: i64,
    pub outcome: ColdAtlasReceiptOutcome,
    pub atlas_id: Option<String>,
    #[serde(default)]
    pub related_atlas_id: Option<String>,
    pub evidence_ref_count: usize,
    pub note: String,
    #[serde(default)]
    pub from_status: Option<ColdAtlasRecordStatus>,
    #[serde(default)]
    pub to_status: Option<ColdAtlasRecordStatus>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColdAtlasEvidence {
    pub evidence_id: String,
    pub bucket_id: String,
    pub bucket_label: String,
    pub placement_id: String,
    pub placement_reason: String,
    pub evidence_ref: ColdEvidenceRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedColdAtlasRun {
    pub source: String,
    pub evidence: Vec<ColdAtlasEvidence>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommittedColdAtlasRun {
    pub receipt_count: usize,
    pub record_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColdAtlasQuery {
    pub query: String,
    pub max_hits: usize,
    pub include_stale: bool,
    pub include_superseded: bool,
}

impl ColdAtlasQuery {
    pub fn remember_current_input(input: impl Into<String>) -> Self {
        Self {
            query: input.into(),
            max_hits: 12,
            include_stale: true,
            include_superseded: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColdAtlasHit {
    pub score: f32,
    pub atlas_id: String,
    pub title: String,
    pub summary: String,
    pub scope: String,
    pub status: ColdAtlasRecordStatus,
    pub confidence: ColdAtlasConfidence,
    pub evidence_index: usize,
    pub evidence_ref: ColdEvidenceRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColdAtlasLineageLink {
    pub recorded_at_unix_ms: i64,
    pub old_atlas_id: String,
    pub replacement_atlas_id: String,
}

impl ColdAtlasReceipt {
    pub fn for_record(
        outcome: ColdAtlasReceiptOutcome,
        record: &ColdAtlasRecord,
        recorded_at_unix_ms: i64,
        note: impl Into<String>,
    ) -> Self {
        Self {
            receipt_id: format!("cold-atlas-{}-{}", outcome.as_str(), recorded_at_unix_ms),
            recorded_at_unix_ms,
            outcome,
            atlas_id: Some(record.atlas_id.clone()),
            related_atlas_id: None,
            evidence_ref_count: record.evidence_refs.len(),
            note: note.into(),
            from_status: None,
            to_status: None,
        }
    }

    pub fn for_status_change(
        record: &ColdAtlasRecord,
        from_status: ColdAtlasRecordStatus,
        to_status: ColdAtlasRecordStatus,
        recorded_at_unix_ms: i64,
        note: impl Into<String>,
    ) -> Self {
        Self {
            receipt_id: format!(
                "cold-atlas-status-{}-{}",
                to_status.as_str(),
                recorded_at_unix_ms
            ),
            recorded_at_unix_ms,
            outcome: ColdAtlasReceiptOutcome::StatusChanged,
            atlas_id: Some(record.atlas_id.clone()),
            related_atlas_id: None,
            evidence_ref_count: record.evidence_refs.len(),
            note: note.into(),
            from_status: Some(from_status),
            to_status: Some(to_status),
        }
    }

    pub fn rejected(
        recorded_at_unix_ms: i64,
        evidence_ref_count: usize,
        note: impl Into<String>,
    ) -> Self {
        Self {
            receipt_id: format!("cold-atlas-rejected-{recorded_at_unix_ms}"),
            recorded_at_unix_ms,
            outcome: ColdAtlasReceiptOutcome::Rejected,
            atlas_id: None,
            related_atlas_id: None,
            evidence_ref_count,
            note: note.into(),
            from_status: None,
            to_status: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdAtlasReceiptOutcome {
    Created,
    Revised,
    MarkedStale,
    StatusChanged,
    Rejected,
}

impl ColdAtlasReceiptOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Revised => "revised",
            Self::MarkedStale => "marked_stale",
            Self::StatusChanged => "status_changed",
            Self::Rejected => "rejected",
        }
    }
}

pub fn prepare_cold_atlas_run(
    paths: &ColdSemanticIndexPaths,
    max_items: usize,
) -> Result<Option<PreparedColdAtlasRun>> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot prepare cold atlas run without bucket_map.json");
    };
    let placements = read_placements_jsonl(&paths.placements_jsonl)?;
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();
    for placement in placements
        .iter()
        .filter(|placement| placement.review_status == ColdBucketReviewStatus::Approved)
    {
        if !seen.insert(placement.evidence_ref.stable_key()) {
            continue;
        }
        let bucket_label = bucket_map
            .bucket(&placement.bucket_id)
            .map(|bucket| bucket.label.clone())
            .unwrap_or_else(|| placement.bucket_id.clone());
        evidence.push(ColdAtlasEvidence {
            evidence_id: format!("atlas-evidence:{}", placement.evidence_ref.stable_key()),
            bucket_id: placement.bucket_id.clone(),
            bucket_label,
            placement_id: placement.placement_id.clone(),
            placement_reason: placement.reason.clone(),
            evidence_ref: placement.evidence_ref.clone(),
        });
    }

    evidence.sort_by(|left, right| {
        left.evidence_ref
            .start_event_index
            .cmp(&right.evidence_ref.start_event_index)
            .then_with(|| left.bucket_id.cmp(&right.bucket_id))
            .then_with(|| left.placement_id.cmp(&right.placement_id))
    });

    let take_count = max_items.max(1);
    let skip_count = evidence.len().saturating_sub(take_count);
    let evidence = evidence.into_iter().skip(skip_count).collect::<Vec<_>>();
    if evidence.is_empty() {
        return Ok(None);
    }

    let prompt = render_cold_atlas_prompt(&evidence)?;
    Ok(Some(PreparedColdAtlasRun {
        source: "approved semantic placements".to_string(),
        evidence,
        prompt,
    }))
}

pub fn prepare_cold_atlas_revision_run(record: &ColdAtlasRecord) -> Result<PreparedColdAtlasRun> {
    record.validate()?;
    let evidence = record
        .evidence_refs
        .iter()
        .enumerate()
        .map(|(index, evidence_ref)| ColdAtlasEvidence {
            evidence_id: format!("atlas-evidence:{}", evidence_ref.stable_key()),
            bucket_id: "cold_atlas_revision".to_string(),
            bucket_label: "Existing atlas evidence".to_string(),
            placement_id: format!("atlas-revision:{}:{index}", record.atlas_id),
            placement_reason: format!(
                "Existing evidence ref {} for atlas revision {}.",
                index + 1,
                record.atlas_id
            ),
            evidence_ref: evidence_ref.clone(),
        })
        .collect::<Vec<_>>();

    let mut prompt = render_cold_atlas_prompt(&evidence)?;
    prompt.push_str("\n\nRevision target:\n");
    prompt.push_str("Revise exactly one atlas record. Keep the same atlas_id. Use only the listed evidence_id values. This is a user-reviewed draft, not current truth.\n");
    prompt.push_str(
        &serde_json::to_string_pretty(&serde_json::json!({
            "atlas_id": record.atlas_id,
            "title": record.title,
            "summary": record.summary,
            "scope": record.scope,
            "confidence": record.confidence,
            "tags": record.tags,
            "status": record.status,
        }))
        .context("serialize cold atlas revision target")?,
    );

    Ok(PreparedColdAtlasRun {
        source: format!("revision of {}", record.atlas_id),
        evidence,
        prompt,
    })
}

pub fn prepare_cold_atlas_supersede_run(record: &ColdAtlasRecord) -> Result<PreparedColdAtlasRun> {
    let mut prepared = prepare_cold_atlas_revision_run(record)?;
    prepared.source = format!("supersede {}", record.atlas_id);
    prepared.prompt.push_str("\n\nSupersede target:\n");
    prepared.prompt.push_str("Create exactly one replacement atlas record. Use a new atlas_id. Use only the listed evidence_id values. The old atlas record will be marked superseded only after user preview and commit.\n");
    Ok(prepared)
}

pub fn render_cold_atlas_prompt(evidence: &[ColdAtlasEvidence]) -> Result<String> {
    let evidence_payload = evidence
        .iter()
        .map(|item| {
            serde_json::json!({
                "evidence_id": item.evidence_id,
                "bucket_id": item.bucket_id,
                "bucket_label": item.bucket_label,
                "placement_id": item.placement_id,
                "placement_reason": item.placement_reason,
                "evidence_ref": item.evidence_ref,
            })
        })
        .collect::<Vec<_>>();

    let mut prompt = String::new();
    prompt
        .push_str("You are drafting cold-atlas records from approved semantic memory evidence.\n");
    prompt.push_str("Cold atlas is broad historical shape memory. It orients recall; it is not current truth and does not replace exact cold-log evidence.\n");
    prompt.push_str("Return JSON only. Do not invent evidence refs. Every atlas record must cite one or more provided evidence_id values.\n\n");
    prompt.push_str("Output schema:\n");
    prompt.push_str(
        "{\"records\":[{\"atlas_id\":\"stable-human-readable-id\",\"title\":\"short label\",\"summary\":\"broad remembered shape\",\"scope\":\"bounded scope\",\"confidence\":{\"score\":0.0,\"label\":\"short confidence label\"},\"tags\":[\"optional\",\"tags\"],\"evidence_ids\":[\"...\"]}]}\n\n",
    );
    prompt.push_str("Evidence:\n");
    prompt.push_str(
        &serde_json::to_string_pretty(&evidence_payload)
            .context("serialize cold atlas evidence")?,
    );
    Ok(prompt)
}

pub fn render_cold_atlas_record_draft_json(record: &ColdAtlasRecord) -> Result<String> {
    record.validate()?;
    let evidence_ids = record
        .evidence_refs
        .iter()
        .map(|evidence_ref| format!("atlas-evidence:{}", evidence_ref.stable_key()))
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "records": [
            {
                "atlas_id": record.atlas_id,
                "title": record.title,
                "summary": record.summary,
                "scope": record.scope,
                "confidence": record.confidence,
                "tags": record.tags,
                "evidence_ids": evidence_ids,
            }
        ]
    }))
    .context("serialize cold atlas record draft JSON")
}

pub fn render_cold_atlas_replacement_draft_json(record: &ColdAtlasRecord) -> Result<String> {
    record.validate()?;
    let mut draft =
        serde_json::from_str::<serde_json::Value>(&render_cold_atlas_record_draft_json(record)?)
            .context("parse cold atlas record draft JSON")?;
    draft["records"][0]["atlas_id"] = serde_json::json!(format!("{}:replacement", record.atlas_id));
    draft["records"][0]["title"] = serde_json::json!(format!("Replacement for {}", record.title));
    draft["records"][0]["summary"] =
        serde_json::json!("Replace this with the updated broad historical shape.");
    serde_json::to_string_pretty(&draft).context("serialize cold atlas replacement draft JSON")
}

pub fn parse_cold_atlas_output(
    evidence: &[ColdAtlasEvidence],
    output_json: &str,
    recorded_at_unix_ms: i64,
) -> Result<Vec<ColdAtlasRecord>> {
    let response =
        serde_json::from_str::<AtlasDraftResponse>(output_json).context("parse cold atlas JSON")?;
    let evidence_by_id = evidence
        .iter()
        .map(|item| (item.evidence_id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut seen_atlas_ids = HashSet::new();
    let mut records = Vec::new();
    for record in response.records {
        let atlas_id = record.atlas_id.trim();
        if atlas_id.is_empty() {
            anyhow::bail!("cold atlas draft has an empty atlas_id");
        }
        if !seen_atlas_ids.insert(atlas_id.to_string()) {
            anyhow::bail!("cold atlas draft repeated atlas_id '{atlas_id}'");
        }
        if record.evidence_ids.is_empty() {
            anyhow::bail!("cold atlas draft '{atlas_id}' must cite evidence_ids");
        }
        let mut evidence_refs = Vec::new();
        let mut seen_evidence_ids = HashSet::new();
        for evidence_id in record.evidence_ids {
            if !seen_evidence_ids.insert(evidence_id.clone()) {
                anyhow::bail!(
                    "cold atlas draft '{}' repeated evidence_id '{}'",
                    atlas_id,
                    evidence_id
                );
            }
            let Some(item) = evidence_by_id.get(evidence_id.as_str()) else {
                anyhow::bail!(
                    "cold atlas draft '{}' referenced unknown evidence_id '{}'",
                    atlas_id,
                    evidence_id
                );
            };
            evidence_refs.push(item.evidence_ref.clone());
        }
        let mut atlas_record = ColdAtlasRecord::new(
            atlas_id.to_string(),
            record.title.trim().to_string(),
            record.summary.trim().to_string(),
            record.scope.trim().to_string(),
            record.confidence,
            recorded_at_unix_ms,
            evidence_refs,
            "Drafted from approved semantic placements; commit is user-triggered.",
        );
        atlas_record.tags = record
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect();
        atlas_record.validate()?;
        records.push(atlas_record);
    }
    Ok(records)
}

pub fn render_cold_atlas_review_preview(
    evidence: &[ColdAtlasEvidence],
    output_json: &str,
    recorded_at_unix_ms: i64,
) -> Result<String> {
    let records = parse_cold_atlas_output(evidence, output_json, recorded_at_unix_ms)?;
    let mut out = String::new();
    out.push_str("Cold atlas review preview\n");
    out.push_str("No atlas records will be written until Commit is clicked. Atlas records orient recall; they do not inject or promote context.\n\n");

    if records.is_empty() {
        out.push_str("  - no atlas records proposed\n");
        return Ok(out);
    }

    for (index, record) in records.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} [{}] confidence={:.2} ({}) evidence_refs={}\n",
            index + 1,
            record.title,
            record.atlas_id,
            record.confidence.score,
            record.confidence.label,
            record.evidence_refs.len()
        ));
        out.push_str(&format!("   scope: {}\n", record.scope));
        out.push_str(&format!("   summary: {}\n", record.summary));
        if !record.tags.is_empty() {
            out.push_str(&format!("   tags: {}\n", record.tags.join(", ")));
        }
    }
    Ok(out)
}

pub fn cold_atlas_json_matches_reviewed(current_json: &str, reviewed_json: &str) -> bool {
    let current_json = current_json.trim();
    !current_json.is_empty() && current_json == reviewed_json
}

pub fn commit_prepared_cold_atlas_run(
    paths: &ColdAtlasPaths,
    prepared: &PreparedColdAtlasRun,
    atlas_output_json: &str,
    committed_at_unix_ms: i64,
) -> Result<CommittedColdAtlasRun> {
    let records =
        parse_cold_atlas_output(&prepared.evidence, atlas_output_json, committed_at_unix_ms)?;
    let mut receipt_count = 0usize;
    for record in &records {
        append_cold_atlas_record_to_store(
            paths,
            record,
            committed_at_unix_ms,
            "Committed from Logs tab cold atlas panel.",
        )?;
        receipt_count += 1;
    }
    Ok(CommittedColdAtlasRun {
        receipt_count,
        record_count: records.len(),
    })
}

pub fn render_cold_atlas_revision_preview(
    existing_record: &ColdAtlasRecord,
    prepared: &PreparedColdAtlasRun,
    revision_json: &str,
    revised_at_unix_ms: i64,
) -> Result<String> {
    let revised_records =
        parse_cold_atlas_output(&prepared.evidence, revision_json, revised_at_unix_ms)?;
    let revised_record = single_revised_cold_atlas_record(
        existing_record.atlas_id.as_str(),
        revised_records,
        revised_at_unix_ms,
    )?;

    let mut out = String::new();
    out.push_str("Cold atlas revision preview\n");
    out.push_str("No atlas record will be revised until Commit is clicked. Revision updates broad shape memory only; it does not inject or promote context.\n\n");
    out.push_str("Before\n");
    out.push_str(&format!(
        "- {} [{}] status={} confidence={:.2} ({}) evidence_refs={}\n",
        existing_record.title,
        existing_record.atlas_id,
        existing_record.status.as_str(),
        existing_record.confidence.score,
        existing_record.confidence.label,
        existing_record.evidence_refs.len()
    ));
    out.push_str(&format!("  scope: {}\n", existing_record.scope));
    out.push_str(&format!("  summary: {}\n\n", existing_record.summary));
    out.push_str("After\n");
    out.push_str(&format!(
        "- {} [{}] status={} confidence={:.2} ({}) evidence_refs={}\n",
        revised_record.title,
        revised_record.atlas_id,
        revised_record.status.as_str(),
        revised_record.confidence.score,
        revised_record.confidence.label,
        revised_record.evidence_refs.len()
    ));
    out.push_str(&format!("  scope: {}\n", revised_record.scope));
    out.push_str(&format!("  summary: {}\n", revised_record.summary));
    if !revised_record.tags.is_empty() {
        out.push_str(&format!("  tags: {}\n", revised_record.tags.join(", ")));
    }
    Ok(out)
}

pub fn commit_prepared_cold_atlas_revision_to_store(
    paths: &ColdAtlasPaths,
    prepared: &PreparedColdAtlasRun,
    target_atlas_id: &str,
    revision_json: &str,
    revised_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<ColdAtlasReceipt> {
    let records = read_cold_atlas_records_jsonl(&paths.records_jsonl)?;
    let existing_index = records
        .iter()
        .position(|record| record.atlas_id == target_atlas_id)
        .ok_or_else(|| anyhow::anyhow!("unknown cold atlas record id {target_atlas_id}"))?;
    let revised_records =
        parse_cold_atlas_output(&prepared.evidence, revision_json, revised_at_unix_ms)?;
    let mut revised_record =
        single_revised_cold_atlas_record(target_atlas_id, revised_records, revised_at_unix_ms)?;
    let existing_record = &records[existing_index];
    revised_record.created_at_unix_ms = existing_record.created_at_unix_ms;
    revised_record.updated_at_unix_ms = revised_at_unix_ms;
    revised_record.status = ColdAtlasRecordStatus::HistoricalShape;
    revised_record.note =
        "Revised from existing atlas evidence; commit is user-triggered.".to_string();
    revised_record.validate()?;

    let mut updated = records;
    updated[existing_index] = revised_record.clone();
    write_cold_atlas_records_jsonl(&paths.records_jsonl, &updated)?;
    let receipt = ColdAtlasReceipt::for_record(
        ColdAtlasReceiptOutcome::Revised,
        &revised_record,
        revised_at_unix_ms,
        note,
    );
    append_cold_atlas_receipt_jsonl(&paths.receipts_jsonl, &receipt)?;
    Ok(receipt)
}

pub fn render_cold_atlas_supersede_preview(
    existing_records: &[ColdAtlasRecord],
    target_record: &ColdAtlasRecord,
    prepared: &PreparedColdAtlasRun,
    replacement_json: &str,
    superseded_at_unix_ms: i64,
) -> Result<String> {
    let replacement_records =
        parse_cold_atlas_output(&prepared.evidence, replacement_json, superseded_at_unix_ms)?;
    let replacement_record = single_replacement_cold_atlas_record(
        existing_records,
        target_record.atlas_id.as_str(),
        replacement_records,
        superseded_at_unix_ms,
    )?;

    let mut out = String::new();
    out.push_str("Cold atlas supersede preview\n");
    out.push_str("No atlas record will be written or superseded until Commit is clicked. Supersede creates a replacement broad shape and marks the old shape as historical superseded evidence.\n\n");
    out.push_str("Old record to mark superseded\n");
    out.push_str(&format!(
        "- {} [{}] status={} confidence={:.2} ({}) evidence_refs={}\n",
        target_record.title,
        target_record.atlas_id,
        target_record.status.as_str(),
        target_record.confidence.score,
        target_record.confidence.label,
        target_record.evidence_refs.len()
    ));
    out.push_str(&format!("  scope: {}\n", target_record.scope));
    out.push_str(&format!("  summary: {}\n\n", target_record.summary));
    out.push_str("Replacement record to create\n");
    out.push_str(&format!(
        "- {} [{}] status={} confidence={:.2} ({}) evidence_refs={}\n",
        replacement_record.title,
        replacement_record.atlas_id,
        replacement_record.status.as_str(),
        replacement_record.confidence.score,
        replacement_record.confidence.label,
        replacement_record.evidence_refs.len()
    ));
    out.push_str(&format!("  scope: {}\n", replacement_record.scope));
    out.push_str(&format!("  summary: {}\n", replacement_record.summary));
    if !replacement_record.tags.is_empty() {
        out.push_str(&format!("  tags: {}\n", replacement_record.tags.join(", ")));
    }
    Ok(out)
}

pub fn commit_prepared_cold_atlas_supersede_to_store(
    paths: &ColdAtlasPaths,
    prepared: &PreparedColdAtlasRun,
    target_atlas_id: &str,
    replacement_json: &str,
    superseded_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<Vec<ColdAtlasReceipt>> {
    let records = read_cold_atlas_records_jsonl(&paths.records_jsonl)?;
    let target_index = records
        .iter()
        .position(|record| record.atlas_id == target_atlas_id)
        .ok_or_else(|| anyhow::anyhow!("unknown cold atlas record id {target_atlas_id}"))?;
    let replacement_records =
        parse_cold_atlas_output(&prepared.evidence, replacement_json, superseded_at_unix_ms)?;
    let mut replacement_record = single_replacement_cold_atlas_record(
        &records,
        target_atlas_id,
        replacement_records,
        superseded_at_unix_ms,
    )?;
    replacement_record.note = format!(
        "Supersedes cold atlas record {}; commit is user-triggered.",
        target_atlas_id
    );
    replacement_record.validate()?;

    let mut updated = records;
    let from_status = updated[target_index].status;
    updated[target_index].status = ColdAtlasRecordStatus::Superseded;
    updated[target_index].updated_at_unix_ms = superseded_at_unix_ms;
    updated.push(replacement_record.clone());
    write_cold_atlas_records_jsonl(&paths.records_jsonl, &updated)?;

    let note = note.into();
    let mut old_receipt = ColdAtlasReceipt::for_status_change(
        &updated[target_index],
        from_status,
        ColdAtlasRecordStatus::Superseded,
        superseded_at_unix_ms,
        note.clone(),
    );
    old_receipt.related_atlas_id = Some(replacement_record.atlas_id.clone());
    let mut new_receipt = ColdAtlasReceipt::for_record(
        ColdAtlasReceiptOutcome::Created,
        &replacement_record,
        superseded_at_unix_ms,
        note,
    );
    new_receipt.related_atlas_id = Some(target_atlas_id.to_string());
    append_cold_atlas_receipt_jsonl(&paths.receipts_jsonl, &old_receipt)?;
    append_cold_atlas_receipt_jsonl(&paths.receipts_jsonl, &new_receipt)?;
    Ok(vec![old_receipt, new_receipt])
}

fn single_revised_cold_atlas_record(
    target_atlas_id: &str,
    revised_records: Vec<ColdAtlasRecord>,
    revised_at_unix_ms: i64,
) -> Result<ColdAtlasRecord> {
    if revised_records.len() != 1 {
        anyhow::bail!("cold atlas revision must contain exactly one record");
    }
    let mut revised_record = revised_records.into_iter().next().unwrap();
    if revised_record.atlas_id != target_atlas_id {
        anyhow::bail!(
            "cold atlas revision must keep atlas_id '{}', got '{}'",
            target_atlas_id,
            revised_record.atlas_id
        );
    }
    revised_record.updated_at_unix_ms = revised_at_unix_ms;
    revised_record.status = ColdAtlasRecordStatus::HistoricalShape;
    revised_record.validate()?;
    Ok(revised_record)
}

fn single_replacement_cold_atlas_record(
    existing_records: &[ColdAtlasRecord],
    target_atlas_id: &str,
    replacement_records: Vec<ColdAtlasRecord>,
    created_at_unix_ms: i64,
) -> Result<ColdAtlasRecord> {
    if replacement_records.len() != 1 {
        anyhow::bail!("cold atlas supersede replacement must contain exactly one record");
    }
    let mut replacement_record = replacement_records.into_iter().next().unwrap();
    if replacement_record.atlas_id == target_atlas_id {
        anyhow::bail!("cold atlas supersede replacement must use a new atlas_id");
    }
    if existing_records
        .iter()
        .any(|record| record.atlas_id == replacement_record.atlas_id)
    {
        anyhow::bail!(
            "cold atlas supersede replacement atlas_id '{}' already exists",
            replacement_record.atlas_id
        );
    }
    replacement_record.created_at_unix_ms = created_at_unix_ms;
    replacement_record.updated_at_unix_ms = created_at_unix_ms;
    replacement_record.status = ColdAtlasRecordStatus::HistoricalShape;
    replacement_record.validate()?;
    Ok(replacement_record)
}

pub fn query_cold_atlas(
    records: &[ColdAtlasRecord],
    query: &ColdAtlasQuery,
) -> Result<Vec<ColdAtlasHit>> {
    let terms = query_terms(&query.query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    for record in records {
        record.validate()?;
        if record.status == ColdAtlasRecordStatus::Retired {
            continue;
        }
        if !query.include_superseded && record.status == ColdAtlasRecordStatus::Superseded {
            continue;
        }
        if !query.include_stale && record.status == ColdAtlasRecordStatus::Stale {
            continue;
        }
        let tag_text = record.tags.join(" ");
        let score = score_terms(
            &terms,
            &[
                record.atlas_id.as_str(),
                record.title.as_str(),
                record.summary.as_str(),
                record.scope.as_str(),
                record.note.as_str(),
                tag_text.as_str(),
            ],
        ) + record.confidence.score.clamp(0.0, 1.0) * 0.15;
        if score <= 0.0 {
            continue;
        }
        for (evidence_index, evidence_ref) in record.evidence_refs.iter().enumerate() {
            let seen_key = format!("{}:{}", record.atlas_id, evidence_ref.stable_key());
            if !seen.insert(seen_key) {
                continue;
            }
            hits.push(ColdAtlasHit {
                score,
                atlas_id: record.atlas_id.clone(),
                title: record.title.clone(),
                summary: record.summary.clone(),
                scope: record.scope.clone(),
                status: record.status,
                confidence: record.confidence.clone(),
                evidence_index,
                evidence_ref: evidence_ref.clone(),
            });
        }
    }

    hits.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.atlas_id.cmp(&right.atlas_id))
            .then_with(|| left.evidence_index.cmp(&right.evidence_index))
    });
    hits.truncate(query.max_hits.max(1));
    Ok(hits)
}

pub fn query_cold_atlas_store(
    paths: &ColdAtlasPaths,
    query: &ColdAtlasQuery,
) -> Result<Vec<ColdAtlasHit>> {
    let records = read_cold_atlas_records_jsonl(&paths.records_jsonl)?;
    query_cold_atlas(&records, query)
}

pub fn render_cold_atlas_query_preview(query: &ColdAtlasQuery, hits: &[ColdAtlasHit]) -> String {
    let mut out = String::new();
    out.push_str("Cold atlas query preview\n");
    out.push_str("Preview uses the atlas recall policy only. It does not hydrate cold-log context, inject into Main AI, or promote memory.\n\n");
    out.push_str(&format!("query: {}\n", query.query.trim()));
    out.push_str(&format!(
        "include_stale: {}\ninclude_superseded: {}\nhits: {}\n\n",
        query.include_stale,
        query.include_superseded,
        hits.len()
    ));

    if hits.is_empty() {
        out.push_str("  - no atlas hits\n");
        return out;
    }

    for (index, hit) in hits.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} [{}] score={:.3} status={} confidence={:.2} ({}) evidence_ref={}\n",
            index + 1,
            hit.title,
            hit.atlas_id,
            hit.score,
            hit.status.as_str(),
            hit.confidence.score,
            hit.confidence.label,
            hit.evidence_index + 1
        ));
        out.push_str(&format!("   scope: {}\n", hit.scope));
        out.push_str(&format!("   summary: {}\n", hit.summary));
        out.push_str(&format!(
            "   source: {} events {}..{}\n",
            hit.evidence_ref.source_log_path,
            hit.evidence_ref.start_event_index,
            hit.evidence_ref.end_event_index
        ));
    }

    out
}

pub fn append_cold_atlas_record_to_store(
    paths: &ColdAtlasPaths,
    record: &ColdAtlasRecord,
    recorded_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<ColdAtlasReceipt> {
    record.validate()?;
    append_cold_atlas_record_jsonl(&paths.records_jsonl, record)?;
    let receipt = ColdAtlasReceipt::for_record(
        ColdAtlasReceiptOutcome::Created,
        record,
        recorded_at_unix_ms,
        note,
    );
    append_cold_atlas_receipt_jsonl(&paths.receipts_jsonl, &receipt)?;
    Ok(receipt)
}

pub fn apply_cold_atlas_status_change(
    records: &[ColdAtlasRecord],
    atlas_id: &str,
    to_status: ColdAtlasRecordStatus,
    changed_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<(Vec<ColdAtlasRecord>, ColdAtlasReceipt)> {
    let atlas_id = atlas_id.trim();
    if atlas_id.is_empty() {
        anyhow::bail!("cold atlas status change requires an atlas id");
    }

    let mut updated = records.to_vec();
    let record = updated
        .iter_mut()
        .find(|record| record.atlas_id == atlas_id)
        .ok_or_else(|| anyhow::anyhow!("unknown cold atlas record id {atlas_id}"))?;
    let from_status = record.status;
    if from_status == to_status {
        anyhow::bail!(
            "cold atlas record {atlas_id} already has status {}",
            to_status.as_str()
        );
    }

    record.status = to_status;
    record.updated_at_unix_ms = changed_at_unix_ms;
    record.validate()?;
    let receipt = ColdAtlasReceipt::for_status_change(
        record,
        from_status,
        to_status,
        changed_at_unix_ms,
        note,
    );
    Ok((updated, receipt))
}

pub fn apply_cold_atlas_status_change_to_store(
    paths: &ColdAtlasPaths,
    atlas_id: &str,
    to_status: ColdAtlasRecordStatus,
    changed_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<ColdAtlasReceipt> {
    let records = read_cold_atlas_records_jsonl(&paths.records_jsonl)?;
    let (updated, receipt) =
        apply_cold_atlas_status_change(&records, atlas_id, to_status, changed_at_unix_ms, note)?;
    write_cold_atlas_records_jsonl(&paths.records_jsonl, &updated)?;
    append_cold_atlas_receipt_jsonl(&paths.receipts_jsonl, &receipt)?;
    Ok(receipt)
}

pub fn append_cold_atlas_record_jsonl(
    path: impl AsRef<Path>,
    record: &ColdAtlasRecord,
) -> Result<()> {
    record.validate()?;
    append_jsonl_record(path, record, "cold atlas record")
}

pub fn write_cold_atlas_records_jsonl(
    path: impl AsRef<Path>,
    records: &[ColdAtlasRecord],
) -> Result<()> {
    for record in records {
        record.validate()?;
    }
    write_jsonl_records(path, records, "cold atlas records")
}

pub fn read_cold_atlas_records_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdAtlasRecord>> {
    let records = read_jsonl_records::<ColdAtlasRecord>(path, "cold atlas record")?;
    for record in &records {
        record.validate()?;
    }
    Ok(records)
}

pub fn append_cold_atlas_receipt_jsonl(
    path: impl AsRef<Path>,
    receipt: &ColdAtlasReceipt,
) -> Result<()> {
    append_jsonl_record(path, receipt, "cold atlas receipt")
}

pub fn read_cold_atlas_receipts_jsonl(path: impl AsRef<Path>) -> Result<Vec<ColdAtlasReceipt>> {
    read_jsonl_records(path, "cold atlas receipt")
}

pub fn render_cold_atlas_overview(records: &[ColdAtlasRecord]) -> String {
    let mut out = String::new();
    out.push_str("Cold atlas\n");
    out.push_str("Atlas records are broad historical shape memory. They orient recall but do not replace cold-log evidence or current truth.\n\n");
    out.push_str(&format!("atlas_records: {}\n\n", records.len()));

    if records.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for record in records.iter().rev() {
        out.push_str(&format!(
            "- {} [{}] status={} confidence={:.2} ({}) evidence_refs={} updated_at={}\n",
            record.title,
            record.atlas_id,
            record.status.as_str(),
            record.confidence.score,
            record.confidence.label,
            record.evidence_refs.len(),
            record.updated_at_unix_ms
        ));
        out.push_str(&format!("  scope: {}\n", record.scope));
        out.push_str(&format!("  summary: {}\n", record.summary));
        if !record.tags.is_empty() {
            out.push_str(&format!("  tags: {}\n", record.tags.join(", ")));
        }
        if !record.note.trim().is_empty() {
            out.push_str(&format!("  note: {}\n", record.note));
        }
    }

    out
}

pub fn render_cold_atlas_receipts_overview(receipts: &[ColdAtlasReceipt]) -> String {
    let mut out = String::new();
    out.push_str("Cold atlas governance receipts\n");
    out.push_str("Receipts record atlas mutations or rejections only; they do not inject or promote context.\n\n");
    out.push_str(&format!("cold_atlas_receipts: {}\n\n", receipts.len()));

    if receipts.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for receipt in receipts.iter().rev() {
        let status_transition = match (receipt.from_status, receipt.to_status) {
            (Some(from_status), Some(to_status)) => {
                format!(" status={}->{}", from_status.as_str(), to_status.as_str())
            }
            _ => String::new(),
        };
        let related = receipt
            .related_atlas_id
            .as_ref()
            .map(|atlas_id| format!(" related_atlas_id={atlas_id}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  - {} at {} outcome={} atlas_id={}{} evidence_refs={}{} note={}\n",
            receipt.receipt_id,
            receipt.recorded_at_unix_ms,
            receipt.outcome.as_str(),
            receipt.atlas_id.as_deref().unwrap_or("-"),
            related,
            receipt.evidence_ref_count,
            status_transition,
            receipt.note
        ));
    }

    out
}

pub fn render_cold_atlas_lineage_overview(
    records: &[ColdAtlasRecord],
    receipts: &[ColdAtlasReceipt],
) -> String {
    let links = cold_atlas_lineage_links(receipts);
    let records_by_id = records
        .iter()
        .map(|record| (record.atlas_id.as_str(), record))
        .collect::<HashMap<_, _>>();

    let mut out = String::new();
    out.push_str("Cold atlas lineage\n");
    out.push_str("Lineage is governance history for replaced broad-shape memory. Superseded records stay inspectable here without becoming normal Remember candidates.\n\n");
    out.push_str(&format!("lineage_links: {}\n\n", links.len()));

    if links.is_empty() {
        out.push_str("  - empty\n");
        return out;
    }

    for link in links {
        let old = records_by_id.get(link.old_atlas_id.as_str()).copied();
        let replacement = records_by_id
            .get(link.replacement_atlas_id.as_str())
            .copied();
        out.push_str(&format!(
            "- {} -> {} at {}\n",
            describe_atlas_lineage_record(link.old_atlas_id.as_str(), old),
            describe_atlas_lineage_record(link.replacement_atlas_id.as_str(), replacement),
            link.recorded_at_unix_ms
        ));
        if let Some(old) = old {
            out.push_str(&format!("  old_summary: {}\n", old.summary));
        }
        if let Some(replacement) = replacement {
            out.push_str(&format!("  replacement_summary: {}\n", replacement.summary));
        }
    }

    out
}

pub fn cold_atlas_lineage_links(receipts: &[ColdAtlasReceipt]) -> Vec<ColdAtlasLineageLink> {
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for receipt in receipts {
        let Some(atlas_id) = receipt.atlas_id.as_deref() else {
            continue;
        };
        let Some(related_atlas_id) = receipt.related_atlas_id.as_deref() else {
            continue;
        };
        let (old_atlas_id, replacement_atlas_id) =
            if receipt.to_status == Some(ColdAtlasRecordStatus::Superseded) {
                (atlas_id, related_atlas_id)
            } else if receipt.outcome == ColdAtlasReceiptOutcome::Created {
                (related_atlas_id, atlas_id)
            } else {
                continue;
            };
        let key = format!("{old_atlas_id}->{replacement_atlas_id}");
        if !seen.insert(key) {
            continue;
        }
        links.push(ColdAtlasLineageLink {
            recorded_at_unix_ms: receipt.recorded_at_unix_ms,
            old_atlas_id: old_atlas_id.to_string(),
            replacement_atlas_id: replacement_atlas_id.to_string(),
        });
    }

    links.sort_by(|left, right| {
        right
            .recorded_at_unix_ms
            .cmp(&left.recorded_at_unix_ms)
            .then_with(|| left.old_atlas_id.cmp(&right.old_atlas_id))
    });
    links
}

fn describe_atlas_lineage_record(atlas_id: &str, record: Option<&ColdAtlasRecord>) -> String {
    record
        .map(|record| {
            format!(
                "{} [{} status={}]",
                record.title,
                record.atlas_id,
                record.status.as_str()
            )
        })
        .unwrap_or_else(|| format!("{atlas_id} [missing record]"))
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

fn write_jsonl_records<T>(path: impl AsRef<Path>, records: &[T], record_label: &str) -> Result<()>
where
    T: Serialize,
{
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create {record_label} dir {}", parent.display()))?;
    }
    let mut file =
        File::create(path).with_context(|| format!("rewrite {record_label} {}", path.display()))?;
    for record in records {
        let json =
            serde_json::to_string(record).with_context(|| format!("serialize {record_label}"))?;
        writeln!(file, "{json}")
            .with_context(|| format!("write {record_label} {}", path.display()))?;
    }
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
        File::open(path).with_context(|| format!("open {record_label} {}", path.display()))?;
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

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|term| term.trim().to_lowercase())
        .filter(|term| term.chars().count() >= 2)
        .collect()
}

fn score_terms(terms: &[String], fields: &[&str]) -> f32 {
    let haystack = fields
        .iter()
        .map(|field| field.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    let matched = terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    matched as f32 / terms.len().max(1) as f32
}

#[derive(Debug, Deserialize)]
struct AtlasDraftResponse {
    #[serde(default)]
    records: Vec<AtlasDraftRecord>,
}

#[derive(Debug, Deserialize)]
struct AtlasDraftRecord {
    atlas_id: String,
    title: String,
    summary: String,
    scope: String,
    confidence: ColdAtlasConfidence,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    evidence_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::memory::semantic_index::ColdEvidenceRef;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(name: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "chatty_cog_cold_atlas_test_{}_{}_{}",
            std::process::id(),
            name,
            id
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn evidence_ref() -> ColdEvidenceRef {
        ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 4, 7)
    }

    fn atlas_evidence() -> ColdAtlasEvidence {
        ColdAtlasEvidence {
            evidence_id: "atlas-evidence:interaction_log:cold_log.jsonl:4:7".to_string(),
            bucket_id: "topic:projects_code".to_string(),
            bucket_label: "Projects and code".to_string(),
            placement_id: "placement:topic:projects_code:interaction_log:cold_log.jsonl:4:7"
                .to_string(),
            placement_reason: "This exchange describes a memory bridge implementation.".to_string(),
            evidence_ref: evidence_ref(),
        }
    }

    fn atlas_output_json() -> String {
        serde_json::json!({
            "records": [
                {
                    "atlas_id": "atlas:memory-bridge:v1",
                    "title": "Memory bridge shape",
                    "summary": "The cold memory work is organized around deliberate recall, semantic buckets, and host-owned injection boundaries.",
                    "scope": "chatty-cog memory architecture",
                    "confidence": {
                        "score": 0.81,
                        "label": "supported by approved placement"
                    },
                    "tags": ["memory", "chatty-cog"],
                    "evidence_ids": ["atlas-evidence:interaction_log:cold_log.jsonl:4:7"]
                }
            ]
        })
        .to_string()
    }

    fn atlas_record() -> ColdAtlasRecord {
        ColdAtlasRecord::new(
            "atlas:miso:v1",
            "Miso vet anxiety shape",
            "Miso discussions often need careful handling around anxiety and vet logistics.",
            "pet care recall",
            ColdAtlasConfidence::new(0.72, "partial but supported"),
            1234,
            vec![evidence_ref()],
            "Drafted from bounded interaction evidence.",
        )
    }

    #[test]
    fn atlas_record_requires_evidence_refs() {
        let mut record = atlas_record();
        record.evidence_refs.clear();

        let err = record.validate().unwrap_err().to_string();

        assert!(err.contains("requires at least one evidence ref"));
    }

    #[test]
    fn atlas_store_appends_record_and_compact_receipt_without_transcript_payload() {
        let dir = temp_dir("append");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let record = atlas_record();

        let receipt =
            append_cold_atlas_record_to_store(&paths, &record, 1300, "User approved atlas record.")
                .unwrap();
        let records = read_cold_atlas_records_jsonl(&paths.records_jsonl).unwrap();
        let receipts = read_cold_atlas_receipts_jsonl(&paths.receipts_jsonl).unwrap();

        assert_eq!(records, vec![record]);
        assert_eq!(receipts, vec![receipt.clone()]);
        assert_eq!(receipt.outcome, ColdAtlasReceiptOutcome::Created);
        assert_eq!(receipt.evidence_ref_count, 1);
        let receipt_json = std::fs::read_to_string(&paths.receipts_jsonl).unwrap();
        assert!(!receipt_json.contains("Miso discussions often need careful handling"));
    }

    #[test]
    fn cold_atlas_status_change_updates_only_status_and_writes_transition_receipt() {
        let dir = temp_dir("status");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let record = atlas_record();
        append_cold_atlas_record_jsonl(&paths.records_jsonl, &record).unwrap();

        let receipt = apply_cold_atlas_status_change_to_store(
            &paths,
            "atlas:miso:v1",
            ColdAtlasRecordStatus::Stale,
            1500,
            "User marked obsolete.",
        )
        .unwrap();
        let records = read_cold_atlas_records_jsonl(&paths.records_jsonl).unwrap();
        let receipts = read_cold_atlas_receipts_jsonl(&paths.receipts_jsonl).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ColdAtlasRecordStatus::Stale);
        assert_eq!(records[0].updated_at_unix_ms, 1500);
        assert_eq!(records[0].summary, record.summary);
        assert_eq!(records[0].evidence_refs, record.evidence_refs);
        assert_eq!(receipts, vec![receipt.clone()]);
        assert_eq!(receipt.outcome, ColdAtlasReceiptOutcome::StatusChanged);
        assert_eq!(
            receipt.from_status,
            Some(ColdAtlasRecordStatus::HistoricalShape)
        );
        assert_eq!(receipt.to_status, Some(ColdAtlasRecordStatus::Stale));
        let receipt_json = std::fs::read_to_string(&paths.receipts_jsonl).unwrap();
        assert!(!receipt_json.contains("Miso discussions often need careful handling"));
    }

    #[test]
    fn cold_atlas_status_change_rejects_noop_status_change() {
        let record = atlas_record();

        let err = apply_cold_atlas_status_change(
            &[record],
            "atlas:miso:v1",
            ColdAtlasRecordStatus::HistoricalShape,
            1500,
            "noop",
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("already has status historical_shape"));
    }

    #[test]
    fn cold_atlas_prompt_carries_refs_not_transcript_payloads() {
        let prompt = render_cold_atlas_prompt(&[atlas_evidence()]).unwrap();

        assert!(prompt.contains("Cold atlas is broad historical shape memory"));
        assert!(prompt.contains("atlas-evidence:interaction_log:cold_log.jsonl:4:7"));
        assert!(prompt.contains("evidence_ref"));
        assert!(!prompt.contains("USER:"));
        assert!(!prompt.contains("ASSISTANT:"));
    }

    #[test]
    fn cold_atlas_output_must_cite_known_evidence() {
        let output = serde_json::json!({
            "records": [
                {
                    "atlas_id": "atlas:bad:v1",
                    "title": "Bad",
                    "summary": "Bad cite.",
                    "scope": "test",
                    "confidence": {"score": 0.5, "label": "weak"},
                    "evidence_ids": ["missing"]
                }
            ]
        })
        .to_string();

        let err = parse_cold_atlas_output(&[atlas_evidence()], &output, 2000)
            .unwrap_err()
            .to_string();

        assert!(err.contains("referenced unknown evidence_id"));
    }

    #[test]
    fn cold_atlas_preview_validates_without_writing() {
        let dir = temp_dir("preview");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);

        let preview =
            render_cold_atlas_review_preview(&[atlas_evidence()], &atlas_output_json(), 2000)
                .unwrap();

        assert!(preview.contains("No atlas records will be written until Commit is clicked"));
        assert!(preview.contains("Memory bridge shape"));
        assert!(!paths.records_jsonl.exists());
        assert!(!paths.receipts_jsonl.exists());
    }

    #[test]
    fn commit_prepared_cold_atlas_run_writes_record_and_receipt() {
        let dir = temp_dir("commit");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let prepared = PreparedColdAtlasRun {
            source: "test".to_string(),
            evidence: vec![atlas_evidence()],
            prompt: "prompt".to_string(),
        };

        let committed =
            commit_prepared_cold_atlas_run(&paths, &prepared, &atlas_output_json(), 2100).unwrap();
        let records = read_cold_atlas_records_jsonl(&paths.records_jsonl).unwrap();
        let receipts = read_cold_atlas_receipts_jsonl(&paths.receipts_jsonl).unwrap();

        assert_eq!(committed.record_count, 1);
        assert_eq!(committed.receipt_count, 1);
        assert_eq!(records[0].atlas_id, "atlas:memory-bridge:v1");
        assert_eq!(receipts[0].outcome, ColdAtlasReceiptOutcome::Created);
        let receipt_json = std::fs::read_to_string(&paths.receipts_jsonl).unwrap();
        assert!(!receipt_json.contains("deliberate recall, semantic buckets"));
    }

    #[test]
    fn cold_atlas_revision_replaces_one_same_id_record_with_revised_receipt() {
        let dir = temp_dir("revision");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let original = atlas_record();
        append_cold_atlas_record_jsonl(&paths.records_jsonl, &original).unwrap();
        let prepared = prepare_cold_atlas_revision_run(&original).unwrap();
        let revision_json = serde_json::json!({
            "records": [
                {
                    "atlas_id": "atlas:miso:v1",
                    "title": "Miso care shape",
                    "summary": "Miso care recall should focus on current anxiety patterns and vet logistics as historical evidence.",
                    "scope": "pet care recall",
                    "confidence": {
                        "score": 0.79,
                        "label": "revised from same evidence"
                    },
                    "tags": ["miso", "vet"],
                    "evidence_ids": [prepared.evidence[0].evidence_id]
                }
            ]
        })
        .to_string();

        let receipt = commit_prepared_cold_atlas_revision_to_store(
            &paths,
            &prepared,
            "atlas:miso:v1",
            &revision_json,
            1700,
            "User approved revision.",
        )
        .unwrap();
        let records = read_cold_atlas_records_jsonl(&paths.records_jsonl).unwrap();
        let receipts = read_cold_atlas_receipts_jsonl(&paths.receipts_jsonl).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].atlas_id, "atlas:miso:v1");
        assert_eq!(records[0].title, "Miso care shape");
        assert_eq!(records[0].created_at_unix_ms, original.created_at_unix_ms);
        assert_eq!(records[0].updated_at_unix_ms, 1700);
        assert_eq!(records[0].status, ColdAtlasRecordStatus::HistoricalShape);
        assert_eq!(records[0].evidence_refs, original.evidence_refs);
        assert_eq!(receipts, vec![receipt.clone()]);
        assert_eq!(receipt.outcome, ColdAtlasReceiptOutcome::Revised);
        assert_eq!(receipt.evidence_ref_count, 1);
        let receipt_json = std::fs::read_to_string(&paths.receipts_jsonl).unwrap();
        assert!(!receipt_json.contains("current anxiety patterns"));
    }

    #[test]
    fn cold_atlas_revision_preview_shows_before_and_after_without_writing() {
        let dir = temp_dir("revision-preview");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let original = atlas_record();
        let prepared = prepare_cold_atlas_revision_run(&original).unwrap();
        let mut revision_json = serde_json::from_str::<serde_json::Value>(
            &render_cold_atlas_record_draft_json(&original).unwrap(),
        )
        .unwrap();
        revision_json["records"][0]["summary"] =
            serde_json::json!("A revised broad shape for inspection.");

        let preview = render_cold_atlas_revision_preview(
            &original,
            &prepared,
            &revision_json.to_string(),
            1800,
        )
        .unwrap();

        assert!(preview.contains("Before"));
        assert!(preview.contains("After"));
        assert!(preview.contains("A revised broad shape for inspection."));
        assert!(!paths.records_jsonl.exists());
    }

    #[test]
    fn cold_atlas_revision_must_keep_same_atlas_id() {
        let original = atlas_record();
        let prepared = prepare_cold_atlas_revision_run(&original).unwrap();
        let revision_json = serde_json::json!({
            "records": [
                {
                    "atlas_id": "atlas:other:v1",
                    "title": "Other",
                    "summary": "Other summary.",
                    "scope": "other",
                    "confidence": {
                        "score": 0.5,
                        "label": "test"
                    },
                    "evidence_ids": [prepared.evidence[0].evidence_id]
                }
            ]
        })
        .to_string();

        let err = render_cold_atlas_revision_preview(&original, &prepared, &revision_json, 1800)
            .unwrap_err()
            .to_string();

        assert!(err.contains("must keep atlas_id 'atlas:miso:v1'"));
    }

    #[test]
    fn cold_atlas_supersede_creates_replacement_and_marks_old_record() {
        let dir = temp_dir("supersede");
        let paths = ColdAtlasPaths::under_memory_dir(&dir);
        let original = atlas_record();
        append_cold_atlas_record_jsonl(&paths.records_jsonl, &original).unwrap();
        let prepared = prepare_cold_atlas_supersede_run(&original).unwrap();
        let replacement_json = serde_json::json!({
            "records": [
                {
                    "atlas_id": "atlas:miso:v2",
                    "title": "Miso care replacement",
                    "summary": "A replacement broad shape for Miso care.",
                    "scope": "pet care recall",
                    "confidence": {
                        "score": 0.82,
                        "label": "replacement from same evidence"
                    },
                    "tags": ["miso", "care"],
                    "evidence_ids": [prepared.evidence[0].evidence_id]
                }
            ]
        })
        .to_string();

        let receipts = commit_prepared_cold_atlas_supersede_to_store(
            &paths,
            &prepared,
            "atlas:miso:v1",
            &replacement_json,
            1900,
            "User approved supersede.",
        )
        .unwrap();
        let records = read_cold_atlas_records_jsonl(&paths.records_jsonl).unwrap();
        let stored_receipts = read_cold_atlas_receipts_jsonl(&paths.receipts_jsonl).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].atlas_id, "atlas:miso:v1");
        assert_eq!(records[0].status, ColdAtlasRecordStatus::Superseded);
        assert_eq!(records[1].atlas_id, "atlas:miso:v2");
        assert_eq!(records[1].status, ColdAtlasRecordStatus::HistoricalShape);
        assert_eq!(records[1].evidence_refs, original.evidence_refs);
        assert_eq!(stored_receipts, receipts);
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].outcome, ColdAtlasReceiptOutcome::StatusChanged);
        assert_eq!(
            receipts[0].to_status,
            Some(ColdAtlasRecordStatus::Superseded)
        );
        assert_eq!(
            receipts[0].related_atlas_id.as_deref(),
            Some("atlas:miso:v2")
        );
        assert_eq!(receipts[1].outcome, ColdAtlasReceiptOutcome::Created);
        assert_eq!(
            receipts[1].related_atlas_id.as_deref(),
            Some("atlas:miso:v1")
        );
        let receipt_json = std::fs::read_to_string(&paths.receipts_jsonl).unwrap();
        assert!(!receipt_json.contains("A replacement broad shape"));
    }

    #[test]
    fn cold_atlas_supersede_preview_requires_new_atlas_id() {
        let original = atlas_record();
        let prepared = prepare_cold_atlas_supersede_run(&original).unwrap();
        let replacement_json = render_cold_atlas_record_draft_json(&original).unwrap();

        let err = render_cold_atlas_supersede_preview(
            &[original.clone()],
            &original,
            &prepared,
            &replacement_json,
            1900,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("must use a new atlas_id"));
    }

    #[test]
    fn atlas_receipts_overview_renders_related_atlas_id() {
        let record = atlas_record();
        let mut receipt = ColdAtlasReceipt::for_record(
            ColdAtlasReceiptOutcome::Created,
            &record,
            1900,
            "created",
        );
        receipt.related_atlas_id = Some("atlas:miso:v0".to_string());

        let overview = render_cold_atlas_receipts_overview(&[receipt]);

        assert!(overview.contains("related_atlas_id=atlas:miso:v0"));
    }

    #[test]
    fn atlas_lineage_overview_renders_supersede_replacement_link() {
        let mut old = atlas_record();
        old.status = ColdAtlasRecordStatus::Superseded;
        let mut replacement = atlas_record();
        replacement.atlas_id = "atlas:miso:v2".to_string();
        replacement.title = "Miso care replacement".to_string();
        replacement.summary = "Replacement summary.".to_string();
        let mut old_receipt = ColdAtlasReceipt::for_status_change(
            &old,
            ColdAtlasRecordStatus::HistoricalShape,
            ColdAtlasRecordStatus::Superseded,
            2000,
            "superseded",
        );
        old_receipt.related_atlas_id = Some(replacement.atlas_id.clone());
        let mut replacement_receipt = ColdAtlasReceipt::for_record(
            ColdAtlasReceiptOutcome::Created,
            &replacement,
            2000,
            "created",
        );
        replacement_receipt.related_atlas_id = Some(old.atlas_id.clone());

        let overview = render_cold_atlas_lineage_overview(
            &[old, replacement],
            &[old_receipt, replacement_receipt],
        );

        assert!(overview.contains("Cold atlas lineage"));
        assert!(overview.contains("Miso vet anxiety shape [atlas:miso:v1 status=superseded]"));
        assert!(overview.contains("Miso care replacement [atlas:miso:v2 status=historical_shape]"));
        assert!(overview.contains("Superseded records stay inspectable here"));
        assert!(overview.contains("replacement_summary: Replacement summary."));
    }

    #[test]
    fn atlas_lineage_links_dedupe_and_order_related_receipts() {
        let mut old = atlas_record();
        old.status = ColdAtlasRecordStatus::Superseded;
        let mut replacement = atlas_record();
        replacement.atlas_id = "atlas:miso:v2".to_string();
        replacement.title = "Miso care replacement".to_string();
        let mut old_receipt = ColdAtlasReceipt::for_status_change(
            &old,
            ColdAtlasRecordStatus::HistoricalShape,
            ColdAtlasRecordStatus::Superseded,
            2000,
            "superseded",
        );
        old_receipt.related_atlas_id = Some(replacement.atlas_id.clone());
        let mut replacement_receipt = ColdAtlasReceipt::for_record(
            ColdAtlasReceiptOutcome::Created,
            &replacement,
            2000,
            "created",
        );
        replacement_receipt.related_atlas_id = Some(old.atlas_id.clone());
        let mut newer_old = old.clone();
        newer_old.atlas_id = "atlas:miso:v2".to_string();
        let mut newer_replacement = replacement.clone();
        newer_replacement.atlas_id = "atlas:miso:v3".to_string();
        let mut newer_receipt = ColdAtlasReceipt::for_status_change(
            &newer_old,
            ColdAtlasRecordStatus::HistoricalShape,
            ColdAtlasRecordStatus::Superseded,
            3000,
            "superseded again",
        );
        newer_receipt.related_atlas_id = Some(newer_replacement.atlas_id.clone());

        let links = cold_atlas_lineage_links(&[
            old_receipt.clone(),
            replacement_receipt,
            old_receipt,
            newer_receipt,
        ]);

        assert_eq!(links.len(), 2);
        assert_eq!(links[0].old_atlas_id, "atlas:miso:v2");
        assert_eq!(links[0].replacement_atlas_id, "atlas:miso:v3");
        assert_eq!(links[1].old_atlas_id, "atlas:miso:v1");
        assert_eq!(links[1].replacement_atlas_id, "atlas:miso:v2");
    }

    #[test]
    fn atlas_lineage_overview_keeps_missing_related_records_visible() {
        let mut old = atlas_record();
        old.status = ColdAtlasRecordStatus::Superseded;
        let mut old_receipt = ColdAtlasReceipt::for_status_change(
            &old,
            ColdAtlasRecordStatus::HistoricalShape,
            ColdAtlasRecordStatus::Superseded,
            2000,
            "superseded",
        );
        old_receipt.related_atlas_id = Some("atlas:miso:v2".to_string());

        let overview = render_cold_atlas_lineage_overview(&[old], &[old_receipt]);

        assert!(overview.contains("atlas:miso:v2 [missing record]"));
    }

    #[test]
    fn cold_atlas_query_returns_evidence_refs_for_matching_shape_records() {
        let mut record = atlas_record();
        record.tags = vec!["vet".to_string(), "miso".to_string()];
        let query = ColdAtlasQuery::remember_current_input("miso vet anxiety");

        let hits = query_cold_atlas(&[record], &query).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].atlas_id, "atlas:miso:v1");
        assert_eq!(hits[0].evidence_ref, evidence_ref());
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn cold_atlas_query_excludes_retired_records() {
        let mut record = atlas_record();
        record.status = ColdAtlasRecordStatus::Retired;
        let query = ColdAtlasQuery::remember_current_input("miso vet anxiety");

        let hits = query_cold_atlas(&[record], &query).unwrap();

        assert!(hits.is_empty());
    }

    #[test]
    fn cold_atlas_query_excludes_superseded_records_by_default() {
        let mut record = atlas_record();
        record.status = ColdAtlasRecordStatus::Superseded;
        let query = ColdAtlasQuery::remember_current_input("miso vet anxiety");

        let hits = query_cold_atlas(&[record], &query).unwrap();

        assert!(hits.is_empty());
    }

    #[test]
    fn cold_atlas_query_can_include_superseded_records_for_lineage() {
        let mut record = atlas_record();
        record.status = ColdAtlasRecordStatus::Superseded;
        let mut query = ColdAtlasQuery::remember_current_input("miso vet anxiety");
        query.include_superseded = true;

        let hits = query_cold_atlas(&[record], &query).unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].status, ColdAtlasRecordStatus::Superseded);
    }

    #[test]
    fn cold_atlas_query_preview_renders_policy_without_hydrating_context() {
        let mut record = atlas_record();
        record.tags = vec!["vet".to_string(), "miso".to_string()];
        let mut query = ColdAtlasQuery::remember_current_input("miso vet anxiety");
        query.include_superseded = true;
        let hits = query_cold_atlas(&[record], &query).unwrap();

        let preview = render_cold_atlas_query_preview(&query, &hits);

        assert!(preview.contains("Cold atlas query preview"));
        assert!(preview.contains("does not hydrate cold-log context"));
        assert!(preview.contains("include_superseded: true"));
        assert!(preview.contains("Miso vet anxiety shape"));
        assert!(preview.contains("source: cold_log.jsonl events 4..7"));
        assert!(!preview.contains("USER:"));
        assert!(!preview.contains("ASSISTANT:"));
    }

    #[test]
    fn cold_atlas_query_preview_renders_empty_hits() {
        let query = ColdAtlasQuery::remember_current_input("nothing");

        let preview = render_cold_atlas_query_preview(&query, &[]);

        assert!(preview.contains("hits: 0"));
        assert!(preview.contains("no atlas hits"));
    }

    #[test]
    fn cold_atlas_query_can_exclude_stale_records() {
        let mut record = atlas_record();
        record.status = ColdAtlasRecordStatus::Stale;
        let mut query = ColdAtlasQuery::remember_current_input("miso vet anxiety");
        query.include_stale = false;

        let hits = query_cold_atlas(&[record], &query).unwrap();

        assert!(hits.is_empty());
    }

    #[test]
    fn atlas_overview_marks_records_as_shape_not_truth() {
        let overview = render_cold_atlas_overview(&[atlas_record()]);

        assert!(overview.contains("broad historical shape memory"));
        assert!(overview.contains("do not replace cold-log evidence or current truth"));
        assert!(overview.contains("Miso vet anxiety shape"));
    }

    #[test]
    fn atlas_receipts_overview_marks_no_injection_or_promotion() {
        let record = atlas_record();
        let receipt = ColdAtlasReceipt::for_record(
            ColdAtlasReceiptOutcome::Created,
            &record,
            1400,
            "approved",
        );

        let overview = render_cold_atlas_receipts_overview(&[receipt]);

        assert!(overview.contains("do not inject or promote context"));
        assert!(overview.contains("outcome=created"));
        assert!(overview.contains("atlas:miso:v1"));
    }

    #[test]
    fn atlas_receipts_overview_renders_status_transition() {
        let record = atlas_record();
        let receipt = ColdAtlasReceipt::for_status_change(
            &record,
            ColdAtlasRecordStatus::HistoricalShape,
            ColdAtlasRecordStatus::NeedsReview,
            1600,
            "review requested",
        );

        let overview = render_cold_atlas_receipts_overview(&[receipt]);

        assert!(overview.contains("outcome=status_changed"));
        assert!(overview.contains("status=historical_shape->needs_review"));
    }
}
