use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use std::{collections::HashMap, path::Path};

use anyhow::Context;
mod ecg_window;
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
use chattycog_gui::networking::{BlockedPeer, NetworkController, ReceivedArtifact, TrustedPeer};
use chattycog_gui::preferences::{self, AppPreferences, GenParams, ModulePreferences, PromptCapsule};
use crossbeam_channel::Receiver;
use ecg_window::EcgWindowState;
use eframe::egui;

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
struct ModelOption {
    label: String,
    value: String, // stored in prefs; either a filename or "modules/<module_id>/<file>.gguf"
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

    show_left_sidebar: bool,
    gguf_path: Option<PathBuf>,
    models_dir: Option<PathBuf>,
    models_cache: Vec<PathBuf>,

    messages: Vec<Message>,
    composer: String,

    // Generation
    is_generating: bool,
    gen_cancel: Option<Arc<AtomicBool>>,
    gen_rx: Option<Receiver<GenEvent>>,
    assistant_draft: String,
    runtime_status: String,
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

        let mut app = Self {
            tab: Tab::Chat,
            prev_tab: Tab::Chat,
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
            is_generating: false,
            gen_cancel: None,
            gen_rx: None,
            assistant_draft: String::new(),
            runtime_status: "Runtime: probing...".to_string(),
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
            lukewarm_summary: String::new(),
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

        if self.capsule_editor_name.trim().is_empty() && self.capsule_editor_text.trim().is_empty() {
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
            self.runtime_status = format!("Runtime: selected model {label}");
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
        let path = path?;
        if let Some(modules_dir) = &self.modules_dir {
            if let Ok(rel) = path.strip_prefix(modules_dir) {
                return Some(format!(
                    "modules/{}",
                    rel.to_string_lossy().replace('\\', "/")
                ));
            }
        }
        if let Some(models_dir) = &self.models_dir {
            if let Ok(rel) = path.strip_prefix(models_dir) {
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
    }

    fn resolve_portable_model_hint(&self, hint: Option<&str>) -> Option<PathBuf> {
        let hint = hint?.trim();
        if hint.is_empty() {
            return None;
        }

        if let Some(rest) = hint.strip_prefix("modules/") {
            let path = self.modules_dir.as_ref()?.join(rest.replace('/', "\\"));
            if path.is_file() {
                return Some(path);
            }
        }

        if let Some(models_dir) = &self.models_dir {
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

        for candidate in scan_ggufs(self.models_dir.as_deref()) {
            if candidate
                .file_name()
                .map(|name| name.to_string_lossy().eq_ignore_ascii_case(&file_name))
                .unwrap_or(false)
            {
                return Some(candidate);
            }
        }
        for candidate in scan_ggufs_in_modules(self.modules_dir.as_deref()) {
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
        // Defensive: if a summarizer ever echoes placeholder text, strip it before injection to avoid prompt bloat.
        if departments_context.contains("<bullet>") || departments_context.contains("<paragraph>") {
            departments_context = departments_context
                .replace("<bullet>", "")
                .replace("<paragraph>", "");
        }
        let lukewarm_context =
            read_lukewarm_from_logs_dir(self.logs_dir.as_deref()).unwrap_or_default();
        let model_label = gguf
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "(no GGUF selected)".to_string());
        let mut system = base_system;
        system.push_str("\n\n### CHATTYCOG COCKPIT ORIENTATION\n");
        system.push_str(&build_wakeup_orientation(
            &model_label,
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
            build_task_ledger_prompt_nudge(&prompt, self.sandbox_dir.as_deref())
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
        match ensure_path_within_dir(&dir, path)
            .and_then(|pp| read_text_file(&pp, 500_000).map(|t| (pp, t)))
        {
            Ok((pp, text)) => {
                self.sandbox_selected = Some(pp.clone());
                self.sandbox_editor_path = Some(pp.clone());
                self.sandbox_last_working_path = Some(pp.clone());
                self.sandbox_editor_text = text;
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

    fn ensure_default_sandbox_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            return;
        };
        match ensure_default_sandbox_scratchpad_file(&dir) {
            Ok(_) => {}
            Err(err) => {
                if self.sandbox_status.trim().is_empty() {
                    self.sandbox_status = format!("Scratchpad setup failed: {err}");
                }
            }
        }
    }

    fn ensure_default_sandbox_task_ledger(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            return;
        };
        match ensure_default_sandbox_task_ledger_file(&dir) {
            Ok(_) => {}
            Err(err) => {
                if self.sandbox_status.trim().is_empty() {
                    self.sandbox_status = format!("Task ledger setup failed: {err}");
                }
            }
        }
    }

    fn open_default_sandbox_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        match ensure_default_sandbox_scratchpad_file(&dir) {
            Ok(path) => self.open_sandbox_file_in_editor(&path),
            Err(err) => self.sandbox_status = format!("Scratchpad setup failed: {err}"),
        }
    }

    fn open_default_sandbox_task_ledger(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        match ensure_default_sandbox_task_ledger_file(&dir) {
            Ok(path) => self.open_sandbox_file_in_editor(&path),
            Err(err) => self.sandbox_status = format!("Task ledger setup failed: {err}"),
        }
    }

    fn seed_default_sandbox_task_ledger_from_context(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let current_task = self
            .messages
            .iter()
            .rev()
            .find(|message| matches!(message.role, Role::User))
            .map(|message| truncate_for_ui(message.content.trim(), 500))
            .unwrap_or_else(|| "Capture the current task here.".to_string());
        let next_step = self
            .hot_memory
            .last()
            .map(|item| truncate_for_ui(item.trim(), 220))
            .unwrap_or_else(|| "Record the next concrete step here.".to_string());
        let files_touched = self
            .sandbox_editor_path
            .as_ref()
            .and_then(|path| path.strip_prefix(&dir).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .into_iter()
            .collect::<Vec<_>>();
        let notes = self
            .hot_memory
            .iter()
            .rev()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>();
        match sandbox_write_task_ledger(
            &dir,
            "active",
            &current_task,
            &next_step,
            &Vec::new(),
            &files_touched,
            &notes,
        ) {
            Ok(path) => {
                self.sandbox_status = format!("Seeded task ledger at {}", path.display());
                self.open_sandbox_file_in_editor(&path);
            }
            Err(err) => {
                self.sandbox_status = format!("Could not seed task ledger: {err}");
            }
        }
    }

    fn reopen_last_sandbox_working_file(&mut self) {
        let Some(path) = self.sandbox_last_working_path.clone() else {
            self.sandbox_status = "No sandbox working file has been opened yet.".to_string();
            return;
        };
        self.open_sandbox_file_and_focus_tab(&path);
    }

    fn current_sandbox_editor_rel_path(&self, dir: &Path) -> Option<String> {
        self.sandbox_editor_path
            .as_ref()
            .and_then(|path| path.strip_prefix(dir).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"))
    }

    fn promote_editor_text_to_scratchpad(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        let block = format!(
            "\n## Promoted note ({})\nSource: `{}`\n\n{}\n",
            now_unix_ms().max(0),
            source,
            text
        );

        match sandbox_append(&dir, DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH, &block) {
            Ok(path) => {
                self.sandbox_status = format!("Promoted editor text to {}", path.display());
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not promote editor text: {err}");
            }
        }
    }

    fn promote_editor_text_to_ledger_notes(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        self.ensure_default_sandbox_task_ledger();

        let mut summary = read_task_ledger_summary(&dir).unwrap_or_default();
        if summary.status.trim().is_empty() {
            summary.status = "active".to_string();
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        if source != "(unsaved scratch buffer)" && !summary.files_touched.contains(&source) {
            summary.files_touched.push(source.clone());
        }

        let mut promoted_notes = vec![format!(
            "Promoted from {} at {}",
            source,
            now_unix_ms().max(0)
        )];
        promoted_notes.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(8)
                .map(|line| truncate_for_ui(line, 220)),
        );
        summary.notes.extend(promoted_notes);
        if summary.notes.len() > 24 {
            let keep_from = summary.notes.len() - 24;
            summary.notes = summary.notes.split_off(keep_from);
        }

        match sandbox_write_task_ledger(
            &dir,
            if summary.status.trim().is_empty() {
                "active"
            } else {
                summary.status.trim()
            },
            summary.current_task.trim(),
            summary.next_step.trim(),
            &summary.open_questions,
            &summary.files_touched,
            &summary.notes,
        ) {
            Ok(path) => {
                self.sandbox_status = format!("Promoted editor notes into {}", path.display());
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not promote editor notes: {err}");
            }
        }
    }

    fn set_task_ledger_field_from_editor(&mut self, set_current_task: bool) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to promote.".to_string();
            return;
        }

        self.ensure_default_sandbox_task_ledger();

        let mut summary = read_task_ledger_summary(&dir).unwrap_or_default();
        if summary.status.trim().is_empty() {
            summary.status = "active".to_string();
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "(unsaved scratch buffer)".to_string());
        if source != "(unsaved scratch buffer)" && !summary.files_touched.contains(&source) {
            summary.files_touched.push(source.clone());
        }

        let normalized = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = truncate_for_ui(&normalized, 420);

        if set_current_task {
            summary.current_task = normalized.clone();
            if summary.next_step.trim().is_empty() {
                summary.next_step =
                    "Review the updated current task and choose the next concrete step."
                        .to_string();
            }
            summary.notes.push(format!(
                "Current task updated from {} at {}",
                source,
                now_unix_ms().max(0)
            ));
        } else {
            summary.next_step = normalized.clone();
            summary.notes.push(format!(
                "Next step updated from {} at {}",
                source,
                now_unix_ms().max(0)
            ));
        }

        if summary.notes.len() > 24 {
            let keep_from = summary.notes.len() - 24;
            summary.notes = summary.notes.split_off(keep_from);
        }

        match sandbox_write_task_ledger(
            &dir,
            summary.status.trim(),
            summary.current_task.trim(),
            summary.next_step.trim(),
            &summary.open_questions,
            &summary.files_touched,
            &summary.notes,
        ) {
            Ok(path) => {
                self.sandbox_status = if set_current_task {
                    format!(
                        "Promoted editor text into current task at {}",
                        path.display()
                    )
                } else {
                    format!("Promoted editor text into next step at {}", path.display())
                };
                self.open_sandbox_file_in_editor(&path);
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_status, 120)),
                );
            }
            Err(err) => {
                self.sandbox_status = format!("Could not update task ledger: {err}");
            }
        }
    }

    fn append_editor_summary_to_hot_memory(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_status = "Sandbox folder not found.".to_string();
            return;
        };
        let text = self.sandbox_editor_text.trim().to_string();
        if text.is_empty() {
            self.sandbox_status = "Editor is empty. Nothing to summarize.".to_string();
            return;
        }

        let source = self
            .current_sandbox_editor_rel_path(&dir)
            .unwrap_or_else(|| "scratch buffer".to_string());
        let summary = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>()
            .join(" ");
        let summary = truncate_for_ui(&summary, 240);
        let hot_item = format!("Sandbox note ({source}): {summary}");

        push_hot_memory(self, hot_item.clone());
        self.sandbox_status = format!("Added editor summary to hot memory from {source}.");
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Chat,
                source: "sandbox".to_string(),
                module: None,
                event_type: Some("hot_memory_summary".to_string()),
                text: hot_item,
                tags: vec!["sandbox".to_string(), "hot_memory_summary".to_string()],
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }

    fn defer_pending_sandbox_actions(&mut self) {
        let deferred_count = self.pending_sandbox_actions.len();
        self.pending_sandbox_actions.clear();
        self.sandbox_action_status = if deferred_count == 0 {
            "No sandbox actions were waiting to be deferred.".to_string()
        } else {
            format!("Deferred {deferred_count} sandbox action(s). No file changes were run.")
        };
        push_hot_memory(
            self,
            format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
        );
        if let Some(bk) = &self.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Chat,
                source: "sandbox".to_string(),
                module: None,
                event_type: Some("deferred_actions".to_string()),
                text: self.sandbox_action_status.clone(),
                tags: vec!["sandbox".to_string(), "deferred_actions".to_string()],
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }

    fn preload_sandbox_and_continue(&mut self) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_action_status = "Sandbox folder not found.".to_string();
            self.pending_sandbox_actions.clear();
            return;
        };

        self.ensure_default_sandbox_scratchpad();
        self.ensure_default_sandbox_task_ledger();

        let mut paths = Vec::new();
        for action in &self.pending_sandbox_actions {
            match action {
                SandboxAction::Write { path, .. }
                | SandboxAction::Append { path, .. }
                | SandboxAction::Read { path } => {
                    if !path.trim().is_empty() {
                        paths.push(path.trim().to_string());
                    }
                }
                SandboxAction::Preload {
                    paths: more_paths, ..
                } => {
                    for path in more_paths {
                        if !path.trim().is_empty() {
                            paths.push(path.trim().to_string());
                        }
                    }
                }
                SandboxAction::Ledger { files_touched, .. } => {
                    for path in files_touched {
                        if !path.trim().is_empty() {
                            paths.push(path.trim().to_string());
                        }
                    }
                }
                SandboxAction::List => {}
            }
        }

        if let Some(editor_path) = self.sandbox_editor_path.as_ref() {
            if let Ok(rel) = editor_path.strip_prefix(&dir) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if !rel.trim().is_empty() {
                    paths.push(rel);
                }
            }
        }

        paths.sort();
        paths.dedup();

        let note = if paths.is_empty() {
            "fast preload before continuing a multi-step task"
        } else {
            "fast preload before continuing; inspect likely relevant sandbox files first"
        };

        match sandbox_preload(&dir, &paths, true, true, true, note) {
            Ok(result) => {
                self.pending_sandbox_actions.clear();
                self.sandbox_last_tool_result = result.prompt_block;
                self.sandbox_action_status = format!(
                    "Preloaded {} item(s); pending sandbox actions were deferred.",
                    result.loaded_count
                );
                if let Ok(path) = ensure_default_sandbox_scratchpad_file(&dir) {
                    self.open_sandbox_file_in_editor(&path);
                }
                push_hot_memory(
                    self,
                    format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
                );
                if let Some(bk) = &self.bookkeeper {
                    bk.append(MemoryEvent {
                        ts_unix_ms: now_unix_ms(),
                        kind: MemoryKind::Cold,
                        category: EventCategory::Chat,
                        source: "sandbox".to_string(),
                        module: None,
                        event_type: Some("tool_result".to_string()),
                        text: self.sandbox_last_tool_result.clone(),
                        tags: vec![
                            "sandbox".to_string(),
                            "tool_result".to_string(),
                            "preload_fast_path".to_string(),
                        ],
                        entities: Vec::new(),
                        payload_json: None,
                    });
                }
                if !self.is_generating && !self.sandbox_last_tool_result.trim().is_empty() {
                    self.start_generation(
                        "Continue from the sandbox preload context and help with the current task. Reconsider the deferred sandbox actions, and only request new sandbox JSON if it is still needed.".to_string(),
                    );
                }
            }
            Err(err) => {
                self.sandbox_action_status = format!("Sandbox preload failed: {err}");
            }
        }
    }

    fn apply_pending_sandbox_actions(&mut self, continue_after: bool) {
        let Some(dir) = self.sandbox_dir.clone() else {
            self.sandbox_action_status = "Sandbox folder not found.".to_string();
            self.pending_sandbox_actions.clear();
            return;
        };

        let mut status_lines = Vec::new();
        let mut result_lines = Vec::new();
        let mut last_opened: Option<PathBuf> = None;
        for action in self.pending_sandbox_actions.drain(..) {
            match action {
                SandboxAction::Write { path, contents } => {
                    match sandbox_ai_text_guard(&path).and_then(|_| sandbox_write(&dir, &path, &contents)) {
                        Ok(p) => {
                            status_lines.push(format!("Wrote {}", p.display()));
                            result_lines.push(format!(
                                "sandbox.write `{}` succeeded.",
                                p.strip_prefix(&dir)
                                    .unwrap_or(&p)
                                    .to_string_lossy()
                                    .replace('\\', "/")
                            ));
                            last_opened = Some(p);
                        }
                        Err(e) => status_lines.push(format!("Write blocked/failed ({path}): {e}")),
                    }
                }
                SandboxAction::Append { path, contents } => {
                    match sandbox_ai_text_guard(&path).and_then(|_| sandbox_append(&dir, &path, &contents)) {
                        Ok(p) => {
                            status_lines.push(format!("Appended {}", p.display()));
                            result_lines.push(format!(
                                "sandbox.append `{}` succeeded.",
                                p.strip_prefix(&dir)
                                    .unwrap_or(&p)
                                    .to_string_lossy()
                                    .replace('\\', "/")
                            ));
                            last_opened = Some(p);
                        }
                        Err(e) => status_lines.push(format!("Append blocked/failed ({path}): {e}")),
                    }
                }
                SandboxAction::Read { path } => match sandbox_ai_text_guard(&path)
                    .and_then(|_| sandbox_read(&dir, &path, 200_000))
                {
                    Ok(s) => {
                        let preview = truncate_for_ui(&s, 400);
                        status_lines.push(format!("Read {path}: {preview}"));
                        result_lines.push(format!(
                            "sandbox.read `{path}` succeeded.\n{}",
                            truncate_for_ui(&s, 4_000)
                        ));
                        if let Ok(rel) = parse_sandbox_rel_path(&path) {
                            last_opened = Some(dir.join(rel));
                        }
                    }
                    Err(e) => status_lines.push(format!("Read blocked/failed ({path}): {e}")),
                },
                SandboxAction::List => match sandbox_list(&dir) {
                    Ok(items) => {
                        let preview = if items.is_empty() {
                            "(sandbox is empty)".to_string()
                        } else {
                            items
                                .iter()
                                .take(80)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join("\n")
                        };
                        status_lines.push(format!("Sandbox files: {}", items.join(", ")));
                        result_lines.push(format!("sandbox.list succeeded.\n{preview}"));
                    }
                    Err(e) => status_lines.push(format!("List failed: {e}")),
                },
                SandboxAction::Ledger {
                    status,
                    current_task,
                    next_step,
                    open_questions,
                    files_touched,
                    notes,
                } => match sandbox_write_task_ledger(
                    &dir,
                    &status,
                    &current_task,
                    &next_step,
                    &open_questions,
                    &files_touched,
                    &notes,
                ) {
                    Ok(path) => {
                        status_lines.push(format!("Updated {}", path.display()));
                        result_lines.push(format!(
                            "sandbox.ledger updated `{}`.\n{}",
                            path.strip_prefix(&dir)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .replace('\\', "/"),
                            render_task_ledger_markdown(
                                &status,
                                &current_task,
                                &next_step,
                                &open_questions,
                                &files_touched,
                                &notes,
                            )
                        ));
                        last_opened = Some(path);
                    }
                    Err(e) => status_lines.push(format!("Ledger update failed: {e}")),
                },
                SandboxAction::Preload {
                    paths,
                    include_list,
                    include_scratchpad,
                    include_ledger,
                    note,
                } => {
                    let original_count = paths.len();
                    let filtered_paths = paths
                        .into_iter()
                        .filter(|path| sandbox_rel_path_is_ai_text_allowed(path))
                        .collect::<Vec<_>>();
                    let skipped_count = original_count.saturating_sub(filtered_paths.len());
                    match sandbox_preload(
                        &dir,
                        &filtered_paths,
                        include_list,
                        include_scratchpad,
                        include_ledger,
                        &note,
                    ) {
                        Ok(result) => {
                            status_lines.push(format!("Preloaded {} item(s)", result.loaded_count));
                            if skipped_count > 0 {
                                status_lines.push(format!(
                                    "Skipped {skipped_count} non-text sandbox path(s)"
                                ));
                            }
                            result_lines.push(result.prompt_block);
                            if include_scratchpad {
                                if let Ok(rel) =
                                    parse_sandbox_rel_path(DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH)
                                {
                                    last_opened = Some(dir.join(rel));
                                }
                            } else if include_ledger {
                                if let Ok(rel) =
                                    parse_sandbox_rel_path(DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH)
                                {
                                    last_opened = Some(dir.join(rel));
                                }
                            } else if let Some(first_path) = filtered_paths.first() {
                                if let Ok(rel) = parse_sandbox_rel_path(first_path) {
                                    last_opened = Some(dir.join(rel));
                                }
                            }
                        }
                        Err(e) => status_lines.push(format!("Preload failed: {e}")),
                    }
                }
            }
        }

        if status_lines.is_empty() {
            self.sandbox_action_status = "No actions applied.".to_string();
        } else {
            self.sandbox_action_status = status_lines.join(" | ");
        }

        if let Some(path) = last_opened {
            self.open_sandbox_file_and_focus_tab(&path);
        }

        if result_lines.is_empty() {
            self.sandbox_last_tool_result.clear();
        } else {
            self.sandbox_last_tool_result = result_lines.join("\n\n");
            push_hot_memory(
                self,
                format!("Sandbox: {}", one_line(&self.sandbox_action_status, 120)),
            );
            if let Some(bk) = &self.bookkeeper {
                bk.append(MemoryEvent {
                    ts_unix_ms: now_unix_ms(),
                    kind: MemoryKind::Cold,
                    category: EventCategory::Chat,
                    source: "sandbox".to_string(),
                    module: None,
                    event_type: Some("tool_result".to_string()),
                    text: self.sandbox_last_tool_result.clone(),
                    tags: vec!["sandbox".to_string(), "tool_result".to_string()],
                    entities: Vec::new(),
                    payload_json: None,
                });
            }
        }

        if continue_after && !self.is_generating && !self.sandbox_last_tool_result.trim().is_empty()
        {
            self.start_generation(
                "Continue from the approved sandbox tool result and help with the current task. If another sandbox action is needed, request it as JSON.".to_string(),
            );
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

        if let Some(rx) = &self.lukewarm_rx {
            if let Ok(s) = rx.try_recv() {
                self.lukewarm_summary = s;
                self.lukewarm_rx = None;
                self.lukewarm_poll_due = Some(Instant::now() + Duration::from_secs(2));
            }
        }

        if matches!(self.tab, Tab::Logs | Tab::Chat) && self.lukewarm_rx.is_none() {
            if let Some(due) = self.lukewarm_poll_due {
                if Instant::now() >= due {
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
                        ui.label(format!("GGUF: {}", p.display()));
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
                            Tab::Chat => left_sidebar_chat(ui, self),
                            Tab::Models => left_sidebar_models(ui, self),
                            Tab::Logs => left_sidebar_logs(ui, self),
                            Tab::Networking => left_sidebar_networking(ui, self),
                            Tab::Sandbox => left_sidebar_sandbox(ui, self),
                            Tab::Settings => left_sidebar_settings(ui, self),
                            Tab::About => left_sidebar_about(ui, self),
                            Tab::Module(_) => left_sidebar_about(ui, self),
                        });
                });
        }

        let tab = self.tab.clone();
        egui::CentralPanel::default().show(ctx, |ui| match tab {
            Tab::Chat => chat_tab(ui, ctx, self),
            Tab::Models => models_tab(ui, self),
            Tab::Logs => logs_tab(ui, self),
            Tab::Networking => networking_tab(ui, self),
            Tab::Sandbox => sandbox_tab(ui, self),
            Tab::Settings => settings_tab(ui, self),
            Tab::About => about_tab(ui),
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

fn left_sidebar_chat(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Chat");
    ui.separator();
    ui.label("Hot memory and luke-warm context now live inside the main chat layout.");
    ui.small("Model selection and orchestrator settings live in the Models tab.");
    ui.add_space(8.0);
    if false && app.hot_memory.is_empty() {
        ui.label("(empty)");
    } else if false {
        ui.group(|ui| {
            for item in &app.hot_memory {
                ui.label(format!("• {item}"));
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            app.hot_memory.clear();
        }
        if ui.button("Pin current").clicked() {
            let last_user = app
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| m.content.clone());
            if let Some(t) = last_user {
                push_hot_memory(app, format!("User intent: {}", one_line(&t, 120)));
            }
        }
    });

    ui.add_space(8.0);
    ui.small("Use the Models tab for GGUF selection, presets, and orchestrator tuning.");

    ui.separator();
    ui.add_enabled_ui(app.is_generating, |ui| {
        if ui.button("Stop current response").clicked() {
            app.stop_generation();
        }
    });

    ui.separator();
    if ui.button("Clear chat transcript").clicked() {
        app.pulse_ecg(18.0, "Cleared the chat transcript.");
        app.messages.retain(|m| m.role == Role::System);
        app.assistant_draft.clear();
        if let Some(bk) = &app.bookkeeper {
            bk.append(MemoryEvent {
                ts_unix_ms: now_unix_ms(),
                kind: MemoryKind::Cold,
                category: EventCategory::Module,
                source: "ui".to_string(),
                module: Some("chat".to_string()),
                event_type: Some("clear".to_string()),
                text: "Cleared chat".to_string(),
                tags: Vec::new(),
                entities: Vec::new(),
                payload_json: None,
            });
        }
    }
}

fn left_sidebar_models(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Models");
    ui.separator();
    ui.label("This tab will manage installed GGUFs (scan a folder, add favorites, etc.).");
    ui.separator();
    if let Some(p) = &app.gguf_path {
        ui.label(format!("Active: {}", p.display()));
    } else {
        ui.label("Active: (none)");
    }
}

fn left_sidebar_logs(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Logs");
    ui.separator();

    ui.heading("Luke Warm");
    ui.label("Rolling summary (auto-updated)");
    ui.group(|ui| {
        let mut text = if app.lukewarm_summary.trim().is_empty() {
            "(no summary yet)".to_string()
        } else {
            app.lukewarm_summary.clone()
        };
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .desired_rows(6)
                .interactive(false),
        );
    });

    ui.separator();
    if let Some(dir) = &app.logs_dir {
        ui.label(format!("Folder: {}", dir.display()));
        if ui.button("Open Folder").clicked() {
            let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
        }
    } else {
        ui.label("Folder: (not found)");
    }

    ui.separator();
    ui.heading("Bookkeeper (CPU)");
    ui.label("Model");

    if ui.button("Refresh models").clicked() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }
    if app.models_cache.is_empty() {
        app.models_cache = scan_ggufs(app.models_dir.as_deref());
    }

    egui::ComboBox::from_id_salt("bookkeeper_model_combo")
        .selected_text(
            app.bookkeeper_model_path
                .as_ref()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "(none)".to_string()),
        )
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(&mut app.bookkeeper_model_path, None, "(none)")
                .changed()
            {
                app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
            }
            for p in &app.models_cache {
                let label = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if ui
                    .selectable_value(&mut app.bookkeeper_model_path, Some(p.clone()), label)
                    .changed()
                {
                    app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
                }
            }
        });

    if ui.button("Pick model...").clicked() {
        let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
        if let Some(dir) = &app.models_dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            app.bookkeeper_model_path = Some(path);
            app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
        }
    }

    ui.separator();
    ui.heading("Params");
    add_presets_bookkeeper(ui, app);
    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_temp, 0.0..=2.0).text("temp"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_top_p, 0.0..=1.0).text("top_p"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_top_k, 0..=200).text("top_k"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut app.bookkeeper_max_tokens, 1..=4096).text("max_tokens"))
        .changed();
    if changed {
        app.bookkeeper_restart_due = Some(Instant::now() + Duration::from_millis(600));
    }

    ui.horizontal(|ui| {
        if ui.button("Start/Restart").clicked() {
            if let Some(bk) = &app.bookkeeper {
                bk.shutdown();
            }
            app.bookkeeper =
                start_bookkeeper(app.bookkeeper_model_path.clone(), app.logs_dir.clone());
        }
        if ui.button("Stop").clicked() {
            if let Some(bk) = &app.bookkeeper {
                bk.shutdown();
            }
            app.bookkeeper = None;
            app.bookkeeper_restart_due = None;
        }
    });
}

fn left_sidebar_networking(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    let snapshot = app.networking.snapshot().clone();

    ui.heading("Networking");
    ui.separator();
    ui.label("Local Wi-Fi / LAN mesh between ChattyCog instances.");
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

fn left_sidebar_settings(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("Settings");
    ui.separator();
    ui.label("Theme + defaults will live here.");
}

fn left_sidebar_sandbox(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Chatty Sandbox");
    ui.separator();

    if let Some(dir) = app.sandbox_dir.clone() {
        ui.label(format!("Folder: {}", dir.display()));
        ui.small(format!(
            "Scratchpad: Chatty_Sandbox/{}",
            DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH
        ));
        ui.small(format!(
            "Task ledger: Chatty_Sandbox/{}",
            DEFAULT_SANDBOX_TASK_LEDGER_REL_PATH
        ));
        ui.horizontal(|ui| {
            if ui.button("Open Folder").clicked() {
                let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
            }
            if ui.button("Open Scratchpad").clicked() {
                app.open_default_sandbox_scratchpad();
            }
            if ui.button("Open Task Ledger").clicked() {
                app.open_default_sandbox_task_ledger();
            }
            if ui.button("Refresh").clicked() {
                app.ensure_default_sandbox_scratchpad();
                app.ensure_default_sandbox_task_ledger();
                app.sandbox_status = "Refreshed".to_string();
            }
        });
    } else {
        ui.label("Folder: (not found)");
        if ui.button("Locate/Create").clicked() {
            app.sandbox_dir = find_or_create_sandbox_dir();
            app.ensure_default_sandbox_scratchpad();
            app.ensure_default_sandbox_task_ledger();
        }
    }

    if !app.sandbox_status.trim().is_empty() {
        ui.add_space(6.0);
        ui.label(app.sandbox_status.clone());
    }
}

fn left_sidebar_about(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("About");
    ui.separator();
    ui.label("ChattyCog • Rust GUI");
}

fn render_chat_hot_memory_panel(ui: &mut egui::Ui, app: &mut ChattyCogApp, panel_height: f32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(panel_height);
        ui.heading("Hot Memory");
        ui.small("Recent working cues that stay visible while the conversation moves.");
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("chat_hot_memory_scroll")
            .max_height((panel_height - 92.0).max(120.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if app.hot_memory.is_empty() {
                    ui.label("(empty)");
                } else {
                    for item in app.hot_memory.iter().rev() {
                        egui::Frame::none()
                            .fill(ui.visuals().faint_bg_color)
                            .rounding(egui::Rounding::same(6.0))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(item.as_str()).wrap());
                            });
                        ui.add_space(6.0);
                    }
                }
            });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            if ui.button("Clear").clicked() {
                app.hot_memory.clear();
            }
            if ui.button("Pin current").clicked() {
                let last_user = app
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.content.clone());
                if let Some(t) = last_user {
                    push_hot_memory(app, format!("User intent: {}", one_line(&t, 160)));
                }
            }
        });
    });
}

fn render_chat_lukewarm_panel(ui: &mut egui::Ui, app: &mut ChattyCogApp, panel_height: f32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_height(panel_height);
        ui.heading("Luke Warm");
        ui.small("Rolling summary from the bookkeeper so longer sessions stay grounded.");
        ui.add_space(8.0);

        let mut text = if app.lukewarm_summary.trim().is_empty() {
            "(no summary yet)".to_string()
        } else {
            app.lukewarm_summary.clone()
        };

        egui::ScrollArea::vertical()
            .id_salt("chat_lukewarm_scroll")
            .max_height((panel_height - 64.0).max(120.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .desired_width(f32::INFINITY)
                        .desired_rows(18)
                        .interactive(false),
                );
            });
    });
}

fn render_chat_ecg_window(ui: &mut egui::Ui, app: &ChattyCogApp) {
    let payload = app.ecg_window.payload();
    let desired_size = egui::vec2(208.0, 60.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

    let surface = egui::Color32::from_rgb(238, 244, 248);
    let chart_surface = egui::Color32::from_rgb(247, 250, 252);
    let border = egui::Color32::from_rgb(152, 168, 182);
    let muted = ui.visuals().weak_text_color();
    let accent = if payload.current_percent >= 55.0 {
        egui::Color32::from_rgb(42, 146, 92)
    } else if payload.current_percent >= 20.0 {
        egui::Color32::from_rgb(88, 123, 168)
    } else {
        muted.gamma_multiply(0.9)
    };
    let state = if !payload.supported {
        "unsupported"
    } else if payload.available {
        "live"
    } else {
        "waiting"
    };

    ui.painter().rect(
        rect,
        egui::Rounding::same(8.0),
        surface,
        egui::Stroke::new(1.0, border),
    );

    let inner = rect.shrink2(egui::vec2(10.0, 8.0));
    let small_font = egui::TextStyle::Small.resolve(ui.style());
    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let heading_font = egui::TextStyle::Button.resolve(ui.style());

    ui.painter().text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        "ECG",
        heading_font,
        egui::Color32::from_rgb(74, 92, 112),
    );
    ui.painter().text(
        egui::pos2(inner.min.x + 34.0, inner.min.y + 2.0),
        egui::Align2::LEFT_TOP,
        state,
        small_font,
        muted.gamma_multiply(0.9),
    );
    ui.painter().text(
        inner.right_top(),
        egui::Align2::RIGHT_TOP,
        format!("{:.0}%", payload.current_percent),
        body_font,
        accent,
    );

    let chart_rect = egui::Rect::from_min_max(
        egui::pos2(inner.min.x, inner.min.y + 22.0),
        egui::pos2(inner.max.x, inner.max.y - 10.0),
    );
    ui.painter().rect_filled(
        chart_rect.expand2(egui::vec2(1.0, 2.0)),
        egui::Rounding::same(5.0),
        chart_surface,
    );
    ui.painter().line_segment(
        [
            egui::pos2(chart_rect.left(), chart_rect.bottom()),
            egui::pos2(chart_rect.right(), chart_rect.bottom()),
        ],
        egui::Stroke::new(1.0, border.gamma_multiply(0.6)),
    );

    let points = app
        .ecg_window
        .points(chart_rect.width(), chart_rect.height())
        .into_iter()
        .map(|point| egui::pos2(chart_rect.left() + point.x, chart_rect.top() + point.y))
        .collect::<Vec<_>>();

    if points.len() >= 2 {
        ui.painter()
            .add(egui::Shape::line(points, egui::Stroke::new(1.8, accent)));
    } else if let Some(point) = points.first() {
        ui.painter().circle_filled(*point, 2.0, accent);
    }

    response.on_hover_text(format!(
        "{}\nState: {}\n{}\nCurrent: {:.0}%",
        payload.label, state, payload.note, payload.current_percent
    ));
}

fn chat_tab(ui: &mut egui::Ui, ctx: &egui::Context, app: &mut ChattyCogApp) {
    egui::TopBottomPanel::top("chat_status").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.small(&app.runtime_status);
                    let (badge, color) = runtime_backend_summary(&app.runtime_status);
                    ui.colored_label(color, format!("[{badge}]"));
                    ui.small("Vulkan can still be active even when a few tensors stay on CPU.");
                });
                ui.horizontal_wrapped(|ui| {
                    if app.models_cache.is_empty() {
                        app.models_cache = scan_ggufs(app.models_dir.as_deref());
                    }
                    let model_opts =
                        build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
                    let selected_hint = app.portable_model_hint(app.gguf_path.as_deref());
                    let selected_label = selected_hint
                        .as_ref()
                        .and_then(|hint| {
                            model_opts
                                .iter()
                                .find(|option| option.value == *hint)
                                .map(|option| option.label.clone())
                        })
                        .or_else(|| {
                            app.gguf_path.as_ref().map(|path| {
                                path.file_name()
                                    .map(|name| name.to_string_lossy().to_string())
                                    .unwrap_or_else(|| path.display().to_string())
                            })
                        })
                        .unwrap_or_else(|| "(none)".to_string());

                    ui.small("Model:");
                    egui::ComboBox::from_id_salt("chat_model_combo")
                        .selected_text(selected_label)
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(app.gguf_path.is_none(), "(none)")
                                .clicked()
                            {
                                app.set_active_chat_model_path(None);
                            }
                            for option in &model_opts {
                                let selected =
                                    selected_hint.as_deref() == Some(option.value.as_str());
                                if ui.selectable_label(selected, &option.label).clicked() {
                                    let path = app.resolve_portable_model_hint(Some(&option.value));
                                    app.set_active_chat_model_path(path);
                                }
                            }
                        });
                    if ui.button("Open GGUF...").clicked() {
                        let mut dialog = rfd::FileDialog::new().add_filter("GGUF", &["gguf"]);
                        if let Some(dir) = &app.models_dir {
                            dialog = dialog.set_directory(dir);
                        }
                        if let Some(path) = dialog.pick_file() {
                            app.set_active_chat_model_path(Some(path));
                        }
                    }
                    if ui.button("Refresh models").clicked() {
                        app.models_cache = scan_ggufs(app.models_dir.as_deref());
                    }
                    ui.small(format!("Chat max tokens: {}", app.orch_max_tokens));
                });
                ui.horizontal_wrapped(|ui| {
                    if let Some(capsule) = app.active_orchestrator_capsule() {
                        let preview = truncate_for_ui(&one_line(&capsule.text, 120), 88);
                        ui.small(format!("Voice: capsule '{}'", capsule.name));
                        ui.small(format!("Preview: {preview}"));
                        if ui.button("Use native voice").clicked() {
                            app.prefs.active_orchestrator_capsule = None;
                            app.prefs_status =
                                "Capsule deselected. ChattyCog native voice restored.".to_string();
                        }
                    } else {
                        ui.small("Voice: native ChattyCog");
                        ui.small("No capsule selected.");
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                render_chat_ecg_window(ui, app);
            });
        });
        if app.sandbox_dir.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.small("Sandbox quick access:");
                if ui.button("Open scratchpad").clicked() {
                    if let Some(dir) = app.sandbox_dir.clone() {
                        match ensure_default_sandbox_scratchpad_file(&dir) {
                            Ok(path) => app.open_sandbox_file_and_focus_tab(&path),
                            Err(err) => {
                                app.sandbox_status = format!("Scratchpad setup failed: {err}")
                            }
                        }
                    }
                }
                if ui.button("Open ledger").clicked() {
                    if let Some(dir) = app.sandbox_dir.clone() {
                        match ensure_default_sandbox_task_ledger_file(&dir) {
                            Ok(path) => app.open_sandbox_file_and_focus_tab(&path),
                            Err(err) => {
                                app.sandbox_status = format!("Task ledger setup failed: {err}")
                            }
                        }
                    }
                }
                let last_label = app
                    .sandbox_last_working_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| truncate_for_ui(&name.to_string_lossy(), 40))
                    .unwrap_or_else(|| "none yet".to_string());
                let reopen_response = ui.add_enabled(
                    app.sandbox_last_working_path.is_some(),
                    egui::Button::new("Reopen last working file"),
                );
                if reopen_response.clicked() {
                    app.reopen_last_sandbox_working_file();
                }
                ui.small(format!("Last: {last_label}"));
            });
        }
        if !app.sandbox_task_nudge.trim().is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Task hint: {}", app.sandbox_task_nudge));
                if ui.button("Open task ledger").clicked() {
                    app.open_default_sandbox_task_ledger();
                }
            });
        }
        if !app.sandbox_action_status.trim().is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Sandbox: {}", app.sandbox_action_status));
            });
        }
    });

    let mut send_now = false;
    egui::TopBottomPanel::bottom("chat_input").show_inside(ui, |ui| {
        ui.add_space(2.0);

        let live_task_nudge = if app.prefs.allow_sandbox_tool_requests {
            build_task_ledger_user_hint(&app.composer, app.sandbox_dir.as_deref())
        } else {
            None
        };
        if let Some(hint) = live_task_nudge {
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Task hint: {hint}"));
            });
            ui.add_space(4.0);
        }

        ui.group(|ui| {
            let sandbox_mode_available =
                app.prefs.allow_sandbox_tool_requests && app.sandbox_dir.is_some();
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut app.sandbox_task_enabled, "Sandbox task");
                ui.small("Mark this turn as a sandbox file request so the model skips the guesswork.");
            });
            if app.sandbox_task_enabled {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_enabled_ui(sandbox_mode_available, |ui| {
                        ui.selectable_value(
                            &mut app.sandbox_task_intent,
                            SandboxTaskIntent::Create,
                            "Create file",
                        );
                        ui.selectable_value(
                            &mut app.sandbox_task_intent,
                            SandboxTaskIntent::Edit,
                            "Edit file",
                        );
                    });
                    ui.label("Target:");
                    let response = ui.add_enabled(
                        sandbox_mode_available,
                        egui::TextEdit::singleline(&mut app.sandbox_task_path)
                            .hint_text("notes/request.md")
                            .desired_width(220.0),
                    );
                    if response.changed() {
                        app.sandbox_task_path =
                            normalize_sandbox_task_path_input(&app.sandbox_task_path);
                    }
                });
                let normalized_path = normalize_sandbox_task_path_input(&app.sandbox_task_path);
                if app.sandbox_task_path != normalized_path {
                    app.sandbox_task_path = normalized_path.clone();
                }
                if !sandbox_mode_available {
                    ui.small(
                        "Sandbox task mode needs `Allow sandbox tool requests` enabled and a live `Chatty_Sandbox/` folder.",
                    );
                } else if normalized_path.is_empty() {
                    ui.small("Enter a sandbox `.md` or `.txt` path for this task.");
                } else if let Err(err) = sandbox_ai_text_guard(&normalized_path) {
                    ui.small(format!("Sandbox path blocked: {err}"));
                } else {
                    ui.small(format!(
                        "This turn will explicitly tell the AI to {} `Chatty_Sandbox/{}`.",
                        app.sandbox_task_intent.summary_verb(),
                        normalized_path
                    ));
                }
            }
        });
        ui.add_space(4.0);

        if !app.pending_sandbox_actions.is_empty() {
            if !app.prefs.allow_sandbox_tool_requests {
                app.pending_sandbox_actions.clear();
            }
        }

        if !app.pending_sandbox_actions.is_empty() {
            ui.group(|ui| {
                ui.label("Pending sandbox actions (requires approval):");
                for a in &app.pending_sandbox_actions {
                    match a {
                        SandboxAction::Write { path, .. } => ui.label(format!("- write: {path}")),
                        SandboxAction::Append { path, .. } => ui.label(format!("- append: {path}")),
                        SandboxAction::Read { path } => ui.label(format!("- read: {path}")),
                        SandboxAction::List => ui.label("- list".to_string()),
                        SandboxAction::Preload {
                            paths,
                            include_list,
                            include_scratchpad,
                            include_ledger,
                            note,
                        } => {
                            let mut parts = Vec::new();
                            if *include_list {
                                parts.push("list".to_string());
                            }
                            if *include_scratchpad {
                                parts.push("scratchpad".to_string());
                            }
                            if *include_ledger {
                                parts.push("task ledger".to_string());
                            }
                            if !paths.is_empty() {
                                parts.push(format!("files: {}", paths.join(", ")));
                            }
                            if !note.trim().is_empty() {
                                parts.push(format!("note: {}", note));
                            }
                            ui.label(format!("- preload: {}", parts.join(" | ")))
                        }
                        SandboxAction::Ledger {
                            status,
                            current_task,
                            next_step,
                            open_questions,
                            files_touched,
                            ..
                        } => {
                            let mut parts = vec![format!("status: {}", status.trim())];
                            if !current_task.trim().is_empty() {
                                parts.push(format!(
                                    "task: {}",
                                    truncate_for_ui(current_task.trim(), 80)
                                ));
                            }
                            if !next_step.trim().is_empty() {
                                parts.push(format!(
                                    "next: {}",
                                    truncate_for_ui(next_step.trim(), 80)
                                ));
                            }
                            if !open_questions.is_empty() {
                                parts.push(format!("questions: {}", open_questions.len()));
                            }
                            if !files_touched.is_empty() {
                                parts.push(format!("files: {}", files_touched.join(", ")));
                            }
                            ui.label(format!("- ledger: {}", parts.join(" | ")))
                        }
                    };
                }
                ui.horizontal(|ui| {
                    if ui.button("Seed ledger from current prompt").clicked() {
                        app.seed_default_sandbox_task_ledger_from_context();
                    }
                    if ui.button("Defer actions").clicked() {
                        app.defer_pending_sandbox_actions();
                    }
                    if ui.button("Preload + Continue").clicked() {
                        app.preload_sandbox_and_continue();
                    }
                    if ui.button("Approve").clicked() {
                        app.apply_pending_sandbox_actions(false);
                    }
                    if ui.button("Approve + Continue").clicked() {
                        app.apply_pending_sandbox_actions(true);
                    }
                    if ui.button("Reject").clicked() {
                        app.pending_sandbox_actions.clear();
                        app.sandbox_action_status = "Rejected sandbox actions.".to_string();
                    }
                });
            });
            ui.add_space(4.0);
        }

        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut app.networking_shared_chat_mirror_main_chat,
                "Mirror this chat into the shared room",
            );
            if app.networking_shared_chat_mirror_main_chat {
                ui.small(format!("Mode: {}", app.shared_chat_policy_summary()));
                if !app.shared_chat_local_ai_allowed() {
                    ui.small("Local AI reply is currently disabled by room policy.");
                }
            }
        });

        ui.horizontal(|ui| {
            let paused = app.orch_freeze_pending || matches!(&app.tab, Tab::Module(_));
            let waiting_for_reply = app.is_generating;
            let composer_enabled = !paused && !waiting_for_reply;
            let input = ui.add_enabled_ui(composer_enabled, |ui| {
                ui.add_sized(
                    [ui.available_width() - 200.0, 56.0],
                    egui::TextEdit::multiline(&mut app.composer)
                        .hint_text(if paused {
                            "Orchestrator paused (module active)..."
                        } else if waiting_for_reply {
                            "Please wait for the current reply to finish or press Interrupt..."
                        } else {
                            "Type a message...  Enter sends, Shift+Enter adds a new line."
                        })
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
            });

            if input.inner.has_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift)
            {
                send_now = true;
            }

            ui.add_enabled_ui(!waiting_for_reply && !paused, |ui| {
                if ui.button("Send").clicked() {
                    send_now = true;
                }
            });
            ui.add_enabled_ui(waiting_for_reply, |ui| {
                if ui.button("Interrupt").clicked() {
                    app.stop_generation();
                }
            });
        });
        if app.is_generating {
            ui.horizontal_wrapped(|ui| {
                ui.small(
                    "Please wait: ChattyCog is still generating the current reply. Interrupt it if you want to change course before sending another message.",
                );
            });
        }
        ui.add_space(2.0);
    });

    egui::CentralPanel::default().show_inside(ui, |ui| {
        let panel_height = ui.available_height().max(320.0);
        let gap = 10.0;
        let side_width = (ui.available_width() * 0.22).clamp(220.0, 320.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(side_width, panel_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| render_chat_hot_memory_panel(ui, app, panel_height),
            );
            ui.add_space(gap);

            let center_width = (ui.available_width() - side_width - gap).max(320.0);
            ui.allocate_ui_with_layout(
                egui::vec2(center_width, panel_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(10.0))
                        .show(ui, |ui| {
                            ui.heading("Chat");
                            ui.add_space(6.0);
                            egui::ScrollArea::vertical()
                                .id_salt("chat_scroll")
                                .stick_to_bottom(app.scroll_to_bottom)
                                .auto_shrink([false, false])
                                .max_height(panel_height - 24.0)
                                .show(ui, |ui| {
                                    for msg in &app.messages {
                                        message_bubble(ui, msg);
                                    }
                                    if app.is_generating && !app.assistant_draft.is_empty() {
                                        let (visible, thinking) =
                                            split_assistant_output(&app.assistant_draft);
                                        message_bubble(
                                            ui,
                                            &Message {
                                                role: Role::Assistant,
                                                content: visible,
                                                thinking,
                                            },
                                        );
                                    }
                                });
                        });
                },
            );
            ui.add_space(gap);

            ui.allocate_ui_with_layout(
                egui::vec2(side_width, panel_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| render_chat_lukewarm_panel(ui, app, panel_height),
            );
        });
    });

    app.scroll_to_bottom = false;

    if send_now {
        if app.is_generating {
            app.runtime_status =
                "Runtime: please wait for the current reply to finish, or press Interrupt."
                    .to_string();
            ctx.request_repaint();
            return;
        }
        let content = app.composer.trim().to_string();
        if !content.is_empty() {
            let sandbox_path = normalize_sandbox_task_path_input(&app.sandbox_task_path);
            let sandbox_mode_active = app.sandbox_task_enabled;
            let mut generation_prompt = content.clone();
            let visible_user_message = if sandbox_mode_active {
                format!(
                    "[Sandbox {} -> {}] {}",
                    app.sandbox_task_intent.label(),
                    sandbox_path,
                    content
                )
            } else {
                content.clone()
            };
            app.sandbox_task_nudge =
                build_task_ledger_user_hint(&content, app.sandbox_dir.as_deref())
                    .unwrap_or_default();
            if sandbox_mode_active {
                if !app.prefs.allow_sandbox_tool_requests {
                    app.sandbox_action_status =
                        "Sandbox task mode needs `Allow sandbox tool requests` turned on."
                            .to_string();
                    ctx.request_repaint();
                    return;
                }
                if app.sandbox_dir.is_none() {
                    app.sandbox_action_status =
                        "Sandbox task mode needs a live `Chatty_Sandbox/` folder.".to_string();
                    ctx.request_repaint();
                    return;
                }
                if sandbox_path.is_empty() {
                    app.sandbox_action_status =
                        "Sandbox task mode needs a target `.md` or `.txt` path.".to_string();
                    ctx.request_repaint();
                    return;
                }
                if let Err(err) = sandbox_ai_text_guard(&sandbox_path) {
                    app.sandbox_action_status = format!("Sandbox task path blocked: {err}");
                    ctx.request_repaint();
                    return;
                }
                generation_prompt = build_explicit_sandbox_task_prompt(
                    &content,
                    &sandbox_path,
                    app.sandbox_task_intent,
                );
            }
            if let Err(reason) = app.shared_chat_can_send_mirrored_main_chat_message() {
                app.networking_status = format!("Shared room: {reason}");
                ctx.request_repaint();
                return;
            }
            app.composer.clear();
            app.pulse_ecg(20.0, "Queued a chat message.");
            app.messages.push(Message {
                role: Role::User,
                content: visible_user_message,
                thinking: None,
            });
            trim_live_chat_messages(&mut app.messages);
            push_hot_memory(app, format!("User: {}", one_line(&content, 120)));
            if let Some(bk) = &app.bookkeeper {
                bk.append(MemoryEvent {
                    ts_unix_ms: now_unix_ms(),
                    kind: MemoryKind::Cold,
                    category: EventCategory::Chat,
                    source: "user".to_string(),
                    module: None,
                    event_type: Some("message".to_string()),
                    text: content.clone(),
                    tags: Vec::new(),
                    entities: Vec::new(),
                    payload_json: None,
                });
            }
            app.scroll_to_bottom = true;
            if app.networking_shared_chat_mirror_main_chat {
                app.broadcast_shared_chat_message("user", "You", &content);
            }
            if app.shared_chat_local_ai_allowed() {
                app.start_generation(generation_prompt);
            } else {
                app.runtime_status =
                    "Runtime: shared room policy left AI off for this local turn.".to_string();
            }
            ctx.request_repaint();
        }
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
                let selected_label = model_opts
                    .iter()
                    .find(|o| o.value == selected)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| if selected.is_empty() { "(none)".to_string() } else { selected.clone() });

                egui::ComboBox::from_id_salt(("preferred_model", m.module_id.as_str()))
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut entry.preferred_model, None, "(none)");
                        for o in &model_opts {
                            ui.selectable_value(&mut entry.preferred_model, Some(o.value.clone()), o.label.clone());
                        }
                    });

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
    ui.heading("Preferences");
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("prefs_scroll_v2")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Prefs file: {}", app.prefs_path.display()));
                if ui.button("Reload").clicked() {
                    match preferences::load_prefs(&app.prefs_path) {
                        Ok(p) => {
                            app.prefs = p;
                            app.ensure_persisted_network_identity();
                            app.apply_prefs_to_runtime_settings();
                            app.sync_capsule_selection_from_prefs();
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
            ui.columns(2, |columns| {
                let left = &mut columns[0];
                left.group(|ui| {
                    ui.heading("Orchestrator (Chat)");
                    let mut live_changed = false;
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
                    live_changed |= ui
                        .add(egui::Slider::new(&mut app.prefs.orchestrator.temp, 0.0..=2.0).text("temp"))
                        .changed();
                    live_changed |= ui
                        .add(egui::Slider::new(&mut app.prefs.orchestrator.top_p, 0.0..=1.0).text("top_p"))
                        .changed();
                    live_changed |= ui
                        .add(egui::Slider::new(&mut app.prefs.orchestrator.top_k, 0..=200).text("top_k"))
                        .changed();
                    live_changed |= ui
                        .add(
                        egui::Slider::new(&mut app.prefs.orchestrator.max_tokens, 1..=4096)
                            .text("max_tokens"))
                        .changed();
                    if live_changed {
                        app.apply_live_orchestrator_prefs();
                        app.prefs_status =
                            format!("Live chat settings updated. Chat max tokens now {}.", app.orch_max_tokens);
                    }
                });

                left.add_space(8.0);
                left.group(|ui| {
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
                            app.bookkeeper_restart_due =
                                Some(Instant::now() + Duration::from_millis(200));
                            app.prefs_status =
                                "Applied bookkeeper settings (restart pending).".to_string();
                        }
                    });
                    add_presets_prefs_bookkeeper(ui, &mut app.prefs.bookkeeper);
                    ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.temp, 0.0..=2.0).text("temp"));
                    ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_p, 0.0..=1.0).text("top_p"));
                    ui.add(egui::Slider::new(&mut app.prefs.bookkeeper.top_k, 0..=200).text("top_k"));
                    ui.add(
                        egui::Slider::new(&mut app.prefs.bookkeeper.max_tokens, 1..=4096)
                            .text("max_tokens"),
                    );
                });

                left.add_space(8.0);
                left.group(|ui| {
                    ui.heading("Access / Tools");
                    ui.checkbox(
                        &mut app.prefs.allow_sandbox_tool_requests,
                        "Allow sandbox tool requests (user-approved)",
                    );
                    ui.small(
                        "If disabled, Chat tab won't parse tool JSON requests and will hide the approval panel.",
                    );
                    ui.add_space(6.0);
                    ui.checkbox(
                        &mut app.prefs.auto_generate_module_suspend_rundown,
                        "Auto-generate module suspend rundown on tab leave (Bookkeeper)",
                    );
                    ui.small(
                        "If enabled, leaving a module tab will auto-write a short department update into cold logs for cross-module awareness.",
                    );
                });

                left.add_space(8.0);
                left.group(|ui| {
                    ui.heading("Per-module preferences");
                    ui.small("Defaults for modules that have AI enabled (or future module runners).");

                    let model_opts =
                        build_model_options(app.models_dir.as_deref(), app.modules_dir.as_deref());
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
                            let selected_label = model_opts
                                .iter()
                                .find(|o| o.value == selected)
                                .map(|o| o.label.clone())
                                .unwrap_or_else(|| {
                                    if selected.is_empty() {
                                        "(none)".to_string()
                                    } else {
                                        selected.clone()
                                    }
                                });

                            egui::ComboBox::from_id_salt(("preferred_model", m.module_id.as_str()))
                                .selected_text(selected_label)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut entry.preferred_model, None, "(none)");
                                    for o in &model_opts {
                                        ui.selectable_value(
                                            &mut entry.preferred_model,
                                            Some(o.value.clone()),
                                            o.label.clone(),
                                        );
                                    }
                                });

                            add_presets_prefs_orchestrator(ui, &mut entry.params);
                            ui.add(egui::Slider::new(&mut entry.params.temp, 0.0..=2.0).text("temp"));
                            ui.add(egui::Slider::new(&mut entry.params.top_p, 0.0..=1.0).text("top_p"));
                            ui.add(egui::Slider::new(&mut entry.params.top_k, 0..=200).text("top_k"));
                            ui.add(
                                egui::Slider::new(&mut entry.params.max_tokens, 1..=4096)
                                    .text("max_tokens"),
                            );
                            ui.checkbox(
                                &mut entry.allow_receive_lukewarm_context,
                                "Allow luke warm context",
                            );
                        });
                    }
                });

                let right = &mut columns[1];
                right.group(|ui| {
                    ui.heading("Capsule Library");
                    ui.small(
                        "Save reusable personality or behavior injections here, then activate one when a task needs a different tone, role, or voice.",
                    );
                    ui.add_space(6.0);

                    let capsule_names = app
                        .prefs
                        .orchestrator_capsules
                        .iter()
                        .map(|capsule| capsule.name.clone())
                        .collect::<Vec<_>>();
                    let active_label = app
                        .prefs
                        .active_orchestrator_capsule
                        .as_deref()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or("(none)")
                        .to_string();

                    ui.horizontal(|ui| {
                        ui.label("Active capsule");
                        egui::ComboBox::from_id_salt("active_orchestrator_capsule")
                            .selected_text(active_label)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        app.prefs.active_orchestrator_capsule.is_none(),
                                        "(none)",
                                    )
                                    .clicked()
                                {
                                    app.prefs.active_orchestrator_capsule = None;
                                }
                                for name in &capsule_names {
                                    if ui
                                        .selectable_label(
                                            app.prefs.active_orchestrator_capsule.as_deref()
                                                == Some(name.as_str()),
                                            name,
                                        )
                                        .clicked()
                                    {
                                        app.prefs.active_orchestrator_capsule = Some(name.clone());
                                        app.capsule_selected_name = Some(name.clone());
                                        if let Some(capsule) = app
                                            .prefs
                                            .orchestrator_capsules
                                            .iter()
                                            .find(|capsule| capsule.name == *name)
                                        {
                                            app.capsule_editor_name = capsule.name.clone();
                                            app.capsule_editor_text = capsule.text.clone();
                                        }
                                    }
                                }
                            });
                        if ui.button("Use native voice").clicked() {
                            app.prefs.active_orchestrator_capsule = None;
                            app.prefs_status =
                                "Capsule deselected. ChattyCog native voice restored.".to_string();
                        }
                    });
                    ui.small(
                        "Choose '(none)' or 'Use native voice' to fall back to ChattyCog's built-in personality.",
                    );

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Saved capsules");
                        if ui.button("Deselect editor").clicked() {
                            app.capsule_selected_name = None;
                            app.capsule_editor_name.clear();
                            app.capsule_editor_text.clear();
                            app.prefs_status = "Capsule editor cleared.".to_string();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("orchestrator_capsule_list")
                        .max_height(180.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if capsule_names.is_empty() {
                                ui.small("No capsules saved yet.");
                            }
                            let nothing_selected = app.capsule_selected_name.is_none()
                                && app.capsule_editor_name.trim().is_empty()
                                && app.capsule_editor_text.trim().is_empty();
                            if ui
                                .selectable_label(nothing_selected, "(deselect editor)")
                                .clicked()
                            {
                                app.capsule_selected_name = None;
                                app.capsule_editor_name.clear();
                                app.capsule_editor_text.clear();
                            }
                            for name in &capsule_names {
                                let selected =
                                    app.capsule_selected_name.as_deref() == Some(name.as_str());
                                if ui.selectable_label(selected, name).clicked() {
                                    app.capsule_selected_name = Some(name.clone());
                                    if let Some(capsule) = app
                                        .prefs
                                        .orchestrator_capsules
                                        .iter()
                                        .find(|capsule| capsule.name == *name)
                                    {
                                        app.capsule_editor_name = capsule.name.clone();
                                        app.capsule_editor_text = capsule.text.clone();
                                    }
                                }
                            }
                        });

                    ui.add_space(8.0);
                    ui.label("Capsule name");
                    ui.text_edit_singleline(&mut app.capsule_editor_name);
                    ui.add_space(4.0);
                    ui.label("Capsule instructions");
                    ui.add(
                        egui::TextEdit::multiline(&mut app.capsule_editor_text)
                            .desired_rows(16)
                            .hint_text(
                                "Your nickname is Barry. You are a scriptwriter. Answer verbosely and keep a cinematic tone.",
                            ),
                    );

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("New").clicked() {
                            app.capsule_selected_name = None;
                            app.capsule_editor_name.clear();
                            app.capsule_editor_text.clear();
                        }

                        if ui.button("Save capsule").clicked() {
                            let name = app.capsule_editor_name.trim().to_string();
                            let text = app.capsule_editor_text.trim().to_string();
                            if name.is_empty() || text.is_empty() {
                                app.prefs_status =
                                    "Capsule needs both a name and instructions.".to_string();
                            } else if let Some(existing) = app
                                .prefs
                                .orchestrator_capsules
                                .iter_mut()
                                .find(|capsule| capsule.name == name)
                            {
                                existing.text = text;
                                app.capsule_selected_name = Some(name.clone());
                                app.prefs_status = format!("Updated capsule '{name}'.");
                            } else {
                                app.prefs.orchestrator_capsules.push(PromptCapsule {
                                    name: name.clone(),
                                    text,
                                });
                                app.prefs.orchestrator_capsules.sort_by(|a, b| {
                                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                                });
                                app.capsule_selected_name = Some(name.clone());
                                app.prefs_status = format!("Saved capsule '{name}'.");
                            }
                        }

                        if ui.button("Set active").clicked() {
                            let name = app.capsule_editor_name.trim().to_string();
                            if name.is_empty() {
                                app.prefs_status = "Choose or save a capsule first.".to_string();
                            } else if app
                                .prefs
                                .orchestrator_capsules
                                .iter()
                                .any(|capsule| capsule.name == name)
                            {
                                app.prefs.active_orchestrator_capsule = Some(name.clone());
                                app.capsule_selected_name = Some(name.clone());
                                app.prefs_status = format!("Activated capsule '{name}'.");
                            } else {
                                app.prefs_status =
                                    "Save the capsule before making it active.".to_string();
                            }
                        }

                        let delete_target = app
                            .capsule_selected_name
                            .clone()
                            .filter(|name| !name.trim().is_empty())
                            .or_else(|| {
                                let editor_name = app.capsule_editor_name.trim().to_string();
                                if editor_name.is_empty() {
                                    None
                                } else {
                                    Some(editor_name)
                                }
                            });
                        if ui
                            .add_enabled(delete_target.is_some(), egui::Button::new("Delete"))
                            .clicked()
                        {
                            if let Some(target) = delete_target {
                                app.prefs
                                    .orchestrator_capsules
                                    .retain(|capsule| capsule.name != target);
                                if app.prefs.active_orchestrator_capsule.as_deref()
                                    == Some(target.as_str())
                                {
                                    app.prefs.active_orchestrator_capsule = None;
                                }
                                app.capsule_selected_name = None;
                                app.capsule_editor_name.clear();
                                app.capsule_editor_text.clear();
                                app.sync_capsule_selection_from_prefs();
                                app.prefs_status = format!("Deleted capsule '{target}'.");
                            }
                        }
                    });
                });
            });
        });
}

fn networking_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    let snapshot = app.networking.snapshot().clone();
    if app
        .networking_focus_flash_until
        .is_some_and(|until| Instant::now() >= until)
    {
        app.networking_focus_flash_until = None;
        app.networking_focus_section = None;
    }
    let pending_focus = app.networking_focus_pending.take();
    let highlighted_section = app.networking_focus_section;
    let highlight_until = app.networking_focus_flash_until;
    let highlight_active = |section: NetworkingFocusSection| {
        highlighted_section == Some(section)
            && highlight_until.is_some_and(|until| Instant::now() < until)
    };
    let blend_color = |base: egui::Color32, tint: egui::Color32, amount: f32| {
        let inverse = 1.0 - amount;
        egui::Color32::from_rgba_premultiplied(
            ((base.r() as f32 * inverse) + (tint.r() as f32 * amount)).round() as u8,
            ((base.g() as f32 * inverse) + (tint.g() as f32 * amount)).round() as u8,
            ((base.b() as f32 * inverse) + (tint.b() as f32 * amount)).round() as u8,
            base.a().max(244),
        )
    };
    let network_card_frame =
        |ui: &egui::Ui, tint: egui::Color32, accent: egui::Color32, selected: bool| {
            let fill_mix = if selected {
                if ui.visuals().dark_mode { 0.26 } else { 0.16 }
            } else if ui.visuals().dark_mode {
                0.18
            } else {
                0.10
            };
            let stroke_mix = if selected {
                if ui.visuals().dark_mode { 0.80 } else { 0.55 }
            } else if ui.visuals().dark_mode {
                0.55
            } else {
                0.35
            };
            let stroke_width = if selected { 1.6 } else { 1.0 };
            egui::Frame::group(ui.style())
                .fill(blend_color(ui.visuals().panel_fill, tint, fill_mix))
                .stroke(egui::Stroke::new(
                    stroke_width,
                    blend_color(
                        ui.visuals().widgets.noninteractive.bg_stroke.color,
                        accent,
                        stroke_mix,
                    ),
                ))
                .inner_margin(egui::Margin::same(8.0))
        };

    let local_connection_info = {
        let mut parts = vec![
            format!("Name: {}", snapshot.device_name),
            format!("Device ID: {}", snapshot.device_id),
        ];
        if let Some(port) = snapshot.listener_port {
            parts.push(format!("Listener port: {port}"));
        } else {
            parts.push("Listener: client only".to_string());
        }
        parts.push(format!(
            "Visibility: {}",
            if snapshot.available_for_connectivity {
                "available"
            } else {
                "hidden"
            }
        ));
        if !snapshot.local_presence.active_tab.trim().is_empty() {
            parts.push(format!(
                "Active tab: {}",
                snapshot.local_presence.active_tab
            ));
        }
        parts.join(" | ")
    };
    if snapshot
        .connected_peers
        .iter()
        .all(|peer| peer.connection_id != app.networking_handoff_target)
    {
        app.networking_handoff_target = snapshot
            .connected_peers
            .first()
            .map(|peer| peer.connection_id.clone())
            .unwrap_or_default();
    }

    let filter_text = app.networking_filter.trim().to_lowercase();
    let matches_filter = |name: &str, device_id: &str, address: &str, group: Option<String>| {
        if filter_text.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            name.to_lowercase(),
            device_id.to_lowercase(),
            address.to_lowercase(),
            group.unwrap_or_default().to_lowercase(),
        );
        haystack.contains(&filter_text)
    };
    let connected_visible = snapshot
        .connected_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let available_visible = snapshot
        .discovered_peers
        .iter()
        .filter(|peer| {
            peer.connected_connection_id.is_none()
                && matches_filter(
                    &app.network_display_name(&peer.device_id, &peer.device_name),
                    &peer.device_id,
                    &format!("{}:{}", peer.address, peer.host_port),
                    app.network_group_label(&peer.device_id),
                )
        })
        .collect::<Vec<_>>();
    let blocked_visible = snapshot
        .blocked_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let trusted_visible = snapshot
        .trusted_peers
        .iter()
        .filter(|peer| {
            matches_filter(
                &app.network_display_name(&peer.device_id, &peer.device_name),
                &peer.device_id,
                &peer.address,
                app.network_group_label(&peer.device_id),
            )
        })
        .collect::<Vec<_>>();
    let shared_room_connection_ids = snapshot
        .connected_peers
        .iter()
        .map(|peer| peer.connection_id.clone())
        .collect::<Vec<_>>();
    let delivery_visible = snapshot
        .outgoing_artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind != "shared_chat_policy_json"
                && artifact.kind != "shared_chat_message_json"
        })
        .collect::<Vec<_>>();
    let received_transfer_visible = snapshot
        .received_artifacts
        .iter()
        .filter(|artifact| {
            artifact.kind != "shared_chat_policy_json"
                && artifact.kind != "shared_chat_message_json"
        })
        .collect::<Vec<_>>();
    let section_heading = |ui: &mut egui::Ui, icon: &str, color: egui::Color32, title: &str| {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(icon).color(color).strong());
            ui.heading(title);
        });
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("Networking");
            ui.separator();
            ui.label(
                "Connect nearby ChattyCog instances over the local Wi-Fi / LAN. Turn one instance on as the host, then scan and connect from the others.",
            );
            egui::CollapsingHeader::new("Quick help")
                .id_salt("chattycog_networking_quick_help")
                .default_open(false)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Presets").strong());
                        if ui
                            .selectable_value(
                                &mut app.networking_help_mode,
                                NetworkingQuickHelpMode::Everyday,
                                "Everyday",
                            )
                            .clicked()
                        {
                            app.focus_networking_section(NetworkingFocusSection::DeviceList);
                        }
                        if ui
                            .selectable_value(
                                &mut app.networking_help_mode,
                                NetworkingQuickHelpMode::HostSetup,
                                "Host setup",
                            )
                            .clicked()
                        {
                            app.focus_networking_section(NetworkingFocusSection::Controls);
                        }
                        if ui
                            .selectable_value(
                                &mut app.networking_help_mode,
                                NetworkingQuickHelpMode::ApprovalFirst,
                                "Approval first",
                            )
                            .clicked()
                        {
                            let target = if snapshot.pending_requests.is_empty() {
                                NetworkingFocusSection::Controls
                            } else {
                                NetworkingFocusSection::PendingRequests
                            };
                            app.focus_networking_section(target);
                        }
                    });
                    ui.add_space(4.0);

                    let help_rows = match app.networking_help_mode {
                        NetworkingQuickHelpMode::Everyday => vec![
                            (
                                "[HOST]",
                                "Turn on `Make available for connectivity` on the machine that should be visible.",
                            ),
                            (
                                "[SCAN]",
                                "Click `Refresh discovery` on the other machine, then `Connect` when it appears.",
                            ),
                            (
                                "[NAME]",
                                "Click a device name to give it a friendlier local name, or click `+ Group` to tag it.",
                            ),
                            (
                                "[FIND]",
                                "Use `Find` to search by name, device ID, address, or group label.",
                            ),
                            (
                                "[FAST]",
                                "`Select Connected` is the quickest way to act on the peers that are live right now.",
                            ),
                            (
                                "[SETUP]",
                                "Use `Workflow bundle` when you want to share the whole ChattyCog setup; use handoffs for short notes and module shares for module-specific state.",
                            ),
                            (
                                "[ROOM]",
                                "Use `Shared room chat` when you want a lightweight cross-instance room with talking-stick and AI-on/off rules.",
                            ),
                        ],
                        NetworkingQuickHelpMode::HostSetup => vec![
                            (
                                "[HOST]",
                                "Use this when you are the visible machine and other nearby ChattyCog instances should connect to you.",
                            ),
                            (
                                "[CHECK]",
                                "Keep an eye on the status line, listener port, and connected section so you can tell whether hosting is actually up.",
                            ),
                            (
                                "[TRUST]",
                                "Leave `Allow unknown devices` on for a relaxed trusted-room setup, or switch it off if you want approval prompts.",
                            ),
                            (
                                "[LABEL]",
                                "Rename frequently used peers so they stay recognizable the next time they appear.",
                            ),
                            (
                                "[BND]",
                                "Send a workflow bundle when you want nearby peers to mirror this machine's current setup, prompts, and AI preferences without copying logs or cold memory.",
                            ),
                            (
                                "[ROOM]",
                                "If you want one orderly shared room instead of several separate local chats, use the `Shared room chat` controls just below.",
                            ),
                            (
                                "[DEBUG]",
                                "Use `Copy info` when you need a clean support/debug snapshot of name, ID, and address.",
                            ),
                            (
                                "[PAIR]",
                                "Use `Export trusted list` / `Import trusted list` for remembered pairings, and `Export blocked list` / `Import blocked list` when you want another ChattyCog machine to inherit the same deny rules.",
                            ),
                            (
                                "[SYNC]",
                                "If a nearby machine shows up but refuses to talk cleanly, check the `Compatibility note` line to spot protocol/version mismatch quickly.",
                            ),
                        ],
                        NetworkingQuickHelpMode::ApprovalFirst => vec![
                            (
                                "[LOCK]",
                                "Turn off `Allow unknown devices` if you want new peers to ask first instead of joining freely.",
                            ),
                            (
                                "[QUEUE]",
                                "Pending requests appear above the device list, where you can Allow, Deny, or Block them.",
                            ),
                            (
                                "[BLOCK]",
                                "`Block` disconnects the peer now and keeps it out until you unblock it later.",
                            ),
                            (
                                "[REVIEW]",
                                "Use `Copy ID` or `Copy info` before allowing a device if you need to confirm which machine it is.",
                            ),
                            (
                                "[INBOX]",
                                "Received workflow bundles land in `Received setup bundles`, where you can preview them calmly before applying anything.",
                            ),
                            (
                                "[ROOM]",
                                "Use `Broadcast current room policy` when you want every connected peer to see the same talking-stick and AI rules.",
                            ),
                            (
                                "[RESET]",
                                "Blocked devices live in their own section so you can review and unblock them deliberately.",
                            ),
                            (
                                "[PAIR]",
                                "Trusted and blocked lists are portable now, so you can export a known-good policy set and import it on another local machine instead of rebuilding it by hand.",
                            ),
                        ],
                    };
                    for (tag, body) in help_rows {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(tag).monospace().strong());
                            ui.small(body);
                        });
                    }
                });

            ui.add_space(8.0);
            let controls_highlight = highlight_active(NetworkingFocusSection::Controls);
            let controls = egui::Frame::group(ui.style())
                .fill(if controls_highlight {
                    egui::Color32::from_rgb(246, 250, 255)
                } else {
                    ui.visuals().panel_fill
                })
                .stroke(if controls_highlight {
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(70, 110, 180))
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                })
                .show(ui, |ui| {
                    section_heading(
                        ui,
                        "[CTL]",
                        egui::Color32::from_rgb(70, 110, 180),
                        "Network controls",
                    );
                    if controls_highlight {
                        ui.small(
                            egui::RichText::new("Focused by Quick help")
                                .strong()
                                .color(egui::Color32::from_rgb(70, 110, 180)),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        let mut available = snapshot.available_for_connectivity;
                        if ui
                            .checkbox(&mut available, "Make available for connectivity")
                            .changed()
                        {
                            app.networking.set_available(available);
                        }
                        let mut allow_unknown = snapshot.allow_unknown_devices;
                        if ui
                            .checkbox(&mut allow_unknown, "Allow unknown devices")
                            .changed()
                        {
                            app.networking.set_allow_unknown_devices(allow_unknown);
                            app.prefs.network_allow_unknown_devices = allow_unknown;
                            app.persist_network_prefs();
                        }
                        let mut allow_shared_lukewarm =
                            app.prefs.network_allow_shared_lukewarm_context;
                        if ui
                            .checkbox(
                                &mut allow_shared_lukewarm,
                                "Allow shared luke warm context",
                            )
                            .changed()
                        {
                            app.prefs.network_allow_shared_lukewarm_context =
                                allow_shared_lukewarm;
                            app.persist_network_prefs();
                        }
                        if ui.button("Refresh discovery").clicked() {
                            app.networking.refresh_discovery();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let has_trusted = !app.prefs.network_trusted_devices.is_empty();
                        let has_blocked = !app.prefs.network_blocked_devices.is_empty();
                        if ui
                            .add_enabled(has_trusted, egui::Button::new("Export trusted list"))
                            .clicked()
                        {
                            app.export_trusted_peer_list();
                        }
                        if ui.button("Import trusted list").clicked() {
                            app.import_trusted_peer_list();
                        }
                        if ui
                            .add_enabled(has_blocked, egui::Button::new("Export blocked list"))
                            .clicked()
                        {
                            app.export_blocked_peer_list();
                        }
                        if ui.button("Import blocked list").clicked() {
                            app.import_blocked_peer_list();
                        }
                        if !has_trusted {
                            ui.small(
                                "Trust a few regular peers first if you want to export a reusable pairing list.",
                            );
                        } else if !has_blocked {
                            ui.small(
                                "Blocked lists are handy when you want another machine to inherit the same deny rules.",
                            );
                        }
                    });
                });
            if pending_focus == Some(NetworkingFocusSection::Controls) {
                controls.response.scroll_to_me(Some(egui::Align::Center));
            }

            if !snapshot.pending_requests.is_empty() {
                ui.add_space(8.0);
                let pending_highlight = highlight_active(NetworkingFocusSection::PendingRequests);
                let pending = egui::Frame::group(ui.style())
                    .fill(if pending_highlight {
                        egui::Color32::from_rgb(255, 248, 240)
                    } else {
                        ui.visuals().panel_fill
                    })
                    .stroke(if pending_highlight {
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(190, 110, 30))
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    })
                    .show(ui, |ui| {
                    section_heading(
                        ui,
                        "[REQ]",
                        egui::Color32::from_rgb(190, 110, 30),
                        "Pending device requests",
                    );
                    if pending_highlight {
                        ui.small(
                            egui::RichText::new("Focused by Quick help")
                                .strong()
                                .color(egui::Color32::from_rgb(190, 110, 30)),
                        );
                    }
                    for request in &snapshot.pending_requests {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!(
                                "Unknown device {} [{}] requesting connection.",
                                request.device_name, request.device_id
                            ));
                            ui.small(format!("{} | {}s ago", request.address, request.requested_secs_ago));
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Allow").clicked() {
                                app.networking.allow_pending_peer(&request.device_id);
                            }
                            if ui.button("Trust").clicked() {
                                app.trust_network_peer(&request.device_id, &request.device_name);
                            }
                            if ui.button("Deny").clicked() {
                                app.networking.deny_pending_peer(&request.device_id);
                            }
                            if ui.button("Block").clicked() {
                                app.block_network_peer(&request.device_id, &request.device_name);
                            }
                        });
                        ui.separator();
                    }
                });
                if pending_focus == Some(NetworkingFocusSection::PendingRequests) {
                    pending.response.scroll_to_me(Some(egui::Align::Center));
                }
            }

            ui.add_space(8.0);
            ui.group(|ui| {
                section_heading(
                    ui,
                    "[ME]",
                    egui::Color32::from_rgb(70, 110, 180),
                    "This device",
                );
                ui.label(format!("Name: {}", snapshot.device_name));
                ui.horizontal(|ui| {
                    ui.label("Device ID:");
                    ui.monospace(&snapshot.device_id);
                    if ui.button("Copy device ID").clicked() {
                        ui.ctx().copy_text(snapshot.device_id.clone());
                        app.networking_status = "Copied local device ID.".to_string();
                    }
                    if ui.button("Copy connection info").clicked() {
                        ui.ctx().copy_text(local_connection_info.clone());
                        app.networking_status = "Copied local connection info.".to_string();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Edit name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.networking_device_name_input)
                            .desired_width(260.0)
                            .hint_text("e.g. Office PC"),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Save name").clicked() {
                        let trimmed = app.networking_device_name_input.trim().to_string();
                        app.networking.set_device_name(&trimmed);
                        app.prefs.network_device_name = trimmed;
                        match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                            Ok(()) => {
                                app.prefs_status = "Saved networking device name.".to_string()
                            }
                            Err(e) => app.prefs_status = format!("Save failed: {e}"),
                        }
                        app.networking_device_name_input =
                            app.networking.snapshot().device_name.clone();
                    }
                    if ui.button("Reset default").clicked() {
                        app.networking.set_device_name("");
                        app.prefs.network_device_name.clear();
                        match preferences::save_prefs(&app.prefs_path, &app.prefs) {
                            Ok(()) => app.prefs_status =
                                "Reset networking device name.".to_string(),
                            Err(e) => app.prefs_status = format!("Save failed: {e}"),
                        }
                        app.networking_device_name_input =
                            app.networking.snapshot().device_name.clone();
                    }
                });
                ui.label(format!(
                    "Visibility: {}",
                    if snapshot.available_for_connectivity {
                        "Available on local network"
                    } else {
                        "Hidden / client only"
                    }
                ));
                if let Some(port) = snapshot.listener_port {
                    ui.label(format!("Host port: {port}"));
                }
                if !snapshot.local_presence.active_tab.trim().is_empty() {
                    ui.label(format!("Shared tab status: {}", snapshot.local_presence.active_tab));
                }
                if !snapshot.local_presence.runtime_status.trim().is_empty() {
                    ui.label(format!(
                        "Shared runtime status: {}",
                        snapshot.local_presence.runtime_status
                    ));
                }
                if !snapshot.status.is_empty() {
                    ui.label(format!("Status: {}", snapshot.status));
                }
                if !snapshot.protocol_notice.trim().is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(190, 110, 30),
                        format!("Compatibility note: {}", snapshot.protocol_notice),
                    );
                }
                if !snapshot.last_error.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(160, 32, 32),
                        format!("Last error: {}", snapshot.last_error),
                    );
                }
                if !app.networking_status.trim().is_empty() {
                    ui.small(app.networking_status.clone());
                }
                if !app.prefs_status.trim().is_empty() {
                    ui.small(app.prefs_status.clone());
                }
            });

            ui.add_space(12.0);
            let device_list_highlight = highlight_active(NetworkingFocusSection::DeviceList);
            let device_list = egui::Frame::group(ui.style())
                .fill(if device_list_highlight {
                    egui::Color32::from_rgb(244, 250, 244)
                } else {
                    ui.visuals().panel_fill
                })
                .stroke(if device_list_highlight {
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(70, 140, 90))
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                })
                .show(ui, |ui| {
                section_heading(
                    ui,
                    "[ACT]",
                    egui::Color32::from_rgb(70, 140, 90),
                    "Peer actions",
                );
                if device_list_highlight {
                    ui.small(
                        egui::RichText::new("Focused by Quick help")
                            .strong()
                            .color(egui::Color32::from_rgb(70, 140, 90)),
                    );
                }
                ui.horizontal_wrapped(|ui| {
                    let connected_keys = connected_visible
                        .iter()
                        .map(|peer| {
                            if peer.device_id.trim().is_empty() {
                                peer.connection_id.clone()
                            } else {
                                peer.device_id.clone()
                            }
                        })
                        .collect::<Vec<_>>();
                    let available_keys = available_visible
                        .iter()
                        .map(|peer| peer.device_id.clone())
                        .collect::<Vec<_>>();
                    let blocked_keys = blocked_visible
                        .iter()
                        .map(|peer| peer.device_id.clone())
                        .collect::<Vec<_>>();
                    if ui.button("Select All").clicked() {
                        app.networking_selected_devices = connected_keys
                            .iter()
                            .chain(available_keys.iter())
                            .chain(blocked_keys.iter())
                            .cloned()
                            .collect();
                    }
                    if ui.button("Deselect All").clicked() {
                        app.networking_selected_devices.clear();
                    }
                    if ui.button("Select Connected").clicked() {
                        app.networking_selected_devices =
                            connected_keys.iter().cloned().collect();
                    }
                    if ui.button("Select Available").clicked() {
                        app.networking_selected_devices =
                            available_keys.iter().cloned().collect();
                    }
                    ui.separator();
                    ui.label("Find:");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.networking_filter)
                            .desired_width(200.0)
                            .hint_text("find device"),
                    );
                });
                let selected_count = app.networking_selected_devices.len();
                ui.horizontal_wrapped(|ui| {
                    let selected_connections = connected_visible
                        .iter()
                        .filter(|peer| {
                            let key = if peer.device_id.trim().is_empty() {
                                peer.connection_id.clone()
                            } else {
                                peer.device_id.clone()
                            };
                            app.networking_selected_devices.contains(&key)
                        })
                        .collect::<Vec<_>>();
                    if ui
                        .add_enabled(selected_count > 0, egui::Button::new("Connect Selected"))
                        .clicked()
                    {
                        for peer in &available_visible {
                            if app.networking_selected_devices.contains(&peer.device_id) {
                                app.networking.connect_peer(&peer.device_id);
                            }
                        }
                    }
                    if ui
                        .add_enabled(
                            selected_count > 0,
                            egui::Button::new("Disconnect Selected"),
                        )
                        .clicked()
                    {
                        for peer in &selected_connections {
                            app.networking.disconnect_connection(&peer.connection_id);
                        }
                    }
                    if ui
                        .add_enabled(selected_count > 0, egui::Button::new("Block Selected"))
                        .clicked()
                    {
                        let mut blocked_count = 0usize;
                        for peer in &connected_visible {
                            let key = if peer.device_id.trim().is_empty() {
                                peer.connection_id.clone()
                            } else {
                                peer.device_id.clone()
                            };
                            if app.networking_selected_devices.contains(&key)
                                && !peer.device_id.trim().is_empty()
                            {
                                app.block_network_peer(&peer.device_id, &peer.device_name);
                                blocked_count += 1;
                            }
                        }
                        for peer in &available_visible {
                            if app.networking_selected_devices.contains(&peer.device_id) {
                                app.block_network_peer(&peer.device_id, &peer.device_name);
                                blocked_count += 1;
                            }
                        }
                        if blocked_count > 0 {
                            app.networking_status =
                                format!("Blocked {} selected device(s).", blocked_count);
                        }
                    }
                });
                ui.small(format!(
                    "Connected: {} | Available: {} | Blocked: {} | Selected: {}",
                    connected_visible.len(),
                    available_visible.len(),
                    blocked_visible.len(),
                    app.networking_selected_devices.len()
                ));
                ui.small(
                    "Tip: click a device name to rename it, and click the group chip to tag it for your own workflow.",
                );
            });
            if pending_focus == Some(NetworkingFocusSection::DeviceList) {
                device_list.response.scroll_to_me(Some(egui::Align::Center));
            }

            ui.add_space(8.0);
            let selected_connected_count = connected_visible
                .iter()
                .filter(|peer| {
                    let key = if peer.device_id.trim().is_empty() {
                        peer.connection_id.clone()
                    } else {
                        peer.device_id.clone()
                    };
                    app.networking_selected_devices.contains(&key)
                })
                .count();
            let selected_available_count = available_visible
                .iter()
                .filter(|peer| app.networking_selected_devices.contains(&peer.device_id))
                .count();
            let selected_blocked_count = blocked_visible
                .iter()
                .filter(|peer| app.networking_selected_devices.contains(&peer.device_id))
                .count();
            let render_selection_chip =
                |ui: &mut egui::Ui, label: &str, count: usize, tint: egui::Color32| {
                    let fill = if count > 0 {
                        blend_color(ui.visuals().panel_fill, tint, 0.18)
                    } else {
                        blend_color(
                            ui.visuals().panel_fill,
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                            0.06,
                        )
                    };
                    let stroke = if count > 0 {
                        blend_color(
                            ui.visuals().widgets.noninteractive.bg_stroke.color,
                            tint,
                            0.55,
                        )
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    };
                    egui::Frame::group(ui.style())
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.0, stroke))
                        .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.small(egui::RichText::new(label).strong());
                                ui.small(count.to_string());
                            });
                        });
                };
            ui.horizontal_wrapped(|ui| {
                render_selection_chip(
                    ui,
                    "Selected",
                    app.networking_selected_devices.len(),
                    egui::Color32::from_rgb(70, 110, 180),
                );
                render_selection_chip(
                    ui,
                    "Connected",
                    selected_connected_count,
                    egui::Color32::from_rgb(70, 110, 180),
                );
                render_selection_chip(
                    ui,
                    "Available",
                    selected_available_count,
                    egui::Color32::from_rgb(70, 140, 90),
                );
                render_selection_chip(
                    ui,
                    "Blocked",
                    selected_blocked_count,
                    egui::Color32::from_rgb(160, 60, 60),
                );
                if !app.networking_selected_devices.is_empty() {
                    if ui.small_button("Clear selection").clicked() {
                        app.networking_selected_devices.clear();
                    }
                    ui.small("Bulk actions apply to the checked devices.");
                }
            });
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                section_heading(
                    &mut cols[0],
                    "[AVL]",
                    egui::Color32::from_rgb(70, 140, 90),
                    &format!("Available ({})", available_visible.len()),
                );
                cols[0].label("Visible on the network but not currently connected.");
                cols[0].add_space(6.0);

                if available_visible.is_empty() {
                    cols[0].label("(none found yet)");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("network_discovered_scroll")
                        .max_height(360.0)
                        .show(&mut cols[0], |ui| {
                            for peer in &available_visible {
                                let key = peer.device_id.clone();
                                let selected_initial =
                                    app.networking_selected_devices.contains(&key);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(223, 241, 228),
                                    egui::Color32::from_rgb(70, 140, 90),
                                    selected_initial,
                                )
                                .show(ui, |ui| {
                                    let display_name =
                                        app.network_display_name(&peer.device_id, &peer.device_name);
                                    let can_persist_identity = !peer.device_id.trim().is_empty();
                                    let is_trusted = app.network_is_trusted(&peer.device_id);
                                    let alias_editing = app.networking_alias_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    let group_editing = app.networking_group_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());

                                    ui.horizontal_wrapped(|ui| {
                                        let mut selected =
                                            app.networking_selected_devices.contains(&key);
                                        if ui.checkbox(&mut selected, "").changed() {
                                            if selected {
                                                app.networking_selected_devices.insert(key.clone());
                                            } else {
                                                app.networking_selected_devices.remove(&key);
                                            }
                                        }
                                        if can_persist_identity {
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        } else {
                                            ui.strong(display_name.clone());
                                        }
                                        if is_trusted {
                                            ui.small(
                                                egui::RichText::new("Trusted")
                                                    .color(egui::Color32::from_rgb(110, 80, 170))
                                                    .strong(),
                                            );
                                        }
                                        ui.small(format!("{}:{}", peer.address, peer.host_port));
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            if ui.small_button(format!("Group: {group}")).clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        } else if can_persist_identity
                                            && ui.small_button("+ Group").clicked()
                                        {
                                            app.begin_network_group_edit(&peer.device_id);
                                        }
                                    });
                                    if alias_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Rename:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_alias_input,
                                                )
                                                .desired_width(220.0)
                                                .hint_text("Office PC West"),
                                            );
                                            if ui.button("Save").clicked() {
                                                app.save_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_alias_edit();
                                            }
                                        });
                                    }
                                    if group_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Group:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_group_input,
                                                )
                                                .desired_width(180.0)
                                                .hint_text("e.g. Research bench"),
                                            );
                                            if ui.button("Save group").clicked() {
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Clear").clicked() {
                                                app.networking_group_input.clear();
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_group_edit();
                                            }
                                        });
                                    }
                                    let mut status_parts =
                                        vec![format!("Seen {}s ago", peer.last_seen_secs_ago)];
                                    if is_trusted {
                                        status_parts.push("Trusted".to_string());
                                    }
                                    if let Some(group) = app.network_group_label(&peer.device_id) {
                                        status_parts.push(format!("Group: {group}"));
                                    }
                                    ui.small(status_parts.join(" | "));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Connect").clicked() {
                                            app.networking.connect_peer(&peer.device_id);
                                        }
                                        if can_persist_identity {
                                            if is_trusted {
                                                if ui.button("Untrust").clicked() {
                                                    app.untrust_network_peer(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                            } else if ui.button("Trust").clicked() {
                                                app.trust_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        }
                                        if ui.button("Block").clicked() {
                                            app.block_network_peer(&peer.device_id, &peer.device_name);
                                        }
                                        if ui.small_button("Copy ID").clicked() {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {}:{} | Seen: {}s ago",
                                                display_name,
                                                peer.device_id,
                                                peer.address,
                                                peer.host_port,
                                                peer.last_seen_secs_ago
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            if selected_initial { 1.9 } else { 1.35 },
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(70, 140, 90),
                                                if selected_initial { 0.78 } else { 0.60 },
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        });
                }

                section_heading(
                    &mut cols[1],
                    "[CON]",
                    egui::Color32::from_rgb(70, 110, 180),
                    &format!("Connected ({})", connected_visible.len()),
                );
                cols[1].label("Live TCP links between ChattyCog instances.");
                cols[1].add_space(6.0);

                if connected_visible.is_empty() {
                    cols[1].label("(no active connections)");
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("network_connected_scroll")
                        .max_height(360.0)
                        .show(&mut cols[1], |ui| {
                            for peer in &connected_visible {
                                let key = if peer.device_id.trim().is_empty() {
                                    peer.connection_id.clone()
                                } else {
                                    peer.device_id.clone()
                                };
                                let selected_initial =
                                    app.networking_selected_devices.contains(&key);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(224, 234, 250),
                                    egui::Color32::from_rgb(70, 110, 180),
                                    selected_initial,
                                )
                                .show(ui, |ui| {
                                    let is_trusted = app.network_is_trusted(&peer.device_id);
                                    ui.horizontal_wrapped(|ui| {
                                        let mut selected =
                                            app.networking_selected_devices.contains(&key);
                                        if ui.checkbox(&mut selected, "").changed() {
                                            if selected {
                                                app.networking_selected_devices.insert(key.clone());
                                            } else {
                                                app.networking_selected_devices.remove(&key);
                                            }
                                        }
                                        let display_name = app
                                            .network_display_name(&peer.device_id, &peer.device_name);
                                        let can_persist_identity = !peer.device_id.trim().is_empty();
                                        if can_persist_identity {
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        } else {
                                            ui.strong(display_name.clone());
                                        }
                                        if is_trusted {
                                            ui.small(
                                                egui::RichText::new("Trusted")
                                                    .color(egui::Color32::from_rgb(110, 80, 170))
                                                    .strong(),
                                            );
                                        }
                                        ui.small(if peer.inbound { "Inbound" } else { "Outbound" });
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            if ui.small_button(format!("Group: {group}")).clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        } else if can_persist_identity
                                            && ui.small_button("+ Group").clicked()
                                        {
                                            app.begin_network_group_edit(&peer.device_id);
                                        }
                                    });
                                    let display_name =
                                        app.network_display_name(&peer.device_id, &peer.device_name);
                                    let alias_editing = app.networking_alias_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    let group_editing = app.networking_group_edit_device.as_deref()
                                        == Some(peer.device_id.as_str());
                                    if alias_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Rename:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_alias_input,
                                                )
                                                .desired_width(220.0)
                                                .hint_text("Chatty Station 2"),
                                            );
                                            if ui.button("Save").clicked() {
                                                app.save_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_alias_edit();
                                            }
                                        });
                                    }
                                    if group_editing {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Group:");
                                            ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut app.networking_group_input,
                                                )
                                                .desired_width(180.0)
                                                .hint_text("e.g. Writers room"),
                                            );
                                            if ui.button("Save group").clicked() {
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Clear").clicked() {
                                                app.networking_group_input.clear();
                                                app.save_network_group_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.button("Cancel").clicked() {
                                                app.cancel_network_group_edit();
                                            }
                                        });
                                    }
                                    ui.label(format!("Address: {}", peer.address));
                                    let mut status_parts = vec![peer.status_summary.clone()];
                                    if is_trusted {
                                        status_parts.push("Trusted".to_string());
                                    }
                                    if let Some(group) = app.network_group_label(&peer.device_id) {
                                        status_parts.push(format!("Group: {group}"));
                                    }
                                    ui.label(format!("Shared status: {}", status_parts.join(" | ")));
                                    if let Some(age) = peer.status_age_secs {
                                        ui.small(format!("Status updated {}s ago", age));
                                    }
                                    ui.small(format!("Connected for {}s", peer.connected_secs));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Disconnect").clicked() {
                                            app.networking.disconnect_connection(&peer.connection_id);
                                        }
                                        if !peer.device_id.trim().is_empty() {
                                            if is_trusted {
                                                if ui.button("Untrust").clicked() {
                                                    app.untrust_network_peer(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                            } else if ui.button("Trust").clicked() {
                                                app.trust_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                        }
                                        if ui.button("Block").clicked()
                                            && !peer.device_id.trim().is_empty()
                                        {
                                            app.block_network_peer(&peer.device_id, &peer.device_name);
                                        }
                                        if !peer.device_id.trim().is_empty()
                                            && ui.small_button("Copy ID").clicked()
                                        {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {} | Direction: {} | Connected: {}s",
                                                display_name,
                                                peer.device_id,
                                                peer.address,
                                                if peer.inbound {
                                                    "inbound"
                                                } else {
                                                    "outbound"
                                                },
                                                peer.connected_secs
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            if selected_initial { 1.9 } else { 1.35 },
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(70, 110, 180),
                                                if selected_initial { 0.78 } else { 0.60 },
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        });
                }
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("[TRU] Trusted ({})", trusted_visible.len()))
                    .color(egui::Color32::from_rgb(110, 80, 170))
                    .strong(),
            )
            .id_salt("network_trusted_section")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("network_trusted_scroll")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if trusted_visible.is_empty() {
                            ui.label("(none)");
                        } else {
                            for peer in &trusted_visible {
                                let display_name =
                                    app.network_display_name(&peer.device_id, &peer.device_name);
                                let card = network_card_frame(
                                    ui,
                                    egui::Color32::from_rgb(235, 230, 247),
                                    egui::Color32::from_rgb(110, 80, 170),
                                    false,
                                )
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(display_name.clone()).strong(),
                                        );
                                        ui.small(
                                            egui::RichText::new("Trusted")
                                                .color(egui::Color32::from_rgb(110, 80, 170))
                                                .strong(),
                                        );
                                        if let Some(group) =
                                            app.network_group_label(&peer.device_id)
                                        {
                                            ui.small(format!("Group: {group}"));
                                        }
                                    });
                                    let mut detail_parts = Vec::new();
                                    if !peer.address.trim().is_empty() {
                                        detail_parts.push(format!("Address: {}", peer.address));
                                    }
                                    if let Some(age) = peer.last_seen_secs_ago {
                                        detail_parts.push(format!("Last seen {}s ago", age));
                                    } else {
                                        detail_parts.push("Not seen recently".to_string());
                                    }
                                    ui.small(detail_parts.join(" | "));
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.button("Untrust").clicked() {
                                            app.untrust_network_peer(
                                                &peer.device_id,
                                                &peer.device_name,
                                            );
                                        }
                                        if ui.small_button("Copy ID").clicked() {
                                            ui.ctx().copy_text(peer.device_id.clone());
                                            app.networking_status = format!(
                                                "Copied device ID for {}.",
                                                display_name
                                            );
                                        }
                                        if ui.small_button("Copy info").clicked() {
                                            ui.ctx().copy_text(format!(
                                                "Name: {} | Device ID: {} | Address: {} | State: trusted",
                                                display_name, peer.device_id, peer.address
                                            ));
                                            app.networking_status = format!(
                                                "Copied connection info for {}.",
                                                display_name
                                            );
                                        }
                                    });
                                });
                                if card.response.hovered() {
                                    ui.painter().rect_stroke(
                                        card.response.rect.expand(1.0),
                                        6.0,
                                        egui::Stroke::new(
                                            1.35,
                                            blend_color(
                                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                                                egui::Color32::from_rgb(110, 80, 170),
                                                0.60,
                                            ),
                                        ),
                                    );
                                }
                                ui.add_space(6.0);
                            }
                        }
                    });
            });

            ui.add_space(8.0);
            egui::CollapsingHeader::new(
                egui::RichText::new(format!("[BLK] Blocked ({})", blocked_visible.len()))
                    .color(egui::Color32::from_rgb(160, 60, 60))
                    .strong(),
            )
                .id_salt("network_blocked_section")
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("network_blocked_scroll")
                        .max_height(220.0)
                        .show(ui, |ui| {
                            if blocked_visible.is_empty() {
                                ui.label("(none)");
                            } else {
                                for peer in &blocked_visible {
                                    let key = peer.device_id.clone();
                                    let selected_initial =
                                        app.networking_selected_devices.contains(&key);
                                    let card = network_card_frame(
                                        ui,
                                        egui::Color32::from_rgb(248, 228, 228),
                                        egui::Color32::from_rgb(160, 60, 60),
                                        selected_initial,
                                    )
                                    .show(ui, |ui| {
                                        let display_name =
                                            app.network_display_name(&peer.device_id, &peer.device_name);
                                        let alias_editing = app.networking_alias_edit_device.as_deref()
                                            == Some(peer.device_id.as_str());
                                        let group_editing = app.networking_group_edit_device.as_deref()
                                            == Some(peer.device_id.as_str());

                                        ui.horizontal_wrapped(|ui| {
                                            let mut selected =
                                                app.networking_selected_devices.contains(&key);
                                            if ui.checkbox(&mut selected, "").changed() {
                                                if selected {
                                                    app.networking_selected_devices.insert(key.clone());
                                                } else {
                                                    app.networking_selected_devices.remove(&key);
                                                }
                                            }
                                            if ui
                                                .link(
                                                    egui::RichText::new(display_name.clone())
                                                        .strong(),
                                                )
                                                .clicked()
                                            {
                                                app.begin_network_alias_edit(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if let Some(group) =
                                                app.network_group_label(&peer.device_id)
                                            {
                                                if ui.small_button(format!("Group: {group}")).clicked() {
                                                    app.begin_network_group_edit(&peer.device_id);
                                                }
                                            } else if ui.small_button("+ Group").clicked() {
                                                app.begin_network_group_edit(&peer.device_id);
                                            }
                                        });
                                        if alias_editing {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label("Rename:");
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut app.networking_alias_input,
                                                    )
                                                    .desired_width(220.0)
                                                    .hint_text("Archive laptop"),
                                                );
                                                if ui.button("Save").clicked() {
                                                    app.save_network_alias_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Cancel").clicked() {
                                                    app.cancel_network_alias_edit();
                                                }
                                            });
                                        }
                                        if group_editing {
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label("Group:");
                                                ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut app.networking_group_input,
                                                    )
                                                    .desired_width(180.0)
                                                    .hint_text("e.g. Spare pool"),
                                                );
                                                if ui.button("Save group").clicked() {
                                                    app.save_network_group_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Clear").clicked() {
                                                    app.networking_group_input.clear();
                                                    app.save_network_group_edit(
                                                        &peer.device_id,
                                                        &peer.device_name,
                                                    );
                                                }
                                                if ui.button("Cancel").clicked() {
                                                    app.cancel_network_group_edit();
                                                }
                                            });
                                        }
                                        if let Some(age) = peer.last_seen_secs_ago {
                                            if let Some(group) = app.network_group_label(&peer.device_id) {
                                                ui.small(format!("Blocked | Group: {} | Seen {}s ago", group, age));
                                            } else {
                                                ui.small(format!("Blocked | Seen {}s ago", age));
                                            }
                                        } else {
                                            if let Some(group) = app.network_group_label(&peer.device_id) {
                                                ui.small(format!("Blocked | Group: {group}"));
                                            } else {
                                                ui.small("Blocked");
                                            }
                                        }
                                        if !peer.address.trim().is_empty() {
                                            ui.small(format!("Address: {}", peer.address));
                                        }
                                        ui.horizontal_wrapped(|ui| {
                                            if ui.button("Unblock").clicked() {
                                                app.unblock_network_peer(
                                                    &peer.device_id,
                                                    &peer.device_name,
                                                );
                                            }
                                            if ui.small_button("Copy ID").clicked() {
                                                ui.ctx().copy_text(peer.device_id.clone());
                                                app.networking_status =
                                                    format!("Copied device ID for {}.", display_name);
                                            }
                                            if ui.small_button("Copy info").clicked() {
                                                ui.ctx().copy_text(format!(
                                                    "Name: {} | Device ID: {} | Address: {} | State: blocked",
                                                    display_name, peer.device_id, peer.address
                                                ));
                                                app.networking_status = format!(
                                                    "Copied connection info for {}.",
                                                    display_name
                                                );
                                            }
                                        });
                                    });
                                    if card.response.hovered() {
                                        ui.painter().rect_stroke(
                                            card.response.rect.expand(1.0),
                                            6.0,
                                            egui::Stroke::new(
                                                if selected_initial { 1.9 } else { 1.35 },
                                                blend_color(
                                                    ui.visuals()
                                                        .widgets
                                                        .noninteractive
                                                        .bg_stroke
                                                        .color,
                                                    egui::Color32::from_rgb(160, 60, 60),
                                                    if selected_initial { 0.78 } else { 0.60 },
                                                ),
                                            ),
                                        );
                                    }
                                    ui.add_space(6.0);
                                }
                            }
                        });
                });

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[BND]",
                egui::Color32::from_rgb(80, 120, 170),
                "Workflow bundle",
            );
            ui.label(
                "Capture the current ChattyCog setup into a portable bundle: system prompt, model hints, sampling settings, sandbox policy, and per-module AI preferences.",
            );
            let selected_connections = app.selected_network_connection_ids();
            ui.add(
                egui::TextEdit::singleline(&mut app.networking_bundle_label)
                    .hint_text("Bundle title..."),
            );
            ui.add(
                egui::TextEdit::multiline(&mut app.networking_bundle_summary)
                    .desired_rows(3)
                    .hint_text("What is this setup for?"),
            );
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Selected connected peers: {}",
                    selected_connections.len()
                ));
                ui.small(format!(
                    "Module prefs included: {}",
                    app.prefs.modules.len()
                ));
                ui.small(format!(
                    "System prompt: {} chars",
                    app.current_system_prompt().chars().count()
                ));
            });
            if selected_connections.is_empty() {
                ui.small(
                    "Select one or more connected peers above before sending a workflow bundle.",
                );
            } else if ui.button("Send current setup to selected peers").clicked() {
                let bundle = app.build_current_workflow_bundle();
                let summary = if bundle.summary.trim().is_empty() {
                    format!(
                        "ChattyCog setup with {} module preference(s)",
                        bundle.module_preferences.len()
                    )
                } else {
                    bundle.summary.trim().to_string()
                };
                match serde_json::to_string_pretty(&bundle) {
                    Ok(text) => {
                        let label = if bundle.label.trim().is_empty() {
                            "ChattyCog setup".to_string()
                        } else {
                            bundle.label.trim().to_string()
                        };
                        let file_name = format!(
                            "workflow_bundle_{}.json",
                            slugify_filename(&label, "workflow_bundle")
                        );
                        for connection_id in &selected_connections {
                            app.networking.send_artifact(
                                connection_id,
                                "workflow_bundle_json",
                                &label,
                                None,
                                &summary,
                                &file_name,
                                &text,
                            );
                        }
                        app.networking_status = format!(
                            "Networking: sent workflow bundle to {} selected peer(s).",
                            selected_connections.len()
                        );
                    }
                    Err(err) => {
                        app.networking_status =
                            format!("Networking: could not serialize workflow bundle: {err}");
                    }
                }
            }

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[MEM]",
                egui::Color32::from_rgb(130, 90, 170),
                "Shared luke warm memory",
            );
            ui.label(
                "Share summary-only recent context with selected peers. Hot memory stays local, and cold logs are not transferred.",
            );
            let local_lukewarm_share = app.build_current_lukewarm_share();
            let applied_lukewarm_count =
                load_applied_lukewarm_contexts(&app.applied_lukewarm_dir()).unwrap_or_default().len();
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Selected connected peers: {}",
                    selected_connections.len()
                ));
                ui.small(format!("Applied peer summaries: {}", applied_lukewarm_count));
                ui.small(if app.prefs.network_allow_shared_lukewarm_context {
                    "Shared luke warm is allowed in prompts"
                } else {
                    "Shared luke warm is stored but not injected into prompts"
                });
            });
            let mut local_lukewarm_preview = if local_lukewarm_share.context_text.trim().is_empty() {
                "(No local luke warm context is ready yet.)".to_string()
            } else {
                local_lukewarm_share.context_text.clone()
            };
            egui::ScrollArea::vertical()
                .id_salt("network_lukewarm_share_preview")
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut local_lukewarm_preview)
                            .desired_rows(6)
                            .interactive(false),
                    );
                });
            if selected_connections.is_empty() {
                ui.small(
                    "Select one or more connected peers above before sharing luke warm context.",
                );
            } else if ui
                .add_enabled(
                    !local_lukewarm_share.context_text.trim().is_empty(),
                    egui::Button::new("Share current luke warm to selected peers"),
                )
                .clicked()
            {
                match serde_json::to_string_pretty(&local_lukewarm_share) {
                    Ok(text) => {
                        let file_name = format!(
                            "lukewarm_context_{}.json",
                            slugify_filename(&local_lukewarm_share.label, "lukewarm_context")
                        );
                        for connection_id in &selected_connections {
                            app.networking.send_artifact(
                                connection_id,
                                "lukewarm_context_json",
                                &local_lukewarm_share.label,
                                None,
                                &local_lukewarm_share.summary,
                                &file_name,
                                &text,
                            );
                        }
                        app.networking_status = format!(
                            "Networking: shared luke warm context to {} selected peer(s).",
                            selected_connections.len()
                        );
                    }
                    Err(err) => {
                        app.networking_status =
                            format!("Networking: could not serialize luke warm context: {err}");
                    }
                }
            }
            app.render_received_lukewarm_inbox(ui, "Received luke warm context");

            ui.add_space(12.0);
            ui.separator();
            let shared_room_highlight = highlight_active(NetworkingFocusSection::SharedRoom);
            let shared_room = egui::Frame::group(ui.style())
                .fill(if shared_room_highlight {
                    egui::Color32::from_rgb(245, 246, 255)
                } else {
                    ui.visuals().panel_fill
                })
                .stroke(if shared_room_highlight {
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 90, 170))
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                })
                .show(ui, |ui| {
                    section_heading(
                        ui,
                        "[ROOM]",
                        egui::Color32::from_rgb(120, 90, 170),
                        "Shared room chat",
                    );
                    if shared_room_highlight {
                        ui.small(
                            egui::RichText::new("Focused by Quick help")
                                .strong()
                                .color(egui::Color32::from_rgb(120, 90, 170)),
                        );
                    }
                    ui.label(
                        "Use this when multiple ChattyCog instances should share one turn-aware room. Main chat can mirror into this room, while hot memory stays local and only luke warm summaries move across the network.",
                    );

                    let capable_modules = app.shared_chat_capable_modules();
                    let mut next_turn_mode = app.networking_shared_chat_policy.turn_mode;
                    let mut next_ai_mode = app.networking_shared_chat_policy.ai_mode;
                    let mut scope_selection = if app.networking_shared_chat_policy.scope_kind
                        == SharedChatScopeKind::Module
                    {
                        app.networking_shared_chat_policy.scope_module_id.clone()
                    } else {
                        "__general__".to_string()
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.label("Scope:");
                        egui::ComboBox::from_id_salt("shared_room_scope")
                            .selected_text(app.shared_chat_scope_label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut scope_selection,
                                    "__general__".to_string(),
                                    "General room",
                                );
                                for (module_id, module_name, multiplayer) in &capable_modules {
                                    let label = if *multiplayer {
                                        format!("{module_name} (multiplayer)")
                                    } else {
                                        module_name.clone()
                                    };
                                    ui.selectable_value(
                                        &mut scope_selection,
                                        module_id.clone(),
                                        label,
                                    );
                                }
                            });
                        ui.separator();
                        ui.label("Turn mode:");
                        egui::ComboBox::from_id_salt("shared_room_turn_mode")
                            .selected_text(next_turn_mode.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut next_turn_mode,
                                    SharedChatTurnMode::Open,
                                    "Open",
                                );
                                ui.selectable_value(
                                    &mut next_turn_mode,
                                    SharedChatTurnMode::TalkingStick,
                                    "Talking stick",
                                );
                            });
                        ui.separator();
                        ui.label("AI mode:");
                        egui::ComboBox::from_id_salt("shared_room_ai_mode")
                            .selected_text(next_ai_mode.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut next_ai_mode,
                                    SharedChatAiMode::Off,
                                    "Off",
                                );
                                ui.selectable_value(
                                    &mut next_ai_mode,
                                    SharedChatAiMode::LocalAllowed,
                                    "Local allowed",
                                );
                                ui.selectable_value(
                                    &mut next_ai_mode,
                                    SharedChatAiMode::HostOnly,
                                    "Host only",
                                );
                            });
                    });
                    let current_scope_selection = if app.networking_shared_chat_policy.scope_kind
                        == SharedChatScopeKind::Module
                    {
                        app.networking_shared_chat_policy.scope_module_id.clone()
                    } else {
                        "__general__".to_string()
                    };
                    if scope_selection != current_scope_selection {
                        if scope_selection == "__general__" {
                            app.set_shared_chat_scope_general();
                        } else if let Some((_, module_name, multiplayer)) = capable_modules
                            .iter()
                            .find(|(module_id, _, _)| module_id == &scope_selection)
                        {
                            app.set_shared_chat_scope_module(
                                scope_selection.clone(),
                                module_name.clone(),
                                *multiplayer,
                            );
                        }
                        app.broadcast_shared_chat_policy("Room scope changed.");
                    }
                    if next_turn_mode != app.networking_shared_chat_policy.turn_mode {
                        app.networking_shared_chat_policy.turn_mode = next_turn_mode;
                        if next_turn_mode == SharedChatTurnMode::Open {
                            app.networking_shared_chat_policy.turn_holder_device_id.clear();
                            app.networking_shared_chat_policy.turn_holder_device_name.clear();
                        }
                        app.broadcast_shared_chat_policy("Turn mode changed.");
                    }
                    if next_ai_mode != app.networking_shared_chat_policy.ai_mode {
                        app.networking_shared_chat_policy.ai_mode = next_ai_mode;
                        app.broadcast_shared_chat_policy("AI mode changed.");
                    }

                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(
                            &mut app.networking_shared_chat_mirror_main_chat,
                            "Mirror local main-chat messages into this room",
                        );
                        ui.small(format!(
                            "Host: {}",
                            if app
                                .networking_shared_chat_policy
                                .host_device_name
                                .trim()
                                .is_empty()
                            {
                                "(not set)"
                            } else {
                                app.networking_shared_chat_policy.host_device_name.trim()
                            }
                        ));
                        ui.small(format!(
                            "Turn holder: {}",
                            app.shared_chat_turn_holder_label()
                        ));
                        ui.small(format!(
                            "Connected peers in room: {}",
                            shared_room_connection_ids.len()
                        ));
                        if app.networking_shared_chat_policy.scope_kind
                            == SharedChatScopeKind::Module
                        {
                            ui.small(format!(
                                "Scoped to module: {}",
                                app.shared_chat_scope_label()
                            ));
                        }
                        if let Some(session_summary) = app.shared_chat_session_summary() {
                            ui.small(format!("Session: {session_summary}"));
                        }
                    });

                    if !app.networking_shared_chat_policy.session_active {
                        if let Some(recoverable) =
                            app.networking_recoverable_shared_chat_policy.clone()
                        {
                            ui.group(|ui| {
                                ui.strong("Recovered host session available");
                                ui.small(format!(
                                    "{} | scope {} | revision {}",
                                    if recoverable.session_label.trim().is_empty() {
                                        recoverable.session_id.trim()
                                    } else {
                                        recoverable.session_label.trim()
                                    },
                                    if recoverable.scope_kind == SharedChatScopeKind::Module
                                        && !recoverable.scope_module_name.trim().is_empty()
                                    {
                                        recoverable.scope_module_name.trim()
                                    } else {
                                        recoverable.label.trim()
                                    },
                                    recoverable.session_revision.max(1)
                                ));
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("Resume saved session").clicked() {
                                        if let Err(err) = app.resume_recoverable_shared_chat_policy()
                                        {
                                            app.networking_status =
                                                format!("Networking: {err}");
                                        }
                                    }
                                    if ui.button("Discard recovery").clicked() {
                                        app.discard_recoverable_shared_chat_policy();
                                        app.networking_status = "Networking: discarded the saved host-session recovery snapshot.".to_string();
                                    }
                                });
                            });
                        }
                    } else if app.shared_chat_is_local_host()
                        && app.networking_recoverable_shared_chat_policy.is_some()
                    {
                        ui.small(
                            "Recovery snapshot armed: if this host restarts, you can resume this session cleanly.",
                        );
                    }

                    if let Some(recovery) = app.networking_recoverable_module_session.clone() {
                        ui.group(|ui| {
                            ui.strong("Recoverable module session state");
                            ui.small(format!(
                                "{} | latest shared state: {} | cached assets: {}",
                                if recovery.scope_module_name.trim().is_empty() {
                                    recovery.scope_module_id.trim()
                                } else {
                                    recovery.scope_module_name.trim()
                                },
                                recovery
                                    .latest_shared_state
                                    .as_ref()
                                    .map(|state| format!("revision {}", state.session_revision.max(1)))
                                    .unwrap_or_else(|| "none yet".to_string()),
                                recovery.recent_assets.len()
                            ));
                            ui.small(
                                "Use this after a restart or host handoff to restore the module bridge locally, then re-share the last good session state or cached assets to selected peers (or everyone in the room if nothing is selected).",
                            );
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Restore state to bridge").clicked() {
                                    if let Err(err) =
                                        app.restore_recoverable_module_shared_state_to_bridge()
                                    {
                                        app.networking_status = format!("Networking: {err}");
                                    }
                                }
                                if ui.button("Re-share latest state").clicked() {
                                    if let Err(err) = app.replay_recoverable_module_shared_state() {
                                        app.networking_status = format!("Networking: {err}");
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        !recovery.recent_assets.is_empty(),
                                        egui::Button::new("Replay cached assets"),
                                    )
                                    .clicked()
                                {
                                    if let Err(err) = app.replay_recoverable_module_assets() {
                                        app.networking_status = format!("Networking: {err}");
                                    }
                                }
                                if ui.button("Open recovery folder").clicked() {
                                    open_path_in_explorer(&app.network_recovery_dir());
                                }
                            });
                        });
                    }

                    if app.networking_shared_chat_policy.session_active
                        && app.shared_chat_host_appears_offline()
                    {
                        ui.group(|ui| {
                            ui.colored_label(
                                egui::Color32::from_rgb(180, 110, 70),
                                "Current room host appears offline.",
                            );
                            ui.small(
                                "You can wait for the host to return, or take over and rebroadcast this room session from here.",
                            );
                            if ui.button("Take over as host").clicked() {
                                if let Err(err) = app.take_over_shared_chat_host() {
                                    app.networking_status = format!("Networking: {err}");
                                }
                            }
                        });
                    }

                    let selected_connected_peers = snapshot
                        .connected_peers
                        .iter()
                        .filter(|peer| {
                            let key = if peer.device_id.trim().is_empty() {
                                peer.connection_id.clone()
                            } else {
                                peer.device_id.clone()
                            };
                            app.networking_selected_devices.contains(&key)
                        })
                        .collect::<Vec<_>>();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Broadcast current room policy").clicked() {
                            app.broadcast_shared_chat_policy("Manual policy refresh.");
                        }
                        if app.networking_shared_chat_policy.scope_kind
                            == SharedChatScopeKind::Module
                        {
                            if !app.networking_shared_chat_policy.session_active {
                                if ui.button("Start module session").clicked() {
                                    if let Some(module_name) = app.begin_shared_chat_module_session()
                                    {
                                        app.broadcast_shared_chat_policy(&format!(
                                            "Started host-guided module session for {module_name}."
                                        ));
                                    }
                                }
                            } else if ui.button("End module session").clicked() {
                                let label = app
                                    .networking_shared_chat_policy
                                    .session_label
                                    .trim()
                                    .to_string();
                                app.end_shared_chat_module_session();
                                app.broadcast_shared_chat_policy(&format!(
                                    "Ended {}.",
                                    if label.is_empty() {
                                        "the module session".to_string()
                                    } else {
                                        label
                                    }
                                ));
                            }
                        }
                        if ui.button("Take stick").clicked() {
                            let local = app.networking.snapshot().clone();
                            app.networking_shared_chat_policy.turn_mode =
                                SharedChatTurnMode::TalkingStick;
                            app.networking_shared_chat_policy.turn_holder_device_id =
                                local.device_id;
                            app.networking_shared_chat_policy.turn_holder_device_name =
                                local.device_name;
                            app.broadcast_shared_chat_policy("Turn stick taken locally.");
                        }
                        if ui
                            .add_enabled(
                                app.shared_chat_is_local_host() && selected_connected_peers.len() == 1,
                                egui::Button::new("Hand off host to selected peer"),
                            )
                            .clicked()
                        {
                            if let Some(peer) = selected_connected_peers.first() {
                                if let Err(err) = app.handoff_shared_chat_host_to_peer(
                                    &peer.device_id,
                                    &app.network_display_name(&peer.device_id, &peer.device_name),
                                ) {
                                    app.networking_status = format!("Networking: {err}");
                                }
                            }
                        }
                        if ui
                            .add_enabled(
                                selected_connected_peers.len() == 1,
                                egui::Button::new("Pass stick to selected peer"),
                            )
                            .clicked()
                        {
                            if let Some(peer) = selected_connected_peers.first() {
                                app.networking_shared_chat_policy.turn_mode =
                                    SharedChatTurnMode::TalkingStick;
                                app.networking_shared_chat_policy.turn_holder_device_id =
                                    peer.device_id.clone();
                                app.networking_shared_chat_policy.turn_holder_device_name =
                                    app.network_display_name(&peer.device_id, &peer.device_name);
                                app.broadcast_shared_chat_policy("Turn stick reassigned.");
                            }
                        }
                        if ui.button("Open room flow").clicked() {
                            app.networking_shared_chat_policy.turn_mode = SharedChatTurnMode::Open;
                            app.networking_shared_chat_policy.turn_holder_device_id.clear();
                            app.networking_shared_chat_policy.turn_holder_device_name.clear();
                            app.broadcast_shared_chat_policy("Talking stick cleared.");
                        }
                    });

                    let room_hint = if shared_room_connection_ids.is_empty() {
                        "Connect to one or more peers to turn the shared room into a live conversation lane."
                            .to_string()
                    } else {
                        match app.shared_chat_can_send_user_message() {
                            Ok(()) => "You can type here to send a room message, or mirror your normal Chat tab into this room.".to_string(),
                            Err(reason) => reason,
                        }
                    };
                    ui.small(room_hint);

                    egui::ScrollArea::vertical()
                        .id_salt("shared_room_log")
                        .max_height(200.0)
                        .show(ui, |ui| {
                            if app.networking_shared_chat_log.is_empty() {
                                ui.label("(no shared room activity yet)");
                            } else {
                                for entry in app.networking_shared_chat_log.iter().rev().take(48).rev() {
                                    ui.group(|ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            let tag_color = match entry.speaker_kind.as_str() {
                                                "assistant" => egui::Color32::from_rgb(50, 140, 90),
                                                "system" => egui::Color32::from_rgb(120, 90, 170),
                                                _ => egui::Color32::from_rgb(70, 110, 180),
                                            };
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "[{}]",
                                                    entry.speaker_kind.to_uppercase()
                                                ))
                                                .color(tag_color)
                                                .strong(),
                                            );
                                            ui.strong(if entry.speaker_label.trim().is_empty() {
                                                entry.from_device_name.trim()
                                            } else {
                                                entry.speaker_label.trim()
                                            });
                                            ui.small(format!(
                                                "{} | {}",
                                                entry.from_device_name,
                                                entry.sent_at_unix_ms
                                            ));
                                            if entry.scope_kind == SharedChatScopeKind::Module {
                                                let scope_name =
                                                    if entry.scope_module_name.trim().is_empty() {
                                                        entry.scope_module_id.trim()
                                                    } else {
                                                        entry.scope_module_name.trim()
                                                    };
                                                ui.small(format!("scope: {scope_name}"));
                                            }
                                        });
                                        ui.label(entry.body.trim());
                                    });
                                    ui.add_space(4.0);
                                }
                            }
                        });

                    ui.horizontal(|ui| {
                        let input = ui.add(
                            egui::TextEdit::singleline(&mut app.networking_shared_chat_input)
                                .desired_width(ui.available_width() - 120.0)
                                .hint_text("Shared room message..."),
                        );
                        let send_enabled = !app.networking_shared_chat_input.trim().is_empty()
                            && !shared_room_connection_ids.is_empty()
                            && app.shared_chat_can_send_user_message().is_ok();
                        if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            if send_enabled {
                                let body = app.networking_shared_chat_input.trim().to_string();
                                app.networking_shared_chat_input.clear();
                                app.broadcast_shared_chat_message("user", "You", &body);
                            }
                        }
                        if ui
                            .add_enabled(send_enabled, egui::Button::new("Send to room"))
                            .clicked()
                        {
                            let body = app.networking_shared_chat_input.trim().to_string();
                            app.networking_shared_chat_input.clear();
                            app.broadcast_shared_chat_message("user", "You", &body);
                        }
                    });
                });
            if pending_focus == Some(NetworkingFocusSection::SharedRoom) {
                shared_room.response.scroll_to_me(Some(egui::Align::Center));
            }

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[EVT]",
                egui::Color32::from_rgb(170, 110, 70),
                "Recent session events",
            );
            ui.label(
                "Low-latency room events are meant for lightweight module signals like turns, small moves, ready states, or other game/program session nudges.",
            );
            ui.horizontal_wrapped(|ui| {
                ui.small(format!(
                    "Recent events cached: {}",
                    snapshot.received_session_events.len()
                ));
                if !snapshot.received_session_events.is_empty()
                    && ui.button("Clear recent events").clicked()
                {
                    app.networking.clear_received_session_events();
                }
            });
            if snapshot.received_session_events.is_empty() {
                ui.label("(no recent session events yet)");
            } else {
                for event in snapshot.received_session_events.iter().rev().take(24) {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(if event.label.trim().is_empty() {
                                event.event_type.trim()
                            } else {
                                event.label.trim()
                            });
                            ui.small(format!(
                                "{} | {}s ago",
                                if event.from_device_name.trim().is_empty() {
                                    "(unknown sender)"
                                } else {
                                    event.from_device_name.trim()
                                },
                                event.received_secs_ago
                            ));
                                if !event.scope_module_id.trim().is_empty() {
                                    ui.small(format!("module: {}", event.scope_module_id.trim()));
                                }
                                if !event.session_id.trim().is_empty() {
                                    ui.small(format!("session: {}", event.session_id.trim()));
                                }
                                if !event.from_address.trim().is_empty() {
                                    ui.small(format!("addr: {}", event.from_address.trim()));
                                }
                                if !event.content_type.trim().is_empty() {
                                    ui.small(event.content_type.trim());
                                }
                            });
                        if !event.payload_text.trim().is_empty() {
                            ui.label(event.payload_text.trim());
                        } else {
                            ui.small("(no text payload)");
                        }
                    });
                    ui.add_space(4.0);
                }
            }

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[ACK]",
                egui::Color32::from_rgb(80, 120, 170),
                "Recent delivery status",
            );
            if delivery_visible.is_empty() {
                ui.label("(no recent outgoing transfers yet)");
            } else {
                for artifact in &delivery_visible {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(if artifact.label.trim().is_empty() {
                                artifact.kind.trim()
                            } else {
                                artifact.label.trim()
                            });
                            ui.small(format!(
                                "{} | {} attempt(s) | {}s ago",
                                artifact.status.trim(),
                                artifact.attempts,
                                artifact.updated_secs_ago
                            ));
                            ui.small(if artifact.waiting_for_ack {
                                "Awaiting ack"
                            } else {
                                "Closed loop"
                            });
                        });
                        ui.monospace(&artifact.artifact_id);
                        if !artifact.to_device_name.trim().is_empty() {
                            ui.small(format!("To: {}", artifact.to_device_name));
                        }
                        if !artifact.to_device_id.trim().is_empty() {
                            ui.monospace(&artifact.to_device_id);
                        }
                        if !artifact.to_address.trim().is_empty() {
                            ui.small(format!("Address: {}", artifact.to_address));
                        }
                        if !artifact.module_id.trim().is_empty() {
                            ui.small(format!("Module: {}", artifact.module_id));
                        }
                        if !artifact.file_name.trim().is_empty() {
                            ui.small(format!("File: {}", artifact.file_name));
                        }
                        ui.small(format_network_transfer_meta(
                            &artifact.content_type,
                            &artifact.transfer_encoding,
                            artifact.byte_len,
                            artifact.chunk_count,
                        ));
                        if !artifact.summary.trim().is_empty() {
                            ui.label(artifact.summary.trim());
                        }
                    });
                    ui.add_space(6.0);
                }
            }

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[OUT]",
                egui::Color32::from_rgb(120, 90, 170),
                "Cross-instance handoff",
            );
            ui.label(
                "Pass a concise brief to another connected ChattyCog instance without leaving the local network.",
            );

            if snapshot.connected_peers.is_empty() {
                ui.label("Connect to another ChattyCog instance to send a handoff.");
            } else {
                let selected_label = snapshot
                    .connected_peers
                    .iter()
                    .find(|peer| peer.connection_id == app.networking_handoff_target)
                    .map(|peer| peer.device_name.clone())
                    .unwrap_or_else(|| "(choose target)".to_string());

                egui::ComboBox::from_id_salt("network_handoff_target")
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        for peer in &snapshot.connected_peers {
                            ui.selectable_value(
                                &mut app.networking_handoff_target,
                                peer.connection_id.clone(),
                                peer.device_name.clone(),
                            );
                        }
                    });

                ui.add(
                    egui::TextEdit::singleline(&mut app.networking_handoff_title)
                        .hint_text("Short handoff title..."),
                );
                ui.add(
                    egui::TextEdit::multiline(&mut app.networking_handoff_body)
                        .desired_rows(5)
                        .hint_text("What should the other instance know or pick up?"),
                );

                let send_enabled = !app.networking_handoff_target.trim().is_empty()
                    && !app.networking_handoff_body.trim().is_empty();
                if ui
                    .add_enabled(send_enabled, egui::Button::new("Send handoff"))
                    .clicked()
                {
                    app.networking.send_handoff(
                        &app.networking_handoff_target,
                        &app.networking_handoff_title,
                        &app.networking_handoff_body,
                    );
                    app.networking_handoff_title.clear();
                    app.networking_handoff_body.clear();
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("[IN]")
                        .color(egui::Color32::from_rgb(120, 90, 170))
                        .strong(),
                );
                ui.heading("Received handoffs");
                if !snapshot.received_handoffs.is_empty() && ui.button("Clear received").clicked() {
                    app.networking.clear_received_handoffs();
                    app.networking_seen_handoffs.clear();
                }
            });

            if snapshot.received_handoffs.is_empty() {
                ui.label("(none yet)");
            } else {
                for handoff in &snapshot.received_handoffs {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(if handoff.title.trim().is_empty() {
                                "(untitled handoff)"
                            } else {
                                handoff.title.trim()
                            });
                            ui.small(format!(
                                "from {} • {}s ago",
                                handoff.from_device_name, handoff.received_secs_ago
                            ));
                        });
                        ui.monospace(&handoff.from_device_id);
                        ui.small(format!("Address: {}", handoff.from_address));
                        ui.label(&handoff.body);
                    });
                    ui.add_space(6.0);
                }
            }

            ui.add_space(12.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("[XFER]")
                        .color(egui::Color32::from_rgb(70, 140, 90))
                        .strong(),
                );
                ui.heading("Received transfers");
                if !snapshot.received_artifacts.is_empty()
                    && ui.button("Clear transfers").clicked()
                {
                    app.networking.clear_received_artifacts();
                    app.networking_seen_artifacts.clear();
                }
            });

            if received_transfer_visible.is_empty() {
                ui.label("(no shared module states or other transfers yet)");
            } else {
                for artifact in &received_transfer_visible {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(if artifact.label.trim().is_empty() {
                                "(untitled transfer)"
                            } else {
                                artifact.label.trim()
                            });
                            ui.small(format!(
                                "{} from {} | {}s ago",
                                artifact.kind.trim(),
                                artifact.from_device_name,
                                artifact.received_secs_ago
                            ));
                        });
                        ui.monospace(&artifact.from_device_id);
                        if !artifact.module_id.trim().is_empty() {
                            ui.small(format!("Module: {}", artifact.module_id));
                        }
                        if !artifact.file_name.trim().is_empty() {
                            ui.small(format!("File: {}", artifact.file_name));
                        }
                        ui.small(format_network_transfer_meta(
                            &artifact.content_type,
                            &artifact.transfer_encoding,
                            artifact.byte_len,
                            artifact.chunk_count,
                        ));
                        if artifact.is_binary() {
                            ui.small("Payload: binary/file-style transfer");
                        }
                        if !artifact.summary.trim().is_empty() {
                            ui.label(artifact.summary.trim());
                        }
                        ui.small(format!("Address: {}", artifact.from_address));
                    });
                    ui.add_space(6.0);
                }
            }

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[FIL]",
                egui::Color32::from_rgb(110, 130, 80),
                "Received file-style transfers",
            );
            app.render_received_generic_transfer_inbox(ui, "Received file transfers");

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[WRK]",
                egui::Color32::from_rgb(70, 140, 90),
                "Workflow inbox",
            );
            app.render_received_workflow_inbox(ui, "Received workflows", None);

            ui.add_space(12.0);
            ui.separator();
            section_heading(
                ui,
                "[SET]",
                egui::Color32::from_rgb(90, 110, 170),
                "Received setup bundles",
            );
            app.render_received_workflow_bundle_inbox(ui, "Received workflow bundles");
        });
}

fn sandbox_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Sandbox");
    ui.separator();

    let Some(dir) = app.sandbox_dir.clone() else {
        ui.label("Sandbox folder not found. Create `Chatty_Sandbox/` inside the app folder.");
        return;
    };

    // Hard safety: never allow the editor to point outside the sandbox.
    if let Some(p) = app.sandbox_editor_path.clone() {
        if ensure_save_path_within_dir(&dir, &p).is_err() {
            app.sandbox_editor_path = None;
            app.sandbox_status = "Blocked unsafe sandbox path.".to_string();
        }
    }

    ui.group(|ui| {
        ui.heading("Scratchpad");
        ui.small(
            "Persistent working notes for ChattyCog. The chat prompt can see this file, and the model can request writes/appends to it through the sandbox tool flow.",
        );
        ui.horizontal(|ui| {
            if ui.button("Open default scratchpad").clicked() {
                app.open_default_sandbox_scratchpad();
            }
            if ui.button("Append hot memory snapshot").clicked() {
                let snapshot = if app.hot_memory.is_empty() {
                    String::new()
                } else {
                    let mut text =
                        format!("# Hot memory snapshot ({})\n", now_unix_ms().max(0));
                    for item in &app.hot_memory {
                        text.push_str("- ");
                        text.push_str(item);
                        text.push('\n');
                    }
                    text.push('\n');
                    text
                };
                if snapshot.trim().is_empty() {
                    app.sandbox_status = "Hot memory is empty.".to_string();
                } else {
                    match sandbox_append(&dir, DEFAULT_SANDBOX_SCRATCHPAD_REL_PATH, &snapshot) {
                        Ok(path) => {
                            app.sandbox_status = format!("Appended hot memory to {}", path.display());
                            app.open_sandbox_file_in_editor(&path);
                        }
                        Err(err) => {
                            app.sandbox_status =
                                format!("Could not append hot memory snapshot: {err}");
                        }
                    }
                }
            }
        });
    });

    ui.add_space(8.0);

    ui.group(|ui| {
        ui.heading("Task Ledger");
        ui.small(
            "Structured durable state for longer tasks: current task, next step, open questions, and files touched.",
        );
        ui.horizontal(|ui| {
            if ui.button("Open task ledger").clicked() {
                app.open_default_sandbox_task_ledger();
            }
            if ui.button("Seed from current context").clicked() {
                app.seed_default_sandbox_task_ledger_from_context();
            }
        });
    });

    ui.add_space(8.0);

    ui.columns(2, |cols| {
        cols[0].heading("Files");
        egui::ScrollArea::vertical()
            .id_salt("sandbox_files_scroll")
            .show(&mut cols[0], |ui| {
                for p in list_sandbox_files(&dir) {
                    let name = p
                        .strip_prefix(&dir)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    if ui
                        .selectable_label(app.sandbox_selected.as_ref() == Some(&p), name)
                        .clicked()
                    {
                        app.open_sandbox_file_in_editor(&p);
                    }
                }
            });

        cols[1].heading("Editor");
        let ledger_summary = read_task_ledger_summary(&dir);
        cols[1].group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("Task Ledger Snapshot");
                if let Some(summary) = ledger_summary.as_ref() {
                    if !summary.status.trim().is_empty() {
                        ui.small(format!("Status: {}", summary.status.trim()));
                    }
                }
                if ui.button("Open ledger").clicked() {
                    app.open_default_sandbox_task_ledger();
                }
            });
            if let Some(summary) = ledger_summary.as_ref() {
                ui.small(
                    "Read-only summary of the structured task ledger. Use the ledger itself for edits.",
                );
                ui.add_space(4.0);
                ui.label(format!(
                    "Current task: {}",
                    if summary.current_task.trim().is_empty() {
                        "(not set)"
                    } else {
                        summary.current_task.trim()
                    }
                ));
                ui.label(format!(
                    "Next step: {}",
                    if summary.next_step.trim().is_empty() {
                        "(not set)"
                    } else {
                        summary.next_step.trim()
                    }
                ));
                ui.horizontal_wrapped(|ui| {
                    ui.small(format!("Open questions: {}", summary.open_questions.len()));
                    ui.small(format!("Files touched: {}", summary.files_touched.len()));
                    ui.small(format!("Working notes: {}", summary.notes.len()));
                });

                if !summary.open_questions.is_empty() {
                    ui.add_space(4.0);
                    ui.small("Open questions:");
                    for item in summary.open_questions.iter().take(3) {
                        ui.small(format!("- {}", truncate_for_ui(item, 120)));
                    }
                }
                if !summary.files_touched.is_empty() {
                    ui.add_space(4.0);
                    ui.small(format!(
                        "Recent files: {}",
                        truncate_for_ui(&summary.files_touched.join(", "), 180)
                    ));
                }
            } else {
                ui.small("Task ledger not available yet.");
            }
        });
        cols[1].add_space(8.0);
        cols[1].horizontal(|ui| {
            if ui.button("New scratch").clicked() {
                app.sandbox_editor_path = None;
                app.sandbox_editor_text.clear();
                app.sandbox_status = "New scratch buffer".to_string();
            }
            if ui.button("Append summary to hot memory").clicked() {
                app.append_editor_summary_to_hot_memory();
            }
            if ui.button("Use as current task").clicked() {
                app.set_task_ledger_field_from_editor(true);
            }
            if ui.button("Use as next step").clicked() {
                app.set_task_ledger_field_from_editor(false);
            }
            if ui.button("Promote to scratchpad").clicked() {
                app.promote_editor_text_to_scratchpad();
            }
            if ui.button("Promote to ledger notes").clicked() {
                app.promote_editor_text_to_ledger_notes();
            }
            if ui.button("Save as...").clicked() {
                if let Some(path) = rfd::FileDialog::new().set_directory(&dir).save_file() {
                    match ensure_save_path_within_dir(&dir, &path).and_then(|pp| {
                        std::fs::write(&pp, &app.sandbox_editor_text)
                            .with_context(|| format!("write {}", pp.display()))?;
                        Ok(pp)
                    }) {
                        Ok(pp) => {
                            app.sandbox_editor_path = Some(pp.clone());
                            app.sandbox_status = format!("Saved {}", pp.display());
                        }
                        Err(e) => app.sandbox_status = format!("Save blocked/failed: {e}"),
                    }
                }
            }
            if ui.button("Save").clicked() {
                if let Some(path) = &app.sandbox_editor_path {
                    match ensure_save_path_within_dir(&dir, path).and_then(|pp| {
                        std::fs::write(&pp, &app.sandbox_editor_text)
                            .with_context(|| format!("write {}", pp.display()))?;
                        Ok(pp)
                    }) {
                        Ok(pp) => {
                            app.sandbox_editor_path = Some(pp.clone());
                            app.sandbox_status = format!("Saved {}", pp.display());
                        }
                        Err(e) => app.sandbox_status = format!("Save blocked/failed: {e}"),
                    }
                } else {
                    app.sandbox_status = "No file path. Use Save as...".to_string();
                }
            }
        });

        egui::ScrollArea::vertical()
            .id_salt("sandbox_editor_scroll")
            .show(&mut cols[1], |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut app.sandbox_editor_text)
                        .desired_rows(24)
                        .code_editor(),
                );
            });
    });
}

fn settings_tab(ui: &mut egui::Ui, _app: &mut ChattyCogApp) {
    ui.heading("Settings");
    ui.separator();
    ui.label("Planned:");
    ui.label("- Keyboard shortcuts");
    ui.label("- Appearance (old-school theme presets)");
    ui.label("- Default model folder");
}

fn about_tab(ui: &mut egui::Ui) {
    ui.heading("ChattyCog");
    ui.separator();
    ui.label("Old-school, tabbed desktop UI for chatting with local GGUF models.");
    ui.add_space(8.0);
    ui.label("Status: llama.cpp runtime wired (llama.dll + ggml backends).");
}

fn module_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp, module_id: &str) {
    let mf = app
        .module_registry
        .modules
        .iter()
        .find(|m| m.module_id == module_id)
        .cloned();
    let module_dir = mf.as_ref().map(|m| m.dir.clone());

    if mf.is_none() {
        ui.heading("Module");
        ui.separator();
        ui.label("Manifest not found (module was removed or rescanned).");
        return;
    }

    let hosted_visual = mf
        .as_ref()
        .and_then(|module| module.visual_load.clone())
        .filter(|visual| visual.hosts_native_window());

    if let (Some(dir), Some(visual)) = (module_dir.as_deref(), hosted_visual.as_ref()) {
        render_module_host_tab(ui, app, mf.as_ref(), module_id, dir, visual);
        return;
    }

    render_standard_module_tab(ui, app, mf.as_ref(), module_id, module_dir.as_deref());
}

fn render_standard_module_tab(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: Option<&Path>,
) {
    egui::ScrollArea::vertical()
        .id_salt(format!("module_tab_scroll_{module_id}"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_module_support_panels(ui, app, manifest, module_id, module_dir, true);
        });
}

fn render_module_host_tab(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: &Path,
    visual: &ModuleVisualLoad,
) {
    let host = app
        .module_hosts
        .entry(module_id.to_string())
        .or_insert_with(ModuleHostState::default);
    let running = host.is_running();

    ui.horizontal_wrapped(|ui| {
        if let Some(manifest) = manifest {
            ui.heading(&manifest.display_name);
            ui.separator();
            ui.small(&manifest.description);
            ui.separator();
        }

        if visual.build.is_some() && ui.button("Build UI").clicked() {
            if let Err(err) = host.start_build(module_dir, visual) {
                host.status = err;
            }
        }

        if running {
            if ui.button("Restart UI").clicked() {
                host.force_stop();
                if let Err(err) = host.launch(module_dir, visual) {
                    host.status = err;
                }
            }
            if ui.button("Close module app").clicked() {
                host.request_close(visual);
            }
        } else if ui.button("Launch in tab").clicked() {
            if let Err(err) = host.launch(module_dir, visual) {
                host.status = err;
            }
        }

        if ui.button("Open module folder").clicked() {
            open_path_in_explorer(module_dir);
        }

        ui.separator();
        ui.small(host.status.clone());
    });

    if !visual.notes.trim().is_empty() {
        ui.small(visual.notes.trim());
    }

    ui.add_space(6.0);
    egui::CollapsingHeader::new("ChattyCog bridge")
        .default_open(false)
        .show(ui, |ui| {
            ui.small(
                "Use this only for the compatibility loop: module-reported status, suspend rundown, and the optional ChattyCog-side helper. The hosted module keeps owning its own real UI/state.",
            );
            ui.add_space(6.0);
            render_module_support_panels(ui, app, manifest, module_id, Some(module_dir), true);
        });

    ui.add_space(8.0);
    let available = ui.available_size();
    let desired = egui::vec2(available.x.max(240.0), available.y.max(320.0));
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, egui::Color32::WHITE);
    ui.painter()
        .rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::LIGHT_GRAY));

    app.set_module_host_target(module_id, rect, ui.ctx().pixels_per_point());

    let host = app
        .module_hosts
        .entry(module_id.to_string())
        .or_insert_with(ModuleHostState::default);
    let centered = rect.center();
    let message = if host.is_running() {
        if host.is_waiting_for_window() {
            if visual.kind.trim().eq_ignore_ascii_case("webview") {
                "Launching hosted webview..."
            } else {
                "Launching module window..."
            }
        } else {
            if visual.kind.trim().eq_ignore_ascii_case("webview") {
                "Module webview is hosted here."
            } else {
                "Module window is hosted here."
            }
        }
    } else {
        if visual.kind.trim().eq_ignore_ascii_case("webview") {
            "Hosted webview is not running yet."
        } else {
            "Native module UI is not running yet."
        }
    };
    ui.painter().text(
        centered,
        egui::Align2::CENTER_CENTER,
        message,
        egui::TextStyle::Body.resolve(ui.style()),
        egui::Color32::DARK_GRAY,
    );
}

fn module_allows_network_feature(
    manifest: Option<&ModuleManifest>,
    feature: ModuleNetworkFeature,
) -> bool {
    manifest
        .and_then(|mf| mf.network_capabilities.as_ref())
        .map(|caps| caps.has(feature))
        .unwrap_or(true)
}

fn render_module_support_panels(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: Option<&Path>,
    include_surface: bool,
) {
    if include_surface {
        if let Some(dir) = module_dir {
            render_module_surface(ui, app, manifest, module_id, dir);
        }
        ui.add_space(8.0);
    }

    if let Some(dir) = module_dir {
        let network_caps = manifest.and_then(|mf| mf.network_capabilities.as_ref());
        egui::CollapsingHeader::new("Declared network capabilities")
            .default_open(false)
            .show(ui, |ui| {
                if let Some(caps) = network_caps {
                    ui.small(
                        "Optional contract: this tells ChattyCog which network lanes the module intentionally supports, so future sharing stays predictable and portable.",
                    );
                    if !caps.features.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            for feature in &caps.features {
                                ui.label(
                                    egui::RichText::new(feature.label())
                                        .small()
                                        .monospace(),
                                );
                            }
                        });
                    }
                    if !caps.asset_lanes.is_empty() {
                        ui.add_space(6.0);
                        ui.label("Declared asset lanes");
                        for lane in &caps.asset_lanes {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(lane.label.trim());
                                    ui.small(format!(
                                        "[{} | {} | {}]",
                                        lane.lane_id,
                                        lane.direction.label(),
                                        lane.delivery_mode.label()
                                    ));
                                });
                                let mut summary_bits = Vec::new();
                                if !lane.artifact_kinds.is_empty() {
                                    summary_bits
                                        .push(format!("Kinds: {}", lane.artifact_kinds.join(", ")));
                                }
                                if !lane.accepted_content_types.is_empty() {
                                    summary_bits.push(format!(
                                        "Content: {}",
                                        lane.accepted_content_types.join(", ")
                                    ));
                                }
                                if let Some(max_bytes) = lane.max_bytes {
                                    summary_bits
                                        .push(format!("Max: {}", format_network_transfer_size(max_bytes)));
                                }
                                summary_bits.push(if lane.replayable {
                                    "Replayable".to_string()
                                } else {
                                    "Not replayable".to_string()
                                });
                                ui.small(summary_bits.join(" | "));
                                for note in &lane.notes {
                                    ui.small(format!("Note: {}", note));
                                }
                            });
                        }
                    }
                    for note in &caps.notes {
                        ui.small(format!("Note: {}", note));
                    }
                    if caps.features.is_empty() && caps.asset_lanes.is_empty() && caps.notes.is_empty() {
                        ui.small("This module's capability block is present but currently empty.");
                    }
                } else {
                    ui.small(
                        "No `network_capabilities.json` declared yet. ChattyCog will keep falling back to bridge-file presence and safe manual controls.",
                    );
                }
            });

        let room_capable = manifest
            .and_then(|mf| mf.network_capabilities.as_ref())
            .map(|caps| {
                caps.has(ModuleNetworkFeature::RoomAware)
                    || caps.has(ModuleNetworkFeature::Multiplayer)
            })
            .unwrap_or(false);
        if room_capable {
            egui::CollapsingHeader::new("Shared room lane")
                .default_open(false)
                .show(ui, |ui| {
                    let multiplayer = manifest
                        .and_then(|mf| mf.network_capabilities.as_ref())
                        .map(|caps| caps.has(ModuleNetworkFeature::Multiplayer))
                        .unwrap_or(false);
                    if app.shared_chat_scope_matches_module(module_id) {
                        ui.small(format!(
                            "The shared room is currently focused on this module: {}.",
                            app.shared_chat_scope_label()
                        ));
                    } else {
                        ui.small(
                            "This module can opt into the shared-room lane. Use the buttons below when you want the room policy to follow this module cleanly.",
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        let button_label = if multiplayer {
                            "Use this module as multiplayer room"
                        } else {
                            "Use this module in shared room"
                        };
                        if ui.button(button_label).clicked() {
                            let module_name = manifest
                                .map(|mf| mf.display_name.clone())
                                .unwrap_or_else(|| module_id.to_string());
                            app.set_shared_chat_scope_module(
                                module_id.to_string(),
                                module_name,
                                multiplayer,
                            );
                            app.broadcast_shared_chat_policy(
                                "Room scope moved to this module.",
                            );
                        }
                        if ui.button("Return room to general").clicked() {
                            app.set_shared_chat_scope_general();
                            app.broadcast_shared_chat_policy(
                                "Room scope returned to the general lane.",
                            );
                        }
                        if ui.button("Open shared room controls").clicked() {
                            set_active_tab(app, Tab::Networking, "Networking");
                            app.focus_networking_section(NetworkingFocusSection::SharedRoom);
                        }
                        if app.shared_chat_scope_matches_module(module_id)
                            && !app.networking_shared_chat_policy.session_active
                            && ui.button("Start room session now").clicked()
                        {
                            if let Some(module_name) = app.begin_shared_chat_module_session() {
                                app.broadcast_shared_chat_policy(&format!(
                                    "Started host-guided module session for {module_name}."
                                ));
                            }
                        } else if app.shared_chat_scope_matches_module(module_id)
                            && app.networking_shared_chat_policy.session_active
                            && ui.button("End room session now").clicked()
                        {
                            let label = app
                                .networking_shared_chat_policy
                                .session_label
                                .trim()
                                .to_string();
                            app.end_shared_chat_module_session();
                            app.broadcast_shared_chat_policy(&format!(
                                "Ended {}.",
                                if label.is_empty() {
                                    "the module session".to_string()
                                } else {
                                    label
                                }
                            ));
                        }
                    });
                });
        }

        egui::CollapsingHeader::new("Module-reported status (portable bridge)")
            .default_open(false)
            .show(ui, |ui| {
                let status_path = bridge_status_path(dir);
                let log_sources_path = bridge_log_sources_path(dir);
                let shared_state_path = bridge_shared_state_path(dir);
                let incoming_shared_state_path = bridge_incoming_shared_state_path(dir);
                let incoming_assets_dir = bridge_incoming_assets_dir(dir);
                let shared_room_state_path = bridge_shared_room_state_path(dir);
                let shared_room_events_path = bridge_shared_room_events_path(dir);
                let outgoing_room_events_path = bridge_outgoing_room_events_path(dir);
                ui.small(
                    "Optional plug: the module stays standalone and only reports summary/snapshot here when it wants ChattyCog context handoff. If `log_sources.json` exists, ChattyCog can also tail declared module-local logs for auto-rundown context.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Open bridge folder").clicked() {
                        open_path_in_explorer(status_path.parent().unwrap_or(dir));
                    }
                    if status_path.is_file() && ui.button("Open status.json").clicked() {
                        open_path_in_explorer(&status_path);
                    }
                    if log_sources_path.is_file() && ui.button("Open log_sources.json").clicked() {
                        open_path_in_explorer(&log_sources_path);
                    }
                    if shared_state_path.is_file() && ui.button("Open shared_state.json").clicked() {
                        open_path_in_explorer(&shared_state_path);
                    }
                    if incoming_shared_state_path.is_file()
                        && ui.button("Open incoming_shared_state.json").clicked()
                    {
                        open_path_in_explorer(&incoming_shared_state_path);
                    }
                    if shared_room_state_path.is_file()
                        && ui.button("Open shared_room_state.json").clicked()
                    {
                        open_path_in_explorer(&shared_room_state_path);
                    }
                    if shared_room_events_path.is_file()
                        && ui.button("Open shared_room_events.json").clicked()
                    {
                        open_path_in_explorer(&shared_room_events_path);
                    }
                    if outgoing_room_events_path.is_file()
                        && ui.button("Open outgoing_room_events.json").clicked()
                    {
                        open_path_in_explorer(&outgoing_room_events_path);
                    }
                    if incoming_assets_dir.is_dir() && ui.button("Open incoming assets").clicked() {
                        open_path_in_explorer(&incoming_assets_dir);
                    }
                });

                match app.read_module_bridge_status(module_id, dir) {
                    Some(status) => {
                        if status.updated_at_unix_ms > 0 {
                            ui.small(format!(
                                "Last update: {}",
                                status.updated_at_unix_ms
                            ));
                        }
                        if !status.tags.is_empty() {
                            ui.small(format!("Tags: {}", status.tags.join(", ")));
                        }
                        if !status.summary.trim().is_empty() {
                            ui.label("Summary");
                            ui.group(|ui| {
                                ui.label(status.summary.trim());
                            });
                        }
                        if !status.snapshot.trim().is_empty() {
                            ui.add_space(6.0);
                            ui.label("Snapshot");
                            let mut snapshot = status.snapshot.clone();
                            egui::ScrollArea::vertical()
                                .id_salt(format!("module_bridge_snapshot_{module_id}"))
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut snapshot)
                                            .desired_rows(8)
                                            .interactive(false),
                                    );
                                });
                        }
                    }
                    None => {
                        ui.small(
                            "No bridge status yet. The module is still standalone; it just has not reported a rundown for ChattyCog to read.",
                        );
                    }
                }

                if room_capable {
                    ui.add_space(6.0);
                    ui.label("Shared room state");
                    match read_bridge_shared_room_state(dir) {
                        Ok(Some(room_state)) => {
                            ui.small(format!(
                                "Last room-state update: {}",
                                room_state.updated_at_unix_ms
                            ));
                            ui.small(format!(
                                "Scope: {}",
                                if room_state.scope_kind.trim() == "module"
                                    && !room_state.scope_module_name.trim().is_empty()
                                {
                                    if room_state.scope_multiplayer {
                                        format!(
                                            "{} (multiplayer)",
                                            room_state.scope_module_name.trim()
                                        )
                                    } else {
                                        format!(
                                            "{} (module)",
                                            room_state.scope_module_name.trim()
                                        )
                                    }
                                } else {
                                    "General room".to_string()
                                }
                            ));
                            ui.small(format!(
                                "Active for this module: {}",
                                if room_state.active_for_module {
                                    "yes"
                                } else {
                                    "no"
                                }
                            ));
                            ui.small(format!(
                                "Turn mode: {} | AI mode: {}",
                                if room_state.turn_mode.trim().is_empty() {
                                    "(unset)"
                                } else {
                                    room_state.turn_mode.trim()
                                },
                                if room_state.ai_mode.trim().is_empty() {
                                    "(unset)"
                                } else {
                                    room_state.ai_mode.trim()
                                }
                            ));
                            if room_state.session_active {
                                ui.small(format!(
                                    "Session: {} | revision {}{}",
                                    if room_state.session_label.trim().is_empty() {
                                        if room_state.session_id.trim().is_empty() {
                                            "(unnamed session)"
                                        } else {
                                            room_state.session_id.trim()
                                        }
                                    } else {
                                        room_state.session_label.trim()
                                    },
                                    room_state.session_revision.max(1),
                                    if room_state.host_authoritative {
                                        " | host-authoritative"
                                    } else {
                                        ""
                                    }
                                ));
                            } else {
                                ui.small("Session: inactive");
                            }
                            if !room_state.host_device_name.trim().is_empty() {
                                ui.small(format!(
                                    "Host: {}",
                                    room_state.host_device_name.trim()
                                ));
                            }
                            if !room_state.turn_holder_device_name.trim().is_empty() {
                                ui.small(format!(
                                    "Turn holder: {}",
                                    room_state.turn_holder_device_name.trim()
                                ));
                            }
                            ui.small(format!(
                                "Connected peers in room: {}",
                                room_state.connected_peer_count
                            ));
                            ui.small(format!(
                                "Participants visible to module: {}",
                                room_state.participant_count
                            ));
                            if !room_state.participants.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for participant in room_state.participants.iter().take(8) {
                                        let label = if participant.device_name.trim().is_empty() {
                                            participant.device_id.trim()
                                        } else {
                                            participant.device_name.trim()
                                        };
                                        ui.small(if participant.is_local {
                                            format!("(local) {label}")
                                        } else {
                                            label.to_string()
                                        });
                                    }
                                });
                            }
                            if !room_state.summary.trim().is_empty() {
                                ui.group(|ui| {
                                    ui.label(room_state.summary.trim());
                                });
                            }
                        }
                        Ok(None) => {
                            ui.small(
                                "No shared_room_state.json yet. Once the shared-room lane is active, ChattyCog will mirror that room policy here for room-aware or multiplayer modules.",
                            );
                        }
                        Err(err) => {
                            ui.small(format!(
                                "Could not read shared_room_state.json: {err}"
                            ));
                        }
                    }
                }

                if room_capable {
                    ui.add_space(6.0);
                    ui.label("Recent shared room events");
                    match read_bridge_shared_room_events(dir) {
                        Ok(Some(events)) => {
                            ui.small(format!(
                                "Last event sync: {} | {} event(s)",
                                events.updated_at_unix_ms,
                                events.events.len()
                            ));
                            for event in events.events.iter().rev().take(8) {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(if event.label.trim().is_empty() {
                                            event.event_type.trim()
                                        } else {
                                            event.label.trim()
                                        });
                                        ui.small(format!(
                                            "{} | {}",
                                            if event.from_device_name.trim().is_empty() {
                                                "(unknown sender)"
                                            } else {
                                                event.from_device_name.trim()
                                            },
                                            event.received_at_unix_ms
                                        ));
                                    });
                                    if !event.payload_text.trim().is_empty() {
                                        ui.label(event.payload_text.trim());
                                    } else {
                                        ui.small("(no text payload)");
                                    }
                                });
                            }
                        }
                        Ok(None) => {
                            ui.small(
                                "No shared_room_events.json yet. Room-aware modules can read a recent event feed here once peers start emitting lightweight room events.",
                            );
                        }
                        Err(err) => {
                            ui.small(format!(
                                "Could not read shared_room_events.json: {err}"
                            ));
                        }
                    }
                    match read_bridge_outgoing_room_events(dir) {
                        Ok(events) if !events.is_empty() => {
                            ui.small(format!(
                                "Queued outgoing room events: {}",
                                events.len()
                            ));
                        }
                        Ok(_) => {}
                        Err(err) => {
                            ui.small(format!(
                                "Could not read outgoing_room_events.json: {err}"
                            ));
                        }
                    }
                }

                ui.add_space(6.0);
                ui.label("Shared session state");
                if let Some(shared_state) = app.read_module_bridge_shared_state(module_id, dir) {
                    let can_publish_shared_state = module_allows_network_feature(
                        manifest,
                        ModuleNetworkFeature::SharedStatePublish,
                    );
                    let can_receive_shared_state = module_allows_network_feature(
                        manifest,
                        ModuleNetworkFeature::SharedStateReceive,
                    );
                    let tracker = app.module_session_trackers.get(module_id).cloned();
                    if shared_state.updated_at_unix_ms > 0 {
                        ui.small(format!(
                            "Last shared-state update: {}",
                            shared_state.updated_at_unix_ms
                        ));
                    }
                    if let Some(tracker) = &tracker {
                        ui.small(format!(
                            "Current shared session: {} | revision {}",
                            tracker.session_id, tracker.last_revision
                        ));
                    } else if !shared_state.session_id.trim().is_empty() {
                        ui.small(format!(
                            "Current shared session: {} | revision {}",
                            shared_state.session_id, shared_state.session_revision
                        ));
                    }
                    if !shared_state.summary.trim().is_empty() {
                        ui.group(|ui| {
                            ui.label(shared_state.summary.trim());
                        });
                    } else {
                        ui.small("This module published shared state without a human summary.");
                    }

                    if !shared_state.payload.is_null() {
                        let mut payload =
                            serde_json::to_string_pretty(&shared_state.payload).unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt(format!("module_bridge_shared_state_{module_id}"))
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut payload)
                                        .desired_rows(6)
                                        .interactive(false),
                                );
                            });
                    }

                    let selected_connections = app.selected_network_connection_ids();
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Start new shared session").clicked() {
                            app.reset_module_shared_session(module_id);
                            app.networking_status = format!(
                                "Networking: reset the shared session for {}.",
                                manifest
                                    .map(|mf| mf.display_name.clone())
                                    .unwrap_or_else(|| module_id.to_string())
                            );
                        }
                        if selected_connections.is_empty() {
                            ui.small(
                                "Select one or more connected peers in Networking to share this module state.",
                            );
                        } else if !can_publish_shared_state {
                            ui.small(
                                "This module has not declared `shared_state_publish` support yet.",
                            );
                        } else if ui.button("Share to selected peers").clicked() {
                            let prepared = app.prepare_outgoing_module_shared_state(module_id, &shared_state);
                            match serde_json::to_string_pretty(&prepared) {
                            Ok(text) => {
                                app.remember_recoverable_module_shared_state(
                                    module_id,
                                    &prepared,
                                    &text,
                                );
                                let label = manifest
                                    .map(|mf| format!("{} shared state", mf.display_name))
                                    .unwrap_or_else(|| format!("{module_id} shared state"));
                                let summary = if prepared.summary.trim().is_empty() {
                                    manifest
                                        .map(|mf| {
                                            format!(
                                                "Shared workflow state for {}",
                                                mf.display_name.trim()
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            format!("Shared workflow state for {module_id}")
                                        })
                                } else {
                                    prepared.summary.trim().to_string()
                                };
                                let file_name =
                                    format!("{}_shared_state.json", slugify_filename(module_id, "module"));
                                for connection_id in &selected_connections {
                                    app.networking.send_artifact(
                                        connection_id,
                                        "module_shared_state_json",
                                        &label,
                                        Some(module_id),
                                        &summary,
                                        &file_name,
                                        &text,
                                    );
                                }
                                let module_label = manifest
                                    .map(|mf| mf.display_name.clone())
                                    .unwrap_or_else(|| module_id.to_string());
                                app.networking_status = format!(
                                    "Networking: shared {} session {} revision {} with {} selected peer(s).",
                                    module_label,
                                    prepared.session_id,
                                    prepared.session_revision,
                                    selected_connections.len()
                                );
                            }
                            Err(err) => {
                                app.networking_status = format!(
                                    "Networking: could not serialize shared state for {}: {}",
                                    module_id, err
                                );
                            }
                        }
                        }
                    });
                    if !can_receive_shared_state {
                        ui.small(
                            "This module has not declared `shared_state_receive` support yet, so incoming workflow applies stay disabled.",
                        );
                    }
                } else {
                    ui.small(
                        "No shared_state.json yet. Add the optional shared-state plug if you want this module to sync a ready-to-use workflow state across the LAN.",
                    );
                }

                let has_pending_workflows = app
                    .received_workflow_inbox
                    .iter()
                    .any(|item| item.record.module_id.trim() == module_id);
                if has_pending_workflows {
                    ui.add_space(6.0);
                    app.render_received_workflow_inbox(
                        ui,
                        "Received workflow inbox",
                        Some(module_id),
                    );
                }

                if let Some(incoming) = app.read_module_bridge_incoming_shared_state(module_id, dir) {
                    ui.add_space(6.0);
                    ui.label("Incoming shared state");
                    ui.small(format!(
                        "Most recent network state came from {} [{}].",
                        if incoming.from_device_name.trim().is_empty() {
                            "(unknown device)"
                        } else {
                            incoming.from_device_name.trim()
                        },
                        incoming.from_device_id.trim()
                    ));
                    if !incoming.session_id.trim().is_empty() {
                        ui.small(format!(
                            "Session {} | revision {}{}",
                            incoming.session_id,
                            incoming.session_revision,
                            if incoming.host_authoritative {
                                " | host-authoritative"
                            } else {
                                ""
                            }
                        ));
                    }
                    if !incoming.summary.trim().is_empty() {
                        ui.group(|ui| {
                            ui.label(incoming.summary.trim());
                        });
                    }
                    if !incoming.payload.is_null() {
                        let mut payload =
                            serde_json::to_string_pretty(&incoming.payload).unwrap_or_default();
                        egui::ScrollArea::vertical()
                            .id_salt(format!("module_bridge_incoming_state_{module_id}"))
                            .max_height(120.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::multiline(&mut payload)
                                        .desired_rows(5)
                                        .interactive(false),
                                );
                            });
                    }
                }

                let incoming_asset_lanes = manifest
                    .and_then(|manifest| manifest.network_capabilities.as_ref())
                    .map(|caps| caps.asset_lanes.clone())
                    .unwrap_or_default();
                if !incoming_asset_lanes.is_empty() {
                    ui.add_space(6.0);
                    ui.label("Incoming asset lanes");
                    for lane in incoming_asset_lanes {
                        let incoming_assets =
                            app.read_module_bridge_incoming_assets(module_id, dir, Some(&lane.lane_id));
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.strong(lane.label.trim());
                                ui.small(format!(
                                    "[{} | {} waiting]",
                                    lane.lane_id,
                                    incoming_assets.len()
                                ));
                            });
                            ui.small(format!(
                                "{} | {}{}",
                                lane.direction.label(),
                                lane.delivery_mode.label(),
                                if lane.replayable { " | replayable" } else { "" }
                            ));
                            ui.horizontal_wrapped(|ui| {
                                if ui.button("Open lane folder").clicked() {
                                    open_path_in_explorer(&bridge_incoming_asset_lane_dir(
                                        dir,
                                        &lane.lane_id,
                                    ));
                                }
                                if !incoming_assets.is_empty() {
                                    ui.small("Modules can consume these from the bridge when ready.");
                                }
                            });
                            if incoming_assets.is_empty() {
                                ui.small("No assets are waiting in this lane right now.");
                            } else {
                                for asset in incoming_assets.iter().take(4) {
                                    ui.small(format!(
                                        "{} | {} | {}",
                                        if asset.label.trim().is_empty() {
                                            asset.kind.trim()
                                        } else {
                                            asset.label.trim()
                                        },
                                        if asset.from_device_name.trim().is_empty() {
                                            asset.from_device_id.trim()
                                        } else {
                                            asset.from_device_name.trim()
                                        },
                                        format_network_transfer_meta(
                                            &asset.content_type,
                                            &asset.transfer_encoding,
                                            asset.byte_len,
                                            asset.chunk_count,
                                        )
                                    ));
                                }
                            }
                            for note in &lane.notes {
                                ui.small(format!("Note: {}", note));
                            }
                        });
                    }
                }

                let receipts = app.module_session_receipts_for(module_id);
                if !receipts.is_empty() {
                    ui.add_space(6.0);
                    ui.label("Recent session apply receipts");
                    egui::ScrollArea::vertical()
                        .id_salt(format!("module_session_receipts_{module_id}"))
                        .max_height(120.0)
                        .show(ui, |ui| {
                            for receipt in receipts.iter().take(8) {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.strong(if receipt.from_device_name.trim().is_empty() {
                                            receipt.from_device_id.trim()
                                        } else {
                                            receipt.from_device_name.trim()
                                        });
                                        ui.small(format!(
                                            "session {} | revision {} | {}",
                                            receipt.session_id,
                                            receipt.session_revision,
                                            if receipt.applied {
                                                "applied"
                                            } else if receipt.stale {
                                                "stale"
                                            } else {
                                                "not applied"
                                            }
                                        ));
                                    });
                                    if !receipt.message.trim().is_empty() {
                                        ui.small(receipt.message.trim());
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                }
            });

        ui.add_space(8.0);
    }

    egui::CollapsingHeader::new("Suspend rundown (what Orchestrator sees)")
        .default_open(false)
        .show(ui, |ui| {
            ui.label("Short status used for the Bookkeeper debrief when you leave this tab.");
            ui.horizontal(|ui| {
                let running = app.module_rundown_jobs.contains_key(module_id);
                ui.add_enabled_ui(!running, |ui| {
                    if ui.button("Auto-generate (Bookkeeper)").clicked() {
                        app.start_module_rundown_job(module_id, true, false);
                    }
                });
                if running {
                    ui.small("Generating...");
                }
                ui.separator();
                ui.small(format!(
                    "Auto-generate on tab leave: {} (set in Preferences)",
                    if app.prefs.auto_generate_module_suspend_rundown {
                        "ON"
                    } else {
                        "OFF"
                    }
                ));
                if ui.button("Clear").clicked() {
                    let notes = app
                        .module_state_notes
                        .entry(module_id.to_string())
                        .or_insert_with(String::new);
                    notes.clear();
                }
            });
            let notes = app
                .module_state_notes
                .entry(module_id.to_string())
                .or_insert_with(String::new);
            egui::ScrollArea::vertical()
                .id_salt(format!("module_notes_{module_id}"))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(notes)
                            .desired_rows(10)
                            .hint_text("One paragraph max. What changed? What's next?"),
                    );
                });
        });

    ui.add_space(8.0);

    if let Some(mf) = manifest {
        if mf.ai_enabled {
            ui.separator();
            ui.heading("Module AI");
            ui.label("This module can run its own local model while the orchestrator is paused.");

            let st = app
                .module_ai
                .entry(module_id.to_string())
                .or_insert_with(ModuleAiState::default);

            if !st.initialized {
                if let Some(p) = app.prefs.modules.get(module_id) {
                    st.temp = p.params.temp;
                    st.top_p = p.params.top_p;
                    st.top_k = p.params.top_k;
                    st.max_tokens = p.params.max_tokens;
                }
                st.initialized = true;
            }

            if st.models_cache.is_empty() {
                st.models_cache = scan_ggufs(app.models_dir.as_deref());
            }
            if st.model_path.is_none() {
                let preferred = app
                    .prefs
                    .modules
                    .get(module_id)
                    .and_then(|p| p.preferred_model.as_ref())
                    .filter(|s| !s.trim().is_empty())
                    .cloned()
                    .or_else(|| mf.default_model.clone());

                if let Some(name) = preferred.as_ref() {
                    if let Some(p) = st
                        .models_cache
                        .iter()
                        .find(|p| {
                            p.file_name()
                                .map(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
                                .unwrap_or(false)
                        })
                        .cloned()
                    {
                        st.model_path = Some(p);
                    }
                }
            }

            ui.horizontal(|ui| {
                if ui.button("Refresh models").clicked() {
                    st.models_cache = scan_ggufs(app.models_dir.as_deref());
                }
                if ui.button("Stop").clicked() {
                    if let Some(c) = &st.cancel {
                        c.store(true, Ordering::Relaxed);
                    }
                }
                if !st.status.trim().is_empty() {
                    ui.label(st.status.clone());
                }
            });

            egui::ComboBox::from_label("Module model")
                .selected_text(
                    st.model_path
                        .as_ref()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                        .unwrap_or_else(|| "(none)".to_string()),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut st.model_path, None, "(none)");
                    for p in &st.models_cache {
                        let label = p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        ui.selectable_value(&mut st.model_path, Some(p.clone()), label);
                    }
                });

            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut st.temp, 0.0..=2.0).text("temp"));
                ui.add(egui::Slider::new(&mut st.top_p, 0.0..=1.0).text("top_p"));
                ui.add(egui::Slider::new(&mut st.top_k, 0..=200).text("top_k"));
                ui.add(egui::Slider::new(&mut st.max_tokens, 1..=2048).text("max_tokens"));
            });

            let events = st
                .rx
                .as_ref()
                .map(|rx| rx.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut close_after = false;
            for ev in events {
                match ev {
                    GenEvent::Token(t) => st.output.push_str(&t),
                    GenEvent::Info(s) => {
                        st.status = format!("Runtime: {}", truncate_for_ui(&s, 120))
                    }
                    GenEvent::Error(e) => {
                        st.status = format!("Error: {e}");
                        st.is_running = false;
                        st.cancel = None;
                        st.rx = None;
                        if app.close_pending_modules.contains(module_id) {
                            close_after = true;
                        }
                    }
                    GenEvent::Done => {
                        st.is_running = false;
                        st.cancel = None;
                        st.rx = None;
                        st.status = "Done.".to_string();
                        if app.close_pending_modules.contains(module_id) {
                            close_after = true;
                        }
                    }
                }
            }
            if close_after {
                let module_id = module_id.to_string();
                let _ = st;
                close_module_tab(app, &module_id);
                return;
            }

            ui.add_space(6.0);
            ui.label("Task input:");
            ui.add(
                egui::TextEdit::multiline(&mut st.user_input)
                    .desired_rows(4)
                    .hint_text("Describe the task this department should handle..."),
            );

            ui.horizontal(|ui| {
                ui.add_enabled_ui(!st.is_running, |ui| {
                    if ui.button("Run").clicked() {
                        let Some(model_path) = st.model_path.clone() else {
                            st.status = "Pick a module model first.".to_string();
                            return;
                        };
                        let runtime_dir = match find_runtime_windows_dir() {
                            Ok(p) => p,
                            Err(e) => {
                                st.status = format!("{e:#}");
                                return;
                            }
                        };

                        let input = st.user_input.trim().to_string();
                        if input.is_empty() {
                            st.status = "Enter a task first.".to_string();
                            return;
                        }

                        let module_name = mf.display_name.clone();
                        let module_description = mf.description.clone();
                        let system = format!(
                            "You are the {module_name} department inside ChattyCog.\n\
Module purpose: {module_description}\n\
Help with the task using the current module state as context.\n\
Keep the reply practical and concise.\n"
                        );

                        let (tx, rx) = crossbeam_channel::unbounded::<GenEvent>();
                        let cancel = Arc::new(AtomicBool::new(false));
                        let cancel_for_thread = Arc::clone(&cancel);
                        let temp = st.temp;
                        let top_p = st.top_p;
                        let top_k = st.top_k;
                        let max_tokens = st.max_tokens.max(1) as usize;

                        st.output.clear();
                        st.status = "Running...".to_string();
                        st.is_running = true;
                        st.cancel = Some(cancel);
                        st.rx = Some(rx);

                        std::thread::spawn(move || {
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
                                &input,
                                max_tokens,
                                temp,
                                top_p,
                                top_k,
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
                    }
                });

                if ui.button("Copy output -> suspend rundown").clicked() {
                    let block = st.output.trim();
                    if !block.is_empty() {
                        let entry = app
                            .module_state_notes
                            .entry(module_id.to_string())
                            .or_insert_with(String::new);
                        if !entry.trim().is_empty() {
                            entry.push_str("\n\n");
                        }
                        entry.push_str(block);
                    }
                }
            });

            ui.add_space(6.0);
            ui.label("Output:");
            egui::ScrollArea::vertical()
                .id_salt(format!("module_ai_out_{module_id}"))
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut st.output)
                            .desired_rows(12)
                            .code_editor(),
                    );
                });
        }
    }
}

fn open_path_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

fn value_is_meaningful(value: &ModuleFieldValue) -> bool {
    match value {
        ModuleFieldValue::Str(s) => !s.trim().is_empty(),
        ModuleFieldValue::Bool(b) => *b,
        ModuleFieldValue::Num(n) => n.abs() > f64::EPSILON,
    }
}

fn filled_field_count(spec: &ModuleUiSpec, values: &HashMap<String, ModuleFieldValue>) -> usize {
    spec.fields
        .iter()
        .filter(|f| values.get(f.id.trim()).is_some_and(value_is_meaningful))
        .count()
}

fn humanize_section_id(section: &str) -> String {
    let cleaned = section.trim().replace(['_', '-'], " ");
    if cleaned.is_empty() {
        return "Workspace".to_string();
    }
    cleaned
        .split_whitespace()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn module_surface_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let trimmed = relative.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Some(root.to_path_buf());
    }

    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute() {
        return None;
    }
    if rel_path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    }) {
        return None;
    }

    Some(root.join(rel_path))
}

fn module_field_label(spec: &ModuleUiSpec, field_id: &str) -> String {
    spec.fields
        .iter()
        .find(|field| field.id.trim() == field_id.trim())
        .map(|field| field.label.clone())
        .unwrap_or_else(|| humanize_section_id(field_id))
}

fn module_field_spec<'a>(spec: &'a ModuleUiSpec, field_id: &str) -> Option<&'a ModuleUiField> {
    spec.fields
        .iter()
        .find(|field| field.id.trim() == field_id.trim())
}

fn module_field_value_as_text(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Option<String> {
    match values.get(field_id.trim()) {
        Some(ModuleFieldValue::Str(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(ModuleFieldValue::Bool(value)) => Some(if *value { "Yes" } else { "No" }.to_string()),
        Some(ModuleFieldValue::Num(value)) => Some(
            format!("{value:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string(),
        ),
        None => None,
    }
}

fn module_field_value_as_number(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Option<f64> {
    match values.get(field_id.trim()) {
        Some(ModuleFieldValue::Num(value)) => Some(*value),
        Some(ModuleFieldValue::Str(value)) => value.trim().parse::<f64>().ok(),
        Some(ModuleFieldValue::Bool(value)) => Some(if *value { 1.0 } else { 0.0 }),
        None => None,
    }
}

fn module_field_candidate_paths(
    values: &HashMap<String, ModuleFieldValue>,
    field_id: &str,
) -> Vec<String> {
    let Some(text) = module_field_value_as_text(values, field_id) else {
        return Vec::new();
    };

    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
        })
        .map(|line| line.trim_matches('"').trim_matches('\'').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn resolve_module_block_paths(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = module_surface_path(module_dir, explicit_path) {
        candidates.push(path);
    }

    if !field_id.trim().is_empty() {
        for relative in module_field_candidate_paths(values, field_id) {
            if let Some(path) = module_surface_path(module_dir, &relative) {
                candidates.push(path);
            }
        }
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for path in candidates {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            deduped.push(path);
        }
    }
    deduped
}

fn looks_like_text_file(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some(
            "md" | "txt" | "json" | "jsonl" | "toml" | "yaml" | "yml" | "rs" | "py" | "js" | "ts"
            | "tsx" | "jsx" | "csv" | "log" | "ini" | "cfg" | "html" | "css" | "xml",
        ) => true,
        _ => false,
    }
}

fn preview_text_file(path: &Path, max_lines: usize) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let limit = bytes.len().min(16 * 1024);
    let preview = String::from_utf8_lossy(&bytes[..limit]).to_string();
    let mut lines = preview.lines().take(max_lines.max(1)).collect::<Vec<_>>();
    if lines.is_empty() && !preview.trim().is_empty() {
        lines.push(preview.trim());
    }
    let mut text = lines.join("\n");
    if bytes.len() > limit || preview.lines().count() > max_lines {
        if !text.is_empty() {
            text.push_str("\n…");
        } else {
            text.push('…');
        }
    }
    Some(text)
}

fn read_module_text_source(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Option<String> {
    if !field_id.trim().is_empty() {
        if let Some(content) = module_field_value_as_text(values, field_id) {
            return Some(content);
        }
    }

    let candidates = resolve_module_block_paths(module_dir, values, explicit_path, field_id);
    let path = candidates.into_iter().find(|path| path.is_file())?;
    if !looks_like_text_file(&path) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn read_module_table_source(
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    explicit_path: &str,
    field_id: &str,
) -> Option<String> {
    let content = read_module_text_source(module_dir, values, explicit_path, field_id)?;
    if content.contains('\n')
        || content.contains('|')
        || content.contains('\t')
        || content.contains(',')
        || content.contains(';')
    {
        Some(content)
    } else {
        None
    }
}

fn split_table_row(line: &str, delimiter: char) -> Vec<String> {
    if delimiter == '|' {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect()
    } else {
        line.split(delimiter)
            .map(|cell| cell.trim().to_string())
            .collect()
    }
}

fn parse_lightweight_table(
    content: &str,
    has_header: bool,
) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let delimiters = ['|', '\t', ',', ';'];
    let delimiter = delimiters.into_iter().find(|delimiter| {
        let matching = lines
            .iter()
            .take(6)
            .filter(|line| line.contains(*delimiter))
            .count();
        matching >= 1
    })?;

    let mut rows = lines
        .iter()
        .map(|line| split_table_row(line, delimiter))
        .filter(|row| row.len() >= 2)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return None;
    }

    let width = rows.iter().map(|row| row.len()).max().unwrap_or(0);
    if width == 0 {
        return None;
    }
    for row in &mut rows {
        while row.len() < width {
            row.push(String::new());
        }
    }

    let (header, body_start) = if has_header && rows.len() >= 2 {
        (rows[0].clone(), 1usize)
    } else {
        (
            (1..=width)
                .map(|idx| format!("Column {idx}"))
                .collect::<Vec<_>>(),
            0usize,
        )
    };

    let body = rows.into_iter().skip(body_start).collect::<Vec<_>>();
    Some((header, body))
}

#[derive(Debug, Clone, Copy)]
enum ChecklistState {
    Pending,
    InProgress,
    Done,
    Note,
}

#[derive(Debug, Clone)]
struct KanbanCard {
    lane: String,
    text: String,
}

#[derive(Debug, Clone)]
struct DependencyNodeView {
    name: String,
    depends_on: Vec<String>,
    unlocks: Vec<String>,
    stage: usize,
}

fn title_case_words(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.extend(first.to_uppercase());
                    out.push_str(&chars.as_str().to_ascii_lowercase());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn canonical_lane_name(raw: &str) -> String {
    let compact = raw
        .trim()
        .replace('_', " ")
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let lowered = compact.to_ascii_lowercase();
    match lowered.as_str() {
        "todo" | "to do" | "backlog" | "queued" | "queue" => "To Do".to_string(),
        "doing" | "in progress" | "inprogress" | "active" | "working" => "Doing".to_string(),
        "review" | "qa" | "verify" | "verification" => "Review".to_string(),
        "blocked" | "waiting" | "stalled" => "Blocked".to_string(),
        "done" | "complete" | "completed" | "finished" => "Done".to_string(),
        "note" | "notes" => "Notes".to_string(),
        "inbox" => "Inbox".to_string(),
        _ if compact.is_empty() => "Inbox".to_string(),
        _ => title_case_words(&compact),
    }
}

fn lane_accent_color(lane: &str) -> egui::Color32 {
    match canonical_lane_name(lane).to_ascii_lowercase().as_str() {
        "to do" | "inbox" => egui::Color32::from_rgb(30, 80, 180),
        "doing" | "review" => egui::Color32::from_rgb(180, 110, 10),
        "blocked" => egui::Color32::from_rgb(180, 40, 40),
        "done" => egui::Color32::from_rgb(20, 120, 60),
        _ => egui::Color32::from_gray(120),
    }
}

fn parse_checklist_line(line: &str) -> Option<(ChecklistState, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidates = [
        ("- [x] ", ChecklistState::Done),
        ("* [x] ", ChecklistState::Done),
        ("[x] ", ChecklistState::Done),
        ("- [X] ", ChecklistState::Done),
        ("* [X] ", ChecklistState::Done),
        ("[X] ", ChecklistState::Done),
        ("- [ ] ", ChecklistState::Pending),
        ("* [ ] ", ChecklistState::Pending),
        ("[ ] ", ChecklistState::Pending),
        ("- [-] ", ChecklistState::InProgress),
        ("* [-] ", ChecklistState::InProgress),
        ("[-] ", ChecklistState::InProgress),
        ("- [~] ", ChecklistState::InProgress),
        ("* [~] ", ChecklistState::InProgress),
        ("[~] ", ChecklistState::InProgress),
    ];

    for (prefix, state) in candidates {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let text = rest.trim();
            if !text.is_empty() {
                return Some((state, text.to_string()));
            }
        }
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let text = rest.trim();
        if !text.is_empty() {
            return Some((ChecklistState::Pending, text.to_string()));
        }
    }

    if let Some((prefix, rest)) = trimmed.split_once(". ") {
        if prefix.chars().all(|ch| ch.is_ascii_digit()) {
            let text = rest.trim();
            if !text.is_empty() {
                return Some((ChecklistState::Pending, text.to_string()));
            }
        }
    }

    Some((ChecklistState::Note, trimmed.to_string()))
}

fn parse_timeline_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return parse_timeline_line(rest);
    }

    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            let stamp = trimmed[1..end].trim();
            let rest = trimmed[end + 1..]
                .trim_start_matches([' ', '-', '—', '|'])
                .trim();
            if !stamp.is_empty() && !rest.is_empty() {
                return Some((stamp.to_string(), rest.to_string()));
            }
        }
    }

    for sep in [" | ", " — ", " - "] {
        if let Some((left, right)) = trimmed.split_once(sep) {
            let left = left.trim();
            let right = right.trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left.to_string(), right.to_string()));
            }
        }
    }

    Some((String::new(), trimmed.to_string()))
}

fn parse_kanban_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        if let Some((lane, text)) = parse_kanban_line(rest) {
            return Some((lane, text));
        }
    }

    if let Some((state, text)) = parse_checklist_line(trimmed) {
        let lane = match state {
            ChecklistState::Pending => "To Do",
            ChecklistState::InProgress => "Doing",
            ChecklistState::Done => "Done",
            ChecklistState::Note => "Notes",
        };
        return Some((lane.to_string(), text));
    }

    if trimmed.starts_with('[') {
        if let Some(end) = trimmed.find(']') {
            let lane = canonical_lane_name(&trimmed[1..end]);
            let rest = trimmed[end + 1..]
                .trim_start_matches([' ', '-', ':', '|'])
                .trim();
            if !rest.is_empty() {
                return Some((lane, rest.to_string()));
            }
        }
    }

    if let Some((left, right)) = trimmed.split_once(" | ") {
        let lane = canonical_lane_name(left);
        let text = right.trim();
        if !text.is_empty() {
            return Some((lane, text.to_string()));
        }
    }

    if let Some((left, right)) = trimmed.split_once(": ") {
        let left = left.trim();
        let right = right.trim();
        if !left.is_empty() && !right.is_empty() && left.len() <= 24 {
            return Some((canonical_lane_name(left), right.to_string()));
        }
    }

    Some(("Inbox".to_string(), trimmed.to_string()))
}

fn parse_kanban_content(
    content: &str,
    preferred_lanes: &[String],
) -> (Vec<String>, Vec<KanbanCard>) {
    let mut lanes = Vec::new();
    let mut seen_lanes = HashSet::new();
    for lane in preferred_lanes {
        let lane = canonical_lane_name(lane);
        if seen_lanes.insert(lane.clone()) {
            lanes.push(lane);
        }
    }

    let mut cards = Vec::new();
    for (lane, text) in content.lines().filter_map(parse_kanban_line) {
        let lane = canonical_lane_name(&lane);
        if seen_lanes.insert(lane.clone()) {
            lanes.push(lane.clone());
        }
        cards.push(KanbanCard { lane, text });
    }

    (lanes, cards)
}

fn normalize_dependency_segments(line: &str) -> Vec<String> {
    line.replace("=>", "->")
        .replace('→', "->")
        .split("->")
        .map(|part| {
            part.trim()
                .trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
}

fn parse_dependency_graph_content(content: &str) -> Vec<DependencyNodeView> {
    let mut node_order = Vec::new();
    let mut seen_nodes = HashSet::new();
    let mut edges = Vec::new();
    let mut seen_edges = HashSet::new();

    let mut remember_node = |node: String| {
        if seen_nodes.insert(node.clone()) {
            node_order.push(node);
        }
    };

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let segments = normalize_dependency_segments(trimmed);
        if segments.len() >= 2 {
            for segment in &segments {
                remember_node(segment.clone());
            }
            for pair in segments.windows(2) {
                let edge = (pair[0].clone(), pair[1].clone());
                let key = format!("{}->{}", edge.0, edge.1);
                if seen_edges.insert(key) {
                    edges.push(edge);
                }
            }
        } else if let Some(node) = segments.first() {
            remember_node(node.clone());
        } else {
            remember_node(trimmed.to_string());
        }
    }

    if node_order.is_empty() {
        return Vec::new();
    }

    let mut depends_on: HashMap<String, Vec<String>> = HashMap::new();
    let mut unlocks: HashMap<String, Vec<String>> = HashMap::new();
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut levels: HashMap<String, usize> = HashMap::new();

    for node in &node_order {
        indegree.insert(node.clone(), 0);
    }

    for (from, to) in &edges {
        unlocks.entry(from.clone()).or_default().push(to.clone());
        depends_on.entry(to.clone()).or_default().push(from.clone());
        *indegree.entry(to.clone()).or_default() += 1;
        indegree.entry(from.clone()).or_default();
    }

    let mut queue = VecDeque::new();
    for node in &node_order {
        if indegree.get(node).copied().unwrap_or(0) == 0 {
            queue.push_back(node.clone());
            levels.entry(node.clone()).or_insert(0);
        }
    }

    let mut processed = HashSet::new();
    while let Some(node) = queue.pop_front() {
        if !processed.insert(node.clone()) {
            continue;
        }
        let node_level = levels.get(&node).copied().unwrap_or(0);
        if let Some(children) = unlocks.get(&node) {
            for child in children {
                let next_level = node_level + 1;
                if next_level > levels.get(child).copied().unwrap_or(0) {
                    levels.insert(child.clone(), next_level);
                }
                if let Some(entry) = indegree.get_mut(child) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
    }

    if processed.len() < node_order.len() {
        let fallback_stage = levels.values().copied().max().unwrap_or(0) + 1;
        for node in &node_order {
            if !processed.contains(node) {
                levels.entry(node.clone()).or_insert(fallback_stage);
            }
        }
    }

    node_order
        .into_iter()
        .map(|name| DependencyNodeView {
            stage: levels.get(&name).copied().unwrap_or(0),
            depends_on: depends_on.remove(&name).unwrap_or_default(),
            unlocks: unlocks.remove(&name).unwrap_or_default(),
            name,
        })
        .collect()
}

fn text_matches_filter(haystack: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return true;
    }
    let haystack = haystack.to_ascii_lowercase();
    trimmed
        .split_whitespace()
        .all(|term| haystack.contains(&term.to_ascii_lowercase()))
}

fn module_block_filter_query(
    ui: &mut egui::Ui,
    module_id: &str,
    ui_id: &str,
    searchable: bool,
    item_count: usize,
    custom_placeholder: &str,
    default_placeholder: &str,
    presets: &[ModuleUiFilterPreset],
) -> String {
    if !searchable {
        return String::new();
    }

    let filter_id = egui::Id::new(("module_surface_filter", module_id, ui_id));
    let mut query = ui
        .ctx()
        .data_mut(|data| data.get_temp::<String>(filter_id))
        .unwrap_or_default();
    let should_show = item_count > 4 || !query.trim().is_empty() || !presets.is_empty();
    if !should_show {
        return query;
    }

    let placeholder = if custom_placeholder.trim().is_empty() {
        default_placeholder.to_string()
    } else {
        custom_placeholder.trim().to_string()
    };

    if !presets.is_empty() {
        ui.horizontal_wrapped(|ui| {
            let all_selected = query.trim().is_empty();
            if ui.selectable_label(all_selected, "All").clicked() {
                query.clear();
            }
            for preset in presets {
                let label = preset.label.trim();
                if label.is_empty() {
                    continue;
                }
                let selected = query.trim().eq_ignore_ascii_case(preset.query.trim());
                if ui.selectable_label(selected, label).clicked() {
                    query = preset.query.trim().to_string();
                }
            }
        });
        ui.add_space(4.0);
    }

    ui.horizontal(|ui| {
        ui.small("Filter");
        ui.add(
            egui::TextEdit::singleline(&mut query)
                .desired_width(180.0)
                .hint_text(placeholder),
        );
        if !query.trim().is_empty() && ui.small_button("Clear").clicked() {
            query.clear();
        }
    });
    ui.add_space(6.0);

    ui.ctx()
        .data_mut(|data| data.insert_temp(filter_id, query.clone()));
    query
}

fn module_layout_preset_selection(
    ui: &mut egui::Ui,
    module_id: &str,
    ui_id: &str,
    presets: &[ModuleUiViewPreset],
) -> Option<usize> {
    if presets.is_empty() {
        return None;
    }

    let preset_id = egui::Id::new(("module_layout_preset", module_id, ui_id));
    let mut active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<usize>(preset_id))
        .unwrap_or(0);
    if active > presets.len() {
        active = 0;
    }

    ui.horizontal_wrapped(|ui| {
        if ui.selectable_label(active == 0, "Default").clicked() {
            active = 0;
        }
        for (idx, preset) in presets.iter().enumerate() {
            let label = preset.label.trim();
            if label.is_empty() {
                continue;
            }
            if ui.selectable_label(active == idx + 1, label).clicked() {
                active = idx + 1;
            }
        }
    });
    ui.add_space(6.0);
    ui.ctx()
        .data_mut(|data| data.insert_temp(preset_id, active));

    active.checked_sub(1)
}

fn module_layout_visible_panes<'a>(
    panes: &'a [ResolvedModuleUiPane],
    preset: Option<&ModuleUiViewPreset>,
) -> Vec<&'a ResolvedModuleUiPane> {
    let Some(preset) = preset else {
        return panes.iter().collect();
    };

    let pane_ids = preset
        .pane_ids
        .iter()
        .map(|pane_id| pane_id.trim())
        .filter(|pane_id| !pane_id.is_empty())
        .collect::<Vec<_>>();
    if pane_ids.is_empty() {
        return panes.iter().collect();
    }

    let mut visible = Vec::new();
    let mut seen = HashSet::new();
    for pane_id in pane_ids {
        if let Some(pane) = panes.iter().find(|pane| pane.id == pane_id) {
            if seen.insert(pane.id.clone()) {
                visible.push(pane);
            }
        }
    }

    if visible.is_empty() {
        panes.iter().collect()
    } else {
        visible
    }
}

fn normalized_block_id(block: &ModuleUiBlock, fallback: &str) -> String {
    if !block.id.trim().is_empty() {
        block.id.trim().to_string()
    } else if !block.title.trim().is_empty() {
        format!(
            "{fallback}:{}",
            block.title.trim().to_lowercase().replace(' ', "_")
        )
    } else if !block.field.trim().is_empty() {
        format!("{fallback}:{}", block.field.trim())
    } else {
        fallback.to_string()
    }
}

fn resolve_module_ui_container_blocks(
    spec: &ModuleUiSpec,
    blocks_cfg: &[ModuleUiBlock],
    field_ids: &[String],
    used: &mut HashSet<String>,
    container_key: &str,
) -> Vec<ResolvedModuleUiBlock> {
    let mut blocks = Vec::new();

    for (idx, block) in blocks_cfg.iter().enumerate() {
        let block_key = format!("{container_key}:block_{idx}");
        if let Some(block) = resolve_module_ui_block(spec, block, used, &block_key) {
            blocks.push(block);
        }
    }

    for field_id in field_ids {
        if let Some(field) = spec
            .fields
            .iter()
            .find(|field| field.id.trim() == field_id.trim())
        {
            if used.insert(field.id.clone()) {
                blocks.push(ResolvedModuleUiBlock::Field(field.clone()));
            }
        }
    }

    blocks
}

fn resolve_module_ui_panes(
    spec: &ModuleUiSpec,
    panes: &[ModuleUiPane],
    used: &mut HashSet<String>,
    container_key: &str,
) -> Vec<ResolvedModuleUiPane> {
    let mut resolved = Vec::new();

    for (idx, pane) in panes.iter().enumerate() {
        let pane_key = if !pane.id.trim().is_empty() {
            format!("{container_key}:{}", pane.id.trim())
        } else {
            format!("{container_key}:pane_{idx}")
        };
        let blocks =
            resolve_module_ui_container_blocks(spec, &pane.blocks, &pane.fields, used, &pane_key);
        if blocks.is_empty() {
            continue;
        }
        resolved.push(ResolvedModuleUiPane {
            id: if !pane.id.trim().is_empty() {
                pane.id.trim().to_string()
            } else {
                format!("pane_{idx}")
            },
            title: pane.title.clone(),
            description: pane.description.clone(),
            summary: pane.summary.clone(),
            summary_field: pane.summary_field.clone(),
            blocks,
            weight: pane.weight.unwrap_or(1.0).max(0.1),
            default_open: pane.default_open.unwrap_or(false),
        });
    }

    resolved
}

fn render_markdownish(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code_lines = Vec::<String>::new();

    let flush_code = |ui: &mut egui::Ui, code_lines: &mut Vec<String>| {
        if code_lines.is_empty() {
            return;
        }
        let code = code_lines.join("\n");
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(code).monospace()).wrap());
            });
        code_lines.clear();
    };

    for raw_line in text.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if trimmed.starts_with("```") {
            if in_code {
                flush_code(ui, &mut code_lines);
            }
            in_code = !in_code;
            continue;
        }

        if in_code {
            code_lines.push(line.to_string());
            continue;
        }

        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("### ") {
            ui.label(egui::RichText::new(rest).strong());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            ui.label(egui::RichText::new(rest).heading());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            ui.label(egui::RichText::new(rest).heading().strong());
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("> ") {
            ui.label(egui::RichText::new(rest).italics());
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            ui.label(format!("• {rest}"));
            continue;
        }
        if let Some((prefix, rest)) = trimmed.split_once(". ") {
            if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
                ui.label(format!("{prefix}. {rest}"));
                continue;
            }
        }

        ui.label(trimmed);
    }

    if in_code {
        flush_code(ui, &mut code_lines);
    }
}

fn resolve_module_ui_block(
    spec: &ModuleUiSpec,
    block: &ModuleUiBlock,
    used: &mut HashSet<String>,
    container_key: &str,
) -> Option<ResolvedModuleUiBlock> {
    let kind = block.kind.trim().to_lowercase();
    match kind.as_str() {
        "" | "field" => {
            let field_id = block.field.trim();
            let field = spec
                .fields
                .iter()
                .find(|field| field.id.trim() == field_id)?;
            if used.insert(field.id.clone()) {
                Some(ResolvedModuleUiBlock::Field(field.clone()))
            } else {
                None
            }
        }
        "text" => {
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Text { title, text })
            }
        }
        "markdown" => {
            let field_id = block.field.trim().to_string();
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && field_id.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Markdown {
                    title,
                    text,
                    field_id,
                    empty: block.empty.trim().to_string(),
                })
            }
        }
        "callout" => {
            let text = block.text.trim().to_string();
            let title = block.title.trim().to_string();
            if text.is_empty() && title.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Callout {
                    title,
                    text,
                    tone: block.tone.trim().to_string(),
                })
            }
        }
        "stat" => {
            let field_id = block.field.trim().to_string();
            if field_id.is_empty() {
                return None;
            }
            Some(ResolvedModuleUiBlock::Stat {
                label: if block.label.trim().is_empty() {
                    module_field_label(spec, &field_id)
                } else {
                    block.label.trim().to_string()
                },
                field_id,
                empty: block.empty.trim().to_string(),
            })
        }
        "actions" => {
            if block.actions.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Actions {
                    actions: block.actions.clone(),
                })
            }
        }
        "progress" => {
            let field_id = block.field.trim().to_string();
            if field_id.is_empty() {
                return None;
            }
            let field_spec = module_field_spec(spec, &field_id);
            Some(ResolvedModuleUiBlock::Progress {
                label: if block.label.trim().is_empty() {
                    module_field_label(spec, &field_id)
                } else {
                    block.label.trim().to_string()
                },
                field_id,
                min: block.min.or_else(|| field_spec.and_then(|field| field.min)),
                max: block.max.or_else(|| field_spec.and_then(|field| field.max)),
                empty: block.empty.trim().to_string(),
            })
        }
        "record" | "key_value" | "kv" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:record"));
            let mut field_ids = Vec::new();
            for field_id in &block.fields {
                let trimmed = field_id.trim();
                if !trimmed.is_empty() && module_field_spec(spec, trimmed).is_some() {
                    field_ids.push(trimmed.to_string());
                }
            }
            if field_ids.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Record {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_ids,
                    empty: block.empty.trim().to_string(),
                })
            }
        }
        "checklist" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:checklist"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Checklist {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(12).clamp(1, 100),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "timeline" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:timeline"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Timeline {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(10).clamp(1, 100),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "kanban" | "board" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:kanban"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Kanban {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(18).clamp(1, 200),
                    lanes: block
                        .lanes
                        .iter()
                        .map(|lane| lane.trim().to_string())
                        .filter(|lane| !lane.is_empty())
                        .collect(),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "table" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:table"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Table {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(8).clamp(1, 50),
                    has_header: block.has_header.unwrap_or(true),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "dependency_graph" | "graph" | "dependencies" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:dependency_graph"));
            let field_id = block.field.trim().to_string();
            let path = block.path.trim().to_string();
            if field_id.is_empty() && path.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::DependencyGraph {
                    title: block.title.trim().to_string(),
                    ui_id,
                    field_id,
                    path,
                    empty: block.empty.trim().to_string(),
                    max_rows: block.max_rows.unwrap_or(16).clamp(1, 200),
                    searchable: block.searchable.unwrap_or(true),
                    filter_placeholder: block.filter_placeholder.trim().to_string(),
                    filter_presets: block.filter_presets.clone(),
                })
            }
        }
        "bar_chart" | "bars" => {
            let mut field_ids = Vec::new();
            for field_id in &block.fields {
                let trimmed = field_id.trim();
                if !trimmed.is_empty() && module_field_spec(spec, trimmed).is_some() {
                    field_ids.push(trimmed.to_string());
                }
            }
            if field_ids.is_empty() {
                return None;
            }
            Some(ResolvedModuleUiBlock::BarChart {
                title: block.title.trim().to_string(),
                field_ids,
                min: block.min,
                max: block.max,
                empty: block.empty.trim().to_string(),
            })
        }
        "tabs" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:tabs"));
            let panes = resolve_module_ui_panes(spec, &block.tabs, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Tabs {
                    title: block.title.trim().to_string(),
                    ui_id,
                    panes,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "split" | "columns" => {
            let ui_id = normalized_block_id(block, &format!("{container_key}:split"));
            let panes = resolve_module_ui_panes(spec, &block.columns, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Split {
                    title: block.title.trim().to_string(),
                    ui_id,
                    direction: if block.direction.trim().is_empty() {
                        "horizontal".to_string()
                    } else {
                        block.direction.trim().to_string()
                    },
                    panes,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "accordion" | "inspector" => {
            let inspector_style = kind == "inspector";
            let ui_id = normalized_block_id(
                block,
                &format!(
                    "{container_key}:{}",
                    if inspector_style {
                        "inspector"
                    } else {
                        "accordion"
                    }
                ),
            );
            let panes = resolve_module_ui_panes(spec, &block.panes, used, &ui_id);
            if panes.is_empty() {
                None
            } else {
                Some(ResolvedModuleUiBlock::Accordion {
                    title: block.title.trim().to_string(),
                    ui_id,
                    panes,
                    inspector_style,
                    view_presets: block.view_presets.clone(),
                })
            }
        }
        "file_list" => Some(ResolvedModuleUiBlock::FileList {
            title: block.title.trim().to_string(),
            ui_id: normalized_block_id(block, &format!("{container_key}:file_list")),
            path: block.path.trim().to_string(),
            empty: block.empty.trim().to_string(),
            max_entries: block.max_entries.unwrap_or(8).clamp(1, 50),
            searchable: block.searchable.unwrap_or(true),
            filter_placeholder: block.filter_placeholder.trim().to_string(),
            filter_presets: block.filter_presets.clone(),
        }),
        "artifact_preview" | "file_preview" => Some(ResolvedModuleUiBlock::ArtifactPreview {
            title: block.title.trim().to_string(),
            path: block.path.trim().to_string(),
            field_id: block.field.trim().to_string(),
            empty: block.empty.trim().to_string(),
            max_lines: block.max_lines.unwrap_or(16).clamp(4, 80),
        }),
        "separator" => Some(ResolvedModuleUiBlock::Separator),
        "spacer" => Some(ResolvedModuleUiBlock::Spacer(
            block.points.unwrap_or(8.0).clamp(0.0, 64.0),
        )),
        _ => None,
    }
}

fn resolve_module_ui_sections(spec: &ModuleUiSpec) -> Vec<ResolvedModuleUiSection> {
    let mut resolved = Vec::new();
    let mut used = HashSet::new();

    if !spec.sections.is_empty() {
        for (idx, section) in spec.sections.iter().enumerate() {
            let section_key = if !section.id.trim().is_empty() {
                format!("section:{}", section.id.trim())
            } else {
                format!("section_{idx}")
            };
            let mut blocks = resolve_module_ui_container_blocks(
                spec,
                &section.blocks,
                &section.fields,
                &mut used,
                &section_key,
            );

            if !section.id.trim().is_empty() {
                for field in &spec.fields {
                    if !used.contains(&field.id) && field.section.trim() == section.id.trim() {
                        used.insert(field.id.clone());
                        blocks.push(ResolvedModuleUiBlock::Field(field.clone()));
                    }
                }
            }

            if !blocks.is_empty() {
                resolved.push(ResolvedModuleUiSection {
                    title: section.title.clone(),
                    description: section.description.clone(),
                    blocks,
                    sidebar: section.sidebar,
                });
            }
        }
    }

    let mut grouped_keys = Vec::<String>::new();
    let mut grouped = HashMap::<String, Vec<ModuleUiField>>::new();
    for field in &spec.fields {
        if used.contains(&field.id) {
            continue;
        }
        let key = if !field.section.trim().is_empty() {
            field.section.trim().to_string()
        } else {
            "workspace".to_string()
        };
        if !grouped.contains_key(&key) {
            grouped_keys.push(key.clone());
        }
        grouped.entry(key).or_default().push(field.clone());
    }

    for key in grouped_keys {
        if let Some(fields) = grouped.remove(&key) {
            if !fields.is_empty() {
                resolved.push(ResolvedModuleUiSection {
                    title: humanize_section_id(&key),
                    description: String::new(),
                    blocks: fields
                        .into_iter()
                        .map(ResolvedModuleUiBlock::Field)
                        .collect(),
                    sidebar: false,
                });
            }
        }
    }

    resolved
}

fn field_str_mut<'a>(
    values: &'a mut HashMap<String, ModuleFieldValue>,
    id: &str,
) -> &'a mut String {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Str(String::new()));
    match v {
        ModuleFieldValue::Str(s) => s,
        _ => {
            *v = ModuleFieldValue::Str(String::new());
            match v {
                ModuleFieldValue::Str(s) => s,
                _ => unreachable!(),
            }
        }
    }
}

fn field_bool_mut<'a>(values: &'a mut HashMap<String, ModuleFieldValue>, id: &str) -> &'a mut bool {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Bool(false));
    match v {
        ModuleFieldValue::Bool(b) => b,
        _ => {
            *v = ModuleFieldValue::Bool(false);
            match v {
                ModuleFieldValue::Bool(b) => b,
                _ => unreachable!(),
            }
        }
    }
}

fn field_num_mut<'a>(
    values: &'a mut HashMap<String, ModuleFieldValue>,
    id: &str,
    default: f64,
) -> &'a mut f64 {
    let v = values
        .entry(id.to_string())
        .or_insert(ModuleFieldValue::Num(default));
    match v {
        ModuleFieldValue::Num(n) => n,
        _ => {
            *v = ModuleFieldValue::Num(default);
            match v {
                ModuleFieldValue::Num(n) => n,
                _ => unreachable!(),
            }
        }
    }
}

fn render_module_field(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    module_id: &str,
    f: &ModuleUiField,
) {
    let id = f.id.trim();
    if id.is_empty() {
        return;
    }

    let kind = f.kind.trim().to_lowercase();
    ui.vertical(|ui| {
        if kind == "bool" {
            let value = field_bool_mut(&mut st.values, id);
            ui.checkbox(value, &f.label);
            if !f.help.trim().is_empty() {
                ui.small(f.help.clone());
            }
            return;
        }

        ui.label(egui::RichText::new(&f.label).strong());
        if !f.help.trim().is_empty() {
            ui.small(f.help.clone());
        }

        match kind.as_str() {
            "number" => {
                let value = field_num_mut(&mut st.values, id, f.min.unwrap_or(0.0));
                if let (Some(min), Some(max)) = (f.min, f.max) {
                    ui.add(egui::Slider::new(value, min..=max).show_value(true));
                } else {
                    ui.add(egui::DragValue::new(value).speed(0.1));
                }
            }
            "choice" => {
                let value = field_str_mut(&mut st.values, id);
                let compact = f.options.len() <= 4 && f.options.iter().all(|opt| opt.len() <= 18);
                if compact {
                    ui.horizontal_wrapped(|ui| {
                        for opt in &f.options {
                            let selected = value == opt;
                            if ui.selectable_label(selected, opt).clicked() {
                                value.clear();
                                value.push_str(opt);
                            }
                        }
                    });
                } else {
                    let selected = if value.trim().is_empty() {
                        "(none)".to_string()
                    } else {
                        value.clone()
                    };
                    egui::ComboBox::from_id_salt((module_id, id))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for opt in &f.options {
                                ui.selectable_value(value, opt.clone(), opt.clone());
                            }
                        });
                }
            }
            "singleline" => {
                let value = field_str_mut(&mut st.values, id);
                ui.add_sized(
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(value).hint_text(&f.placeholder),
                );
            }
            _ => {
                let value = field_str_mut(&mut st.values, id);
                let rows = f.rows.unwrap_or(4).clamp(2, 24);
                ui.add(
                    egui::TextEdit::multiline(value)
                        .desired_rows(rows)
                        .hint_text(&f.placeholder),
                );
            }
        }
    });
}

fn render_module_builtin_actions(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    module_dir: &Path,
    state_path: &Path,
    actions: &[String],
) {
    ui.horizontal_wrapped(|ui| {
        for action in actions {
            let normalized = action.trim().to_lowercase();
            match normalized.as_str() {
                "save" => {
                    if ui.button("Save").clicked() {
                        st.save();
                    }
                }
                "reload" => {
                    if ui.button("Reload").clicked() {
                        st.reload();
                    }
                }
                "open_folder" | "open_module" | "open_module_folder" => {
                    if ui.button("Open Folder").clicked() {
                        open_path_in_explorer(module_dir);
                    }
                }
                "open_readme" => {
                    let path = module_dir.join("README.md");
                    if path.is_file() && ui.button("README").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_manual" => {
                    let path = module_dir.join("USER_MANUAL.md");
                    if path.is_file() && ui.button("Manual").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_handshake" => {
                    let path = module_dir.join("HANDSHAKE.md");
                    if path.is_file() && ui.button("Handshake").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                "open_state" => {
                    if state_path.is_file() && ui.button("State JSON").clicked() {
                        open_path_in_explorer(state_path);
                    }
                }
                "open_manifest" => {
                    let path = module_dir.join("manifest.json");
                    if path.is_file() && ui.button("Manifest").clicked() {
                        open_path_in_explorer(&path);
                    }
                }
                _ => {}
            }
        }
    });
}

fn render_module_file_list(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    relative_path: &str,
    empty: &str,
    max_entries: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(target_dir) = module_surface_path(module_dir, relative_path) else {
        ui.small(
            "This file list path is not allowed. Use a relative path inside the module folder.",
        );
        return;
    };

    if !target_dir.exists() {
        ui.small(if empty.trim().is_empty() {
            "Nothing here yet."
        } else {
            empty
        });
        return;
    }

    let Ok(read_dir) = std::fs::read_dir(&target_dir) else {
        ui.small("Couldn't read this folder.");
        return;
    };

    let mut entries = read_dir.flatten().collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        let a_dir = a.path().is_dir();
        let b_dir = b.path().is_dir();
        b_dir.cmp(&a_dir).then_with(|| {
            a.file_name()
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.file_name().to_string_lossy().to_lowercase())
        })
    });

    if entries.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "Nothing here yet."
        } else {
            empty
        });
        return;
    }

    let total_entries = entries.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_entries,
        filter_placeholder,
        "Search files...",
        filter_presets,
    );
    let filtered_entries = entries
        .into_iter()
        .filter(|entry| {
            if query.trim().is_empty() {
                true
            } else {
                text_matches_filter(&entry.file_name().to_string_lossy(), &query)
            }
        })
        .take(max_entries)
        .collect::<Vec<_>>();

    if filtered_entries.is_empty() {
        ui.small("No files match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!(
            "Showing {}/{}",
            filtered_entries.len(),
            total_entries
        ));
        ui.add_space(4.0);
    }

    for entry in filtered_entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let label = if path.is_dir() {
            format!("📁 {name}")
        } else {
            format!("📄 {name}")
        };
        if ui.button(label).clicked() {
            open_path_in_explorer(&path);
        }
    }
}

fn render_module_artifact_preview(
    ui: &mut egui::Ui,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    explicit_path: &str,
    field_id: &str,
    empty: &str,
    max_lines: usize,
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let candidates = resolve_module_block_paths(module_dir, values, explicit_path, field_id);
    let Some(path) = candidates.into_iter().find(|path| path.exists()) else {
        ui.small(if empty.trim().is_empty() {
            "No artifact available yet."
        } else {
            empty
        });
        return;
    };

    ui.horizontal_wrapped(|ui| {
        ui.small(format!("Artifact: {}", path.display()));
        if ui.button("Open").clicked() {
            open_path_in_explorer(&path);
        }
    });

    if path.is_dir() {
        render_module_file_list(
            ui,
            "",
            &path,
            "",
            "artifact_preview_dir",
            ".",
            "This folder is empty.",
            max_lines.clamp(1, 20),
            true,
            "",
            &[],
        );
        return;
    }

    let metadata = std::fs::metadata(&path).ok();
    if let Some(metadata) = metadata {
        ui.small(format!("Size: {} bytes", metadata.len()));
    }

    if looks_like_text_file(&path) {
        if let Some(preview) = preview_text_file(&path, max_lines) {
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                render_markdownish(ui, &preview);
            } else {
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(8.0))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(preview).monospace()).wrap());
                    });
            }
        } else {
            ui.small("Couldn't preview this file.");
        }
    } else {
        ui.small("Preview unavailable for this file type. Use Open to inspect it directly.");
    }
}

fn render_module_record_block(
    ui: &mut egui::Ui,
    spec: &ModuleUiSpec,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_ids: &[String],
    empty: &str,
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let rows = field_ids
        .iter()
        .filter_map(|field_id| {
            module_field_value_as_text(values, field_id)
                .map(|value| (module_field_label(spec, field_id), value))
        })
        .collect::<Vec<_>>();

    if rows.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No values to show yet."
        } else {
            empty
        });
        return;
    }

    egui::Grid::new(("module_record", ui_id))
        .num_columns(2)
        .spacing([12.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.small(label);
                ui.label(value);
                ui.end_row();
            }
        });
}

fn render_module_table_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    has_header: bool,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_table_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No table data available yet."
        } else {
            empty
        });
        return;
    };
    let Some((header, rows)) = parse_lightweight_table(&content, has_header) else {
        ui.small(if empty.trim().is_empty() {
            "Couldn't parse table data. Use CSV, TSV, semicolon, or pipe-delimited rows."
        } else {
            empty
        });
        return;
    };
    if rows.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No table rows available yet."
        } else {
            empty
        });
        return;
    }

    let total_rows = rows.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_rows,
        filter_placeholder,
        "Filter rows...",
        filter_presets,
    );
    let filtered_rows = rows
        .into_iter()
        .filter(|row| {
            if query.trim().is_empty() {
                true
            } else {
                text_matches_filter(&row.join(" "), &query)
            }
        })
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_rows.is_empty() {
        ui.small("No table rows match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_rows.len(), total_rows));
        ui.add_space(4.0);
    }

    egui::ScrollArea::horizontal()
        .id_salt(("module_table_scroll", ui_id))
        .show(ui, |ui| {
            egui::Grid::new(("module_table", ui_id))
                .num_columns(header.len().max(1))
                .spacing([12.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    for column in &header {
                        ui.label(egui::RichText::new(column).strong());
                    }
                    ui.end_row();

                    for row in filtered_rows {
                        for cell in row {
                            ui.label(cell);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn render_module_checklist_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No checklist available yet."
        } else {
            empty
        });
        return;
    };

    let items = content
        .lines()
        .filter_map(parse_checklist_line)
        .collect::<Vec<_>>();

    if items.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No checklist items available yet."
        } else {
            empty
        });
        return;
    }

    let total_items = items.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_items,
        filter_placeholder,
        "Filter checklist...",
        filter_presets,
    );
    let filtered_items = items
        .into_iter()
        .filter(|(_, text)| text_matches_filter(text, &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_items.is_empty() {
        ui.small("No checklist items match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_items.len(), total_items));
        ui.add_space(4.0);
    }

    for (state, text) in filtered_items {
        ui.horizontal_wrapped(|ui| {
            let (marker, color) = match state {
                ChecklistState::Done => ("[x]", egui::Color32::from_rgb(20, 120, 60)),
                ChecklistState::InProgress => ("[~]", egui::Color32::from_rgb(180, 110, 10)),
                ChecklistState::Pending => ("[ ]", egui::Color32::from_gray(120)),
                ChecklistState::Note => ("•", egui::Color32::from_gray(120)),
            };
            ui.colored_label(color, marker);
            ui.label(text);
        });
    }
}

fn render_module_timeline_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No timeline available yet."
        } else {
            empty
        });
        return;
    };

    let items = content
        .lines()
        .filter_map(parse_timeline_line)
        .collect::<Vec<_>>();

    if items.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No timeline entries available yet."
        } else {
            empty
        });
        return;
    }

    let total_items = items.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_items,
        filter_placeholder,
        "Filter timeline...",
        filter_presets,
    );
    let filtered_items = items
        .into_iter()
        .filter(|(stamp, text)| text_matches_filter(&format!("{stamp} {text}"), &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_items.is_empty() {
        ui.small("No timeline entries match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_items.len(), total_items));
        ui.add_space(4.0);
    }

    let total = filtered_items.len();
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            for (idx, (stamp, text)) in filtered_items.into_iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(30, 80, 180), "•");
                    if !stamp.trim().is_empty() {
                        ui.small(egui::RichText::new(stamp).strong());
                    }
                    ui.label(text);
                });
                if idx + 1 < total {
                    ui.add_space(4.0);
                }
            }
        });
}

fn render_module_kanban_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    lanes: &[String],
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No kanban board available yet."
        } else {
            empty
        });
        return;
    };

    let (lane_order, cards) = parse_kanban_content(&content, lanes);
    if cards.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No kanban cards available yet."
        } else {
            empty
        });
        return;
    }

    let total_cards = cards.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_cards,
        filter_placeholder,
        "Filter board...",
        filter_presets,
    );
    let filtered_cards = cards
        .into_iter()
        .filter(|card| text_matches_filter(&format!("{} {}", card.lane, card.text), &query))
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_cards.is_empty() {
        ui.small("No kanban cards match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_cards.len(), total_cards));
        ui.add_space(4.0);
    }

    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (idx, lane) in lane_order.iter().enumerate() {
                    let lane_cards = filtered_cards
                        .iter()
                        .filter(|card| card.lane == *lane)
                        .collect::<Vec<_>>();
                    let accent = lane_accent_color(lane);

                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.set_min_width(220.0);
                            ui.colored_label(
                                accent,
                                egui::RichText::new(format!("{} ({})", lane, lane_cards.len()))
                                    .strong(),
                            );
                            ui.add_space(6.0);

                            if lane_cards.is_empty() {
                                ui.small("No cards in this lane yet.");
                            } else {
                                for (card_idx, card) in lane_cards.iter().enumerate() {
                                    egui::Frame::group(ui.style())
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.label(card.text.clone());
                                        });
                                    if card_idx + 1 < lane_cards.len() {
                                        ui.add_space(6.0);
                                    }
                                }
                            }
                        });

                    if idx + 1 < lane_order.len() {
                        ui.add_space(8.0);
                    }
                }
            });
        });
}

fn render_module_dependency_graph_block(
    ui: &mut egui::Ui,
    module_id: &str,
    module_dir: &Path,
    values: &HashMap<String, ModuleFieldValue>,
    title: &str,
    ui_id: &str,
    field_id: &str,
    path: &str,
    empty: &str,
    max_rows: usize,
    searchable: bool,
    filter_placeholder: &str,
    filter_presets: &[ModuleUiFilterPreset],
) {
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
    }

    let Some(content) = read_module_text_source(module_dir, values, path, field_id) else {
        ui.small(if empty.trim().is_empty() {
            "No dependency graph available yet."
        } else {
            empty
        });
        return;
    };

    let nodes = parse_dependency_graph_content(&content);
    if nodes.is_empty() {
        ui.small(if empty.trim().is_empty() {
            "No dependency graph nodes available yet."
        } else {
            empty
        });
        return;
    }

    let total_nodes = nodes.len();
    let query = module_block_filter_query(
        ui,
        module_id,
        ui_id,
        searchable,
        total_nodes,
        filter_placeholder,
        "Filter graph...",
        filter_presets,
    );
    let filtered_nodes = nodes
        .into_iter()
        .filter(|node| {
            text_matches_filter(
                &format!(
                    "{} {} {}",
                    node.name,
                    node.depends_on.join(" "),
                    node.unlocks.join(" ")
                ),
                &query,
            )
        })
        .take(max_rows)
        .collect::<Vec<_>>();

    if filtered_nodes.is_empty() {
        ui.small("No dependency graph nodes match the current filter.");
        return;
    }

    if !query.trim().is_empty() {
        ui.small(format!("Showing {}/{}", filtered_nodes.len(), total_nodes));
        ui.add_space(4.0);
    }

    let max_stage = filtered_nodes
        .iter()
        .map(|node| node.stage)
        .max()
        .unwrap_or(0);
    egui::ScrollArea::horizontal()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for stage in 0..=max_stage {
                    let stage_nodes = filtered_nodes
                        .iter()
                        .filter(|node| node.stage == stage)
                        .collect::<Vec<_>>();

                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ui.set_min_width(230.0);
                            ui.label(egui::RichText::new(format!("Stage {}", stage + 1)).strong());
                            ui.add_space(6.0);

                            if stage_nodes.is_empty() {
                                ui.small("No nodes in this stage.");
                            } else {
                                for (idx, node) in stage_nodes.iter().enumerate() {
                                    egui::Frame::group(ui.style())
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.label(egui::RichText::new(&node.name).strong());
                                            if !node.depends_on.is_empty() {
                                                ui.small(format!(
                                                    "Depends on: {}",
                                                    truncate_for_ui(
                                                        &node.depends_on.join(", "),
                                                        120
                                                    )
                                                ));
                                            }
                                            if !node.unlocks.is_empty() {
                                                ui.small(format!(
                                                    "Unblocks: {}",
                                                    truncate_for_ui(&node.unlocks.join(", "), 120)
                                                ));
                                            }
                                        });
                                    if idx + 1 < stage_nodes.len() {
                                        ui.add_space(6.0);
                                    }
                                }
                            }
                        });

                    if stage < max_stage {
                        ui.add_space(8.0);
                    }
                }
            });
        });
}

fn render_module_pane(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    pane: &ResolvedModuleUiPane,
    framed: bool,
) {
    let mut render_contents = |ui: &mut egui::Ui| {
        if !pane.title.trim().is_empty() {
            ui.label(egui::RichText::new(&pane.title).strong());
        }
        if !pane.description.trim().is_empty() {
            ui.small(pane.description.clone());
            ui.add_space(6.0);
        }
        for (idx, block) in pane.blocks.iter().enumerate() {
            render_module_block(ui, st, spec, module_id, module_dir, block);
            if idx + 1 < pane.blocks.len() {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
            }
        }
    };

    if framed {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, render_contents);
    } else {
        render_contents(ui);
    }
}

fn render_module_pane_summary(
    values: &HashMap<String, ModuleFieldValue>,
    pane: &ResolvedModuleUiPane,
) -> String {
    if !pane.summary_field.trim().is_empty() {
        if let Some(value) = module_field_value_as_text(values, &pane.summary_field) {
            return truncate_for_ui(&value.replace('\n', " "), 80);
        }
    }
    if !pane.summary.trim().is_empty() {
        return pane.summary.trim().to_string();
    }
    String::new()
}

fn render_module_tabs_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    panes: &[ResolvedModuleUiPane],
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let tabs_id = egui::Id::new(("module_surface_tabs", module_id, ui_id));
    let mut active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<usize>(tabs_id))
        .unwrap_or(0);
    if active >= visible_panes.len() {
        active = 0;
    }

    ui.horizontal_wrapped(|ui| {
        for (idx, pane) in visible_panes.iter().enumerate() {
            let selected = idx == active;
            if ui.selectable_label(selected, &pane.title).clicked() {
                active = idx;
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(tabs_id, active));
    ui.add_space(8.0);

    render_module_pane(
        ui,
        st,
        spec,
        module_id,
        module_dir,
        visible_panes[active],
        true,
    );
}

fn render_module_accordion_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    panes: &[ResolvedModuleUiPane],
    inspector_style: bool,
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let render_stack = |ui: &mut egui::Ui,
                        st: &mut ModuleFormState,
                        spec: &ModuleUiSpec,
                        module_id: &str,
                        module_dir: &Path| {
        for (idx, pane) in visible_panes.iter().enumerate() {
            let summary = render_module_pane_summary(&st.values, pane);
            let header = if summary.is_empty() {
                pane.title.clone()
            } else {
                format!("{} — {}", pane.title, summary)
            };
            egui::CollapsingHeader::new(header)
                .id_salt((module_id, ui_id, pane.id.as_str()))
                .default_open(pane.default_open || preset_idx.is_some())
                .show(ui, |ui| {
                    if !pane.description.trim().is_empty() {
                        ui.small(pane.description.clone());
                        ui.add_space(6.0);
                    }
                    for (block_idx, block) in pane.blocks.iter().enumerate() {
                        render_module_block(ui, st, spec, module_id, module_dir, block);
                        if block_idx + 1 < pane.blocks.len() {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                        }
                    }
                });

            if idx + 1 < visible_panes.len() {
                ui.add_space(if inspector_style { 4.0 } else { 8.0 });
            }
        }
    };

    if inspector_style {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8.0))
            .show(ui, |ui| {
                if !title.trim().is_empty() {
                    ui.label(egui::RichText::new(title).strong());
                    ui.add_space(4.0);
                }
                render_stack(ui, st, spec, module_id, module_dir);
            });
    } else {
        if !title.trim().is_empty() {
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(4.0);
        }
        render_stack(ui, st, spec, module_id, module_dir);
    }
}

fn render_module_split_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    title: &str,
    ui_id: &str,
    direction: &str,
    panes: &[ResolvedModuleUiPane],
    view_presets: &[ModuleUiViewPreset],
) {
    if panes.is_empty() {
        return;
    }
    if !title.trim().is_empty() {
        ui.label(egui::RichText::new(title).strong());
        ui.add_space(4.0);
    }

    let preset_idx = module_layout_preset_selection(ui, module_id, ui_id, view_presets);
    let visible_panes =
        module_layout_visible_panes(panes, preset_idx.and_then(|idx| view_presets.get(idx)));
    if visible_panes.is_empty() {
        ui.small("No panes available for this view.");
        return;
    }

    let horizontal =
        !direction.trim().eq_ignore_ascii_case("vertical") && ui.available_width() >= 720.0;
    if horizontal && visible_panes.len() > 1 {
        let total_width = ui.available_width();
        let spacing = ui.spacing().item_spacing.x;
        let total_spacing = spacing * (visible_panes.len().saturating_sub(1) as f32);
        let total_weight: f32 = visible_panes.iter().map(|pane| pane.weight.max(0.1)).sum();
        ui.horizontal_top(|ui| {
            for (idx, pane) in visible_panes.iter().enumerate() {
                let width = ((total_width - total_spacing).max(200.0)
                    * (pane.weight.max(0.1) / total_weight))
                    .max(180.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| render_module_pane(ui, st, spec, module_id, module_dir, pane, true),
                );
                if idx + 1 < visible_panes.len() {
                    ui.add_space(spacing);
                }
            }
        });
    } else {
        for (idx, pane) in visible_panes.iter().enumerate() {
            render_module_pane(ui, st, spec, module_id, module_dir, pane, true);
            if idx + 1 < visible_panes.len() {
                ui.add_space(8.0);
            }
        }
    }
}

fn render_module_block(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    block: &ResolvedModuleUiBlock,
) {
    match block {
        ResolvedModuleUiBlock::Field(field) => render_module_field(ui, st, module_id, field),
        ResolvedModuleUiBlock::Text { title, text } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }
            if !text.trim().is_empty() {
                ui.label(text.clone());
            }
        }
        ResolvedModuleUiBlock::Markdown {
            title,
            text,
            field_id,
            empty,
        } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }
            let content = if !field_id.trim().is_empty() {
                module_field_value_as_text(&st.values, field_id)
            } else if !text.trim().is_empty() {
                Some(text.clone())
            } else {
                None
            };

            if let Some(content) = content {
                render_markdownish(ui, &content);
            } else {
                ui.small(if empty.trim().is_empty() {
                    "Nothing to preview yet."
                } else {
                    empty
                });
            }
        }
        ResolvedModuleUiBlock::Callout { title, text, tone } => {
            let accent = match tone.trim().to_lowercase().as_str() {
                "success" => egui::Color32::from_rgb(20, 120, 60),
                "warning" | "warn" => egui::Color32::from_rgb(180, 110, 10),
                "error" | "danger" => egui::Color32::from_rgb(180, 40, 40),
                "info" => egui::Color32::from_rgb(30, 80, 180),
                _ => egui::Color32::from_gray(120),
            };
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    if !title.trim().is_empty() {
                        ui.colored_label(accent, egui::RichText::new(title).strong());
                    }
                    if !text.trim().is_empty() {
                        ui.label(text.clone());
                    }
                });
        }
        ResolvedModuleUiBlock::Stat {
            label,
            field_id,
            empty,
        } => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.small(label.clone());
                    let value =
                        module_field_value_as_text(&st.values, field_id).unwrap_or_else(|| {
                            if empty.trim().is_empty() {
                                "(empty)".to_string()
                            } else {
                                empty.clone()
                            }
                        });
                    ui.label(egui::RichText::new(value).strong());
                });
        }
        ResolvedModuleUiBlock::Actions { actions } => {
            let state_path = st.state_path.clone();
            render_module_builtin_actions(ui, st, module_dir, &state_path, actions);
        }
        ResolvedModuleUiBlock::Progress {
            label,
            field_id,
            min,
            max,
            empty,
        } => {
            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(8.0))
                .show(ui, |ui| {
                    ui.small(label.clone());
                    let Some(value) = module_field_value_as_number(&st.values, field_id) else {
                        ui.label(if empty.trim().is_empty() {
                            "(empty)".to_string()
                        } else {
                            empty.clone()
                        });
                        return;
                    };

                    let field_spec = module_field_spec(spec, field_id);
                    let min = min
                        .or_else(|| field_spec.and_then(|field| field.min))
                        .unwrap_or(0.0);
                    let max = max
                        .or_else(|| field_spec.and_then(|field| field.max))
                        .unwrap_or(100.0);
                    let denom = (max - min).abs().max(f64::EPSILON);
                    let progress = ((value - min) / denom).clamp(0.0, 1.0) as f32;
                    ui.add(egui::ProgressBar::new(progress).text(format!("{value:.2} / {max:.2}")));
                });
        }
        ResolvedModuleUiBlock::Record {
            title,
            ui_id,
            field_ids,
            empty,
        } => {
            render_module_record_block(ui, spec, &st.values, title, ui_id, field_ids, empty);
        }
        ResolvedModuleUiBlock::Table {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            has_header,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_table_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *has_header,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Checklist {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_checklist_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Timeline {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_timeline_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Kanban {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            lanes,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_kanban_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                lanes,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::BarChart {
            title,
            field_ids,
            min,
            max,
            empty,
        } => {
            if !title.trim().is_empty() {
                ui.label(egui::RichText::new(title).strong());
            }

            let mut rendered = 0usize;
            for field_id in field_ids {
                let Some(value) = module_field_value_as_number(&st.values, field_id) else {
                    continue;
                };
                let field_spec = module_field_spec(spec, field_id);
                let field_min = min
                    .or_else(|| field_spec.and_then(|field| field.min))
                    .unwrap_or(0.0);
                let field_max = max
                    .or_else(|| field_spec.and_then(|field| field.max))
                    .unwrap_or(100.0);
                let denom = (field_max - field_min).abs().max(f64::EPSILON);
                let progress = ((value - field_min) / denom).clamp(0.0, 1.0) as f32;
                ui.small(module_field_label(spec, field_id));
                ui.add(egui::ProgressBar::new(progress).text(format!("{value:.2}")));
                rendered += 1;
            }

            if rendered == 0 {
                ui.small(if empty.trim().is_empty() {
                    "No chart values available yet."
                } else {
                    empty
                });
            }
        }
        ResolvedModuleUiBlock::DependencyGraph {
            title,
            ui_id,
            field_id,
            path,
            empty,
            max_rows,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_dependency_graph_block(
                ui,
                module_id,
                module_dir,
                &st.values,
                title,
                ui_id,
                field_id,
                path,
                empty,
                *max_rows,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::Tabs {
            title,
            ui_id,
            panes,
            view_presets,
        } => {
            render_module_tabs_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                panes,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::Split {
            title,
            ui_id,
            direction,
            panes,
            view_presets,
        } => {
            render_module_split_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                direction,
                panes,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::Accordion {
            title,
            ui_id,
            panes,
            inspector_style,
            view_presets,
        } => {
            render_module_accordion_block(
                ui,
                st,
                spec,
                module_id,
                module_dir,
                title,
                ui_id,
                panes,
                *inspector_style,
                view_presets,
            );
        }
        ResolvedModuleUiBlock::FileList {
            title,
            ui_id,
            path,
            empty,
            max_entries,
            searchable,
            filter_placeholder,
            filter_presets,
        } => {
            render_module_file_list(
                ui,
                module_id,
                module_dir,
                title,
                ui_id,
                path,
                empty,
                *max_entries,
                *searchable,
                filter_placeholder,
                filter_presets,
            );
        }
        ResolvedModuleUiBlock::ArtifactPreview {
            title,
            path,
            field_id,
            empty,
            max_lines,
        } => {
            render_module_artifact_preview(
                ui, module_dir, &st.values, title, path, field_id, empty, *max_lines,
            );
        }
        ResolvedModuleUiBlock::Separator => {
            ui.separator();
        }
        ResolvedModuleUiBlock::Spacer(points) => {
            ui.add_space(*points);
        }
    }
}

fn render_module_section(
    ui: &mut egui::Ui,
    st: &mut ModuleFormState,
    spec: &ModuleUiSpec,
    module_id: &str,
    module_dir: &Path,
    section: &ResolvedModuleUiSection,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(&section.title).heading());
            if !section.description.trim().is_empty() {
                ui.label(section.description.clone());
                ui.add_space(6.0);
            }

            for (idx, block) in section.blocks.iter().enumerate() {
                render_module_block(ui, st, spec, module_id, module_dir, block);
                if idx + 1 < section.blocks.len() {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
            }
        });
}

fn render_module_tools_card(
    ui: &mut egui::Ui,
    module_dir: &Path,
    state_path: &Path,
    spec_path: &Path,
    filled_fields: usize,
    total_fields: usize,
    status: &str,
) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Module Tools").heading());
            ui.small(format!(
                "{} / {} fields populated",
                filled_fields, total_fields
            ));
            ui.small(format!("UI: {}", spec_path.display()));
            ui.small(format!("State: {}", state_path.display()));
            if !status.trim().is_empty() {
                ui.add_space(4.0);
                ui.small(status.to_string());
            }
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                if ui.button("Open Folder").clicked() {
                    open_path_in_explorer(module_dir);
                }
                let candidates = [
                    ("README", module_dir.join("README.md")),
                    ("Manual", module_dir.join("USER_MANUAL.md")),
                    ("Handshake", module_dir.join("HANDSHAKE.md")),
                    ("State JSON", state_path.to_path_buf()),
                ];
                for (label, path) in candidates {
                    if path.is_file() && ui.button(label).clicked() {
                        open_path_in_explorer(&path);
                    }
                }
            });
        });
}

fn render_module_surface(
    ui: &mut egui::Ui,
    app: &mut ChattyCogApp,
    manifest: Option<&ModuleManifest>,
    module_id: &str,
    module_dir: &Path,
) {
    // Prefer a declarative form UI if the module provides `ui.json`.
    let spec_path = module_dir.join("ui.json");
    if spec_path.is_file() {
        let st = app
            .module_forms
            .entry(module_id.to_string())
            .or_insert_with(|| ModuleFormState::new(module_dir));
        st.ensure_loaded();

        if let Some(spec) = st.spec.clone() {
            let title = spec
                .title
                .clone()
                .or_else(|| manifest.map(|m| m.display_name.clone()))
                .unwrap_or_else(|| "Module Workspace".to_string());
            let description = spec
                .description
                .clone()
                .or_else(|| manifest.map(|m| m.description.clone()))
                .unwrap_or_default();

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(title).heading());
                            if !description.trim().is_empty() {
                                ui.label(description);
                            }
                            if let Some(mf) = manifest {
                                ui.small(format!("Module ID: {}", mf.module_id));
                            } else {
                                ui.small(format!("Module ID: {module_id}"));
                            }
                        });
                    });

                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Reload UI").clicked() {
                            st.reload();
                        }
                        if ui.button("Save").clicked() {
                            st.save();
                        }
                        if ui.button("Open Folder").clicked() {
                            open_path_in_explorer(module_dir);
                        }
                    });
                });

            ui.add_space(10.0);
            let sections = resolve_module_ui_sections(&spec);
            let mut main_sections = Vec::new();
            let mut sidebar_sections = Vec::new();
            for section in sections {
                if section.sidebar {
                    sidebar_sections.push(section);
                } else {
                    main_sections.push(section);
                }
            }

            let filled = filled_field_count(&spec, &st.values);
            let total = spec.fields.len();
            let status = st.status.clone();

            if !sidebar_sections.is_empty() && ui.available_width() >= 980.0 {
                let total_width = ui.available_width();
                let sidebar_width = total_width.clamp(260.0, 330.0);
                let main_width = (total_width - sidebar_width - 12.0).max(320.0);

                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(main_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            for (idx, section) in main_sections.iter().enumerate() {
                                render_module_section(
                                    ui, st, &spec, module_id, module_dir, section,
                                );
                                if idx + 1 < main_sections.len() {
                                    ui.add_space(10.0);
                                }
                            }
                        },
                    );

                    ui.add_space(12.0);

                    ui.allocate_ui_with_layout(
                        egui::vec2(sidebar_width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            render_module_tools_card(
                                ui,
                                module_dir,
                                &st.state_path,
                                &spec_path,
                                filled,
                                total,
                                &status,
                            );
                            ui.add_space(10.0);
                            for (idx, section) in sidebar_sections.iter().enumerate() {
                                render_module_section(
                                    ui, st, &spec, module_id, module_dir, section,
                                );
                                if idx + 1 < sidebar_sections.len() {
                                    ui.add_space(10.0);
                                }
                            }
                        },
                    );
                });
            } else {
                render_module_tools_card(
                    ui,
                    module_dir,
                    &st.state_path,
                    &spec_path,
                    filled,
                    total,
                    &status,
                );
                ui.add_space(10.0);

                for section in &main_sections {
                    render_module_section(ui, st, &spec, module_id, module_dir, section);
                    ui.add_space(10.0);
                }
                for section in &sidebar_sections {
                    render_module_section(ui, st, &spec, module_id, module_dir, section);
                    ui.add_space(10.0);
                }
            }
        }

        return;
    }

    // Fallback: module-provided template-backed workspace text.
    let ws = app
        .module_workspaces
        .entry(module_id.to_string())
        .or_insert_with(|| ModuleWorkspaceState::new(module_dir));
    ws.ensure_loaded();

    ui.heading("Workspace");
    ui.horizontal(|ui| {
        if ui.button("Reload").clicked() {
            ws.reload();
        }
        if ui.button("Load template").clicked() {
            ws.load_template();
        }
        if ui.button("Save").clicked() {
            ws.save();
        }
        if ui.button("Open Folder").clicked() {
            let _ = std::process::Command::new("explorer.exe")
                .arg(module_dir)
                .spawn();
        }
        if !ws.status.trim().is_empty() {
            ui.label(ws.status.clone());
        }
    });

    egui::ScrollArea::vertical()
        .id_salt(format!("module_workspace_{module_id}"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut ws.text)
                    .desired_rows(18)
                    .hint_text("Module workspace...")
                    .code_editor(),
            );
        });
}

fn message_bubble(ui: &mut egui::Ui, msg: &Message) {
    let (label, color, fill) = match msg.role {
        Role::System => (
            "SYSTEM",
            egui::Color32::from_gray(120),
            egui::Color32::from_rgb(246, 246, 246),
        ),
        Role::User => (
            "YOU",
            egui::Color32::from_rgb(30, 80, 180),
            egui::Color32::from_rgb(240, 246, 255),
        ),
        Role::Assistant => (
            "ASSISTANT",
            egui::Color32::from_rgb(20, 120, 60),
            egui::Color32::from_rgb(244, 250, 245),
        ),
    };

    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            egui::Frame::none()
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.45)))
                .rounding(egui::Rounding::same(6.0))
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                ui.set_max_width(width);
                ui.horizontal(|ui| {
                    ui.colored_label(color, label);
                });
                ui.add(egui::Label::new(msg.content.clone()).wrap());
                if matches!(msg.role, Role::Assistant) {
                    if let Some(thinking) = msg
                        .thinking
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        ui.add_space(4.0);
                        let toggle_id = ui.make_persistent_id((
                            "assistant_thinking_toggle",
                            msg.content.as_str(),
                            thinking,
                        ));
                        let is_open = ui.ctx().data_mut(|data| {
                            data.get_persisted::<bool>(toggle_id).unwrap_or(false)
                        });
                        let label = if is_open {
                            if msg.content.trim().is_empty() {
                                "Hide thinking (live)"
                            } else {
                                "Hide thinking"
                            }
                        } else if msg.content.trim().is_empty() {
                            "Show thinking (live)"
                        } else {
                            "Show thinking"
                        };
                        egui::Frame::none()
                            .fill(ui.visuals().faint_bg_color)
                            .stroke(egui::Stroke::new(
                                1.0,
                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                            ))
                            .rounding(egui::Rounding::same(6.0))
                            .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let chevron = if is_open { "▼" } else { "▶" };
                                    let response = ui.add(
                                        egui::Button::new(format!("{chevron} {label}"))
                                            .frame(false),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.small(
                                                egui::RichText::new("Reasoning trace")
                                                    .weak()
                                                    .monospace(),
                                            );
                                        },
                                    );
                                    if response.clicked() {
                                        ui.ctx().data_mut(|data| {
                                            data.insert_persisted(toggle_id, !is_open);
                                        });
                                    }
                                });
                            });
                        if is_open {
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical()
                                .id_salt(("assistant_thinking", msg.content.as_str()))
                                .max_height(220.0)
                                .show(ui, |ui| {
                                    let mut thinking_text = thinking.to_string();
                                    ui.add(
                                        egui::TextEdit::multiline(&mut thinking_text)
                                            .code_editor()
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(8)
                                            .interactive(false),
                                    );
                                });
                        }
                    }
                }
            });
        },
    );
    ui.add_space(6.0);
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
            opts.push(ModelOption {
                label: format!("models/{}", name),
                value: name,
            });
        }
    }

    if let Some(mod_root) = modules_dir {
        let module_models = scan_ggufs_in_modules(Some(mod_root));
        for p in module_models {
            if let Ok(rel) = p.strip_prefix(mod_root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                opts.push(ModelOption {
                    label: format!("modules/{}", rel),
                    value: format!("modules/{}", rel),
                });
            }
        }
    }

    opts.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    opts
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

fn find_models_dir() -> Option<PathBuf> {
    // Prefer workspace layout: <root>/models next to <root>/runtime and <root>/chattycog_gui
    find_upwards_with_child("models")
        .ok()
        .map(|root| root.join("models"))
}

fn find_modules_dir() -> Option<PathBuf> {
    find_upwards_with_child("modules")
        .ok()
        .map(|root| root.join("modules"))
}

fn find_runtime_windows_dir() -> anyhow::Result<PathBuf> {
    let root = find_upwards_with_child("runtime")?;
    let p = root.join("runtime").join("windows");
    if p.is_dir() {
        Ok(p)
    } else {
        anyhow::bail!("runtime/windows not found at {}", p.display());
    }
}

fn find_upwards_with_child(child: &str) -> anyhow::Result<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }

    for start in starts {
        let mut cur = Some(start.as_path());
        while let Some(dir) = cur {
            let candidate = dir.join(child);
            if candidate.is_dir() {
                return Ok(dir.to_path_buf());
            }
            cur = dir.parent();
        }
    }

    anyhow::bail!("could not locate `{child}` by searching upwards from exe/cwd");
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

fn find_default_logs_dir() -> Option<PathBuf> {
    // Prefer a local sibling folder first: <cwd>/memory
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("memory");
        if p.is_dir() {
            return Some(p);
        }
    }

    // Fallback to <repo>/chattycog_gui/memory
    find_upwards_with_child("chattycog_gui")
        .ok()
        .map(|root| root.join("chattycog_gui").join("memory"))
}

fn find_sandbox_dir() -> Option<PathBuf> {
    // Prefer workspace layout: <root>/Chatty_Sandbox next to <root>/models and <root>/runtime
    find_upwards_with_child("Chatty_Sandbox")
        .ok()
        .map(|root| root.join("Chatty_Sandbox"))
}

fn find_or_create_sandbox_dir() -> Option<PathBuf> {
    if let Some(existing) = find_sandbox_dir() {
        return Some(existing);
    }

    let root = find_upwards_with_child("chattycog_gui")
        .ok()
        .or_else(|| std::env::current_dir().ok())?;
    let dir = root.join("Chatty_Sandbox");
    if std::fs::create_dir_all(&dir).is_ok() {
        Some(dir)
    } else {
        None
    }
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

fn read_lukewarm_from_logs_dir(logs_dir: Option<&Path>) -> anyhow::Result<String> {
    let dir: PathBuf = if let Some(d) = logs_dir {
        d.to_path_buf()
    } else if let Some(d) = find_default_logs_dir() {
        d
    } else {
        return Ok(String::new());
    };
    let path = dir.join("lukewarm.txt");
    if !path.is_file() {
        return Ok(String::new());
    }
    read_text_file(&path, 200_000)
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

fn runtime_backend_summary(status: &str) -> (&'static str, egui::Color32) {
    let lower = status.to_ascii_lowercase();
    if lower.contains("runtime error")
        || lower.contains("fallback failed")
        || lower.contains("load error")
    {
        ("Runtime issue", egui::Color32::from_rgb(170, 40, 40))
    } else if lower.contains("vulkan") {
        ("GPU path active", egui::Color32::from_rgb(25, 110, 70))
    } else if lower.contains("cpu") {
        ("CPU path active", egui::Color32::from_rgb(140, 95, 20))
    } else {
        ("Runtime ready", egui::Color32::from_rgb(50, 90, 150))
    }
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
    raw
        .replace("<|im_start|>", "")
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
        1536usize, 1280, 1024, 896, 768, 640, 512, 384, 320, 256, 192, 160, 128, 96, 80, 64,
        48, 32,
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

fn read_departments_from_logs_dir(logs_dir: Option<&Path>) -> anyhow::Result<String> {
    let dir: PathBuf = if let Some(d) = logs_dir {
        d.to_path_buf()
    } else if let Some(d) = find_default_logs_dir() {
        d
    } else {
        return Ok(String::new());
    };
    let path = dir.join("departments.md");
    if !path.is_file() {
        return Ok(String::new());
    }
    read_text_file(&path, 200_000)
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

fn sandbox_ai_text_guard(rel_path: &str) -> anyhow::Result<()> {
    if sandbox_rel_path_is_ai_text_allowed(rel_path) {
        Ok(())
    } else {
        anyhow::bail!("only .txt and .md sandbox files are allowed for AI tool actions")
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

fn build_explicit_sandbox_task_prompt(
    request: &str,
    rel_path: &str,
    intent: SandboxTaskIntent,
) -> String {
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

fn logs_tab(ui: &mut egui::Ui, app: &mut ChattyCogApp) {
    ui.heading("Logs");
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Filter module:");
        ui.add_sized(
            [180.0, 24.0],
            egui::TextEdit::singleline(&mut app.logs_filter_module).hint_text("any"),
        );
        ui.label("Tag:");
        ui.add_sized(
            [140.0, 24.0],
            egui::TextEdit::singleline(&mut app.logs_filter_tag).hint_text("any"),
        );
        if ui.button("Clear filters").clicked() {
            app.logs_filter_module.clear();
            app.logs_filter_tag.clear();
        }
    });

    ui.horizontal(|ui| {
        ui.label("Semantic:");
        ui.add_sized(
            [ui.available_width() - 90.0, 28.0],
            egui::TextEdit::singleline(&mut app.logs_query_semantic).hint_text("Ask bookkeeper..."),
        );
        if ui.button("Search").clicked() {
            if let Some(bk) = &app.bookkeeper {
                app.logs_results_semantic = bk
                    .search(
                        app.logs_query_semantic.clone(),
                        Some(app.logs_filter_module.clone()),
                        Some(app.logs_filter_tag.clone()),
                        16,
                    )
                    .ok()
                    .unwrap_or_default();
            } else {
                app.logs_results_semantic.clear();
                app.logs_results_keyword =
                    vec!["Bookkeeper not running. Use Logs sidebar to Start.".to_string()];
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Keyword:");
        ui.add_sized(
            [ui.available_width() - 90.0, 28.0],
            egui::TextEdit::singleline(&mut app.logs_query_keyword)
                .hint_text("Search cold_log.jsonl..."),
        );
        if ui.button("Find").clicked() {
            app.logs_results_keyword =
                keyword_search_cold_log(app.logs_dir.as_deref(), &app.logs_query_keyword, 50);
        }
    });

    ui.add_space(8.0);

    ui.group(|ui| {
        ui.label("New cold-log event (schemaless)");
        ui.horizontal(|ui| {
            ui.label("Module/Dept:");
            ui.add_sized(
                [160.0, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_module).hint_text("general"),
            );
            ui.label("Type:");
            ui.add_sized(
                [120.0, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_event_type).hint_text("note"),
            );
            ui.label("Tags:");
            ui.add_sized(
                [ui.available_width() - 70.0, 24.0],
                egui::TextEdit::singleline(&mut app.logs_new_tags).hint_text("comma,separated"),
            );
        });
        ui.label("Summary:");
        ui.add(
            egui::TextEdit::multiline(&mut app.logs_new_summary)
                .desired_rows(2)
                .hint_text("Short human summary..."),
        );
        ui.label("Payload JSON (optional):");
        ui.add(
            egui::TextEdit::multiline(&mut app.logs_new_payload_json)
                .desired_rows(3)
                .hint_text("{\"anything\": \"goes\"}"),
        );
        if ui.button("Append to cold log").clicked() {
            let summary = app.logs_new_summary.trim().to_string();
            if !summary.is_empty() {
                let tags = app
                    .logs_new_tags
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                let payload = app.logs_new_payload_json.trim().to_string();
                let payload = if payload.is_empty() {
                    None
                } else {
                    Some(payload)
                };
                if let Some(bk) = &app.bookkeeper {
                    bk.append_module_event(
                        app.logs_new_module.trim().to_string(),
                        app.logs_new_event_type.trim().to_string(),
                        summary,
                        tags,
                        payload,
                    );
                }
                app.logs_new_summary.clear();
            }
        }
    });

    ui.columns(2, |cols| {
        cols[0].heading("Results");
        egui::ScrollArea::vertical()
            .id_salt("logs_results_scroll")
            .show(&mut cols[0], |ui| {
                if !app.logs_results_semantic.is_empty() {
                    ui.label("Semantic hits:");
                    for h in &app.logs_results_semantic {
                        let module = h.module.clone().unwrap_or_else(|| "-".to_string());
                        let ty = h.event_type.clone().unwrap_or_else(|| "-".to_string());
                        let tags = if h.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" tags=[{}]", h.tags.join(", "))
                        };
                        ui.label(format!(
                            "{:.3} [{}] ({}/{}){} {}",
                            h.score, h.source, module, ty, tags, h.text
                        ));
                    }
                    ui.separator();
                }
                if !app.logs_results_keyword.is_empty() {
                    ui.label("Keyword hits:");
                    for l in &app.logs_results_keyword {
                        ui.label(l);
                    }
                }
                if app.logs_results_semantic.is_empty() && app.logs_results_keyword.is_empty() {
                    ui.label("No results yet.");
                }
            });

        cols[1].heading("Log Folder");
        egui::ScrollArea::vertical()
            .id_salt("logs_folder_scroll")
            .show(&mut cols[1], |ui| {
                let Some(dir) = &app.logs_dir else {
                    ui.label("No logs dir.");
                    return;
                };
                for p in list_dir_files(dir) {
                    let name = p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if ui
                        .selectable_label(app.logs_selected.as_ref() == Some(&p), name)
                        .clicked()
                    {
                        app.logs_selected = Some(p.clone());
                        app.logs_view =
                            read_text_file(&p, 200_000).unwrap_or_else(|e| format!("Error: {e:#}"));
                    }
                }
            });
    });

    ui.separator();
    ui.heading("Preview");
    egui::ScrollArea::vertical()
        .id_salt("logs_preview_scroll")
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.logs_view)
                    .desired_rows(12)
                    .code_editor(),
            );
        });
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
