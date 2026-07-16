use anyhow::{Context, Result, anyhow};
use serde_json::json;

use super::{CloudProviderAdapter, ProviderCapabilities, ResolvedCloudTarget};

#[derive(Debug, Clone)]
pub struct GeminiAdapter {
    target: ResolvedCloudTarget,
}

impl GeminiAdapter {
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
            .set("Authorization", &format!("Bearer {}", self.target.api_key))
            .set("Content-Type", "application/json")
            .send_json(body.clone())
            .with_context(|| format!("POST {url}"))?;

        response
            .into_json::<serde_json::Value>()
            .with_context(|| format!("parse JSON response from {url}"))
    }
}

impl CloudProviderAdapter for GeminiAdapter {
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
        let url = format!("{}/chat/completions", self.target.base_url);
        let body = json!({
            "model": self.target.chat_model_name,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "temperature": temp,
            "top_p": top_p,
            "max_tokens": max_tokens.max(1),
            "stream": false
        });
        let value = self.post_json(&url, &body)?;
        extract_chat_completion_text(&value).with_context(|| {
            format!(
                "cloud model '{}' returned an unexpected Gemini chat response shape",
                self.target.display_name
            )
        })
    }

    fn embed_text(&self, input: &str) -> Result<Vec<f32>> {
        let model = self
            .target
            .embedding_model_name
            .as_deref()
            .ok_or_else(|| {
                anyhow!(
                    "cloud model '{}' is missing an embedding model name",
                    self.target.display_name
                )
            })?;

        let url = format!("{}/embeddings", self.target.base_url);
        let body = json!({
            "model": model,
            "input": input
        });
        let value = self.post_json(&url, &body)?;
        extract_embedding(&value).with_context(|| {
            format!(
                "cloud model '{}' returned an unexpected Gemini embeddings response shape",
                self.target.display_name
            )
        })
    }
}

fn extract_chat_completion_text(value: &serde_json::Value) -> Result<String> {
    let content = value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .ok_or_else(|| anyhow!("missing choices[0].message.content"))?;

    if let Some(text) = content.as_str() {
        return Ok(text.trim().to_string());
    }

    if let Some(parts) = content.as_array() {
        let mut out = String::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(|text| text.as_str()) {
                out.push_str(text);
            }
        }
        let out = out.trim().to_string();
        if !out.is_empty() {
            return Ok(out);
        }
    }

    Err(anyhow!("unsupported content shape"))
}

fn extract_embedding(value: &serde_json::Value) -> Result<Vec<f32>> {
    let items = value
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|data| data.first())
        .and_then(|item| item.get("embedding"))
        .and_then(|embedding| embedding.as_array())
        .ok_or_else(|| anyhow!("missing data[0].embedding"))?;

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let number = item
            .as_f64()
            .ok_or_else(|| anyhow!("embedding contained a non-numeric value"))?;
        out.push(number as f32);
    }
    Ok(out)
}
