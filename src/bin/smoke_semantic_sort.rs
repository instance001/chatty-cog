use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chattycog_gui::app_paths::{find_default_logs_dir, find_models_dir, find_runtime_windows_dir};
use chattycog_gui::memory::bookkeeper::{BookkeeperConfig, BookkeeperHandle};
use chattycog_gui::memory::semantic_index::{ColdSemanticIndexPaths, read_bucket_map_json};
use chattycog_gui::memory::semantic_sorter::{
    commit_prepared_semantic_sort_run, parse_semantic_sorter_output, prepare_semantic_sort_run,
    render_semantic_sort_review_preview, skip_prepared_semantic_sort_run,
};

fn main() -> Result<()> {
    let runtime_dir = find_runtime_windows_dir().context("locate runtime/windows/")?;
    let models_dir = find_models_dir().context("locate models/")?;
    let logs_dir = find_default_logs_dir().context("locate memory/")?;
    let model = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| models_dir.join("gpt-oss-20b-Q4_K_M.gguf"));
    let max_items = std::env::args()
        .nth(2)
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("parse max evidence items")?
        .unwrap_or(3)
        .clamp(1, 16);
    let max_tokens = std::env::args()
        .nth(3)
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("parse max draft tokens")?
        .unwrap_or(256)
        .clamp(256, 1024);
    // Supplying an exact reviewed JSON string as argument four is the only mode
    // that writes. The normal smoke path remains draft-only.
    let reviewed_json_to_commit = std::env::args().nth(4);
    let skip_reason = std::env::args().nth(5);
    let commit_reviewed_json = reviewed_json_to_commit.is_some() && skip_reason.is_none();

    let paths = ColdSemanticIndexPaths::under_memory_dir(&logs_dir);
    let prepared = prepare_semantic_sort_run(Some(&logs_dir), &paths, max_items)?
        .context("no unsorted interaction evidence found")?;
    let bucket_map = read_bucket_map_json(&paths.bucket_map_json)?
        .context("no semantic bucket map found")?;

    let output = if let Some(reviewed_json) = reviewed_json_to_commit {
        let action = if skip_reason.is_some() {
            "Recording a skip for the supplied reviewed JSON only."
        } else {
            "Committing the supplied reviewed JSON only."
        };
        println!(
            "Prepared {} evidence item(s) after cursor {:?}. {action}",
            prepared.evidence.len(),
            prepared.after_event_index,
        );
        reviewed_json
    } else {
        println!(
            "Prepared {} evidence item(s) after cursor {:?}. Asking Bookkeeper for a draft only; no placements will be written.",
            prepared.evidence.len(),
            prepared.after_event_index
        );
        let bookkeeper = BookkeeperHandle::start_local(
            runtime_dir,
            model,
            logs_dir.clone(),
            BookkeeperConfig::default(),
        )?;
        let output = bookkeeper.draft_semantic_sort(prepared.prompt.clone(), max_tokens)?;
        bookkeeper.shutdown();
        output
    };

    println!("\n--- Raw draft JSON ---\n{output}");
    let items = parse_semantic_sorter_output(&bucket_map, &prepared.evidence, &output)
        .context("Bookkeeper draft did not satisfy the semantic-sort schema")?;
    let preview = render_semantic_sort_review_preview(&bucket_map, &prepared.evidence, &output)?;
    if items.is_empty() {
        bail!("Bookkeeper draft was valid JSON but proposed no decisions");
    }
    println!("\n--- Validated review preview ---\n{preview}");
    let now_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("read system clock")?
        .as_millis() as i64;
    if let Some(reason) = skip_reason {
        let skipped = skip_prepared_semantic_sort_run(
            &paths,
            &prepared,
            format!("semantic-sort-skipped-{now_unix_ms}"),
            now_unix_ms,
            reason,
        )?;
        println!(
            "\nSKIPPED: {} evidence item(s), no placements, receipt {}, cursor {:?}.",
            skipped.evidence_count,
            skipped.receipt.receipt_id,
            skipped.last_seen_event_index
        );
    } else if commit_reviewed_json {
        let committed = commit_prepared_semantic_sort_run(
            &paths,
            &prepared,
            &output,
            format!("semantic-sort-reviewed-{now_unix_ms}"),
            now_unix_ms,
            "Explicitly reviewed live semantic-sort JSON committed by smoke harness.",
        )?;
        println!(
            "\nCOMMITTED: {} placement(s), receipt {}, cursor {:?}.",
            committed.proposed_placement_count,
            committed.receipt.receipt_id,
            committed.last_seen_event_index
        );
    } else {
        println!("\nPASS: validated draft only; no semantic placement or receipt was committed.");
    }
    Ok(())
}
