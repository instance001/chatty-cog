use std::collections::HashMap;
use std::path::PathBuf;

use crate::module_registry::{
    ModuleAssetDeliveryMode, ModuleManifest, ModuleNetworkAssetLane, ModuleNetworkCapabilities,
    ModuleNetworkFeature, ModuleRegistry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCapabilityLaneKind {
    SharedState,
    WorkflowBundle,
    Pack,
    LukewarmContext,
    SharedRoom,
    AssetLane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCapabilityAuthority {
    Shared,
    HostAuthoritative,
}

#[derive(Debug, Clone)]
pub struct HostedCapabilityLane {
    pub lane_key: String,
    pub lane_kind: HostCapabilityLaneKind,
    pub authority: HostCapabilityAuthority,
    pub supports_publish: bool,
    pub supports_receive: bool,
    pub asset_lane: Option<ModuleNetworkAssetLane>,
}

#[derive(Debug, Clone)]
pub struct HostedModuleCapabilityRecord {
    pub module_id: String,
    pub display_name: String,
    pub dir: PathBuf,
    pub host_authoritative: bool,
    pub room_aware: bool,
    pub multiplayer: bool,
    pub features: Vec<ModuleNetworkFeature>,
    pub lanes: Vec<HostedCapabilityLane>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct HostedReceiveArtifactCandidate {
    pub module_id: String,
    pub display_name: String,
    pub host_authoritative: bool,
    pub lane: ModuleNetworkAssetLane,
}

#[derive(Debug, Clone, Default)]
pub struct HostCapabilityOrchestrator {
    modules: Vec<HostedModuleCapabilityRecord>,
    module_index: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct AssetLaneRankKey {
    authority_rank: u8,
    delivery_rank: u8,
    artifact_specificity_rank: u8,
    artifact_count_rank: usize,
    content_specificity_rank: u8,
    content_count_rank: usize,
    bounded_size_rank: u8,
    max_bytes_rank: u64,
    replayable_rank: u8,
}

impl HostCapabilityOrchestrator {
    pub fn from_registry(registry: &ModuleRegistry) -> Self {
        let modules = registry
            .modules
            .iter()
            .map(HostedModuleCapabilityRecord::from_manifest)
            .collect::<Vec<_>>();
        let module_index = modules
            .iter()
            .enumerate()
            .map(|(index, module)| (module.module_id.clone(), index))
            .collect::<HashMap<_, _>>();
        Self {
            modules,
            module_index,
        }
    }

    pub fn modules(&self) -> &[HostedModuleCapabilityRecord] {
        &self.modules
    }

    pub fn module(&self, module_id: &str) -> Option<&HostedModuleCapabilityRecord> {
        let index = self.module_index.get(module_id.trim())?;
        self.modules.get(*index)
    }

    pub fn module_allows_feature(
        &self,
        module_id: &str,
        feature: ModuleNetworkFeature,
    ) -> bool {
        self.module(module_id)
            .map(|module| module.features.contains(&feature))
            .unwrap_or(false)
    }

    pub fn module_is_host_authoritative(&self, module_id: &str) -> bool {
        self.module(module_id)
            .map(|module| module.host_authoritative)
            .unwrap_or(false)
    }

    pub fn room_capable_modules(&self) -> Vec<&HostedModuleCapabilityRecord> {
        self.modules
            .iter()
            .filter(|module| module.room_aware || module.multiplayer)
            .collect()
    }

    pub fn modules_matching_receive_artifact(
        &self,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Vec<(&HostedModuleCapabilityRecord, &HostedCapabilityLane)> {
        let mut out = Vec::new();
        for module in &self.modules {
            for lane in &module.lanes {
                if lane.lane_kind != HostCapabilityLaneKind::AssetLane {
                    continue;
                }
                let Some(asset_lane) = lane.asset_lane.as_ref() else {
                    continue;
                };
                if asset_lane.matches_artifact(kind, content_type, byte_len) {
                    out.push((module, lane));
                }
            }
        }
        out
    }

    pub fn ranked_receive_asset_lanes_for_module(
        &self,
        module_id: &str,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Vec<ModuleNetworkAssetLane> {
        let Some(module) = self.module(module_id) else {
            return Vec::new();
        };
        let mut lanes = module
            .lanes
            .iter()
            .filter(|lane| lane.lane_kind == HostCapabilityLaneKind::AssetLane)
            .filter(|lane| lane.supports_receive)
            .filter_map(|lane| lane.asset_lane.clone())
            .filter(|lane| lane.matches_artifact(kind, content_type, byte_len))
            .collect::<Vec<_>>();
        lanes.sort_by(|left, right| {
            rank_asset_lane(module.host_authoritative, left)
                .cmp(&rank_asset_lane(module.host_authoritative, right))
                .then_with(|| left.lane_id.cmp(&right.lane_id))
        });
        lanes
    }

    pub fn preferred_receive_asset_lane_for_module(
        &self,
        module_id: &str,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Option<ModuleNetworkAssetLane> {
        let Some(module) = self.module(module_id) else {
            return None;
        };
        let lanes = self.ranked_receive_asset_lanes_for_module(
            module_id,
            kind,
            content_type,
            byte_len,
        );
        let first = lanes.first()?.clone();
        let first_rank = rank_asset_lane(module.host_authoritative, &first);
        if lanes
            .get(1)
            .is_some_and(|second| rank_asset_lane(module.host_authoritative, second) == first_rank)
        {
            return None;
        }
        Some(first)
    }

    pub fn ranked_receive_artifact_candidates(
        &self,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Vec<HostedReceiveArtifactCandidate> {
        let mut candidates = Vec::new();
        for module in &self.modules {
            for lane in self.ranked_receive_asset_lanes_for_module(
                &module.module_id,
                kind,
                content_type,
                byte_len,
            ) {
                candidates.push(HostedReceiveArtifactCandidate {
                    module_id: module.module_id.clone(),
                    display_name: module.display_name.clone(),
                    host_authoritative: module.host_authoritative,
                    lane,
                });
            }
        }
        candidates.sort_by(|left, right| {
            rank_asset_lane(left.host_authoritative, &left.lane)
                .cmp(&rank_asset_lane(right.host_authoritative, &right.lane))
                .then_with(|| left.display_name.cmp(&right.display_name))
                .then_with(|| left.module_id.cmp(&right.module_id))
                .then_with(|| left.lane.lane_id.cmp(&right.lane.lane_id))
        });
        candidates
    }

    pub fn preferred_receive_artifact_candidate(
        &self,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Option<HostedReceiveArtifactCandidate> {
        let candidates = self.ranked_receive_artifact_candidates(kind, content_type, byte_len);
        let first = candidates.first()?.clone();
        let first_rank = rank_asset_lane(first.host_authoritative, &first.lane);
        if candidates.get(1).is_some_and(|second| {
            rank_asset_lane(second.host_authoritative, &second.lane) == first_rank
        }) {
            return None;
        }
        Some(first)
    }
}

impl HostedModuleCapabilityRecord {
    pub fn from_manifest(manifest: &ModuleManifest) -> Self {
        let capabilities = manifest
            .network_capabilities
            .clone()
            .unwrap_or_default()
            .normalize();
        let host_authoritative = capabilities.has(ModuleNetworkFeature::HostAuthoritative);
        let room_aware = capabilities.has(ModuleNetworkFeature::RoomAware);
        let multiplayer = capabilities.has(ModuleNetworkFeature::Multiplayer);
        let mut lanes = capability_lanes_from_caps(&capabilities, host_authoritative);
        lanes.sort_by(|left, right| left.lane_key.cmp(&right.lane_key));
        Self {
            module_id: manifest.module_id.clone(),
            display_name: manifest.display_name.clone(),
            dir: manifest.dir.clone(),
            host_authoritative,
            room_aware,
            multiplayer,
            features: capabilities.features,
            lanes,
            notes: capabilities.notes,
        }
    }
}

fn capability_lanes_from_caps(
    capabilities: &ModuleNetworkCapabilities,
    host_authoritative: bool,
) -> Vec<HostedCapabilityLane> {
    let authority = if host_authoritative {
        HostCapabilityAuthority::HostAuthoritative
    } else {
        HostCapabilityAuthority::Shared
    };
    let mut lanes = Vec::new();

    push_feature_lane(
        &mut lanes,
        capabilities,
        ModuleNetworkFeature::SharedStatePublish,
        ModuleNetworkFeature::SharedStateReceive,
        "shared_state",
        HostCapabilityLaneKind::SharedState,
        authority,
    );
    push_feature_lane(
        &mut lanes,
        capabilities,
        ModuleNetworkFeature::WorkflowBundleSend,
        ModuleNetworkFeature::WorkflowBundleReceive,
        "workflow_bundle",
        HostCapabilityLaneKind::WorkflowBundle,
        authority,
    );
    push_feature_lane(
        &mut lanes,
        capabilities,
        ModuleNetworkFeature::PackSend,
        ModuleNetworkFeature::PackReceive,
        "pack",
        HostCapabilityLaneKind::Pack,
        authority,
    );
    push_feature_lane(
        &mut lanes,
        capabilities,
        ModuleNetworkFeature::LukewarmContextPublish,
        ModuleNetworkFeature::LukewarmContextReceive,
        "lukewarm_context",
        HostCapabilityLaneKind::LukewarmContext,
        authority,
    );

    if capabilities.has(ModuleNetworkFeature::RoomAware)
        || capabilities.has(ModuleNetworkFeature::Multiplayer)
    {
        lanes.push(HostedCapabilityLane {
            lane_key: "shared_room".to_string(),
            lane_kind: HostCapabilityLaneKind::SharedRoom,
            authority,
            supports_publish: true,
            supports_receive: true,
            asset_lane: None,
        });
    }

    for asset_lane in &capabilities.asset_lanes {
        lanes.push(HostedCapabilityLane {
            lane_key: format!("asset:{}", asset_lane.lane_id),
            lane_kind: HostCapabilityLaneKind::AssetLane,
            authority,
            supports_publish: asset_lane.supports_send(),
            supports_receive: asset_lane.supports_receive(),
            asset_lane: Some(asset_lane.clone()),
        });
    }

    lanes
}

fn push_feature_lane(
    lanes: &mut Vec<HostedCapabilityLane>,
    capabilities: &ModuleNetworkCapabilities,
    publish_feature: ModuleNetworkFeature,
    receive_feature: ModuleNetworkFeature,
    lane_key: &str,
    lane_kind: HostCapabilityLaneKind,
    authority: HostCapabilityAuthority,
) {
    let supports_publish = capabilities.has(publish_feature);
    let supports_receive = capabilities.has(receive_feature);
    if !supports_publish && !supports_receive {
        return;
    }
    lanes.push(HostedCapabilityLane {
        lane_key: lane_key.to_string(),
        lane_kind,
        authority,
        supports_publish,
        supports_receive,
        asset_lane: None,
    });
}

fn rank_asset_lane(
    host_authoritative: bool,
    lane: &ModuleNetworkAssetLane,
) -> AssetLaneRankKey {
    AssetLaneRankKey {
        authority_rank: if host_authoritative { 0 } else { 1 },
        delivery_rank: match lane.delivery_mode {
            ModuleAssetDeliveryMode::BridgeInbox => 0,
            ModuleAssetDeliveryMode::InboxOnly => 1,
        },
        artifact_specificity_rank: wildcard_rank(&lane.artifact_kinds),
        artifact_count_rank: lane.artifact_kinds.len(),
        content_specificity_rank: wildcard_rank(&lane.accepted_content_types),
        content_count_rank: lane.accepted_content_types.len(),
        bounded_size_rank: if lane.max_bytes.is_some() { 0 } else { 1 },
        max_bytes_rank: lane.max_bytes.unwrap_or(u64::MAX),
        replayable_rank: if lane.replayable { 0 } else { 1 },
    }
}

fn wildcard_rank(values: &[String]) -> u8 {
    if values.is_empty() {
        return 2;
    }
    if values.iter().any(|value| value.trim() == "*") {
        return 3;
    }
    if values.iter().any(|value| value.trim().ends_with("/*")) {
        return 1;
    }
    0
}
