//! The local Ollama API: model list, loaded-model memory, and one chat call
//! with structured output. The model gets no tools.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ExtractError;

#[derive(Debug, Clone)]
pub struct Ollama {
    base: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

#[derive(Deserialize)]
struct Ps {
    #[serde(default)]
    models: Vec<PsModel>,
}

#[derive(Deserialize)]
struct PsModel {
    name: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

/// One chat request: a system prompt, a user message with images, and the
/// JSON schema the answer should follow.
pub struct Chat<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub user: &'a str,
    /// Base64-encoded images.
    pub images: &'a [String],
    pub schema: &'a Value,
}

/// `name` matches an installed model, allowing Ollama's implicit `:latest`.
fn same_model(installed: &str, wanted: &str) -> bool {
    installed == wanted || installed.strip_suffix(":latest") == Some(wanted)
}

impl Ollama {
    pub fn new(base: &str, timeout: Duration) -> Result<Ollama, ExtractError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .no_proxy()
            .build()
            .map_err(|_| ExtractError::Internal)?;
        Ok(Ollama {
            base: base.trim_end_matches('/').to_string(),
            http,
        })
    }

    /// Whether Ollama answers and has `model` installed.
    pub async fn has_model(&self, model: &str) -> Result<bool, ExtractError> {
        let tags: Tags = self
            .http
            .get(format!("{}/api/tags", self.base))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map_err(|_| ExtractError::OllamaUnavailable)?
            .json()
            .await
            .map_err(|_| ExtractError::OllamaUnavailable)?;
        Ok(tags.models.iter().any(|m| same_model(&m.name, model)))
    }

    /// Memory Ollama reports for a loaded model, in bytes, if it is loaded.
    pub async fn loaded_size(&self, model: &str) -> Option<u64> {
        let ps: Ps = self
            .http
            .get(format!("{}/api/ps", self.base))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        ps.models
            .into_iter()
            .find(|m| same_model(&m.name, model))
            .map(|m| m.size)
    }

    /// Sends one chat request and returns the message content.
    pub async fn chat(&self, chat: Chat<'_>) -> Result<String, ExtractError> {
        let body = json!({
            "model": chat.model,
            "stream": false,
            "format": chat.schema,
            "options": { "temperature": 0 },
            "messages": [
                { "role": "system", "content": chat.system },
                { "role": "user", "content": chat.user, "images": chat.images }
            ]
        });
        let response = self
            .http
            .post(format!("{}/api/chat", self.base))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ExtractError::Timeout
                } else {
                    ExtractError::OllamaUnavailable
                }
            })?;
        match response.status().as_u16() {
            200 => {}
            404 => return Err(ExtractError::ModelMissing(chat.model.to_string())),
            _ => return Err(ExtractError::ModelFailed),
        }
        let reply: ChatResponse = response.json().await.map_err(|e| {
            if e.is_timeout() {
                ExtractError::Timeout
            } else {
                ExtractError::MalformedAnswer
            }
        })?;
        Ok(reply.message.content)
    }
}

#[cfg(test)]
mod tests {
    use super::same_model;

    #[test]
    fn latest_is_implicit() {
        assert!(same_model("qwen2.5vl:7b", "qwen2.5vl:7b"));
        assert!(same_model("llava:latest", "llava"));
        assert!(!same_model("llava:13b", "llava"));
    }
}
