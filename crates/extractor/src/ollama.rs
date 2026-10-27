//! The local Ollama API: model list, model details, loaded-model memory,
//! unloading, and one chat call with structured output. The model gets no
//! tools.
//!
//! A chat that doesn't finish (timeout, error, or the caller giving up)
//! drops its HTTP connection, which makes Ollama stop generating. Before the
//! next chat, a one-token probe confirms the model is idle again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ExtractError;

/// Default cap on generated tokens, so a runaway answer stops.
pub const DEFAULT_MAX_TOKENS: u32 = 2048;

/// The longest the idle probe waits for an earlier generation to end. It
/// never waits longer than the request timeout either.
pub const IDLE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Ollama {
    base: String,
    http: reqwest::Client,
    max_tokens: u32,
    timeout: Duration,
    /// Set when a chat didn't finish; shared by clones.
    unsettled: Arc<AtomicBool>,
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
struct Show {
    #[serde(default)]
    capabilities: Vec<String>,
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
    /// Overrides the service's token cap; the warm-up asks for one token.
    pub max_tokens: Option<u32>,
}

/// `name` matches an installed model, allowing Ollama's implicit `:latest`.
fn same_model(installed: &str, wanted: &str) -> bool {
    installed == wanted || installed.strip_suffix(":latest") == Some(wanted)
}

/// Marks the client unsettled unless the request it guards completes.
struct Pending<'a> {
    unsettled: &'a AtomicBool,
    done: bool,
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.unsettled.store(true, Ordering::SeqCst);
        }
    }
}

fn send_error(e: &reqwest::Error) -> ExtractError {
    if e.is_timeout() {
        ExtractError::Timeout
    } else {
        ExtractError::OllamaUnavailable
    }
}

impl Ollama {
    pub fn new(base: &str, timeout: Duration, max_tokens: u32) -> Result<Ollama, ExtractError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .no_proxy()
            .build()
            .map_err(|_| ExtractError::Internal)?;
        Ok(Ollama {
            base: base.trim_end_matches('/').to_string(),
            http,
            max_tokens: max_tokens.max(1),
            timeout,
            unsettled: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The cap on generated tokens sent with every chat.
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
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

    /// Whether the model reports thinking support (`/api/show`
    /// capabilities), or `None` if Ollama didn't say.
    pub async fn supports_thinking(&self, model: &str) -> Option<bool> {
        let show: Show = self
            .http
            .post(format!("{}/api/show", self.base))
            .timeout(Duration::from_secs(5))
            .json(&json!({ "model": model }))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .json()
            .await
            .ok()?;
        Some(show.capabilities.iter().any(|c| c == "thinking"))
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

    /// Unloads `model` now (`keep_alive: 0`), freeing its memory.
    pub async fn unload(&self, model: &str) -> Result<(), ExtractError> {
        self.http
            .post(format!("{}/api/generate", self.base))
            .json(&json!({ "model": model, "keep_alive": 0 }))
            .send()
            .await
            .map_err(|e| send_error(&e))?
            .error_for_status()
            .map_err(|_| ExtractError::ModelFailed)?;
        Ok(())
    }

    /// After a chat that didn't finish, waits until `model` answers a
    /// one-token request, which it can only do once the earlier generation
    /// has stopped. Does nothing when the last chat finished. `chat` calls
    /// it too; the service calls it first so the wait isn't counted against
    /// the receipt's own timeout.
    pub async fn wait_until_idle(&self, model: &str) -> Result<(), ExtractError> {
        if !self.unsettled.load(Ordering::SeqCst) {
            return Ok(());
        }
        let probe = json!({
            "model": model,
            "stream": false,
            "think": false,
            "options": { "temperature": 0, "num_predict": 1 },
            "messages": [{ "role": "user", "content": "ok" }]
        });
        let response = self
            .http
            .post(format!("{}/api/chat", self.base))
            .timeout(IDLE_PROBE_TIMEOUT.min(self.timeout))
            .json(&probe)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ExtractError::Busy
                } else {
                    ExtractError::OllamaUnavailable
                }
            })?;
        // Read the whole answer so the probe itself completes.
        let _ = response.bytes().await;
        self.unsettled.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Sends one chat request and returns the message content. Thinking is
    /// always off and the answer is capped at `max_tokens`.
    pub async fn chat(&self, chat: Chat<'_>) -> Result<String, ExtractError> {
        self.wait_until_idle(chat.model).await?;
        let mut pending = Pending {
            unsettled: &self.unsettled,
            done: false,
        };
        let body = json!({
            "model": chat.model,
            "stream": false,
            "think": false,
            "format": chat.schema,
            "options": {
                "temperature": 0,
                "num_predict": chat.max_tokens.unwrap_or(self.max_tokens)
            },
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
            .map_err(|e| send_error(&e))?;
        let status = response.status().as_u16();
        let reply = response.bytes().await.map_err(|e| {
            if e.is_timeout() {
                ExtractError::Timeout
            } else {
                ExtractError::MalformedAnswer
            }
        });
        pending.done = reply.is_ok();
        let reply = reply?;
        match status {
            200 => {}
            404 => return Err(ExtractError::ModelMissing(chat.model.to_string())),
            _ => return Err(ExtractError::ModelFailed),
        }
        let reply: ChatResponse =
            serde_json::from_slice(&reply).map_err(|_| ExtractError::MalformedAnswer)?;
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
