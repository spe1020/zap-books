//! Event size limits. A full-size import is sent the way `docs/protocol.md`
//! specifies: batches start at [`IMPORT_BATCH_START`] rows, and a `tooLarge`
//! rejection sizes the next one from the reported size and limit. Every
//! proposal and acceptance that results fits under [`MAX_EVENT_BYTES`].

mod common;

use common::*;
use ledger_core::{
    commit_import, parse_csv, preview_import, BusinessData, CsvMapping, ImportKind, ImportOptions,
    ImportProvider, ImportRow, SignConvention,
};
use nostr::prelude::{Event, EventId};
use protocol::{
    build_acceptance, build_proposal, build_rejection, diff, next_batch, open_proposal,
    open_rejection, AcceptanceContent, AcceptanceHeader, Command, EpochKey, Outcome,
    ProposalPayload, ProtocolError, Reason, RecordingEnv, RejectionCode, IMPORT_BATCH_START,
    MAX_EVENT_BYTES, PROTOCOL_VERSION,
};

/// The most rows one CSV import may hold (`ledger-core`'s "Import up to
/// 2,000 transactions at a time.").
const FULL_IMPORT: usize = 2_000;

/// Service ids are UUIDs: 36 characters.
fn uuids() -> impl FnMut() -> String {
    let mut n = 0u64;
    move || {
        n += 1;
        format!("00000000-0000-4000-8000-{n:012}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Text {
    /// Short descriptions and vendor names, as most bank exports have.
    Typical,
    /// Long text in every free-form column.
    Long,
}

/// `n` invented rows, each with its own vendor, so every row also creates a
/// vendor record.
fn csv(n: usize, text: Text, history: bool) -> String {
    let mut out = String::from(if history {
        "date,description,amount,currency,reference,vendor,category,purpose\n"
    } else {
        "date,description,amount,currency,reference,vendor\n"
    });
    for i in 0..n {
        let day = 1 + i % 28;
        let amount = format!("-{}.{:02}", 10 + i % 900, i % 100);
        let (description, vendor, reference, category, purpose) = match text {
            Text::Typical => (
                format!("Card purchase {i:05}"),
                format!("Invented Vendor {i:05}"),
                format!("R{i:06}"),
                "Supplies".to_string(),
                "Office use".to_string(),
            ),
            Text::Long => (
                format!("Invented supplier order {i:05} for studio equipment, shipping and handling, quarterly batch with extra detail text"),
                format!("Invented Supplier Company Number {i:05} Limited Partnership"),
                format!("INV-REF-{i:08}-ABCDEFGHIJKLMNOP"),
                "Invented category name".to_string(),
                "Invented purpose text for this expense line".to_string(),
            ),
        };
        out.push_str(&format!(
            "2023-02-{day:02},\"{description}\",{amount},USD,{reference},\"{vendor}\""
        ));
        if history {
            out.push_str(&format!(",{category},\"{purpose}\""));
        }
        out.push('\n');
    }
    out
}

fn preview(b: &BusinessData, text: &str, history: bool) -> Vec<ImportRow> {
    let mut env = RecordingEnv::new(uuids(), "2023-03-01T09:00:00.000Z");
    let map = CsvMapping {
        date: "date".into(),
        description: "description".into(),
        amount: "amount".into(),
        currency: "currency".into(),
        reference: "reference".into(),
        vendor: Some("vendor".into()),
        category: history.then(|| "category".into()),
        purpose: history.then(|| "purpose".into()),
        transaction_type: None,
        source_account: None,
    };
    let options = if history {
        ImportOptions {
            kind: Some(ImportKind::ExpenseHistory),
            provider: Some(ImportProvider::Quickbooks),
            date_format: None,
            filename: Some("invented-expense-history-export.csv".into()),
        }
    } else {
        ImportOptions::default()
    };
    let rows = parse_csv(text).unwrap();
    let account = &b.accounts[0].id;
    let preview = preview_import(
        b,
        &rows,
        &map,
        account,
        SignConvention::Negative,
        &options,
        &mut env,
    )
    .unwrap();
    assert_eq!(preview.len(), FULL_IMPORT);
    assert!(preview.iter().all(|r| r.error.is_none()));
    preview
}

/// The service's answer to a proposal: a signed acceptance or rejection.
enum Answer {
    Accepted(Event),
    Rejected(Event),
}

/// The service's chain and books, as far as this test needs them.
struct Service {
    business: BusinessData,
    head: Option<EventId>,
    seq: u64,
    epoch: EpochKey,
    ids: Box<dyn FnMut() -> String>,
}

impl Service {
    fn new(business: BusinessData) -> Service {
        Service {
            business,
            // Seq 0 (genesis) is not part of this test; stand in for its id.
            head: Some(EventId::from_slice(&[9; 32]).unwrap()),
            seq: 0,
            epoch: EpochKey::generate(0),
            ids: Box::new(uuids()),
        }
    }

    /// Accepts the proposal, or rejects it. Every check, including the size
    /// of the acceptance, happens before anything is stored.
    fn receive(&mut self, proposal: &Event) -> Answer {
        let (header, payload) = open_proposal(proposal, &service()).unwrap();
        let Command::CommitImport { account_id, rows } = &payload.command else {
            unreachable!()
        };
        let attempt = (|| {
            let mut env = RecordingEnv::new(&mut self.ids, "2023-03-01T09:00:00.000Z");
            let next = commit_import(&self.business, rows, account_id, &mut env).unwrap();
            let content = AcceptanceContent {
                v: PROTOCOL_VERSION,
                proposal: proposal.clone(),
                payload: payload.clone(),
                env: env.into_record(),
                changes: diff(&self.business, &next)?,
                outcome: Outcome::Applied,
            };
            let acceptance = AcceptanceHeader {
                business: business_id(),
                seq: self.seq + 1,
                prev: self.head,
                proposal: header.id,
                epoch: 0,
                next_service: None,
            };
            let event = build_acceptance(
                &service(),
                &self.epoch.public_key(),
                &acceptance,
                &content,
                at(self.seq + 1),
            )?;
            Ok::<_, ProtocolError>((event, next))
        })();
        match attempt {
            Ok((event, next)) => {
                // Stored only now.
                self.business = next;
                self.head = Some(event.id);
                self.seq += 1;
                Answer::Accepted(event)
            }
            Err(e) => Answer::Rejected(
                build_rejection(
                    &service(),
                    &header,
                    &Reason::from_error(&e),
                    at(self.seq + 1),
                )
                .unwrap(),
            ),
        }
    }
}

#[derive(Debug)]
struct Report {
    accepted: usize,
    too_large: usize,
    final_batch: usize,
    largest_proposal: usize,
    largest_acceptance: usize,
}

/// The client: sends `rows` in order, starting at [`IMPORT_BATCH_START`] rows
/// a batch. On `tooLarge` it sends the same rows again in a batch sized from
/// the reported size, and keeps that size for the rest of the file.
fn send(books: &mut Service, rows: &[ImportRow]) -> Report {
    let account = books.business.accounts[0].id.clone();
    let mut report = Report {
        accepted: 0,
        too_large: 0,
        final_batch: IMPORT_BATCH_START,
        largest_proposal: 0,
        largest_acceptance: 0,
    };
    let mut size = IMPORT_BATCH_START;
    let mut next = 0;
    // `docs/protocol.md`: rows × limit ÷ size × 0.9, rounded down.
    let shrink = |rows: usize, size: Option<u64>, limit: Option<u64>| {
        assert!(rows > 1, "a single row is over the limit");
        let (size, limit) = (size.unwrap(), limit.unwrap());
        assert!(size > limit);
        next_batch(rows, size, limit)
    };
    while next < rows.len() {
        let batch = &rows[next..rows.len().min(next + size)];
        let payload = ProposalPayload::new(
            business_id(),
            Some(books.seq),
            Command::CommitImport {
                account_id: account.clone(),
                rows: batch.to_vec(),
            },
        );
        // The client's own builder refuses an oversized proposal the same way.
        let proposal = match build_proposal(&bookkeeper(), &pk(&service()), &payload, at(1)) {
            Ok(p) => p,
            Err(e) if RejectionCode::of(&e) == RejectionCode::TooLarge => {
                let reason = Reason::from_error(&e);
                size = shrink(batch.len(), reason.size, reason.limit);
                report.too_large += 1;
                continue;
            }
            Err(e) => panic!("{e}"),
        };
        report.largest_proposal = report.largest_proposal.max(proposal.as_json().len());
        let seq_before = books.seq;
        match books.receive(&proposal) {
            Answer::Accepted(acceptance) => {
                report.largest_acceptance =
                    report.largest_acceptance.max(acceptance.as_json().len());
                report.accepted += 1;
                next += batch.len();
            }
            Answer::Rejected(rejection) => {
                let rejection = open_rejection(&rejection, &bookkeeper(), &pk(&service())).unwrap();
                let reason = rejection.reason;
                assert_eq!(reason.code, RejectionCode::TooLarge, "{}", reason.message);
                assert!(reason.message.contains("Send it as smaller proposals."));
                assert_eq!(reason.limit, Some(MAX_EVENT_BYTES as u64));
                // Nothing was stored.
                assert_eq!(books.seq, seq_before);
                size = shrink(batch.len(), reason.size, reason.limit);
                report.too_large += 1;
            }
        }
    }
    report.final_batch = size;
    report
}

#[test]
fn a_full_size_import_batched_by_size_stays_under_the_limit() {
    for text in [Text::Typical, Text::Long] {
        for history in [false, true] {
            let (start, _) = seed_business();
            let rows = preview(&start, &csv(FULL_IMPORT, text, history), history);
            let mut service = Service::new(start.clone());
            let report = send(&mut service, &rows);
            let form = if history {
                "expense history"
            } else {
                "payments"
            };
            eprintln!(
                "{text:?} text, {form}: {} accepted proposals, {} tooLarge rejections, final batch {} rows, largest proposal {} bytes, largest acceptance {} bytes",
                report.accepted,
                report.too_large,
                report.final_batch,
                report.largest_proposal,
                report.largest_acceptance
            );
            assert!(report.largest_proposal <= MAX_EVENT_BYTES);
            assert!(report.largest_acceptance <= MAX_EVENT_BYTES);
            let added = service.business.drafts.len() - start.drafts.len();
            assert_eq!(added, FULL_IMPORT);

            // Retrying a batch, as a client does when it never saw the
            // acknowledgment, changes nothing: its rows are already imported.
            let mut env = RecordingEnv::new(uuids(), "2023-03-02T09:00:00.000Z");
            let retry = commit_import(
                &service.business,
                &rows[..5],
                &start.accounts[0].id,
                &mut env,
            )
            .unwrap();
            assert!(diff(&service.business, &retry).unwrap().is_empty());
        }
    }
}

#[test]
fn an_oversized_event_is_refused_with_its_size() {
    let payload = ProposalPayload::new(
        business_id(),
        Some(0),
        Command::Correct {
            original_id: "draft-1".into(),
            replacement: seed_business().0.drafts[0].clone(),
            // About 50 KB of text: within NIP-44, but over the event limit
            // once encrypted and base64-encoded.
            reason: "x".repeat(50_000),
            documents: vec![],
            extraction: None,
        },
    );
    let err = build_proposal(&bookkeeper(), &pk(&service()), &payload, at(1)).unwrap_err();
    let ProtocolError::EventTooLarge { size, max } = err else {
        panic!("expected EventTooLarge, got {err:?}");
    };
    assert!(size > max);
    assert_eq!(max, MAX_EVENT_BYTES);
    assert_eq!(RejectionCode::of(&err), RejectionCode::TooLarge);
    assert!(err.to_string().contains("over the 65536-byte limit"));
}

#[test]
fn other_failures_are_not_too_large() {
    assert_eq!(
        RejectionCode::of(&ProtocolError::Changed),
        RejectionCode::Rejected
    );
    assert_eq!(
        RejectionCode::of(&ProtocolError::ContentTooLarge {
            size: 70_000,
            max: 65_535
        }),
        RejectionCode::TooLarge
    );
}
