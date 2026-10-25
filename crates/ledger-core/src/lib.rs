//! Zap Books accounting rules.
//!
//! A behavior-preserving port of the TypeScript in `src/domain/`. The
//! TypeScript is the specification; `PARITY_NOTES.md` records every place the
//! two knowingly differ.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod date;
mod error;
mod js;
mod money;

pub use date::{normalize_import_date, valid_date, DateFormat};
pub use error::{LedgerError, Result};
pub use money::{amount_input, parse_money, Currency, Minor, MAX_MINOR};
