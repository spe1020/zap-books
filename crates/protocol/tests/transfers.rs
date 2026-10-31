//! Owner funding through the protocol: `RecordTransfer`, `CorrectTransfer`
//! and `RecordOpeningBalance`. Owners and bookkeepers may send them; each
//! other role, and the recovery key, has its own refusal test.

mod common;

use common::*;
use ledger_core::{AccountKind, BusinessData, Currency, Minor, SequentialEnv, TransferType};
use nostr::prelude::{EventId, Keys};
use protocol::{
    apply_changes, apply_command, diff, targets, Command, CommandType, History, MembershipChange,
    MembershipState, OpeningBalanceInput, ProtocolError, RecordKey, RecordType, RecordingEnv,
    ReplayEnv, Role, SettingsChange, TransferInput,
};

fn id(n: u8) -> EventId {
    EventId::from_slice(&[n; 32]).unwrap()
}

/// Owner and backup key, plus a bookkeeper, a viewer, a submitter and an
/// assistant.
fn team() -> MembershipState {
    let mut s = MembershipState::genesis(
        business_id(),
        pk(&owner()),
        pk(&service()),
        backup_recovery(),
    )
    .unwrap();
    for (n, (keys, role)) in [
        (bookkeeper(), Role::Bookkeeper),
        (viewer(), Role::Viewer),
        (submitter(), Role::Submitter),
        (assistant(), Role::Assistant),
    ]
    .into_iter()
    .enumerate()
    {
        let change = MembershipChange::Add {
            member: pk(&keys),
            role,
            history: History::Full,
        };
        s = s
            .propose(id(n as u8 + 10), &pk(&owner()), &change, 1_800_000_000)
            .unwrap()
            .state;
    }
    s
}

/// The seed's first business with a loan account added through the
/// protocol.
fn books() -> (BusinessData, SequentialEnv, String) {
    let (b, mut env) = seed_business();
    let change = Command::UpdateSettings {
        change: SettingsChange::AddAccount {
            name: "Loan from owner".into(),
            kind: AccountKind::Liability,
            currency: Currency::Usd,
        },
    };
    let b = apply_command(&b, &change, &mut env).unwrap();
    let loan = b.accounts.last().unwrap().id.clone();
    (b, env, loan)
}

fn loan(b: &BusinessData, loan: &str, amount: i64) -> TransferInput {
    TransferInput {
        date: "2026-10-20".into(),
        amount_minor: Minor::new(amount).unwrap(),
        currency: Currency::Usd,
        from_account_id: loan.into(),
        to_account_id: b.accounts[0].id.clone(),
        transfer_type: TransferType::OwnerLoan,
        purpose: "Invented working capital".into(),
        reference: None,
        document_ids: vec![],
    }
}

/// One of each command, against `b`.
fn commands(b: &BusinessData, loan_id: &str) -> [Command; 3] {
    [
        Command::RecordTransfer {
            transfer: loan(b, loan_id, 50_000),
        },
        Command::CorrectTransfer {
            original_id: b
                .journals
                .iter()
                .find(|j| j.kind == ledger_core::JournalKind::Transfer)
                .unwrap()
                .id
                .clone(),
            replacement: TransferInput {
                transfer_type: TransferType::BetweenAccounts,
                from_account_id: b.accounts[0].id.clone(),
                to_account_id: b.accounts[2].id.clone(),
                ..loan(b, loan_id, 1_000)
            },
            reason: "Invented reason".into(),
        },
        Command::RecordOpeningBalance {
            opening_balance: OpeningBalanceInput {
                account_id: loan_id.into(),
                date: "2026-01-01".into(),
                amount_minor: Minor::new(25_000).unwrap(),
                reference: None,
            },
        },
    ]
}

fn refused(signer: &Keys, why: ProtocolError) {
    let (b, _, loan_id) = books();
    let s = team();
    for command in commands(&b, &loan_id) {
        assert_eq!(
            s.authorize(&pk(signer), &command),
            Err(why.clone()),
            "{:?}",
            command.command_type()
        );
    }
}

#[test]
fn owners_and_bookkeepers_record_and_correct_transfers_and_opening_balances() {
    let s = team();
    for signer in [owner(), bookkeeper()] {
        let (b, mut env, loan_id) = books();
        for command in commands(&b, &loan_id) {
            s.authorize(&pk(&signer), &command).unwrap();
            let after = apply_command(&b, &command, &mut env).unwrap();
            assert!(after.journals.len() > b.journals.len());
        }
    }
}

#[test]
fn an_assistant_cannot_record_or_correct_transfers_or_opening_balances() {
    refused(&assistant(), ProtocolError::Forbidden);
}

#[test]
fn a_viewer_cannot_record_or_correct_transfers_or_opening_balances() {
    refused(&viewer(), ProtocolError::Forbidden);
}

#[test]
fn a_submitter_cannot_record_or_correct_transfers_or_opening_balances() {
    refused(&submitter(), ProtocolError::Forbidden);
}

#[test]
fn the_recovery_key_cannot_record_or_correct_transfers_or_opening_balances() {
    refused(&backup(), ProtocolError::Forbidden);
}

#[test]
fn an_outsider_cannot_record_or_correct_transfers_or_opening_balances() {
    refused(&outsider(), ProtocolError::NotMember);
}

#[test]
fn transfer_commands_have_their_wire_names() {
    let (b, _, loan_id) = books();
    for (command, name) in commands(&b, &loan_id).into_iter().zip([
        "RecordTransfer",
        "CorrectTransfer",
        "RecordOpeningBalance",
    ]) {
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["type"], name);
        assert_eq!(command.command_type().as_str(), name);
        assert_eq!(CommandType::parse(name), Some(command.command_type()));
        let back: Command = serde_json::from_value(json).unwrap();
        assert_eq!(back, command);
    }
    let json = serde_json::to_value(&commands(&b, &loan_id)[2]).unwrap();
    assert_eq!(json["openingBalance"]["accountId"], loan_id.as_str());
}

#[test]
fn a_transfer_rebuilds_from_its_changes_and_replays_to_the_same_changes() {
    let (b, _, loan_id) = books();
    let mut n = 0;
    let mut env = RecordingEnv::new(
        move || {
            n += 1;
            format!("svc-{n}")
        },
        "2026-10-21T09:00:00.000Z",
    );
    let command = Command::RecordTransfer {
        transfer: loan(&b, &loan_id, 50_000),
    };
    let after = apply_command(&b, &command, &mut env).unwrap();
    let changes = diff(&b, &after).unwrap();
    assert_eq!(apply_changes(&b, &changes).unwrap(), after);
    let record = env.into_record();
    let mut replay = ReplayEnv::new(&record);
    let again = apply_command(&b, &command, &mut replay).unwrap();
    replay.finish().unwrap();
    assert_eq!(diff(&b, &again).unwrap(), changes);
}

#[test]
fn a_transfer_correction_targets_its_original_and_records_target_nothing() {
    let (b, _, loan_id) = books();
    let [record, correct, opening] = commands(&b, &loan_id);
    assert!(targets(&record, &b).is_empty());
    assert!(targets(&opening, &b).is_empty());
    let Command::CorrectTransfer { original_id, .. } = &correct else {
        unreachable!()
    };
    assert_eq!(
        targets(&correct, &b),
        vec![RecordKey::new(RecordType::Journal, original_id.clone())]
    );
}
