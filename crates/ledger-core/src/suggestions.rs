//! Suggestions attached to drafts (`src/domain/suggestions.ts`).

use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::types::{BusinessData, ExpenseStatus, Suggestion};

/// `SuggestionInput`: a suggestion to attach. The record's id and time are
/// generated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionInput {
    pub draft_id: String,
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub evidence: Vec<String>,
}

/// `/^[0-9a-f]{64}$/`
fn is_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Trimmed text, or `None` where the TypeScript's `?.trim()` is falsy.
fn text(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(js::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
}

/// `suggest`: attaches a suggestion to a draft of this business. It needs
/// its author's key, at least one of a category, a purpose or a note, and
/// its evidence. Text is trimmed and blank optional fields are left out. The
/// draft itself is never changed.
pub fn suggest(
    business: &BusinessData,
    input: &SuggestionInput,
    env: &mut impl Env,
) -> Result<BusinessData> {
    if !is_key(&input.author) {
        return Err(LedgerError::SuggestionAuthor);
    }
    let draft = business
        .drafts
        .iter()
        .find(|d| d.id == input.draft_id && d.business_id == business.profile.id)
        .ok_or(LedgerError::DraftNotFound)?;
    if draft.status() != ExpenseStatus::Draft {
        return Err(LedgerError::SuggestionNotDraft);
    }
    let category_id = input.category_id.clone().filter(|c| !c.is_empty());
    let purpose = text(&input.purpose);
    let note = text(&input.note);
    if category_id.is_none() && purpose.is_none() && note.is_none() {
        return Err(LedgerError::SuggestionEmpty);
    }
    if let Some(id) = &category_id {
        if !business
            .categories
            .iter()
            .any(|c| c.id == *id && c.business_id == business.profile.id)
        {
            return Err(LedgerError::RuleCategory);
        }
    }
    let evidence: Vec<String> = input
        .evidence
        .iter()
        .map(|e| js::trim(e).to_string())
        .collect();
    if evidence.is_empty() || evidence.iter().any(String::is_empty) {
        return Err(LedgerError::SuggestionEvidence);
    }
    let mut b = business.clone();
    let suggestion = Suggestion {
        id: env.new_id(),
        business_id: b.profile.id.clone(),
        draft_id: draft.id.clone(),
        author: input.author.clone(),
        category_id,
        purpose,
        note,
        evidence,
        created_at: env.now(),
    };
    b.suggestions.push(suggestion);
    Ok(b)
}
