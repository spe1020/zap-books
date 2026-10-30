//! Applying commands: genesis, document references, receipt readings, the
//! submitter's limits, and rebuilding and auditing a chain of real commands.

mod common;

use common::*;
use ledger_core::{BusinessData, ExpenseStatus, ExtractionRecord, RuleInput, SettingsChange};
use protocol::{
    apply_command, audit, diff, genesis_business, genesis_changes, rebuild, AcceptanceContent,
    Command, CommittedPayload, DocumentRef, Outcome, ProposalPayload, ProtocolError, RecordChange,
    RecordType, RecordingEnv, Refusal, PROTOCOL_VERSION,
};

const NOW: &str = "2023-03-01T09:00:00.000Z";

fn uuids() -> impl FnMut() -> String {
    let mut n = 0u64;
    move || {
        n += 1;
        format!("00000000-0000-4000-8000-{n:012}")
    }
}

fn genesis_command() -> Command {
    let mut payload = genesis_payload(backup_recovery());
    if let Command::CreateBusiness {
        profile,
        accounts,
        categories,
        ..
    } = &mut payload.command
    {
        // The seed's records carry the seed's business id; genesis needs ours.
        let id = business_id().to_string();
        profile.id = id.clone();
        profile.business_id = id.clone();
        for a in accounts.iter_mut() {
            a.business_id = id.clone();
        }
        for c in categories.iter_mut() {
            c.business_id = id.clone();
        }
    }
    payload.command
}

fn document(id: &str) -> DocumentRef {
    DocumentRef {
        id: id.into(),
        sha256: "4c".repeat(32),
        name: "receipt.pdf".into(),
        mime: "application/pdf".into(),
        size: 4096,
    }
}

/// A ready-to-approve draft in the genesis business.
fn draft(b: &BusinessData, id: &str, documents: &[&str]) -> ledger_core::ExpenseDraft {
    let mut env = RecordingEnv::new(uuids(), NOW);
    let mut d = ledger_core::blank_draft(b, &mut env);
    d.id = id.into();
    d.vendor = "Invented Vendor".into();
    d.description = "Invented supplies".into();
    d.purpose = "Office use".into();
    d.date = "2023-02-14".into();
    d.amount_minor = ledger_core::Minor::new(1234).unwrap();
    d.category_id = b.categories[0].id.clone();
    d.document_ids = documents.iter().map(|s| s.to_string()).collect();
    d
}

fn reading(b: &BusinessData, draft_id: &str, document_id: &str) -> ExtractionRecord {
    let absent = serde_json::json!({"status": "absent", "value": null, "evidence": ""});
    let lines = serde_json::json!({"status": "absent", "value": null, "lines": []});
    serde_json::from_value(serde_json::json!({
        "id": "client-chosen",
        "businessId": b.profile.id,
        "documentId": document_id,
        "draftId": draft_id,
        "model": "invented-model",
        "promptVersion": "receipt-v4",
        "createdAt": "2023-01-01T00:00:00.000Z",
        "fields": {
            "vendor": absent, "date": absent, "total": absent, "currency": absent,
            "reference": absent, "subtotal": absent, "tax": lines, "fees": lines,
            "discounts": lines, "tip": absent, "category": absent
        },
        "accepted": [],
        "outcomes": null
    }))
    .unwrap()
}

fn run(b: &BusinessData, command: &Command) -> Result<BusinessData, Refusal> {
    let mut env = RecordingEnv::new(uuids(), NOW);
    apply_command(b, command, &mut env)
}

#[test]
fn genesis_needs_the_business_id_and_sound_records() {
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    assert!(b.drafts.is_empty() && b.journals.is_empty());
    let changes = genesis_changes(&b);
    assert!(
        matches!(&changes[0], RecordChange::Put(r) if r.key().record_type == RecordType::Profile)
    );

    // The seed's own business id is not this business's id.
    let seed = genesis_payload(backup_recovery()).command;
    assert!(matches!(
        genesis_business(business_id(), &seed).unwrap_err(),
        Refusal::Protocol(ProtocolError::InvalidGenesis(_))
    ));

    // A category whose account isn't an expense account.
    let mut bad = genesis_command();
    if let Command::CreateBusiness {
        categories,
        accounts,
        ..
    } = &mut bad
    {
        let bank = accounts
            .iter()
            .find(|a| a.kind == ledger_core::AccountKind::Bank)
            .unwrap();
        categories[0].account_id = bank.id.clone();
    }
    assert!(genesis_business(business_id(), &bad).is_err());
}

#[test]
fn a_saved_draft_attaches_its_documents_with_their_hashes() {
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    let command = Command::SaveDraft {
        draft: draft(&b, "draft-1", &["doc-1"]),
        documents: vec![document("doc-1")],
        extraction: None,
    };
    let after = run(&b, &command).unwrap();
    let doc = &after.documents[0];
    assert_eq!(doc.sha256.as_deref(), Some("4c".repeat(32).as_str()));
    assert_eq!(doc.created_at, NOW);
    assert_eq!(doc.business_id, b.profile.id);

    // Without the reference, the draft points at nothing: the ledger's own
    // receipt check refuses it.
    let missing = Command::SaveDraft {
        draft: draft(&b, "draft-1", &["doc-1"]),
        documents: vec![],
        extraction: None,
    };
    assert_eq!(
        run(&b, &missing).unwrap_err().to_string(),
        "Invalid receipt reference."
    );
    // A bad hash, or an id already used.
    let mut bad = document("doc-2");
    bad.sha256 = "XY".repeat(32);
    let bad_hash = Command::SaveDraft {
        draft: draft(&b, "draft-2", &["doc-2"]),
        documents: vec![bad],
        extraction: None,
    };
    assert_eq!(
        run(&b, &bad_hash).unwrap_err(),
        Refusal::Protocol(ProtocolError::BadDocument)
    );
    let reused = Command::SaveDraft {
        draft: draft(&after, "draft-2", &["doc-1"]),
        documents: vec![document("doc-1")],
        extraction: None,
    };
    assert_eq!(
        run(&after, &reused).unwrap_err(),
        Refusal::Protocol(ProtocolError::BadDocument)
    );
}

#[test]
fn a_receipt_reading_must_belong_to_the_draft_and_business() {
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    let save = |b: &BusinessData, r: ExtractionRecord| {
        run(
            b,
            &Command::SaveDraft {
                draft: draft(b, "draft-1", &["doc-1"]),
                documents: if b.documents.is_empty() {
                    vec![document("doc-1")]
                } else {
                    vec![]
                },
                extraction: Some(r),
            },
        )
    };
    let after = save(&b, reading(&b, "draft-1", "doc-1")).unwrap();
    let stored = &after.extractions[0];
    // The service's id and time, not the client's.
    assert_ne!(stored.id, "client-chosen");
    assert!(stored.id.starts_with("00000000-0000-4000-8000-"));
    assert_eq!(stored.created_at, NOW);

    let scope = Refusal::Protocol(ProtocolError::ExtractionScope);
    let mut foreign = reading(&b, "draft-1", "doc-1");
    foreign.business_id = "another-business".into();
    assert_eq!(save(&b, foreign).unwrap_err(), scope);
    assert_eq!(
        save(&b, reading(&b, "draft-9", "doc-1")).unwrap_err(),
        scope
    );
    assert_eq!(
        save(&b, reading(&b, "draft-1", "doc-9")).unwrap_err(),
        scope
    );
    let mut approved = reading(&b, "draft-1", "doc-1");
    approved.outcomes = Some(Default::default());
    assert_eq!(save(&b, approved).unwrap_err(), scope);

    // Reading the same receipt again replaces the unapproved reading, as the
    // browser does, and the record changes say so.
    let again = save(&after, reading(&after, "draft-1", "doc-1")).unwrap();
    assert_eq!(again.extractions.len(), 1);
    assert_ne!(again.extractions[0].id, stored.id);
    let changes = diff(&after, &again).unwrap();
    assert!(changes
        .iter()
        .any(|c| matches!(c, RecordChange::Remove(r) if r.id == stored.id)));
}

#[test]
fn a_correction_attaches_documents_and_a_reading_like_a_save() {
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    // `accept` draws fresh ids for each step, as the service does.
    let (saved, _) = accept(
        &b,
        Command::SaveDraft {
            draft: draft(&b, "draft-1", &[]),
            documents: vec![],
            extraction: None,
        },
    );
    let (posted, _) = accept(
        &saved,
        Command::Approve {
            expense_id: "draft-1".into(),
        },
    );
    let correct = |documents: Vec<DocumentRef>, extraction: Option<ExtractionRecord>| {
        let mut replacement = draft(&posted, "draft-2", &["doc-2"]);
        replacement.amount_minor = ledger_core::Minor::new(1334).unwrap();
        Command::Correct {
            original_id: "draft-1".into(),
            replacement,
            reason: "The receipt shows a higher total.".into(),
            documents,
            extraction,
        }
    };
    let mut r = reading(&posted, "draft-2", "doc-2");
    r.fields.total = serde_json::from_value(
        serde_json::json!({"status": "found", "value": 1334, "evidence": "TOTAL 13.34"}),
    )
    .unwrap();
    r.accepted = vec![ledger_core::ExtractionFieldName::Total];
    let command = correct(vec![document("doc-2")], Some(r.clone()));
    let (after, accepted) = accept(&posted, command);
    let doc = after.documents.iter().find(|d| d.id == "doc-2").unwrap();
    assert_eq!(doc.sha256.as_deref(), Some("4c".repeat(32).as_str()));
    let stored = after
        .extractions
        .iter()
        .find(|x| x.draft_id == "draft-2")
        .unwrap();
    assert_ne!(stored.id, "client-chosen");
    // Posting the replacement finalizes its reading at once.
    let outcomes = stored.outcomes.clone().unwrap();
    assert_eq!(
        outcomes.get(&ledger_core::ExtractionFieldName::Total),
        Some(&ledger_core::ExtractionOutcome::Accepted)
    );
    let replacement = after.drafts.iter().find(|d| d.id == "draft-2").unwrap();
    assert_eq!(replacement.status(), ExpenseStatus::Posted);

    // An auditor re-running it gets the same books.
    assert_eq!(audit(Some(&posted), &accepted).unwrap(), after);

    // Checked as for SaveDraft.
    let mut bad = document("doc-2");
    bad.sha256 = "not-a-hash".into();
    assert_eq!(
        run(&posted, &correct(vec![bad], None)).unwrap_err(),
        Refusal::Protocol(ProtocolError::BadDocument)
    );
    let scope = Refusal::Protocol(ProtocolError::ExtractionScope);
    // A reading for the original, for a document the replacement doesn't
    // have, or one already judged.
    assert_eq!(
        run(
            &posted,
            &correct(
                vec![document("doc-2")],
                Some(reading(&posted, "draft-1", "doc-2"))
            )
        )
        .unwrap_err(),
        scope
    );
    assert_eq!(
        run(
            &posted,
            &correct(
                vec![document("doc-2")],
                Some(reading(&posted, "draft-2", "doc-9"))
            )
        )
        .unwrap_err(),
        scope
    );
    let mut judged = r.clone();
    judged.outcomes = Some(Default::default());
    assert_eq!(
        run(&posted, &correct(vec![document("doc-2")], Some(judged))).unwrap_err(),
        scope
    );

    // Without either, a correction reads as it always did.
    let old: Command = serde_json::from_value(serde_json::json!({
        "type": "Correct",
        "originalId": "draft-1",
        "replacement": draft(&posted, "draft-3", &[]),
        "reason": "Wrong category.",
    }))
    .unwrap();
    assert!(!serde_json::to_string(&old).unwrap().contains("documents"));
    assert!(run(&posted, &old).is_ok());
}

#[test]
fn a_submitter_can_only_add_new_drafts() {
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    let submit = |b: &BusinessData| Command::SubmitReceiptDraft {
        draft: draft(b, "draft-1", &[]),
        documents: vec![],
    };
    let after = run(&b, &submit(&b)).unwrap();
    assert_eq!(after.drafts[0].status(), ExpenseStatus::Draft);
    assert_eq!(
        run(&after, &submit(&after)).unwrap_err(),
        Refusal::Protocol(ProtocolError::SubmitterNewDraftsOnly)
    );
}

/// The service's side: applies each command and records what an acceptance
/// would carry.
fn accept(b: &BusinessData, command: Command) -> (BusinessData, AcceptanceContent) {
    let mut ids = uuids();
    // Each acceptance draws fresh ids, as the service's generator would.
    for _ in 0..b.events.len() * 4 {
        ids();
    }
    let mut env = RecordingEnv::new(ids, NOW);
    let after = apply_command(b, &command, &mut env).unwrap();
    let changes = diff(b, &after).unwrap();
    let content = content(command, env.into_record(), changes);
    (after, content)
}

fn content(
    command: Command,
    env: protocol::EnvRecord,
    changes: Vec<RecordChange>,
) -> AcceptanceContent {
    let payload = ProposalPayload::new(business_id(), Some(0), command);
    let proposal =
        protocol::build_proposal(&bookkeeper(), &pk(&service()), &payload, at(1)).unwrap();
    AcceptanceContent {
        v: PROTOCOL_VERSION,
        proposal,
        payload: CommittedPayload::new(payload).unwrap(),
        env,
        changes,
        outcome: Outcome::Applied,
    }
}

#[test]
fn a_chain_of_commands_rebuilds_and_audits() {
    let genesis = genesis_business(business_id(), &genesis_command()).unwrap();
    let mut genesis_content = content(
        genesis_command(),
        protocol::EnvRecord {
            ids: vec![],
            now: NOW.into(),
        },
        genesis_changes(&genesis),
    );
    let mut payload = genesis_content.payload.payload().clone();
    payload.base_seq = None;
    genesis_content.payload = CommittedPayload::new(payload).unwrap();
    let mut contents = vec![genesis_content];
    let mut b = genesis.clone();
    let category = b.categories[0].id.clone();
    let commands = vec![
        Command::SaveDraft {
            draft: draft(&b, "draft-1", &["doc-1"]),
            documents: vec![document("doc-1")],
            extraction: Some(reading(&b, "draft-1", "doc-1")),
        },
        Command::Approve {
            expense_id: "draft-1".into(),
        },
        Command::UpdateSettings {
            change: SettingsChange::AddCategory {
                name: "Invented travel".into(),
                currency: ledger_core::Currency::Usd,
            },
        },
        Command::SaveRule {
            rule: RuleInput {
                id: None,
                vendor_includes: "Invented Vendor".into(),
                category_id: category,
                purpose: "Office use".into(),
                enabled: true,
            },
        },
    ];
    for command in commands {
        let (next, content) = accept(&b, command);
        contents.push(content);
        b = next;
    }
    assert_eq!(b.journals.len(), 1);

    // Rebuild: the record changes alone give the same books.
    assert_eq!(rebuild(&contents).unwrap(), b);

    // Audit: re-running every command with its recorded ids and time gives
    // the recorded changes.
    let mut audited = audit(None, &contents[0]).unwrap();
    for content in &contents[1..] {
        audited = audit(Some(&audited), content).unwrap();
    }
    assert_eq!(audited, b);

    // A recorded change the command didn't make fails the audit.
    let mut forged = contents[2].clone();
    forged.changes.pop();
    let before = rebuild(&contents[..2]).unwrap();
    assert_eq!(
        audit(Some(&before), &forged).unwrap_err(),
        Refusal::Protocol(ProtocolError::AuditMismatch)
    );
    // And so do different recorded ids.
    let mut reids = contents[2].clone();
    reids.env.ids.reverse();
    assert!(audit(Some(&before), &reids).is_err());
}

/// The service's `today()` reaches no stored value: every command type the
/// service runs gives identical books under two environments that differ
/// only in `today()`.
#[test]
fn today_never_reaches_the_books() {
    use ledger_core::{
        parse_csv, preview_import, CsvMapping, ImportOptions, SequentialEnv, SignConvention,
    };
    let base = genesis_business(business_id(), &genesis_command()).unwrap();
    let envs = || {
        (
            SequentialEnv::new(1, NOW, "2016-01-01"),
            SequentialEnv::new(1, NOW, "2026-12-31"),
        )
    };
    let run_both = |b: &BusinessData, command: &Command| {
        let (mut early, mut late) = envs();
        let x = apply_command(b, command, &mut early).unwrap();
        let y = apply_command(b, command, &mut late).unwrap();
        assert_eq!(x, y, "today() changed the result of {:?}", command);
        x
    };
    let saved = run_both(
        &base,
        &Command::SaveDraft {
            draft: draft(&base, "draft-1", &["doc-1"]),
            documents: vec![document("doc-1")],
            extraction: Some(reading(&base, "draft-1", "doc-1")),
        },
    );
    let approved = run_both(
        &saved,
        &Command::Approve {
            expense_id: "draft-1".into(),
        },
    );
    let mut replacement = approved.drafts[0].replacement("draft-1b");
    replacement.amount_minor = ledger_core::Minor::new(999).unwrap();
    let corrected = run_both(
        &approved,
        &Command::Correct {
            original_id: "draft-1".into(),
            replacement,
            reason: "Invented correction".into(),
            documents: vec![],
            extraction: None,
        },
    );
    let settings = run_both(
        &corrected,
        &Command::UpdateSettings {
            change: SettingsChange::AddCategory {
                name: "Invented travel".into(),
                currency: ledger_core::Currency::Usd,
            },
        },
    );
    let ruled = run_both(
        &settings,
        &Command::SaveRule {
            rule: RuleInput {
                id: None,
                vendor_includes: "Invented Vendor".into(),
                category_id: settings.categories[0].id.clone(),
                purpose: "Office use".into(),
                enabled: true,
            },
        },
    );
    let submitted = run_both(
        &ruled,
        &Command::SubmitReceiptDraft {
            draft: draft(&ruled, "draft-2", &[]),
            documents: vec![],
        },
    );
    // commit_import starts each draft from blank_draft, dated today(), and
    // then sets the row's date.
    let account = submitted.accounts[0].id.clone();
    let csv = "date,description,amount,currency,reference\n2023-02-10,Invented card purchase,-12.34,USD,R1\n";
    let map = CsvMapping {
        date: "date".into(),
        description: "description".into(),
        amount: "amount".into(),
        currency: "currency".into(),
        reference: "reference".into(),
        vendor: None,
        category: None,
        purpose: None,
        transaction_type: None,
        source_account: None,
    };
    let (mut env, _) = envs();
    let rows = preview_import(
        &submitted,
        &parse_csv(csv).unwrap(),
        &map,
        &account,
        SignConvention::Negative,
        &ImportOptions::default(),
        &mut env,
    )
    .unwrap();
    let imported = run_both(
        &submitted,
        &Command::CommitImport {
            account_id: account,
            rows,
        },
    );
    assert_eq!(imported.drafts.last().unwrap().date, "2023-02-10");
}

#[test]
fn proposer_chosen_ids_are_short_plain_and_not_empty() {
    // PROTO-7: draft, replacement, document and rule ids.
    let b = genesis_business(business_id(), &genesis_command()).unwrap();
    let long = "a".repeat(protocol::MAX_ID_LEN + 1);
    let bad_ids = ["", "draft 1", "draft/1", "dräft", long.as_str()];
    for id in bad_ids {
        let save = Command::SaveDraft {
            draft: draft(&b, id, &[]),
            documents: vec![],
            extraction: None,
        };
        assert_eq!(
            run(&b, &save).unwrap_err(),
            Refusal::Protocol(ProtocolError::BadId),
            "draft id {id:?}"
        );
        let submit = Command::SubmitReceiptDraft {
            draft: draft(&b, id, &[]),
            documents: vec![],
        };
        assert_eq!(
            run(&b, &submit).unwrap_err(),
            Refusal::Protocol(ProtocolError::BadId)
        );
        let attach = Command::SaveDraft {
            draft: draft(&b, "draft-1", &[id]),
            documents: vec![document(id)],
            extraction: None,
        };
        assert_eq!(
            run(&b, &attach).unwrap_err(),
            Refusal::Protocol(ProtocolError::BadDocument)
        );
        let rule = Command::SaveRule {
            rule: RuleInput {
                id: Some(id.into()),
                vendor_includes: "invented".into(),
                category_id: b.categories[0].id.clone(),
                purpose: "Office use".into(),
                enabled: true,
            },
        };
        assert_eq!(
            run(&b, &rule).unwrap_err(),
            Refusal::Protocol(ProtocolError::BadId)
        );
    }

    // A correction's replacement is a new draft too.
    let saved = run(
        &b,
        &Command::SaveDraft {
            draft: draft(&b, "draft-1", &[]),
            documents: vec![],
            extraction: None,
        },
    )
    .unwrap();
    let posted = run(
        &saved,
        &Command::Approve {
            expense_id: "draft-1".into(),
        },
    )
    .unwrap();
    let correct = |id: &str| Command::Correct {
        original_id: "draft-1".into(),
        replacement: draft(&posted, id, &[]),
        reason: "Wrong amount".into(),
        documents: vec![],
        extraction: None,
    };
    assert_eq!(
        run(&posted, &correct("")).unwrap_err(),
        Refusal::Protocol(ProtocolError::BadId)
    );

    // The longest allowed id, a UUID and the app's derived ids all pass.
    let longest = "a".repeat(protocol::MAX_ID_LEN);
    for id in [
        longest.as_str(),
        "00000000-0000-4000-8000-000000000001",
        "invented_draft-7",
    ] {
        assert!(protocol::valid_id(id), "{id}");
    }
    assert!(run(&posted, &correct("draft-2")).is_ok());
}

#[test]
fn genesis_names_follow_the_settings_rules() {
    // PROTO-9 (a): the case-insensitive name rule, as for later accounts.
    let mut command = genesis_command();
    if let Command::CreateBusiness { accounts, .. } = &mut command {
        accounts[1].name = accounts[0].name.to_uppercase();
    }
    assert!(matches!(
        genesis_business(business_id(), &command).unwrap_err(),
        Refusal::Protocol(ProtocolError::InvalidGenesis(_))
    ));
    let mut command = genesis_command();
    if let Command::CreateBusiness { categories, .. } = &mut command {
        categories[1].name = format!(" {} ", categories[0].name.to_lowercase());
    }
    assert!(matches!(
        genesis_business(business_id(), &command).unwrap_err(),
        Refusal::Protocol(ProtocolError::InvalidGenesis(_))
    ));
    let mut command = genesis_command();
    if let Command::CreateBusiness { accounts, .. } = &mut command {
        accounts[0].name = "  ".into();
    }
    assert!(genesis_business(business_id(), &command).is_err());
    // And its ids are proposer-chosen ids.
    let mut command = genesis_command();
    if let Command::CreateBusiness { accounts, .. } = &mut command {
        accounts[0].id = String::new();
    }
    assert_eq!(
        genesis_business(business_id(), &command).unwrap_err(),
        Refusal::Protocol(ProtocolError::BadId)
    );
}

#[test]
fn a_genesis_acceptance_records_no_ids() {
    // PROTO-9 (b): audit covers genesis's environment as it does every
    // other acceptance's.
    let command = genesis_command();
    let b = genesis_business(business_id(), &command).unwrap();
    let clean = content(
        command.clone(),
        protocol::EnvRecord {
            ids: vec![],
            now: NOW.into(),
        },
        genesis_changes(&b),
    );
    assert_eq!(audit(None, &clean).unwrap(), b);
    let spurious = AcceptanceContent {
        env: protocol::EnvRecord {
            ids: vec!["00000000-0000-4000-8000-000000000001".into()],
            now: NOW.into(),
        },
        ..clean
    };
    assert_eq!(
        audit(None, &spurious).unwrap_err(),
        Refusal::Protocol(ProtocolError::AuditMismatch)
    );
}
