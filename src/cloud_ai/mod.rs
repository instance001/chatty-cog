use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::preferences::{CloudModelEntry, CloudProviderKind};

pub mod openai_compatible;
pub mod anthropic;
pub mod gemini;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub chat: bool,
    pub embeddings: bool,
    pub multimodal: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelLane {
    Orchestrator,
    Bookkeeper,
    Multimodal,
}

impl ModelLane {
    pub fn supports(self, capabilities: ProviderCapabilities) -> bool {
        match self {
            Self::Orchestrator => capabilities.chat,
            Self::Bookkeeper => capabilities.chat && capabilities.embeddings,
            Self::Multimodal => capabilities.chat && capabilities.multimodal,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedCloudTarget {
    pub id: String,
    pub display_name: String,
    pub provider_kind: CloudProviderKind,
    pub api_key: String,
    pub base_url: String,
    pub chat_model_name: String,
    pub embedding_model_name: Option<String>,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone)]
pub enum ResolvedModelTarget {
    Local {
        selection: String,
        path: PathBuf,
    },
    Cloud {
        selection: String,
        target: ResolvedCloudTarget,
    },
}

pub trait CloudProviderAdapter: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    fn display_name(&self) -> &str;
    fn chat_model_name(&self) -> &str;
    fn chat_completion(
        &self,
        system: &str,
        user: &str,
        max_tokens: usize,
        temp: f32,
        top_p: f32,
    ) -> Result<String>;
    fn embed_text(&self, input: &str) -> Result<Vec<f32>>;
}

pub fn provider_capabilities(kind: CloudProviderKind) -> ProviderCapabilities {
    match kind {
        CloudProviderKind::OpenAi | CloudProviderKind::OpenAiCompatible => ProviderCapabilities {
            chat: true,
            embeddings: true,
            multimodal: false,
            streaming: false,
        },
        CloudProviderKind::Anthropic => ProviderCapabilities {
            chat: true,
            embeddings: false,
            multimodal: false,
            streaming: false,
        },
        CloudProviderKind::Gemini => ProviderCapabilities {
            chat: true,
            embeddings: true,
            multimodal: true,
            streaming: false,
        },
    }
}

pub fn resolve_cloud_target(entry: &CloudModelEntry, lane: ModelLane) -> Result<ResolvedCloudTarget> {
    let id = entry.id.trim().to_string();
    let mut display_name = entry.display_name.trim().to_string();
    let api_key = entry.api_key.trim().to_string();
    let mut base_url = entry.base_url.trim().trim_end_matches('/').to_string();
    let chat_model_name = entry.model_name.trim().to_string();
    let embedding_model_name = (!entry.embedding_model_name.trim().is_empty())
        .then(|| entry.embedding_model_name.trim().to_string());

    if id.is_empty() {
        return Err(anyhow!("cloud model entry is missing an id"));
    }
    if display_name.is_empty() {
        display_name = chat_model_name.clone();
    }
    if display_name.is_empty() {
        return Err(anyhow!("cloud model entry is missing a display name"));
    }
    if api_key.is_empty() {
        return Err(anyhow!("cloud model '{}' is missing an API key", display_name));
    }
    if base_url.is_empty() {
        base_url = default_base_url(entry.provider_kind.clone()).to_string();
    }
    if base_url.is_empty() {
        return Err(anyhow!("cloud model '{}' is missing a base URL", display_name));
    }
    if chat_model_name.is_empty() {
        return Err(anyhow!("cloud model '{}' is missing a chat model name", display_name));
    }

    let capabilities = provider_capabilities(entry.provider_kind.clone());
    if lane == ModelLane::Bookkeeper && embedding_model_name.is_none() {
        return Err(anyhow!(
            "cloud model '{}' needs an embeddings model name for the Bookkeeper lane",
            display_name
        ));
    }
    if !lane.supports(capabilities) {
        return Err(anyhow!(
            "cloud model '{}' does not support the {:?} lane",
            display_name,
            lane
        ));
    }

    Ok(ResolvedCloudTarget {
        id,
        display_name,
        provider_kind: entry.provider_kind.clone(),
        api_key,
        base_url,
        chat_model_name,
        embedding_model_name,
        capabilities,
    })
}

pub fn build_adapter(target: ResolvedCloudTarget) -> Result<Box<dyn CloudProviderAdapter>> {
    match target.provider_kind {
        CloudProviderKind::OpenAi | CloudProviderKind::OpenAiCompatible => {
            Ok(Box::new(openai_compatible::OpenAiCompatibleAdapter::new(target)?))
        }
        CloudProviderKind::Anthropic => Ok(Box::new(anthropic::AnthropicAdapter::new(target)?)),
        CloudProviderKind::Gemini => Ok(Box::new(gemini::GeminiAdapter::new(target)?)),
    }
}

pub fn cloud_selection_id(id: impl AsRef<str>) -> String {
    let id = id.as_ref().trim();
    if id.is_empty() {
        String::new()
    } else if id.starts_with("cloud:") {
        id.to_string()
    } else {
        format!("cloud:{id}")
    }
}

pub fn local_selection_id(value: impl AsRef<str>) -> String {
    let value = value.as_ref().trim();
    if value.is_empty() {
        String::new()
    } else if value.starts_with("local:") || value.starts_with("cloud:") {
        value.to_string()
    } else {
        format!("local:{value}")
    }
}

pub fn resolve_model_selection_for_dirs(
    models_dir: Option<&Path>,
    modules_dir: Option<&Path>,
    cloud_models: &[CloudModelEntry],
    selection: Option<&str>,
    lane: ModelLane,
    resolve_portable_model_hint_for_dirs: fn(Option<&Path>, Option<&Path>, Option<&str>) -> Option<PathBuf>,
) -> Option<ResolvedModelTarget> {
    let selection = selection?.trim();
    if selection.is_empty() {
        return None;
    }

    if let Some(id) = selection.strip_prefix("cloud:") {
        let entry = cloud_models
            .iter()
            .find(|entry| entry.id == id && entry.enabled)?;
        let target = resolve_cloud_target(entry, lane).ok()?;
        return Some(ResolvedModelTarget::Cloud {
            selection: selection.to_string(),
            target,
        });
    }

    let local_hint = selection.strip_prefix("local:").unwrap_or(selection);
    let path = resolve_portable_model_hint_for_dirs(models_dir, modules_dir, Some(local_hint))?;
    Some(ResolvedModelTarget::Local {
        selection: local_selection_id(local_hint),
        path,
    })
}

fn default_base_url(kind: CloudProviderKind) -> &'static str {
    match kind {
        CloudProviderKind::OpenAi => "https://api.openai.com/v1",
        CloudProviderKind::OpenAiCompatible => "",
        CloudProviderKind::Anthropic => "https://api.anthropic.com/v1",
        CloudProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
    }
}
