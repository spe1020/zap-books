//! One extraction: file check, PDF pages, model call, normalization.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use ledger_core::{Currency, ExtractedField, ExtractionFields, FieldStatus};
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
}

/// Field statuses only, for logs: never values or evidence.
pub fn status_summary(fields: &ExtractionFields) -> String {
    fn s<T>(name: &str, f: &ExtractedField<T>) -> String {
        let status = match f.status {
            FieldStatus::Found => "found",
            FieldStatus::Unclear => "unclear",
            FieldStatus::Absent => "absent",
        };
        format!("{name}={status}")
    }
    [
        s("vendor", &fields.vendor),
        s("date", &fields.date),
        s("total", &fields.total),
        s("currency", &fields.currency),
        s("reference", &fields.reference),
        s("subtotal", &fields.subtotal),
        s("tax", &fields.tax),
        s("tip", &fields.tip),
        s("category", &fields.category),
    ]
    .join(" ")
}

pub struct Extractor {
    pub ollama: Ollama,
    pub model: String,
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
        let content = self
            .ollama
            .chat(Chat {
                model: &self.model,
                system: &system,
                user: &user,
                images: &images,
                schema: &schema,
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
