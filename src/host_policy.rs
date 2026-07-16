use super::*;
use chattycog_gui::capability_orchestrator::HostedReceiveArtifactCandidate;

#[derive(Debug, Clone)]
pub(super) struct WorkflowApplyRecommendation {
    pub can_apply: bool,
    pub tone: egui::Color32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(super) struct BundleApplyReadiness {
    pub recommended: bool,
    pub summary: String,
    pub orchestrator_model_status: String,
    pub bookkeeper_model_status: String,
    pub installed_module_pref_count: usize,
    pub missing_module_ids: Vec<String>,
    pub missing_module_model_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct TransferInboxRecommendation {
    pub preferred_candidate: Option<HostedReceiveArtifactCandidate>,
    pub module_candidates: Vec<HostedReceiveArtifactCandidate>,
    pub preferred_lane_id: Option<String>,
}

pub(super) fn workflow_apply_recommendation(
    app: &ChattyCogApp,
    record: &ReceivedWorkflowStateRecord,
) -> WorkflowApplyRecommendation {
    let Some(module) = app
        .module_registry
        .modules
        .iter()
        .find(|module| module.module_id == record.module_id)
    else {
        return WorkflowApplyRecommendation {
            can_apply: false,
            tone: egui::Color32::from_rgb(160, 90, 40),
            message: "This target module is not installed here yet. Keep the workflow in the inbox until the module is available.".to_string(),
        };
    };
    if !app.capability_orchestrator().module_allows_feature(
        &record.module_id,
        ModuleNetworkFeature::SharedStateReceive,
    ) {
        return WorkflowApplyRecommendation {
            can_apply: false,
            tone: egui::Color32::from_rgb(160, 90, 40),
            message: "This module has not declared `shared_state_receive` support yet, so ChattyCog will keep this workflow in the inbox.".to_string(),
        };
    }
    if let Some(reason) = app.stale_module_state_message(&module.dir, record) {
        return WorkflowApplyRecommendation {
            can_apply: false,
            tone: egui::Color32::from_rgb(160, 90, 40),
            message: format!("This looks stale against the currently applied session state: {reason}"),
        };
    }
    WorkflowApplyRecommendation {
        can_apply: true,
        tone: egui::Color32::from_rgb(50, 110, 70),
        message: "Host recommendation: this workflow target looks ready to apply on this machine."
            .to_string(),
    }
}

pub(super) fn bundle_apply_readiness(
    app: &ChattyCogApp,
    bundle: &WorkflowBundle,
) -> BundleApplyReadiness {
    let (orchestrator_ok, orchestrator_model_status) = describe_model_hint_resolution(
        app,
        bundle.orchestrator_model_hint.as_deref(),
        ModelLane::Orchestrator,
    );
    let (bookkeeper_ok, bookkeeper_model_status) = describe_model_hint_resolution(
        app,
        bundle.bookkeeper_model_hint.as_deref(),
        ModelLane::Bookkeeper,
    );
    let mut installed_module_pref_count = 0;
    let mut missing_module_ids = Vec::new();
    let mut missing_module_model_ids = Vec::new();
    for (module_id, pref) in &bundle.module_preferences {
        if app.module_manifest_by_id(module_id).is_some() {
            installed_module_pref_count += 1;
        } else {
            missing_module_ids.push(module_id.clone());
        }
        if pref
            .preferred_model
            .as_deref()
            .is_some_and(|hint| app.resolve_portable_model_hint(Some(hint)).is_none())
        {
            missing_module_model_ids.push(module_id.clone());
        }
    }
    missing_module_ids.sort();
    missing_module_model_ids.sort();
    let recommended = orchestrator_ok
        && bookkeeper_ok
        && missing_module_ids.is_empty()
        && missing_module_model_ids.is_empty();
    let mut summary_bits = Vec::new();
    if recommended {
        summary_bits.push("ready to apply cleanly on this host".to_string());
    } else {
        if !orchestrator_ok {
            summary_bits.push("orchestrator model hint needs attention".to_string());
        }
        if !bookkeeper_ok {
            summary_bits.push("bookkeeper model hint needs attention".to_string());
        }
        if !missing_module_ids.is_empty() {
            summary_bits.push(format!(
                "{} module preference target(s) are not installed",
                missing_module_ids.len()
            ));
        }
        if !missing_module_model_ids.is_empty() {
            summary_bits.push(format!(
                "{} module preference model(s) are missing locally",
                missing_module_model_ids.len()
            ));
        }
    }
    BundleApplyReadiness {
        recommended,
        summary: summary_bits.join("; "),
        orchestrator_model_status,
        bookkeeper_model_status,
        installed_module_pref_count,
        missing_module_ids,
        missing_module_model_ids,
    }
}

pub(super) fn preferred_module_asset_lane_for_transfer(
    app: &ChattyCogApp,
    module_id: &str,
    kind: &str,
    content_type: &str,
    byte_len: u64,
) -> Option<ModuleNetworkAssetLane> {
    app.capability_orchestrator()
        .preferred_receive_asset_lane_for_module(module_id, kind, content_type, byte_len)
}

pub(super) fn ranked_module_asset_targets_for_transfer(
    app: &ChattyCogApp,
    kind: &str,
    content_type: &str,
    byte_len: u64,
) -> Vec<HostedReceiveArtifactCandidate> {
    app.capability_orchestrator()
        .ranked_receive_artifact_candidates(kind, content_type, byte_len)
}

pub(super) fn preferred_module_asset_target_for_transfer(
    app: &ChattyCogApp,
    kind: &str,
    content_type: &str,
    byte_len: u64,
) -> Option<HostedReceiveArtifactCandidate> {
    app.capability_orchestrator()
        .preferred_receive_artifact_candidate(kind, content_type, byte_len)
}

pub(super) fn transfer_inbox_recommendation(
    app: &ChattyCogApp,
    record: &ReceivedGenericTransferRecord,
) -> TransferInboxRecommendation {
    if record.module_id.trim().is_empty() {
        let module_candidates = ranked_module_asset_targets_for_transfer(
            app,
            &record.kind,
            &record.content_type,
            record.byte_len,
        );
        let preferred_candidate = preferred_module_asset_target_for_transfer(
            app,
            &record.kind,
            &record.content_type,
            record.byte_len,
        );
        return TransferInboxRecommendation {
            preferred_candidate,
            module_candidates,
            preferred_lane_id: None,
        };
    }
    TransferInboxRecommendation {
        preferred_candidate: None,
        module_candidates: Vec::new(),
        preferred_lane_id: preferred_module_asset_lane_for_transfer(
            app,
            &record.module_id,
            &record.kind,
            &record.content_type,
            record.byte_len,
        )
        .map(|lane| lane.lane_id),
    }
}

fn describe_model_hint_resolution(
    app: &ChattyCogApp,
    selection: Option<&str>,
    lane: ModelLane,
) -> (bool, String) {
    let Some(selection) = selection.filter(|value| !value.trim().is_empty()) else {
        return (true, "none".to_string());
    };
    match app.resolve_model_selection(Some(selection), lane) {
        Some(ResolvedModelTarget::Local { path, .. }) => (
            true,
            format!(
                "local -> {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            ),
        ),
        Some(ResolvedModelTarget::Cloud { target, .. }) => {
            (true, format!("cloud -> {}", target.display_name))
        }
        None => (false, format!("missing -> {selection}")),
    }
}
