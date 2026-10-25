//! Shared test setup: the deterministic seed recorded by the fixture
//! generator (`fixtures/parity/states/seed.json`).
#![allow(dead_code)]

use std::path::Path;

use ledger_core::{BusinessData, Minor, SequentialEnv, Workspace};
use serde_json::Value;

/// `currentPeriod()` under the generator's frozen clock (2026-10-15).
pub const PERIOD: &str = "2026-10";

/// `seedWorkspace().businesses`, with an `Env` that continues the id sequence.
pub fn seed() -> (Vec<BusinessData>, SequentialEnv) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/parity/states/seed.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let workspace: Workspace = serde_json::from_value(value["workspace"].clone()).unwrap();
    let env = SequentialEnv::new(
        value["env"]["nextId"].as_u64().unwrap(),
        value["env"]["now"].as_str().unwrap(),
    );
    (workspace.businesses, env)
}

/// `seedWorkspace().businesses[0]` (Northstar Studio).
pub fn fresh() -> (BusinessData, SequentialEnv) {
    let (mut businesses, env) = seed();
    (businesses.remove(0), env)
}

pub fn minor(n: i64) -> Minor {
    Minor::new(n).unwrap()
}
