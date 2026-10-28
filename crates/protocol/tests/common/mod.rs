//! Shared test setup: fixed keys, a fictional business and a genesis.
#![allow(dead_code)]

use std::path::Path;

use ledger_core::{BusinessData, SequentialEnv, Workspace};
use nostr::prelude::{Event, Keys, PublicKey, SecretKey, Timestamp};
use protocol::{BusinessId, Command, ProposalPayload, Recovery, PROTOCOL_VERSION};
use serde_json::Value;

/// A fixed key, so failures are reproducible: secret `[n; 32]`.
pub fn key(n: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[n; 32]).unwrap())
}

pub fn service() -> Keys {
    key(1)
}
pub fn owner() -> Keys {
    key(2)
}
pub fn backup() -> Keys {
    key(3)
}
pub fn bookkeeper() -> Keys {
    key(4)
}
pub fn viewer() -> Keys {
    key(5)
}
pub fn submitter() -> Keys {
    key(6)
}
pub fn outsider() -> Keys {
    key(7)
}

pub fn business_id() -> BusinessId {
    BusinessId::from_bytes([0xab; 32])
}

pub fn at(secs: u64) -> Timestamp {
    Timestamp::from_secs(secs)
}

/// The first business in the parity seed (fictional), with an `Env` that
/// continues its id sequence.
pub fn seed_business() -> (BusinessData, SequentialEnv) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/parity/states/seed.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut workspace: Workspace = serde_json::from_value(value["workspace"].clone()).unwrap();
    let env = SequentialEnv::new(
        value["env"]["nextId"].as_u64().unwrap(),
        value["env"]["now"].as_str().unwrap(),
        value["env"]["today"].as_str().unwrap(),
    );
    (workspace.businesses.remove(0), env)
}

pub fn genesis_payload(recovery: Recovery) -> ProposalPayload {
    let (b, _) = seed_business();
    ProposalPayload {
        v: PROTOCOL_VERSION,
        business: business_id(),
        base_seq: None,
        command: Command::CreateBusiness {
            service: service().public_key(),
            recovery,
            profile: b.profile,
            accounts: b.accounts,
            categories: b.categories,
        },
    }
}

pub fn backup_recovery() -> Recovery {
    Recovery::BackupKey {
        pubkey: backup().public_key(),
    }
}

pub fn genesis_event() -> Event {
    protocol::build_proposal(
        &owner(),
        &service().public_key(),
        &genesis_payload(backup_recovery()),
        at(1_000),
    )
    .unwrap()
}

pub fn pk(keys: &Keys) -> PublicKey {
    keys.public_key()
}
