//! The side effects the TypeScript reaches for directly:
//! `crypto.randomUUID()`, `new Date()`, and the user's local date. Commands
//! take them through [`Env`] so callers, tests, and parity fixtures can make
//! them deterministic.

/// Source of new record ids, the current time, and the user's local date.
pub trait Env {
    /// A fresh, unique record id (`crypto.randomUUID()`).
    fn new_id(&mut self) -> String;
    /// The current instant as ISO-8601 in the exact format of JavaScript's
    /// `Date.prototype.toISOString()`, e.g. `2026-10-15T12:00:00.000Z`.
    fn now(&self) -> String;
    /// `today()`: the user's local calendar date as `YYYY-MM-DD`. The crate
    /// does not compute time zones; the caller supplies the date, which can
    /// differ from the date part of [`Env::now`].
    fn today(&self) -> String;
}

/// A deterministic [`Env`]: ids `id-0001`, `id-0002`, … and a fixed clock and
/// local date.
///
/// This is the generator the parity fixtures were recorded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequentialEnv {
    next: u64,
    now: String,
    today: String,
}

impl SequentialEnv {
    /// `next` is the number the next id will carry; `today` is the local date
    /// at `now`.
    pub fn new(next: u64, now: impl Into<String>, today: impl Into<String>) -> SequentialEnv {
        SequentialEnv {
            next,
            now: now.into(),
            today: today.into(),
        }
    }

    /// The number the next id will carry.
    pub fn next(&self) -> u64 {
        self.next
    }
}

impl Env for SequentialEnv {
    fn new_id(&mut self) -> String {
        let id = format!("id-{:04}", self.next);
        self.next += 1;
        id
    }

    fn now(&self) -> String {
        self.now.clone()
    }

    fn today(&self) -> String {
        self.today.clone()
    }
}
