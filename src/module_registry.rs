use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::module_host::ModuleVisualLoad;

#[derive(
    serde::Deserialize, serde::Serialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ModuleNetworkFeature {
    SharedStatePublish,
    SharedStateReceive,
    WorkflowBundleSend,
    WorkflowBundleReceive,
    PackSend,
    PackReceive,
    LukewarmContextPublish,
    LukewarmContextReceive,
    RoomAware,
    Multiplayer,
    HostAuthoritative,
}

impl ModuleNetworkFeature {
    pub fn label(self) -> &'static str {
        match self {
            Self::SharedStatePublish => "Shared state out",
            Self::SharedStateReceive => "Shared state in",
            Self::WorkflowBundleSend => "Workflow bundles out",
            Self::WorkflowBundleReceive => "Workflow bundles in",
            Self::PackSend => "Packs out",
            Self::PackReceive => "Packs in",
            Self::LukewarmContextPublish => "Luke warm out",
            Self::LukewarmContextReceive => "Luke warm in",
            Self::RoomAware => "Room-aware",
            Self::Multiplayer => "Multiplayer",
            Self::HostAuthoritative => "Host-authoritative",
        }
    }
}

#[derive(
    serde::Deserialize,
    serde::Serialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ModuleAssetDirection {
    Incoming,
    Outgoing,
    #[default]
    InOut,
}

impl ModuleAssetDirection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Incoming => "Incoming",
            Self::Outgoing => "Outgoing",
            Self::InOut => "In + out",
        }
    }

    pub fn supports_receive(self) -> bool {
        matches!(self, Self::Incoming | Self::InOut)
    }

    pub fn supports_send(self) -> bool {
        matches!(self, Self::Outgoing | Self::InOut)
    }

    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::InOut, _) | (_, Self::InOut) => Self::InOut,
            (Self::Incoming, Self::Outgoing) | (Self::Outgoing, Self::Incoming) => Self::InOut,
            (_, incoming) => incoming,
        }
    }
}

#[derive(
    serde::Deserialize,
    serde::Serialize,
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ModuleAssetDeliveryMode {
    InboxOnly,
    #[default]
    BridgeInbox,
}

impl ModuleAssetDeliveryMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::InboxOnly => "Inbox only",
            Self::BridgeInbox => "Bridge inbox",
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct ModuleNetworkAssetLane {
    #[serde(default)]
    pub lane_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub direction: ModuleAssetDirection,
    #[serde(default)]
    pub delivery_mode: ModuleAssetDeliveryMode,
    #[serde(default)]
    pub artifact_kinds: Vec<String>,
    #[serde(default)]
    pub accepted_content_types: Vec<String>,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default = "default_true")]
    pub replayable: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl ModuleNetworkAssetLane {
    pub fn normalize(mut self) -> Self {
        self.lane_id = canonical_asset_lane_id(&self.lane_id, &self.label, &self.artifact_kinds);
        self.label = self.label.trim().to_string();
        if self.label.is_empty() {
            self.label = self
                .lane_id
                .replace(['-', '_'], " ")
                .split_whitespace()
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
        }
        self.artifact_kinds = normalize_string_list(self.artifact_kinds);
        self.accepted_content_types = normalize_string_list(self.accepted_content_types);
        self.notes = normalize_string_list(self.notes);
        if matches!(self.max_bytes, Some(0)) {
            self.max_bytes = None;
        }
        self
    }

    pub fn merge(self, other: Self) -> Self {
        let mut merged = self;
        merged.label = if other.label.trim().is_empty() {
            merged.label
        } else {
            other.label.trim().to_string()
        };
        merged.direction = merged.direction.merge(other.direction);
        merged.delivery_mode = other.delivery_mode;
        merged.artifact_kinds.extend(other.artifact_kinds);
        merged
            .accepted_content_types
            .extend(other.accepted_content_types);
        merged.notes.extend(other.notes);
        merged.max_bytes = match (merged.max_bytes, other.max_bytes) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        };
        merged.replayable = merged.replayable && other.replayable;
        merged.normalize()
    }

    pub fn supports_receive(&self) -> bool {
        self.direction.supports_receive()
    }

    pub fn supports_send(&self) -> bool {
        self.direction.supports_send()
    }

    pub fn matches_artifact(&self, kind: &str, content_type: &str, byte_len: u64) -> bool {
        if !self.supports_receive() {
            return false;
        }
        if self.max_bytes.is_some_and(|max_bytes| byte_len > max_bytes) {
            return false;
        }

        let kind = kind.trim().to_ascii_lowercase();
        let expected_kinds = if self.artifact_kinds.is_empty() {
            vec![self.lane_id.to_ascii_lowercase()]
        } else {
            self.artifact_kinds
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<Vec<_>>()
        };
        let kind_matches = expected_kinds.iter().any(|expected| {
            expected == "*"
                || (!kind.is_empty()
                    && (expected == &kind
                        || expected
                            .strip_suffix("/*")
                            .is_some_and(|prefix| kind.starts_with(prefix))))
        });
        if !kind_matches {
            return false;
        }

        if self.accepted_content_types.is_empty() {
            return true;
        }
        let content_type = content_type.trim().to_ascii_lowercase();
        self.accepted_content_types.iter().any(|accepted| {
            let accepted = accepted.to_ascii_lowercase();
            accepted == "*"
                || accepted == content_type
                || accepted
                    .strip_suffix("/*")
                    .is_some_and(|prefix| content_type.starts_with(prefix))
        })
    }
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct ModuleNetworkCapabilities {
    #[serde(default)]
    pub features: Vec<ModuleNetworkFeature>,
    #[serde(default)]
    pub asset_lanes: Vec<ModuleNetworkAssetLane>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl ModuleNetworkCapabilities {
    pub fn normalize(mut self) -> Self {
        self.features.sort();
        self.features.dedup();
        self.asset_lanes = self
            .asset_lanes
            .into_iter()
            .map(ModuleNetworkAssetLane::normalize)
            .filter(|lane| !lane.lane_id.is_empty())
            .fold(Vec::<ModuleNetworkAssetLane>::new(), |mut lanes, lane| {
                if let Some(existing) = lanes
                    .iter_mut()
                    .find(|existing| existing.lane_id == lane.lane_id)
                {
                    *existing = existing.clone().merge(lane);
                } else {
                    lanes.push(lane);
                }
                lanes
            });
        self.notes = normalize_string_list(self.notes);
        self
    }

    pub fn merge(self, other: Self) -> Self {
        let mut merged = self;
        merged.features.extend(other.features);
        merged.asset_lanes.extend(other.asset_lanes);
        merged.notes.extend(other.notes);
        merged.normalize()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty() && self.asset_lanes.is_empty() && self.notes.is_empty()
    }

    pub fn has(&self, feature: ModuleNetworkFeature) -> bool {
        self.features.contains(&feature)
    }

    pub fn matching_receive_asset_lanes<'a>(
        &'a self,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Vec<&'a ModuleNetworkAssetLane> {
        self.asset_lanes
            .iter()
            .filter(|lane| lane.matches_artifact(kind, content_type, byte_len))
            .collect()
    }
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct ModuleManifest {
    pub module_id: String,
    pub display_name: String,
    pub icon: String,
    pub description: String,

    /// On-disk folder path for this module (filled by discovery).
    #[serde(skip, default)]
    pub dir: PathBuf,

    // Optional (reserved for future module-owned AI/runtime config)
    #[serde(default)]
    pub ai_enabled: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub visual_load: Option<ModuleVisualLoad>,
    #[serde(default)]
    pub network_capabilities: Option<ModuleNetworkCapabilities>,
}

#[derive(Debug, Default, Clone)]
pub struct ModuleRegistry {
    pub modules_dir: Option<PathBuf>,
    pub modules: Vec<ModuleManifest>,
}

impl ModuleRegistry {
    pub fn scan(modules_dir: Option<PathBuf>) -> Self {
        let modules = discover_modules(modules_dir.as_deref()).unwrap_or_default();
        Self {
            modules_dir,
            modules,
        }
    }

    pub fn refresh(&mut self) {
        self.modules = discover_modules(self.modules_dir.as_deref()).unwrap_or_default();
    }
}

pub fn discover_modules(modules_dir: Option<&Path>) -> Result<Vec<ModuleManifest>> {
    let Some(dir) = modules_dir else {
        return Ok(Vec::new());
    };
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for ent in std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))? {
        let Ok(ent) = ent else { continue };
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        let manifest_path = p.join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let mut mf: ModuleManifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        if mf.module_id.trim().is_empty() || mf.display_name.trim().is_empty() {
            continue;
        }
        mf.dir = p.clone();
        if mf.visual_load.is_none() {
            let visual_path = p.join("visual_load.json");
            if visual_path.is_file() {
                let visual_bytes = std::fs::read(&visual_path)
                    .with_context(|| format!("read {}", visual_path.display()))?;
                let visual: ModuleVisualLoad = serde_json::from_slice(&visual_bytes)
                    .with_context(|| format!("parse {}", visual_path.display()))?;
                mf.visual_load = Some(visual);
            }
        }
        let companion_network_caps = load_network_capabilities(&p)?;
        mf.network_capabilities = match (mf.network_capabilities.take(), companion_network_caps) {
            (Some(inline), Some(file)) => Some(inline.merge(file)),
            (Some(inline), None) => Some(inline.normalize()),
            (None, Some(file)) => Some(file.normalize()),
            (None, None) => None,
        }
        .filter(|caps| !caps.is_empty());
        out.push(mf);
    }

    out.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    Ok(out)
}

fn load_network_capabilities(dir: &Path) -> Result<Option<ModuleNetworkCapabilities>> {
    let path = dir.join("network_capabilities.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let caps: ModuleNetworkCapabilities =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(caps.normalize()))
}

fn default_true() -> bool {
    true
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn canonical_asset_lane_id(raw: &str, label: &str, kinds: &[String]) -> String {
    for candidate in [raw, label]
        .into_iter()
        .chain(kinds.first().map(|value| value.as_str()))
    {
        let normalized = candidate
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        if !normalized.is_empty() {
            return normalized;
        }
    }
    String::new()
}
