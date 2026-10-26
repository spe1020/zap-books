//! TypeScript JSON deserializes and re-serializes to the same JSON.

use ledger_core::{BusinessData, ExpenseStatus, JournalEntry, Workspace};
use serde_json::{json, Value};

fn sample() -> Value {
    json!({
        "schemaVersion": 1,
        "revision": 3,
        "businesses": [{
            "profile": { "id": "b1", "businessId": "b1", "name": "Shop", "industry": "Retail",
                         "currency": "USD", "initials": "SH", "color": "#fff" },
            "accounts": [
                { "id": "bank", "businessId": "b1", "name": "Bank", "kind": "bank", "currency": "USD" },
                { "id": "exp", "businessId": "b1", "name": "Supplies", "kind": "expense", "currency": "USD" }
            ],
            "categories": [{ "id": "cat", "businessId": "b1", "name": "Supplies", "accountId": "exp" }],
            "vendors": [{ "id": "v1", "businessId": "b1", "name": "Acme" }],
            "documents": [{ "id": "doc", "businessId": "b1", "name": "r.svg", "mime": "image/svg+xml",
                            "size": 1234, "kind": "receipt", "createdAt": "2026-10-15T12:00:00.000Z",
                            "demo": true }],
            "payments": [{ "id": "pay", "businessId": "b1", "date": "2026-10-01", "description": "Acme",
                           "amountMinor": 500, "currency": "USD", "accountId": "bank",
                           "reference": "R1", "fingerprint": "[\"b1\"]", "draftId": "d1" }],
            "drafts": [
                { "id": "d1", "businessId": "b1", "vendor": "Acme", "date": "2026-10-01",
                  "description": "Paper", "amountMinor": 500, "currency": "USD", "categoryId": "cat",
                  "accountId": "bank", "purpose": "Office", "documentIds": ["doc"], "paymentId": "pay",
                  "status": "posted", "journalId": "j1", "duplicateReason": "",
                  "createdAt": "2026-10-15T12:00:00.000Z" },
                { "id": "d2", "businessId": "b1", "vendor": "", "date": "2026-10-02",
                  "description": "", "amountMinor": 0, "currency": "EUR", "categoryId": "",
                  "accountId": "", "purpose": "", "documentIds": [], "status": "draft",
                  "correctionOf": "d0", "duplicateReason": "", "createdAt": "2026-10-15T12:00:00.000Z",
                  "importSource": { "id": "src", "businessId": "b1", "provider": "quickbooks",
                                    "reference": "QB-1", "fingerprint": "fp", "date": "2026-10-02",
                                    "amountMinor": 9007199254740991_i64, "currency": "EUR", "vendor": "",
                                    "category": "", "filename": "x.csv" } }
            ],
            "journals": [{ "id": "j1", "businessId": "b1", "date": "2026-10-01", "currency": "USD",
                           "lines": [
                               { "accountId": "exp", "categoryId": "cat", "debit": 500, "credit": 0 },
                               { "accountId": "bank", "debit": 0, "credit": 500 }
                           ],
                           "kind": "expense", "expenseId": "d1", "memo": "Acme · Paper",
                           "createdAt": "2026-10-15T12:00:00.000Z" },
                         { "id": "j2", "businessId": "b1", "date": "2026-10-01", "currency": "USD",
                           "lines": [
                               { "accountId": "exp", "categoryId": "cat", "debit": 0, "credit": 500 },
                               { "accountId": "bank", "debit": 500, "credit": 0 }
                           ],
                           "kind": "reversal", "expenseId": "d1", "reversesId": "j1",
                           "memo": "Correction: x", "createdAt": "2026-10-15T12:00:00.000Z" }],
            "approvals": [{ "id": "a1", "businessId": "b1", "expenseId": "d1", "actor": "local-demo-user",
                            "at": "2026-10-15T12:00:00.000Z", "duplicateReason": "" }],
            "rules": [{ "id": "r1", "businessId": "b1", "vendorIncludes": "Acme", "categoryId": "cat",
                        "purpose": "Office", "enabled": true }],
            "audits": [{ "id": "au", "businessId": "b1", "action": "expense.posted", "entityId": "d1",
                         "at": "2026-10-15T12:00:00.000Z", "actor": "local-demo-user", "detail": "x" }],
            "events": [{ "id": "e1", "businessId": "b1", "schemaVersion": 1, "type": "settings.updated",
                         "actor": "local-demo-user", "occurredAt": "2026-10-15T12:00:00.000Z",
                         "sourceIds": ["doc"], "entityId": "d1",
                         "payload": { "detail": "x", "count": 2, "ratio": 0.5, "ok": false } }],
            "extractions": [{
                "id": "x1", "businessId": "b1", "documentId": "doc", "draftId": "d1",
                "model": "m", "promptVersion": "p1", "createdAt": "2026-10-15T12:00:00.000Z",
                "fields": {
                    "vendor": { "status": "found", "value": "Acme", "evidence": "ACME" },
                    "date": { "status": "found", "value": "2026-10-01", "evidence": "10/01/2026" },
                    "total": { "status": "found", "value": 500, "evidence": "TOTAL $5.00" },
                    "currency": { "status": "found", "value": "USD", "evidence": "$" },
                    "reference": { "status": "absent", "value": null, "evidence": "" },
                    "subtotal": { "status": "unclear", "value": null, "evidence": "SUB ?" },
                    "tax": { "status": "absent", "value": null, "evidence": "" },
                    "category": { "status": "found", "value": "cat", "evidence": "Office" }
                },
                "accepted": ["vendor", "total"],
                "outcomes": { "vendor": "accepted", "total": "edited", "date": "ignored" }
            }]
        }]
    })
}

#[test]
fn workspace_round_trips_to_identical_json() {
    let input = sample();
    let workspace: Workspace = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(serde_json::to_value(&workspace).unwrap(), input);
    let b = &workspace.businesses[0];
    assert_eq!(b.drafts[0].status(), ExpenseStatus::Posted);
    assert_eq!(b.drafts[1].status(), ExpenseStatus::Draft);
}

#[test]
fn rejects_amounts_outside_the_safe_integer_range() {
    let mut input = sample();
    input["businesses"][0]["drafts"][1]["importSource"]["amountMinor"] =
        json!(9007199254740992_i64);
    assert!(serde_json::from_value::<Workspace>(input).is_err());
}

#[test]
fn rejects_unbalanced_or_unsupported_shapes() {
    let mut input = sample();
    input["businesses"][0]["journals"][0]["lines"][0]["debit"] = json!(501);
    let err = serde_json::from_value::<Workspace>(input).unwrap_err();
    assert!(err.to_string().contains("Journal entry must balance."));

    let mut input = sample();
    input["schemaVersion"] = json!(2);
    assert!(serde_json::from_value::<Workspace>(input).is_err());

    let mut input = sample();
    input["businesses"][0]["accounts"][0]["currency"] = json!("JPY");
    assert!(serde_json::from_value::<Workspace>(input).is_err());
}

#[test]
fn journal_entries_build_only_through_checked_constructors() {
    let workspace: Workspace = serde_json::from_value(sample()).unwrap();
    let b: &BusinessData = &workspace.businesses[0];
    let mut entry = b.journals[0].to_unchecked();
    entry.lines[0].debit = ledger_core::Minor::new(499).unwrap();
    assert_eq!(
        JournalEntry::new(b, entry).unwrap_err().to_string(),
        "Journal entry must balance."
    );
    assert!(JournalEntry::new(b, b.journals[0].to_unchecked()).is_ok());
}

#[test]
fn workspaces_saved_before_extractions_load_with_none() {
    let mut input = sample();
    input["businesses"][0]
        .as_object_mut()
        .unwrap()
        .remove("extractions");
    let workspace: Workspace = serde_json::from_value(input).unwrap();
    assert!(workspace.businesses[0].extractions.is_empty());
}
