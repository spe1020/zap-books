//! The two side effects the TypeScript reaches for directly:
//! `crypto.randomUUID()` and `new Date()`. Commands take them through
//! [`Env`] so callers, tests, and parity fixtures can make them deterministic.

/// Source of new record ids and the current time.
pub trait Env {
    /// A fresh, unique record id (`crypto.randomUUID()`).
    fn new_id(&mut self) -> String;
    /// The current instant as ISO-8601 in the exact format of JavaScript's
    /// `Date.prototype.toISOString()`, e.g. `2026-10-15T12:00:00.000Z`.
    fn now(&self) -> String;
}

/// `today()`: the date part of [`Env::now`].
pub(crate) fn today(env: &impl Env) -> String {
    let now = env.now();
    now.get(..10).unwrap_or(&now).to_string()
}

/// A deterministic [`Env`]: ids `id-0001`, `id-0002`, … and a fixed clock.
///
/// This is the generator the parity fixtures were recorded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequentialEnv {
    next: u64,
    now: String,
}

impl SequentialEnv {
    /// `next` is the number the next id will carry.
    pub fn new(next: u64, now: impl Into<String>) -> SequentialEnv {
        SequentialEnv {
            next,
            now: now.into(),
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
}
