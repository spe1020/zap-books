//! Settings and classification-rule edits (`src/domain/settings.ts`).

use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::{LedgerError, Result};
use crate::js;
use crate::ledger::push_event;
use crate::money::Currency;
use crate::types::{Account, AccountKind, BusinessData, Category, ClassificationRule, EventType};

/// `SettingsChange`: one edit from the Settings screen. New records get
/// generated ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum SettingsChange {
    #[serde(rename_all = "camelCase")]
    Profile { name: String, industry: String },
    #[serde(rename_all = "camelCase")]
    AddAccount {
        name: String,
        kind: AccountKind,
        currency: Currency,
    },
    #[serde(rename_all = "camelCase")]
    RenameAccount { id: String, name: String },
    /// Creates the category and its expense account.
    #[serde(rename_all = "camelCase")]
    AddCategory { name: String, currency: Currency },
    #[serde(rename_all = "camelCase")]
    RenameCategory { id: String, name: String },
}

/// `RuleInput`: a classification rule to save. Without an `id` it is a new
/// rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub vendor_includes: String,
    pub category_id: String,
    pub purpose: String,
    pub enabled: bool,
}

/// `name.toLowerCase()`
fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// `updateSettings`: applies one Settings edit and logs it. Account and
/// category names are unique within their list, ignoring case. Adding a
/// category also adds its expense account.
pub fn update_settings(
    business: &BusinessData,
    change: &SettingsChange,
    env: &mut impl Env,
) -> Result<BusinessData> {
    let mut b = business.clone();
    let profile_id = b.profile.id.clone();
    let (is_account, raw_name, editing) = match change {
        SettingsChange::Profile { name, industry } => {
            if !js::has_text(name) || !js::has_text(industry) {
                return Err(LedgerError::ProfileRequired);
            }
            b.profile.name = js::trim(name).to_string();
            b.profile.industry = js::trim(industry).to_string();
            push_event(
                &mut b,
                env,
                EventType::SettingsUpdated,
                &profile_id,
                "Business profile updated",
                Vec::new(),
            );
            return Ok(b);
        }
        SettingsChange::AddAccount { name, .. } => (true, name, None),
        SettingsChange::RenameAccount { id, name } => (true, name, Some(id)),
        SettingsChange::AddCategory { name, .. } => (false, name, None),
        SettingsChange::RenameCategory { id, name } => (false, name, Some(id)),
    };
    let name = js::trim(raw_name).to_string();
    if name.is_empty() {
        return Err(LedgerError::NameRequired);
    }
    let wanted = lower(&name);
    let taken = |id: &str, existing: &str| {
        Some(id) != editing.map(String::as_str) && lower(existing) == wanted
    };
    let in_use = if is_account {
        b.accounts.iter().any(|x| taken(&x.id, &x.name))
    } else {
        b.categories.iter().any(|x| taken(&x.id, &x.name))
    };
    if in_use {
        return Err(LedgerError::NameInUse);
    }
    match change {
        SettingsChange::RenameAccount { id, .. } => {
            let record = b
                .accounts
                .iter_mut()
                .find(|x| x.id == *id && x.business_id == profile_id)
                .ok_or(LedgerError::AccountNotFound)?;
            record.name = name.clone();
        }
        SettingsChange::RenameCategory { id, .. } => {
            let record = b
                .categories
                .iter_mut()
                .find(|x| x.id == *id && x.business_id == profile_id)
                .ok_or(LedgerError::CategoryNotFound)?;
            record.name = name.clone();
        }
        // `currencies.includes(currency)` holds by construction (D2).
        SettingsChange::AddAccount { kind, currency, .. } => {
            if *kind == AccountKind::Expense {
                return Err(LedgerError::AccountKindInvalid);
            }
            b.accounts.push(Account {
                id: env.new_id(),
                business_id: profile_id.clone(),
                name: name.clone(),
                kind: *kind,
                currency: *currency,
            });
        }
        SettingsChange::AddCategory { currency, .. } => {
            let account_id = env.new_id();
            b.accounts.push(Account {
                id: account_id.clone(),
                business_id: profile_id.clone(),
                name: name.clone(),
                kind: AccountKind::Expense,
                currency: *currency,
            });
            b.categories.push(Category {
                id: env.new_id(),
                business_id: profile_id.clone(),
                name: name.clone(),
                account_id,
            });
        }
        SettingsChange::Profile { .. } => {}
    }
    let kind = if is_account { "account" } else { "category" };
    push_event(
        &mut b,
        env,
        EventType::SettingsUpdated,
        &profile_id,
        &format!("{kind} saved: {name}"),
        Vec::new(),
    );
    Ok(b)
}

/// `saveRule`: saves a classification rule and logs it. Vendor text is unique
/// within the business, ignoring case. A saved rule moves to the end of the
/// list, so an edited rule is consulted after the others.
pub fn save_rule(
    business: &BusinessData,
    input: &RuleInput,
    env: &mut impl Env,
) -> Result<BusinessData> {
    if !js::has_text(&input.vendor_includes) || !js::has_text(&input.purpose) {
        return Err(LedgerError::RuleFieldsRequired);
    }
    if !business
        .categories
        .iter()
        .any(|c| c.id == input.category_id && c.business_id == business.profile.id)
    {
        return Err(LedgerError::RuleCategory);
    }
    let wanted = lower(js::trim(&input.vendor_includes));
    if business
        .rules
        .iter()
        .any(|r| Some(&r.id) != input.id.as_ref() && lower(&r.vendor_includes) == wanted)
    {
        return Err(LedgerError::DuplicateRule);
    }
    let mut b = business.clone();
    let rule = ClassificationRule {
        id: input.id.clone().unwrap_or_else(|| env.new_id()),
        business_id: b.profile.id.clone(),
        vendor_includes: js::trim(&input.vendor_includes).to_string(),
        category_id: input.category_id.clone(),
        purpose: js::trim(&input.purpose).to_string(),
        enabled: input.enabled,
    };
    let rule_id = rule.id.clone();
    b.rules.retain(|r| r.id != rule_id);
    b.rules.push(rule);
    push_event(
        &mut b,
        env,
        EventType::RuleSaved,
        &rule_id,
        &format!("Classification rule saved: {}", input.vendor_includes),
        Vec::new(),
    );
    Ok(b)
}
