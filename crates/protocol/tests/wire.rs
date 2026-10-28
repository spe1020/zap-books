//! The JSON names `docs/protocol.md` documents.

use protocol::{History, MembershipChange, Outcome, Recovery, RejectionCode, Role, SettingsChange};
use serde_json::{json, to_value};

mod common;
use common::*;

#[test]
fn json_names_match_the_spec() {
    assert_eq!(
        to_value(Outcome::Applied).unwrap(),
        json!({"type": "applied"})
    );
    assert_eq!(
        to_value(Outcome::Pending { effective_at: 9 }).unwrap(),
        json!({"type": "pending", "effectiveAt": 9})
    );
    assert_eq!(
        to_value(Outcome::Activated).unwrap(),
        json!({"type": "activated"})
    );
    assert_eq!(
        to_value(Outcome::Lapsed).unwrap(),
        json!({"type": "lapsed"})
    );

    let p = pk(&viewer());
    assert_eq!(
        to_value(Recovery::BackupKey { pubkey: p }).unwrap(),
        json!({"type": "backupKey", "pubkey": p.to_hex()})
    );
    assert_eq!(
        to_value(Recovery::SecondOwner { pubkey: p }).unwrap(),
        json!({"type": "secondOwner", "pubkey": p.to_hex()})
    );
    assert_eq!(
        to_value(MembershipChange::Add {
            member: p,
            role: Role::Viewer,
            history: History::FromJoin
        })
        .unwrap(),
        json!({"op": "add", "member": p.to_hex(), "role": "viewer", "history": "fromJoin"})
    );
    let defaulted: MembershipChange =
        serde_json::from_value(json!({"op": "add", "member": p.to_hex(), "role": "owner"}))
            .unwrap();
    assert_eq!(
        defaulted,
        MembershipChange::Add {
            member: p,
            role: Role::Owner,
            history: History::Full
        }
    );
    assert_eq!(
        to_value(MembershipChange::ReplaceRecoveryKey { key: None }).unwrap(),
        json!({"op": "replaceRecoveryKey", "key": null})
    );
    assert_eq!(
        to_value(SettingsChange::RenameCategory {
            id: "c".into(),
            name: "n".into()
        })
        .unwrap(),
        json!({"op": "renameCategory", "id": "c", "name": "n"})
    );
    assert_eq!(
        to_value(RejectionCode::TooLarge).unwrap(),
        json!("tooLarge")
    );
    assert_eq!(
        to_value(RejectionCode::Rejected).unwrap(),
        json!("rejected")
    );
}
