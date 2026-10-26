//! Local receipt extraction for Zap Books.
//!
//! A small HTTP service bound to `127.0.0.1`. It checks an uploaded receipt,
//! renders PDF pages with Poppler, asks a local Ollama vision model for a fixed
//! JSON schema, and then validates and normalizes every value itself with
//! `ledger-core`'s money and date rules. The model proposes; nothing here
//! changes any record.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

pub mod bench;
pub mod config;
pub mod error;
pub mod extract;
pub mod files;
pub mod log;
pub mod normalize;
pub mod ollama;
pub mod pdf;
pub mod prompt;
pub mod server;

pub use config::Config;
pub use error::ExtractError;
pub use extract::{Category, ExtractionResult, Extractor};
