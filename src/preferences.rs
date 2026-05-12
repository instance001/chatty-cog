use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenParams {
    pub temp: f32,
    pub top_p: f32,
    pub top_k: i32,
    pub max_tokens: i32,
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            temp: 0.7,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 1024,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModulePreferences {
    #[serde(default)]
    pub preferred_model: Option<String>, // GGUF filename
    #[serde(default)]
    pub params: GenParams,
    #[serde(default)]
    pub allow_receive_lukewarm_context: bool,
}

impl Default for ModulePreferences {
    fn default() -> Self {
        Self {
            preferred_model: None,
            params: GenParams {
                temp: 0.3,
                top_p: 0.9,
                top_k: 40,
                max_tokens: 1024,
            },
            allow_receive_lukewarm_context: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PromptCapsule {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum VisionRoutingMode {
    #[default]
    Auto,
    PreferActive,
    ForceFallback,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppPreferences {
    #[serde(default)]
    pub network_device_id: String,
    #[serde(default)]
    pub network_recoverable_shared_chat_policy_json: Option<String>,
    #[serde(default)]
    pub active_orchestrator_capsule: Option<String>,
    #[serde(default)]
    pub orchestrator: GenParams,
    #[serde(default)]
    pub bookkeeper: GenParams,
    #[serde(default)]
    pub network_device_name: String,
    #[serde(default = "default_true")]
    pub network_allow_unknown_devices: bool,
    #[serde(default = "default_true")]
    pub network_allow_shared_lukewarm_context: bool,
    #[serde(default)]
    pub network_trusted_devices: Vec<StoredNetworkPeer>,
    #[serde(default)]
    pub network_blocked_devices: Vec<StoredNetworkPeer>,
    #[serde(default)]
    pub network_device_aliases: HashMap<String, String>,
    #[serde(default)]
    pub network_device_groups: HashMap<String, String>,
    #[serde(default)]
    pub allow_sandbox_tool_requests: bool,
    #[serde(default)]
    pub vision_routing_mode: VisionRoutingMode,
    #[serde(default = "default_true")]
    pub auto_generate_module_suspend_rundown: bool,
    #[serde(default)]
    pub orchestrator_capsules: Vec<PromptCapsule>,
    #[serde(default)]
    pub modules: HashMap<String, ModulePreferences>, // module_id -> prefs
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct StoredNetworkPeer {
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            network_device_id: String::new(),
            network_recoverable_shared_chat_policy_json: None,
            active_orchestrator_capsule: None,
            orchestrator: GenParams::default(),
            bookkeeper: GenParams {
                temp: 0.2,
                top_p: 0.9,
                top_k: 40,
                max_tokens: 256,
            },
            network_device_name: String::new(),
            network_allow_unknown_devices: true,
            network_allow_shared_lukewarm_context: true,
            network_trusted_devices: Vec::new(),
            network_blocked_devices: Vec::new(),
            network_device_aliases: HashMap::new(),
            network_device_groups: HashMap::new(),
            allow_sandbox_tool_requests: true,
            vision_routing_mode: VisionRoutingMode::Auto,
            auto_generate_module_suspend_rundown: true,
            orchestrator_capsules: Vec::new(),
            modules: HashMap::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

pub fn default_prefs_path() -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("current_dir")?;
    Ok(cwd.join("config").join("preferences.json"))
}

pub fn load_prefs(path: &Path) -> Result<AppPreferences> {
    if !path.is_file() {
        return Ok(AppPreferences::default());
    }
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let mut prefs: AppPreferences =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    // Ensure new fields are always present with sane defaults.
    if prefs.orchestrator.max_tokens <= 0 {
        prefs.orchestrator.max_tokens = 1024;
    } else if prefs.orchestrator.max_tokens == 256
        && (prefs.orchestrator.temp - 0.7).abs() < f32::EPSILON
        && (prefs.orchestrator.top_p - 0.9).abs() < f32::EPSILON
        && prefs.orchestrator.top_k == 40
    {
        // Migrate the legacy chat default so older installs don't keep truncating replies.
        prefs.orchestrator.max_tokens = 1024;
    }
    if prefs.bookkeeper.max_tokens <= 0 {
        prefs.bookkeeper.max_tokens = 256;
    }
    prefs.orchestrator_capsules.retain(|capsule| {
        !capsule.name.trim().is_empty() && !capsule.text.trim().is_empty()
    });
    Ok(prefs)
}

pub fn save_prefs(path: &Path, prefs: &AppPreferences) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(prefs).context("serialize prefs")?;
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
