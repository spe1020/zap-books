//! Owner funding through the service (owner funding brief, Required
//! tests), against Postgres: a bookkeeper's transfer is accepted, an
//! assistant's is refused, and the books rebuild and audit from the chain.

mod common;

use common::*;
use ledger_core::{AccountKind, Currency, Minor, OpeningBalanceInput, TransferType};
use ledger_service::Status;
use protocol::{
    audit, rebuild, Command, History, MembershipChange, Role, SettingsChange, TransferInput,
};

#[tokio::test]
async fn a_bookkeepers_transfer_is_accepted_an_assistants_refused_and_both_rebuild_and_audit() {
    let w = World::new().await;
    w.create().await; // 0
    let bookkeeper = key(51);
    let assistant = key(52);
    for (keys, role) in [
        (&bookkeeper, Role::Bookkeeper),
        (&assistant, Role::Assistant),
    ] {
        let change = MembershipChange::Add {
            member: pk(keys),
            role,
            history: History::Full,
        };
        assert_eq!(w.change(&owner(), change).await.status, Status::Accepted);
    } // 1, 2
    let add_loan = Command::UpdateSettings {
        change: SettingsChange::AddAccount {
            name: "Loan from owner".into(),
            kind: AccountKind::Liability,
            currency: Currency::Usd,
        },
    };
    let ack = w.send(&bookkeeper, Some(2), add_loan).await;
    assert_eq!(ack.status, Status::Accepted, "{}", ack.text); // 3
    let books = w.snapshot().await;
    let loan = books.accounts.last().unwrap().id.clone();
    let bank = books
        .accounts
        .iter()
        .find(|a| a.kind == AccountKind::Bank && a.currency == Currency::Usd)
        .unwrap()
        .id
        .clone();
    let transfer = TransferInput {
        date: "2023-02-01".into(),
        amount_minor: Minor::new(50_000).unwrap(),
        currency: Currency::Usd,
        from_account_id: loan.clone(),
        to_account_id: bank.clone(),
        transfer_type: TransferType::OwnerLoan,
        purpose: "Invented working capital".into(),
        reference: None,
        document_ids: vec![],
        fiat_value: None,
        millisats: None,
    };

    // The assistant is refused, signed, and nothing is stored.
    let refused = w
        .send(
            &assistant,
            Some(3),
            Command::RecordTransfer {
                transfer: transfer.clone(),
            },
        )
        .await;
    assert_eq!(
        rejection(&refused, &assistant).reason.message,
        "Your role can't make this change."
    );
    assert_eq!(w.head().await, 3);

    // The bookkeeper records an opening balance, the loan, and a correction.
    let opening = Command::RecordOpeningBalance {
        opening_balance: OpeningBalanceInput {
            account_id: bank.clone(),
            date: "2023-01-01".into(),
            amount_minor: Minor::new(120_000).unwrap(),
            reference: None,
            fiat_value: None,
        },
    };
    let ack = w.send(&bookkeeper, Some(3), opening).await;
    assert_eq!(ack.status, Status::Accepted, "{}", ack.text); // 4
    let ack = w
        .send(
            &bookkeeper,
            Some(4),
            Command::RecordTransfer {
                transfer: transfer.clone(),
            },
        )
        .await;
    assert_eq!(ack.status, Status::Accepted, "{}", ack.text); // 5
    let original = w.snapshot().await.journals.last().unwrap().id.clone();
    let correction = Command::CorrectTransfer {
        original_id: original.clone(),
        replacement: TransferInput {
            amount_minor: Minor::new(40_000).unwrap(),
            ..transfer
        },
        reason: "Invented correction".into(),
    };
    let ack = w.send(&bookkeeper, Some(5), correction).await;
    assert_eq!(ack.status, Status::Accepted, "{}", ack.text); // 6

    let books = w.snapshot().await;
    let rows = ledger_core::balances(&books, "2023-12-31", Currency::Usd).unwrap();
    let balance = |id: &str| rows.iter().find(|r| r.account_id == id).unwrap().balance;
    assert_eq!(balance(&bank), Minor::new(160_000).unwrap());
    assert_eq!(balance(&loan), Minor::new(40_000).unwrap());

    // Rebuilding from the chain's record changes, and re-running every
    // command, both reproduce the stored books.
    let (_, contents) = w.read(&owner()).await;
    assert_eq!(contents.len(), 7);
    assert_eq!(rebuild(&contents).unwrap(), books);
    let mut audited = audit(None, &contents[0]).unwrap();
    for content in &contents[1..] {
        audited = audit(Some(&audited), content).unwrap();
    }
    assert_eq!(audited, books);
    w.finish().await;
}
