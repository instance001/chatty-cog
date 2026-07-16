use anyhow::{Context, Result, anyhow};
use serde_json::json;

use super::{CloudProviderAdapter, ProviderCapabilities, ResolvedCloudTarget};

#[derive(Debug, Clone)]
pub struct AnthropicAdapter {
    target: ResolvedCloudTarget,
}

impl AnthropicAdapter {
    pub fn new(target: ResolvedCloudTarget) -> Result<Self> {
        if target.base_url.trim().is_empty() {
            return Err(anyhow!(
                "cloud model '{}' is missing a base URL",
                target.display_name
            ));
        }
        Ok(Self { target })
    }

    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
        let response = ureq::post(url)
            .set("x-api-key", &self.target.api_key)
            .set("anthropic-version", "2023-06-01")
            .set("Content-Type", "application/json")
            .send_json(body.clone())
            .with_context(|| format!("POST {url}"))?;

        response
            .into_json::<serde_json::Value>()
            .with_context(|| format!("parse JSON response from {url}"))
    }
}

impl CloudProviderAdapter for AnthropicAdapter {
    fn capabilities(&self) -> ProviderCapabilities {
        self.target.capabilities
    }

    fn display_name(&self) -> &str {
        &self.target.display_name
    }

    fn chat_model_name(&self) -> &str {
        &self.target.chat_model_name
    }

    fn chat_completion(
        &self,
        system: &str,
        user: &str,
        max_tokens: usize,
        temp: f32,
        top_p: f32,
    ) -> Result<String> {
        let url = format!("{}/messages", self.target.base_url);
        let body = json!({
            "model": self.target.chat_model_name,
            "system": system,
            "messages": [
                { "role": "user", "content": user }
            ],
            "temperature": temp,
            "top_p": top_p,
            "max_tokens": max_tokens.max(1)
        });
        let value = self.post_json(&url, &body)?;
        extract_message_text(&value).with_context(|| {
            format!(
                "cloud model '{}' returned an unexpected Anthropic response shape",
                self.target.display_name
            )
        })
    }

    fn embed_text(&self, _input: &str) -> Result<Vec<f32>> {
        Err(anyhow!(
            "cloud model '{}' does not expose embeddings through the current Anthropic adapter",
            self.target.display_name
        ))
    }
}

fn extract_message_text(value: &serde_json::Value) -> Result<String> {
    let content = value
        .get("content")
        .and_then(|content| content.as_array())
        .ok_or_else(|| anyhow!("missing content array"))?;

    let mut out = String::new();
    for block in content {
        if block.get("type").and_then(|kind| kind.as_str()) == Some("text")
            && let Some(text) = block.get("text").and_then(|text| text.as_str())
        {
            out.push_str(text);
        }
    }

    let out = out.trim().to_string();
    if out.is_empty() {
        Err(anyhow!("no text blocks returned"))
    } else {
        Ok(out)
    }
}
