use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::memory::governor::{
    ColdInteractionEvidence, ColdInteractionEvidenceQuery, collect_cold_interaction_evidence,
};
use crate::memory::semantic_index::{
    ColdBucketFit, ColdBucketPlacementAuthor, ColdBucketPlacementDecision, ColdSemanticBucketKind,
    ColdSemanticBucketMap, ColdSemanticIndexPaths, ColdSemanticSortItem, ColdSemanticSortReceipt,
    build_semantic_placement_proposals, commit_semantic_sort_run_to_store,
    latest_sort_cursor_for_store, read_bucket_map_json, read_sort_receipts_jsonl,
    skip_semantic_sort_run_to_store,
    ColdSemanticSortOutcome,
};

pub const DEFAULT_SORT_SOURCE_LOG_PATH: &str = "cold_log.jsonl";
/// The host keeps local semantic-sort work deliberately small so modest models
/// can classify one exchange reliably instead of juggling a large transcript batch.
pub const HOST_SEMANTIC_SORT_MAX_EVIDENCE_PER_DRAFT: usize = 1;
pub const HOST_SEMANTIC_SORT_EVIDENCE_CHARS: usize = 1_200;
pub const HOST_SEMANTIC_SORT_OUTPUT_TOKENS: usize = 512;

#[derive(Debug, Clone)]
pub struct PreparedSemanticSortRun {
    pub source_log_path: String,
    pub after_event_index: Option<usize>,
    pub evidence: Vec<ColdInteractionEvidence>,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommittedSemanticSortRun {
    pub receipt: ColdSemanticSortReceipt,
    pub evidence_count: usize,
    pub proposed_placement_count: usize,
    pub last_seen_event_index: Option<usize>,
}

pub fn render_semantic_sorter_prompt(
    bucket_map: &ColdSemanticBucketMap,
    evidence: &[ColdInteractionEvidence],
) -> Result<String> {
    bucket_map.validate()?;

    let buckets = bucket_map
        .buckets
        .iter()
        .map(|bucket| {
            serde_json::json!({
                "bucket_id": bucket.bucket_id,
                "label": bucket.label,
                "kind": bucket.kind,
                "description": bucket.description,
                "sort_intent": bucket.sort_intent,
            })
        })
        .collect::<Vec<_>>();
    let evidence_payload = evidence
        .iter()
        .map(|item| {
            serde_json::json!({
                "evidence_id": item.id,
                "title": item.title,
                "evidence_ref": item.evidence_ref,
                "exchange_excerpt": clamp_chars(&item.exchange_block, HOST_SEMANTIC_SORT_EVIDENCE_CHARS),
            })
        })
        .collect::<Vec<_>>();

    let mut prompt = String::new();
    prompt
        .push_str("You are sorting cold interaction evidence into a frozen semantic bucket map.\n");
    prompt.push_str("Cold logs are historical evidence, not automatically current truth.\n");
    prompt.push_str("Return JSON only. Do not summarize the transcript as replacement memory.\n");
    prompt.push_str("The host gives you one compact evidence card at a time. Propose zero or more bucket placements for it.\n");
    prompt.push_str("Use positive buckets for clean fits.\n");
    prompt.push_str("A generic request to recall a rule, policy, guideline, or principle has zero decisions unless the User text also directly names a bucket subject (for example a project, code, workflow preference, person, place, pet, or explicit commitment). Do not infer a bucket from the Assistant explanation.\n");
    prompt.push_str("A weak placement is allowed only when it is directly anchored in the User portion of the evidence card. For every weak decision, include `user_evidence` containing a short exact excerpt from that User text. Never infer a user commitment, task, or preference from the Assistant portion. If there is no clean user-originated fit, return zero decisions.\n");
    prompt.push_str("CRITICAL: the misc bucket may use ONLY fit \"overflow\". Never use \"strong\", \"medium\", or \"weak\" with bucket_id \"misc\".\n");
    prompt.push_str("If any positive bucket fits, do NOT also add misc for that same evidence item. misc is only for evidence with no clean positive fit.\n");
    prompt.push_str("Valid misc decision: {\"bucket_id\":\"misc\",\"fit\":\"overflow\",\"reason\":\"no positive bucket fits cleanly\"}.\n\n");
    prompt.push_str("Output schema:\n");
    prompt.push_str(
        "{\"items\":[{\"evidence_id\":\"...\",\"decisions\":[{\"bucket_id\":\"...\",\"fit\":\"strong|medium|weak|overflow\",\"reason\":\"why this evidence belongs here\",\"user_evidence\":\"exact User excerpt; required for weak\"}]}]}\n\n",
    );
    prompt.push_str("Bucket map:\n");
    prompt.push_str(
        &serde_json::to_string_pretty(&buckets).context("serialize semantic sorter buckets")?,
    );
    prompt.push_str("\n\nEvidence:\n");
    prompt.push_str(
        &serde_json::to_string_pretty(&evidence_payload)
            .context("serialize semantic sorter evidence")?,
    );
    Ok(prompt)
}

pub fn prepare_semantic_sort_run(
    logs_dir: Option<&Path>,
    paths: &ColdSemanticIndexPaths,
    max_items: usize,
) -> Result<Option<PreparedSemanticSortRun>> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot prepare semantic sort run without bucket_map.json");
    };
    let source_log_path = DEFAULT_SORT_SOURCE_LOG_PATH.to_string();
    let after_event_index = latest_sort_cursor_for_store(paths, &source_log_path)?;
    let host_batch_size = max_items
        .clamp(1, 64)
        .min(HOST_SEMANTIC_SORT_MAX_EVIDENCE_PER_DRAFT);
    // Scan ahead only in host memory so fixture/noise records can be skipped;
    // only `host_batch_size` compact cards are ever given to the model.
    let host_scan_size = 64;
    let evidence_query = match after_event_index {
        Some(cursor) => ColdInteractionEvidenceQuery::after(cursor, host_scan_size),
        None => ColdInteractionEvidenceQuery::oldest(host_scan_size),
    };
    let mut evidence = collect_cold_interaction_evidence(logs_dir, &evidence_query)?;
    // Development fixtures are useful for smoke tests but are not user evidence
    // and must never become durable semantic placements.
    evidence.retain(|item| !item.exchange_block.contains("[SMOKE FIXTURE]"));
    evidence.truncate(host_batch_size);
    if evidence.is_empty() {
        return Ok(None);
    }
    let mut prompt = render_semantic_sorter_prompt(&bucket_map, &evidence)?;
    let recent_skip_notes = read_sort_receipts_jsonl(&paths.sort_receipts_jsonl)?
        .into_iter()
        .rev()
        .filter(|receipt| {
            receipt.source_log_path == source_log_path
                && receipt.outcome == ColdSemanticSortOutcome::Skipped
        })
        .take(3)
        .map(|receipt| receipt.note)
        .collect::<Vec<_>>();
    if !recent_skip_notes.is_empty() {
        prompt.push_str("\n\nRecent reviewer skip feedback (do not repeat these category mistakes; return zero decisions when no clean fit remains):\n");
        for note in recent_skip_notes {
            prompt.push_str("- ");
            prompt.push_str(&note);
            prompt.push('\n');
        }
    }

    Ok(Some(PreparedSemanticSortRun {
        source_log_path,
        after_event_index,
        evidence,
        prompt,
    }))
}

fn clamp_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

pub fn parse_semantic_sorter_output(
    bucket_map: &ColdSemanticBucketMap,
    evidence: &[ColdInteractionEvidence],
    output_json: &str,
) -> Result<Vec<ColdSemanticSortItem>> {
    bucket_map.validate()?;
    let response = serde_json::from_str::<SorterResponse>(output_json)
        .context("parse semantic sorter JSON output")?;
    let evidence_by_id = evidence
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    let mut seen_output_ids = HashSet::new();
    let mut decisions_by_id: HashMap<&str, Vec<ColdBucketPlacementDecision>> = HashMap::new();

    for item in response.items {
        if !seen_output_ids.insert(item.evidence_id.clone()) {
            anyhow::bail!(
                "semantic sorter output repeated evidence_id '{}'",
                item.evidence_id
            );
        }
        let Some(evidence_item) = evidence_by_id.get(item.evidence_id.as_str()) else {
            anyhow::bail!(
                "semantic sorter output referenced unknown evidence_id '{}'",
                item.evidence_id
            );
        };
        if is_generic_policy_recall(user_text_from_exchange(&evidence_item.exchange_block)) {
            // Policy recalls are useful cold-log evidence, but they are not a semantic
            // placement unless the user explicitly supplies a bucket subject. The host
            // makes that conservative decision even if a small model over-classifies it.
            decisions_by_id.insert(evidence_item.id.as_str(), Vec::new());
            continue;
        }
        let mut decisions = Vec::new();
        for decision in item.decisions {
            let fit = decision.fit.to_bucket_fit()?;
            let Some(bucket) = bucket_map.bucket(&decision.bucket_id) else {
                anyhow::bail!(
                    "semantic sorter decision referenced missing bucket '{}'",
                    decision.bucket_id
                );
            };
            if bucket.kind == ColdSemanticBucketKind::Misc && fit != ColdBucketFit::Overflow {
                anyhow::bail!(
                    "semantic sorter decision for misc bucket '{}' must use overflow fit",
                    decision.bucket_id
                );
            }
            if fit == ColdBucketFit::Overflow && bucket.kind != ColdSemanticBucketKind::Misc {
                anyhow::bail!(
                    "semantic sorter overflow decision must target misc bucket '{}'",
                    bucket_map.misc_bucket_id
                );
            }
            let reason = decision.reason.trim();
            if reason.is_empty() {
                anyhow::bail!(
                    "semantic sorter decision for bucket '{}' has an empty reason",
                    decision.bucket_id
                );
            }
            if fit == ColdBucketFit::Weak {
                let user_evidence = decision.user_evidence.trim();
                if user_evidence.is_empty() {
                    anyhow::bail!(
                        "semantic sorter weak decision for bucket '{}' must include an exact user_evidence excerpt",
                        decision.bucket_id
                    );
                }
                let user_text = user_text_from_exchange(&evidence_item.exchange_block);
                if !user_text.contains(user_evidence) {
                    anyhow::bail!(
                        "semantic sorter weak decision for bucket '{}' cites user_evidence not present in the User portion of evidence_id '{}'",
                        decision.bucket_id,
                        evidence_item.id
                    );
                }
            }
            decisions.push(ColdBucketPlacementDecision::new(
                decision.bucket_id,
                fit,
                reason.to_string(),
                ColdBucketPlacementAuthor::Sorter,
            ));
        }
        let has_positive_fit = decisions
            .iter()
            .any(|decision| decision.fit != ColdBucketFit::Overflow);
        let has_misc_overflow = decisions
            .iter()
            .any(|decision| decision.fit == ColdBucketFit::Overflow);
        if has_positive_fit && has_misc_overflow {
            anyhow::bail!(
                "semantic sorter output must not mix positive bucket placements with misc overflow for evidence_id '{}'",
                evidence_item.id
            );
        }
        build_semantic_placement_proposals(bucket_map, &evidence_item.evidence_ref, &decisions)?;
        decisions_by_id.insert(evidence_item.id.as_str(), decisions);
    }

    evidence
        .iter()
        .map(|item| {
            Ok(ColdSemanticSortItem::new(
                item.evidence_ref.clone(),
                decisions_by_id.remove(item.id.as_str()).unwrap_or_default(),
            ))
        })
        .collect()
}

fn user_text_from_exchange(exchange_block: &str) -> &str {
    let Some(user_start) = exchange_block.find("User [") else {
        return "";
    };
    let user_text = exchange_block[user_start..]
        .split_once("]:\n")
        .map(|(_, text)| text)
        .unwrap_or("");
    user_text.split("\nAssistant [").next().unwrap_or(user_text)
}

fn is_generic_policy_recall(user_text: &str) -> bool {
    let text = user_text.to_ascii_lowercase();
    let asks_for_recall = text.contains("recall") || text.contains("remember");
    let names_policy = ["rule", "policy", "guideline", "principle"]
        .iter()
        .any(|term| text.contains(term));
    let names_bucket_subject = [
        "project", "repo", "code", "bug", "test", "architecture", "workflow", "preference",
        "collaborat", "task", "commitment", "follow-up", "follow up", "person", "pet", "place",
    ]
    .iter()
    .any(|term| text.contains(term));
    asks_for_recall && names_policy && !names_bucket_subject
}

pub fn render_semantic_sort_review_preview(
    bucket_map: &ColdSemanticBucketMap,
    evidence: &[ColdInteractionEvidence],
    output_json: &str,
) -> Result<String> {
    let items = parse_semantic_sorter_output(bucket_map, evidence, output_json)?;
    let evidence_by_key = evidence
        .iter()
        .map(|item| (item.evidence_ref.stable_key(), item))
        .collect::<HashMap<_, _>>();
    let mut out = String::new();
    out.push_str("Semantic sort review preview\n");
    out.push_str("No placements will be written until Commit is clicked.\n\n");

    let mut total_decisions = 0usize;
    for (index, item) in items.iter().enumerate() {
        let key = item.evidence_ref.stable_key();
        let title = evidence_by_key
            .get(&key)
            .map(|evidence| evidence.title.as_str())
            .unwrap_or("unknown evidence");
        out.push_str(&format!(
            "{}. {} [{}..{}]\n",
            index + 1,
            title,
            item.evidence_ref.start_event_index,
            item.evidence_ref.end_event_index
        ));
        if item.decisions.is_empty() {
            out.push_str("   - no placements proposed\n");
            continue;
        }
        for decision in &item.decisions {
            total_decisions += 1;
            let bucket_label = bucket_map
                .bucket(&decision.bucket_id)
                .map(|bucket| bucket.label.as_str())
                .unwrap_or("unknown bucket");
            out.push_str(&format!(
                "   - {} ({}, {}): {}\n",
                bucket_label,
                decision.bucket_id,
                decision.fit.as_str(),
                decision.reason
            ));
        }
    }

    out.push_str(&format!(
        "\nTotal proposed placement(s): {total_decisions}\n"
    ));
    Ok(out)
}

pub fn semantic_sort_json_matches_reviewed(current_json: &str, reviewed_json: &str) -> bool {
    let current_json = current_json.trim();
    !current_json.is_empty() && current_json == reviewed_json
}

pub fn commit_prepared_semantic_sort_run(
    paths: &ColdSemanticIndexPaths,
    prepared: &PreparedSemanticSortRun,
    sorter_output_json: &str,
    receipt_id: impl Into<String>,
    sorted_at_unix_ms: i64,
    note: impl Into<String>,
) -> Result<CommittedSemanticSortRun> {
    let Some(bucket_map) = read_bucket_map_json(&paths.bucket_map_json)? else {
        anyhow::bail!("cannot commit semantic sort run without bucket_map.json");
    };
    let items = parse_semantic_sorter_output(&bucket_map, &prepared.evidence, sorter_output_json)?;
    let receipt = commit_semantic_sort_run_to_store(
        paths,
        receipt_id,
        sorted_at_unix_ms,
        prepared.source_log_path.clone(),
        &items,
        note,
    )?;
    Ok(CommittedSemanticSortRun {
        evidence_count: receipt.evidence_ref_count,
        proposed_placement_count: receipt.proposed_placement_ids.len(),
        last_seen_event_index: receipt.last_seen_event_index,
        receipt,
    })
}

pub fn skip_prepared_semantic_sort_run(
    paths: &ColdSemanticIndexPaths,
    prepared: &PreparedSemanticSortRun,
    receipt_id: impl Into<String>,
    sorted_at_unix_ms: i64,
    reviewer_reason: impl Into<String>,
) -> Result<CommittedSemanticSortRun> {
    let reviewer_reason = reviewer_reason.into();
    if reviewer_reason.trim().is_empty() {
        anyhow::bail!("provide a reviewer reason before skipping semantic evidence");
    }
    let evidence_refs = prepared
        .evidence
        .iter()
        .map(|item| item.evidence_ref.clone())
        .collect::<Vec<_>>();
    let receipt = skip_semantic_sort_run_to_store(
        paths,
        receipt_id,
        sorted_at_unix_ms,
        prepared.source_log_path.clone(),
        &evidence_refs,
        reviewer_reason,
    )?;
    Ok(CommittedSemanticSortRun {
        evidence_count: receipt.evidence_ref_count,
        proposed_placement_count: 0,
        last_seen_event_index: receipt.last_seen_event_index,
        receipt,
    })
}

#[derive(Debug, Deserialize)]
struct SorterResponse {
    #[serde(default)]
    items: Vec<SorterResponseItem>,
}

#[derive(Debug, Deserialize)]
struct SorterResponseItem {
    evidence_id: String,
    #[serde(default)]
    decisions: Vec<SorterDecision>,
}

#[derive(Debug, Deserialize)]
struct SorterDecision {
    bucket_id: String,
    fit: SorterFit,
    reason: String,
    #[serde(default)]
    user_evidence: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SorterFit {
    Strong,
    Medium,
    Weak,
    Overflow,
}

impl SorterFit {
    fn to_bucket_fit(&self) -> Result<ColdBucketFit> {
        Ok(match self {
            Self::Strong => ColdBucketFit::Strong,
            Self::Medium => ColdBucketFit::Medium,
            Self::Weak => ColdBucketFit::Weak,
            Self::Overflow => ColdBucketFit::Overflow,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::semantic_index::{
        ColdEvidenceRef, ColdSemanticBucket, ColdSemanticIndexPaths, ColdSemanticSortOutcome,
        read_placements_jsonl, read_sort_receipts_jsonl, write_bucket_map_json,
    };

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
                let unique = format!(
                    "chattycog-semantic-sorter-test-{}-{}",
                    std::process::id(),
                    nonce
                );
                Self {
                    dir: std::env::temp_dir().join(unique),
                }
            }
        }

        impl Drop for TempStore {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }

    fn bucket_map() -> ColdSemanticBucketMap {
        ColdSemanticBucketMap::new(
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
        )
    }

    fn evidence(id: &str, start: usize, end: usize, text: &str) -> ColdInteractionEvidence {
        ColdInteractionEvidence {
            id: id.to_string(),
            title: format!("interaction events {start}..={end}"),
            preview: text.to_string(),
            exchange_block: format!("[exchange events {start}..={end}]\nUser:\n{text}"),
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", start, end),
        }
    }

    fn log_line(ts: i64, source: &str, text: &str) -> String {
        serde_json::json!({
            "ts": ts,
            "kind": "cold",
            "cat": "chat",
            "source": source,
            "module_id": "",
            "event_type": "message",
            "summary": text,
            "tags": [],
            "payload_json": ""
        })
        .to_string()
    }

    fn write_test_store(lines: &[String]) -> (tempfile_path::TempStore, ColdSemanticIndexPaths) {
        let temp = tempfile_path::TempStore::new();
        std::fs::create_dir_all(&temp.dir).unwrap();
        std::fs::write(temp.dir.join("cold_log.jsonl"), lines.join("\n")).unwrap();
        let paths = ColdSemanticIndexPaths::under_memory_dir(&temp.dir);
        write_bucket_map_json(&paths.bucket_map_json, &bucket_map()).unwrap();
        (temp, paths)
    }

    #[test]
    fn sorter_prompt_includes_frozen_buckets_evidence_and_schema() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Miso needs vet notes.")];

        let prompt = render_semantic_sorter_prompt(&map, &evidence).unwrap();

        assert!(prompt.contains("Return JSON only"));
        assert!(prompt.contains("\"bucket_id\": \"entity:miso\""));
        assert!(prompt.contains("\"bucket_id\": \"misc\""));
        assert!(prompt.contains("\"evidence_id\": \"interaction:1:2\""));
        assert!(prompt.contains("fit\":\"strong|medium|weak|overflow"));
        assert!(prompt.contains("the misc bucket may use ONLY fit \"overflow\""));
        assert!(prompt.contains("do NOT also add misc"));
        assert!(prompt.contains("Valid misc decision"));
        assert!(prompt.contains("Cold logs are historical evidence"));
        assert!(prompt.contains("directly anchored in the User portion"));
    }

    #[test]
    fn sorter_prompt_uses_a_bounded_host_excerpt() {
        let map = bucket_map();
        let long_text = "x".repeat(HOST_SEMANTIC_SORT_EVIDENCE_CHARS + 80);
        let prompt = render_semantic_sorter_prompt(
            &map,
            &[evidence("interaction:1:2", 1, 2, &long_text)],
        )
        .unwrap();

        assert!(prompt.contains("exchange_excerpt"));
        assert!(prompt.contains("..."));
        assert!(!prompt.contains(&"x".repeat(HOST_SEMANTIC_SORT_EVIDENCE_CHARS + 1)));
    }

    #[test]
    fn parser_returns_sort_items_for_all_evidence_with_sorter_decisions() {
        let map = bucket_map();
        let evidence = vec![
            evidence("interaction:1:2", 1, 2, "Miso needs vet notes."),
            evidence("interaction:3:4", 3, 4, "General small talk."),
        ];
        let output = serde_json::json!({
            "items": [
                {
                    "evidence_id": "interaction:1:2",
                    "decisions": [
                        {
                            "bucket_id": "entity:miso",
                            "fit": "strong",
                            "reason": "Direct Miso care context."
                        }
                    ]
                }
            ]
        })
        .to_string();

        let items = parse_semantic_sorter_output(&map, &evidence, &output).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].evidence_ref.start_event_index, 1);
        assert_eq!(items[0].decisions.len(), 1);
        assert_eq!(items[0].decisions[0].bucket_id, "entity:miso");
        assert_eq!(items[0].decisions[0].fit, ColdBucketFit::Strong);
        assert_eq!(
            items[0].decisions[0].proposed_by,
            ColdBucketPlacementAuthor::Sorter
        );
        assert!(items[1].decisions.is_empty());
    }

    #[test]
    fn parser_accepts_explicit_zero_decisions() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Small talk.")];
        let output = serde_json::json!({
            "items": [
                {
                    "evidence_id": "interaction:1:2",
                    "decisions": []
                }
            ]
        })
        .to_string();

        let items = parse_semantic_sorter_output(&map, &evidence, &output).unwrap();

        assert_eq!(items.len(), 1);
        assert!(items[0].decisions.is_empty());
    }

    #[test]
    fn parser_suppresses_generic_policy_recall_placements() {
        let map = bucket_map();
        let evidence = vec![ColdInteractionEvidence {
            id: "interaction:1:2".to_string(),
            title: "interaction events 1..=2".to_string(),
            preview: "recall the governor evidence attachment rule".to_string(),
            exchange_block: "[exchange events 1..=2]\nUser [ts=1 source=user event_index=1]:\nrecall the governor evidence attachment rule\nAssistant [ts=2 source=assistant event_index=2]:\nEvidence must be attached.".to_string(),
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 1, 2),
        }];
        let output = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "topic:build",
                    "fit": "strong",
                    "reason": "The explanation sounds technical."
                }]
            }]
        })
        .to_string();

        let items = parse_semantic_sorter_output(&map, &evidence, &output).unwrap();
        assert!(items[0].decisions.is_empty());
    }

    #[test]
    fn parser_rejects_weak_decision_without_exact_user_anchor() {
        let map = bucket_map();
        let evidence = vec![ColdInteractionEvidence {
            id: "interaction:1:2".to_string(),
            title: "interaction events 1..=2".to_string(),
            preview: "User makes no commitment.".to_string(),
            exchange_block: "[exchange events 1..=2]\nUser [ts=1 source=user event_index=1]:\nUser makes no commitment.\nAssistant [ts=2 source=assistant event_index=2]:\nI will handle an open task.".to_string(),
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 1, 2),
        }];
        let output = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "topic:build",
                    "fit": "weak",
                    "reason": "Assistant mentions an open task.",
                    "user_evidence": "open task"
                }]
            }]
        })
        .to_string();

        let err = parse_semantic_sorter_output(&map, &evidence, &output).unwrap_err();
        assert!(err.to_string().contains("not present in the User portion"));
    }

    #[test]
    fn parser_accepts_weak_decision_with_exact_user_anchor() {
        let map = bucket_map();
        let evidence = vec![ColdInteractionEvidence {
            id: "interaction:1:2".to_string(),
            title: "interaction events 1..=2".to_string(),
            preview: "Please keep the build task open.".to_string(),
            exchange_block: "[exchange events 1..=2]\nUser [ts=1 source=user event_index=1]:\nPlease keep the build task open.\nAssistant [ts=2 source=assistant event_index=2]:\nI will keep it visible.".to_string(),
            evidence_ref: ColdEvidenceRef::interaction_log_range("cold_log.jsonl", 1, 2),
        }];
        let output = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "topic:build",
                    "fit": "weak",
                    "reason": "The user explicitly refers to the build task.",
                    "user_evidence": "build task"
                }]
            }]
        })
        .to_string();

        let items = parse_semantic_sorter_output(&map, &evidence, &output).unwrap();
        assert_eq!(items[0].decisions[0].fit, ColdBucketFit::Weak);
    }

    #[test]
    fn review_preview_renders_human_readable_decisions_without_committing() {
        let map = bucket_map();
        let evidence = vec![
            evidence("interaction:1:2", 1, 2, "Miso needs vet notes."),
            evidence("interaction:3:4", 3, 4, "General small talk."),
        ];
        let output = serde_json::json!({
            "items": [
                {
                    "evidence_id": "interaction:1:2",
                    "decisions": [
                        {
                            "bucket_id": "entity:miso",
                            "fit": "strong",
                            "reason": "Direct Miso care context."
                        }
                    ]
                },
                {
                    "evidence_id": "interaction:3:4",
                    "decisions": []
                }
            ]
        })
        .to_string();

        let preview = render_semantic_sort_review_preview(&map, &evidence, &output).unwrap();

        assert!(preview.contains("No placements will be written until Commit is clicked."));
        assert!(preview.contains("interaction events 1..=2"));
        assert!(preview.contains("Miso (entity:miso, strong): Direct Miso care context."));
        assert!(preview.contains("interaction events 3..=4"));
        assert!(preview.contains("no placements proposed"));
        assert!(preview.contains("Total proposed placement(s): 1"));
    }

    #[test]
    fn review_preview_rejects_invalid_sorter_json_before_commit() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Miso needs vet notes.")];
        let output = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "missing",
                    "fit": "strong",
                    "reason": "Bad bucket."
                }]
            }]
        })
        .to_string();

        assert!(render_semantic_sort_review_preview(&map, &evidence, &output).is_err());
    }

    #[test]
    fn review_receipt_requires_exact_current_json() {
        let reviewed = "{\"items\":[]}";

        assert!(semantic_sort_json_matches_reviewed(
            "  {\"items\":[]}  ",
            reviewed
        ));
        assert!(!semantic_sort_json_matches_reviewed(
            "{\"items\":[{\"evidence_id\":\"e1\",\"decisions\":[]}]}",
            reviewed
        ));
        assert!(!semantic_sort_json_matches_reviewed("", reviewed));
    }

    #[test]
    fn parser_rejects_unknown_evidence_unknown_bucket_and_bad_misc_fit() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Miso needs vet notes.")];
        let unknown_evidence = serde_json::json!({
            "items": [{"evidence_id": "missing", "decisions": []}]
        })
        .to_string();
        assert!(parse_semantic_sorter_output(&map, &evidence, &unknown_evidence).is_err());

        let unknown_bucket = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "missing",
                    "fit": "strong",
                    "reason": "Bad bucket."
                }]
            }]
        })
        .to_string();
        assert!(parse_semantic_sorter_output(&map, &evidence, &unknown_bucket).is_err());

        let bad_misc = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "misc",
                    "fit": "weak",
                    "reason": "Misc must be overflow."
                }]
            }]
        })
        .to_string();
        assert!(parse_semantic_sorter_output(&map, &evidence, &bad_misc).is_err());
    }

    #[test]
    fn parser_rejects_duplicate_evidence_and_empty_reason() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Miso needs vet notes.")];
        let duplicate = serde_json::json!({
            "items": [
                {"evidence_id": "interaction:1:2", "decisions": []},
                {"evidence_id": "interaction:1:2", "decisions": []}
            ]
        })
        .to_string();
        assert!(parse_semantic_sorter_output(&map, &evidence, &duplicate).is_err());

        let empty_reason = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [{
                    "bucket_id": "entity:miso",
                    "fit": "strong",
                    "reason": " "
                }]
            }]
        })
        .to_string();
        assert!(parse_semantic_sorter_output(&map, &evidence, &empty_reason).is_err());
    }

    #[test]
    fn parser_rejects_mixing_positive_and_misc_overflow_for_one_evidence_item() {
        let map = bucket_map();
        let evidence = vec![evidence("interaction:1:2", 1, 2, "Miso needs vet notes.")];
        let output = serde_json::json!({
            "items": [{
                "evidence_id": "interaction:1:2",
                "decisions": [
                    {"bucket_id": "entity:miso", "fit": "strong", "reason": "Miso is named."},
                    {"bucket_id": "misc", "fit": "overflow", "reason": "Redundant overflow."}
                ]
            }]
        })
        .to_string();

        let err = parse_semantic_sorter_output(&map, &evidence, &output).unwrap_err();
        assert!(err.to_string().contains("must not mix positive"));
    }

    #[test]
    fn prepare_semantic_sort_run_collects_new_evidence_and_renders_prompt() {
        let lines = vec![
            log_line(1, "user", "Miso needs vet notes."),
            log_line(2, "assistant", "The vet notes are ready."),
        ];
        let (temp, paths) = write_test_store(&lines);

        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();

        assert_eq!(prepared.source_log_path, DEFAULT_SORT_SOURCE_LOG_PATH);
        assert_eq!(prepared.after_event_index, None);
        assert_eq!(prepared.evidence.len(), 1);
        assert!(prepared.prompt.contains("Miso needs vet notes"));
        assert!(prepared.prompt.contains("\"bucket_id\": \"entity:miso\""));
    }

    #[test]
    fn prepare_semantic_sort_run_host_caps_each_model_draft_to_one_exchange() {
        let lines = vec![
            log_line(1, "user", "First interaction."),
            log_line(2, "assistant", "First answer."),
            log_line(3, "user", "Second interaction."),
            log_line(4, "assistant", "Second answer."),
        ];
        let (temp, paths) = write_test_store(&lines);

        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 64)
            .unwrap()
            .unwrap();

        assert_eq!(HOST_SEMANTIC_SORT_MAX_EVIDENCE_PER_DRAFT, 1);
        assert_eq!(prepared.evidence.len(), 1);
    }

    #[test]
    fn commit_prepared_semantic_sort_run_parses_commits_and_summarizes() {
        let lines = vec![
            log_line(1, "user", "Miso needs vet notes."),
            log_line(2, "assistant", "The vet notes are ready."),
        ];
        let (temp, paths) = write_test_store(&lines);
        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();
        let output = serde_json::json!({
            "items": [{
                "evidence_id": prepared.evidence[0].id,
                "decisions": [{
                    "bucket_id": "entity:miso",
                    "fit": "strong",
                    "reason": "Direct Miso care context."
                }]
            }]
        })
        .to_string();

        let committed = commit_prepared_semantic_sort_run(
            &paths,
            &prepared,
            &output,
            "sort-run-1",
            100,
            "Committed prepared run.",
        )
        .unwrap();

        assert_eq!(committed.evidence_count, 1);
        assert_eq!(committed.proposed_placement_count, 1);
        assert_eq!(committed.last_seen_event_index, Some(1));
        assert_eq!(committed.receipt.receipt_id, "sort-run-1");
        assert_eq!(
            read_placements_jsonl(&paths.placements_jsonl)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            read_sort_receipts_jsonl(&paths.sort_receipts_jsonl).unwrap(),
            vec![committed.receipt]
        );
    }

    #[test]
    fn skip_prepared_semantic_sort_run_records_reason_advances_cursor_and_writes_no_placement() {
        let lines = vec![
            log_line(1, "user", "First interaction."),
            log_line(2, "assistant", "First answer."),
            log_line(3, "user", "Second interaction."),
            log_line(4, "assistant", "Second answer."),
        ];
        let (temp, paths) = write_test_store(&lines);
        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();

        let skipped = skip_prepared_semantic_sort_run(
            &paths,
            &prepared,
            "sort-skip-1",
            100,
            "Reviewer rejected the inferred category.",
        )
        .unwrap();

        assert_eq!(skipped.evidence_count, 1);
        assert_eq!(skipped.proposed_placement_count, 0);
        assert_eq!(skipped.receipt.outcome, ColdSemanticSortOutcome::Skipped);
        assert_eq!(
            read_placements_jsonl(&paths.placements_jsonl).unwrap(),
            Vec::new()
        );
        assert_eq!(
            read_sort_receipts_jsonl(&paths.sort_receipts_jsonl).unwrap(),
            vec![skipped.receipt]
        );
        let next = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();
        assert_eq!(next.after_event_index, skipped.last_seen_event_index);
        assert_ne!(next.evidence[0].id, prepared.evidence[0].id);
        assert!(next
            .prompt
            .contains("Reviewer rejected the inferred category."));
    }

    #[test]
    fn prepare_semantic_sort_run_uses_receipt_cursor_and_returns_none_when_caught_up() {
        let lines = vec![
            log_line(1, "user", "Miso needs vet notes."),
            log_line(2, "assistant", "The vet notes are ready."),
        ];
        let (temp, paths) = write_test_store(&lines);
        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();
        let output = serde_json::json!({
            "items": [{
                "evidence_id": prepared.evidence[0].id,
                "decisions": []
            }]
        })
        .to_string();
        commit_prepared_semantic_sort_run(
            &paths,
            &prepared,
            &output,
            "sort-run-empty",
            100,
            "Advanced cursor with no placements.",
        )
        .unwrap();

        let next = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8).unwrap();

        assert!(next.is_none());
    }

    #[test]
    fn commit_prepared_semantic_sort_run_rejects_bad_output_without_writing() {
        let lines = vec![
            log_line(1, "user", "Miso needs vet notes."),
            log_line(2, "assistant", "The vet notes are ready."),
        ];
        let (temp, paths) = write_test_store(&lines);
        let prepared = prepare_semantic_sort_run(Some(&temp.dir), &paths, 8)
            .unwrap()
            .unwrap();
        let output = serde_json::json!({
            "items": [{
                "evidence_id": prepared.evidence[0].id,
                "decisions": [{
                    "bucket_id": "missing",
                    "fit": "strong",
                    "reason": "Bad bucket."
                }]
            }]
        })
        .to_string();

        assert!(
            commit_prepared_semantic_sort_run(
                &paths,
                &prepared,
                &output,
                "sort-run-bad",
                100,
                "Bad run.",
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
}
