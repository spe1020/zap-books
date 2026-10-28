//! The environment values the service used for a command, so an auditor can
//! re-run it exactly.

use std::collections::VecDeque;

use ledger_core::Env;
use serde::{Deserialize, Serialize};

use crate::error::{ProtocolError, Result};

/// Recorded in every acceptance: each id the command generated, in order,
/// and the timestamp (`Env::now`). The service's `today()` is the UTC date
/// of `now`, so it is not recorded separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRecord {
    pub ids: Vec<String>,
    pub now: String,
}

/// The UTC calendar date of an ISO-8601 instant (`2026-10-15T…` → `2026-10-15`).
fn utc_date(now: &str) -> String {
    now.get(..10).unwrap_or(now).to_string()
}

/// The service's [`Env`]: ids from `generate`, a fixed `now`, and every id
/// handed out recorded for the acceptance.
pub struct RecordingEnv<G: FnMut() -> String> {
    generate: G,
    now: String,
    ids: Vec<String>,
}

impl<G: FnMut() -> String> RecordingEnv<G> {
    pub fn new(generate: G, now: impl Into<String>) -> RecordingEnv<G> {
        RecordingEnv {
            generate,
            now: now.into(),
            ids: Vec::new(),
        }
    }

    pub fn into_record(self) -> EnvRecord {
        EnvRecord {
            ids: self.ids,
            now: self.now,
        }
    }
}

impl<G: FnMut() -> String> Env for RecordingEnv<G> {
    fn new_id(&mut self) -> String {
        let id = (self.generate)();
        self.ids.push(id.clone());
        id
    }

    fn now(&self) -> String {
        self.now.clone()
    }

    fn today(&self) -> String {
        utc_date(&self.now)
    }
}

/// An auditor's [`Env`]: hands back the recorded ids in order. Call
/// [`ReplayEnv::finish`] afterwards: a replay that asked for more ids, or
/// fewer, than were recorded did not reproduce the original run.
pub struct ReplayEnv {
    ids: VecDeque<String>,
    now: String,
    overrun: bool,
}

impl ReplayEnv {
    pub fn new(record: &EnvRecord) -> ReplayEnv {
        ReplayEnv {
            ids: record.ids.iter().cloned().collect(),
            now: record.now.clone(),
            overrun: false,
        }
    }

    pub fn finish(self) -> Result<()> {
        if self.overrun || !self.ids.is_empty() {
            return Err(ProtocolError::BadChange(
                "the replay used a different number of ids than were recorded".into(),
            ));
        }
        Ok(())
    }
}

impl Env for ReplayEnv {
    fn new_id(&mut self) -> String {
        match self.ids.pop_front() {
            Some(id) => id,
            None => {
                self.overrun = true;
                String::new()
            }
        }
    }

    fn now(&self) -> String {
        self.now.clone()
    }

    fn today(&self) -> String {
        utc_date(&self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_hands_back_recorded_ids_and_checks_the_count() {
        let mut n = 0;
        let mut rec = RecordingEnv::new(
            || {
                n += 1;
                format!("gen-{n}")
            },
            "2026-03-04T05:06:07.000Z",
        );
        let a = rec.new_id();
        let b = rec.new_id();
        assert_eq!(rec.today(), "2026-03-04");
        let record = rec.into_record();
        assert_eq!(record.ids, vec![a.clone(), b.clone()]);

        let mut replay = ReplayEnv::new(&record);
        assert_eq!((replay.new_id(), replay.new_id()), (a, b));
        assert!(replay.finish().is_ok());

        let mut short = ReplayEnv::new(&record);
        short.new_id();
        assert!(short.finish().is_err());

        let mut long = ReplayEnv::new(&record);
        for _ in 0..3 {
            long.new_id();
        }
        assert!(long.finish().is_err());
    }
}
