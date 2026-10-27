//! One extraction: file check, PDF pages, model call, normalization.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use ledger_core::{Currency, ExtractionFields, FieldStatus};
use serde::{Deserialize, Serialize};

use crate::error::ExtractError;
use crate::files::{detect, FileKind};
use crate::normalize::{normalize, parse_answer, utc_date, Context};
use crate::ollama::{Chat, Ollama};
use crate::{pdf, prompt};

/// A category of the current business, as sent by the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
}

/// What the service returns: `ExtractionResult` in `src/domain/types.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionResult {
    pub fields: ExtractionFields,
    pub warnings: Vec<String>,
    pub model: String,
    pub prompt_version: String,
    pub pages_read: u32,
    pub used_text_layer: bool,
    pub duration_ms: u64,
    /// Whether the model reports thinking support (`/api/show`), or `null`
    /// if Ollama didn't say. Thinking is always requested off.
    pub model_supports_thinking: Option<bool>,
}

/// Field statuses only, for logs: never values or evidence.
pub fn status_summary(fields: &ExtractionFields) -> String {
    fn s(name: &str, status: FieldStatus) -> String {
        let status = match status {
            FieldStatus::Found => "found",
            FieldStatus::Unclear => "unclear",
            FieldStatus::Absent => "absent",
        };
        format!("{name}={status}")
    }
    [
        s("vendor", fields.vendor.status),
        s("date", fields.date.status),
        s("total", fields.total.status),
        s("currency", fields.currency.status),
        s("reference", fields.reference.status),
        s("subtotal", fields.subtotal.status),
        s("tax", fields.tax.status),
        s("fees", fields.fees.status),
        s("discounts", fields.discounts.status),
        s("tip", fields.tip.status),
        s("category", fields.category.status),
    ]
    .join(" ")
}

pub struct Extractor {
    pub ollama: Ollama,
    pub model: String,
}

/// The committed fictional sample the model is warmed up with.
const WARM_UP_RECEIPT: &[u8] = include_bytes!("../../../docs/samples/demo-receipt.png");

impl Extractor {
    /// Loads the model before the first real receipt: the same prompt and
    /// schema with a fictional sample, answering a single token. The answer
    /// is discarded.
    pub async fn warm_up(&self) -> Result<(), ExtractError> {
        let images = vec![base64::engine::general_purpose::STANDARD.encode(WARM_UP_RECEIPT)];
        let system = prompt::system_prompt(&[]);
        let user = prompt::user_message(None);
        let schema = prompt::answer_schema();
        self.ollama
            .chat(Chat {
                model: &self.model,
                system: &system,
                user: &user,
                images: &images,
                schema: &schema,
                max_tokens: Some(1),
            })
            .await
            .map(|_| ())
    }
}

fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    utc_date(i64::try_from(secs).unwrap_or(i64::MAX))
}

impl Extractor {
    /// Reads one receipt. The file is checked here as well as by the caller.
    pub async fn extract(
        &self,
        bytes: &[u8],
        categories: &[Category],
        currencies: &[Currency],
    ) -> Result<ExtractionResult, ExtractError> {
        let started = Instant::now();
        let kind = detect(bytes)?;
        let b64 = base64::engine::general_purpose::STANDARD;
        let (images, text) = if kind == FileKind::Pdf {
            let pages = pdf::read(bytes).await?;
            let images: Vec<String> = pages.images.iter().map(|p| b64.encode(p)).collect();
            (images, pages.text)
        } else {
            (vec![b64.encode(bytes)], None)
        };
        let system = prompt::system_prompt(categories);
        let user = prompt::user_message(text.as_deref());
        let schema = prompt::answer_schema();
        let model_supports_thinking = self.ollama.supports_thinking(&self.model).await;
        let content = self
            .ollama
            .chat(Chat {
                model: &self.model,
                system: &system,
                user: &user,
                images: &images,
                schema: &schema,
                max_tokens: None,
            })
            .await?;
        let raw = parse_answer(&content)?;
        let today = today_utc();
        let normalized = normalize(
            &raw,
            &Context {
                categories,
                currencies,
                today: &today,
            },
        );
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let result = ExtractionResult {
            fields: normalized.fields,
            warnings: normalized.warnings,
            model: self.model.clone(),
            prompt_version: prompt::PROMPT_VERSION.to_string(),
            pages_read: u32::try_from(images.len()).unwrap_or(u32::MAX),
            used_text_layer: text.is_some(),
            duration_ms,
            model_supports_thinking,
        };
        tracing::info!(
            model = %self.model,
            duration_ms,
            pdf = kind.is_pdf(),
            pages = result.pages_read,
            text_layer = result.used_text_layer,
            fields = %status_summary(&result.fields),
            "receipt read"
        );
        Ok(result)
    }
}
