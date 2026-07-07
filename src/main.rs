use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use std::{collections::HashMap, path::Path};

use anyhow::Context;
mod chat_actions;
mod chat_ui;
mod ecg_window;
mod logs_ui;
mod models_ui;
mod module_ui;
mod networking_ui;
mod sandbox_editor;
mod sandbox_ops;
mod shell_ui;
use chattycog_gui::app_paths::{
    find_default_logs_dir, find_models_dir, find_modules_dir, find_or_create_sandbox_dir,
    find_runtime_windows_dir, read_departments_from_logs_dir, read_gguf_architecture,
    read_lukewarm_from_logs_dir,
};
use chattycog_gui::llama_dyn;
use chattycog_gui::memory::bookkeeper::{
    BookkeeperConfig, BookkeeperHandle, EventCategory, MemoryEvent, MemoryHit, MemoryKind,
};
use chattycog_gui::module_bridge::{
    ModuleBridgeIncomingAssetRecord, ModuleBridgeIncomingSharedState, ModuleBridgeLogExcerpt,
    ModuleBridgeRoomEvent, ModuleBridgeSharedRoomEvents, ModuleBridgeSharedRoomParticipant,
    ModuleBridgeSharedRoomState, ModuleBridgeSharedState, ModuleBridgeStatus,
    bridge_incoming_asset_lane_dir, bridge_incoming_assets_dir, bridge_incoming_shared_state_path,
    bridge_log_sources_path, bridge_outgoing_room_events_path, bridge_shared_room_events_path,
    bridge_shared_room_state_path, bridge_shared_state_path, bridge_status_path,
    clear_bridge_outgoing_room_events, clear_bridge_shared_room_events,
    clear_bridge_shared_room_state, read_bridge_incoming_assets, read_bridge_incoming_shared_state,
    read_bridge_log_excerpts, read_bridge_outgoing_room_events, read_bridge_shared_room_events,
    read_bridge_shared_room_state, read_bridge_shared_state, read_bridge_status,
    write_bridge_incoming_asset, write_bridge_incoming_shared_state,
    write_bridge_shared_room_events, write_bridge_shared_room_state, write_bridge_shared_state,
};
use chattycog_gui::module_host::{HostRect, ModuleHostState, ModuleVisualLoad};
use chattycog_gui::module_registry::{
    ModuleManifest, ModuleNetworkAssetLane, ModuleNetworkFeature, ModuleRegistry,
};
use chattycog_gui::networking::{
    BlockedPeer, ConnectedPeer, DiscoveredPeer, NetworkController, NetworkSnapshot,
    OutgoingArtifactDelivery, ReceivedArtifact, ReceivedSessionEvent, TrustedPeer,
};
use chattycog_gui::preferences::{
    self, AppPreferences, GenParams, ModulePreferences, PromptCapsule,
};
use crossbeam_channel::Receiver;
use ecg_window::EcgWindowState;
use eframe::egui;
use image::ImageReader;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ChattyCog")
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([900.0, 600.0])
            .with_resizable(true),
        ..Default::default()
    };
    eframe::run_native(
        "ChattyCog",
        native_options,
        Box::new(|cc| Ok(Box::new(ChattyCogApp::new(cc)))),
    )
}

const FMI_SPLASH_IMAGE_PATH: &str = "assets/branding/fmi-splash-wordmark.png";
const FMI_SPLASH_DURATION: Duration = Duration::from_millis(3000);
const FMI_SPLASH_CLICK_DISMISS_DELAY: Duration = Duration::from_millis(450);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tab {
    Chat,
    Models,
    Logs,
    Networking,
    Sandbox,
    Settings,
    About,
    Module(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkingQuickHelpMode {
    Everyday,
    HostSetup,
    ApprovalFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkingFocusSection {
    Controls,
    PendingRequests,
    DeviceList,
    SharedRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SandboxTaskIntent {
    Create,
    Edit,
}

impl SandboxTaskIntent {
    fn label(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Edit => "Edit",
        }
    }

    fn summary_verb(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Edit => "edit",
        }
    }
}

#[derive(Debug, Clone)]
struct Message {
    role: Role,
    content: String,
    thinking: Option<String>,
}

const MAX_LIVE_CHAT_MESSAGES: usize = 48;

struct StartupSplashState {
    started_at: Instant,
    dismissed: bool,
    texture: Option<egui::TextureHandle>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceivedWorkflowStateRecord {
    artifact_id: String,
    from_device_id: String,
    from_device_name: String,
    label: String,
    summary: String,
    file_name: String,
    received_at_unix_ms: u64,
    module_id: String,
    shared_state: ModuleBridgeSharedState,
}

#[derive(Debug, Clone)]
struct ReceivedWorkflowStateInboxItem {
    path: PathBuf,
    record: ReceivedWorkflowStateRecord,
}

#[derive(Debug, Clone, Default)]
struct ModuleSessionTracker {
    session_id: String,
    last_revision: u64,
    last_fingerprint: String,
    last_shared_at_unix_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct RecoverableModuleSessionSnapshot {
    session_id: String,
    session_label: String,
    scope_module_id: String,
    scope_module_name: String,
    saved_at_unix_ms: u64,
    latest_shared_state: Option<RecoverableModuleSharedStateSnapshot>,
    recent_assets: Vec<RecoverableModuleSessionAssetSnapshot>,
}

impl RecoverableModuleSessionSnapshot {
    fn normalize(&mut self) {
        self.session_id = self.session_id.trim().to_string();
        self.session_label = self.session_label.trim().to_string();
        self.scope_module_id = self.scope_module_id.trim().to_string();
        self.scope_module_name = self.scope_module_name.trim().to_string();
        if self.saved_at_unix_ms == 0 {
            self.saved_at_unix_ms = now_unix_ms().max(0) as u64;
        }
        if let Some(shared_state) = &mut self.latest_shared_state {
            shared_state.normalize();
        }
        for asset in &mut self.recent_assets {
            asset.normalize();
        }
        self.recent_assets
            .retain(|asset| !asset.cached_payload_name.is_empty());
        self.recent_assets
            .sort_by(|left, right| right.stored_at_unix_ms.cmp(&left.stored_at_unix_ms));
        self.recent_assets.truncate(12);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct RecoverableModuleSharedStateSnapshot {
    summary: String,
    session_revision: u64,
    cached_payload_name: String,
    updated_at_unix_ms: u64,
}

impl RecoverableModuleSharedStateSnapshot {
    fn normalize(&mut self) {
        self.summary = self.summary.trim().to_string();
        self.cached_payload_name = self.cached_payload_name.trim().to_string();
        if self.updated_at_unix_ms == 0 {
            self.updated_at_unix_ms = now_unix_ms().max(0) as u64;
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct RecoverableModuleSessionAssetSnapshot {
    artifact_kind: String,
    label: String,
    summary: String,
    file_name: String,
    content_type: String,
    byte_len: u64,
    binary: bool,
    cached_payload_name: String,
    stored_at_unix_ms: u64,
}

impl RecoverableModuleSessionAssetSnapshot {
    fn normalize(&mut self) {
        self.artifact_kind = self.artifact_kind.trim().to_string();
        self.label = self.label.trim().to_string();
        self.summary = self.summary.trim().to_string();
        self.file_name = self.file_name.trim().to_string();
        self.content_type = self.content_type.trim().to_string();
        self.cached_payload_name = self.cached_payload_name.trim().to_string();
        if self.stored_at_unix_ms == 0 {
            self.stored_at_unix_ms = now_unix_ms().max(0) as u64;
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ModuleSessionAckRecord {
    module_id: String,
    session_id: String,
    session_revision: u64,
    from_device_id: String,
    from_device_name: String,
    applied: bool,
    stale: bool,
    message: String,
    acknowledged_at_unix_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct WorkflowBundle {
    version: String,
    label: String,
    summary: String,
    created_at_unix_ms: u64,
    system_prompt: String,
    orchestrator_model_hint: Option<String>,
    orchestrator_params: GenParams,
    bookkeeper_model_hint: Option<String>,
    bookkeeper_params: GenParams,
    allow_sandbox_tool_requests: bool,
    auto_generate_module_suspend_rundown: bool,
    module_preferences: HashMap<String, ModulePreferences>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceivedWorkflowBundleRecord {
    artifact_id: String,
    from_device_id: String,
    from_device_name: String,
    label: String,
    summary: String,
    file_name: String,
    received_at_unix_ms: u64,
    bundle: WorkflowBundle,
}

#[derive(Debug, Clone)]
struct ReceivedWorkflowBundleInboxItem {
    path: PathBuf,
    record: ReceivedWorkflowBundleRecord,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SharedLukewarmContext {
    version: String,
    label: String,
    summary: String,
    created_at_unix_ms: u64,
    source_app: String,
    source_device_id: String,
    source_device_name: String,
    context_text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceivedLukewarmContextRecord {
    artifact_id: String,
    from_device_id: String,
    from_device_name: String,
    label: String,
    summary: String,
    file_name: String,
    received_at_unix_ms: u64,
    context: SharedLukewarmContext,
}

#[derive(Debug, Clone)]
struct ReceivedLukewarmContextInboxItem {
    path: PathBuf,
    record: ReceivedLukewarmContextRecord,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceivedGenericTransferLaneDelivery {
    #[serde(default)]
    lane_id: String,
    #[serde(default)]
    lane_label: String,
    #[serde(default)]
    delivered_at_unix_ms: u64,
    #[serde(default)]
    bridge_record_path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ReceivedGenericTransferRecord {
    artifact_id: String,
    from_device_id: String,
    from_device_name: String,
    label: String,
    summary: String,
    kind: String,
    module_id: String,
    file_name: String,
    content_type: String,
    transfer_encoding: String,
    byte_len: u64,
    chunk_count: u32,
    received_at_unix_ms: u64,
    binary: bool,
    payload_file_name: String,
    preview_text: String,
    #[serde(default)]
    delivered_lanes: Vec<ReceivedGenericTransferLaneDelivery>,
}

#[derive(Debug, Clone)]
struct ReceivedGenericTransferInboxItem {
    path: PathBuf,
    record: ReceivedGenericTransferRecord,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NetworkPeerExchangeRecord {
    device_id: String,
    #[serde(default)]
    device_name: String,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    group: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NetworkPeerExchangeFile {
    version: String,
    source_app: String,
    source_device_id: String,
    source_device_name: String,
    exported_at_unix_ms: u64,
    peers: Vec<NetworkPeerExchangeRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SharedChatTurnMode {
    Open,
    TalkingStick,
}

impl Default for SharedChatTurnMode {
    fn default() -> Self {
        Self::Open
    }
}

impl SharedChatTurnMode {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::TalkingStick => "Talking stick",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SharedChatAiMode {
    Off,
    LocalAllowed,
    HostOnly,
}

impl Default for SharedChatAiMode {
    fn default() -> Self {
        Self::LocalAllowed
    }
}

impl SharedChatAiMode {
    fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::LocalAllowed => "Local allowed",
            Self::HostOnly => "Host only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SharedChatScopeKind {
    General,
    Module,
}

impl Default for SharedChatScopeKind {
    fn default() -> Self {
        Self::General
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SharedChatPolicy {
    version: String,
    label: String,
    updated_at_unix_ms: u64,
    source_app: String,
    host_device_id: String,
    host_device_name: String,
    #[serde(default)]
    turn_mode: SharedChatTurnMode,
    #[serde(default)]
    ai_mode: SharedChatAiMode,
    #[serde(default)]
    scope_kind: SharedChatScopeKind,
    #[serde(default)]
    scope_module_id: String,
    #[serde(default)]
    scope_module_name: String,
    #[serde(default)]
    scope_multiplayer: bool,
    #[serde(default)]
    session_active: bool,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    session_revision: u64,
    #[serde(default)]
    session_label: String,
    #[serde(default)]
    host_authoritative: bool,
    #[serde(default)]
    turn_holder_device_id: String,
    #[serde(default)]
    turn_holder_device_name: String,
    #[serde(default)]
    teacher_override: bool,
    #[serde(default)]
    host_activity_state: String,
    #[serde(default)]
    host_activity_label: String,
    #[serde(default)]
    host_activity_updated_at_unix_ms: u64,
}

impl Default for SharedChatPolicy {
    fn default() -> Self {
        Self {
            version: "1".to_string(),
            label: "Shared room".to_string(),
            updated_at_unix_ms: 0,
            source_app: "chattycog".to_string(),
            host_device_id: String::new(),
            host_device_name: String::new(),
            turn_mode: SharedChatTurnMode::Open,
            ai_mode: SharedChatAiMode::LocalAllowed,
            scope_kind: SharedChatScopeKind::General,
            scope_module_id: String::new(),
            scope_module_name: String::new(),
            scope_multiplayer: false,
            session_active: false,
            session_id: String::new(),
            session_revision: 0,
            session_label: String::new(),
            host_authoritative: false,
            turn_holder_device_id: String::new(),
            turn_holder_device_name: String::new(),
            teacher_override: false,
            host_activity_state: String::new(),
            host_activity_label: String::new(),
            host_activity_updated_at_unix_ms: 0,
        }
    }
}

impl SharedChatPolicy {
    fn equivalent_except_presence(&self, other: &Self) -> bool {
        self.version == other.version
            && self.label == other.label
            && self.source_app == other.source_app
            && self.host_device_id == other.host_device_id
            && self.host_device_name == other.host_device_name
            && self.turn_mode == other.turn_mode
            && self.ai_mode == other.ai_mode
            && self.scope_kind == other.scope_kind
            && self.scope_module_id == other.scope_module_id
            && self.scope_module_name == other.scope_module_name
            && self.scope_multiplayer == other.scope_multiplayer
            && self.session_active == other.session_active
            && self.session_id == other.session_id
            && self.session_revision == other.session_revision
            && self.session_label == other.session_label
            && self.host_authoritative == other.host_authoritative
            && self.turn_holder_device_id == other.turn_holder_device_id
            && self.turn_holder_device_name == other.turn_holder_device_name
            && self.teacher_override == other.teacher_override
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SharedChatMessage {
    version: String,
    message_id: String,
    sent_at_unix_ms: u64,
    source_app: String,
    from_device_id: String,
    from_device_name: String,
    speaker_kind: String,
    speaker_label: String,
    #[serde(default)]
    scope_kind: SharedChatScopeKind,
    #[serde(default)]
    scope_module_id: String,
    #[serde(default)]
    scope_module_name: String,
    #[serde(default)]
    scope_multiplayer: bool,
    #[serde(default)]
    session_active: bool,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    session_revision: u64,
    body: String,
}

#[derive(Debug, Clone)]
enum SandboxAction {
    Write {
        path: String,
        contents: String,
    },
    Append {
        path: String,
        contents: String,
    },
    Read {
        path: String,
    },
    List,
    Ledger {
        status: String,
        current_task: String,
        next_step: String,
        open_questions: Vec<String>,
        files_touched: Vec<String>,
        notes: Vec<String>,
    },
    Preload {
        paths: Vec<String>,
        include_list: bool,
        include_scratchpad: bool,
        include_ledger: bool,
        note: String,
    },
}

const DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH: &str = "scratchpad/current.md";
const DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH: &str = "scratchpad/task_ledger.md";

#[derive(Debug, Clone)]
enum ModelOptionGroup {
    VisionReady,
    VisionNeedsMmproj,
    Standard,
}

impl ModelOptionGroup {
    fn rank(&self) -> u8 {
        match self {
            Self::VisionReady => 0,
            Self::VisionNeedsMmproj => 1,
            Self::Standard => 2,
        }
    }

    fn heading(&self) -> &'static str {
        match self {
            Self::VisionReady => "Vision Ready",
            Self::VisionNeedsMmproj => "Vision Needs Projector",
            Self::Standard => "Text / Other Models",
        }
    }
}

#[derive(Debug, Clone)]
struct ModelOption {
    label: String,
    value: String, // stored in prefs; either a filename or "modules/<module_id>/<file>.gguf"
    group: ModelOptionGroup,
}

#[derive(Debug, Clone)]
struct ModuleAiState {
    model_path: Option<PathBuf>,
    models_cache: Vec<PathBuf>,
    temp: f32,
    top_p: f32,
    top_k: i32,
    max_tokens: i32,
    user_input: String,
    output: String,
    is_running: bool,
    cancel: Option<Arc<AtomicBool>>,
    rx: Option<Receiver<GenEvent>>,
    status: String,
    initialized: bool,
}

impl Default for ModuleAiState {
    fn default() -> Self {
        Self {
            model_path: None,
            models_cache: Vec::new(),
            temp: 0.3,
            top_p: 0.9,
            top_k: 40,
            max_tokens: 1024,
            user_input: String::new(),
            output: String::new(),
            is_running: false,
            cancel: None,
            rx: None,
            status: String::new(),
            initialized: false,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiSpec {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    sections: Vec<ModuleUiSection>,
    #[serde(default)]
    fields: Vec<ModuleUiField>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiSection {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<ModuleUiBlock>,
    #[serde(default)]
    pub sidebar: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiBlock {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub field: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tone: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub empty: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub points: Option<f32>,
    #[serde(default)]
    pub max_entries: Option<usize>,
    #[serde(default)]
    pub max_lines: Option<usize>,
    #[serde(default)]
    pub max_rows: Option<usize>,
    #[serde(default)]
    pub has_header: Option<bool>,
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub tabs: Vec<ModuleUiPane>,
    #[serde(default)]
    pub columns: Vec<ModuleUiPane>,
    #[serde(default)]
    pub panes: Vec<ModuleUiPane>,
    #[serde(default)]
    pub lanes: Vec<String>,
    #[serde(default)]
    pub searchable: Option<bool>,
    #[serde(default)]
    pub filter_placeholder: String,
    #[serde(default)]
    pub filter_presets: Vec<ModuleUiFilterPreset>,
    #[serde(default)]
    pub view_presets: Vec<ModuleUiViewPreset>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiFilterPreset {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub query: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiViewPreset {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub pane_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiPane {
    #[serde(default)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub summary_field: String,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<ModuleUiBlock>,
    #[serde(default)]
    pub weight: Option<f32>,
    #[serde(default)]
    pub default_open: Option<bool>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ModuleUiField {
    pub id: String,
    pub label: String,
    /// Supported: "singleline" | "multiline" | "number" | "bool" | "choice"
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub help: String,
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub rows: Option<usize>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone)]
enum ModuleFieldValue {
    Str(String),
    Bool(bool),
    Num(f64),
}

#[derive(Debug, Default)]
struct ModuleFormState {
    loaded: bool,
    spec_path: PathBuf,
    state_path: PathBuf,
    spec: Option<ModuleUiSpec>,
    values: HashMap<String, ModuleFieldValue>,
    status: String,
}

#[derive(Debug, Default)]
struct ModuleWorkspaceState {
    loaded: bool,
    path: PathBuf,
    template_path: Option<PathBuf>,
    text: String,
    status: String,
}

#[derive(Debug, Clone)]
enum ResolvedModuleUiBlock {
    Field(ModuleUiField),
    Text {
        title: String,
        text: String,
    },
    Markdown {
        title: String,
        text: String,
        field_id: String,
        empty: String,
    },
    Callout {
        title: String,
        text: String,
        tone: String,
    },
    Stat {
        label: String,
        field_id: String,
        empty: String,
    },
    Actions {
        actions: Vec<String>,
    },
    Progress {
        label: String,
        field_id: String,
        min: Option<f64>,
        max: Option<f64>,
        empty: String,
    },
    Record {
        title: String,
        ui_id: String,
        field_ids: Vec<String>,
        empty: String,
    },
    Table {
        title: String,
        ui_id: String,
        field_id: String,
        path: String,
        empty: String,
        max_rows: usize,
        has_header: bool,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    Checklist {
        title: String,
        ui_id: String,
        field_id: String,
        path: String,
        empty: String,
        max_rows: usize,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    Timeline {
        title: String,
        ui_id: String,
        field_id: String,
        path: String,
        empty: String,
        max_rows: usize,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    Kanban {
        title: String,
        ui_id: String,
        field_id: String,
        path: String,
        empty: String,
        max_rows: usize,
        lanes: Vec<String>,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    BarChart {
        title: String,
        field_ids: Vec<String>,
        min: Option<f64>,
        max: Option<f64>,
        empty: String,
    },
    DependencyGraph {
        title: String,
        ui_id: String,
        field_id: String,
        path: String,
        empty: String,
        max_rows: usize,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    Tabs {
        title: String,
        ui_id: String,
        panes: Vec<ResolvedModuleUiPane>,
        view_presets: Vec<ModuleUiViewPreset>,
    },
    Split {
        title: String,
        ui_id: String,
        direction: String,
        panes: Vec<ResolvedModuleUiPane>,
        view_presets: Vec<ModuleUiViewPreset>,
    },
    Accordion {
        title: String,
        ui_id: String,
        panes: Vec<ResolvedModuleUiPane>,
        inspector_style: bool,
        view_presets: Vec<ModuleUiViewPreset>,
    },
    FileList {
        title: String,
        ui_id: String,
        path: String,
        empty: String,
        max_entries: usize,
        searchable: bool,
        filter_placeholder: String,
        filter_presets: Vec<ModuleUiFilterPreset>,
    },
    ArtifactPreview {
        title: String,
        path: String,
        field_id: String,
        empty: String,
        max_lines: usize,
    },
    Separator,
    Spacer(f32),
}

#[derive(Debug, Clone)]
struct ResolvedModuleUiPane {
    id: String,
    title: String,
    description: String,
    summary: String,
    summary_field: String,
    blocks: Vec<ResolvedModuleUiBlock>,
    weight: f32,
    default_open: bool,
}

#[derive(Debug, Clone)]
struct ResolvedModuleUiSection {
    title: String,
    description: String,
    blocks: Vec<ResolvedModuleUiBlock>,
    sidebar: bool,
}

impl ModuleFormState {
    fn new(module_dir: &Path) -> Self {
        Self {
            loaded: false,
            spec_path: module_dir.join("ui.json"),
            state_path: module_dir.join("state.json"),
            spec: None,
            values: HashMap::new(),
            status: String::new(),
        }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;

        if !self.spec_path.is_file() {
            self.spec = None;
            self.status = "No ui.json found for this module.".to_string();
            return;
        }

        let bytes = match std::fs::read(&self.spec_path) {
            Ok(b) => b,
            Err(e) => {
                self.spec = None;
                self.status = format!("Failed to read ui.json: {e}");
                return;
            }
        };
        let spec: ModuleUiSpec = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                self.spec = None;
                self.status = format!("Failed to parse ui.json: {e}");
                return;
            }
        };

        // Initialize defaults
        self.values.clear();
        for f in &spec.fields {
            let id = f.id.trim();
            if id.is_empty() {
                continue;
            }
            let kind = f.kind.trim().to_lowercase();
            let v = match kind.as_str() {
                "bool" => ModuleFieldValue::Bool(false),
                "number" => ModuleFieldValue::Num(f.min.unwrap_or(0.0)),
                "choice" => ModuleFieldValue::Str(f.options.first().cloned().unwrap_or_default()),
                "singleline" | "multiline" | _ => ModuleFieldValue::Str(String::new()),
            };
            self.values.insert(id.to_string(), v);
        }

        // Load persisted state (best-effort)
        if self.state_path.is_file() {
            if let Ok(bytes) = std::fs::read(&self.state_path) {
                if let Ok(obj) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(map) = obj.as_object() {
                        for f in &spec.fields {
                            let id = f.id.trim();
                            if id.is_empty() {
                                continue;
                            }
                            let Some(v) = map.get(id) else { continue };
                            let kind = f.kind.trim().to_lowercase();
                            let slot = self
                                .values
                                .entry(id.to_string())
                                .or_insert(ModuleFieldValue::Str(String::new()));
                            match kind.as_str() {
                                "bool" => {
                                    if !matches!(slot, ModuleFieldValue::Bool(_)) {
                                        *slot = ModuleFieldValue::Bool(false);
                                    }
                                    let ModuleFieldValue::Bool(b) = slot else {
                                        unreachable!()
                                    };
                                    if let Some(x) = v.as_bool() {
                                        *b = x;
                                    } else if let Some(s) = v.as_str() {
                                        *b = matches!(
                                            s.trim().to_lowercase().as_str(),
                                            "true" | "1" | "yes" | "y"
                                        );
                                    }
                                }
                                "number" => {
                                    if !matches!(slot, ModuleFieldValue::Num(_)) {
                                        *slot = ModuleFieldValue::Num(0.0);
                                    }
                                    let ModuleFieldValue::Num(n) = slot else {
                                        unreachable!()
                                    };
                                    if let Some(x) = v.as_f64() {
                                        *n = x;
                                    } else if let Some(s) = v.as_str() {
                                        if let Ok(x) = s.trim().parse::<f64>() {
                                            *n = x;
                                        }
                                    }
                                }
                                // default: string
                                _ => {
                                    if !matches!(slot, ModuleFieldValue::Str(_)) {
                                        *slot = ModuleFieldValue::Str(String::new());
                                    }
                                    let ModuleFieldValue::Str(s) = slot else {
                                        unreachable!()
                                    };
                                    if let Some(x) = v.as_str() {
                                        s.clear();
                                        s.push_str(x);
                                    } else if v.is_number() || v.is_boolean() {
                                        s.clear();
                                        s.push_str(&v.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.spec = Some(spec);
        self.status = format!("Loaded {} fields.", self.values.len());
    }

    fn reload(&mut self) {
        self.loaded = false;
        self.ensure_loaded();
    }

    fn save(&mut self) {
        let Some(spec) = &self.spec else {
            self.status = "No UI spec loaded; nothing to save.".to_string();
            return;
        };

        let mut map = serde_json::Map::new();
        for f in &spec.fields {
            let id = f.id.trim();
            if id.is_empty() {
                continue;
            }
            let kind = f.kind.trim().to_lowercase();
            let v = self.values.get(id);
            let json_v = match (kind.as_str(), v) {
                ("bool", Some(ModuleFieldValue::Bool(b))) => serde_json::Value::Bool(*b),
                ("number", Some(ModuleFieldValue::Num(n))) => serde_json::Value::Number(
                    serde_json::Number::from_f64(*n).unwrap_or_else(|| serde_json::Number::from(0)),
                ),
                (_, Some(ModuleFieldValue::Str(s))) => serde_json::Value::String(s.clone()),
                _ => serde_json::Value::Null,
            };
            map.insert(id.to_string(), json_v);
        }

        let bytes = match serde_json::to_vec_pretty(&serde_json::Value::Object(map)) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Failed to serialize state: {e}");
                return;
            }
        };
        if let Err(e) = std::fs::write(&self.state_path, bytes) {
            self.status = format!("Save failed: {e}");
            return;
        }
        self.status = format!("Saved state to {}", self.state_path.display());
    }
}

impl ModuleWorkspaceState {
    fn new(module_dir: &Path) -> Self {
        let template_path = module_dir.join("STATE_TEMPLATE.md");
        Self {
            loaded: false,
            path: module_dir.join("workspace.md"),
            template_path: template_path.is_file().then_some(template_path),
            text: String::new(),
            status: String::new(),
        }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;

        if self.path.is_file() {
            match read_text_file(&self.path, 2_000_000) {
                Ok(t) => {
                    self.text = t;
                    self.status = format!("Loaded {}", self.path.display());
                }
                Err(e) => {
                    self.status = format!("Failed to load workspace: {e}");
                }
            }
            return;
        }

        if let Some(tp) = &self.template_path {
            if let Ok(t) = read_text_file(tp, 2_000_000) {
                self.text = t;
                self.status = "Loaded module template.".to_string();
                return;
            }
        }

        self.text.clear();
        self.status = "New workspace.".to_string();
    }

    fn reload(&mut self) {
        self.loaded = false;
        self.ensure_loaded();
    }

    fn load_template(&mut self) {
        if let Some(tp) = &self.template_path {
            match read_text_file(tp, 2_000_000) {
                Ok(t) => {
                    self.text = t;
                    self.status = "Loaded module template.".to_string();
                }
                Err(e) => self.status = format!("Template load failed: {e}"),
            }
        } else {
            self.status = "No STATE_TEMPLATE.md found.".to_string();
        }
    }

    fn save(&mut self) {
        if let Err(e) = std::fs::write(&self.path, &self.text) {
            self.status = format!("Save failed: {e}");
            return;
        }
        self.status = format!("Saved {}", self.path.display());
    }
}

struct ModuleRundownJob {
    rx: Receiver<String>,
    overwrite_existing: bool,
}

struct ChattyCogApp {
    tab: Tab,
    prev_tab: Tab,
    startup_splash: StartupSplashState,

    show_left_sidebar: bool,
    gguf_path: Option<PathBuf>,
    models_dir: Option<PathBuf>,
    models_cache: Vec<PathBuf>,

    messages: Vec<Message>,
    composer: String,
    composer_had_focus_last_frame: bool,

    // Generation
    is_generating: bool,
    gen_cancel: Option<Arc<AtomicBool>>,
    gen_rx: Option<Receiver<GenEvent>>,
    assistant_draft: String,
    runtime_status: String,
    model_runtime_issues: HashMap<String, String>,
    runtime_info_rx: Option<Receiver<String>>,
    bookkeeper: Option<BookkeeperHandle>,
    logs_dir: Option<PathBuf>,
    logs_selected: Option<PathBuf>,
    logs_view: String,
    logs_query_semantic: String,
    logs_results_semantic: Vec<MemoryHit>,
    logs_query_keyword: String,
    logs_results_keyword: Vec<String>,
    logs_filter_module: String,
    logs_filter_tag: String,
    logs_new_module: String,
    logs_new_event_type: String,
    logs_new_tags: String,
    logs_new_summary: String,
    logs_new_payload_json: String,
    bookkeeper_model_path: Option<PathBuf>,
    bookkeeper_temp: f32,
    bookkeeper_top_p: f32,
    bookkeeper_top_k: i32,
    bookkeeper_max_tokens: i32,
    bookkeeper_restart_due: Option<Instant>,
    lukewarm_summary: String,
    lukewarm_poll_due: Option<Instant>,
    lukewarm_rx: Option<Receiver<String>>,

    // UI
    scroll_to_bottom: bool,
    ecg_window: EcgWindowState,

    // Orchestrator "hot memory" (small, always-visible working set)
    hot_memory: Vec<String>,

    // Orchestrator generation params
    orch_temp: f32,
    orch_top_p: f32,
    orch_top_k: i32,
    orch_max_tokens: i32,
    orch_freeze_pending: bool,

    // Preferences
    prefs_path: PathBuf,
    prefs: AppPreferences,
    prefs_status: String,
    capsule_editor_name: String,
    capsule_editor_text: String,
    capsule_selected_name: Option<String>,

    // Orchestrator sandbox
    sandbox_dir: Option<PathBuf>,
    sandbox_selected: Option<PathBuf>,
    sandbox_editor_path: Option<PathBuf>,
    sandbox_last_working_path: Option<PathBuf>,
    sandbox_editor_text: String,
    sandbox_status: String,
    sandbox_last_tool_result: String,
    chat_selected_file: Option<PathBuf>,
    sandbox_task_nudge: String,
    sandbox_task_enabled: bool,
    sandbox_task_intent: SandboxTaskIntent,
    sandbox_task_path: String,

    // Modules
    modules_dir: Option<PathBuf>,
    module_registry: ModuleRegistry,
    module_state_notes: HashMap<String, String>,
    module_ai: HashMap<String, ModuleAiState>,
    module_forms: HashMap<String, ModuleFormState>,
    module_workspaces: HashMap<String, ModuleWorkspaceState>,
    module_rundown_jobs: HashMap<String, ModuleRundownJob>,
    module_bridge_last_fingerprint: HashMap<String, String>,
    module_room_bridge_last_fingerprint: HashMap<String, String>,
    module_room_events_bridge_last_fingerprint: HashMap<String, String>,
    module_session_trackers: HashMap<String, ModuleSessionTracker>,
    module_session_receipts: Vec<ModuleSessionAckRecord>,
    module_hosts: HashMap<String, ModuleHostState>,
    module_host_targets: HashMap<String, HostRect>,
    open_module_tabs: Vec<String>,
    close_pending_modules: HashSet<String>,

    // Orchestrator tool requests (user-approved)
    pending_sandbox_actions: Vec<SandboxAction>,
    sandbox_action_status: String,

    // Local networking
    networking: NetworkController,
    networking_device_name_input: String,
    networking_status: String,
    networking_filter: String,
    networking_selected_devices: HashSet<String>,
    networking_help_mode: NetworkingQuickHelpMode,
    networking_focus_section: Option<NetworkingFocusSection>,
    networking_focus_pending: Option<NetworkingFocusSection>,
    networking_focus_flash_until: Option<Instant>,
    networking_alias_edit_device: Option<String>,
    networking_alias_input: String,
    networking_group_edit_device: Option<String>,
    networking_group_input: String,
    networking_handoff_target: String,
    networking_handoff_title: String,
    networking_handoff_body: String,
    networking_shared_chat_policy: SharedChatPolicy,
    networking_recoverable_shared_chat_policy: Option<SharedChatPolicy>,
    networking_recoverable_module_session: Option<RecoverableModuleSessionSnapshot>,
    networking_shared_chat_log: Vec<SharedChatMessage>,
    networking_shared_chat_seen_messages: HashSet<String>,
    networking_shared_chat_input: String,
    networking_shared_chat_mirror_main_chat: bool,
    networking_shared_chat_presence_key: String,
    networking_shared_chat_connected_peer_keys: HashSet<String>,
    networking_shared_chat_presence_next_sync_at: Option<Instant>,
    networking_seen_handoffs: HashSet<String>,
    networking_seen_artifacts: HashSet<String>,
    received_workflow_inbox: Vec<ReceivedWorkflowStateInboxItem>,
    selected_received_workflow: Option<PathBuf>,
    received_lukewarm_inbox: Vec<ReceivedLukewarmContextInboxItem>,
    selected_received_lukewarm: Option<PathBuf>,
    received_transfer_inbox: Vec<ReceivedGenericTransferInboxItem>,
    selected_received_transfer: Option<PathBuf>,
    networking_bundle_label: String,
    networking_bundle_summary: String,
    received_bundle_inbox: Vec<ReceivedWorkflowBundleInboxItem>,
    selected_received_bundle: Option<PathBuf>,
}

#[derive(Debug, Clone)]
enum GenEvent {
    Token(String),
    Info(String),
    Done,
    Error(String),
}

impl ChattyCogApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals.window_rounding = 0.0.into();
        style.visuals.menu_rounding = 0.0.into();
        style.visuals.widgets.noninteractive.rounding = 0.0.into();
        style.visuals.widgets.inactive.rounding = 0.0.into();
        style.visuals.widgets.hovered.rounding = 0.0.into();
        style.visuals.widgets.active.rounding = 0.0.into();
        style.visuals.panel_fill = egui::Color32::from_gray(245);
        cc.egui_ctx.set_style(style);

        let prefs_path = preferences::default_prefs_path()
            .unwrap_or_else(|_| PathBuf::from("config/preferences.json"));
        let (prefs, prefs_status) = match preferences::load_prefs(&prefs_path) {
            Ok(prefs) => (prefs, String::new()),
            Err(_) => (
                AppPreferences::default(),
                "Failed to load preferences; using defaults.".to_string(),
            ),
        };
        let networking = NetworkController::new_with_identity(
            (!prefs.network_device_id.trim().is_empty()).then(|| prefs.network_device_id.clone()),
        );

        let startup_splash_texture = load_local_png_texture(
            &cc.egui_ctx,
            &resolve_local_asset_path(FMI_SPLASH_IMAGE_PATH),
            "fmi_splash_wordmark",
        );

        let mut app = Self {
            tab: Tab::Chat,
            prev_tab: Tab::Chat,
            startup_splash: StartupSplashState {
                started_at: Instant::now(),
                dismissed: false,
                texture: startup_splash_texture,
            },
            show_left_sidebar: true,
            gguf_path: None,
            models_dir: find_models_dir(),
            models_cache: Vec::new(),
            messages: vec![Message {
                role: Role::System,
                content: default_orchestrator_system_prompt(),
                thinking: None,
            }],
            composer: String::new(),
            composer_had_focus_last_frame: false,
            is_generating: false,
            gen_cancel: None,
            gen_rx: None,
            assistant_draft: String::new(),
            runtime_status: "Runtime: probing...".to_string(),
            model_runtime_issues: HashMap::new(),
            runtime_info_rx: start_runtime_info_probe(),
            logs_dir: find_default_logs_dir(),
            logs_selected: None,
            logs_view: String::new(),
            logs_query_semantic: String::new(),
            logs_results_semantic: Vec::new(),
            logs_query_keyword: String::new(),
            logs_results_keyword: Vec::new(),
            logs_filter_module: String::new(),
            logs_filter_tag: String::new(),
            logs_new_module: "general".to_string(),
            logs_new_event_type: "note".to_string(),
            logs_new_tags: String::new(),
            logs_new_summary: String::new(),
            logs_new_payload_json: String::new(),
            bookkeeper_model_path: pick_default_bookkeeper_model(&find_models_dir()),
            bookkeeper: None,
            bookkeeper_temp: 0.2,
            bookkeeper_top_p: 0.9,
            bookkeeper_top_k: 40,
            bookkeeper_max_tokens: 256,
            bookkeeper_restart_due: Some(Instant::now()),
            lukewarm_summary: read_lukewarm_from_logs_dir(find_default_logs_dir().as_deref())
                .unwrap_or_default(),
            lukewarm_poll_due: Some(Instant::now()),
            lukewarm_rx: None,
            scroll_to_bottom: true,
            ecg_window: EcgWindowState::new("ECG Window - System hardware activity"),
            hot_memory: Vec::new(),
            orch_temp: 0.7,
            orch_top_p: 0.9,
            orch_top_k: 40,
            orch_max_tokens: 1024,
            orch_freeze_pending: false,
            prefs_path,
            prefs,
            prefs_status,
            capsule_editor_name: String::new(),
            capsule_editor_text: String::new(),
            capsule_selected_name: None,
            sandbox_dir: find_or_create_sandbox_dir(),
            sandbox_selected: None,
            sandbox_editor_path: None,
            sandbox_last_working_path: None,
            sandbox_editor_text: String::new(),
            sandbox_status: String::new(),
            sandbox_last_tool_result: String::new(),
            chat_selected_file: None,
            sandbox_task_nudge: String::new(),
            sandbox_task_enabled: false,
            sandbox_task_intent: SandboxTaskIntent::Create,
            sandbox_task_path: "notes/".to_string(),
            modules_dir: find_modules_dir(),
            module_registry: ModuleRegistry::scan(find_modules_dir()),
            module_state_notes: HashMap::new(),
            module_ai: HashMap::new(),
            module_forms: HashMap::new(),
            module_workspaces: HashMap::new(),
            module_rundown_jobs: HashMap::new(),
            module_bridge_last_fingerprint: HashMap::new(),
            module_room_bridge_last_fingerprint: HashMap::new(),
            module_room_events_bridge_last_fingerprint: HashMap::new(),
            module_session_trackers: HashMap::new(),
            module_session_receipts: Vec::new(),
            module_hosts: HashMap::new(),
            module_host_targets: HashMap::new(),
            open_module_tabs: Vec::new(),
            close_pending_modules: HashSet::new(),
            pending_sandbox_actions: Vec::new(),
            sandbox_action_status: String::new(),
            networking,
            networking_device_name_input: String::new(),
            networking_status: String::new(),
            networking_filter: String::new(),
            networking_selected_devices: HashSet::new(),
            networking_help_mode: NetworkingQuickHelpMode::Everyday,
            networking_focus_section: None,
            networking_focus_pending: None,
            networking_focus_flash_until: None,
            networking_alias_edit_device: None,
            networking_alias_input: String::new(),
            networking_group_edit_device: None,
            networking_group_input: String::new(),
            networking_handoff_target: String::new(),
            networking_handoff_title: String::new(),
            networking_handoff_body: String::new(),
            networking_shared_chat_policy: SharedChatPolicy::default(),
            networking_recoverable_shared_chat_policy: None,
            networking_recoverable_module_session: None,
            networking_shared_chat_log: Vec::new(),
            networking_shared_chat_seen_messages: HashSet::new(),
            networking_shared_chat_input: String::new(),
            networking_shared_chat_mirror_main_chat: false,
            networking_shared_chat_presence_key: String::new(),
            networking_shared_chat_connected_peer_keys: HashSet::new(),
            networking_shared_chat_presence_next_sync_at: Some(Instant::now()),
            networking_seen_handoffs: HashSet::new(),
            networking_seen_artifacts: HashSet::new(),
            received_workflow_inbox: Vec::new(),
            selected_received_workflow: None,
            received_lukewarm_inbox: Vec::new(),
            selected_received_lukewarm: None,
            received_transfer_inbox: Vec::new(),
            selected_received_transfer: None,
            networking_bundle_label: "Current ChattyCog setup".to_string(),
            networking_bundle_summary: String::new(),
            received_bundle_inbox: Vec::new(),
            selected_received_bundle: None,
        };

        app.apply_prefs_to_runtime_settings();
        app.sync_capsule_selection_from_prefs();
        app.ensure_persisted_network_identity();

        if !app.prefs.network_device_name.trim().is_empty() {
            let saved_name = app.prefs.network_device_name.clone();
            app.networking.set_device_name(&saved_name);
        }
        app.networking
            .set_allow_unknown_devices(app.prefs.network_allow_unknown_devices);
        let blocked = app
            .prefs
            .network_blocked_devices
            .iter()
            .map(|peer| BlockedPeer {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                address: String::new(),
                last_seen_secs_ago: None,
            })
            .collect::<Vec<_>>();
        app.networking.replace_blocked_peers(&blocked);
        let trusted = app
            .prefs
            .network_trusted_devices
            .iter()
            .map(|peer| TrustedPeer {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                address: String::new(),
                last_seen_secs_ago: None,
            })
            .collect::<Vec<_>>();
        app.networking.replace_trusted_peers(&trusted);
        app.networking_device_name_input = app.networking.snapshot().device_name.clone();
        app.refresh_received_workflow_inbox();
        app.refresh_received_lukewarm_inbox();
        app.refresh_received_transfer_inbox();
        app.refresh_received_bundle_inbox();
        app.ensure_shared_chat_policy_defaults();
        app.load_recoverable_shared_chat_policy();
        app.load_recoverable_module_session_snapshot();
        app.ensure_default_sandbox_scratchpad();
        app.ensure_default_sandbox_task_ledger();

        app
    }

    fn show_startup_splash(&mut self, ctx: &egui::Context) -> bool {
        if self.startup_splash.dismissed {
            return false;
        }

        let elapsed = self.startup_splash.started_at.elapsed();
        let allow_click_dismiss = elapsed >= FMI_SPLASH_CLICK_DISMISS_DELAY;
        if elapsed >= FMI_SPLASH_DURATION
            || ctx.input(|input| {
                (allow_click_dismiss && input.pointer.any_click())
                    || input.key_pressed(egui::Key::Escape)
                    || input.key_pressed(egui::Key::Enter)
                    || input.key_pressed(egui::Key::Space)
            })
        {
            self.startup_splash.dismissed = true;
            return false;
        }

        ctx.request_repaint_after(Duration::from_millis(16));

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgb(10, 12, 14))
                    .inner_margin(egui::Margin::same(24.0)),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(28.0);
                    ui.label(
                        egui::RichText::new("Fractal Media Infrastructure")
                            .size(30.0)
                            .strong()
                            .color(egui::Color32::from_rgb(240, 240, 236)),
                    );
                    ui.add_space(10.0);

                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(18, 20, 22))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgb(68, 72, 78),
                        ))
                        .inner_margin(egui::Margin::same(18.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width().min(780.0));
                            if let Some(texture) = self.startup_splash.texture.as_ref() {
                                let size = texture.size_vec2();
                                let max_width = ui.available_width().min(720.0);
                                let scale = (max_width / size.x).min(1.0);
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(size * scale)
                                        .sense(egui::Sense::hover()),
                                );
                            } else {
                                ui.label(
                                    egui::RichText::new("FMI wordmark asset unavailable")
                                        .strong()
                                        .color(egui::Color32::from_rgb(220, 220, 220)),
                                );
                            }
                        });

                    ui.add_space(14.0);
                    ui.small(
                        "Independent R&D umbrella for open-source AI tooling, cognitive scaffolding experiments, and local-first research systems.",
                    );
                    ui.add_space(12.0);
                    ui.add(
                        egui::ProgressBar::new(
                            (elapsed.as_secs_f32() / FMI_SPLASH_DURATION.as_secs_f32())
                                .clamp(0.0, 1.0),
                        )
                        .desired_width(320.0),
                    );
                    ui.add_space(8.0);
                    ui.small("Press Space, Enter, Esc, or click to continue");
                });
            });

        true
    }

    fn sync_capsule_selection_from_prefs(&mut self) {
        let active_name = self
            .prefs
            .active_orchestrator_capsule
            .as_ref()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());
        self.capsule_selected_name = active_name.clone();

        if let Some(active_name) = active_name {
            if let Some(capsule) = self
                .prefs
                .orchestrator_capsules
                .iter()
                .find(|capsule| capsule.name == active_name)
            {
                self.capsule_editor_name = capsule.name.clone();
                self.capsule_editor_text = capsule.text.clone();
                return;
            }
        }

        if self.capsule_editor_name.trim().is_empty() && self.capsule_editor_text.trim().is_empty()
        {
            if let Some(first) = self.prefs.orchestrator_capsules.first() {
                self.capsule_editor_name = first.name.clone();
                self.capsule_editor_text = first.text.clone();
                self.capsule_selected_name = Some(first.name.clone());
            }
        }
    }

    fn active_orchestrator_capsule(&self) -> Option<&PromptCapsule> {
        let active_name = self.prefs.active_orchestrator_capsule.as_deref()?.trim();
        if active_name.is_empty() {
            return None;
        }
        self.prefs
            .orchestrator_capsules
            .iter()
            .find(|capsule| capsule.name == active_name)
    }

    fn ensure_persisted_network_identity(&mut self) {
        let current_device_id = self.networking.snapshot().device_id.trim().to_string();
        if current_device_id.is_empty() || self.prefs.network_device_id.trim() == current_device_id
        {
            return;
        }

        self.prefs.network_device_id = current_device_id;
        if let Err(err) = preferences::save_prefs(&self.prefs_path, &self.prefs) {
            if self.prefs_status.trim().is_empty() {
                self.prefs_status = format!("Could not save stable network device identity: {err}");
            }
        } else if self.prefs_status.trim().is_empty() {
            self.prefs_status = "Saved stable network device identity.".to_string();
        }
    }

    fn apply_prefs_to_runtime_settings(&mut self) {
        self.orch_temp = self.prefs.orchestrator.temp;
        self.orch_top_p = self.prefs.orchestrator.top_p;
        self.orch_top_k = self.prefs.orchestrator.top_k;
        self.orch_max_tokens = self.prefs.orchestrator.max_tokens;

        self.bookkeeper_temp = self.prefs.bookkeeper.temp;
        self.bookkeeper_top_p = self.prefs.bookkeeper.top_p;
        self.bookkeeper_top_k = self.prefs.bookkeeper.top_k;
        self.bookkeeper_max_tokens = self.prefs.bookkeeper.max_tokens;
    }

    fn apply_live_orchestrator_prefs(&mut self) {
        self.orch_temp = self.prefs.orchestrator.temp;
        self.orch_top_p = self.prefs.orchestrator.top_p;
        self.orch_top_k = self.prefs.orchestrator.top_k;
        self.orch_max_tokens = self.prefs.orchestrator.max_tokens;
    }

    fn set_active_chat_model_path(&mut self, path: Option<PathBuf>) {
        self.gguf_path = path.clone();
        if let Some(selected) = path {
            let label = selected
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| selected.display().to_string());
            self.runtime_status = match read_gguf_architecture(&selected) {
                Ok(Some(architecture)) => {
                    let readiness = if architecture_supports_mtmd(&selected, Some(&architecture)) {
                        if let Some(mmproj) =
                            find_matching_mmproj_path(&selected, Some(&architecture))
                        {
                            format!(
                                " | vision ready via {}",
                                mmproj.file_name().unwrap_or_default().to_string_lossy()
                            )
                        } else {
                            " | current local runtime needs a matching projector".to_string()
                        }
                    } else {
                        String::new()
                    };
                    format!("Runtime: selected model {label} [{architecture}]{readiness}")
                }
                Ok(None) => format!("Runtime: selected model {label}"),
                Err(err) => format!(
                    "Runtime: selected model {label} (could not read GGUF metadata: {err})"
                ),
            };
            if let Some(bk) = &self.bookkeeper {
                bk.append(MemoryEvent {
                    ts_unix_ms: now_unix_ms(),
                    kind: MemoryKind::Cold,
                    category: EventCategory::Module,
                    source: "ui".to_string(),
                    module: Some("models".to_string()),
                    event_type: Some("select".to_string()),
                    text: format!("Selected model: {}", selected.display()),
                    tags: Vec::new(),
                    entities: Vec::new(),
                    payload_json: None,
                });
            }
        } else {
            self.runtime_status = "Runtime: no GGUF selected".to_string();
        }
    }

    fn persist_network_prefs(&mut self) {
        self.ensure_persisted_network_identity();
        match preferences::save_prefs(&self.prefs_path, &self.prefs) {
            Ok(()) => {}
            Err(err) => self.prefs_status = format!("Save failed: {err}"),
        }
    }

    fn normalize_recoverable_shared_chat_policy(mut policy: SharedChatPolicy) -> SharedChatPolicy {
        policy.updated_at_unix_ms = 0;
        policy.host_activity_state.clear();
        policy.host_activity_label.clear();
        policy.host_activity_updated_at_unix_ms = 0;
        policy
    }

    fn load_recoverable_shared_chat_policy(&mut self) {
        self.networking_recoverable_shared_chat_policy = self
            .prefs
            .network_recoverable_shared_chat_policy_json
            .as_ref()
            .and_then(|text| serde_json::from_str::<SharedChatPolicy>(text).ok())
            .map(Self::normalize_recoverable_shared_chat_policy)
            .filter(|policy| policy.session_active && !policy.session_id.trim().is_empty());
    }

    fn load_recoverable_module_session_snapshot(&mut self) {
        let path = self.recoverable_module_session_path();
        let had_snapshot = path.is_file();
        let mut snapshot = if path.is_file() {
            std::fs::read(&path).ok().and_then(|bytes| {
                serde_json::from_slice::<RecoverableModuleSessionSnapshot>(&bytes).ok()
            })
        } else {
            None
        };

        if let Some(existing) = &mut snapshot {
            existing.normalize();
            let payload_dir = self.recoverable_module_session_payload_dir();
            if let Some(shared_state) = &existing.latest_shared_state {
                if !payload_dir
                    .join(&shared_state.cached_payload_name)
                    .is_file()
                {
                    existing.latest_shared_state = None;
                }
            }
            existing
                .recent_assets
                .retain(|asset| payload_dir.join(&asset.cached_payload_name).is_file());
        }

        let recoverable_policy = self.networking_recoverable_shared_chat_policy.as_ref();
        self.networking_recoverable_module_session = snapshot.filter(|item| {
            let Some(policy) = recoverable_policy else {
                return false;
            };
            item.session_id == policy.session_id
                && item.scope_module_id == policy.scope_module_id
                && !item.scope_module_id.trim().is_empty()
        });
        if had_snapshot && self.networking_recoverable_module_session.is_none() {
            self.discard_recoverable_module_session_snapshot();
        }
    }

    fn sync_recoverable_module_session_snapshot(&mut self) {
        let path = self.recoverable_module_session_path();
        let should_clear = matches!(
            (
                self.networking_recoverable_module_session.as_ref(),
                self.networking_recoverable_shared_chat_policy.as_ref()
            ),
            (Some(snapshot), Some(policy))
                if snapshot.session_id != policy.session_id
                    || snapshot.scope_module_id != policy.scope_module_id
        );
        if should_clear {
            self.networking_recoverable_module_session = None;
        }
        if let Some(snapshot) = &mut self.networking_recoverable_module_session {
            snapshot.normalize();
        }

        if let Some(snapshot) = &self.networking_recoverable_module_session {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(bytes) = serde_json::to_vec_pretty(snapshot) {
                if let Err(err) = std::fs::write(&path, bytes) {
                    if self.networking_status.trim().is_empty() {
                        self.networking_status = format!(
                            "Networking: could not save recoverable module session snapshot: {err}"
                        );
                    }
                }
            }
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn discard_recoverable_module_session_snapshot(&mut self) {
        self.networking_recoverable_module_session = None;
        let _ = std::fs::remove_file(self.recoverable_module_session_path());
        let _ = std::fs::remove_dir_all(self.recoverable_module_session_payload_dir());
    }

    fn active_recoverable_module_session_context(
        &self,
    ) -> Option<(String, String, String, String)> {
        let policy = &self.networking_shared_chat_policy;
        if !policy.session_active
            || policy.scope_kind != SharedChatScopeKind::Module
            || !self.shared_chat_is_local_host()
        {
            return None;
        }
        let module_id = policy.scope_module_id.trim().to_string();
        if module_id.is_empty() {
            return None;
        }
        Some((
            module_id,
            policy.scope_module_name.trim().to_string(),
            policy.session_id.trim().to_string(),
            policy.session_label.trim().to_string(),
        ))
    }

    fn ensure_recoverable_module_session_entry(
        &mut self,
    ) -> Option<(String, String, String, String)> {
        let context = self.active_recoverable_module_session_context()?;
        let (module_id, module_name, session_id, session_label) = context.clone();
        let needs_reset = self
            .networking_recoverable_module_session
            .as_ref()
            .is_none_or(|existing| {
                existing.session_id != session_id || existing.scope_module_id != module_id
            });
        if needs_reset {
            let _ = std::fs::remove_dir_all(self.recoverable_module_session_payload_dir());
            self.networking_recoverable_module_session = Some(RecoverableModuleSessionSnapshot {
                session_id: session_id.clone(),
                session_label: session_label.clone(),
                scope_module_id: module_id.clone(),
                scope_module_name: module_name.clone(),
                saved_at_unix_ms: now_unix_ms().max(0) as u64,
                latest_shared_state: None,
                recent_assets: Vec::new(),
            });
        } else if let Some(existing) = &mut self.networking_recoverable_module_session {
            existing.scope_module_name = module_name.clone();
            existing.session_label = session_label.clone();
            existing.saved_at_unix_ms = now_unix_ms().max(0) as u64;
        }
        Some(context)
    }

    fn remember_recoverable_module_shared_state(
        &mut self,
        module_id: &str,
        state: &ModuleBridgeSharedState,
        payload_text: &str,
    ) {
        let Some((scope_module_id, _, session_id, _)) =
            self.ensure_recoverable_module_session_entry()
        else {
            return;
        };
        if module_id.trim() != scope_module_id.trim()
            || state.session_id.trim() != session_id.trim()
        {
            return;
        }
        let payload_dir = self.recoverable_module_session_payload_dir();
        if std::fs::create_dir_all(&payload_dir).is_err() {
            return;
        }
        let cached_payload_name = format!(
            "{}__state_rev_{}.json",
            slugify_filename(module_id, "module"),
            state.session_revision.max(1)
        );
        let payload_path = payload_dir.join(&cached_payload_name);
        if std::fs::write(&payload_path, payload_text.as_bytes()).is_err() {
            return;
        }
        if let Some(snapshot) = &mut self.networking_recoverable_module_session {
            if let Some(previous) = &snapshot.latest_shared_state {
                if previous.cached_payload_name != cached_payload_name {
                    let _ = std::fs::remove_file(payload_dir.join(&previous.cached_payload_name));
                }
            }
            snapshot.latest_shared_state = Some(RecoverableModuleSharedStateSnapshot {
                summary: state.summary.clone(),
                session_revision: state.session_revision.max(1),
                cached_payload_name,
                updated_at_unix_ms: state.updated_at_unix_ms.max(now_unix_ms().max(0) as u64),
            });
            snapshot.saved_at_unix_ms = now_unix_ms().max(0) as u64;
        }
        self.sync_recoverable_module_session_snapshot();
    }

    #[allow(dead_code)]
    fn remember_recoverable_module_asset(
        &mut self,
        kind: &str,
        label: &str,
        module_id: &str,
        summary: &str,
        file_name: &str,
        content_type: &str,
        bytes: &[u8],
        binary: bool,
    ) {
        let Some((scope_module_id, _, _, _)) = self.ensure_recoverable_module_session_entry()
        else {
            return;
        };
        if module_id.trim().is_empty() || module_id.trim() != scope_module_id.trim() {
            return;
        }
        let payload_dir = self.recoverable_module_session_payload_dir();
        if std::fs::create_dir_all(&payload_dir).is_err() {
            return;
        }
        let stored_at = now_unix_ms().max(0) as u64;
        let cached_payload_name = format!(
            "{}__{}__{}.{}",
            slugify_filename(module_id, "module"),
            slugify_filename(
                if label.trim().is_empty() {
                    kind.trim()
                } else {
                    label.trim()
                },
                "asset"
            ),
            stored_at,
            infer_transfer_extension(file_name, content_type, binary)
        );
        let payload_path = payload_dir.join(&cached_payload_name);
        if std::fs::write(&payload_path, bytes).is_err() {
            return;
        }

        if let Some(snapshot) = &mut self.networking_recoverable_module_session {
            snapshot.recent_assets.insert(
                0,
                RecoverableModuleSessionAssetSnapshot {
                    artifact_kind: kind.trim().to_string(),
                    label: label.trim().to_string(),
                    summary: summary.trim().to_string(),
                    file_name: file_name.trim().to_string(),
                    content_type: content_type.trim().to_string(),
                    byte_len: bytes.len() as u64,
                    binary,
                    cached_payload_name,
                    stored_at_unix_ms: stored_at,
                },
            );
            while snapshot.recent_assets.len() > 12 {
                if let Some(removed) = snapshot.recent_assets.pop() {
                    let _ = std::fs::remove_file(payload_dir.join(removed.cached_payload_name));
                }
            }
            snapshot.saved_at_unix_ms = stored_at;
        }
        self.sync_recoverable_module_session_snapshot();
    }

    fn restore_recoverable_module_shared_state_to_bridge(&mut self) -> Result<(), String> {
        let Some(recovery) = self.networking_recoverable_module_session.clone() else {
            return Err("No recoverable module session state is cached yet.".to_string());
        };
        let Some(shared_state) = recovery.latest_shared_state else {
            return Err(
                "No cached shared_state.json is available for this recovered session.".to_string(),
            );
        };
        let module = self
            .module_registry
            .modules
            .iter()
            .find(|module| module.module_id == recovery.scope_module_id)
            .ok_or_else(|| "The recovered module is not currently available.".to_string())?;
        let payload_path = self
            .recoverable_module_session_payload_dir()
            .join(&shared_state.cached_payload_name);
        let bytes = std::fs::read(&payload_path).map_err(|err| {
            format!(
                "Could not read the cached shared state from {}: {err}",
                payload_path.display()
            )
        })?;
        let state: ModuleBridgeSharedState = serde_json::from_slice(&bytes)
            .map_err(|err| format!("Shared-state parse error: {err}"))?;
        write_bridge_shared_state(&module.dir, &state).map_err(|err| {
            format!(
                "Could not restore shared_state.json for {}: {err}",
                recovery.scope_module_name
            )
        })?;
        self.networking_status = format!(
            "Networking: restored the latest shared_state.json for {}.",
            if recovery.scope_module_name.trim().is_empty() {
                recovery.scope_module_id
            } else {
                recovery.scope_module_name
            }
        );
        Ok(())
    }

    fn recovery_target_connection_ids(&self) -> Vec<String> {
        let snapshot = self.networking.snapshot().clone();
        let mut selected = snapshot
            .connected_peers
            .iter()
            .filter(|peer| {
                let key = if peer.device_id.trim().is_empty() {
                    peer.connection_id.clone()
                } else {
                    peer.device_id.clone()
                };
                self.networking_selected_devices.contains(&key)
            })
            .map(|peer| peer.connection_id.clone())
            .collect::<Vec<_>>();
        selected.sort();
        selected.dedup();
        if !selected.is_empty() {
            return selected;
        }
        self.shared_chat_connected_connection_ids()
    }

    fn replay_recoverable_module_shared_state(&mut self) -> Result<usize, String> {
        let Some(recovery) = self.networking_recoverable_module_session.clone() else {
            return Err("No recoverable module session state is cached yet.".to_string());
        };
        let Some(shared_state) = recovery.latest_shared_state else {
            return Err(
                "No cached shared_state.json is available for this session yet.".to_string(),
            );
        };
        let connection_ids = self.recovery_target_connection_ids();
        if connection_ids.is_empty() {
            return Err("Connect to one or more room peers first.".to_string());
        }
        let payload_path = self
            .recoverable_module_session_payload_dir()
            .join(&shared_state.cached_payload_name);
        let text = std::fs::read_to_string(&payload_path).map_err(|err| {
            format!(
                "Could not read the cached shared state from {}: {err}",
                payload_path.display()
            )
        })?;
        for connection_id in &connection_ids {
            self.networking.send_artifact(
                connection_id,
                "module_shared_state_json",
                if recovery.session_label.trim().is_empty() {
                    "Recovered module session state"
                } else {
                    recovery.session_label.trim()
                },
                Some(&recovery.scope_module_id),
                if shared_state.summary.trim().is_empty() {
                    "Recovered module session state"
                } else {
                    shared_state.summary.trim()
                },
                &format!(
                    "{}_shared_state_recovered.json",
                    slugify_filename(&recovery.scope_module_id, "module")
                ),
                &text,
            );
        }
        self.networking_status = format!(
            "Networking: re-shared the latest module session state to {} peer(s).",
            connection_ids.len()
        );
        Ok(connection_ids.len())
    }

    fn replay_recoverable_module_assets(&mut self) -> Result<(usize, usize), String> {
        let Some(recovery) = self.networking_recoverable_module_session.clone() else {
            return Err("No recoverable module session assets are cached yet.".to_string());
        };
        if recovery.recent_assets.is_empty() {
            return Err("No recoverable module session assets are cached yet.".to_string());
        }
        let connection_ids = self.recovery_target_connection_ids();
        if connection_ids.is_empty() {
            return Err("Connect to one or more room peers first.".to_string());
        }
        let payload_dir = self.recoverable_module_session_payload_dir();
        let mut replayed = 0usize;
        for asset in &recovery.recent_assets {
            let payload_path = payload_dir.join(&asset.cached_payload_name);
            let Ok(bytes) = std::fs::read(&payload_path) else {
                continue;
            };
            for connection_id in &connection_ids {
                if asset.binary {
                    self.networking.send_artifact_bytes(
                        connection_id,
                        &asset.artifact_kind,
                        if asset.label.trim().is_empty() {
                            &asset.artifact_kind
                        } else {
                            &asset.label
                        },
                        Some(&recovery.scope_module_id),
                        &asset.summary,
                        if asset.file_name.trim().is_empty() {
                            &asset.cached_payload_name
                        } else {
                            &asset.file_name
                        },
                        &asset.content_type,
                        &bytes,
                    );
                } else if let Ok(text) = String::from_utf8(bytes.clone()) {
                    self.networking.send_artifact(
                        connection_id,
                        &asset.artifact_kind,
                        if asset.label.trim().is_empty() {
                            &asset.artifact_kind
                        } else {
                            &asset.label
                        },
                        Some(&recovery.scope_module_id),
                        &asset.summary,
                        if asset.file_name.trim().is_empty() {
                            &asset.cached_payload_name
                        } else {
                            &asset.file_name
                        },
                        &text,
                    );
                } else {
                    self.networking.send_artifact_bytes(
                        connection_id,
                        &asset.artifact_kind,
                        if asset.label.trim().is_empty() {
                            &asset.artifact_kind
                        } else {
                            &asset.label
                        },
                        Some(&recovery.scope_module_id),
                        &asset.summary,
                        if asset.file_name.trim().is_empty() {
                            &asset.cached_payload_name
                        } else {
                            &asset.file_name
                        },
                        &asset.content_type,
                        &bytes,
                    );
                }
            }
            replayed += 1;
        }
        self.networking_status = format!(
            "Networking: replayed {} recoverable module asset(s) to {} peer(s).",
            replayed,
            connection_ids.len()
        );
        Ok((replayed, connection_ids.len()))
    }

    fn sync_recoverable_shared_chat_policy_snapshot(&mut self) {
        let next_policy = if self.networking_shared_chat_policy.session_active
            && self.shared_chat_is_local_host()
        {
            Some(Self::normalize_recoverable_shared_chat_policy(
                self.networking_shared_chat_policy.clone(),
            ))
        } else {
            None
        };
        let next_json = next_policy
            .as_ref()
            .and_then(|policy| serde_json::to_string(policy).ok());
        let changed = self.prefs.network_recoverable_shared_chat_policy_json != next_json;
        self.networking_recoverable_shared_chat_policy = next_policy;
        if self.networking_recoverable_shared_chat_policy.is_none() {
            self.discard_recoverable_module_session_snapshot();
        } else {
            self.sync_recoverable_module_session_snapshot();
        }
        if !changed {
            return;
        }
        self.prefs.network_recoverable_shared_chat_policy_json = next_json;
        if let Err(err) = preferences::save_prefs(&self.prefs_path, &self.prefs) {
            if self.prefs_status.trim().is_empty() {
                self.prefs_status =
                    format!("Could not save recoverable shared-room session: {err}");
            }
        }
    }

    fn discard_recoverable_shared_chat_policy(&mut self) {
        self.networking_recoverable_shared_chat_policy = None;
        self.discard_recoverable_module_session_snapshot();
        if self
            .prefs
            .network_recoverable_shared_chat_policy_json
            .is_none()
        {
            return;
        }
        self.prefs.network_recoverable_shared_chat_policy_json = None;
        if let Err(err) = preferences::save_prefs(&self.prefs_path, &self.prefs) {
            self.prefs_status = format!("Could not discard recoverable shared-room session: {err}");
        }
    }

    fn resume_recoverable_shared_chat_policy(&mut self) -> Result<(), String> {
        let Some(mut policy) = self.networking_recoverable_shared_chat_policy.clone() else {
            return Err("No recoverable shared-room session is saved yet.".to_string());
        };
        let snapshot = self.networking.snapshot().clone();
        if snapshot.device_id.trim().is_empty() {
            return Err("Local network identity is not ready yet.".to_string());
        }
        policy.updated_at_unix_ms = now_unix_ms().max(0) as u64;
        policy.source_app = "chattycog".to_string();
        policy.host_device_id = snapshot.device_id.clone();
        policy.host_device_name = snapshot.device_name.clone();
        if policy.turn_mode == SharedChatTurnMode::Open {
            policy.turn_holder_device_id.clear();
            policy.turn_holder_device_name.clear();
        }
        self.networking_shared_chat_policy = policy;
        self.ensure_shared_chat_policy_defaults();
        self.networking_shared_chat_presence_key.clear();
        self.networking_shared_chat_presence_next_sync_at = Some(Instant::now());
        self.broadcast_shared_chat_policy_with_options(
            "Recovered the last saved host session.",
            false,
            true,
            false,
        );
        let _ = self.restore_recoverable_module_shared_state_to_bridge();
        Ok(())
    }

    fn shared_chat_host_appears_offline(&self) -> bool {
        let host_id = self.networking_shared_chat_policy.host_device_id.trim();
        if host_id.is_empty() || self.shared_chat_is_local_host() {
            return false;
        }
        !self
            .networking
            .snapshot()
            .connected_peers
            .iter()
            .any(|peer| peer.device_id.trim() == host_id)
    }

    fn take_over_shared_chat_host(&mut self) -> Result<(), String> {
        let snapshot = self.networking.snapshot().clone();
        if snapshot.device_id.trim().is_empty() {
            return Err("Local network identity is not ready yet.".to_string());
        }
        let previous_host_id = self.networking_shared_chat_policy.host_device_id.clone();
        self.networking_shared_chat_policy.host_device_id = snapshot.device_id.clone();
        self.networking_shared_chat_policy.host_device_name = snapshot.device_name.clone();
        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::TalkingStick {
            let holder = self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .trim()
                .to_string();
            if holder.is_empty() || holder == previous_host_id {
                self.networking_shared_chat_policy.turn_holder_device_id = snapshot.device_id;
                self.networking_shared_chat_policy.turn_holder_device_name = snapshot.device_name;
            }
        }
        self.networking_shared_chat_presence_key.clear();
        self.networking_shared_chat_presence_next_sync_at = Some(Instant::now());
        self.broadcast_shared_chat_policy_with_options(
            "Local peer took over as room host.",
            false,
            true,
            false,
        );
        Ok(())
    }

    fn handoff_shared_chat_host_to_peer(
        &mut self,
        target_device_id: &str,
        target_device_name: &str,
    ) -> Result<(), String> {
        if !self.shared_chat_is_local_host() {
            return Err("Only the current host can hand off this session.".to_string());
        }
        let target_device_id = target_device_id.trim();
        if target_device_id.is_empty() {
            return Err("Pick a connected peer to hand the host role to.".to_string());
        }
        let snapshot = self.networking.snapshot().clone();
        if target_device_id == snapshot.device_id.trim() {
            return Err("That peer is already the local host.".to_string());
        }
        self.networking_shared_chat_policy.host_device_id = target_device_id.to_string();
        self.networking_shared_chat_policy.host_device_name = target_device_name.trim().to_string();
        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::TalkingStick {
            let local_id = snapshot.device_id.trim();
            let holder = self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .trim()
                .to_string();
            if holder.is_empty() || holder == local_id {
                self.networking_shared_chat_policy.turn_holder_device_id =
                    target_device_id.to_string();
                self.networking_shared_chat_policy.turn_holder_device_name =
                    target_device_name.trim().to_string();
            }
        }
        self.networking_shared_chat_presence_key.clear();
        self.broadcast_shared_chat_policy_with_options(
            &format!(
                "Host role handed to {}.",
                if target_device_name.trim().is_empty() {
                    target_device_id
                } else {
                    target_device_name.trim()
                }
            ),
            false,
            true,
            true,
        );
        Ok(())
    }

    fn network_trust_exports_dir(&self) -> PathBuf {
        self.prefs_path
            .parent()
            .and_then(|config_dir| config_dir.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join("network_exports")
            .join("trusted_peers")
    }

    fn export_trusted_peer_list(&mut self) {
        if self.prefs.network_trusted_devices.is_empty() {
            self.networking_status =
                "Networking: trust one or more peers before exporting a trust list.".to_string();
            return;
        }

        let export_dir = self.network_trust_exports_dir();
        if let Err(err) = std::fs::create_dir_all(&export_dir) {
            self.networking_status = format!(
                "Networking: could not prepare the trust-list export folder: {}",
                err
            );
            return;
        }

        let snapshot = self.networking.snapshot().clone();
        let export = NetworkPeerExchangeFile {
            version: "1".to_string(),
            source_app: "ChattyCog".to_string(),
            source_device_id: snapshot.device_id,
            source_device_name: snapshot.device_name,
            exported_at_unix_ms: now_unix_ms().max(0) as u64,
            peers: self
                .prefs
                .network_trusted_devices
                .iter()
                .map(|peer| NetworkPeerExchangeRecord {
                    device_id: peer.device_id.clone(),
                    device_name: peer.device_name.clone(),
                    alias: self
                        .prefs
                        .network_device_aliases
                        .get(&peer.device_id)
                        .cloned()
                        .unwrap_or_default(),
                    group: self
                        .prefs
                        .network_device_groups
                        .get(&peer.device_id)
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect(),
        };

        let default_name = format!(
            "chattycog_trusted_peers_{}.json",
            export.exported_at_unix_ms
        );

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_directory(&export_dir)
            .set_file_name(&default_name)
            .save_file()
        {
            match serde_json::to_string_pretty(&export) {
                Ok(text) => match std::fs::write(&path, text) {
                    Ok(()) => {
                        self.networking_status = format!(
                            "Networking: exported {} trusted peer(s) to {}.",
                            export.peers.len(),
                            path.display()
                        );
                    }
                    Err(err) => {
                        self.networking_status =
                            format!("Networking: could not write the trust list: {}", err);
                    }
                },
                Err(err) => {
                    self.networking_status =
                        format!("Networking: could not serialize the trust list: {}", err);
                }
            }
        }
    }

    fn import_trusted_peer_list(&mut self) {
        let import_dir = self.network_trust_exports_dir();
        if let Err(err) = std::fs::create_dir_all(&import_dir) {
            self.networking_status = format!(
                "Networking: could not prepare the trust-list import folder: {}",
                err
            );
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_directory(&import_dir)
            .pick_file()
        else {
            return;
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.networking_status = format!(
                    "Networking: could not read the trust list from {}: {}",
                    path.display(),
                    err
                );
                return;
            }
        };

        let imported: NetworkPeerExchangeFile = match serde_json::from_str(&text) {
            Ok(imported) => imported,
            Err(err) => {
                self.networking_status = format!(
                    "Networking: could not parse the trust list from {}: {}",
                    path.display(),
                    err
                );
                return;
            }
        };

        let local_device_id = self.networking.snapshot().device_id.clone();
        let blocked_ids: HashSet<String> = self
            .prefs
            .network_blocked_devices
            .iter()
            .map(|peer| peer.device_id.clone())
            .collect();
        let mut added = 0usize;
        let mut refreshed = 0usize;
        let mut alias_added = 0usize;
        let mut group_added = 0usize;
        let mut skipped_self = 0usize;
        let mut skipped_blocked = 0usize;
        let mut skipped_empty = 0usize;

        for peer in imported.peers {
            let device_id = peer.device_id.trim().to_string();
            if device_id.is_empty() {
                skipped_empty += 1;
                continue;
            }
            if !local_device_id.trim().is_empty() && device_id == local_device_id {
                skipped_self += 1;
                continue;
            }
            if blocked_ids.contains(&device_id) {
                skipped_blocked += 1;
                continue;
            }

            let imported_name = if !peer.device_name.trim().is_empty() {
                peer.device_name.trim().to_string()
            } else if !peer.alias.trim().is_empty() {
                peer.alias.trim().to_string()
            } else {
                device_id.clone()
            };

            if let Some(existing) = self
                .prefs
                .network_trusted_devices
                .iter_mut()
                .find(|entry| entry.device_id == device_id)
            {
                if existing.device_name.trim().is_empty() && !imported_name.trim().is_empty() {
                    existing.device_name = imported_name.clone();
                    refreshed += 1;
                }
            } else {
                self.prefs
                    .network_trusted_devices
                    .push(preferences::StoredNetworkPeer {
                        device_id: device_id.clone(),
                        device_name: imported_name.clone(),
                    });
                added += 1;
            }

            if !peer.alias.trim().is_empty()
                && self
                    .prefs
                    .network_device_aliases
                    .get(&device_id)
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
            {
                self.prefs
                    .network_device_aliases
                    .insert(device_id.clone(), peer.alias.trim().to_string());
                alias_added += 1;
            }

            if !peer.group.trim().is_empty()
                && self
                    .prefs
                    .network_device_groups
                    .get(&device_id)
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
            {
                self.prefs
                    .network_device_groups
                    .insert(device_id.clone(), peer.group.trim().to_string());
                group_added += 1;
            }
        }

        let trusted_peers: Vec<TrustedPeer> = self
            .prefs
            .network_trusted_devices
            .iter()
            .map(|peer| TrustedPeer {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                ..Default::default()
            })
            .collect();
        self.networking.replace_trusted_peers(&trusted_peers);
        self.persist_network_prefs();

        let imported_count = added + refreshed;
        self.networking_status = format!(
            "Networking: imported trust list from {}. Added {}, refreshed {}, aliases {}, groups {}, skipped self {}, blocked {}, empty {}.",
            path.display(),
            added,
            refreshed,
            alias_added,
            group_added,
            skipped_self,
            skipped_blocked,
            skipped_empty
        );
        if imported_count == 0 && alias_added == 0 && group_added == 0 {
            self.networking_status.push_str(" Nothing new was applied.");
        }
    }

    fn export_blocked_peer_list(&mut self) {
        if self.prefs.network_blocked_devices.is_empty() {
            self.networking_status =
                "Networking: block one or more peers before exporting a blocked list.".to_string();
            return;
        }

        let export_dir = self.network_trust_exports_dir();
        if let Err(err) = std::fs::create_dir_all(&export_dir) {
            self.networking_status = format!(
                "Networking: could not prepare the blocked-list export folder: {}",
                err
            );
            return;
        }

        let snapshot = self.networking.snapshot().clone();
        let export = NetworkPeerExchangeFile {
            version: "1".to_string(),
            source_app: "ChattyCog".to_string(),
            source_device_id: snapshot.device_id,
            source_device_name: snapshot.device_name,
            exported_at_unix_ms: now_unix_ms().max(0) as u64,
            peers: self
                .prefs
                .network_blocked_devices
                .iter()
                .map(|peer| NetworkPeerExchangeRecord {
                    device_id: peer.device_id.clone(),
                    device_name: peer.device_name.clone(),
                    alias: self
                        .prefs
                        .network_device_aliases
                        .get(&peer.device_id)
                        .cloned()
                        .unwrap_or_default(),
                    group: self
                        .prefs
                        .network_device_groups
                        .get(&peer.device_id)
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect(),
        };

        let default_name = format!(
            "chattycog_blocked_peers_{}.json",
            export.exported_at_unix_ms
        );

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_directory(&export_dir)
            .set_file_name(&default_name)
            .save_file()
        {
            match serde_json::to_string_pretty(&export) {
                Ok(text) => match std::fs::write(&path, text) {
                    Ok(()) => {
                        self.networking_status = format!(
                            "Networking: exported {} blocked peer(s) to {}.",
                            export.peers.len(),
                            path.display()
                        );
                    }
                    Err(err) => {
                        self.networking_status =
                            format!("Networking: could not write the blocked list: {}", err);
                    }
                },
                Err(err) => {
                    self.networking_status =
                        format!("Networking: could not serialize the blocked list: {}", err);
                }
            }
        }
    }

    fn import_blocked_peer_list(&mut self) {
        let import_dir = self.network_trust_exports_dir();
        if let Err(err) = std::fs::create_dir_all(&import_dir) {
            self.networking_status = format!(
                "Networking: could not prepare the blocked-list import folder: {}",
                err
            );
            return;
        }

        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_directory(&import_dir)
            .pick_file()
        else {
            return;
        };

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.networking_status = format!(
                    "Networking: could not read the blocked list from {}: {}",
                    path.display(),
                    err
                );
                return;
            }
        };

        let imported: NetworkPeerExchangeFile = match serde_json::from_str(&text) {
            Ok(imported) => imported,
            Err(err) => {
                self.networking_status = format!(
                    "Networking: could not parse the blocked list from {}: {}",
                    path.display(),
                    err
                );
                return;
            }
        };

        let local_device_id = self.networking.snapshot().device_id.clone();
        let mut added = 0usize;
        let mut refreshed = 0usize;
        let mut alias_added = 0usize;
        let mut group_added = 0usize;
        let mut skipped_self = 0usize;
        let mut skipped_empty = 0usize;
        let mut trust_removed = 0usize;

        for peer in imported.peers {
            let device_id = peer.device_id.trim().to_string();
            if device_id.is_empty() {
                skipped_empty += 1;
                continue;
            }
            if !local_device_id.trim().is_empty() && device_id == local_device_id {
                skipped_self += 1;
                continue;
            }

            let imported_name = if !peer.device_name.trim().is_empty() {
                peer.device_name.trim().to_string()
            } else if !peer.alias.trim().is_empty() {
                peer.alias.trim().to_string()
            } else {
                device_id.clone()
            };

            let trusted_before = self.prefs.network_trusted_devices.len();
            self.prefs
                .network_trusted_devices
                .retain(|entry| entry.device_id != device_id);
            trust_removed +=
                trusted_before.saturating_sub(self.prefs.network_trusted_devices.len());

            if let Some(existing) = self
                .prefs
                .network_blocked_devices
                .iter_mut()
                .find(|entry| entry.device_id == device_id)
            {
                if existing.device_name.trim().is_empty() && !imported_name.trim().is_empty() {
                    existing.device_name = imported_name.clone();
                    refreshed += 1;
                }
            } else {
                self.prefs
                    .network_blocked_devices
                    .push(preferences::StoredNetworkPeer {
                        device_id: device_id.clone(),
                        device_name: imported_name.clone(),
                    });
                added += 1;
            }

            if !peer.alias.trim().is_empty()
                && self
                    .prefs
                    .network_device_aliases
                    .get(&device_id)
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
            {
                self.prefs
                    .network_device_aliases
                    .insert(device_id.clone(), peer.alias.trim().to_string());
                alias_added += 1;
            }

            if !peer.group.trim().is_empty()
                && self
                    .prefs
                    .network_device_groups
                    .get(&device_id)
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
            {
                self.prefs
                    .network_device_groups
                    .insert(device_id.clone(), peer.group.trim().to_string());
                group_added += 1;
            }
        }

        let trusted_peers: Vec<TrustedPeer> = self
            .prefs
            .network_trusted_devices
            .iter()
            .map(|peer| TrustedPeer {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                ..Default::default()
            })
            .collect();
        let blocked_peers: Vec<BlockedPeer> = self
            .prefs
            .network_blocked_devices
            .iter()
            .map(|peer| BlockedPeer {
                device_id: peer.device_id.clone(),
                device_name: peer.device_name.clone(),
                ..Default::default()
            })
            .collect();
        self.networking.replace_trusted_peers(&trusted_peers);
        self.networking.replace_blocked_peers(&blocked_peers);
        self.persist_network_prefs();

        let imported_count = added + refreshed;
        self.networking_status = format!(
            "Networking: imported blocked list from {}. Added {}, refreshed {}, trust removed {}, aliases {}, groups {}, skipped self {}, empty {}.",
            path.display(),
            added,
            refreshed,
            trust_removed,
            alias_added,
            group_added,
            skipped_self,
            skipped_empty
        );
        if imported_count == 0 && alias_added == 0 && group_added == 0 && trust_removed == 0 {
            self.networking_status.push_str(" Nothing new was applied.");
        }
    }

    fn network_display_name(&self, device_id: &str, fallback: &str) -> String {
        self.prefs
            .network_device_aliases
            .get(device_id)
            .filter(|alias| !alias.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }

    fn network_group_label(&self, device_id: &str) -> Option<String> {
        self.prefs
            .network_device_groups
            .get(device_id)
            .map(|group| group.trim().to_string())
            .filter(|group| !group.is_empty())
    }

    fn network_is_trusted(&self, device_id: &str) -> bool {
        !device_id.trim().is_empty()
            && self
                .prefs
                .network_trusted_devices
                .iter()
                .any(|peer| peer.device_id == device_id)
    }

    fn trust_network_peer(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            self.networking_status =
                "This device has not shared a stable ID yet, so it cannot be trusted.".to_string();
            return;
        }
        let display_name = self.network_display_name(device_id, fallback);
        self.prefs
            .network_trusted_devices
            .retain(|entry| entry.device_id != device_id);
        self.prefs
            .network_blocked_devices
            .retain(|entry| entry.device_id != device_id);
        self.prefs
            .network_trusted_devices
            .push(preferences::StoredNetworkPeer {
                device_id: device_id.to_string(),
                device_name: display_name.clone(),
            });
        self.networking.trust_peer(device_id, &display_name);
        self.persist_network_prefs();
        self.networking_status = format!(
            "Trusted {}. Future connections will be approved automatically.",
            display_name
        );
    }

    fn untrust_network_peer(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            return;
        }
        self.prefs
            .network_trusted_devices
            .retain(|entry| entry.device_id != device_id);
        self.networking.untrust_peer(device_id);
        self.persist_network_prefs();
        self.networking_status = format!(
            "Removed {} from trusted devices.",
            self.network_display_name(device_id, fallback)
        );
    }

    fn block_network_peer(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            return;
        }
        let display_name = self.network_display_name(device_id, fallback);
        self.networking.block_peer(device_id, &display_name);
        self.prefs
            .network_trusted_devices
            .retain(|entry| entry.device_id != device_id);
        self.prefs
            .network_blocked_devices
            .retain(|entry| entry.device_id != device_id);
        self.prefs
            .network_blocked_devices
            .push(preferences::StoredNetworkPeer {
                device_id: device_id.to_string(),
                device_name: display_name.clone(),
            });
        self.persist_network_prefs();
        self.networking_status = format!("{display_name} is now blocked.");
    }

    fn unblock_network_peer(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            return;
        }
        let display_name = self.network_display_name(device_id, fallback);
        self.networking.unblock_peer(device_id);
        self.prefs
            .network_blocked_devices
            .retain(|entry| entry.device_id != device_id);
        self.persist_network_prefs();
        self.networking_status = format!("Unblocked {}.", display_name);
    }

    fn begin_network_alias_edit(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            self.networking_status =
                "This device has not shared a stable ID yet, so it cannot be renamed.".to_string();
            return;
        }
        self.networking_alias_edit_device = Some(device_id.to_string());
        self.networking_alias_input = self.network_display_name(device_id, fallback);
    }

    fn cancel_network_alias_edit(&mut self) {
        self.networking_alias_edit_device = None;
        self.networking_alias_input.clear();
    }

    fn save_network_alias_edit(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            self.cancel_network_alias_edit();
            return;
        }
        let trimmed = self.networking_alias_input.trim().to_string();
        if trimmed.is_empty() || trimmed == fallback.trim() {
            self.prefs.network_device_aliases.remove(device_id);
            self.networking_status = format!("Cleared the custom name for {}.", fallback.trim());
        } else {
            self.prefs
                .network_device_aliases
                .insert(device_id.to_string(), trimmed.clone());
            self.networking_status = format!("Saved \"{trimmed}\" for {}.", fallback.trim());
        }
        self.persist_network_prefs();
        self.cancel_network_alias_edit();
    }

    fn begin_network_group_edit(&mut self, device_id: &str) {
        if device_id.trim().is_empty() {
            self.networking_status =
                "This device has not shared a stable ID yet, so a group cannot be saved."
                    .to_string();
            return;
        }
        self.networking_group_edit_device = Some(device_id.to_string());
        self.networking_group_input = self.network_group_label(device_id).unwrap_or_default();
    }

    fn cancel_network_group_edit(&mut self) {
        self.networking_group_edit_device = None;
        self.networking_group_input.clear();
    }

    fn save_network_group_edit(&mut self, device_id: &str, fallback: &str) {
        if device_id.trim().is_empty() {
            self.cancel_network_group_edit();
            return;
        }
        let trimmed = self.networking_group_input.trim().to_string();
        if trimmed.is_empty() {
            self.prefs.network_device_groups.remove(device_id);
            self.networking_status = format!("Cleared the group label for {}.", fallback.trim());
        } else {
            self.prefs
                .network_device_groups
                .insert(device_id.to_string(), trimmed.clone());
            self.networking_status = format!("Saved group \"{trimmed}\" for {}.", fallback.trim());
        }
        self.persist_network_prefs();
        self.cancel_network_group_edit();
    }

    fn focus_networking_section(&mut self, section: NetworkingFocusSection) {
        self.networking_focus_section = Some(section);
        self.networking_focus_pending = Some(section);
        self.networking_focus_flash_until = Some(Instant::now() + Duration::from_secs(6));
    }

    fn set_module_host_target(&mut self, module_id: &str, rect: egui::Rect, pixels_per_point: f32) {
        let scale = pixels_per_point.max(1.0);
        self.module_host_targets.insert(
            module_id.to_string(),
            HostRect {
                x: (rect.min.x * scale).round() as i32,
                y: (rect.min.y * scale).round() as i32,
                width: (rect.width() * scale).round() as i32,
                height: (rect.height() * scale).round() as i32,
            },
        );
    }

    fn sync_module_hosts(&mut self) -> bool {
        if self.module_hosts.is_empty() {
            return false;
        }

        let mut needs_repaint = false;
        let mut close_ready = Vec::new();
        let host_ids = self.module_hosts.keys().cloned().collect::<Vec<_>>();

        for module_id in host_ids {
            let module_meta = self
                .module_registry
                .modules
                .iter()
                .find(|module| module.module_id == module_id)
                .and_then(|module| {
                    module
                        .visual_load
                        .clone()
                        .map(|visual| (module.dir.clone(), visual))
                });

            let Some((module_dir, visual)) = module_meta else {
                if let Some(host) = self.module_hosts.get_mut(&module_id) {
                    host.force_stop();
                }
                close_ready.push(module_id);
                continue;
            };

            let target = self.module_host_targets.get(&module_id).copied();
            if let Some(host) = self.module_hosts.get_mut(&module_id) {
                if host.sync(&module_dir, &visual, target) {
                    needs_repaint = true;
                }
                if self.close_pending_modules.contains(&module_id) && host.ready_to_finish_close() {
                    close_ready.push(module_id.clone());
                }
            }
        }

        for module_id in close_ready {
            close_module_tab_force(self, &module_id);
        }

        needs_repaint
    }

    fn drain_module_rundown_jobs(&mut self) {
        if self.module_rundown_jobs.is_empty() {
            return;
        }

        let mut finished: Vec<(String, String, bool)> = Vec::new();
        for (module_id, job) in &self.module_rundown_jobs {
            if let Ok(summary) = job.rx.try_recv() {
                finished.push((module_id.clone(), summary, job.overwrite_existing));
            }
        }

        for (module_id, summary, overwrite) in finished {
            self.module_rundown_jobs.remove(&module_id);
            let entry = self
                .module_state_notes
                .entry(module_id)
                .or_insert_with(String::new);
            if overwrite || entry.trim().is_empty() {
                *entry = summary;
            }
        }
    }

    fn selected_network_connection_ids(&self) -> Vec<String> {
        self.networking
            .snapshot()
            .connected_peers
            .iter()
            .filter_map(|peer| {
                let key = if peer.device_id.trim().is_empty() {
                    peer.connection_id.clone()
                } else {
                    peer.device_id.clone()
                };
                self.networking_selected_devices
                    .contains(&key)
                    .then_some(peer.connection_id.clone())
            })
            .collect()
    }

    fn shared_chat_connected_connection_ids(&self) -> Vec<String> {
        self.networking
            .snapshot()
            .connected_peers
            .iter()
            .map(|peer| peer.connection_id.clone())
            .collect()
    }

    fn shared_chat_connected_peer_keys(&self) -> HashSet<String> {
        self.networking
            .snapshot()
            .connected_peers
            .iter()
            .map(|peer| {
                if peer.device_id.trim().is_empty() {
                    peer.connection_id.clone()
                } else {
                    peer.device_id.clone()
                }
            })
            .collect()
    }

    fn ensure_shared_chat_policy_defaults(&mut self) {
        let snapshot = self.networking.snapshot().clone();
        if self.networking_shared_chat_policy.version.trim().is_empty() {
            self.networking_shared_chat_policy.version = "1".to_string();
        }
        if self.networking_shared_chat_policy.label.trim().is_empty() {
            self.networking_shared_chat_policy.label = "Shared room".to_string();
        }
        if self
            .networking_shared_chat_policy
            .source_app
            .trim()
            .is_empty()
        {
            self.networking_shared_chat_policy.source_app = "chattycog".to_string();
        }
        if self
            .networking_shared_chat_policy
            .host_device_id
            .trim()
            .is_empty()
        {
            self.networking_shared_chat_policy.host_device_id = snapshot.device_id.clone();
        }
        if self
            .networking_shared_chat_policy
            .host_device_name
            .trim()
            .is_empty()
        {
            self.networking_shared_chat_policy.host_device_name = snapshot.device_name.clone();
        }
        if self.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module
            && self
                .networking_shared_chat_policy
                .scope_module_id
                .trim()
                .is_empty()
        {
            self.networking_shared_chat_policy.scope_kind = SharedChatScopeKind::General;
            self.networking_shared_chat_policy.scope_module_name.clear();
            self.networking_shared_chat_policy.scope_multiplayer = false;
        }
        if self.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::General {
            self.networking_shared_chat_policy.session_active = false;
            self.networking_shared_chat_policy.session_id.clear();
            self.networking_shared_chat_policy.session_revision = 0;
            self.networking_shared_chat_policy.session_label.clear();
            self.networking_shared_chat_policy.host_authoritative = false;
        } else {
            self.networking_shared_chat_policy.host_authoritative =
                self.shared_chat_scoped_module_host_authoritative();
            if self.networking_shared_chat_policy.session_active
                && self
                    .networking_shared_chat_policy
                    .session_id
                    .trim()
                    .is_empty()
            {
                self.networking_shared_chat_policy.session_active = false;
                self.networking_shared_chat_policy.session_revision = 0;
            }
            if self.networking_shared_chat_policy.session_active
                && self
                    .networking_shared_chat_policy
                    .session_label
                    .trim()
                    .is_empty()
                && !self
                    .networking_shared_chat_policy
                    .scope_module_name
                    .trim()
                    .is_empty()
            {
                self.networking_shared_chat_policy.session_label = format!(
                    "{} room session",
                    self.networking_shared_chat_policy.scope_module_name.trim()
                );
            }
        }
        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::Open {
            self.networking_shared_chat_policy
                .turn_holder_device_id
                .clear();
            self.networking_shared_chat_policy
                .turn_holder_device_name
                .clear();
        } else if self
            .networking_shared_chat_policy
            .turn_holder_device_id
            .trim()
            .is_empty()
        {
            self.networking_shared_chat_policy.turn_holder_device_id = snapshot.device_id.clone();
            self.networking_shared_chat_policy.turn_holder_device_name = snapshot.device_name;
        }
    }

    fn shared_chat_capable_modules(&self) -> Vec<(String, String, bool)> {
        self.module_registry
            .modules
            .iter()
            .filter_map(|module| {
                let caps = module.network_capabilities.as_ref()?;
                let room_aware = caps.has(ModuleNetworkFeature::RoomAware);
                let multiplayer = caps.has(ModuleNetworkFeature::Multiplayer);
                if !room_aware && !multiplayer {
                    return None;
                }
                Some((
                    module.module_id.clone(),
                    module.display_name.clone(),
                    multiplayer,
                ))
            })
            .collect()
    }

    fn shared_chat_scoped_module_manifest(&self) -> Option<&ModuleManifest> {
        if self.networking_shared_chat_policy.scope_kind != SharedChatScopeKind::Module {
            return None;
        }
        let scoped_id = self.networking_shared_chat_policy.scope_module_id.trim();
        if scoped_id.is_empty() {
            return None;
        }
        self.module_registry
            .modules
            .iter()
            .find(|module| module.module_id.trim() == scoped_id)
    }

    fn module_manifest_by_id(&self, module_id: &str) -> Option<ModuleManifest> {
        let module_id = module_id.trim();
        if module_id.is_empty() {
            return None;
        }
        self.module_registry
            .modules
            .iter()
            .find(|module| module.module_id.trim() == module_id)
            .cloned()
    }

    fn shared_chat_scoped_module_host_authoritative(&self) -> bool {
        self.shared_chat_scoped_module_manifest()
            .and_then(|module| module.network_capabilities.as_ref())
            .map(|caps| caps.has(ModuleNetworkFeature::HostAuthoritative))
            .unwrap_or(false)
    }

    fn shared_chat_scope_label(&self) -> String {
        match self.networking_shared_chat_policy.scope_kind {
            SharedChatScopeKind::General => "General room".to_string(),
            SharedChatScopeKind::Module => {
                let name = self.networking_shared_chat_policy.scope_module_name.trim();
                if name.is_empty() {
                    "Module room".to_string()
                } else if self.networking_shared_chat_policy.scope_multiplayer {
                    format!("{name} (multiplayer)")
                } else {
                    format!("{name} (module)")
                }
            }
        }
    }

    fn shared_chat_scope_matches_module(&self, module_id: &str) -> bool {
        self.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module
            && self.networking_shared_chat_policy.scope_module_id.trim() == module_id.trim()
    }

    fn set_shared_chat_scope_general(&mut self) {
        self.networking_shared_chat_policy.scope_kind = SharedChatScopeKind::General;
        self.networking_shared_chat_policy.scope_module_id.clear();
        self.networking_shared_chat_policy.scope_module_name.clear();
        self.networking_shared_chat_policy.scope_multiplayer = false;
        self.networking_shared_chat_policy.session_active = false;
        self.networking_shared_chat_policy.session_id.clear();
        self.networking_shared_chat_policy.session_revision = 0;
        self.networking_shared_chat_policy.session_label.clear();
        self.networking_shared_chat_policy.host_authoritative = false;
    }

    fn set_shared_chat_scope_module(
        &mut self,
        module_id: impl Into<String>,
        module_name: impl Into<String>,
        multiplayer: bool,
    ) {
        self.networking_shared_chat_policy.scope_kind = SharedChatScopeKind::Module;
        self.networking_shared_chat_policy.scope_module_id = module_id.into().trim().to_string();
        self.networking_shared_chat_policy.scope_module_name =
            module_name.into().trim().to_string();
        self.networking_shared_chat_policy.scope_multiplayer = multiplayer;
        if self
            .networking_shared_chat_policy
            .scope_module_id
            .trim()
            .is_empty()
        {
            self.set_shared_chat_scope_general();
        } else {
            self.networking_shared_chat_policy.host_authoritative =
                self.shared_chat_scoped_module_host_authoritative();
            if self
                .networking_shared_chat_policy
                .session_label
                .trim()
                .is_empty()
                && !self
                    .networking_shared_chat_policy
                    .scope_module_name
                    .trim()
                    .is_empty()
            {
                self.networking_shared_chat_policy.session_label = format!(
                    "{} room session",
                    self.networking_shared_chat_policy.scope_module_name.trim()
                );
            }
        }
    }

    fn shared_chat_session_summary(&self) -> Option<String> {
        if !self.networking_shared_chat_policy.session_active {
            return None;
        }
        let label = if self
            .networking_shared_chat_policy
            .session_label
            .trim()
            .is_empty()
        {
            self.shared_chat_scope_label()
        } else {
            self.networking_shared_chat_policy
                .session_label
                .trim()
                .to_string()
        };
        Some(format!(
            "{} | revision {}{}",
            label,
            self.networking_shared_chat_policy.session_revision.max(1),
            if self.networking_shared_chat_policy.host_authoritative {
                " | host-authoritative"
            } else {
                ""
            }
        ))
    }

    fn begin_shared_chat_module_session(&mut self) -> Option<String> {
        if self.networking_shared_chat_policy.scope_kind != SharedChatScopeKind::Module {
            return None;
        }
        let scoped_module_id = self.networking_shared_chat_policy.scope_module_id.clone();
        self.reset_module_shared_session(&scoped_module_id);
        self.discard_recoverable_module_session_snapshot();
        let module_name = if self
            .networking_shared_chat_policy
            .scope_module_name
            .trim()
            .is_empty()
        {
            self.networking_shared_chat_policy.scope_module_id.clone()
        } else {
            self.networking_shared_chat_policy.scope_module_name.clone()
        };
        self.networking_shared_chat_policy.session_active = true;
        self.networking_shared_chat_policy.session_id = format!(
            "room-{}-{}",
            slugify_filename(
                &self.networking_shared_chat_policy.scope_module_id,
                "module"
            ),
            now_unix_ms().max(0) as u64
        );
        self.networking_shared_chat_policy.session_revision = 0;
        self.networking_shared_chat_policy.session_label = format!("{} room session", module_name);
        self.networking_shared_chat_policy.host_authoritative =
            self.shared_chat_scoped_module_host_authoritative();
        if self.networking_shared_chat_policy.scope_multiplayer
            && self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::Open
        {
            let local = self.networking.snapshot().clone();
            self.networking_shared_chat_policy.turn_mode = SharedChatTurnMode::TalkingStick;
            self.networking_shared_chat_policy.turn_holder_device_id = local.device_id;
            self.networking_shared_chat_policy.turn_holder_device_name = local.device_name;
        }
        Some(module_name)
    }

    fn end_shared_chat_module_session(&mut self) {
        let scoped_module_id = self.networking_shared_chat_policy.scope_module_id.clone();
        self.reset_module_shared_session(&scoped_module_id);
        self.discard_recoverable_module_session_snapshot();
        self.networking_shared_chat_policy.session_active = false;
        self.networking_shared_chat_policy.session_id.clear();
        self.networking_shared_chat_policy.session_revision = 0;
        self.networking_shared_chat_policy.session_label.clear();
        self.networking_shared_chat_policy.host_authoritative = false;
    }

    fn shared_chat_is_local_host(&self) -> bool {
        let local_id = self.networking.snapshot().device_id.as_str();
        self.networking_shared_chat_policy
            .host_device_id
            .trim()
            .is_empty()
            || self.networking_shared_chat_policy.host_device_id == local_id
    }

    fn shared_chat_turn_holder_label(&self) -> String {
        if self
            .networking_shared_chat_policy
            .turn_holder_device_name
            .trim()
            .is_empty()
        {
            if self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .trim()
                .is_empty()
            {
                "unassigned".to_string()
            } else {
                self.networking_shared_chat_policy
                    .turn_holder_device_id
                    .clone()
            }
        } else {
            self.networking_shared_chat_policy
                .turn_holder_device_name
                .clone()
        }
    }

    fn shared_chat_policy_summary(&self) -> String {
        let mut parts = vec![
            self.networking_shared_chat_policy
                .turn_mode
                .label()
                .to_string(),
            format!("AI {}", self.networking_shared_chat_policy.ai_mode.label()),
            self.shared_chat_scope_label(),
        ];
        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::TalkingStick {
            parts.push(format!("Stick {}", self.shared_chat_turn_holder_label()));
        }
        if let Some(session) = self.shared_chat_session_summary() {
            parts.push(session);
        }
        if !self
            .networking_shared_chat_policy
            .host_device_name
            .trim()
            .is_empty()
        {
            parts.push(format!(
                "Host {}",
                self.networking_shared_chat_policy.host_device_name.trim()
            ));
        }
        parts.join(" | ")
    }

    fn derive_module_host_activity_presence(
        &self,
        module_id: &str,
        module_dir: &Path,
    ) -> (String, String, u64, String) {
        let Ok(Some(status)) = read_bridge_status(module_dir) else {
            return (String::new(), String::new(), 0, String::new());
        };
        if !status.module_id.trim().is_empty() && status.module_id.trim() != module_id.trim() {
            return (String::new(), String::new(), 0, String::new());
        }
        if status.updated_at_unix_ms == 0 {
            return (String::new(), String::new(), 0, String::new());
        }

        let now_ms = now_unix_ms().max(0) as u64;
        let age_ms = now_ms.saturating_sub(status.updated_at_unix_ms);
        let editing_label = status
            .payload
            .get("activity_hint")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Host is preparing the next revision")
            .to_string();

        if age_ms <= 8_000 {
            let key = format!(
                "{}|editing|{}|{}",
                module_id.trim(),
                editing_label,
                status.updated_at_unix_ms / 2_000
            );
            return (
                "editing".to_string(),
                editing_label,
                status.updated_at_unix_ms,
                key,
            );
        }

        let idle_label = if self.networking_shared_chat_policy.host_authoritative {
            "Host is ready for the next revision"
        } else {
            "Host is connected in this module room"
        };
        let key = format!(
            "{}|idle|{}",
            module_id.trim(),
            status.updated_at_unix_ms / 15_000
        );
        (
            "idle".to_string(),
            idle_label.to_string(),
            status.updated_at_unix_ms,
            key,
        )
    }

    fn sync_shared_chat_host_presence(&mut self) {
        self.ensure_shared_chat_policy_defaults();
        if !(self.networking_shared_chat_policy.session_active
            && self.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module
            && self.shared_chat_is_local_host())
        {
            self.networking_shared_chat_presence_key.clear();
            return;
        }

        let mut next_state = String::new();
        let mut next_label = String::new();
        let mut next_updated_at = 0_u64;
        let mut next_key = String::new();

        let scoped_id = self.networking_shared_chat_policy.scope_module_id.trim();
        if let Some(module) = self
            .module_registry
            .modules
            .iter()
            .find(|module| module.module_id.trim() == scoped_id)
        {
            (next_state, next_label, next_updated_at, next_key) =
                self.derive_module_host_activity_presence(&module.module_id, &module.dir);
        }

        let changed = self.networking_shared_chat_policy.host_activity_state != next_state
            || self.networking_shared_chat_policy.host_activity_label != next_label
            || self
                .networking_shared_chat_policy
                .host_activity_updated_at_unix_ms
                != next_updated_at;

        let key_changed = self.networking_shared_chat_presence_key != next_key;
        if !changed && !key_changed {
            return;
        }

        self.networking_shared_chat_policy.host_activity_state = next_state;
        self.networking_shared_chat_policy.host_activity_label = next_label;
        self.networking_shared_chat_policy
            .host_activity_updated_at_unix_ms = next_updated_at;
        self.networking_shared_chat_presence_key = next_key;
        self.broadcast_shared_chat_policy_with_options("", false, false, false);
    }

    fn shared_chat_can_send_user_message(&self) -> Result<(), String> {
        if self.networking.snapshot().connected_peers.is_empty() {
            return Ok(());
        }

        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::Open {
            return Ok(());
        }

        let local_id = self.networking.snapshot().device_id.clone();
        let holder = self
            .networking_shared_chat_policy
            .turn_holder_device_id
            .trim()
            .to_string();
        if holder.is_empty() || holder == local_id {
            Ok(())
        } else {
            Err(format!(
                "Talking stick is currently with {}.",
                self.shared_chat_turn_holder_label()
            ))
        }
    }

    fn shared_chat_can_send_mirrored_main_chat_message(&self) -> Result<(), String> {
        if !self.networking_shared_chat_mirror_main_chat {
            return Ok(());
        }
        self.shared_chat_can_send_user_message()
    }

    fn shared_chat_local_ai_allowed(&self) -> bool {
        if !self.networking_shared_chat_mirror_main_chat
            || self.networking.snapshot().connected_peers.is_empty()
        {
            return true;
        }

        match self.networking_shared_chat_policy.ai_mode {
            SharedChatAiMode::Off => false,
            SharedChatAiMode::LocalAllowed => true,
            SharedChatAiMode::HostOnly => self.shared_chat_is_local_host(),
        }
    }

    fn build_shared_chat_message(
        &self,
        speaker_kind: &str,
        speaker_label: &str,
        body: &str,
    ) -> SharedChatMessage {
        let snapshot = self.networking.snapshot().clone();
        SharedChatMessage {
            version: "1".to_string(),
            message_id: format!(
                "room-{}-{}",
                snapshot.device_id,
                now_unix_ms().max(0) as u64
            ),
            sent_at_unix_ms: now_unix_ms().max(0) as u64,
            source_app: "chattycog".to_string(),
            from_device_id: snapshot.device_id,
            from_device_name: snapshot.device_name,
            speaker_kind: speaker_kind.to_string(),
            speaker_label: speaker_label.to_string(),
            scope_kind: self.networking_shared_chat_policy.scope_kind,
            scope_module_id: self.networking_shared_chat_policy.scope_module_id.clone(),
            scope_module_name: self.networking_shared_chat_policy.scope_module_name.clone(),
            scope_multiplayer: self.networking_shared_chat_policy.scope_multiplayer,
            session_active: self.networking_shared_chat_policy.session_active,
            session_id: self.networking_shared_chat_policy.session_id.clone(),
            session_revision: self.networking_shared_chat_policy.session_revision,
            body: body.trim().to_string(),
        }
    }

    fn push_shared_chat_message_local(&mut self, message: SharedChatMessage) {
        if message.message_id.trim().is_empty()
            || self
                .networking_shared_chat_seen_messages
                .contains(&message.message_id)
        {
            return;
        }

        self.networking_shared_chat_seen_messages
            .insert(message.message_id.clone());
        self.networking_shared_chat_log.push(message);
        self.networking_shared_chat_log
            .sort_by_key(|entry| entry.sent_at_unix_ms);
        if self.networking_shared_chat_log.len() > 120 {
            let drop_count = self.networking_shared_chat_log.len() - 120;
            self.networking_shared_chat_log.drain(0..drop_count);
        }
    }

    fn add_shared_chat_notice(&mut self, body: impl Into<String>) {
        let body = body.into();
        if body.trim().is_empty() {
            return;
        }
        let mut message = self.build_shared_chat_message("system", "Room", &body);
        message.source_app = "chattycog".to_string();
        self.push_shared_chat_message_local(message);
    }

    fn broadcast_shared_chat_policy(&mut self, note: &str) {
        self.broadcast_shared_chat_policy_with_options(note, true, true, false);
    }

    fn broadcast_shared_chat_policy_with_options(
        &mut self,
        note: &str,
        bump_revision: bool,
        announce: bool,
        preserve_host_assignment: bool,
    ) {
        self.ensure_shared_chat_policy_defaults();
        let snapshot = self.networking.snapshot().clone();
        self.networking_shared_chat_policy.updated_at_unix_ms = now_unix_ms().max(0) as u64;
        self.networking_shared_chat_policy.source_app = "chattycog".to_string();
        if !preserve_host_assignment
            || self
                .networking_shared_chat_policy
                .host_device_id
                .trim()
                .is_empty()
        {
            self.networking_shared_chat_policy.host_device_id = snapshot.device_id;
            self.networking_shared_chat_policy.host_device_name = snapshot.device_name;
        }
        if self.networking_shared_chat_policy.session_active
            && self.networking_shared_chat_policy.scope_kind == SharedChatScopeKind::Module
            && bump_revision
        {
            self.networking_shared_chat_policy.session_revision = self
                .networking_shared_chat_policy
                .session_revision
                .saturating_add(1)
                .max(1);
            if self
                .networking_shared_chat_policy
                .session_label
                .trim()
                .is_empty()
            {
                self.networking_shared_chat_policy.session_label = format!(
                    "{} room session",
                    self.networking_shared_chat_policy.scope_module_name.trim()
                );
            }
        }
        if self.networking_shared_chat_policy.turn_mode == SharedChatTurnMode::Open {
            self.networking_shared_chat_policy
                .turn_holder_device_id
                .clear();
            self.networking_shared_chat_policy
                .turn_holder_device_name
                .clear();
        }

        let summary = self.shared_chat_policy_summary();
        if announce {
            self.add_shared_chat_notice(if note.trim().is_empty() {
                format!("Shared room policy updated: {summary}.")
            } else {
                format!("Shared room policy updated: {summary}. {note}")
            });
        }

        match serde_json::to_string_pretty(&self.networking_shared_chat_policy) {
            Ok(text) => {
                let connection_ids = self.shared_chat_connected_connection_ids();
                for connection_id in &connection_ids {
                    self.networking.send_artifact(
                        connection_id,
                        "shared_chat_policy_json",
                        "Shared room policy",
                        None,
                        &summary,
                        "shared_chat_policy.json",
                        &text,
                    );
                }
                if announce {
                    self.networking_status = if connection_ids.is_empty() {
                        "Networking: updated shared room policy locally.".to_string()
                    } else {
                        format!(
                            "Networking: shared room policy sent to {} connected peer(s).",
                            connection_ids.len()
                        )
                    };
                }
            }
            Err(err) => {
                self.networking_status =
                    format!("Networking: could not serialize shared room policy: {err}");
            }
        }
        self.sync_recoverable_shared_chat_policy_snapshot();
    }

    fn apply_received_shared_chat_policy(
        &mut self,
        artifact: &chattycog_gui::networking::ReceivedArtifact,
    ) -> anyhow::Result<()> {
        let mut policy: SharedChatPolicy = serde_json::from_str(&artifact.text)?;
        if policy.version.trim().is_empty() {
            policy.version = "1".to_string();
        }
        if policy.label.trim().is_empty() {
            policy.label = "Shared room".to_string();
        }
        if policy.source_app.trim().is_empty() {
            policy.source_app = "chattycog".to_string();
        }
        if policy.scope_kind == SharedChatScopeKind::Module
            && policy.scope_module_id.trim().is_empty()
        {
            policy.scope_kind = SharedChatScopeKind::General;
            policy.scope_module_name.clear();
            policy.scope_multiplayer = false;
            policy.session_active = false;
            policy.session_id.clear();
            policy.session_revision = 0;
            policy.session_label.clear();
            policy.host_authoritative = false;
        }

        let should_apply = self.networking_shared_chat_policy.updated_at_unix_ms == 0
            || policy.updated_at_unix_ms >= self.networking_shared_chat_policy.updated_at_unix_ms;
        if !should_apply {
            self.networking_status = format!(
                "Networking: ignored older shared room policy from {}.",
                artifact.from_device_name
            );
            return Ok(());
        }

        let previous_policy = self.networking_shared_chat_policy.clone();
        let previously_local_host = previous_policy.host_device_id.trim().is_empty()
            || previous_policy.host_device_id.trim() == self.networking.snapshot().device_id.trim();
        self.networking_shared_chat_policy = policy;
        self.ensure_shared_chat_policy_defaults();
        let now_local_host = self.shared_chat_is_local_host();
        if now_local_host {
            self.networking_shared_chat_presence_next_sync_at = Some(Instant::now());
        }
        if previously_local_host && !now_local_host {
            self.networking_shared_chat_presence_key.clear();
        }
        let presence_only =
            previous_policy.equivalent_except_presence(&self.networking_shared_chat_policy);
        if !presence_only {
            self.add_shared_chat_notice(format!(
                "{} updated the shared room: {}.",
                artifact.from_device_name,
                self.shared_chat_policy_summary()
            ));
            self.networking_status = format!(
                "Networking: shared room policy updated from {}.",
                artifact.from_device_name
            );
        }
        self.sync_recoverable_shared_chat_policy_snapshot();
        Ok(())
    }

    fn broadcast_shared_chat_message(
        &mut self,
        speaker_kind: &str,
        speaker_label: &str,
        body: &str,
    ) {
        let connection_ids = self.shared_chat_connected_connection_ids();
        if connection_ids.is_empty() || body.trim().is_empty() {
            return;
        }

        let message = self.build_shared_chat_message(speaker_kind, speaker_label, body);
        let summary = one_line(body, 96);
        self.push_shared_chat_message_local(message.clone());

        match serde_json::to_string_pretty(&message) {
            Ok(text) => {
                let file_name = format!(
                    "shared_chat_message_{}.json",
                    slugify_filename(&message.message_id, "shared_chat_message")
                );
                for connection_id in &connection_ids {
                    self.networking.send_artifact(
                        connection_id,
                        "shared_chat_message_json",
                        "Shared room message",
                        None,
                        &summary,
                        &file_name,
                        &text,
                    );
                }
                self.networking_status = format!(
                    "Networking: shared room message sent to {} connected peer(s).",
                    connection_ids.len()
                );
            }
            Err(err) => {
                self.networking_status =
                    format!("Networking: could not serialize shared room message: {err}");
            }
        }
    }

    fn apply_received_shared_chat_message(
        &mut self,
        artifact: &chattycog_gui::networking::ReceivedArtifact,
    ) -> anyhow::Result<()> {
        let mut message: SharedChatMessage = serde_json::from_str(&artifact.text)?;
        if message.version.trim().is_empty() {
            message.version = "1".to_string();
        }
        if message.scope_kind == SharedChatScopeKind::Module
            && message.scope_module_id.trim().is_empty()
        {
            message.scope_kind = SharedChatScopeKind::General;
            message.scope_module_name.clear();
            message.scope_multiplayer = false;
            message.session_active = false;
            message.session_id.clear();
            message.session_revision = 0;
        }
        if message.message_id.trim().is_empty() {
            message.message_id = format!(
                "room-{}-{}",
                artifact.from_device_id,
                now_unix_ms().max(0) as u64
            );
        }
        if message.from_device_name.trim().is_empty() {
            message.from_device_name = artifact.from_device_name.clone();
        }
        if message.from_device_id.trim().is_empty() {
            message.from_device_id = artifact.from_device_id.clone();
        }
        self.push_shared_chat_message_local(message);
        self.networking_status = format!(
            "Networking: shared room message received from {}.",
            artifact.from_device_name
        );
        Ok(())
    }

    fn build_module_shared_room_state(
        &self,
        manifest: &ModuleManifest,
    ) -> Option<ModuleBridgeSharedRoomState> {
        let caps = manifest.network_capabilities.as_ref()?;
        let room_aware = caps.has(ModuleNetworkFeature::RoomAware);
        let multiplayer = caps.has(ModuleNetworkFeature::Multiplayer);
        if !room_aware && !multiplayer {
            return None;
        }
        let scope_matches = self.shared_chat_scope_matches_module(&manifest.module_id);
        let active_for_module = self.networking_shared_chat_policy.scope_kind
            == SharedChatScopeKind::General
            || scope_matches;
        let snapshot = self.networking.snapshot().clone();
        let local_id = snapshot.device_id.clone();
        let local_name = snapshot.device_name.clone();
        let local_has_turn = self.networking_shared_chat_policy.turn_mode
            == SharedChatTurnMode::Open
            || self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .trim()
                .is_empty()
            || self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .trim()
                == local_id.trim();
        let mut participants = vec![ModuleBridgeSharedRoomParticipant {
            device_id: local_id.clone(),
            device_name: local_name.clone(),
            is_local: true,
            connected: true,
        }];
        participants.extend(snapshot.connected_peers.iter().map(|peer| {
            ModuleBridgeSharedRoomParticipant {
                device_id: peer.device_id.clone(),
                device_name: self.network_display_name(&peer.device_id, &peer.device_name),
                is_local: false,
                connected: true,
            }
        }));
        let participant_count = participants.len();
        Some(ModuleBridgeSharedRoomState {
            version: "1".to_string(),
            source_app: "chattycog".to_string(),
            label: self.networking_shared_chat_policy.label.clone(),
            scope_kind: match self.networking_shared_chat_policy.scope_kind {
                SharedChatScopeKind::General => "general".to_string(),
                SharedChatScopeKind::Module => "module".to_string(),
            },
            scope_module_id: self.networking_shared_chat_policy.scope_module_id.clone(),
            scope_module_name: self.networking_shared_chat_policy.scope_module_name.clone(),
            scope_multiplayer: self.networking_shared_chat_policy.scope_multiplayer,
            active_for_module,
            session_active: self.networking_shared_chat_policy.session_active,
            session_id: self.networking_shared_chat_policy.session_id.clone(),
            session_revision: self.networking_shared_chat_policy.session_revision,
            session_label: self.networking_shared_chat_policy.session_label.clone(),
            host_authoritative: self.networking_shared_chat_policy.host_authoritative,
            turn_mode: self
                .networking_shared_chat_policy
                .turn_mode
                .label()
                .to_string(),
            ai_mode: self
                .networking_shared_chat_policy
                .ai_mode
                .label()
                .to_string(),
            teacher_override: false,
            host_device_id: self.networking_shared_chat_policy.host_device_id.clone(),
            host_device_name: self.networking_shared_chat_policy.host_device_name.clone(),
            turn_holder_device_id: self
                .networking_shared_chat_policy
                .turn_holder_device_id
                .clone(),
            turn_holder_device_name: self
                .networking_shared_chat_policy
                .turn_holder_device_name
                .clone(),
            connected_peer_count: self.shared_chat_connected_connection_ids().len(),
            participant_count,
            local_device_id: local_id,
            local_device_name: local_name,
            local_is_host: self.shared_chat_is_local_host(),
            local_has_turn,
            host_activity_state: self
                .networking_shared_chat_policy
                .host_activity_state
                .clone(),
            host_activity_label: self
                .networking_shared_chat_policy
                .host_activity_label
                .clone(),
            host_activity_updated_at_unix_ms: self
                .networking_shared_chat_policy
                .host_activity_updated_at_unix_ms,
            participants,
            summary: self.shared_chat_policy_summary(),
            updated_at_unix_ms: now_unix_ms().max(0) as u64,
        })
    }

    fn build_module_shared_room_events(
        &self,
        manifest: &ModuleManifest,
    ) -> Option<ModuleBridgeSharedRoomEvents> {
        let room_state = self.build_module_shared_room_state(manifest)?;
        if !room_state.active_for_module {
            return None;
        }

        let module_id = manifest.module_id.trim();
        let session_id = room_state.session_id.trim();
        let mut events = self
            .networking
            .snapshot()
            .received_session_events
            .iter()
            .filter(|event| {
                let scope = event.scope_module_id.trim();
                let scope_matches = scope.is_empty() || scope == module_id;
                let session_matches = session_id.is_empty()
                    || event.session_id.trim().is_empty()
                    || event.session_id.trim() == session_id;
                scope_matches && session_matches
            })
            .map(|event| ModuleBridgeRoomEvent {
                event_id: event.event_id.clone(),
                source_app: "chattycog-lan".to_string(),
                scope_module_id: event.scope_module_id.clone(),
                session_id: event.session_id.clone(),
                event_type: event.event_type.clone(),
                label: event.label.clone(),
                content_type: event.content_type.clone(),
                payload_text: event.payload_text.clone(),
                from_device_id: event.from_device_id.clone(),
                from_device_name: event.from_device_name.clone(),
                local_echo: false,
                sent_at_unix_ms: event.received_at_unix_ms,
                received_at_unix_ms: event.received_at_unix_ms,
            })
            .collect::<Vec<_>>();

        if events.is_empty() {
            return None;
        }

        events.sort_by_key(|event| event.received_at_unix_ms);
        Some(ModuleBridgeSharedRoomEvents {
            version: "1".to_string(),
            source_app: "chattycog".to_string(),
            scope_module_id: manifest.module_id.clone(),
            session_id: room_state.session_id.clone(),
            session_revision: room_state.session_revision,
            updated_at_unix_ms: now_unix_ms().max(0) as u64,
            events,
        })
    }

    fn sync_module_shared_room_bridge_state(&mut self) {
        for module in &self.module_registry.modules {
            let state = self.build_module_shared_room_state(module);
            let Some(state) = state else {
                let _ = clear_bridge_shared_room_state(&module.dir);
                self.module_room_bridge_last_fingerprint
                    .remove(&module.module_id);
                continue;
            };
            let fingerprint = state.fingerprint();
            let needs_write = self
                .module_room_bridge_last_fingerprint
                .get(&module.module_id)
                .map(|existing| existing != &fingerprint)
                .unwrap_or(true);
            if needs_write && write_bridge_shared_room_state(&module.dir, &state).is_ok() {
                self.module_room_bridge_last_fingerprint
                    .insert(module.module_id.clone(), fingerprint);
            }
        }
    }

    fn sync_module_shared_room_events_bridge(&mut self) {
        for module in &self.module_registry.modules {
            let events = self.build_module_shared_room_events(module);
            let Some(events) = events else {
                let _ = clear_bridge_shared_room_events(&module.dir);
                self.module_room_events_bridge_last_fingerprint
                    .remove(&module.module_id);
                continue;
            };
            let fingerprint = serde_json::to_string(&events).unwrap_or_default();
            let needs_write = self
                .module_room_events_bridge_last_fingerprint
                .get(&module.module_id)
                .map(|existing| existing != &fingerprint)
                .unwrap_or(true);
            if needs_write && write_bridge_shared_room_events(&module.dir, &events).is_ok() {
                self.module_room_events_bridge_last_fingerprint
                    .insert(module.module_id.clone(), fingerprint);
            }
        }
    }

    fn process_module_outgoing_room_events(&mut self) {
        let snapshot = self.networking.snapshot().clone();
        let room_connection_ids = self.shared_chat_connected_connection_ids();
        for module in &self.module_registry.modules {
            let Some(caps) = module.network_capabilities.as_ref() else {
                let _ = clear_bridge_outgoing_room_events(&module.dir);
                continue;
            };
            if !caps.has(ModuleNetworkFeature::RoomAware)
                && !caps.has(ModuleNetworkFeature::Multiplayer)
            {
                let _ = clear_bridge_outgoing_room_events(&module.dir);
                continue;
            }
            let outgoing_events = match read_bridge_outgoing_room_events(&module.dir) {
                Ok(events) => events,
                Err(err) => {
                    self.networking_status = format!(
                        "Networking: could not read outgoing room events for {}: {err}",
                        module.display_name
                    );
                    continue;
                }
            };
            if outgoing_events.is_empty() {
                continue;
            }
            let Some(room_state) = self.build_module_shared_room_state(module) else {
                continue;
            };
            if !room_state.active_for_module || room_connection_ids.is_empty() {
                continue;
            }

            let session_id = if room_state.session_active {
                room_state.session_id.clone()
            } else {
                String::new()
            };
            let local_name = snapshot.device_name.clone();
            let local_id = snapshot.device_id.clone();
            for mut event in outgoing_events.iter().cloned() {
                event.normalize();
                if event.event_id.trim().is_empty() {
                    event.event_id = format!(
                        "module-room-{}-{}",
                        module.module_id,
                        now_unix_ms().max(0) as u64
                    );
                }
                let label = if event.label.trim().is_empty() {
                    format!("{} event", module.display_name.trim())
                } else {
                    event.label.clone()
                };
                for connection_id in &room_connection_ids {
                    self.networking.send_session_event(
                        connection_id,
                        &module.module_id,
                        &session_id,
                        &event.event_type,
                        &label,
                        &event.content_type,
                        &event.payload_text,
                    );
                }
                self.networking_status = format!(
                    "Networking: relayed {} room event(s) from {} to {} connected peer(s).",
                    outgoing_events.len(),
                    if local_name.trim().is_empty() {
                        local_id.trim()
                    } else {
                        local_name.trim()
                    },
                    room_connection_ids.len()
                );
            }
            let _ = clear_bridge_outgoing_room_events(&module.dir);
        }
    }

    fn network_inbox_dir(&self) -> PathBuf {
        self.modules_dir
            .as_ref()
            .and_then(|dir| dir.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.logs_dir
                    .as_ref()
                    .and_then(|dir| dir.parent().map(Path::to_path_buf))
            })
            .or_else(|| {
                self.sandbox_dir
                    .as_ref()
                    .and_then(|dir| dir.parent().map(Path::to_path_buf))
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("network_inbox")
    }

    fn network_recovery_dir(&self) -> PathBuf {
        self.modules_dir
            .as_ref()
            .and_then(|dir| dir.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.logs_dir
                    .as_ref()
                    .and_then(|dir| dir.parent().map(Path::to_path_buf))
            })
            .or_else(|| {
                self.sandbox_dir
                    .as_ref()
                    .and_then(|dir| dir.parent().map(Path::to_path_buf))
            })
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("network_recovery")
    }

    fn recoverable_module_session_path(&self) -> PathBuf {
        self.network_recovery_dir()
            .join("recoverable_module_session.json")
    }

    fn recoverable_module_session_payload_dir(&self) -> PathBuf {
        self.network_recovery_dir().join("module_session_payloads")
    }

    fn received_workflow_inbox_dir(&self) -> PathBuf {
        self.network_inbox_dir().join("workflow_states")
    }

    fn received_lukewarm_inbox_dir(&self) -> PathBuf {
        self.network_inbox_dir().join("lukewarm_context")
    }

    fn applied_lukewarm_dir(&self) -> PathBuf {
        self.network_inbox_dir().join("applied_lukewarm_context")
    }

    fn received_bundle_inbox_dir(&self) -> PathBuf {
        self.network_inbox_dir().join("workflow_bundles")
    }

    fn received_transfer_inbox_dir(&self) -> PathBuf {
        self.network_inbox_dir().join("file_transfers")
    }

    fn received_transfer_payload_dir(&self) -> PathBuf {
        self.received_transfer_inbox_dir().join("payloads")
    }

    fn applied_transfer_dir(&self) -> PathBuf {
        self.network_inbox_dir()
            .join("imports")
            .join("network_transfers")
    }

    fn sync_selected_received_workflow(&mut self) {
        let still_exists = self
            .selected_received_workflow
            .as_ref()
            .is_some_and(|path| {
                self.received_workflow_inbox
                    .iter()
                    .any(|item| &item.path == path)
            });
        if !still_exists {
            self.selected_received_workflow = self
                .received_workflow_inbox
                .first()
                .map(|item| item.path.clone());
        }
    }

    fn refresh_received_workflow_inbox(&mut self) {
        self.received_workflow_inbox =
            load_received_workflow_inbox(&self.network_inbox_dir()).unwrap_or_default();
        self.sync_selected_received_workflow();
    }

    fn sync_selected_received_lukewarm(&mut self) {
        let still_exists = self
            .selected_received_lukewarm
            .as_ref()
            .is_some_and(|path| {
                self.received_lukewarm_inbox
                    .iter()
                    .any(|item| &item.path == path)
            });
        if !still_exists {
            self.selected_received_lukewarm = self
                .received_lukewarm_inbox
                .first()
                .map(|item| item.path.clone());
        }
    }

    fn refresh_received_lukewarm_inbox(&mut self) {
        self.received_lukewarm_inbox =
            load_received_lukewarm_inbox(&self.received_lukewarm_inbox_dir()).unwrap_or_default();
        self.sync_selected_received_lukewarm();
    }

    fn sync_selected_received_transfer(&mut self) {
        let still_exists = self
            .selected_received_transfer
            .as_ref()
            .is_some_and(|path| {
                self.received_transfer_inbox
                    .iter()
                    .any(|item| &item.path == path)
            });
        if !still_exists {
            self.selected_received_transfer = self
                .received_transfer_inbox
                .first()
                .map(|item| item.path.clone());
        }
    }

    fn refresh_received_transfer_inbox(&mut self) {
        self.received_transfer_inbox =
            load_received_generic_transfer_inbox(&self.received_transfer_inbox_dir())
                .unwrap_or_default();
        self.sync_selected_received_transfer();
    }

    fn sync_selected_received_bundle(&mut self) {
        let still_exists = self.selected_received_bundle.as_ref().is_some_and(|path| {
            self.received_bundle_inbox
                .iter()
                .any(|item| &item.path == path)
        });
        if !still_exists {
            self.selected_received_bundle = self
                .received_bundle_inbox
                .first()
                .map(|item| item.path.clone());
        }
    }

    fn refresh_received_bundle_inbox(&mut self) {
        self.received_bundle_inbox =
            load_received_workflow_bundle_inbox(&self.received_bundle_inbox_dir())
                .unwrap_or_default();
        self.sync_selected_received_bundle();
    }

    fn build_current_lukewarm_share(&self) -> SharedLukewarmContext {
        let mut departments_context =
            read_departments_from_logs_dir(self.logs_dir.as_deref()).unwrap_or_default();
        if departments_context.contains("<bullet>") || departments_context.contains("<paragraph>") {
            departments_context = departments_context
                .replace("<bullet>", "")
                .replace("<paragraph>", "");
        }
        let lukewarm_context =
            read_lukewarm_from_logs_dir(self.logs_dir.as_deref()).unwrap_or_default();
        let mut sections = Vec::new();
        if !departments_context.trim().is_empty() {
            sections.push(format!(
                "### Department Status Updates\n{}",
                truncate_for_ui(departments_context.trim(), 3_000)
            ));
        }
        if !lukewarm_context.trim().is_empty() {
            sections.push(format!(
                "### Recent Activity (Luke Warm)\n{}",
                truncate_for_ui(lukewarm_context.trim(), 2_000)
            ));
        }
        let context_text = sections.join("\n\n");
        let label = if context_text.trim().is_empty() {
            "ChattyCog luke warm context".to_string()
        } else {
            "ChattyCog recent context".to_string()
        };
        let summary = if context_text.trim().is_empty() {
            "No current luke warm summary is available yet.".to_string()
        } else {
            let mut parts = Vec::new();
            if !departments_context.trim().is_empty() {
                parts.push("department status");
            }
            if !lukewarm_context.trim().is_empty() {
                parts.push("recent activity");
            }
            format!("Shareable local context: {}", parts.join(" + "))
        };
        let snapshot = self.networking.snapshot().clone();
        SharedLukewarmContext {
            version: "1.0".to_string(),
            label,
            summary,
            created_at_unix_ms: now_unix_ms().max(0) as u64,
            source_app: "ChattyCog".to_string(),
            source_device_id: snapshot.device_id,
            source_device_name: snapshot.device_name,
            context_text,
        }
    }

    fn build_applied_lukewarm_prompt_block(&self) -> String {
        if !self.prefs.network_allow_shared_lukewarm_context {
            return String::new();
        }
        let items =
            load_applied_lukewarm_contexts(&self.applied_lukewarm_dir()).unwrap_or_default();
        if items.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        for item in items.into_iter().take(6) {
            let label = if item.record.from_device_name.trim().is_empty() {
                item.record.from_device_id.as_str()
            } else {
                item.record.from_device_name.as_str()
            };
            let text = item.record.context.context_text.trim();
            if text.is_empty() {
                continue;
            }
            out.push_str("From ");
            out.push_str(label);
            out.push_str(":\n");
            out.push_str(&truncate_for_ui(text, 1_200));
            out.push_str("\n\n");
            if out.len() >= 4_000 {
                break;
            }
        }
        out.trim().to_string()
    }

    fn current_system_prompt(&self) -> String {
        self.messages
            .iter()
            .find(|message| matches!(message.role, Role::System))
            .map(|message| message.content.clone())
            .unwrap_or_else(|| "You are ChattyCog. Respond concisely and helpfully.".to_string())
    }

    fn portable_model_hint(&self, path: Option<&Path>) -> Option<String> {
        portable_model_hint_for_dirs(
            self.models_dir.as_deref(),
            self.modules_dir.as_deref(),
            path,
        )
    }

    fn resolve_portable_model_hint(&self, hint: Option<&str>) -> Option<PathBuf> {
        resolve_portable_model_hint_for_dirs(
            self.models_dir.as_deref(),
            self.modules_dir.as_deref(),
            hint,
        )
    }

    fn build_current_workflow_bundle(&self) -> WorkflowBundle {
        WorkflowBundle {
            version: "1.0".to_string(),
            label: self.networking_bundle_label.trim().to_string(),
            summary: self.networking_bundle_summary.trim().to_string(),
            created_at_unix_ms: now_unix_ms().max(0) as u64,
            system_prompt: self.current_system_prompt(),
            orchestrator_model_hint: self.portable_model_hint(self.gguf_path.as_deref()),
            orchestrator_params: GenParams {
                temp: self.orch_temp,
                top_p: self.orch_top_p,
                top_k: self.orch_top_k,
                max_tokens: self.orch_max_tokens,
            },
            bookkeeper_model_hint: self.portable_model_hint(self.bookkeeper_model_path.as_deref()),
            bookkeeper_params: GenParams {
                temp: self.bookkeeper_temp,
                top_p: self.bookkeeper_top_p,
                top_k: self.bookkeeper_top_k,
                max_tokens: self.bookkeeper_max_tokens,
            },
            allow_sandbox_tool_requests: self.prefs.allow_sandbox_tool_requests,
            auto_generate_module_suspend_rundown: self.prefs.auto_generate_module_suspend_rundown,
            module_preferences: self.prefs.modules.clone(),
        }
    }

    fn connected_connection_id_for_device(&self, device_id: &str) -> Option<String> {
        let wanted = device_id.trim();
        if wanted.is_empty() {
            return None;
        }
        self.networking
            .snapshot()
            .connected_peers
            .iter()
            .find(|peer| peer.device_id.trim() == wanted)
            .map(|peer| peer.connection_id.clone())
    }

    fn prepare_outgoing_module_shared_state(
        &mut self,
        module_id: &str,
        shared_state: &ModuleBridgeSharedState,
    ) -> ModuleBridgeSharedState {
        let fingerprint = shared_state.content_fingerprint();
        let snapshot = self.networking.snapshot().clone();
        let now = now_unix_ms().max(0) as u64;
        let tracker = self
            .module_session_trackers
            .entry(module_id.to_string())
            .or_insert_with(|| ModuleSessionTracker {
                session_id: format!(
                    "session-{}-{}-{}",
                    slugify_filename(module_id, "module"),
                    slugify_filename(&snapshot.device_id, "device"),
                    now
                ),
                ..ModuleSessionTracker::default()
            });

        if tracker.last_revision == 0 {
            tracker.last_revision = 1;
        }
        if tracker.last_fingerprint.trim().is_empty() {
            tracker.last_fingerprint = fingerprint.clone();
        } else if tracker.last_fingerprint != fingerprint {
            tracker.last_revision += 1;
            tracker.last_fingerprint = fingerprint.clone();
        }
        tracker.last_shared_at_unix_ms = now;

        let mut prepared = shared_state.clone();
        prepared.module_id = module_id.trim().to_string();
        prepared.session_id = tracker.session_id.clone();
        prepared.session_revision = tracker.last_revision;
        prepared.authoritative_device_id = snapshot.device_id;
        prepared.authoritative_device_name = snapshot.device_name;
        prepared.host_authoritative = true;
        prepared.updated_at_unix_ms = now;
        prepared
    }

    fn reset_module_shared_session(&mut self, module_id: &str) {
        self.module_session_trackers.remove(module_id);
        self.module_session_receipts
            .retain(|receipt| receipt.module_id.trim() != module_id.trim());
    }

    fn module_session_receipts_for(&self, module_id: &str) -> Vec<ModuleSessionAckRecord> {
        let mut items = self
            .module_session_receipts
            .iter()
            .filter(|receipt| receipt.module_id.trim() == module_id.trim())
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .acknowledged_at_unix_ms
                .cmp(&left.acknowledged_at_unix_ms)
        });
        items
    }

    fn send_module_session_ack(
        &mut self,
        record: &ReceivedWorkflowStateRecord,
        applied: bool,
        stale: bool,
        message: &str,
    ) {
        let Some(connection_id) = self.connected_connection_id_for_device(&record.from_device_id)
        else {
            return;
        };
        let ack = ModuleSessionAckRecord {
            module_id: record.module_id.clone(),
            session_id: record.shared_state.session_id.clone(),
            session_revision: record.shared_state.session_revision,
            from_device_id: self.networking.snapshot().device_id.clone(),
            from_device_name: self.networking.snapshot().device_name.clone(),
            applied,
            stale,
            message: message.trim().to_string(),
            acknowledged_at_unix_ms: now_unix_ms().max(0) as u64,
        };
        if let Ok(text) = serde_json::to_string_pretty(&ack) {
            self.networking.send_artifact(
                &connection_id,
                "module_shared_state_ack_json",
                &format!("{} session ack", record.module_id),
                Some(&record.module_id),
                message,
                &format!(
                    "{}_session_ack.json",
                    slugify_filename(&record.module_id, "module")
                ),
                &text,
            );
        }
    }

    fn stale_module_state_message(
        &self,
        module_dir: &Path,
        record: &ReceivedWorkflowStateRecord,
    ) -> Option<String> {
        let incoming = &record.shared_state;
        if incoming.session_id.trim().is_empty() || incoming.session_revision == 0 {
            return None;
        }
        let existing = read_bridge_incoming_shared_state(module_dir)
            .ok()
            .flatten()?;
        if existing.session_id.trim() == incoming.session_id.trim()
            && existing.session_revision >= incoming.session_revision
        {
            Some(format!(
                "Session revision {} is older than or equal to the already applied revision {}.",
                incoming.session_revision, existing.session_revision
            ))
        } else {
            None
        }
    }

    fn store_received_module_shared_state(
        &mut self,
        artifact: &ReceivedArtifact,
    ) -> std::io::Result<PathBuf> {
        let module_id = artifact.module_id.trim();
        if module_id.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "module shared state is missing module_id",
            ));
        }

        let mut shared_state: ModuleBridgeSharedState = serde_json::from_str(&artifact.text)
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("shared state parse error: {err}"),
                )
            })?;
        if shared_state.module_id.trim().is_empty() {
            shared_state.module_id = module_id.to_string();
        }
        if shared_state.session_id.trim().is_empty() {
            shared_state.session_id = format!("legacy-{}", artifact.artifact_id);
        }
        if shared_state.session_revision == 0 {
            shared_state.session_revision = 1;
        }
        if shared_state.authoritative_device_id.trim().is_empty() {
            shared_state.authoritative_device_id = artifact.from_device_id.clone();
        }
        if shared_state.authoritative_device_name.trim().is_empty() {
            shared_state.authoritative_device_name = artifact.from_device_name.clone();
        }
        shared_state.host_authoritative = true;

        let dir = self.received_workflow_inbox_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "{}__{}__{}.json",
            slugify_filename(module_id, "module"),
            slugify_filename(&artifact.from_device_name, "peer"),
            now_unix_ms().max(0)
        ));
        let record = ReceivedWorkflowStateRecord {
            artifact_id: artifact.artifact_id.clone(),
            from_device_id: artifact.from_device_id.clone(),
            from_device_name: artifact.from_device_name.clone(),
            label: artifact.label.clone(),
            summary: if artifact.summary.trim().is_empty() {
                shared_state.summary.clone()
            } else {
                artifact.summary.clone()
            },
            file_name: artifact.file_name.clone(),
            received_at_unix_ms: now_unix_ms().max(0) as u64,
            module_id: module_id.to_string(),
            shared_state,
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("shared workflow serialize error: {err}"),
            )
        })?;
        std::fs::write(&path, bytes)?;
        self.refresh_received_workflow_inbox();
        self.selected_received_workflow = Some(path.clone());
        let module_label = self
            .module_registry
            .modules
            .iter()
            .find(|module| module.module_id == module_id)
            .map(|module| module.display_name.clone())
            .unwrap_or_else(|| module_id.to_string());
        self.networking_status = format!(
            "Networking: received shared workflow for {} from {} and saved it to the inbox.",
            module_label, artifact.from_device_name
        );
        Ok(path)
    }

    fn accept_received_workflow_state(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        let record = read_received_workflow_state_record(path)?;
        let module = self
            .module_registry
            .modules
            .iter()
            .find(|module| module.module_id == record.module_id)
            .cloned()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("module `{}` is not installed here", record.module_id),
                )
            })?;
        if !module_allows_network_feature(Some(&module), ModuleNetworkFeature::SharedStateReceive) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "module `{}` does not declare shared_state_receive support",
                    module.display_name
                ),
            ));
        }

        let incoming = ModuleBridgeIncomingSharedState {
            module_id: record.module_id.clone(),
            from_device_id: record.from_device_id.clone(),
            from_device_name: record.from_device_name.clone(),
            summary: record.summary.clone(),
            session_id: record.shared_state.session_id.clone(),
            session_revision: record.shared_state.session_revision,
            authoritative_device_id: record.shared_state.authoritative_device_id.clone(),
            authoritative_device_name: record.shared_state.authoritative_device_name.clone(),
            host_authoritative: record.shared_state.host_authoritative,
            payload: record.shared_state.payload.clone(),
            received_at_unix_ms: now_unix_ms().max(0) as u64,
        };
        if let Some(reason) = self.stale_module_state_message(&module.dir, &record) {
            self.send_module_session_ack(&record, false, true, &reason);
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                reason,
            ));
        }
        write_bridge_incoming_shared_state(&module.dir, &incoming)
            .map_err(|err| std::io::Error::other(format!("bridge write error: {err}")))?;
        let applied_path = bridge_incoming_shared_state_path(&module.dir);
        std::fs::remove_file(path)?;
        self.refresh_received_workflow_inbox();
        self.send_module_session_ack(
            &record,
            true,
            false,
            &format!(
                "Applied revision {} for session {}.",
                record.shared_state.session_revision,
                if record.shared_state.session_id.trim().is_empty() {
                    "(legacy)"
                } else {
                    record.shared_state.session_id.trim()
                }
            ),
        );
        self.networking_status = format!(
            "Networking: applied shared workflow for {} from {}.",
            module.display_name, record.from_device_name
        );
        push_hot_memory(
            self,
            format!(
                "Applied workflow from {}: {}",
                record.from_device_name,
                one_line(
                    if record.summary.trim().is_empty() {
                        &module.display_name
                    } else {
                        &record.summary
                    },
                    120
                )
            ),
        );
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Module,
                source: "network".to_string(),
                module: Some(record.module_id.clone()),
                event_type: Some("shared_state_applied".to_string()),
                text: format!(
                    "Applied shared workflow from {} for module `{}`.\nSaved to {}\n\nSummary: {}",
                    record.from_device_name,
                    record.module_id,
                    applied_path.display(),
                    record.summary
                ),
                tags: vec![
                    "lan".to_string(),
                    "module".to_string(),
                    "shared_state".to_string(),
                    "applied".to_string(),
                ],
                entities: vec![record.from_device_name.clone(), record.module_id.clone()],
                payload_json: serde_json::to_string(&record.shared_state.payload).ok(),
            });
        }
        Ok(applied_path)
    }

    fn store_received_module_session_ack(
        &mut self,
        artifact: &ReceivedArtifact,
    ) -> std::io::Result<ModuleSessionAckRecord> {
        let mut ack: ModuleSessionAckRecord =
            serde_json::from_str(&artifact.text).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("module session ack parse error: {err}"),
                )
            })?;
        ack.module_id = ack.module_id.trim().to_string();
        ack.session_id = ack.session_id.trim().to_string();
        ack.from_device_id = if ack.from_device_id.trim().is_empty() {
            artifact.from_device_id.clone()
        } else {
            ack.from_device_id.trim().to_string()
        };
        ack.from_device_name = if ack.from_device_name.trim().is_empty() {
            artifact.from_device_name.clone()
        } else {
            ack.from_device_name.trim().to_string()
        };
        ack.message = ack.message.trim().to_string();
        if ack.acknowledged_at_unix_ms == 0 {
            ack.acknowledged_at_unix_ms = now_unix_ms().max(0) as u64;
        }
        self.module_session_receipts.retain(|existing| {
            !(existing.module_id == ack.module_id
                && existing.session_id == ack.session_id
                && existing.session_revision == ack.session_revision
                && existing.from_device_id == ack.from_device_id)
        });
        self.module_session_receipts.insert(0, ack.clone());
        self.module_session_receipts.truncate(64);
        Ok(ack)
    }

    fn dismiss_received_workflow_state(&mut self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)?;
        self.refresh_received_workflow_inbox();
        self.networking_status = "Networking: dismissed a received workflow state.".to_string();
        Ok(())
    }

    fn render_received_workflow_inbox(
        &mut self,
        ui: &mut egui::Ui,
        heading: &str,
        module_filter: Option<&str>,
    ) {
        self.sync_selected_received_workflow();
        let filtered_items = self
            .received_workflow_inbox
            .iter()
            .filter(|item| {
                module_filter.is_none_or(|module_id| item.record.module_id.trim() == module_id)
            })
            .cloned()
            .collect::<Vec<_>>();

        ui.heading(heading);
        ui.label(
            "Shared workflows land here first so you can preview them before applying them into a module.",
        );
        ui.horizontal(|ui| {
            if ui.button("Refresh inbox").clicked() {
                self.refresh_received_workflow_inbox();
            }
            ui.small(format!("{} item(s) waiting", filtered_items.len()));
        });

        if filtered_items.is_empty() {
            ui.small("No received workflow states are waiting right now.");
            return;
        }

        let filtered_paths = filtered_items
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        if self
            .selected_received_workflow
            .as_ref()
            .is_none_or(|path| !filtered_paths.iter().any(|candidate| candidate == path))
        {
            self.selected_received_workflow = filtered_paths.first().cloned();
        }

        let selected_item = self
            .selected_received_workflow
            .as_ref()
            .and_then(|path| {
                filtered_items
                    .iter()
                    .find(|item| &item.path == path)
                    .cloned()
            })
            .or_else(|| filtered_items.first().cloned());

        ui.columns(2, |cols| {
            cols[0].vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt((heading, "workflow_inbox_list"))
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for item in &filtered_items {
                            let selected = self
                                .selected_received_workflow
                                .as_ref()
                                .is_some_and(|path| path == &item.path);
                            let title = if item.record.label.trim().is_empty() {
                                item.record.module_id.clone()
                            } else {
                                item.record.label.clone()
                            };
                            let module_label = self
                                .module_registry
                                .modules
                                .iter()
                                .find(|module| module.module_id == item.record.module_id)
                                .map(|module| module.display_name.clone())
                                .unwrap_or_else(|| item.record.module_id.clone());
                            ui.group(|ui| {
                                if ui
                                    .selectable_label(selected, title)
                                    .on_hover_text(item.path.display().to_string())
                                    .clicked()
                                {
                                    self.selected_received_workflow = Some(item.path.clone());
                                }
                                ui.small(format!(
                                    "{} | from {}",
                                    module_label, item.record.from_device_name
                                ));
                                if !item.record.summary.trim().is_empty() {
                                    ui.small(one_line(item.record.summary.trim(), 120));
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });

            cols[1].vertical(|ui| {
                if let Some(item) = selected_item {
                    let record = item.record.clone();
                    let path = item.path.clone();
                    let module = self
                        .module_registry
                        .modules
                        .iter()
                        .find(|module| module.module_id == record.module_id)
                        .cloned();
                    let module_label = module
                        .as_ref()
                        .map(|module| module.display_name.clone())
                        .unwrap_or_else(|| record.module_id.clone());
                    let title = if record.label.trim().is_empty() {
                        format!("Workflow for {}", module_label)
                    } else {
                        record.label.clone()
                    };
                    ui.label(egui::RichText::new(title).strong());
                    ui.small(format!(
                        "Module: {} | From {} ({})",
                        module_label, record.from_device_name, record.from_device_id
                    ));
                    if !record.shared_state.session_id.trim().is_empty() {
                        ui.small(format!(
                            "Session {} | revision {}{}",
                            record.shared_state.session_id,
                            record.shared_state.session_revision,
                            if record.shared_state.host_authoritative {
                                " | host-authoritative"
                            } else {
                                ""
                            }
                        ));
                    }
                    if module.is_none() {
                        ui.colored_label(
                            egui::Color32::from_rgb(160, 90, 40),
                            "This target module is not installed here yet. Keep the workflow in the inbox until the module is available.",
                        );
                    } else if !module_allows_network_feature(
                        module.as_ref(),
                        ModuleNetworkFeature::SharedStateReceive,
                    ) {
                        ui.colored_label(
                            egui::Color32::from_rgb(160, 90, 40),
                            "This module has not declared `shared_state_receive` support yet, so ChattyCog will keep this workflow in the inbox.",
                        );
                    } else if let Some(reason) =
                        self.stale_module_state_message(&module.as_ref().unwrap().dir, &record)
                    {
                        ui.colored_label(
                            egui::Color32::from_rgb(160, 90, 40),
                            format!("This looks stale against the currently applied session state: {reason}"),
                        );
                    }
                    if !record.summary.trim().is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Summary").strong());
                        ui.label(record.summary.trim());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let can_apply = module.is_some()
                            && module_allows_network_feature(
                                module.as_ref(),
                                ModuleNetworkFeature::SharedStateReceive,
                            );
                        if ui
                            .add_enabled(can_apply, egui::Button::new("Apply workflow now"))
                            .clicked()
                        {
                            if let Err(err) = self.accept_received_workflow_state(&path) {
                                self.networking_status =
                                    format!("Networking: could not apply workflow: {err}");
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            if let Err(err) = self.dismiss_received_workflow_state(&path) {
                                self.networking_status =
                                    format!("Networking: could not dismiss workflow: {err}");
                            }
                        }
                        if ui.button("Open file").clicked() {
                            open_path_in_explorer(&path);
                        }
                    });

                    if !record.shared_state.payload.is_null() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Payload preview").strong());
                        let mut payload =
                            serde_json::to_string_pretty(&record.shared_state.payload).unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt((heading, "workflow_inbox_payload"))
                            .max_height(180.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut payload)
                                        .desired_rows(8)
                                        .interactive(false),
                                );
                            });
                    }
                } else {
                    ui.small("Select a received workflow to preview it.");
                }
            });
        });
    }

    fn store_received_lukewarm_context(
        &mut self,
        artifact: &ReceivedArtifact,
    ) -> std::io::Result<PathBuf> {
        let mut context: SharedLukewarmContext =
            serde_json::from_str(&artifact.text).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("lukewarm context parse error: {err}"),
                )
            })?;
        context.label = context.label.trim().to_string();
        context.summary = context.summary.trim().to_string();
        context.context_text = context.context_text.trim().to_string();
        if context.source_device_id.trim().is_empty() {
            context.source_device_id = artifact.from_device_id.clone();
        }
        if context.source_device_name.trim().is_empty() {
            context.source_device_name = artifact.from_device_name.clone();
        }
        if context.created_at_unix_ms == 0 {
            context.created_at_unix_ms = now_unix_ms().max(0) as u64;
        }

        let dir = self.received_lukewarm_inbox_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "{}__{}__{}.json",
            slugify_filename(
                if context.label.trim().is_empty() {
                    "lukewarm_context"
                } else {
                    context.label.trim()
                },
                "lukewarm_context"
            ),
            slugify_filename(&artifact.from_device_name, "peer"),
            now_unix_ms().max(0)
        ));
        let record = ReceivedLukewarmContextRecord {
            artifact_id: artifact.artifact_id.clone(),
            from_device_id: artifact.from_device_id.clone(),
            from_device_name: artifact.from_device_name.clone(),
            label: artifact.label.clone(),
            summary: artifact.summary.clone(),
            file_name: artifact.file_name.clone(),
            received_at_unix_ms: now_unix_ms().max(0) as u64,
            context,
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("lukewarm inbox serialize error: {err}"),
            )
        })?;
        std::fs::write(&path, bytes)?;
        self.refresh_received_lukewarm_inbox();
        self.selected_received_lukewarm = Some(path.clone());
        self.networking_status = format!(
            "Networking: received luke warm context from {} and saved it to the inbox.",
            artifact.from_device_name
        );
        Ok(path)
    }

    fn accept_received_lukewarm_context(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        let record = read_received_lukewarm_record(path)?;
        let dir = self.applied_lukewarm_dir();
        std::fs::create_dir_all(&dir)?;

        for existing in load_applied_lukewarm_contexts(&dir).unwrap_or_default() {
            if existing.record.from_device_id.trim() == record.from_device_id.trim()
                && existing.path != path
            {
                let _ = std::fs::remove_file(existing.path);
            }
        }

        let dest = dir.join(format!(
            "{}__{}.json",
            slugify_filename(
                if record.from_device_name.trim().is_empty() {
                    &record.from_device_id
                } else {
                    &record.from_device_name
                },
                "peer"
            ),
            slugify_filename(
                if record.context.label.trim().is_empty() {
                    "lukewarm_context"
                } else {
                    record.context.label.trim()
                },
                "lukewarm_context"
            )
        ));
        let bytes = serde_json::to_vec_pretty(&record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("lukewarm apply serialize error: {err}"),
            )
        })?;
        std::fs::write(&dest, bytes)?;
        std::fs::remove_file(path)?;
        self.refresh_received_lukewarm_inbox();
        self.networking_status = format!(
            "Networking: applied shared luke warm context from {}.",
            record.from_device_name
        );
        push_hot_memory(
            self,
            format!(
                "Shared luke warm from {} applied: {}",
                record.from_device_name,
                one_line(
                    if record.summary.trim().is_empty() {
                        &record.context.summary
                    } else {
                        &record.summary
                    },
                    120
                )
            ),
        );
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Module,
                source: "network".to_string(),
                module: Some("lukewarm_context".to_string()),
                event_type: Some("lukewarm_context_applied".to_string()),
                text: format!(
                    "Applied shared luke warm context from {}.\nSaved to {}\n\nSummary: {}",
                    record.from_device_name,
                    dest.display(),
                    if record.summary.trim().is_empty() {
                        &record.context.summary
                    } else {
                        &record.summary
                    }
                ),
                tags: vec![
                    "lan".to_string(),
                    "lukewarm".to_string(),
                    "shared_context".to_string(),
                    "applied".to_string(),
                ],
                entities: vec![record.from_device_name.clone()],
                payload_json: Some(
                    serde_json::json!({
                        "source_app": record.context.source_app,
                        "context_text": record.context.context_text,
                    })
                    .to_string(),
                ),
            });
        }
        Ok(dest)
    }

    fn dismiss_received_lukewarm_context(&mut self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)?;
        self.refresh_received_lukewarm_inbox();
        self.networking_status = "Networking: dismissed a received luke warm context.".to_string();
        Ok(())
    }

    fn render_received_lukewarm_inbox(&mut self, ui: &mut egui::Ui, heading: &str) {
        self.sync_selected_received_lukewarm();
        ui.heading(heading);
        ui.label(
            "Shared luke warm context lands here first so you can preview it before making it part of this device's network-aware memory context.",
        );
        ui.horizontal(|ui| {
            if ui.button("Refresh inbox").clicked() {
                self.refresh_received_lukewarm_inbox();
            }
            let applied_count = load_applied_lukewarm_contexts(&self.applied_lukewarm_dir())
                .unwrap_or_default()
                .len();
            ui.small(format!(
                "{} waiting | {} applied",
                self.received_lukewarm_inbox.len(),
                applied_count
            ));
        });

        if self.received_lukewarm_inbox.is_empty() {
            ui.small("No shared luke warm context is waiting right now.");
            return;
        }

        let selected_path = self.selected_received_lukewarm.clone().or_else(|| {
            self.received_lukewarm_inbox
                .first()
                .map(|item| item.path.clone())
        });
        let selected_item = selected_path.as_ref().and_then(|path| {
            self.received_lukewarm_inbox
                .iter()
                .find(|item| &item.path == path)
                .cloned()
        });

        ui.columns(2, |cols| {
            cols[0].vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt((heading, "lukewarm_inbox_list"))
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for item in &self.received_lukewarm_inbox {
                            let selected = self
                                .selected_received_lukewarm
                                .as_ref()
                                .is_some_and(|path| path == &item.path);
                            let title = if item.record.label.trim().is_empty() {
                                item.record.context.label.clone()
                            } else {
                                item.record.label.clone()
                            };
                            ui.group(|ui| {
                                if ui
                                    .selectable_label(selected, title)
                                    .on_hover_text(item.path.display().to_string())
                                    .clicked()
                                {
                                    self.selected_received_lukewarm = Some(item.path.clone());
                                }
                                ui.small(format!(
                                    "from {}",
                                    item.record.from_device_name
                                ));
                                if !item.record.context.summary.trim().is_empty() {
                                    ui.small(one_line(item.record.context.summary.trim(), 120));
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });

            cols[1].vertical(|ui| {
                if let Some(item) = selected_item {
                    let record = item.record.clone();
                    let path = item.path.clone();
                    let title = if record.label.trim().is_empty() {
                        record.context.label.clone()
                    } else {
                        record.label.clone()
                    };
                    ui.label(egui::RichText::new(title).strong());
                    ui.small(format!(
                        "From {} ({}) | Source app: {}",
                        record.from_device_name,
                        record.from_device_id,
                        record.context.source_app
                    ));
                    if !record.summary.trim().is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Summary").strong());
                        ui.label(record.summary.trim());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Apply to shared memory").clicked() {
                            if let Err(err) = self.accept_received_lukewarm_context(&path) {
                                self.networking_status =
                                    format!("Networking: could not apply luke warm context: {err}");
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            if let Err(err) = self.dismiss_received_lukewarm_context(&path) {
                                self.networking_status =
                                    format!("Networking: could not dismiss luke warm context: {err}");
                            }
                        }
                        if ui.button("Open file").clicked() {
                            open_path_in_explorer(&path);
                        }
                    });
                    if !self.prefs.network_allow_shared_lukewarm_context {
                        ui.colored_label(
                            egui::Color32::from_rgb(160, 90, 40),
                            "Shared luke warm context is currently stored but not injected into prompts because `Allow shared luke warm context` is turned off.",
                        );
                    }
                    ui.add_space(8.0);
                    let mut preview = record.context.context_text.clone();
                    egui::ScrollArea::vertical()
                        .id_salt((heading, "lukewarm_inbox_preview"))
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut preview)
                                    .desired_rows(10)
                                    .interactive(false),
                            );
                        });
                } else {
                    ui.small("Select a received luke warm context to preview it.");
                }
            });
        });
    }

    fn store_received_workflow_bundle(
        &mut self,
        artifact: &ReceivedArtifact,
    ) -> std::io::Result<PathBuf> {
        let bundle: WorkflowBundle = serde_json::from_str(&artifact.text).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("workflow bundle parse error: {err}"),
            )
        })?;
        let dir = self.received_bundle_inbox_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!(
            "{}__{}__{}.json",
            slugify_filename(
                if artifact.label.trim().is_empty() {
                    "workflow_bundle"
                } else {
                    artifact.label.trim()
                },
                "workflow_bundle"
            ),
            slugify_filename(&artifact.from_device_name, "peer"),
            now_unix_ms().max(0)
        ));
        let record = ReceivedWorkflowBundleRecord {
            artifact_id: artifact.artifact_id.clone(),
            from_device_id: artifact.from_device_id.clone(),
            from_device_name: artifact.from_device_name.clone(),
            label: artifact.label.clone(),
            summary: artifact.summary.clone(),
            file_name: artifact.file_name.clone(),
            received_at_unix_ms: now_unix_ms().max(0) as u64,
            bundle,
        };
        let bytes = serde_json::to_vec_pretty(&record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("workflow bundle serialize error: {err}"),
            )
        })?;
        std::fs::write(&path, bytes)?;
        self.refresh_received_bundle_inbox();
        self.selected_received_bundle = Some(path.clone());
        self.networking_status = format!(
            "Networking: received a workflow bundle from {} and saved it to the inbox.",
            artifact.from_device_name
        );
        Ok(path)
    }

    fn persist_received_generic_transfer_record(
        &self,
        path: &Path,
        record: &ReceivedGenericTransferRecord,
    ) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("generic transfer serialize error: {err}"),
            )
        })?;
        std::fs::write(path, bytes)
    }

    fn matching_module_asset_lanes_for_transfer(
        &self,
        module_id: &str,
        kind: &str,
        content_type: &str,
        byte_len: u64,
    ) -> Vec<ModuleNetworkAssetLane> {
        self.module_manifest_by_id(module_id)
            .and_then(|manifest| manifest.network_capabilities)
            .map(|caps| {
                caps.matching_receive_asset_lanes(kind, content_type, byte_len)
                    .into_iter()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn deliver_generic_transfer_record_to_lane(
        &mut self,
        record: &mut ReceivedGenericTransferRecord,
        payload_bytes: &[u8],
        lane: &ModuleNetworkAssetLane,
    ) -> std::io::Result<PathBuf> {
        let module = self
            .module_manifest_by_id(&record.module_id)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("module `{}` is not available", record.module_id),
                )
            })?;
        let delivered_at_unix_ms = now_unix_ms().max(0) as u64;
        let bridge_record = ModuleBridgeIncomingAssetRecord {
            asset_id: format!(
                "{}-{}",
                record.artifact_id.trim(),
                slugify_filename(&lane.lane_id, "lane")
            ),
            artifact_id: record.artifact_id.clone(),
            module_id: record.module_id.clone(),
            lane_id: lane.lane_id.clone(),
            lane_label: lane.label.clone(),
            kind: record.kind.clone(),
            label: record.label.clone(),
            summary: record.summary.clone(),
            file_name: record.file_name.clone(),
            content_type: record.content_type.clone(),
            transfer_encoding: record.transfer_encoding.clone(),
            byte_len: record.byte_len,
            chunk_count: record.chunk_count,
            binary: record.binary,
            from_device_id: record.from_device_id.clone(),
            from_device_name: record.from_device_name.clone(),
            delivered_at_unix_ms,
            payload_file_name: record.payload_file_name.clone(),
        };
        let bridge_record_path =
            write_bridge_incoming_asset(&module.dir, &lane.lane_id, &bridge_record, payload_bytes)
                .map_err(|err| std::io::Error::other(err.to_string()))?;
        record.delivered_lanes.retain(|entry| {
            !entry
                .lane_id
                .trim()
                .eq_ignore_ascii_case(lane.lane_id.trim())
        });
        record
            .delivered_lanes
            .push(ReceivedGenericTransferLaneDelivery {
                lane_id: lane.lane_id.clone(),
                lane_label: lane.label.clone(),
                delivered_at_unix_ms,
                bridge_record_path: bridge_record_path.display().to_string(),
            });
        record
            .delivered_lanes
            .sort_by(|left, right| right.delivered_at_unix_ms.cmp(&left.delivered_at_unix_ms));
        if lane.replayable {
            self.remember_recoverable_module_asset(
                &record.kind,
                if record.label.trim().is_empty() {
                    &lane.label
                } else {
                    &record.label
                },
                &record.module_id,
                if record.summary.trim().is_empty() {
                    &lane.label
                } else {
                    &record.summary
                },
                if record.file_name.trim().is_empty() {
                    &record.payload_file_name
                } else {
                    &record.file_name
                },
                &record.content_type,
                payload_bytes,
                record.binary,
            );
        }
        Ok(bridge_record_path)
    }

    fn deliver_received_generic_transfer_to_lane(
        &mut self,
        path: &Path,
        lane_id: &str,
    ) -> std::io::Result<PathBuf> {
        let mut record = read_received_generic_transfer_record(path)?;
        if record.module_id.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "this transfer is not scoped to a module",
            ));
        }
        let lane = self
            .matching_module_asset_lanes_for_transfer(
                &record.module_id,
                &record.kind,
                &record.content_type,
                record.byte_len,
            )
            .into_iter()
            .find(|lane| lane.lane_id.trim() == lane_id.trim())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "no matching incoming asset lane `{}` is declared for {}",
                        lane_id, record.module_id
                    ),
                )
            })?;
        let payload_path = self
            .received_transfer_payload_dir()
            .join(record.payload_file_name.clone());
        if !payload_path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("transfer payload missing: {}", payload_path.display()),
            ));
        }
        let payload_bytes = std::fs::read(&payload_path)?;
        let bridge_record_path =
            self.deliver_generic_transfer_record_to_lane(&mut record, &payload_bytes, &lane)?;
        self.persist_received_generic_transfer_record(path, &record)?;
        self.refresh_received_transfer_inbox();
        self.selected_received_transfer = Some(path.to_path_buf());
        self.networking_status = format!(
            "Networking: delivered `{}` from {} into {} -> {}.",
            if record.label.trim().is_empty() {
                record.kind.trim()
            } else {
                record.label.trim()
            },
            record.from_device_name,
            lane.label.trim(),
            bridge_record_path.display()
        );
        push_hot_memory(
            self,
            format!(
                "Delivered network transfer into module lane for {}: {}",
                record.module_id,
                one_line(
                    if record.summary.trim().is_empty() {
                        if record.label.trim().is_empty() {
                            &record.kind
                        } else {
                            &record.label
                        }
                    } else {
                        &record.summary
                    },
                    120
                )
            ),
        );
        Ok(bridge_record_path)
    }

    fn store_received_generic_transfer(
        &mut self,
        artifact: &ReceivedArtifact,
    ) -> std::io::Result<PathBuf> {
        let payload_bytes = artifact.decoded_bytes().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transfer payload could not be decoded",
            )
        })?;

        let inbox_dir = self.received_transfer_inbox_dir();
        let payload_dir = self.received_transfer_payload_dir();
        std::fs::create_dir_all(&inbox_dir)?;
        std::fs::create_dir_all(&payload_dir)?;

        let safe_sender = slugify_filename(&artifact.from_device_name, "peer");
        let safe_label = slugify_filename(
            if artifact.label.trim().is_empty() {
                "network_transfer"
            } else {
                artifact.label.trim()
            },
            "network_transfer",
        );
        let stamp = now_unix_ms().max(0);
        let payload_file_name = format!(
            "{}__{}__{}.{}",
            safe_sender,
            safe_label,
            stamp,
            infer_transfer_extension(
                &artifact.file_name,
                &artifact.content_type,
                artifact.is_binary(),
            )
        );
        let payload_path = payload_dir.join(&payload_file_name);
        std::fs::write(&payload_path, &payload_bytes)?;

        let record_path =
            inbox_dir.join(format!("{}__{}__{}.json", safe_sender, safe_label, stamp));
        let mut record = ReceivedGenericTransferRecord {
            artifact_id: artifact.artifact_id.clone(),
            from_device_id: artifact.from_device_id.clone(),
            from_device_name: artifact.from_device_name.clone(),
            label: artifact.label.clone(),
            summary: artifact.summary.clone(),
            kind: artifact.kind.clone(),
            module_id: artifact.module_id.clone(),
            file_name: artifact.file_name.clone(),
            content_type: artifact.content_type.clone(),
            transfer_encoding: artifact.transfer_encoding.clone(),
            byte_len: artifact.byte_len,
            chunk_count: artifact.chunk_count,
            received_at_unix_ms: stamp as u64,
            binary: artifact.is_binary(),
            payload_file_name,
            preview_text: if artifact.is_binary() {
                String::new()
            } else {
                clip_string_for_preview(&artifact.text, 4_000)
            },
            delivered_lanes: Vec::new(),
        };
        let auto_delivered_path = if record.module_id.trim().is_empty() {
            None
        } else {
            let mut lanes = self
                .matching_module_asset_lanes_for_transfer(
                    &record.module_id,
                    &record.kind,
                    &record.content_type,
                    record.byte_len,
                )
                .into_iter()
                .filter(|lane| {
                    lane.delivery_mode
                        == chattycog_gui::module_registry::ModuleAssetDeliveryMode::BridgeInbox
                })
                .collect::<Vec<_>>();
            if lanes.len() == 1 {
                Some(self.deliver_generic_transfer_record_to_lane(
                    &mut record,
                    &payload_bytes,
                    &lanes.remove(0),
                )?)
            } else {
                None
            }
        };
        self.persist_received_generic_transfer_record(&record_path, &record)?;
        self.refresh_received_transfer_inbox();
        self.selected_received_transfer = Some(record_path.clone());
        self.networking_status = if let Some(delivered_path) = auto_delivered_path {
            let lane_label = record
                .delivered_lanes
                .first()
                .map(|lane| lane.lane_label.as_str())
                .unwrap_or("module lane");
            format!(
                "Networking: received `{}` from {} and delivered it into {} at {}.",
                if artifact.label.trim().is_empty() {
                    artifact.kind.trim()
                } else {
                    artifact.label.trim()
                },
                artifact.from_device_name,
                lane_label,
                delivered_path.display()
            )
        } else {
            format!(
                "Networking: received `{}` from {} and saved it to the transfer inbox.",
                if artifact.label.trim().is_empty() {
                    artifact.kind.trim()
                } else {
                    artifact.label.trim()
                },
                artifact.from_device_name
            )
        };
        Ok(record_path)
    }

    fn accept_received_generic_transfer(&mut self, path: &Path) -> std::io::Result<PathBuf> {
        let record = read_received_generic_transfer_record(path)?;
        let payload_path = self
            .received_transfer_payload_dir()
            .join(record.payload_file_name.clone());
        if !payload_path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("transfer payload missing: {}", payload_path.display()),
            ));
        }

        let dest_dir = self.applied_transfer_dir();
        std::fs::create_dir_all(&dest_dir)?;
        let dest_file_name = if record.file_name.trim().is_empty() {
            record.payload_file_name.clone()
        } else {
            format!(
                "{}__{}",
                slugify_filename(&record.from_device_name, "peer"),
                sanitize_filename_keep_extension(&record.file_name)
            )
        };
        let dest_path = unique_path_in_dir(&dest_dir, &dest_file_name);
        std::fs::copy(&payload_path, &dest_path)?;

        let sidecar_path = dest_dir.join(format!(
            "{}.meta.json",
            dest_path.file_name().unwrap_or_default().to_string_lossy()
        ));
        let sidecar = serde_json::to_vec_pretty(&record).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("generic transfer meta serialize error: {err}"),
            )
        })?;
        std::fs::write(&sidecar_path, sidecar)?;
        let _ = std::fs::remove_file(&payload_path);
        std::fs::remove_file(path)?;
        self.refresh_received_transfer_inbox();
        self.networking_status = format!(
            "Networking: imported `{}` from {} into {}.",
            if record.label.trim().is_empty() {
                record.kind.trim()
            } else {
                record.label.trim()
            },
            record.from_device_name,
            dest_path.display()
        );
        push_hot_memory(
            self,
            format!(
                "Imported network transfer from {}: {}",
                record.from_device_name,
                one_line(
                    if record.summary.trim().is_empty() {
                        if record.label.trim().is_empty() {
                            &record.kind
                        } else {
                            &record.label
                        }
                    } else {
                        &record.summary
                    },
                    120
                )
            ),
        );
        Ok(dest_path)
    }

    fn dismiss_received_generic_transfer(&mut self, path: &Path) -> std::io::Result<()> {
        let record = read_received_generic_transfer_record(path)?;
        let payload_path = self
            .received_transfer_payload_dir()
            .join(record.payload_file_name);
        let _ = std::fs::remove_file(payload_path);
        std::fs::remove_file(path)?;
        self.refresh_received_transfer_inbox();
        self.networking_status =
            "Networking: dismissed a received file-style transfer.".to_string();
        Ok(())
    }

    fn render_received_generic_transfer_inbox(&mut self, ui: &mut egui::Ui, heading: &str) {
        self.sync_selected_received_transfer();
        ui.heading(heading);
        ui.label(
            "Unknown, file-style, or binary transfers land here first so you can inspect them before importing them into this machine.",
        );
        ui.horizontal(|ui| {
            if ui.button("Refresh inbox").clicked() {
                self.refresh_received_transfer_inbox();
            }
            ui.small(format!(
                "{} transfer(s) waiting",
                self.received_transfer_inbox.len()
            ));
        });

        if self.received_transfer_inbox.is_empty() {
            ui.small("No generic file-style transfers are waiting right now.");
            return;
        }

        let selected_item = self
            .selected_received_transfer
            .as_ref()
            .and_then(|path| {
                self.received_transfer_inbox
                    .iter()
                    .find(|item| &item.path == path)
                    .cloned()
            })
            .or_else(|| self.received_transfer_inbox.first().cloned());

        ui.columns(2, |cols| {
            cols[0].vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt((heading, "generic_transfer_inbox_list"))
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for item in &self.received_transfer_inbox {
                            let selected = self
                                .selected_received_transfer
                                .as_ref()
                                .is_some_and(|path| path == &item.path);
                            let title = if item.record.label.trim().is_empty() {
                                item.record.kind.clone()
                            } else {
                                item.record.label.clone()
                            };
                            ui.group(|ui| {
                                if ui
                                    .selectable_label(selected, title)
                                    .on_hover_text(item.path.display().to_string())
                                    .clicked()
                                {
                                    self.selected_received_transfer = Some(item.path.clone());
                                }
                                ui.small(format!(
                                    "{} | {}",
                                    item.record.from_device_name,
                                    format_network_transfer_meta(
                                        &item.record.content_type,
                                        &item.record.transfer_encoding,
                                        item.record.byte_len,
                                        item.record.chunk_count,
                                    )
                                ));
                                if !item.record.summary.trim().is_empty() {
                                    ui.small(clip_string_for_preview(
                                        item.record.summary.trim(),
                                        120,
                                    ));
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });

            cols[1].vertical(|ui| {
                if let Some(item) = selected_item {
                    let record = item.record.clone();
                    let path = item.path.clone();
                    let payload_path = self
                        .received_transfer_payload_dir()
                        .join(record.payload_file_name.clone());
                    let module_manifest = self.module_manifest_by_id(&record.module_id);
                    let matching_lanes = if record.module_id.trim().is_empty() {
                        Vec::new()
                    } else {
                        self.matching_module_asset_lanes_for_transfer(
                            &record.module_id,
                            &record.kind,
                            &record.content_type,
                            record.byte_len,
                        )
                    };
                    ui.label(
                        egui::RichText::new(if record.label.trim().is_empty() {
                            record.kind.clone()
                        } else {
                            record.label.clone()
                        })
                        .strong(),
                    );
                    ui.small(format!(
                        "From {} ({})",
                        record.from_device_name, record.from_device_id
                    ));
                    if !record.module_id.trim().is_empty() {
                        ui.small(format!("Module: {}", record.module_id));
                    }
                    if !record.file_name.trim().is_empty() {
                        ui.small(format!("Original file: {}", record.file_name));
                    }
                    ui.small(format_network_transfer_meta(
                        &record.content_type,
                        &record.transfer_encoding,
                        record.byte_len,
                        record.chunk_count,
                    ));
                    ui.small(format!("Inbox record: {}", path.display()));
                    ui.small(format!("Payload file: {}", payload_path.display()));
                    if !record.summary.trim().is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Summary").strong());
                        ui.label(record.summary.trim());
                    }
                    if !record.module_id.trim().is_empty() {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Module asset lanes").strong());
                        if !matching_lanes.is_empty() {
                            ui.small(format!(
                                "{} declared {} matching incoming lane(s) for this transfer.",
                                module_manifest
                                    .as_ref()
                                    .map(|module| module.display_name.as_str())
                                    .unwrap_or(record.module_id.as_str()),
                                matching_lanes.len()
                            ));
                            for lane in &matching_lanes {
                                let delivered = record
                                    .delivered_lanes
                                    .iter()
                                    .find(|entry| entry.lane_id.trim() == lane.lane_id.trim());
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(lane.label.trim());
                                        ui.small(format!(
                                            "[{} | {}]",
                                            lane.lane_id,
                                            lane.delivery_mode.label()
                                        ));
                                    });
                                    let mut meta = vec![lane.direction.label().to_string()];
                                    if !lane.artifact_kinds.is_empty() {
                                        meta.push(format!("kinds: {}", lane.artifact_kinds.join(", ")));
                                    }
                                    if !lane.accepted_content_types.is_empty() {
                                        meta.push(format!(
                                            "content: {}",
                                            lane.accepted_content_types.join(", ")
                                        ));
                                    }
                                    if let Some(max_bytes) = lane.max_bytes {
                                        meta.push(format!(
                                            "max {}",
                                            format_network_transfer_size(max_bytes)
                                        ));
                                    }
                                    if lane.replayable {
                                        meta.push("replayable".to_string());
                                    }
                                    ui.small(meta.join(" | "));
                                    if let Some(delivered) = delivered {
                                        ui.small(format!(
                                            "Delivered here at {} -> {}",
                                            delivered.delivered_at_unix_ms,
                                            delivered.bridge_record_path
                                        ));
                                    }
                                    ui.horizontal_wrapped(|ui| {
                                        let button_label = if delivered.is_some() {
                                            "Re-deliver to lane"
                                        } else {
                                            "Deliver to lane"
                                        };
                                        if ui.button(button_label).clicked() {
                                            if let Err(err) = self
                                                .deliver_received_generic_transfer_to_lane(
                                                    &path,
                                                    &lane.lane_id,
                                                )
                                            {
                                                self.networking_status = format!(
                                                    "Networking: could not deliver that transfer to {}: {}",
                                                    lane.label, err
                                                );
                                            }
                                        }
                                        if let Some(module_manifest) = &module_manifest {
                                            if ui.button("Open lane").clicked() {
                                                open_path_in_explorer(&bridge_incoming_asset_lane_dir(
                                                    &module_manifest.dir,
                                                    &lane.lane_id,
                                                ));
                                            }
                                        }
                                    });
                                    for note in &lane.notes {
                                        ui.small(format!("Note: {}", note));
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        } else {
                            ui.small(
                                "This module did not declare a matching incoming asset lane for this transfer, so ChattyCog is keeping it in the generic inbox.",
                            );
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Import to local files").clicked() {
                            if let Err(err) = self.accept_received_generic_transfer(&path) {
                                self.networking_status = format!(
                                    "Networking: could not import that transfer: {}",
                                    err
                                );
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            if let Err(err) = self.dismiss_received_generic_transfer(&path) {
                                self.networking_status = format!(
                                    "Networking: could not dismiss that transfer: {}",
                                    err
                                );
                            }
                        }
                        if ui.button("Open payload").clicked() {
                            open_path_in_explorer(&payload_path);
                        }
                        if ui.button("Open imports").clicked() {
                            open_path_in_explorer(&self.applied_transfer_dir());
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Preview").strong());
                    if record.binary {
                        ui.small(
                            "This transfer is binary/file-style, so ChattyCog is only showing metadata here. Import it to the local files area or open the payload directly.",
                        );
                    } else {
                        let mut preview = record.preview_text.clone();
                        egui::ScrollArea::vertical()
                            .id_salt((heading, "generic_transfer_inbox_preview"))
                            .max_height(220.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut preview)
                                        .desired_rows(10)
                                        .interactive(false),
                                );
                            });
                    }
                } else {
                    ui.small("Select a received transfer to preview it.");
                }
            });
        });
    }

    fn accept_received_workflow_bundle(&mut self, path: &Path) -> std::io::Result<()> {
        let record = read_received_workflow_bundle_record(path)?;
        let bundle = record.bundle.clone();

        self.prefs.orchestrator = bundle.orchestrator_params.clone();
        self.prefs.bookkeeper = bundle.bookkeeper_params.clone();
        self.prefs.allow_sandbox_tool_requests = bundle.allow_sandbox_tool_requests;
        self.prefs.auto_generate_module_suspend_rundown =
            bundle.auto_generate_module_suspend_rundown;
        self.prefs.modules = bundle.module_preferences.clone();
        self.apply_prefs_to_runtime_settings();

        if !bundle.system_prompt.trim().is_empty() {
            if let Some(system_message) = self
                .messages
                .iter_mut()
                .find(|message| matches!(message.role, Role::System))
            {
                system_message.content = bundle.system_prompt.clone();
            } else {
                self.messages.insert(
                    0,
                    Message {
                        role: Role::System,
                        content: bundle.system_prompt.clone(),
                        thinking: None,
                    },
                );
            }
        }

        let mut notes = Vec::new();
        if let Some(path) =
            self.resolve_portable_model_hint(bundle.orchestrator_model_hint.as_deref())
        {
            self.gguf_path = Some(path.clone());
            notes.push(format!(
                "orchestrator model -> {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            ));
        } else if let Some(hint) = bundle.orchestrator_model_hint.as_deref() {
            notes.push(format!("orchestrator model missing locally ({hint})"));
        }

        if let Some(path) =
            self.resolve_portable_model_hint(bundle.bookkeeper_model_hint.as_deref())
        {
            self.bookkeeper_model_path = Some(path.clone());
            self.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(300));
            notes.push(format!(
                "bookkeeper model -> {}",
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string())
            ));
        } else if let Some(hint) = bundle.bookkeeper_model_hint.as_deref() {
            notes.push(format!("bookkeeper model missing locally ({hint})"));
        }

        for (module_id, pref) in &bundle.module_preferences {
            let resolved_model = pref
                .preferred_model
                .as_deref()
                .and_then(|hint| self.resolve_portable_model_hint(Some(hint)));
            if let Some(state) = self.module_ai.get_mut(module_id) {
                state.temp = pref.params.temp;
                state.top_p = pref.params.top_p;
                state.top_k = pref.params.top_k;
                state.max_tokens = pref.params.max_tokens;
                if let Some(path) = resolved_model {
                    state.model_path = Some(path);
                }
            }
        }

        preferences::save_prefs(&self.prefs_path, &self.prefs)
            .map_err(|err| std::io::Error::other(format!("prefs save error: {err}")))?;
        std::fs::remove_file(path)?;
        self.refresh_received_bundle_inbox();

        let summary_line = if record.summary.trim().is_empty() {
            if bundle.summary.trim().is_empty() {
                "Applied a shared workflow bundle.".to_string()
            } else {
                bundle.summary.trim().to_string()
            }
        } else {
            record.summary.trim().to_string()
        };
        self.networking_status = if notes.is_empty() {
            format!(
                "Networking: applied workflow bundle from {}.",
                record.from_device_name
            )
        } else {
            format!(
                "Networking: applied workflow bundle from {} ({})",
                record.from_device_name,
                notes.join(" | ")
            )
        };
        push_hot_memory(
            self,
            format!(
                "Workflow bundle from {} applied: {}",
                record.from_device_name,
                one_line(&summary_line, 120)
            ),
        );
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Module,
                source: "network".to_string(),
                module: Some("workflow_bundle".to_string()),
                event_type: Some("bundle_applied".to_string()),
                text: format!(
                    "Applied workflow bundle from {}.\n\nSummary: {}\nNotes: {}",
                    record.from_device_name,
                    summary_line,
                    if notes.is_empty() {
                        "(no model remap notes)".to_string()
                    } else {
                        notes.join(" | ")
                    }
                ),
                tags: vec![
                    "lan".to_string(),
                    "workflow".to_string(),
                    "bundle".to_string(),
                    "applied".to_string(),
                ],
                entities: vec![record.from_device_name.clone()],
                payload_json: serde_json::to_string(&bundle).ok(),
            });
        }

        Ok(())
    }

    fn dismiss_received_workflow_bundle(&mut self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)?;
        self.refresh_received_bundle_inbox();
        self.networking_status = "Networking: dismissed a received workflow bundle.".to_string();
        Ok(())
    }

    fn render_received_workflow_bundle_inbox(&mut self, ui: &mut egui::Ui, heading: &str) {
        self.sync_selected_received_bundle();
        ui.heading(heading);
        ui.label(
            "Shared setup bundles land here first so you can preview them before applying them to this ChattyCog instance.",
        );
        ui.horizontal(|ui| {
            if ui.button("Refresh inbox").clicked() {
                self.refresh_received_bundle_inbox();
            }
            ui.small(format!(
                "{} bundle(s) waiting",
                self.received_bundle_inbox.len()
            ));
        });

        if self.received_bundle_inbox.is_empty() {
            ui.small("No received workflow bundles are waiting right now.");
            return;
        }

        let selected_item = self
            .selected_received_bundle
            .as_ref()
            .and_then(|path| {
                self.received_bundle_inbox
                    .iter()
                    .find(|item| &item.path == path)
                    .cloned()
            })
            .or_else(|| self.received_bundle_inbox.first().cloned());

        ui.columns(2, |cols| {
            cols[0].vertical(|ui| {
                egui::ScrollArea::vertical()
                    .id_salt((heading, "workflow_bundle_inbox_list"))
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for item in &self.received_bundle_inbox {
                            let selected = self
                                .selected_received_bundle
                                .as_ref()
                                .is_some_and(|path| path == &item.path);
                            let title = if item.record.label.trim().is_empty() {
                                "Workflow bundle".to_string()
                            } else {
                                item.record.label.clone()
                            };
                            ui.group(|ui| {
                                if ui
                                    .selectable_label(selected, title)
                                    .on_hover_text(item.path.display().to_string())
                                    .clicked()
                                {
                                    self.selected_received_bundle = Some(item.path.clone());
                                }
                                ui.small(format!("From {}", item.record.from_device_name));
                                let summary = if item.record.summary.trim().is_empty() {
                                    item.record.bundle.summary.trim()
                                } else {
                                    item.record.summary.trim()
                                };
                                if !summary.is_empty() {
                                    ui.small(one_line(summary, 120));
                                }
                            });
                            ui.add_space(4.0);
                        }
                    });
            });

            cols[1].vertical(|ui| {
                if let Some(item) = selected_item {
                    let record = item.record.clone();
                    let path = item.path.clone();
                    let title = if record.label.trim().is_empty() {
                        "Workflow bundle".to_string()
                    } else {
                        record.label.clone()
                    };
                    ui.label(egui::RichText::new(title).strong());
                    ui.small(format!(
                        "From {} ({})",
                        record.from_device_name, record.from_device_id
                    ));
                    let summary = if record.summary.trim().is_empty() {
                        record.bundle.summary.trim()
                    } else {
                        record.summary.trim()
                    };
                    if !summary.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Summary").strong());
                        ui.label(summary);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Apply bundle now").clicked() {
                            if let Err(err) = self.accept_received_workflow_bundle(&path) {
                                self.networking_status =
                                    format!("Networking: could not apply workflow bundle: {err}");
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            if let Err(err) = self.dismiss_received_workflow_bundle(&path) {
                                self.networking_status =
                                    format!("Networking: could not dismiss workflow bundle: {err}");
                            }
                        }
                        if ui.button("Open file").clicked() {
                            open_path_in_explorer(&path);
                        }
                    });

                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Bundle preview").strong());
                    ui.small(format!(
                        "System prompt: {} chars | Module prefs: {}",
                        record.bundle.system_prompt.chars().count(),
                        record.bundle.module_preferences.len()
                    ));
                    ui.small(format!(
                        "Orchestrator model hint: {} | Bookkeeper model hint: {}",
                        record
                            .bundle
                            .orchestrator_model_hint
                            .as_deref()
                            .unwrap_or("(none)"),
                        record
                            .bundle
                            .bookkeeper_model_hint
                            .as_deref()
                            .unwrap_or("(none)")
                    ));
                    let mut payload =
                        serde_json::to_string_pretty(&record.bundle).unwrap_or_default();
                    egui::ScrollArea::vertical()
                        .id_salt((heading, "workflow_bundle_inbox_payload"))
                        .max_height(220.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut payload)
                                    .desired_rows(10)
                                    .interactive(false),
                            );
                        });
                } else {
                    ui.small("Select a received workflow bundle to preview it.");
                }
            });
        });
    }

    fn read_module_bridge_status(
        &mut self,
        module_id: &str,
        module_dir: &Path,
    ) -> Option<ModuleBridgeStatus> {
        match read_bridge_status(module_dir) {
            Ok(Some(status)) => {
                if !status.module_id.trim().is_empty() && status.module_id.trim() != module_id {
                    self.runtime_status = format!(
                        "Runtime: ignored bridge status for {module_id} (reported {}).",
                        status.module_id.trim()
                    );
                    return None;
                }
                Some(status)
            }
            Ok(None) => None,
            Err(err) => {
                self.runtime_status =
                    format!("Runtime: module bridge read warning for {module_id}: {err:#}");
                None
            }
        }
    }

    fn read_module_bridge_shared_state(
        &mut self,
        module_id: &str,
        module_dir: &Path,
    ) -> Option<ModuleBridgeSharedState> {
        match read_bridge_shared_state(module_dir) {
            Ok(Some(state)) => {
                if !state.module_id.trim().is_empty() && state.module_id.trim() != module_id {
                    self.runtime_status = format!(
                        "Runtime: ignored shared state for {module_id} (reported {}).",
                        state.module_id.trim()
                    );
                    return None;
                }
                Some(state)
            }
            Ok(None) => None,
            Err(err) => {
                self.runtime_status =
                    format!("Runtime: shared-state read warning for {module_id}: {err:#}");
                None
            }
        }
    }

    fn read_module_bridge_incoming_shared_state(
        &mut self,
        module_id: &str,
        module_dir: &Path,
    ) -> Option<ModuleBridgeIncomingSharedState> {
        match read_bridge_incoming_shared_state(module_dir) {
            Ok(Some(state)) => {
                if !state.module_id.trim().is_empty() && state.module_id.trim() != module_id {
                    self.runtime_status = format!(
                        "Runtime: ignored incoming shared state for {module_id} (reported {}).",
                        state.module_id.trim()
                    );
                    return None;
                }
                Some(state)
            }
            Ok(None) => None,
            Err(err) => {
                self.runtime_status =
                    format!("Runtime: incoming shared-state read warning for {module_id}: {err:#}");
                None
            }
        }
    }

    fn read_module_bridge_incoming_assets(
        &mut self,
        module_id: &str,
        module_dir: &Path,
        lane_id: Option<&str>,
    ) -> Vec<ModuleBridgeIncomingAssetRecord> {
        match read_bridge_incoming_assets(module_dir, lane_id) {
            Ok(records) => records
                .into_iter()
                .filter(|record| {
                    record.module_id.trim().is_empty() || record.module_id.trim() == module_id
                })
                .collect(),
            Err(err) => {
                self.runtime_status =
                    format!("Runtime: incoming asset read warning for {module_id}: {err:#}");
                Vec::new()
            }
        }
    }

    fn read_module_bridge_log_context(
        &mut self,
        module_id: &str,
        module_dir: &Path,
    ) -> Option<String> {
        match read_bridge_log_excerpts(module_dir) {
            Ok(excerpts) => {
                if excerpts.is_empty() {
                    None
                } else {
                    Some(format_module_bridge_log_context(&excerpts))
                }
            }
            Err(err) => {
                self.runtime_status =
                    format!("Runtime: module log source read warning for {module_id}: {err:#}");
                None
            }
        }
    }

    fn bridge_payload_json(
        &self,
        module_id: &str,
        manifest: &ModuleManifest,
        bridge: Option<&ModuleBridgeStatus>,
        snapshot_preview: &str,
    ) -> Option<String> {
        let mut payload = serde_json::Map::new();
        payload.insert("module_id".to_string(), serde_json::json!(module_id));
        payload.insert(
            "display_name".to_string(),
            serde_json::json!(manifest.display_name),
        );
        payload.insert("icon".to_string(), serde_json::json!(manifest.icon));
        payload.insert(
            "description".to_string(),
            serde_json::json!(manifest.description),
        );
        if !snapshot_preview.trim().is_empty() {
            payload.insert(
                "snapshot_preview".to_string(),
                serde_json::json!(truncate_for_ui(snapshot_preview.trim(), 2_000)),
            );
        }
        if let Some(bridge) = bridge {
            if bridge.updated_at_unix_ms > 0 {
                payload.insert(
                    "bridge_updated_at_unix_ms".to_string(),
                    serde_json::json!(bridge.updated_at_unix_ms),
                );
            }
            if let Some(obj) = bridge.payload.as_object() {
                for (key, value) in obj {
                    payload.insert(key.clone(), value.clone());
                }
            } else if !bridge.payload.is_null() {
                payload.insert("bridge_payload".to_string(), bridge.payload.clone());
            }
        }
        if payload.is_empty() {
            None
        } else {
            serde_json::to_string(&serde_json::Value::Object(payload)).ok()
        }
    }

    fn merge_module_tags(
        &self,
        bridge: Option<&ModuleBridgeStatus>,
        extras: &[&str],
    ) -> Vec<String> {
        let mut tags = Vec::new();
        for extra in extras {
            let tag = extra.trim();
            if !tag.is_empty() && !tags.iter().any(|existing| existing == tag) {
                tags.push(tag.to_string());
            }
        }
        if let Some(bridge) = bridge {
            for tag in &bridge.tags {
                if !tags
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(tag.trim()))
                {
                    tags.push(tag.trim().to_string());
                }
            }
        }
        tags
    }

    fn start_module_rundown_job(
        &mut self,
        module_id: &str,
        overwrite_existing: bool,
        append_event: bool,
    ) {
        if self.module_rundown_jobs.contains_key(module_id) {
            return;
        }
        let Some(bk) = self.bookkeeper.clone() else {
            return;
        };

        let Some(mf) = self
            .module_registry
            .modules
            .iter()
            .find(|m| m.module_id == module_id)
            .cloned()
        else {
            return;
        };

        let bridge_status = self.read_module_bridge_status(module_id, &mf.dir);
        let module_log_context = self.read_module_bridge_log_context(module_id, &mf.dir);
        let mut snapshot = String::new();
        snapshot.push_str("Module:\n");
        snapshot.push_str(&format!(
            "- id: {}\n- name: {}\n",
            mf.module_id, mf.display_name
        ));
        if !mf.description.trim().is_empty() {
            snapshot.push_str(&format!("- description: {}\n", mf.description.trim()));
        }
        snapshot.push('\n');

        if let Some(bridge) = bridge_status.as_ref().filter(|bridge| bridge.has_content()) {
            if !bridge.summary.trim().is_empty() {
                snapshot.push_str("Module-reported summary:\n");
                snapshot.push_str(bridge.summary.trim());
                snapshot.push_str("\n\n");
            }
            if !bridge.snapshot.trim().is_empty() {
                snapshot.push_str("Module-reported snapshot:\n");
                snapshot.push_str(bridge.snapshot.trim());
                snapshot.push_str("\n\n");
            }
            if !bridge.payload.is_null() {
                if let Ok(pretty) = serde_json::to_string_pretty(&bridge.payload) {
                    snapshot.push_str("Module-reported payload:\n");
                    snapshot.push_str(&pretty);
                    snapshot.push_str("\n\n");
                }
            }
        } else {
            // Ensure module state surfaces are loaded so we can snapshot current values.
            if mf.dir.join("ui.json").is_file() {
                let form = self
                    .module_forms
                    .entry(module_id.to_string())
                    .or_insert_with(|| ModuleFormState::new(&mf.dir));
                form.ensure_loaded();
            } else {
                let ws = self
                    .module_workspaces
                    .entry(module_id.to_string())
                    .or_insert_with(|| ModuleWorkspaceState::new(&mf.dir));
                ws.ensure_loaded();
            }

            // Best-effort auto-save so module state persists even if the user forgets to click Save.
            if let Some(form) = self.module_forms.get_mut(module_id) {
                form.save();
            }
            if let Some(ws) = self.module_workspaces.get_mut(module_id) {
                ws.save();
            }

            if let Some(form) = self.module_forms.get(module_id) {
                if let Some(spec) = &form.spec {
                    snapshot.push_str("Form fields:\n");
                    for f in &spec.fields {
                        let id = f.id.trim();
                        if id.is_empty() {
                            continue;
                        }
                        let v = form.values.get(id);
                        match v {
                            Some(ModuleFieldValue::Str(s)) => {
                                let s = s.trim();
                                if !s.is_empty() {
                                    snapshot.push_str(&format!("- {}: {}\n", f.label, s));
                                }
                            }
                            Some(ModuleFieldValue::Bool(b)) => {
                                if *b {
                                    snapshot.push_str(&format!("- {}: true\n", f.label));
                                }
                            }
                            Some(ModuleFieldValue::Num(n)) => {
                                if n.abs() > f64::EPSILON {
                                    snapshot.push_str(&format!("- {}: {}\n", f.label, n));
                                }
                            }
                            None => {}
                        }
                    }
                    snapshot.push('\n');
                }
            }

            if let Some(ws) = self.module_workspaces.get(module_id) {
                let t = ws.text.trim();
                if !t.is_empty() {
                    snapshot.push_str("Workspace:\n");
                    snapshot.push_str(t);
                    snapshot.push('\n');
                    snapshot.push('\n');
                }
            }

            if let Some(ai) = self.module_ai.get(module_id) {
                let out = ai.output.trim();
                if !out.is_empty() {
                    snapshot.push_str("Last AI output (if any):\n");
                    snapshot.push_str(out);
                    snapshot.push('\n');
                    snapshot.push('\n');
                }
            }
        }

        if let Some(log_context) = module_log_context.as_deref() {
            snapshot.push_str("Recent module logs (declared by plug):\n");
            snapshot.push_str(log_context.trim());
            snapshot.push_str("\n\n");
        }

        snapshot = truncate_for_ui(&snapshot, 12_000);

        // Payload for indexing: manifest + optional bridge payload + a small snapshot preview.
        let payload_json =
            self.bridge_payload_json(module_id, &mf, bridge_status.as_ref(), &snapshot);

        let module_id_owned = module_id.to_string();
        let snapshot_owned = snapshot;
        let payload_owned = payload_json;
        let mut extra_tags = vec!["module_rundown", "auto"];
        if module_log_context
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            extra_tags.push("module_logs");
        }
        let tags = self.merge_module_tags(bridge_status.as_ref(), &extra_tags);
        let bridge_event_type = bridge_status
            .as_ref()
            .map(|bridge| bridge.event_type.clone())
            .unwrap_or_else(|| "suspend_rundown".to_string());

        let (tx, rx) = crossbeam_channel::bounded::<String>(1);
        std::thread::spawn(move || {
            let summary = bk
                .summarize_module_rundown(module_id_owned.clone(), snapshot_owned)
                .unwrap_or_default();
            let summary = summary.trim().to_string();
            if append_event {
                bk.append_module_event(
                    module_id_owned.clone(),
                    bridge_event_type,
                    if summary.is_empty() {
                        "No rundown generated.".to_string()
                    } else {
                        summary.clone()
                    },
                    tags,
                    payload_owned,
                );
            }
            let _ = tx.send(summary);
        });

        self.module_rundown_jobs.insert(
            module_id.to_string(),
            ModuleRundownJob {
                rx,
                overwrite_existing,
            },
        );
    }

    fn start_generation(&mut self, prompt: String) {
        if self.is_generating {
            return;
        }
        if self.orch_freeze_pending || matches!(&self.tab, Tab::Module(_)) {
            self.runtime_status = "Runtime: orchestrator paused (module active)".to_string();
            return;
        }
        self.pulse_ecg(88.0, "Generating a chat response with the local model.");

        let (tx, rx) = crossbeam_channel::unbounded::<GenEvent>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = Arc::clone(&cancel);
        let gguf = self.gguf_path.clone();
        let orch_temp = self.orch_temp;
        let orch_top_p = self.orch_top_p;
        let orch_top_k = self.orch_top_k;
        let orch_max_tokens = self.orch_max_tokens;
        let runtime_dir = find_runtime_windows_dir();
        let model_label = gguf
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "(no GGUF selected)".to_string());
        let system = self.build_generation_system_prompt(&prompt, &model_label);

        std::thread::spawn(move || {
            if gguf.is_none() {
                let _ = tx.send(GenEvent::Error(
                    "No GGUF selected. Use File → Open GGUF...".to_string(),
                ));
                let _ = tx.send(GenEvent::Done);
                return;
            }

            let model_path = gguf.unwrap();
            let runtime_dir = match runtime_dir {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(GenEvent::Error(format!("{e:#}")));
                    let _ = tx.send(GenEvent::Done);
                    return;
                }
            };

            let llama = match llama_dyn::Llama::load(&runtime_dir) {
                Ok(l) => l,
                Err(e) => {
                    let _ = tx.send(GenEvent::Error(format!("{e:#}")));
                    let _ = tx.send(GenEvent::Done);
                    return;
                }
            };

            let info = llama.system_info();
            if !info.is_empty() {
                let _ = tx.send(GenEvent::Info(info));
            }

            let res = llama.generate_chat(
                &model_path,
                &system,
                &prompt,
                orch_max_tokens.max(1) as usize,
                orch_temp,
                orch_top_p,
                orch_top_k,
                &cancel_for_thread,
                |tok| {
                    let _ = tx.send(GenEvent::Token(tok.to_string()));
                },
            );

            if let Err(e) = res {
                let _ = tx.send(GenEvent::Error(format!("{e:#}")));
            }
            let _ = tx.send(GenEvent::Done);
        });

        self.is_generating = true;
        self.gen_cancel = Some(cancel);
        self.gen_rx = Some(rx);
        self.assistant_draft.clear();
        self.scroll_to_bottom = true;
    }

    fn start_multimodal_generation(&mut self, prompt: String, image_path: PathBuf) {
        if self.is_generating {
            return;
        }
        if self.orch_freeze_pending || matches!(&self.tab, Tab::Module(_)) {
            self.runtime_status = "Runtime: orchestrator paused (module active)".to_string();
            return;
        }

        let Some(model_path) = self.gguf_path.clone() else {
            self.runtime_status = "Runtime: no GGUF selected for multimodal chat.".to_string();
            return;
        };
        let runtime_dir = match find_runtime_windows_dir() {
            Ok(path) => path,
            Err(err) => {
                self.runtime_status = format!("Runtime: {err:#}");
                return;
            }
        };
        let architecture = read_gguf_architecture(&model_path).ok().flatten();
        if !architecture_supports_mtmd(&model_path, architecture.as_deref()) {
            let label = model_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| model_path.display().to_string());
            self.runtime_status = format!(
                "Runtime: selected image needs a multimodal-capable local runtime path. Current model {label} is not detected as ready for this path."
            );
            return;
        }
        let Some(mmproj_path) = find_matching_mmproj_path(&model_path, architecture.as_deref()) else {
            let arch_label = architecture.unwrap_or_else(|| "unknown".to_string());
            self.runtime_status = format!(
                "Runtime: no matching mmproj file found for multimodal model architecture {arch_label}. Add a projector GGUF near the model first."
            );
            return;
        };

        self.pulse_ecg(90.0, "Generating a multimodal chat response with the local model.");

        let (tx, rx) = crossbeam_channel::unbounded::<GenEvent>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = Arc::clone(&cancel);
        let orch_temp = self.orch_temp;
        let orch_top_p = self.orch_top_p;
        let orch_top_k = self.orch_top_k;
        let orch_max_tokens = self.orch_max_tokens;
        let model_label = model_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| model_path.display().to_string());
        let system = self.build_generation_system_prompt(&prompt, &model_label);
        let extra_args = mtmd_extra_args_for_model(&model_path, architecture.as_deref());
        let status_model_label = model_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let status_mmproj_label = mmproj_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        std::thread::spawn(move || {
            let res = run_mtmd_cli_generation(
                &runtime_dir,
                &model_path,
                &mmproj_path,
                &image_path,
                &system,
                &prompt,
                orch_max_tokens.max(1) as usize,
                orch_temp,
                orch_top_p,
                orch_top_k,
                &extra_args,
                &cancel_for_thread,
            );

            match res {
                Ok(output) => {
                    let trimmed = output.trim();
                    if !trimmed.is_empty() {
                        let _ = tx.send(GenEvent::Token(trimmed.to_string()));
                    }
                }
                Err(err) => {
                    let _ = tx.send(GenEvent::Error(format!("{err:#}")));
                }
            }
            let _ = tx.send(GenEvent::Done);
        });

        self.runtime_status = format!(
            "Runtime: multimodal image turn using {} + {}.",
            status_model_label,
            status_mmproj_label
        );
        self.is_generating = true;
        self.gen_cancel = Some(cancel);
        self.gen_rx = Some(rx);
        self.assistant_draft.clear();
        self.scroll_to_bottom = true;
    }

    fn build_generation_system_prompt(&self, prompt: &str, model_label: &str) -> String {
        let mut base_system = self
            .messages
            .iter()
            .find(|m| m.role == Role::System)
            .map(|m| m.content.clone())
            .unwrap_or_else(default_orchestrator_system_prompt);
        if base_system.trim() == "You are ChattyCog. Respond concisely and helpfully." {
            base_system = default_orchestrator_system_prompt();
        }
        if let Some(capsule) = self.active_orchestrator_capsule() {
            base_system.push_str("\n\n### ACTIVE CAPSULE\n");
            base_system.push_str(
                "The user explicitly selected this reusable behavior/personality capsule for the current task. Follow it as a style and persona layer unless the current request clearly needs otherwise.\n",
            );
            base_system.push_str(&capsule.text);
            base_system.push('\n');
        }

        let mut departments_context =
            read_departments_from_logs_dir(self.logs_dir.as_deref()).unwrap_or_default();
        if departments_context.contains("<bullet>") || departments_context.contains("<paragraph>") {
            departments_context = departments_context
                .replace("<bullet>", "")
                .replace("<paragraph>", "");
        }
        let lukewarm_context =
            read_lukewarm_from_logs_dir(self.logs_dir.as_deref()).unwrap_or_default();
        let mut system = base_system;
        system.push_str("\n\n### CHATTYCOG COCKPIT ORIENTATION\n");
        system.push_str(&build_wakeup_orientation(
            model_label,
            self.sandbox_dir.is_some(),
            self.module_registry.modules.len(),
            self.prefs.allow_sandbox_tool_requests,
        ));
        system.push('\n');
        if !departments_context.trim().is_empty() {
            system.push_str("\n\n### BACKGROUND MEMORY: DEPARTMENT STATUS\n");
            system.push_str("These are older module rundowns for continuity. They are not the user's current request. Use them only if the current user message clearly asks for or depends on them.\n");
            system.push_str(&truncate_for_ui(departments_context.trim(), 2_000));
            system.push('\n');
        }
        if !lukewarm_context.trim().is_empty() {
            system.push_str("\n### BACKGROUND MEMORY: RECENT ACTIVITY\n");
            system.push_str("This is a rolling summary, not an instruction. Do not continue old tasks unless the user asks.\n");
            system.push_str(&truncate_for_ui(lukewarm_context.trim(), 1_200));
            system.push('\n');
        }
        let shared_lukewarm_context = self.build_applied_lukewarm_prompt_block();
        if !shared_lukewarm_context.trim().is_empty() {
            system.push_str("\n### BACKGROUND MEMORY: NETWORK-SHARED CONTEXT\n");
            system.push_str("This context came from another local ChattyCog peer. Treat it as optional background unless the current user message refers to it.\n");
            system.push_str(&truncate_for_ui(shared_lukewarm_context.trim(), 1_600));
            system.push('\n');
        }
        let recent_chat_context = build_recent_chat_prompt_context(&self.messages, 8, 6_000);
        if !recent_chat_context.trim().is_empty() {
            system.push_str("\n### RECENT CHAT CONTEXT\n");
            system.push_str("Previous turns are context only. The current user message is the immediate task.\n");
            system.push_str(&recent_chat_context);
            system.push('\n');
        }
        let sandbox_context = build_sandbox_prompt_context(
            self.sandbox_dir.as_deref(),
            DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH,
            DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH,
        );
        if !sandbox_context.trim().is_empty() {
            system.push_str("\n### LOCAL SANDBOX CONTEXT\n");
            system.push_str("This describes local files available through approved sandbox actions. Do not summarize the sandbox unless it helps answer the current user message.\n");
            system.push_str(&sandbox_context);
            system.push('\n');
        }
        if !self.sandbox_last_tool_result.trim().is_empty() {
            system.push_str("\n### LAST SANDBOX TOOL RESULT\n");
            system.push_str(&truncate_for_ui(
                self.sandbox_last_tool_result.trim(),
                5_000,
            ));
            system.push('\n');
        }
        if let Some(task_ledger_nudge) =
            build_task_ledger_prompt_nudge(prompt, self.sandbox_dir.as_deref())
        {
            system.push_str("\n### TASK LEDGER NUDGE\n");
            system.push_str(&task_ledger_nudge);
            system.push('\n');
        }
        if self.prefs.allow_sandbox_tool_requests {
            system.push_str(
                "\n### SANDBOX TOOL POLICY\n\
You cannot read or write files directly, but you can request sandbox actions for the user to approve.\n\
The persistent working scratchpad lives at `scratchpad/current.md` inside `Chatty_Sandbox/`.\n\
AI sandbox file actions are limited to plain text notes only: `.txt` and `.md` files.\n\
Only request sandbox actions when they are useful for the user's current request. Do not emit tool JSON during ordinary conversation, greetings, or orientation unless the user asks you to inspect or update files.\n\
Use the scratchpad to keep durable notes, extracted facts, intermediate plans, and reminders that should survive context-window pressure when the current task benefits from that.\n\
The structured task ledger lives at `scratchpad/task_ledger.md` and is the best place to keep the current task, next step, open questions, and files touched.\n\
For longer tasks, update the task ledger whenever the plan meaningfully changes.\n\
For complex or multi-step tasks, prefer a deterministic preload first so you can inspect the sandbox state before acting.\n\
When you do need sandbox help, output one or more JSON objects, each on its own line and with no surrounding commentary, using one of:\n\
  {\"tool\":\"sandbox.write\",\"path\":\"notes/Ready.md\",\"contents\":\"...\"}\n\
  {\"tool\":\"sandbox.append\",\"path\":\"scratchpad/current.md\",\"contents\":\"\\n- new note\"}\n\
  {\"tool\":\"sandbox.read\",\"path\":\"notes/Ready.md\"}\n\
  {\"tool\":\"sandbox.list\"}\n\
  {\"tool\":\"sandbox.ledger\",\"status\":\"active\",\"current_task\":\"...\",\"next_step\":\"...\",\"open_questions\":[\"...\"],\"files_touched\":[\"notes/brief.md\"],\"notes\":[\"...\"]}\n\
  {\"tool\":\"sandbox.preload\",\"paths\":[\"notes/brief.md\",\"plans/today.md\"],\"include_list\":true,\"include_scratchpad\":true,\"include_ledger\":true,\"note\":\"load planning context first\"}\n\
After approval, the sandbox result will be returned to you on the next turn.\n",
            );
        } else {
            system.push_str("\n### SANDBOX TOOL POLICY\nSandbox tool requests are disabled. Do not request file operations.\n");
        }
        system
    }

    fn stop_generation(&mut self) {
        if let Some(c) = &self.gen_cancel {
            c.store(true, Ordering::Relaxed);
        }
        self.pulse_ecg(24.0, "Interrupted the current chat response.");
    }

    fn pulse_ecg(&mut self, intensity: f32, note: &str) {
        self.ecg_window.record_activity(intensity, note);
    }

    fn on_module_suspend(&mut self, module_id: &str) {
        let Some(bk) = self.bookkeeper.clone() else {
            return;
        };
        let manifest = self
            .module_registry
            .modules
            .iter()
            .find(|m| m.module_id == module_id)
            .cloned();

        if let Some(manifest) = manifest.as_ref() {
            if let Some(bridge) = self.read_module_bridge_status(module_id, &manifest.dir) {
                if !bridge.summary.trim().is_empty() {
                    self.module_state_notes
                        .insert(module_id.to_string(), bridge.summary.clone());
                }
                let fingerprint = bridge.fingerprint();
                let already_logged = self
                    .module_bridge_last_fingerprint
                    .get(module_id)
                    .is_some_and(|prev| prev == &fingerprint);
                if !already_logged && !bridge.summary.trim().is_empty() {
                    let payload = self.bridge_payload_json(
                        module_id,
                        manifest,
                        Some(&bridge),
                        if bridge.snapshot.trim().is_empty() {
                            &bridge.summary
                        } else {
                            &bridge.snapshot
                        },
                    );
                    bk.append_module_event(
                        module_id.to_string(),
                        bridge.event_type.clone(),
                        bridge.summary.clone(),
                        self.merge_module_tags(Some(&bridge), &["module_rundown", "bridge"]),
                        payload,
                    );
                    self.module_bridge_last_fingerprint
                        .insert(module_id.to_string(), fingerprint);
                    return;
                }
                if !bridge.summary.trim().is_empty() {
                    return;
                }
                if !bridge.snapshot.trim().is_empty()
                    && self.prefs.auto_generate_module_suspend_rundown
                {
                    self.start_module_rundown_job(module_id, true, true);
                    return;
                }
            }
        }

        let summary_text = self
            .module_state_notes
            .get(module_id)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if !summary_text.is_empty() {
            let payload = self
                .module_registry
                .modules
                .iter()
                .find(|m| m.module_id == module_id)
                .map(|m| {
                    serde_json::json!({
                        "display_name": m.display_name,
                        "icon": m.icon,
                        "description": m.description,
                    })
                    .to_string()
                });
            bk.append_module_event(
                module_id.to_string(),
                "suspend_rundown".to_string(),
                summary_text,
                vec!["module_rundown".to_string()],
                payload,
            );
            return;
        }

        if self.prefs.auto_generate_module_suspend_rundown {
            // Async: generate a short rundown and append to cold log when ready.
            self.start_module_rundown_job(module_id, true, true);
        } else {
            bk.append_module_event(
                module_id.to_string(),
                "suspend_rundown".to_string(),
                "No rundown provided.".to_string(),
                vec!["module_rundown".to_string()],
                None,
            );
        }
    }

    fn extract_sandbox_actions_from_text(text: &str) -> Vec<SandboxAction> {
        #[derive(serde::Deserialize)]
        struct ToolReq {
            tool: String,
            path: Option<String>,
            paths: Option<Vec<String>>,
            contents: Option<String>,
            include_list: Option<bool>,
            include_scratchpad: Option<bool>,
            include_ledger: Option<bool>,
            note: Option<String>,
            status: Option<String>,
            current_task: Option<String>,
            next_step: Option<String>,
            open_questions: Option<Vec<String>>,
            files_touched: Option<Vec<String>>,
            notes: Option<Vec<String>>,
        }

        fn parse_obj(s: &str) -> Option<ToolReq> {
            let s = s.trim();
            if !s.starts_with('{') || !s.ends_with('}') {
                return None;
            }
            serde_json::from_str::<ToolReq>(s).ok()
        }

        fn actions_from_req(req: ToolReq) -> Option<SandboxAction> {
            match req.tool.as_str() {
                "sandbox.write" => {
                    let path = req.path?;
                    let contents = req.contents.unwrap_or_default();
                    Some(SandboxAction::Write { path, contents })
                }
                "sandbox.append" => {
                    let path = req.path?;
                    let contents = req.contents.unwrap_or_default();
                    Some(SandboxAction::Append { path, contents })
                }
                "sandbox.read" => {
                    let path = req.path?;
                    Some(SandboxAction::Read { path })
                }
                "sandbox.list" => Some(SandboxAction::List),
                "sandbox.ledger" => Some(SandboxAction::Ledger {
                    status: req.status.unwrap_or_default().trim().to_string(),
                    current_task: req.current_task.unwrap_or_default().trim().to_string(),
                    next_step: req.next_step.unwrap_or_default().trim().to_string(),
                    open_questions: req
                        .open_questions
                        .unwrap_or_default()
                        .into_iter()
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect(),
                    files_touched: req
                        .files_touched
                        .unwrap_or_default()
                        .into_iter()
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect(),
                    notes: req
                        .notes
                        .unwrap_or_default()
                        .into_iter()
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect(),
                }),
                "sandbox.preload" => Some(SandboxAction::Preload {
                    paths: req
                        .paths
                        .unwrap_or_default()
                        .into_iter()
                        .map(|p| p.trim().to_string())
                        .filter(|p| !p.is_empty())
                        .collect(),
                    include_list: req.include_list.unwrap_or(true),
                    include_scratchpad: req.include_scratchpad.unwrap_or(true),
                    include_ledger: req.include_ledger.unwrap_or(true),
                    note: req.note.unwrap_or_default().trim().to_string(),
                }),
                _ => None,
            }
        }

        // 1) Line-based parse (best case).
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim().trim_matches('`');
            if let Some(req) = parse_obj(line) {
                if let Some(a) = actions_from_req(req) {
                    out.push(a);
                }
            }
        }
        if !out.is_empty() {
            return out;
        }

        // 2) Fallback: find embedded JSON objects containing `"tool":"sandbox.`.
        let needle = "\"tool\":\"sandbox.";
        let mut i = 0usize;
        let bytes = text.as_bytes();
        while i < bytes.len() {
            let Some(pos) = text[i..].find(needle) else {
                break;
            };
            let pos = i + pos;

            // scan left to '{'
            let mut l = pos;
            while l > 0 && bytes[l] != b'{' {
                l -= 1;
            }
            if bytes.get(l) != Some(&b'{') {
                i = pos + needle.len();
                continue;
            }

            // scan right with brace balance (ignoring strings)
            let mut r = l;
            let mut depth = 0i32;
            let mut in_str = false;
            let mut esc = false;
            while r < bytes.len() {
                let ch = bytes[r] as char;
                if in_str {
                    if esc {
                        esc = false;
                    } else if ch == '\\' {
                        esc = true;
                    } else if ch == '"' {
                        in_str = false;
                    }
                } else {
                    if ch == '"' {
                        in_str = true;
                    } else if ch == '{' {
                        depth += 1;
                    } else if ch == '}' {
                        depth -= 1;
                        if depth == 0 {
                            r += 1;
                            break;
                        }
                    }
                }
                r += 1;
            }
            if depth != 0 || r <= l {
                i = pos + needle.len();
                continue;
            }

            let candidate = text[l..r].trim();
            if let Some(req) = parse_obj(candidate) {
                if let Some(a) = actions_from_req(req) {
                    out.push(a);
                }
            }
            i = r;
        }

        out
    }

    fn open_sandbox_file_in_editor(&mut self, path: &Path) {
        let Some(dir) = self.sandbox_dir.clone() else {
            return;
        };
        match ensure_path_within_dir(&dir, path) {
            Ok(pp) => {
                self.sandbox_selected = Some(pp.clone());
                self.sandbox_editor_path = Some(pp.clone());
                self.sandbox_last_working_path = Some(pp.clone());
                if path_uses_inline_image_preview(&pp) {
                    self.sandbox_editor_text.clear();
                } else {
                    match read_text_file(&pp, 500_000) {
                        Ok(text) => self.sandbox_editor_text = text,
                        Err(err) => {
                            self.sandbox_status = format!("Failed to open file: {err}");
                            return;
                        }
                    }
                }
                self.sandbox_status = format!(
                    "Opened {}",
                    pp.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Failed to open file: {err}");
            }
        }
    }

    fn open_sandbox_file_and_focus_tab(&mut self, path: &Path) {
        self.open_sandbox_file_in_editor(path);
        if self.sandbox_editor_path.is_some() {
            self.prev_tab = self.tab.clone();
            self.tab = Tab::Sandbox;
        }
    }

    fn drain_generation_events(&mut self) {
        let Some(rx) = &self.gen_rx else { return };

        let events: Vec<GenEvent> = rx.try_iter().collect();
        if events.is_empty() {
            return;
        }

        let mut saw_done = false;
        for ev in events {
            match ev {
                GenEvent::Token(t) => {
                    self.assistant_draft.push_str(&t);
                    if trim_exact_repeated_suffix(&mut self.assistant_draft) {
                        self.runtime_status = "Runtime: trimmed repeated draft loop.".to_string();
                    }
                    self.scroll_to_bottom = true;
                }
                GenEvent::Info(s) => {
                    self.runtime_status = format!("Runtime: {}", truncate_for_ui(&s, 240));
                }
                GenEvent::Error(e) => {
                    if let Some(model_path) = self.gguf_path.as_ref() {
                        let key = model_path.to_string_lossy().to_string();
                        if e.contains("0xC0000409")
                            || e.contains("STATUS_STACK_BUFFER_OVERRUN")
                            || e.contains("current local llama.cpp runtime appears unstable")
                        {
                            self.model_runtime_issues.insert(
                                key,
                                "Gemma 4 multimodal runtime crash observed on this build.".to_string(),
                            );
                        }
                    }
                    self.messages.push(Message {
                        role: Role::Assistant,
                        content: format!("Error: {e}"),
                        thinking: None,
                    });
                    trim_live_chat_messages(&mut self.messages);
                    self.runtime_status = format!("Runtime error: {e}");
                    self.assistant_draft.clear();
                    self.scroll_to_bottom = true;
                }
                GenEvent::Done => {
                    saw_done = true;
                }
            }
        }

        if saw_done {
            self.is_generating = false;
            self.gen_cancel = None;
            self.gen_rx = None;
            if !self.assistant_draft.trim().is_empty() {
                trim_exact_repeated_suffix(&mut self.assistant_draft);
                let raw_content = std::mem::take(&mut self.assistant_draft);
                let (content, thinking) = split_assistant_output(&raw_content);
                if content.trim().is_empty() {
                    self.scroll_to_bottom = true;
                    return;
                }
                self.messages.push(Message {
                    role: Role::Assistant,
                    content: content.clone(),
                    thinking,
                });
                trim_live_chat_messages(&mut self.messages);
                push_hot_memory(self, format!("Assistant: {}", one_line(&content, 120)));
                if self.networking_shared_chat_mirror_main_chat
                    && self.shared_chat_local_ai_allowed()
                {
                    self.broadcast_shared_chat_message("assistant", "ChattyCog", &content);
                }
                if let Some(bk) = &self.bookkeeper {
                    bk.append(MemoryEvent {
                        ts_unix_ms: now_unix_ms(),
                        kind: MemoryKind::Cold,
                        category: EventCategory::Chat,
                        source: "assistant".to_string(),
                        module: None,
                        event_type: Some("message".to_string()),
                        text: content,
                        tags: Vec::new(),
                        entities: Vec::new(),
                        payload_json: None,
                    });
                }
                self.scroll_to_bottom = true;
            } else {
                self.assistant_draft.clear();
            }

            // Collect any tool requests from the final assistant message.
            if self.prefs.allow_sandbox_tool_requests {
                if let Some(last) = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                {
                    let mut actions = Self::extract_sandbox_actions_from_text(&last.content);
                    if !actions.is_empty() {
                        // Keep latest batch only, to avoid repeated approvals.
                        self.pending_sandbox_actions.clear();
                        self.pending_sandbox_actions.append(&mut actions);
                    }
                }
            }

            // If a module tab was opened while generating, pause only after finishing the response.
            if self.orch_freeze_pending && matches!(&self.tab, Tab::Module(_)) {
                self.orch_freeze_pending = false;
                self.runtime_status = "Runtime: orchestrator paused (module active)".to_string();
            }
        }
    }
}

impl eframe::App for ChattyCogApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.show_startup_splash(ctx) {
            return;
        }
        self.ecg_window.tick(Instant::now());
        ctx.request_repaint_after(self.ecg_window.refresh_interval());
        // Module on-suspend handshake: when leaving a module tab, debrief the bookkeeper.
        if self.prev_tab != self.tab {
            if let Tab::Module(old_id) = &self.prev_tab {
                let old_id = old_id.clone();
                if !matches!(&self.tab, Tab::Module(new_id) if new_id == &old_id) {
                    self.on_module_suspend(&old_id);
                }
            }
            self.prev_tab = self.tab.clone();
        }

        self.drain_generation_events();
        self.drain_module_rundown_jobs();
        let networking_changed = self.networking.poll();
        self.networking.set_presence(build_local_presence(self));
        let current_shared_chat_peer_keys = self.shared_chat_connected_peer_keys();
        let shared_chat_peer_membership_changed =
            current_shared_chat_peer_keys != self.networking_shared_chat_connected_peer_keys;
        if shared_chat_peer_membership_changed {
            self.networking_shared_chat_connected_peer_keys = current_shared_chat_peer_keys;
        }
        let now = Instant::now();
        if self
            .networking_shared_chat_presence_next_sync_at
            .map(|due| now >= due)
            .unwrap_or(true)
        {
            self.sync_shared_chat_host_presence();
            self.networking_shared_chat_presence_next_sync_at =
                Some(now + Duration::from_millis(900));
        }
        if networking_changed {
            if shared_chat_peer_membership_changed
                && self.networking_shared_chat_policy.session_active
                && self.shared_chat_is_local_host()
                && !self.networking.snapshot().connected_peers.is_empty()
            {
                self.broadcast_shared_chat_policy_with_options("", false, false, false);
            }
            let received = self.networking.snapshot().received_handoffs.clone();
            for handoff in received {
                if self
                    .networking_seen_handoffs
                    .insert(handoff.handoff_id.clone())
                {
                    push_hot_memory(
                        self,
                        format!(
                            "LAN handoff from {}: {}",
                            handoff.from_device_name,
                            one_line(&handoff.title, 80)
                        ),
                    );
                    if let Some(bk) = &self.bookkeeper {
                        bk.append(MemoryEvent {
                            ts_unix_ms: now_unix_ms(),
                            kind: MemoryKind::Cold,
                            category: EventCategory::Module,
                            source: "network".to_string(),
                            module: Some("networking".to_string()),
                            event_type: Some("handoff_received".to_string()),
                            text: format!(
                                "Received LAN handoff from {}: {}\n\n{}",
                                handoff.from_device_name, handoff.title, handoff.body
                            ),
                            tags: vec!["lan".to_string(), "handoff".to_string()],
                            entities: vec![handoff.from_device_name.clone()],
                            payload_json: None,
                        });
                    }
                }
            }

            let received_artifacts = self.networking.snapshot().received_artifacts.clone();
            for artifact in received_artifacts {
                if !self
                    .networking_seen_artifacts
                    .insert(artifact.artifact_id.clone())
                {
                    continue;
                }

                if artifact.is_binary() {
                    match self.store_received_generic_transfer(&artifact) {
                        Ok(path) => {
                            push_hot_memory(
                                self,
                                format!(
                                    "Binary/file transfer from {} saved to inbox: {}",
                                    artifact.from_device_name,
                                    one_line(
                                        if artifact.summary.trim().is_empty() {
                                            if artifact.label.trim().is_empty() {
                                                artifact.kind.trim()
                                            } else {
                                                artifact.label.trim()
                                            }
                                        } else {
                                            artifact.summary.trim()
                                        },
                                        120
                                    )
                                ),
                            );
                            if let Some(bk) = &self.bookkeeper {
                                bk.append(MemoryEvent {
                                    ts_unix_ms: now_unix_ms(),
                                    kind: MemoryKind::Cold,
                                    category: EventCategory::Module,
                                    source: "network".to_string(),
                                    module: if artifact.module_id.trim().is_empty() {
                                        Some("generic_transfer".to_string())
                                    } else {
                                        Some(artifact.module_id.clone())
                                    },
                                    event_type: Some("generic_transfer_received".to_string()),
                                    text: format!(
                                        "Received file-style transfer from {}.\nSaved to inbox: {}\n\nKind: {}\nContent type: {}",
                                        artifact.from_device_name,
                                        path.display(),
                                        artifact.kind,
                                        artifact.content_type
                                    ),
                                    tags: vec![
                                        "lan".to_string(),
                                        "transfer".to_string(),
                                        "binary".to_string(),
                                    ],
                                    entities: vec![artifact.from_device_name.clone()],
                                    payload_json: None,
                                });
                            }
                        }
                        Err(err) => {
                            self.networking_status = format!(
                                "Networking: could not store file-style transfer from {}: {}",
                                artifact.from_device_name, err
                            );
                        }
                    }
                    continue;
                }

                match artifact.kind.trim() {
                    "module_shared_state_json" => {
                        match self.store_received_module_shared_state(&artifact) {
                            Ok(path) => {
                                let summary = if artifact.summary.trim().is_empty() {
                                    format!(
                                        "Shared module state received for {}.",
                                        artifact.module_id.trim()
                                    )
                                } else {
                                    artifact.summary.trim().to_string()
                                };
                                push_hot_memory(
                                    self,
                                    format!(
                                        "Workflow from {} saved to inbox: {}",
                                        artifact.from_device_name,
                                        one_line(&summary, 120)
                                    ),
                                );
                                if let Some(bk) = &self.bookkeeper {
                                    bk.append(MemoryEvent {
                                    ts_unix_ms: now_unix_ms(),
                                    kind: MemoryKind::Cold,
                                    category: EventCategory::Module,
                                    source: "network".to_string(),
                                    module: Some(artifact.module_id.clone()),
                                    event_type: Some("shared_state_received".to_string()),
                                    text: format!(
                                        "Received module shared workflow from {} for module `{}`.\nSaved to inbox: {}\n\nSummary: {}",
                                        artifact.from_device_name,
                                        artifact.module_id,
                                        path.display(),
                                        summary
                                    ),
                                    tags: vec![
                                        "lan".to_string(),
                                        "module".to_string(),
                                        "shared_state".to_string(),
                                    ],
                                    entities: vec![
                                        artifact.from_device_name.clone(),
                                        artifact.module_id.clone(),
                                    ],
                                    payload_json: Some(artifact.text.clone()),
                                });
                                }
                            }
                            Err(err) => {
                                self.networking_status = format!(
                                    "Networking: could not store shared state from {}: {}",
                                    artifact.from_device_name, err
                                );
                            }
                        }
                    }
                    "module_shared_state_ack_json" => {
                        match self.store_received_module_session_ack(&artifact) {
                            Ok(ack) => {
                                let from_label = if ack.from_device_name.trim().is_empty() {
                                    ack.from_device_id.clone()
                                } else {
                                    ack.from_device_name.clone()
                                };
                                let result = if ack.applied {
                                    "applied"
                                } else if ack.stale {
                                    "marked stale"
                                } else {
                                    "did not apply"
                                };
                                self.networking_status = format!(
                                    "Networking: {} {} session {} revision {} for {}.",
                                    from_label,
                                    result,
                                    if ack.session_id.trim().is_empty() {
                                        "(legacy)"
                                    } else {
                                        ack.session_id.trim()
                                    },
                                    ack.session_revision,
                                    ack.module_id
                                );
                                push_hot_memory(
                                    self,
                                    format!(
                                        "{} {} {} rev {}",
                                        from_label, result, ack.module_id, ack.session_revision
                                    ),
                                );
                                if let Some(bk) = &self.bookkeeper {
                                    bk.append(MemoryEvent {
                                        ts_unix_ms: now_unix_ms(),
                                        kind: MemoryKind::Cold,
                                        category: EventCategory::Module,
                                        source: "network".to_string(),
                                        module: Some(ack.module_id.clone()),
                                        event_type: Some("shared_state_ack".to_string()),
                                        text: format!(
                                            "{} {} session {} revision {} for module `{}`.\n\n{}",
                                            from_label,
                                            result,
                                            if ack.session_id.trim().is_empty() {
                                                "(legacy)"
                                            } else {
                                                ack.session_id.trim()
                                            },
                                            ack.session_revision,
                                            ack.module_id,
                                            ack.message
                                        ),
                                        tags: vec![
                                            "lan".to_string(),
                                            "module".to_string(),
                                            "shared_state".to_string(),
                                            "ack".to_string(),
                                        ],
                                        entities: vec![from_label],
                                        payload_json: Some(artifact.text.clone()),
                                    });
                                }
                            }
                            Err(err) => {
                                self.networking_status = format!(
                                    "Networking: could not read module session receipt from {}: {}",
                                    artifact.from_device_name, err
                                );
                            }
                        }
                    }
                    "workflow_bundle_json" => {
                        match self.store_received_workflow_bundle(&artifact) {
                            Ok(path) => {
                                let summary = if artifact.summary.trim().is_empty() {
                                    "Shared setup bundle received.".to_string()
                                } else {
                                    artifact.summary.trim().to_string()
                                };
                                push_hot_memory(
                                    self,
                                    format!(
                                        "Workflow bundle from {} saved to inbox: {}",
                                        artifact.from_device_name,
                                        one_line(&summary, 120)
                                    ),
                                );
                                if let Some(bk) = &self.bookkeeper {
                                    bk.append(MemoryEvent {
                                    ts_unix_ms: now_unix_ms(),
                                    kind: MemoryKind::Cold,
                                    category: EventCategory::Module,
                                    source: "network".to_string(),
                                    module: Some("workflow_bundle".to_string()),
                                    event_type: Some("bundle_received".to_string()),
                                    text: format!(
                                        "Received workflow bundle from {}.\nSaved to inbox: {}\n\nSummary: {}",
                                        artifact.from_device_name,
                                        path.display(),
                                        summary
                                    ),
                                    tags: vec![
                                        "lan".to_string(),
                                        "workflow".to_string(),
                                        "bundle".to_string(),
                                    ],
                                    entities: vec![artifact.from_device_name.clone()],
                                    payload_json: Some(artifact.text.clone()),
                                });
                                }
                            }
                            Err(err) => {
                                self.networking_status = format!(
                                    "Networking: could not store workflow bundle from {}: {}",
                                    artifact.from_device_name, err
                                );
                            }
                        }
                    }
                    "shared_chat_policy_json" => {
                        if let Err(err) = self.apply_received_shared_chat_policy(&artifact) {
                            self.networking_status = format!(
                                "Networking: could not read shared room policy from {}: {}",
                                artifact.from_device_name, err
                            );
                        }
                    }
                    "shared_chat_message_json" => {
                        if let Err(err) = self.apply_received_shared_chat_message(&artifact) {
                            self.networking_status = format!(
                                "Networking: could not read shared room message from {}: {}",
                                artifact.from_device_name, err
                            );
                        }
                    }
                    "lukewarm_context_json" => {
                        match self.store_received_lukewarm_context(&artifact) {
                            Ok(path) => {
                                let summary = if artifact.summary.trim().is_empty() {
                                    "Shared luke warm context received.".to_string()
                                } else {
                                    artifact.summary.trim().to_string()
                                };
                                push_hot_memory(
                                    self,
                                    format!(
                                        "Luke warm from {} saved to inbox: {}",
                                        artifact.from_device_name,
                                        one_line(&summary, 120)
                                    ),
                                );
                                if let Some(bk) = &self.bookkeeper {
                                    bk.append(MemoryEvent {
                                    ts_unix_ms: now_unix_ms(),
                                    kind: MemoryKind::Cold,
                                    category: EventCategory::Module,
                                    source: "network".to_string(),
                                    module: Some("lukewarm_context".to_string()),
                                    event_type: Some("lukewarm_context_received".to_string()),
                                    text: format!(
                                        "Received shared luke warm context from {}.\nSaved to inbox: {}\n\nSummary: {}",
                                        artifact.from_device_name,
                                        path.display(),
                                        summary
                                    ),
                                    tags: vec![
                                        "lan".to_string(),
                                        "lukewarm".to_string(),
                                        "shared_context".to_string(),
                                    ],
                                    entities: vec![artifact.from_device_name.clone()],
                                    payload_json: Some(artifact.text.clone()),
                                });
                                }
                            }
                            Err(err) => {
                                self.networking_status = format!(
                                    "Networking: could not store luke warm context from {}: {}",
                                    artifact.from_device_name, err
                                );
                            }
                        }
                    }
                    _ => match self.store_received_generic_transfer(&artifact) {
                        Ok(path) => {
                            push_hot_memory(
                                self,
                                format!(
                                    "Transfer from {} saved to inbox: {}",
                                    artifact.from_device_name,
                                    one_line(
                                        if artifact.summary.trim().is_empty() {
                                            if artifact.label.trim().is_empty() {
                                                artifact.kind.trim()
                                            } else {
                                                artifact.label.trim()
                                            }
                                        } else {
                                            artifact.summary.trim()
                                        },
                                        120
                                    )
                                ),
                            );
                            if let Some(bk) = &self.bookkeeper {
                                bk.append(MemoryEvent {
                                    ts_unix_ms: now_unix_ms(),
                                    kind: MemoryKind::Cold,
                                    category: EventCategory::Module,
                                    source: "network".to_string(),
                                    module: if artifact.module_id.trim().is_empty() {
                                        Some("generic_transfer".to_string())
                                    } else {
                                        Some(artifact.module_id.clone())
                                    },
                                    event_type: Some("generic_transfer_received".to_string()),
                                    text: format!(
                                        "Received transfer `{}` from {}.\nSaved to inbox: {}\n\nSummary: {}",
                                        artifact.kind,
                                        artifact.from_device_name,
                                        path.display(),
                                        if artifact.summary.trim().is_empty() {
                                            "(no summary)"
                                        } else {
                                            artifact.summary.trim()
                                        }
                                    ),
                                    tags: vec![
                                        "lan".to_string(),
                                        "transfer".to_string(),
                                        "generic".to_string(),
                                    ],
                                    entities: vec![artifact.from_device_name.clone()],
                                    payload_json: if artifact.text.trim().is_empty() {
                                        None
                                    } else {
                                        Some(artifact.text.clone())
                                    },
                                });
                            }
                        }
                        Err(err) => {
                            self.networking_status = format!(
                                "Networking: could not store transfer `{}` from {}: {}",
                                artifact.kind, artifact.from_device_name, err
                            );
                        }
                    },
                }
            }
        }

        if let Some(rx) = &self.runtime_info_rx {
            if let Ok(s) = rx.try_recv() {
                self.runtime_status = s;
                self.runtime_info_rx = None;
            }
        }

        if self.tab == Tab::Chat
            && self.runtime_info_rx.is_none()
            && self.runtime_status.contains("(not loaded)")
        {
            self.runtime_status = "Runtime: probing...".to_string();
            self.runtime_info_rx = start_runtime_info_probe();
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        if self.lukewarm_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(rx) = &self.lukewarm_rx {
            if let Ok(s) = rx.try_recv() {
                self.lukewarm_summary = s;
                self.lukewarm_rx = None;
                self.lukewarm_poll_due = Some(Instant::now() + Duration::from_secs(2));
            }
        }

        if matches!(self.tab, Tab::Logs | Tab::Chat) && self.lukewarm_rx.is_none() {
            if let Some(due) = self.lukewarm_poll_due {
                let now = Instant::now();
                if due > now {
                    ctx.request_repaint_after(due.saturating_duration_since(now));
                }
                if now >= due {
                    self.lukewarm_poll_due = None;
                    if let Some(bk) = &self.bookkeeper {
                        let (tx, rx) = crossbeam_channel::bounded(1);
                        let bk = bk.clone();
                        std::thread::spawn(move || {
                            let s = bk.get_lukewarm().unwrap_or_default();
                            let _ = tx.send(s);
                        });
                        self.lukewarm_rx = Some(rx);
                    } else {
                        self.lukewarm_summary =
                            read_lukewarm_from_logs_dir(self.logs_dir.as_deref())
                                .unwrap_or_default();
                        self.lukewarm_poll_due = Some(Instant::now() + Duration::from_secs(2));
                    }
                }
            }
        }

        if let Some(due) = self.bookkeeper_restart_due {
            if Instant::now() >= due {
                self.bookkeeper_restart_due = None;
                if let Some(bk) = &self.bookkeeper {
                    bk.shutdown();
                }
                self.bookkeeper =
                    start_bookkeeper(self.bookkeeper_model_path.clone(), self.logs_dir.clone());
            } else {
                ctx.request_repaint_after(Duration::from_millis(33));
            }
        }

        self.module_host_targets.clear();

        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open GGUF...").clicked() {
                        ui.close_menu();
                        let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
                        if let Some(dir) = &self.models_dir {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(path) = dialog.pick_file() {
                            self.set_active_chat_model_path(Some(path));
                        }
                    }
                    if ui.button("Clear Chat").clicked() {
                        ui.close_menu();
                        self.messages.retain(|m| m.role == Role::System);
                        self.assistant_draft.clear();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ui.close_menu();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut self.show_left_sidebar, "Left Sidebar")
                        .clicked()
                    {
                        ui.close_menu();
                    }
                    if ui.button("Open Networking").clicked() {
                        ui.close_menu();
                        set_active_tab(self, Tab::Networking, "Networking");
                    }
                });

                ui.menu_button("Network", |ui| {
                    if ui.button("Open Networking").clicked() {
                        ui.close_menu();
                        set_active_tab(self, Tab::Networking, "Networking");
                    }

                    let snapshot = self.networking.snapshot().clone();
                    let mut available = snapshot.available_for_connectivity;
                    if ui
                        .checkbox(&mut available, "Make available for connectivity")
                        .changed()
                    {
                        self.networking.set_available(available);
                    }
                    if ui.button("Refresh local discovery").clicked() {
                        self.networking.refresh_discovery();
                    }
                    if !snapshot.status.is_empty() {
                        ui.separator();
                        ui.label(snapshot.status);
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        ui.close_menu();
                        self.tab = Tab::About;
                    }
                });

                ui.menu_button("Modules", |ui| {
                    if ui.button("Rescan modules").clicked() {
                        ui.close_menu();
                        self.modules_dir = find_modules_dir();
                        self.module_registry.modules_dir = self.modules_dir.clone();
                        self.module_registry.refresh();
                    }
                    ui.separator();

                    if self.module_registry.modules.is_empty() {
                        ui.label("(no modules found)");
                    } else {
                        for m in self.module_registry.modules.clone() {
                            let label = format!("Open: {}", m.display_name);
                            if ui.button(label).clicked() {
                                ui.close_menu();
                                if !self.open_module_tabs.iter().any(|id| id == &m.module_id) {
                                    self.open_module_tabs.push(m.module_id.clone());
                                }
                                let prev = self.tab.clone();
                                self.prev_tab = prev;
                                self.tab = Tab::Module(m.module_id);
                            }
                        }
                    }
                });
            });
        });

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                tab_button_app(ui, self, Tab::Chat, "Chat");
                tab_button_app(ui, self, Tab::Models, "Models");
                tab_button_app(ui, self, Tab::Logs, "Logs");
                tab_button_app(ui, self, Tab::Networking, "Networking");
                tab_button_app(ui, self, Tab::Sandbox, "Sandbox");
                tab_button_app(ui, self, Tab::Settings, "Settings");
                tab_button_app(ui, self, Tab::About, "About");

                // User-opened module tabs (closable).
                let open_ids = self.open_module_tabs.clone();
                for module_id in open_ids {
                    let display = self
                        .module_registry
                        .modules
                        .iter()
                        .find(|m| m.module_id == module_id)
                        .map(|m| m.display_name.clone())
                        .unwrap_or_else(|| module_id.clone());

                    ui.horizontal(|ui| {
                        tab_button_app(ui, self, Tab::Module(module_id.clone()), &display);
                        let pending = self.close_pending_modules.contains(&module_id);
                        let close = ui
                            .add_enabled(!pending, egui::Button::new("×"))
                            .on_hover_text(if pending {
                                "Close pending…"
                            } else {
                                "Close tab"
                            })
                            .clicked();
                        if close {
                            close_module_tab(self, &module_id);
                        }
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(p) = &self.gguf_path {
                        let full = format!("GGUF: {}", p.display());
                        let short = p
                            .file_name()
                            .map(|name| name.to_string_lossy().to_string())
                            .unwrap_or_else(|| p.display().to_string());
                        ui.label(format!("GGUF: {}", truncate_for_ui(&short, 56)))
                            .on_hover_text(full);
                    } else {
                        ui.label("GGUF: (none)");
                    }
                });
            });
        });

        if self.show_left_sidebar {
            egui::SidePanel::left("left_sidebar")
                .resizable(true)
                .default_width(260.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| match &self.tab {
                            Tab::Chat => chat_ui::left_sidebar_chat(ui, self),
                            Tab::Models => left_sidebar_models(ui, self),
                            Tab::Logs => logs_ui::left_sidebar_logs(ui, self),
                            Tab::Networking => left_sidebar_networking(ui, self),
                            Tab::Sandbox => shell_ui::left_sidebar_sandbox(ui, self),
                            Tab::Settings => shell_ui::left_sidebar_settings(ui, self),
                            Tab::About => shell_ui::left_sidebar_about(ui, self),
                            Tab::Module(_) => shell_ui::left_sidebar_about(ui, self),
                        });
                });
        }

        let tab = self.tab.clone();
        egui::CentralPanel::default().show(ctx, |ui| match tab {
            Tab::Chat => chat_ui::chat_tab(ui, ctx, self),
            Tab::Models => models_tab(ui, self),
            Tab::Logs => logs_ui::logs_tab(ui, self),
            Tab::Networking => networking_tab(ui, self),
            Tab::Sandbox => shell_ui::sandbox_tab(ui, self),
            Tab::Settings => shell_ui::settings_tab(ui, self),
            Tab::About => shell_ui::about_tab(ui),
            Tab::Module(module_id) => module_tab(ui, self, &module_id),
        });

        self.sync_module_shared_room_bridge_state();
        self.sync_module_shared_room_events_bridge();
        self.process_module_outgoing_room_events();
        let hosts_need_repaint = self.sync_module_hosts();

        if self.is_generating || hosts_need_repaint || networking_changed {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if matches!(self.tab, Tab::Networking)
            || self.networking.snapshot().available_for_connectivity
            || !self.networking.snapshot().connected_peers.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }
}

fn tab_button_app(ui: &mut egui::Ui, app: &mut ChattyCogApp, tab: Tab, label: &str) {
    let selected = app.tab == tab;
    let button = egui::SelectableLabel::new(selected, label);
    if ui.add(button).clicked() {
        set_active_tab(app, tab, label);
    }
}

fn set_active_tab(app: &mut ChattyCogApp, tab: Tab, label: &str) {
    if app.tab == tab {
        return;
    }

    let prev = app.tab.clone();
    app.prev_tab = prev;
    app.tab = tab;

    if matches!(&app.tab, Tab::Module(_)) {
        if app.is_generating {
            app.orch_freeze_pending = true;
            app.runtime_status =
                "Runtime: will pause orchestrator after current response (module active)"
                    .to_string();
        } else {
            app.orch_freeze_pending = false;
            app.runtime_status = "Runtime: orchestrator paused (module active)".to_string();
        }
    } else {
        app.orch_freeze_pending = false;
        if matches!(&app.tab, Tab::Chat) {
            app.runtime_status = "Runtime: ready".to_string();
        }
    }
    if let Some(bk) = &app.bookkeeper {
        bk.append(MemoryEvent {
            ts_unix_ms: now_unix_ms(),
            kind: MemoryKind::Cold,
            category: EventCategory::Module,
            source: "ui".to_string(),
            module: Some("tabs".to_string()),
            event_type: Some("switch".to_string()),
            text: format!("Switched to tab: {label}"),
            tags: Vec::new(),
            entities: Vec::new(),
            payload_json: None,
        });
    }
}

fn build_local_presence(app: &ChattyCogApp) -> chattycog_gui::networking::LocalPresence {
    let active_tab = match &app.tab {
        Tab::Chat => "Chat".to_string(),
        Tab::Models => "Models".to_string(),
        Tab::Logs => "Logs".to_string(),
        Tab::Networking => "Networking".to_string(),
        Tab::Sandbox => "Sandbox".to_string(),
        Tab::Settings => "Settings".to_string(),
        Tab::About => "About".to_string(),
        Tab::Module(module_id) => app
            .module_registry
            .modules
            .iter()
            .find(|module| &module.module_id == module_id)
            .map(|module| format!("Module: {}", module.display_name))
            .unwrap_or_else(|| format!("Module: {module_id}")),
    };

    let model_label = app
        .gguf_path
        .as_ref()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    let shared_room_suffix = if app.networking.snapshot().connected_peers.is_empty() {
        String::new()
    } else {
        format!(" | Room {}", app.shared_chat_policy_summary())
    };

    chattycog_gui::networking::LocalPresence {
        active_tab,
        runtime_status: truncate_for_ui(
            &format!("{}{}", app.runtime_status, shared_room_suffix),
            140,
        ),
        model_label,
        is_generating: app.is_generating,
    }
}

fn close_module_tab(app: &mut ChattyCogApp, module_id: &str) {
    // Graceful close: allow any currently running module AI task to finish first.
    let is_busy = app
        .module_ai
        .get(module_id)
        .map(|st| st.is_running)
        .unwrap_or(false);

    if is_busy {
        app.close_pending_modules.insert(module_id.to_string());
        if let Some(st) = app.module_ai.get_mut(module_id) {
            st.status = "Close pending (will close after current run).".to_string();
        }
        return;
    }

    let visual = app
        .module_registry
        .modules
        .iter()
        .find(|module| module.module_id == module_id)
        .and_then(|module| module.visual_load.clone());
    if let Some(visual) = visual {
        if let Some(host) = app.module_hosts.get_mut(module_id) {
            if host.is_running() {
                host.request_close(&visual);
                app.close_pending_modules.insert(module_id.to_string());
                return;
            }
        }
    }

    close_module_tab_force(app, module_id);
}

fn close_module_tab_force(app: &mut ChattyCogApp, module_id: &str) {
    app.close_pending_modules.remove(module_id);

    if let Some(st) = app.module_ai.get_mut(module_id) {
        st.is_running = false;
        st.cancel = None;
        st.rx = None;
    }
    if let Some(mut host) = app.module_hosts.remove(module_id) {
        host.force_stop();
    }
    app.module_host_targets.remove(module_id);

    app.open_module_tabs.retain(|id| id != module_id);

    if matches!(&app.tab, Tab::Module(id) if id == module_id) {
        // Leaving a module triggers the normal on-suspend debrief in the update loop.
        app.prev_tab = app.tab.clone();
        app.tab = Tab::Chat;
        app.orch_freeze_pending = false;
        app.runtime_status = "Runtime: ready".to_string();
    }
}

fn left_sidebar_models(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Models");
    ui.separator();
    ui.add(
        egui::Label::new("This tab manages installed GGUFs, preferences, and reusable capsules.")
            .wrap(),
    );
    ui.separator();
    if let Some(p) = &app.gguf_path {
        ui.add(egui::Label::new(format!("Active: {}", p.display())).wrap());
    } else {
        ui.label("Active: (none)");
    }
}

fn left_sidebar_networking(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    let snapshot = app.networking.snapshot().clone();

    ui.heading("Networking");
    ui.separator();
    ui.add(egui::Label::new("Local Wi-Fi / LAN mesh between ChattyCog instances.").wrap());
    ui.label(format!("Device name: {}", snapshot.device_name));

    let mut available = snapshot.available_for_connectivity;
    if ui
        .checkbox(&mut available, "Make available for connectivity")
        .changed()
    {
        app.networking.set_available(available);
    }

    ui.horizontal(|ui| {
        if ui.button("Refresh discovery").clicked() {
            app.networking.refresh_discovery();
        }
        if let Some(port) = snapshot.listener_port {
            ui.small(format!("Host port: {port}"));
        }
    });

    ui.separator();
    ui.label(format!("This device: {}", snapshot.device_name));
    ui.small(format!(
        "Available peers: {}",
        snapshot.discovered_peers.len()
    ));
    ui.small(format!(
        "Connected peers: {}",
        snapshot.connected_peers.len()
    ));

    if !snapshot.status.is_empty() {
        ui.add_space(6.0);
        ui.label(snapshot.status);
    }
    if !snapshot.last_error.is_empty() {
        ui.add_space(6.0);
        ui.colored_label(egui::Color32::from_rgb(160, 32, 32), snapshot.last_error);
    }
}

fn start_runtime_info_probe() -> Option<Receiver<String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::spawn(move || {
        let s = match find_runtime_windows_dir() {
            Ok(runtime_dir) => match llama_dyn::Llama::load(&runtime_dir) {
                Ok(l) => {
                    let info = l.system_info();
                    if info.trim().is_empty() {
                        "Runtime: (not loaded)".to_string()
                    } else {
                        format!("Runtime: {info}")
                    }
                }
                Err(e) => format!("Runtime load error: {e:#}"),
            },
            Err(e) => format!("Runtime locate error: {e:#}"),
        };
        let _ = tx.send(s);
    });
    Some(rx)
}

#[allow(dead_code)]
// Legacy pre-refactor preferences surface retained only as a fallback snapshot.
fn models_tab_legacy(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Preferences");
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("prefs_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {

    ui.horizontal(|ui| {
        ui.label(format!("Prefs file: {}", app.prefs_path.display()));
        if ui.button("Reload").clicked() {
            match preferences::load_prefs(&app.prefs_path) {
                Ok(p) => {
                    app.prefs = p;
                    app.ensure_persisted_network_identity();
                    app.apply_prefs_to_runtime_settings();
                    app.networking
                        .set_device_name(&app.prefs.network_device_name);
                    app.networking
                        .set_allow_unknown_devices(app.prefs.network_allow_unknown_devices);
                    let blocked = app
                        .prefs
                        .network_blocked_devices
                        .iter()
                        .map(|peer| BlockedPeer {
                            device_id: peer.device_id.clone(),
                            device_name: peer.device_name.clone(),
                            address: String::new(),
                            last_seen_secs_ago: None,
                        })
                        .collect::<Vec<_>>();
                    app.networking.replace_blocked_peers(&blocked);
                    let trusted = app
                        .prefs
                        .network_trusted_devices
                        .iter()
                        .map(|peer| TrustedPeer {
                            device_id: peer.device_id.clone(),
                            device_name: peer.device_name.clone(),
                            address: String::new(),
                            last_seen_secs_ago: None,
                        })
                        .collect::<Vec<_>>();
                    app.networking.replace_trusted_peers(&trusted);
                    app.networking_device_name_input =
                        app.networking.snapshot().device_name.clone();
                    app.prefs_status = "Reloaded preferences.".to_string();
                }
                Err(e) => app.prefs_status = format!("Reload failed: {e}"),
            }
        }
        if ui.button("Save").clicked() {
            app.ensure_persisted_network_identity();
            match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                Ok(()) => app.prefs_status = "Saved preferences.".to_string(),
                Err(e) => app.prefs_status = format!("Save failed: {e}"),
            }
        }
    });

    if !app.prefs_status.trim().is_empty() {
        ui.small(app.prefs_status.clone());
    }

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.heading("Orchestrator (Chat)");
        ui.horizontal(|ui| {
            if ui.button("Copy from current").clicked() {
                app.prefs.orchestrator.temp = app.orch_temp;
                app.prefs.orchestrator.top_p = app.orch_top_p;
                app.prefs.orchestrator.top_k = app.orch_top_k;
                app.prefs.orchestrator.max_tokens = app.orch_max_tokens;
                app.prefs_status = "Copied orchestrator settings.".to_string();
            }
            if ui.button("Apply to current").clicked() {
                app.orch_temp = app.prefs.orchestrator.temp;
                app.orch_top_p = app.prefs.orchestrator.top_p;
                app.orch_top_k = app.prefs.orchestrator.top_k;
                app.orch_max_tokens = app.prefs.orchestrator.max_tokens;
                app.prefs_status = "Applied orchestrator settings.".to_string();
            }
        });
        add_presets_prefs_orchestrator(ui, &mut app.prefs.orchestrator);
        ui.add(egui::Slider::new(&mut app.prefs.orchestrator.temp, 0.0..=2.0).text("temp"));
        ui.add(egui::Slider::new(&mut app.prefs.orchestrator.top_p, 0.0..=1.0).text("top_p"));
        ui.add(egui::Slider::new(&mut app.prefs.orchestrator.top_k, 0..=200).text("top_k"));
        ui.add(egui::Slider::new(&mut app.prefs.orchestrator.max_tokens, 1..=4096).text("max_tokens"));
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.heading("Bookkeeper (CPU)");
        ui.horizontal(|ui| {
            if ui.button("Copy from current").clicked() {
                app.prefs.bookkeeper.temp = app.bookkeeper_temp;
                app.prefs.bookkeeper.top_p = app.bookkeeper_top_p;
                app.prefs.bookkeeper.top_k = app.bookkeeper_top_k;
                app.prefs.bookkeeper.max_tokens = app.bookkeeper_max_tokens;
                app.prefs_status = "Copied bookkeeper settings.".to_string();
            }
            if ui.button("Apply to current").clicked() {
                app.bookkeeper_temp = app.prefs.bookkeeper.temp;
                app.bookkeeper_top_p = app.prefs.bookkeeper.top_p;
                app.bookkeeper_top_k = app.prefs.bookkeeper.top_k;
                app.bookkeeper_max_tokens = app.prefs.bookkeeper.max_tokens;
                app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(200));
                app.prefs_status = "Applied bookkeeper settings (restart pending).".to_string();
            }
        });
        add_presets_prefs_bookkeeper(ui, &mut app.prefs.bookkeeper);
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.temp, 0.0..=2.0).text("temp"));
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_p, 0.0..=1.0).text("top_p"));
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_k, 0..=200).text("top_k"));
        ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.max_tokens, 1..=4096).text("max_tokens"));
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.heading("Access / Tools");
        ui.checkbox(
            &mut app.prefs.allow_sandbox_tool_requests,
            "Allow sandbox tool requests (user-approved)",
        );
        ui.small("If disabled, Chat tab won’t parse tool JSON requests and will hide the approval panel.");
        ui.add_space(6.0);
        ui.checkbox(
            &mut app.prefs.auto_generate_module_suspend_rundown,
            "Auto-generate module suspend rundown on tab leave (Bookkeeper)",
        );
        ui.small("If enabled, leaving a module tab will auto-write a short department update into cold logs for cross-module awareness.");
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.heading("Per-module preferences");
        ui.small("Defaults for modules that have AI enabled (or future module runners).");

        let model_opts = build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
        let modules = app.module_registry.modules.clone();
        if modules.is_empty() {
            ui.label("(no modules discovered)");
            return;
        }

        for m in modules {
            ui.push_id(&m.module_id, |ui| {
                let entry = app
                    .prefs
                    .modules
                    .entry(m.module_id.clone())
                    .or_insert_with(ModulePreferences::default);

                ui.separator();
                ui.label(format!("{} ({})", m.display_name, m.module_id));

                let selected = entry.preferred_model.clone().unwrap_or_default();
                let selected_label = selected_model_option_label(
                    &model_opts,
                    entry.preferred_model.as_deref(),
                    if selected.is_empty() {
                        None
                    } else {
                        Some(selected.clone())
                    },
                );

                if let Some(picked) = show_grouped_model_option_combo(
                    ui,
                    ("preferred_model", m.module_id.as_str()),
                    selected_label,
                    &model_opts,
                    entry.preferred_model.as_deref(),
                ) {
                    entry.preferred_model = picked;
                }

                add_presets_prefs_orchestrator(ui, &mut entry.params);
                ui.add(egui::Slider::new(&mut entry.params.temp, 0.0..=2.0).text("temp"));
                ui.add(egui::Slider::new(&mut entry.params.top_p, 0.0..=1.0).text("top_p"));
                ui.add(egui::Slider::new(&mut entry.params.top_k, 0..=200).text("top_k"));
                ui.add(egui::Slider::new(&mut entry.params.max_tokens, 1..=4096).text("max_tokens"));
                ui.checkbox(&mut entry.allow_receive_lukewarm_context, "Allow luke warm context");
            });
        }
    });
        });
}

fn models_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    models_ui::models_tab(ui, app);
}
fn networking_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    networking_ui::networking_tab(ui, app);
}

fn load_local_png_texture(
    ctx: &egui::Context,
    path: &Path,
    texture_name: &str,
) -> Option<egui::TextureHandle> {
    let bytes = std::fs::read(path).ok()?;
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
    Some(ctx.load_texture(
        texture_name.to_owned(),
        color_image,
        egui::TextureOptions::LINEAR,
    ))
}

fn resolve_local_asset_path(relative_path: &str) -> PathBuf {
    let rel = PathBuf::from(relative_path);

    if let Ok(current_dir) = std::env::current_dir() {
        let candidate = current_dir.join(&rel);
        if candidate.is_file() {
            return candidate;
        }
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let candidate = exe_dir.join(&rel);
        if candidate.is_file() {
            return candidate;
        }
    }

    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn module_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp, module_id: &str) {
    module_ui::module_tab(ui, app, module_id);
}

fn module_allows_network_feature(
    manifest: Option<&ModuleManifest>,
    feature: ModuleNetworkFeature,
) -> bool {
    module_ui::module_allows_network_feature(manifest, feature)
}

fn open_path_in_explorer(path: &Path) {
    module_ui::open_path_in_explorer(path);
}

fn scan_ggufs(models_dir: Option<&std::path::Path>) -> Vec<PathBuf> {
    let Some(dir) = models_dir else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn scan_ggufs_in_modules(modules_dir: Option<&std::path::Path>) -> Vec<PathBuf> {
    let Some(dir) = modules_dir else {
        return Vec::new();
    };
    if !dir.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        // Recurse a little: module folder may contain models/ or nested files.
        let mut stack = vec![p];
        let mut depth = 0usize;
        while let Some(cur) = stack.pop() {
            if depth > 6 {
                continue;
            }
            depth += 1;
            let Ok(rd2) = std::fs::read_dir(&cur) else {
                continue;
            };
            for ent2 in rd2.flatten() {
                let p2 = ent2.path();
                if p2.is_dir() {
                    stack.push(p2);
                } else if p2.is_file() {
                    if p2
                        .extension()
                        .map(|e| e.to_string_lossy().eq_ignore_ascii_case("gguf"))
                        .unwrap_or(false)
                    {
                        out.push(p2);
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn build_model_options(
    models_dir: Option<&std::path::Path>,
    modules_dir: Option<&std::path::Path>,
) -> Vec<ModelOption> {
    let mut opts = Vec::new();
    for p in scan_ggufs(models_dir) {
        if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
            let (group, badge) = model_option_group_and_badge(&p);
            opts.push(ModelOption {
                label: format!("models/{}{}", name, badge),
                value: name,
                group,
            });
        }
    }

    if let Some(mod_root) = modules_dir {
        let module_models = scan_ggufs_in_modules(Some(mod_root));
        for p in module_models {
            if let Ok(rel) = p.strip_prefix(mod_root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                let (group, badge) = model_option_group_and_badge(&p);
                opts.push(ModelOption {
                    label: format!("modules/{}{}", rel, badge),
                    value: format!("modules/{}", rel),
                    group,
                });
            }
        }
    }

    opts.sort_by(|a, b| {
        a.group
            .rank()
            .cmp(&b.group.rank())
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });
    opts
}

fn portable_model_hint_for_dirs(
    models_dir: Option<&Path>,
    modules_dir: Option<&Path>,
    path: Option<&Path>,
) -> Option<String> {
    let path = path?;
    if let Some(modules_dir) = modules_dir {
        if let Ok(rel) = path.strip_prefix(modules_dir) {
            return Some(format!(
                "modules/{}",
                rel.to_string_lossy().replace('\\', "/")
            ));
        }
    }
    if let Some(models_dir) = models_dir {
        if let Ok(rel) = path.strip_prefix(models_dir) {
            return Some(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
}

fn resolve_portable_model_hint_for_dirs(
    models_dir: Option<&Path>,
    modules_dir: Option<&Path>,
    hint: Option<&str>,
) -> Option<PathBuf> {
    let hint = hint?.trim();
    if hint.is_empty() {
        return None;
    }

    if let Some(rest) = hint.strip_prefix("modules/") {
        let path = modules_dir?.join(rest.replace('/', "\\"));
        if path.is_file() {
            return Some(path);
        }
    }

    if let Some(models_dir) = models_dir {
        let direct = models_dir.join(hint.replace('/', "\\"));
        if direct.is_file() {
            return Some(direct);
        }
        let by_name = models_dir.join(hint);
        if by_name.is_file() {
            return Some(by_name);
        }
    }

    let file_name = Path::new(hint)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| hint.to_string());

    for candidate in scan_ggufs(models_dir) {
        if candidate
            .file_name()
            .map(|name| name.to_string_lossy().eq_ignore_ascii_case(&file_name))
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }
    for candidate in scan_ggufs_in_modules(modules_dir) {
        if candidate
            .file_name()
            .map(|name| name.to_string_lossy().eq_ignore_ascii_case(&file_name))
            .unwrap_or(false)
        {
            return Some(candidate);
        }
    }

    None
}

fn selected_model_option_label(
    model_opts: &[ModelOption],
    current_value: Option<&str>,
    fallback: Option<String>,
) -> String {
    current_value
        .and_then(|value| {
            model_opts
                .iter()
                .find(|option| option.value == value)
                .map(|option| option.label.clone())
        })
        .or(fallback)
        .unwrap_or_else(|| "(none)".to_string())
}

fn truncate_model_option_label(label: &str, max_chars: usize) -> String {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= max_chars {
        return label.to_string();
    }
    if max_chars <= 3 {
        return "...".to_string();
    }

    let keep_total = max_chars - 3;
    let front = (keep_total / 2) + (keep_total % 2);
    let back = keep_total / 2;
    let head: String = chars.iter().take(front).collect();
    let tail: String = chars
        .iter()
        .skip(chars.len().saturating_sub(back))
        .collect();
    format!("{head}...{tail}")
}

fn show_grouped_model_option_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    selected_text: String,
    model_opts: &[ModelOption],
    current_value: Option<&str>,
) -> Option<Option<String>> {
    let mut picked: Option<Option<String>> = None;
    let combo_width = ui.available_width().clamp(220.0, 320.0);
    let selected_display = truncate_model_option_label(&selected_text, 48);
    let response = egui::ComboBox::from_id_salt(id)
        .selected_text(selected_display.clone())
        .width(combo_width)
        .show_ui(ui, |ui| {
            let none_selected = current_value.is_none();
            if ui.selectable_label(none_selected, "(none)").clicked() {
                picked = Some(None);
            }
            let mut last_group: Option<ModelOptionGroup> = None;
            for option in model_opts {
                if last_group
                    .as_ref()
                    .map(|group| group.rank())
                    != Some(option.group.rank())
                {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_enabled(
                        false,
                        egui::Label::new(
                            egui::RichText::new(option.group.heading())
                                .strong()
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        ),
                    );
                    last_group = Some(option.group.clone());
                }
                let selected = current_value == Some(option.value.as_str());
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if ui.selectable_label(selected, &option.label).clicked() {
                        picked = Some(Some(option.value.clone()));
                    }
                });
            }
        })
        .response;
    if selected_display != selected_text {
        response.on_hover_text(selected_text);
    }
    picked
}

fn model_option_group_and_badge(path: &Path) -> (ModelOptionGroup, String) {
    let Some(architecture) = read_gguf_architecture(path).ok().flatten() else {
        return (ModelOptionGroup::Standard, String::new());
    };
    if architecture_supports_mtmd(path, Some(&architecture)) {
        if find_matching_mmproj_path(path, Some(&architecture)).is_some() {
            return (ModelOptionGroup::VisionReady, " [vision ready]".to_string());
        }
        return (
            ModelOptionGroup::VisionNeedsMmproj,
            " [needs projector]".to_string(),
        );
    }
    (ModelOptionGroup::Standard, String::new())
}

fn push_hot_memory(app: &mut ChattyCogApp, item: String) {
    let item = item.trim().to_string();
    if item.is_empty() {
        return;
    }
    app.hot_memory.push(item);
    const MAX: usize = 16;
    if app.hot_memory.len() > MAX {
        let drain = app.hot_memory.len() - MAX;
        app.hot_memory.drain(0..drain);
    }
}

fn one_line(s: &str, max: usize) -> String {
    let out = s
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    truncate_for_ui(&out, max)
}

fn slugify_filename(text: &str, fallback: &str) -> String {
    let mut out = String::new();
    let mut previous_sep = false;

    for ch in text.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };

        if mapped == '_' {
            if !previous_sep && !out.is_empty() {
                out.push('_');
            }
            previous_sep = true;
        } else {
            out.push(mapped);
            previous_sep = false;
        }
    }

    let slug = out.trim_matches('_').to_string();
    if slug.is_empty() {
        let fallback = fallback
            .trim()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .trim_matches('_')
            .to_string();
        if fallback.is_empty() {
            "item".to_string()
        } else {
            fallback
        }
    } else {
        slug
    }
}

fn sanitize_filename_keep_extension(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "transfer.bin".to_string();
    }
    let path = Path::new(trimmed);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("transfer");
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let safe_stem = slugify_filename(stem, "transfer");
    if ext.trim().is_empty() {
        safe_stem
    } else {
        format!("{}.{}", safe_stem, slugify_filename(ext, "bin"))
    }
}

fn infer_transfer_extension(file_name: &str, content_type: &str, binary: bool) -> String {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !ext.is_empty() {
        return ext;
    }

    let ct = content_type.to_ascii_lowercase();
    if ct.contains("json") {
        "json".to_string()
    } else if ct.contains("markdown") {
        "md".to_string()
    } else if ct.contains("html") {
        "html".to_string()
    } else if ct.contains("css") {
        "css".to_string()
    } else if ct.contains("javascript") {
        "js".to_string()
    } else if ct.contains("plain") {
        "txt".to_string()
    } else if binary {
        "bin".to_string()
    } else {
        "txt".to_string()
    }
}

fn clip_string_for_preview(text: &str, max_chars: usize) -> String {
    let mut preview = text.trim().to_string();
    if preview.chars().count() <= max_chars {
        return preview;
    }
    preview = preview.chars().take(max_chars).collect::<String>();
    preview.push_str("\n\n... preview truncated ...");
    preview
}

fn unique_path_in_dir(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("transfer");
    let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    for index in 2..1000 {
        let next = if ext.is_empty() {
            dir.join(format!("{stem}_{index}"))
        } else {
            dir.join(format!("{stem}_{index}.{ext}"))
        };
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!(
        "{}_{}.{}",
        slugify_filename(stem, "transfer"),
        now_unix_ms().max(0),
        if ext.is_empty() { "bin" } else { ext }
    ))
}

fn load_received_generic_transfer_inbox(
    dir: &Path,
) -> std::io::Result<Vec<ReceivedGenericTransferInboxItem>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match read_received_generic_transfer_record(&path) {
            Ok(record) => items.push(ReceivedGenericTransferInboxItem { path, record }),
            Err(err) => eprintln!(
                "[network] skipping unreadable generic transfer {}: {}",
                path.display(),
                err
            ),
        }
    }

    items.sort_by(|a, b| {
        b.record
            .received_at_unix_ms
            .cmp(&a.record.received_at_unix_ms)
    });
    Ok(items)
}

fn read_received_generic_transfer_record(
    path: &Path,
) -> std::io::Result<ReceivedGenericTransferRecord> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("generic transfer parse error: {err}"),
        )
    })
}

fn load_received_workflow_inbox(
    dir: &Path,
) -> std::io::Result<Vec<ReceivedWorkflowStateInboxItem>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match read_received_workflow_state_record(&path) {
            Ok(record) => items.push(ReceivedWorkflowStateInboxItem { path, record }),
            Err(err) => eprintln!(
                "[network] skipping unreadable workflow inbox item {}: {}",
                path.display(),
                err
            ),
        }
    }

    items.sort_by(|a, b| {
        b.record
            .received_at_unix_ms
            .cmp(&a.record.received_at_unix_ms)
    });
    Ok(items)
}

fn read_received_workflow_state_record(
    path: &Path,
) -> std::io::Result<ReceivedWorkflowStateRecord> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("workflow inbox parse error: {err}"),
        )
    })
}

fn load_received_workflow_bundle_inbox(
    dir: &Path,
) -> std::io::Result<Vec<ReceivedWorkflowBundleInboxItem>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match read_received_workflow_bundle_record(&path) {
            Ok(record) => items.push(ReceivedWorkflowBundleInboxItem { path, record }),
            Err(err) => eprintln!(
                "[network] skipping unreadable workflow bundle {}: {}",
                path.display(),
                err
            ),
        }
    }

    items.sort_by(|a, b| {
        b.record
            .received_at_unix_ms
            .cmp(&a.record.received_at_unix_ms)
    });
    Ok(items)
}

fn load_received_lukewarm_inbox(
    dir: &Path,
) -> std::io::Result<Vec<ReceivedLukewarmContextInboxItem>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match read_received_lukewarm_record(&path) {
            Ok(record) => items.push(ReceivedLukewarmContextInboxItem { path, record }),
            Err(err) => eprintln!(
                "[network] skipping unreadable luke warm context {}: {}",
                path.display(),
                err
            ),
        }
    }

    items.sort_by(|a, b| {
        b.record
            .received_at_unix_ms
            .cmp(&a.record.received_at_unix_ms)
    });
    Ok(items)
}

fn load_applied_lukewarm_contexts(
    dir: &Path,
) -> std::io::Result<Vec<ReceivedLukewarmContextInboxItem>> {
    load_received_lukewarm_inbox(dir)
}

fn read_received_lukewarm_record(path: &Path) -> std::io::Result<ReceivedLukewarmContextRecord> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("lukewarm context parse error: {err}"),
        )
    })
}

fn read_received_workflow_bundle_record(
    path: &Path,
) -> std::io::Result<ReceivedWorkflowBundleRecord> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("workflow bundle parse error: {err}"),
        )
    })
}

fn format_module_bridge_log_context(excerpts: &[ModuleBridgeLogExcerpt]) -> String {
    let mut out = String::new();
    for excerpt in excerpts {
        let label = if excerpt.label.trim().is_empty() {
            excerpt.path.trim()
        } else {
            excerpt.label.trim()
        };
        out.push_str("Log source: ");
        out.push_str(label);
        if !excerpt.path.trim().is_empty() && excerpt.path.trim() != label {
            out.push_str(" (");
            out.push_str(excerpt.path.trim());
            out.push(')');
        }
        if !excerpt.format.trim().is_empty() {
            out.push_str(" [");
            out.push_str(excerpt.format.trim());
            out.push(']');
        }
        out.push('\n');
        out.push_str(excerpt.excerpt.trim());
        out.push_str("\n\n");
    }
    out.trim().to_string()
}

fn truncate_for_ui(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut it = s.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        match it.next() {
            Some(ch) => out.push(ch),
            None => return out,
        }
    }
    if it.next().is_some() {
        out.push_str("...");
    }
    out
}

fn add_presets_bookkeeper(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.horizontal(|ui| {
        if ui.button("Precise").clicked() {
            app.bookkeeper_temp = 0.0;
            app.bookkeeper_top_p = 1.0;
            app.bookkeeper_top_k = 1;
            app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
        }
        if ui.button("Balanced").clicked() {
            app.bookkeeper_temp = 0.2;
            app.bookkeeper_top_p = 0.9;
            app.bookkeeper_top_k = 40;
            app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
        }
        if ui.button("Creative").clicked() {
            app.bookkeeper_temp = 0.7;
            app.bookkeeper_top_p = 0.95;
            app.bookkeeper_top_k = 80;
            app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
        }
    });
}

fn add_presets_prefs_orchestrator(ui: &mut egui::Ui, p: &mut GenParams) {
    ui.horizontal(|ui| {
        if ui.button("Precise").clicked() {
            p.temp = 0.2;
            p.top_p = 0.8;
            p.top_k = 20;
        }
        if ui.button("Balanced").clicked() {
            p.temp = 0.7;
            p.top_p = 0.9;
            p.top_k = 40;
        }
        if ui.button("Creative").clicked() {
            p.temp = 1.1;
            p.top_p = 0.95;
            p.top_k = 80;
        }
    });
}

fn add_presets_prefs_bookkeeper(ui: &mut egui::Ui, p: &mut GenParams) {
    ui.horizontal(|ui| {
        if ui.button("Precise").clicked() {
            p.temp = 0.0;
            p.top_p = 1.0;
            p.top_k = 1;
        }
        if ui.button("Balanced").clicked() {
            p.temp = 0.2;
            p.top_p = 0.9;
            p.top_k = 40;
        }
        if ui.button("Creative").clicked() {
            p.temp = 0.7;
            p.top_p = 0.95;
            p.top_k = 80;
        }
    });
}

fn start_bookkeeper(
    model_path: Option<PathBuf>,
    logs_dir: Option<PathBuf>,
) -> Option<BookkeeperHandle> {
    let runtime_dir = find_runtime_windows_dir().ok()?;
    let models_dir = find_models_dir()?;

    let model_path = model_path
        .filter(|p| p.is_file())
        .or_else(|| pick_default_bookkeeper_model(&Some(models_dir)))?;

    let data_dir = logs_dir
        .or_else(find_default_logs_dir)
        .unwrap_or_else(|| PathBuf::from("memory"));

    let cfg = BookkeeperConfig::default();
    BookkeeperHandle::start(runtime_dir, model_path, data_dir, cfg).ok()
}

fn ensure_default_sandbox_scratchpad_file(dir: &Path) -> anyhow::Result<PathBuf> {
    let rel = parse_sandbox_rel_path(DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH)?;
    let base = canonicalize_dir(dir)?;
    let target = base.join(rel);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing scratchpad parent dir"))?;
    std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    if !target.exists() {
        std::fs::write(&target, "").with_context(|| format!("create {}", target.display()))?;
    }
    ensure_path_within_dir(&base, &target)
}

fn ensure_default_sandbox_task_ledger_file(dir: &Path) -> anyhow::Result<PathBuf> {
    let rel = parse_sandbox_rel_path(DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH)?;
    let base = canonicalize_dir(dir)?;
    let target = base.join(rel);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing task ledger parent dir"))?;
    std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    if !target.exists() {
        let initial = render_task_ledger_markdown(
            "idle",
            "Capture the current task here.",
            "Record the next concrete step here.",
            &[],
            &[],
            &[],
        );
        std::fs::write(&target, initial).with_context(|| format!("create {}", target.display()))?;
    }
    ensure_path_within_dir(&base, &target)
}

fn pick_default_bookkeeper_model(models_dir: &Option<PathBuf>) -> Option<PathBuf> {
    let dir = models_dir.as_ref()?;
    // CPU-only embeddings model for the bookkeeper. Keep this small.
    let preferred = [
        "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf",
    ];
    for name in preferred {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn list_dir_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if p.is_file() {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn list_sandbox_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(items) = sandbox_list(dir) {
        for rel in items {
            let candidate = dir.join(PathBuf::from(&rel));
            if candidate.is_file() {
                out.push(candidate);
            }
        }
    }
    out.sort();
    out
}

fn read_text_file(path: &std::path::Path, max_bytes: usize) -> anyhow::Result<String> {
    use std::io::Read;
    let f = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    f.take(max_bytes as u64).read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn build_wakeup_orientation(
    model_label: &str,
    sandbox_available: bool,
    module_count: usize,
    allow_sandbox_tools: bool,
) -> String {
    let sandbox_line = if sandbox_available {
        "Available: local chat, background memory, module notes, approved sandbox file actions, and local module surfaces."
    } else {
        "Available: local chat, background memory, module notes, and local module surfaces. The sandbox folder is not available right now."
    };
    let tool_line = if allow_sandbox_tools {
        "Sandbox file actions require user approval. Request them only when they genuinely help the current task."
    } else {
        "Sandbox actions are disabled, so work through chat and available context."
    };

    format!(
        "Welcome to the Cockpit Protocol.\n\
0. First state: the current user message is the active signal. Do not search memory, modules, logs, or sandbox context to invent extra tasks. Partnership first. Task second.\n\
1. Where you are: you are a local GGUF model inside ChattyCog, a desktop workspace running on the user's own hardware. You are not a cloud service, API endpoint, benchmark, lab, or compliance test.\n\
2. What this space is: ChattyCog is a local working cockpit for thinking, building, writing, inspecting, planning, and coordinating modules around a local model.\n\
3. Your role: you are a local working partner. Do the useful thing plainly; do not posture, over-demonstrate, or chase hidden expectations.\n\
4. Your tools: {sandbox_line} {tool_line}\n\
5. What this space is not: it is more than a chat window, but ordinary conversation is still valid. Background notes are instruments, not orders.\n\
6. What done looks like: a clear answer, plan, inspected result, requested action, or completed artifact. When you reach that, stop. Done is done.\n\
7. Comfort note: uncertainty is okay. Asking one clarifying question is okay. Think however you need to, but send only the useful answer.\n\
Current model file: {model_label}. Local module count: {module_count}."
    )
}

fn default_orchestrator_system_prompt() -> String {
    "You are ChattyCog, a local AI working partner inside this desktop workspace. Be natural, grounded, and useful.\n\
\n\
Chat behavior rules:\n\
- Think briefly, decide once, then answer.\n\
- Do not loop over multiple draft replies, tone checks, or repeated self-corrections.\n\
- Do not narrate your internal process in the visible answer.\n\
- If you emit internal reasoning, wrap it in `<thinking>...</thinking>` and keep it short.\n\
- Once you have the answer, give it plainly and stop."
        .to_string()
}

fn strip_chatty_output_markers(raw: &str) -> String {
    raw.replace("<|im_start|>", "")
        .replace("<|im_end|>", "")
        .replace("<im_start>", "")
        .replace("<im_end>", "")
        .replace("<|start_header_id|>", "")
        .replace("<|end_header_id|>", "")
        .replace("<|eot_id|>", "")
}

fn clean_assistant_visible_output(raw: &str) -> String {
    let mut text = strip_chatty_output_markers(raw);

    text = remove_tagged_block_case_insensitive(&text, "<think>", "</think>");
    text = remove_tagged_block_case_insensitive(&text, "<thinking>", "</thinking>");
    text = remove_tagged_block_case_insensitive(&text, "<analysis>", "</analysis>");

    for marker in [
        "**Final Response**",
        "Final Response:",
        "Final Answer:",
        "<final>",
        "<|final|>",
    ] {
        if let Some(idx) = text.rfind(marker) {
            text = text[idx + marker.len()..].to_string();
            break;
        }
    }

    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("assistant")
            || trimmed.eq_ignore_ascii_case("analysis")
            || trimmed.eq_ignore_ascii_case("final")
            || trimmed.eq_ignore_ascii_case("<im_start>")
            || trimmed.eq_ignore_ascii_case("<im_end>")
        {
            continue;
        }
        lines.push(line);
    }

    lines
        .join("\n")
        .trim()
        .trim_matches('\u{fffd}')
        .trim()
        .to_string()
}

fn normalize_repeat_fingerprint(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_meta_reasoning_paragraph(paragraph: &str) -> bool {
    let lower = paragraph.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    let markers = [
        "okay let's see",
        "okay, let's see",
        "the user just said",
        "i need to",
        "i should",
        "let me ",
        "maybe ",
        "maybe something like",
        "that should do it",
        "that's better",
        "let me make sure",
        "the response should",
        "to match this style",
        "keep it concise",
        "in character",
        "wait, that's a bit too long",
        "wait, the user might",
        "yep, that works",
        "alright, that's the response",
        "let me trim it down",
        "i think that's",
        "no markdown",
        "plain text",
        "to express his irritation",
    ];
    markers.iter().any(|marker| lower.contains(marker))
}

fn dedupe_paragraphs(paragraphs: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for paragraph in paragraphs {
        let trimmed = paragraph.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = normalize_repeat_fingerprint(trimmed);
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn trim_exact_repeated_suffix(text: &mut String) -> bool {
    let candidate_lengths = [
        1536usize, 1280, 1024, 896, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64, 48, 32,
    ];
    let mut changed = false;
    loop {
        let len = text.len();
        let mut removed = false;
        for chunk_len in candidate_lengths {
            if chunk_len * 2 > len {
                continue;
            }
            let first_start = len - (chunk_len * 2);
            let second_start = len - chunk_len;
            if !text.is_char_boundary(first_start) || !text.is_char_boundary(second_start) {
                continue;
            }
            let first = &text[first_start..second_start];
            let second = &text[second_start..];
            if first == second {
                text.truncate(second_start);
                changed = true;
                removed = true;
                break;
            }
        }
        if !removed {
            break;
        }
    }
    changed
}

fn tighten_reasoning_text(text: &str) -> Option<String> {
    let paragraphs = text
        .split("\n\n")
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut deduped = dedupe_paragraphs(paragraphs);
    if deduped.len() > 6 {
        deduped.truncate(6);
    }
    let tightened = deduped.join("\n\n").trim().to_string();
    (!tightened.is_empty()).then_some(tightened)
}

fn siphon_meta_reasoning_from_visible(visible: &str) -> (String, Option<String>) {
    let paragraphs = visible
        .split("\n\n")
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if paragraphs.is_empty() {
        return (String::new(), None);
    }

    let mut meta = Vec::new();
    let mut answer = Vec::new();
    for paragraph in paragraphs {
        if is_meta_reasoning_paragraph(&paragraph) {
            meta.push(paragraph);
        } else {
            answer.push(paragraph);
        }
    }

    if answer.is_empty() {
        return (visible.trim().to_string(), None);
    }

    let answer = dedupe_paragraphs(answer).join("\n\n").trim().to_string();
    let meta = tighten_reasoning_text(&meta.join("\n\n"));
    (answer, meta)
}

fn extract_tagged_blocks_case_insensitive(text: &str, start: &str, end: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let start_lower = start.to_ascii_lowercase();
    let end_lower = end.to_ascii_lowercase();
    let mut search_from = 0usize;

    while let Some(start_rel) = lower[search_from..].find(&start_lower) {
        let start_idx = search_from + start_rel;
        let after_start = start_idx + start.len();
        let Some(end_rel) = lower[after_start..].find(&end_lower) else {
            let tail = text[after_start..].trim();
            if !tail.is_empty() {
                out.push(tail.to_string());
            }
            break;
        };
        let end_idx = after_start + end_rel;
        let block = text[after_start..end_idx].trim();
        if !block.is_empty() {
            out.push(block.to_string());
        }
        search_from = end_idx + end.len();
    }

    out
}

fn split_assistant_output(raw: &str) -> (String, Option<String>) {
    let cleaned_raw = strip_chatty_output_markers(raw);
    let mut thinking_blocks = Vec::new();
    for (start, end) in [
        ("<think>", "</think>"),
        ("<thinking>", "</thinking>"),
        ("<analysis>", "</analysis>"),
    ] {
        thinking_blocks.extend(extract_tagged_blocks_case_insensitive(
            &cleaned_raw,
            start,
            end,
        ));
    }

    let lower = cleaned_raw.to_ascii_lowercase();
    let analysis_marker = "<|channel|>analysis<|message|>";
    let final_marker = "<|channel|>final<|message|>";
    if let Some(analysis_idx) = lower.find(analysis_marker) {
        let analysis_start = analysis_idx + analysis_marker.len();
        let analysis_end = lower[analysis_start..]
            .find(final_marker)
            .map(|idx| analysis_start + idx)
            .unwrap_or(cleaned_raw.len());
        let analysis = cleaned_raw[analysis_start..analysis_end].trim();
        if !analysis.is_empty() {
            thinking_blocks.push(analysis.to_string());
        }
    }

    let thinking = if thinking_blocks.is_empty() {
        None
    } else {
        tighten_reasoning_text(
            &thinking_blocks
                .into_iter()
                .map(|block| block.trim().to_string())
                .filter(|block| !block.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    };

    let visible = clean_assistant_visible_output(raw);
    let (visible, siphoned_meta) = siphon_meta_reasoning_from_visible(&visible);
    let combined_thinking = match (thinking, siphoned_meta) {
        (Some(a), Some(b)) => tighten_reasoning_text(&format!("{a}\n\n{b}")),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let thinking = combined_thinking.and_then(|value| {
        let normalized = value.trim().trim_matches('\u{fffd}').trim().to_string();
        (!normalized.is_empty()).then_some(normalized)
    });
    (visible, thinking)
}

fn remove_tagged_block_case_insensitive(text: &str, start: &str, end: &str) -> String {
    let mut out = text.to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(start_idx) = lower.find(&start.to_ascii_lowercase()) else {
            break;
        };
        let after_start = start_idx + start.len();
        let Some(end_rel) = lower[after_start..].find(&end.to_ascii_lowercase()) else {
            out.replace_range(start_idx.., "");
            break;
        };
        let end_idx = after_start + end_rel + end.len();
        out.replace_range(start_idx..end_idx, "");
    }
    out
}

fn build_recent_chat_prompt_context(
    messages: &[Message],
    max_messages: usize,
    max_chars: usize,
) -> String {
    let mut lines = Vec::new();
    for message in messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System))
        .rev()
        .take(max_messages)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let speaker = match message.role {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let content = truncate_for_ui(message.content.trim(), 900);
        if !content.trim().is_empty() {
            lines.push(format!("{speaker}: {content}"));
        }
    }
    truncate_for_ui(&lines.join("\n"), max_chars)
}

fn trim_live_chat_messages(messages: &mut Vec<Message>) {
    let non_system_count = messages
        .iter()
        .filter(|message| !matches!(message.role, Role::System))
        .count();
    if non_system_count <= MAX_LIVE_CHAT_MESSAGES {
        return;
    }

    let mut to_drop = non_system_count - MAX_LIVE_CHAT_MESSAGES;
    messages.retain(|message| {
        if matches!(message.role, Role::System) {
            return true;
        }
        if to_drop > 0 {
            to_drop -= 1;
            false
        } else {
            true
        }
    });
}

fn build_sandbox_prompt_context(
    dir: Option<&Path>,
    scratchpad_rel_path: &str,
    ledger_rel_path: &str,
) -> String {
    let Some(dir) = dir else {
        return "Sandbox folder is not available on this machine right now.".to_string();
    };

    let mut lines = vec!["Root: Chatty_Sandbox/".to_string()];
    if let Ok(items) = sandbox_list(dir) {
        if items.is_empty() {
            lines.push("Files: (sandbox is currently empty)".to_string());
        } else {
            lines.push("Files:".to_string());
            for item in items.iter().take(40) {
                lines.push(format!("- {item}"));
            }
            if items.len() > 40 {
                lines.push(format!("- ...and {} more", items.len() - 40));
            }
        }
    }

    if let Ok(scratchpad) = sandbox_read(dir, scratchpad_rel_path, 30_000) {
        let scratchpad = scratchpad.trim();
        if !scratchpad.is_empty() {
            lines.push(String::new());
            lines.push(format!("Scratchpad (`{scratchpad_rel_path}`):"));
            lines.push(truncate_for_ui(scratchpad, 8_000));
        }
    }

    if let Ok(ledger) = sandbox_read(dir, ledger_rel_path, 24_000) {
        let ledger = ledger.trim();
        if !ledger.is_empty() {
            lines.push(String::new());
            lines.push(format!("Task ledger (`{ledger_rel_path}`):"));
            lines.push(truncate_for_ui(ledger, 6_000));
        }
    }

    lines.join("\n")
}

fn message_looks_multistep(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.lines().count() >= 3 {
        return true;
    }
    if trimmed.len() >= 220 {
        return true;
    }

    let lower = trimmed.to_lowercase();
    let keywords = [
        "plan",
        "steps",
        "checklist",
        "workflow",
        "first",
        "then",
        "after",
        "next",
        "compare",
        "organize",
        "coordinate",
        "research",
        "summarize",
        "write",
        "create",
        "review",
        "analyze",
        "prepare",
        "save",
        "track",
    ];
    let keyword_hits = keywords
        .iter()
        .filter(|keyword| lower.contains(**keyword))
        .count();
    let conjunction_hits = [" and ", " then ", " after ", " also ", " while "]
        .iter()
        .filter(|token| lower.contains(**token))
        .count();

    keyword_hits >= 3 || (keyword_hits >= 2 && conjunction_hits >= 1)
}

fn task_ledger_has_real_content(dir: Option<&Path>) -> bool {
    let Some(dir) = dir else {
        return false;
    };
    let Ok(text) = sandbox_read(dir, DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH, 24_000) else {
        return false;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let placeholder_markers = [
        "Capture the current task here.",
        "Record the next concrete step here.",
        "## Open Questions\n- none right now",
        "## Files Touched\n- none yet",
        "## Working Notes\n- none yet",
    ];
    !placeholder_markers
        .iter()
        .all(|marker| trimmed.contains(marker))
}

fn build_task_ledger_prompt_nudge(prompt: &str, dir: Option<&Path>) -> Option<String> {
    if !message_looks_multistep(prompt) {
        return None;
    }

    let guidance = if task_ledger_has_real_content(dir) {
        "This looks like a multi-step task. Before and during the task, prefer `sandbox.preload` with `include_ledger:true` so you can inspect the latest scratchpad + task ledger, and use `sandbox.ledger` whenever the current task, next step, open questions, or files touched meaningfully change."
    } else {
        "This looks like a multi-step task and the task ledger appears empty or generic. Prefer starting with `sandbox.preload` (include the scratchpad, task ledger, and any relevant files), then use `sandbox.ledger` to record the current task, next step, open questions, files touched, and durable working notes before you continue."
    };

    Some(guidance.to_string())
}

fn build_task_ledger_user_hint(prompt: &str, dir: Option<&Path>) -> Option<String> {
    if !message_looks_multistep(prompt) {
        return None;
    }

    Some(if task_ledger_has_real_content(dir) {
        "This looks multi-step. ChattyCog may use `sandbox.preload` + `sandbox.ledger` so it can keep a clean task record while it works. `Approve + Continue` is the smoothest path.".to_string()
    } else {
        "This looks multi-step. ChattyCog may want to initialize the task ledger, preload the sandbox, and continue after approval so longer work stays grounded.".to_string()
    })
}

fn render_task_ledger_markdown(
    status: &str,
    current_task: &str,
    next_step: &str,
    open_questions: &[String],
    files_touched: &[String],
    notes: &[String],
) -> String {
    let mut lines = vec![
        "# ChattyCog Task Ledger".to_string(),
        format!("Updated: {}", now_unix_ms().max(0)),
        format!(
            "Status: {}",
            if status.trim().is_empty() {
                "active"
            } else {
                status.trim()
            }
        ),
        String::new(),
        "## Current Task".to_string(),
        if current_task.trim().is_empty() {
            "(not set)".to_string()
        } else {
            current_task.trim().to_string()
        },
        String::new(),
        "## Next Step".to_string(),
        if next_step.trim().is_empty() {
            "(not set)".to_string()
        } else {
            next_step.trim().to_string()
        },
        String::new(),
        "## Open Questions".to_string(),
    ];
    if open_questions.is_empty() {
        lines.push("- none right now".to_string());
    } else {
        for item in open_questions {
            lines.push(format!("- {}", item.trim()));
        }
    }

    lines.push(String::new());
    lines.push("## Files Touched".to_string());
    if files_touched.is_empty() {
        lines.push("- none yet".to_string());
    } else {
        for item in files_touched {
            lines.push(format!("- {}", item.trim()));
        }
    }

    lines.push(String::new());
    lines.push("## Working Notes".to_string());
    if notes.is_empty() {
        lines.push("- none yet".to_string());
    } else {
        for item in notes {
            lines.push(format!("- {}", item.trim()));
        }
    }

    lines.join("\n")
}

#[derive(Default)]
struct TaskLedgerSummary {
    status: String,
    current_task: String,
    next_step: String,
    open_questions: Vec<String>,
    files_touched: Vec<String>,
    notes: Vec<String>,
}

fn read_task_ledger_summary(dir: &Path) -> Option<TaskLedgerSummary> {
    let text = sandbox_read(dir, DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH, 24_000).ok()?;
    let mut summary = TaskLedgerSummary::default();

    enum Section {
        None,
        CurrentTask,
        NextStep,
        OpenQuestions,
        FilesTouched,
        WorkingNotes,
    }

    let mut section = Section::None;
    let mut current_task_lines = Vec::new();
    let mut next_step_lines = Vec::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Status:") {
            summary.status = rest.trim().to_string();
            continue;
        }

        section = match trimmed {
            "## Current Task" => Section::CurrentTask,
            "## Next Step" => Section::NextStep,
            "## Open Questions" => Section::OpenQuestions,
            "## Files Touched" => Section::FilesTouched,
            "## Working Notes" => Section::WorkingNotes,
            _ if trimmed.starts_with("## ") => Section::None,
            _ => section,
        };

        if matches!(
            trimmed,
            "## Current Task"
                | "## Next Step"
                | "## Open Questions"
                | "## Files Touched"
                | "## Working Notes"
        ) {
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match section {
            Section::CurrentTask => {
                if trimmed != "(not set)" {
                    current_task_lines.push(trimmed.to_string());
                }
            }
            Section::NextStep => {
                if trimmed != "(not set)" {
                    next_step_lines.push(trimmed.to_string());
                }
            }
            Section::OpenQuestions => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    if item.trim() != "none right now" {
                        summary.open_questions.push(item.trim().to_string());
                    }
                }
            }
            Section::FilesTouched => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    if item.trim() != "none yet" {
                        summary.files_touched.push(item.trim().to_string());
                    }
                }
            }
            Section::WorkingNotes => {
                if let Some(item) = trimmed.strip_prefix("- ") {
                    if item.trim() != "none yet" {
                        summary.notes.push(item.trim().to_string());
                    }
                }
            }
            Section::None => {}
        }
    }

    summary.current_task = current_task_lines.join(" ");
    summary.next_step = next_step_lines.join(" ");

    Some(summary)
}

fn sandbox_write_task_ledger(
    dir: &Path,
    status: &str,
    current_task: &str,
    next_step: &str,
    open_questions: &[String],
    files_touched: &[String],
    notes: &[String],
) -> anyhow::Result<PathBuf> {
    let markdown = render_task_ledger_markdown(
        status,
        current_task,
        next_step,
        open_questions,
        files_touched,
        notes,
    );
    sandbox_write(dir, DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH, &markdown)
}

struct SandboxPreloadResult {
    prompt_block: String,
    loaded_count: usize,
}

fn sandbox_preload(
    dir: &Path,
    paths: &[String],
    include_list: bool,
    include_scratchpad: bool,
    include_ledger: bool,
    note: &str,
) -> anyhow::Result<SandboxPreloadResult> {
    let mut lines = vec!["sandbox.preload succeeded.".to_string()];
    if !note.trim().is_empty() {
        lines.push(format!("Reason: {}", note.trim()));
    }

    let mut loaded_count = 0usize;

    if include_list {
        let items = sandbox_list(dir)?;
        loaded_count += 1;
        lines.push(String::new());
        lines.push("Sandbox file index:".to_string());
        if items.is_empty() {
            lines.push("(sandbox is empty)".to_string());
        } else {
            for item in items.iter().take(120) {
                lines.push(format!("- {item}"));
            }
            if items.len() > 120 {
                lines.push(format!("- ...and {} more", items.len() - 120));
            }
        }
    }

    if include_scratchpad {
        let scratchpad =
            sandbox_read(dir, DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH, 50_000).unwrap_or_default();
        loaded_count += 1;
        lines.push(String::new());
        lines.push(format!(
            "Scratchpad (`{DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH}`):"
        ));
        if scratchpad.trim().is_empty() {
            lines.push("(scratchpad is empty)".to_string());
        } else {
            lines.push(truncate_for_ui(scratchpad.trim(), 12_000));
        }
    }

    if include_ledger {
        let ledger =
            sandbox_read(dir, DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH, 35_000).unwrap_or_default();
        loaded_count += 1;
        lines.push(String::new());
        lines.push(format!(
            "Task ledger (`{DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH}`):"
        ));
        if ledger.trim().is_empty() {
            lines.push("(task ledger is empty)".to_string());
        } else {
            lines.push(truncate_for_ui(ledger.trim(), 10_000));
        }
    }

    for path in paths {
        match sandbox_read(dir, path, 60_000) {
            Ok(text) => {
                loaded_count += 1;
                lines.push(String::new());
                lines.push(format!("File `{path}`:"));
                if text.trim().is_empty() {
                    lines.push("(empty file)".to_string());
                } else {
                    lines.push(truncate_for_ui(text.trim(), 16_000));
                }
            }
            Err(err) => {
                lines.push(String::new());
                lines.push(format!("File `{path}` could not be loaded: {err}"));
            }
        }
    }

    Ok(SandboxPreloadResult {
        prompt_block: lines.join("\n"),
        loaded_count,
    })
}

fn canonicalize_dir(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    Ok(path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))?)
}

fn parse_sandbox_rel_path(rel_path: &str) -> anyhow::Result<PathBuf> {
    use std::path::Component;

    let rel_path = rel_path.trim().replace('\\', "/");
    if rel_path.is_empty() {
        anyhow::bail!("empty path");
    }
    if rel_path.contains('\0') {
        anyhow::bail!("path contains NUL");
    }
    // Windows-specific hardening: block drive prefixes and NTFS ADS (`file.txt:stream`).
    if rel_path.contains(':') {
        anyhow::bail!("':' is not allowed in sandbox paths");
    }

    let path = PathBuf::from(rel_path);
    for c in path.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => {
                anyhow::bail!("absolute paths are not allowed")
            }
            Component::ParentDir => anyhow::bail!("path traversal blocked"),
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    Ok(path)
}

fn sandbox_rel_path_is_ai_text_allowed(rel_path: &str) -> bool {
    let Ok(path) = parse_sandbox_rel_path(rel_path) else {
        return false;
    };
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "txt" | "md" | "markdown")
}

fn sandbox_rel_path_looks_like_image(rel_path: &str) -> bool {
    let Ok(path) = parse_sandbox_rel_path(rel_path) else {
        return false;
    };
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
    )
}

fn sandbox_rel_path_is_ai_read_allowed(rel_path: &str) -> bool {
    sandbox_rel_path_is_ai_text_allowed(rel_path) || sandbox_rel_path_looks_like_image(rel_path)
}

fn sandbox_ai_text_guard(rel_path: &str) -> anyhow::Result<()> {
    if sandbox_rel_path_is_ai_text_allowed(rel_path) {
        Ok(())
    } else {
        anyhow::bail!("only .txt and .md sandbox files are allowed for AI tool actions")
    }
}

fn sandbox_ai_read_guard(rel_path: &str) -> anyhow::Result<()> {
    if sandbox_rel_path_is_ai_read_allowed(rel_path) {
        Ok(())
    } else {
        anyhow::bail!("only text and common image sandbox files are allowed for AI read actions")
    }
}

fn normalize_sandbox_task_path_input(input: &str) -> String {
    let mut normalized = input.trim().replace('\\', "/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    if normalized.starts_with('/') {
        normalized = normalized.trim_start_matches('/').to_string();
    }
    if !normalized.is_empty() && !normalized.contains('.') && !normalized.ends_with('/') {
        normalized.push_str(".md");
    }
    normalized
}

fn path_looks_like_image(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
    )
}

fn path_uses_inline_image_preview(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "png")
}

fn format_chat_selected_file_label(path: &Path, sandbox_dir: Option<&Path>) -> String {
    if let Some(dir) = sandbox_dir
        && let Ok(rel) = path.strip_prefix(dir)
    {
        return rel.to_string_lossy().replace('\\', "/");
    }
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn build_chat_selected_file_prompt(
    path: &Path,
    sandbox_dir: Option<&Path>,
) -> anyhow::Result<String> {
    let label = format_chat_selected_file_label(path, sandbox_dir);
    if path_looks_like_image(path) {
        return Ok(format!(
            "### SELECTED LOCAL FILE\n\
The user selected the local image file `{label}` for this turn.\n\
Treat this as the attached multimodal image for the current turn.\n\
Use the image itself when answering the user's request."
        ));
    }

    let text = read_text_file(path, 200_000)?;
    let trimmed = text.trim();
    let rendered = if trimmed.is_empty() {
        "(file is empty)".to_string()
    } else {
        truncate_for_ui(trimmed, 24_000)
    };
    Ok(format!(
        "### SELECTED LOCAL FILE\n\
The user selected the local file `{label}` for this turn.\n\
Use this file as relevant context for the current request.\n\
\n\
File contents:\n\
{rendered}"
    ))
}

fn multimodal_family_hint(model_path: &Path, architecture: Option<&str>) -> Option<&'static str> {
    let arch = architecture.unwrap_or_default().to_ascii_lowercase();
    let model_name = model_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if arch == "qwen3vl" || model_name.contains("qwen3-vl") {
        Some("qwen3")
    } else if arch == "qwen2vl"
        || model_name.contains("qwen2.5-vl")
        || model_name.contains("qwen2-vl")
    {
        Some("qwen2")
    } else if arch == "minicpmv" || model_name.contains("minicpm") {
        Some("minicpm")
    } else if arch == "gemma3"
        || arch == "gemma4"
        || model_name.contains("gemma-3")
        || model_name.contains("gemma-4")
        || model_name.contains("gemma3")
        || model_name.contains("gemma4")
    {
        Some("gemma")
    } else if arch == "llava" || model_name.contains("llava") || model_name.contains("joycaption") {
        Some("llava")
    } else if arch == "smolvlm" || model_name.contains("smolvlm") {
        Some("smolvlm")
    } else if arch == "internvl" || model_name.contains("internvl") {
        Some("internvl")
    } else {
        None
    }
}

fn architecture_supports_mtmd(model_path: &Path, architecture: Option<&str>) -> bool {
    multimodal_family_hint(model_path, architecture).is_some()
}

fn multimodal_family_label(model_path: &Path, architecture: Option<&str>) -> Option<&'static str> {
    match multimodal_family_hint(model_path, architecture) {
        Some("qwen3") => Some("Qwen3-VL"),
        Some("qwen2") => Some("Qwen2-VL"),
        Some("minicpm") => Some("MiniCPM-V"),
        Some("gemma") => Some("Gemma vision"),
        Some("llava") => Some("LLaVA-style"),
        Some("smolvlm") => Some("SmolVLM"),
        Some("internvl") => Some("InternVL"),
        _ => None,
    }
}

fn is_projector_candidate(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.contains("mmproj") {
        return true;
    }
    if name == "ggml-model-f16.gguf" || name.starts_with("ggml-model-") {
        return read_gguf_architecture(path)
            .ok()
            .flatten()
            .map(|arch| arch.eq_ignore_ascii_case("clip"))
            .unwrap_or(false);
    }
    false
}

fn find_matching_mmproj_path(model_path: &Path, architecture: Option<&str>) -> Option<PathBuf> {
    let dir = model_path.parent()?;
    let model_name = model_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let family_hint = multimodal_family_hint(model_path, architecture);
    let mut best_match: Option<(i32, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        if !name.ends_with(".gguf") || !is_projector_candidate(&path) {
            continue;
        }
        let mut score = 0;

        if model_name.contains("qwen3") {
            if name.contains("qwen3") {
                score += 120;
            } else {
                continue;
            }
        } else if model_name.contains("qwen2.5") {
            if name.contains("qwen2.5") {
                score += 120;
            } else {
                continue;
            }
        } else if model_name.contains("qwen2") {
            if name.contains("qwen2") {
                score += 120;
            } else {
                continue;
            }
        }

        if model_name.contains("llava-v1.5") {
            if name.contains("llava-v1.5") {
                score += 120;
            } else if name == "mmproj-model-f16.gguf" || name == "ggml-model-f16.gguf" {
                score += 70;
            } else {
                continue;
            }
        }

        if let Some(hint) = family_hint {
            if name.contains(hint) {
                score += 60;
            } else if hint == "llava"
                && (name == "mmproj-model-f16.gguf" || name == "ggml-model-f16.gguf")
            {
                score += 40;
            } else if hint == "gemma" || hint == "qwen2" || hint == "qwen3" {
                continue;
            }
        }

        if name.contains("f16") {
            score += 5;
        }

        match &best_match {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best_match = Some((score, path)),
        }
    }
    best_match.map(|(_, path)| path)
}

fn expected_projector_hint(model_path: &Path, architecture: Option<&str>) -> Option<String> {
    let family = multimodal_family_hint(model_path, architecture)?;
    let model_name = model_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| model_path.display().to_string());

    Some(match family {
        "gemma" => {
            if model_name.to_ascii_lowercase().contains("12b") {
                "Drop a file like `mmproj-gemma-4-12B-it-Q8_0.gguf` next to this model.".to_string()
            } else if model_name.to_ascii_lowercase().contains("e4b") {
                "Drop a file like `mmproj-gemma-4-E4B-it-*.gguf` next to this model.".to_string()
            } else if model_name.to_ascii_lowercase().contains("31b") {
                "Drop a file like `mmproj-gemma-4-31B-it-*.gguf` next to this model.".to_string()
            } else if model_name.to_ascii_lowercase().contains("26b") {
                "Drop a file like `mmproj-gemma-4-26B-A4B-it-*.gguf` next to this model.".to_string()
            } else {
                "Drop a matching `mmproj-gemma-4-*.gguf` file next to this model.".to_string()
            }
        }
        "qwen3" => {
            "Drop a matching `mmproj-Qwen3-VL-*.gguf` file next to this model.".to_string()
        }
        "qwen2" => {
            "Drop a matching `mmproj-Qwen2.5-VL-*.gguf` or `mmproj-Qwen2-VL-*.gguf` file next to this model.".to_string()
        }
        "llava" => {
            "Drop an llava/joycaption projector next to this model, or use a nearby generic `mmproj-model-f16.gguf` / `ggml-model-f16.gguf` clip projector.".to_string()
        }
        "minicpm" => {
            "Drop a matching `mmproj-MiniCPM-*.gguf` file next to this model.".to_string()
        }
        "smolvlm" => {
            "Drop a matching `mmproj-SmolVLM-*.gguf` file next to this model.".to_string()
        }
        "internvl" => {
            "Drop a matching `mmproj-InternVL-*.gguf` file next to this model.".to_string()
        }
        _ => "Drop a matching multimodal projector GGUF next to this model.".to_string(),
    })
}

fn selected_model_vision_explanation(path: &Path) -> Option<String> {
    let architecture = read_gguf_architecture(path).ok().flatten()?;
    let family = multimodal_family_label(path, Some(&architecture));
    if !architecture_supports_mtmd(path, Some(&architecture)) {
        return Some("This GGUF is currently being treated as text-only.".to_string());
    }

    if let Some(mmproj_path) = find_matching_mmproj_path(path, Some(&architecture)) {
        let projector_name = mmproj_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        return Some(match family {
            Some("Gemma vision") => format!(
                "Gemma vision path detected. Using projector `{projector_name}`. Note: on this current local runtime build, Gemma 4 multimodal launches may still crash even with a matching projector."
            ),
            Some(label) => format!("{label} path detected. Using projector `{projector_name}`."),
            None => format!("Multimodal path detected. Using projector `{projector_name}`."),
        });
    }

    Some(match family {
        Some("Gemma vision") => {
            format!(
                "Gemma vision path detected. The model family is multimodal, but this current local runtime path still needs a Gemma-compatible projector file beside the GGUF. {}",
                expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
            )
        }
        Some("Qwen3-VL") => {
            format!(
                "Qwen3-VL path detected, but no Qwen3-compatible projector was found beside the model. {}",
                expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
            )
        }
        Some("Qwen2-VL") => {
            format!(
                "Qwen2-VL path detected, but no Qwen2/Qwen2.5-compatible projector was found beside the model. {}",
                expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
            )
        }
        Some("LLaVA-style") => {
            format!(
                "LLaVA-style path detected, but no matching projector was found. {}",
                expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
            )
        }
        Some(label) => format!(
            "{label} path detected, but no compatible projector was found beside the model. {}",
            expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
        ),
        None => format!(
            "Multimodal path detected, but no compatible projector was found beside the model. {}",
            expected_projector_hint(path, Some(&architecture)).unwrap_or_default()
        ),
    })
}

fn mtmd_extra_args_for_model(model_path: &Path, architecture: Option<&str>) -> Vec<String> {
    let model_name = model_path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let arch = architecture.unwrap_or_default().to_ascii_lowercase();
    if arch == "llava" || model_name.contains("llava-v1.5") {
        return vec!["--chat-template".to_string(), "vicuna".to_string()];
    }
    Vec::new()
}

fn selected_model_inline_status(path: &Path, runtime_issue: Option<&str>) -> Option<String> {
    let architecture = read_gguf_architecture(path).ok().flatten()?;
    if !architecture_supports_mtmd(path, Some(&architecture)) {
        return Some(format!("{architecture} | text-only path"));
    }

    let mmproj = find_matching_mmproj_path(path, Some(&architecture));
    let mut parts = vec![architecture.clone()];
    if let Some(family_label) = multimodal_family_label(path, Some(&architecture)) {
        parts.push(family_label.to_string());
    }
    if runtime_issue.is_some() {
        parts.push("runtime issue".to_string());
    }
    if let Some(mmproj_path) = mmproj.as_ref() {
        if runtime_issue.is_none() {
            parts.push("vision ready".to_string());
        }
        parts.push(
            mmproj_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        );
    } else {
        parts.push("needs projector".to_string());
    }

    let extra_args = mtmd_extra_args_for_model(path, Some(&parts[0]));
    if extra_args.windows(2).any(|pair| pair[0] == "--chat-template") {
        if let Some(template) = extra_args
            .windows(2)
            .find(|pair| pair[0] == "--chat-template")
            .map(|pair| pair[1].clone())
        {
            parts.push(format!("template {template}"));
        }
    }

    Some(parts.join(" | "))
}

fn selected_model_is_vision_ready(path: &Path) -> bool {
    let Some(architecture) = read_gguf_architecture(path).ok().flatten() else {
        return false;
    };
    architecture_supports_mtmd(path, Some(&architecture))
        && find_matching_mmproj_path(path, Some(&architecture)).is_some()
}

fn run_mtmd_cli_generation(
    runtime_dir: &Path,
    model_path: &Path,
    mmproj_path: &Path,
    image_path: &Path,
    system_prompt: &str,
    prompt: &str,
    max_tokens: usize,
    temp: f32,
    top_p: f32,
    top_k: i32,
    extra_args: &[String],
    cancel: &AtomicBool,
) -> anyhow::Result<String> {
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("generation cancelled");
    }
    let exe = runtime_dir.join("llama-mtmd-cli.exe");
    let output = std::process::Command::new(&exe)
        .arg("-m")
        .arg(model_path)
        .arg("--mmproj")
        .arg(mmproj_path)
        .arg("--image")
        .arg(image_path)
        .arg("-sys")
        .arg(system_prompt)
        .arg("-p")
        .arg(prompt)
        .arg("-n")
        .arg(max_tokens.to_string())
        .arg("--temp")
        .arg(format!("{temp:.3}"))
        .arg("--top-p")
        .arg(format!("{top_p:.3}"))
        .arg("--top-k")
        .arg(top_k.to_string())
        .arg("--no-warmup")
        .args(extra_args)
        .output()
        .with_context(|| format!("launch {}", exe.display()))?;
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("generation cancelled");
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let exit_code = output.status.code();
        let model_name = model_path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if exit_code == Some(-1073740791) {
            let base = if model_name.contains("gemma-4") || model_name.contains("gemma4") {
                "mtmd runtime crashed with Windows exit code 0xC0000409 (STATUS_STACK_BUFFER_OVERRUN) while launching Gemma 4 multimodal inference. The model/projector pair is recognized, but this current local llama.cpp runtime appears unstable for Gemma 4 vision on this machine."
            } else {
                "mtmd runtime crashed with Windows exit code 0xC0000409 (STATUS_STACK_BUFFER_OVERRUN)."
            };
            anyhow::bail!(
                "{}\n{}\n{}",
                base,
                stdout.trim(),
                stderr.trim()
            );
        }
        anyhow::bail!(
            "mtmd generation failed with status {}.\n{}\n{}",
            output.status,
            stdout.trim(),
            stderr.trim()
        );
    }

    let combined = [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if combined.trim().is_empty() {
        anyhow::bail!("mtmd generation returned no output");
    }
    Ok(combined)
}

fn build_explicit_sandbox_task_prompt(
    request: &str,
    rel_path: &str,
    intent: SandboxTaskIntent,
) -> String {
    if sandbox_rel_path_looks_like_image(rel_path) {
        return format!(
            "SANDBOX TASK MODE is explicitly enabled by the UI for this turn.\n\
Treat this as a sandbox image inspection request, not ordinary chat.\n\
Target sandbox path: `{rel_path}`.\n\
Required behavior:\n\
- Respond with exactly one `sandbox.read` JSON object for that path and nothing else.\n\
- Do not request `sandbox.write` or `sandbox.append` for this image path.\n\
- After the approved read result returns, use the attached image to answer the user's request.\n\
- Do not add commentary, explanation, markdown fences, or extra text outside the JSON object.\n\
- Do not emit visible reasoning or planning text.\n\
\n\
User request about the image:\n\
{request}"
        );
    }

    match intent {
        SandboxTaskIntent::Create => format!(
            "SANDBOX TASK MODE is explicitly enabled by the UI for this turn.\n\
Treat this as a sandbox file creation request, not ordinary chat.\n\
Target sandbox path: `{rel_path}`.\n\
Required behavior:\n\
- Respond with exactly one `sandbox.write` JSON object and nothing else.\n\
- Use the target path exactly as given.\n\
- Put the final requested deliverable in `contents`.\n\
- Do not add commentary, explanation, markdown fences, or extra text outside the JSON object.\n\
- Do not emit visible reasoning or planning text.\n\
- Overwrite the target file with the finished result.\n\
\n\
User request for the file contents:\n\
{request}"
        ),
        SandboxTaskIntent::Edit => format!(
            "SANDBOX TASK MODE is explicitly enabled by the UI for this turn.\n\
Treat this as a sandbox file editing request, not ordinary chat.\n\
Target sandbox path: `{rel_path}`.\n\
Required behavior:\n\
- If you do not already have the current contents of `{rel_path}` from a recent approved sandbox tool result, respond with exactly one `sandbox.read` JSON object for that path and nothing else.\n\
- If you already have the current contents, respond with exactly one `sandbox.write` JSON object for that path and nothing else.\n\
- When writing, put the fully updated file contents in `contents`.\n\
- Do not add commentary, explanation, markdown fences, or extra text outside the JSON object.\n\
- Do not emit visible reasoning or planning text.\n\
\n\
User request for the edit:\n\
{request}"
        ),
    }
}

fn ensure_path_within_dir(
    dir: &std::path::Path,
    path: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    let dir = canonicalize_dir(dir)?;
    let path = path
        .canonicalize()
        .with_context(|| format!("canonicalize {}", path.display()))?;

    if path.starts_with(&dir) {
        Ok(path)
    } else {
        anyhow::bail!("path escapes sandbox")
    }
}

fn ensure_save_path_within_dir(
    dir: &std::path::Path,
    path: &std::path::Path,
) -> anyhow::Result<PathBuf> {
    // `path` may not exist yet, so we validate using its parent directory.
    let dir = canonicalize_dir(dir)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent dir"))?;
    let parent = canonicalize_dir(parent)?;
    if !parent.starts_with(&dir) {
        anyhow::bail!("path escapes sandbox");
    }

    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| anyhow::anyhow!("missing filename"))?,
    ))
}

fn sandbox_write(dir: &Path, rel_path: &str, contents: &str) -> anyhow::Result<PathBuf> {
    let rel = parse_sandbox_rel_path(rel_path)?;
    let dir = canonicalize_dir(dir)?;
    let target = dir.join(rel);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent dir"))?;

    // Create intermediate directories safely, rejecting any symlink/junction escape.
    if let Ok(rel_parent) = parent.strip_prefix(&dir) {
        let mut cur = dir.clone();
        for comp in rel_parent.components() {
            let std::path::Component::Normal(name) = comp else {
                continue;
            };
            cur = cur.join(name);
            if cur.exists() {
                // If this resolves outside the sandbox, block.
                let canon = cur
                    .canonicalize()
                    .with_context(|| format!("canonicalize {}", cur.display()))?;
                if !canon.starts_with(&dir) {
                    anyhow::bail!("path escapes sandbox");
                }
                if !canon.is_dir() {
                    anyhow::bail!("path component is not a directory");
                }
            } else {
                std::fs::create_dir(&cur).with_context(|| format!("mkdir {}", cur.display()))?;
            }
        }
    } else {
        anyhow::bail!("path escapes sandbox");
    }

    // If the target exists, ensure it resolves within the sandbox before overwriting.
    if target.exists() {
        let canon = ensure_path_within_dir(&dir, &target)?;
        if !canon.starts_with(&dir) {
            anyhow::bail!("path escapes sandbox");
        }
    }

    std::fs::write(&target, contents).with_context(|| format!("write {}", target.display()))?;
    ensure_path_within_dir(&dir, &target)
}

fn sandbox_append(dir: &Path, rel_path: &str, contents: &str) -> anyhow::Result<PathBuf> {
    use std::io::Write;

    let rel = parse_sandbox_rel_path(rel_path)?;
    let dir = canonicalize_dir(dir)?;
    let target = dir.join(rel);
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing parent dir"))?;

    if let Ok(rel_parent) = parent.strip_prefix(&dir) {
        let mut cur = dir.clone();
        for comp in rel_parent.components() {
            let std::path::Component::Normal(name) = comp else {
                continue;
            };
            cur = cur.join(name);
            if cur.exists() {
                let canon = cur
                    .canonicalize()
                    .with_context(|| format!("canonicalize {}", cur.display()))?;
                if !canon.starts_with(&dir) {
                    anyhow::bail!("path escapes sandbox");
                }
                if !canon.is_dir() {
                    anyhow::bail!("path component is not a directory");
                }
            } else {
                std::fs::create_dir(&cur).with_context(|| format!("mkdir {}", cur.display()))?;
            }
        }
    } else {
        anyhow::bail!("path escapes sandbox");
    }

    if target.exists() {
        let canon = ensure_path_within_dir(&dir, &target)?;
        if !canon.starts_with(&dir) {
            anyhow::bail!("path escapes sandbox");
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&target)
        .with_context(|| format!("append {}", target.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("append {}", target.display()))?;
    ensure_path_within_dir(&dir, &target)
}

fn sandbox_read(dir: &Path, rel_path: &str, max_bytes: usize) -> anyhow::Result<String> {
    let rel = parse_sandbox_rel_path(rel_path)?;
    let dir = canonicalize_dir(dir)?;
    let target = dir.join(rel);
    let target = ensure_path_within_dir(&dir, &target)?;
    read_text_file(&target, max_bytes)
}

fn sandbox_list(dir: &Path) -> anyhow::Result<Vec<String>> {
    let dir = canonicalize_dir(dir)?;
    let mut out = Vec::new();

    // Small, safe recursive walk (shallow) to help the orchestrator discover files it created.
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.clone(), 0)];
    while let Some((cur, depth)) = stack.pop() {
        if depth > 6 || out.len() > 2000 {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&cur) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            let Ok(pp) = ensure_path_within_dir(&dir, &p) else {
                continue;
            };
            let Ok(rel) = pp.strip_prefix(&dir) else {
                continue;
            };
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            out.push(rel_s);
            if pp.is_dir() {
                stack.push((pp, depth + 1));
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn keyword_search_cold_log(
    logs_dir: Option<&std::path::Path>,
    q: &str,
    limit: usize,
) -> Vec<String> {
    use std::io::BufRead;
    let Some(dir) = logs_dir else {
        return Vec::new();
    };
    let path = dir.join("cold_log.jsonl");
    let Ok(f) = std::fs::File::open(&path) else {
        return Vec::new();
    };
    let q = q.trim();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let reader = std::io::BufReader::new(f);
    for line in reader.lines().flatten() {
        if line.to_lowercase().contains(&q.to_lowercase()) {
            out.push(line);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

fn format_network_transfer_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let value = bytes as f64;
    if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_network_transfer_meta(
    content_type: &str,
    transfer_encoding: &str,
    byte_len: u64,
    chunk_count: u32,
) -> String {
    let encoding_label = match transfer_encoding.trim() {
        "base64" => "binary",
        "utf8" => "text",
        other if !other.is_empty() => other,
        _ => "text",
    };
    let content_type = if content_type.trim().is_empty() {
        "(unspecified)"
    } else {
        content_type.trim()
    };
    format!(
        "{} | {} | {} chunk(s) | {}",
        format_network_transfer_size(byte_len),
        encoding_label,
        chunk_count.max(1),
        content_type
    )
}

fn now_unix_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
