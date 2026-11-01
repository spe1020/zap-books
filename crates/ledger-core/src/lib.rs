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
mod documents;
mod env;
mod error;
mod export;
mod extraction;
mod import;
mod js;
mod ledger;
mod money;
mod settings;
mod suggestions;
mod transfers;
mod types;

pub use date::{normalize_import_date, valid_date, DateFormat};
pub use documents::{check_document, MAX_RECEIPT_BYTES, MAX_STORED_BYTES, STORED_ALGORITHMS};
pub use env::{Env, SequentialEnv};
pub use error::{LedgerError, Result};
pub use export::{
    encode_csv, export_balances, export_records, CsvExport, ExportFilter, ExportFormat,
};
pub use extraction::outcomes_for;
pub use import::{
    commit_import, guess_mapping, parse_csv, preview_import, CsvMapping, ImportKind, ImportOptions,
    ImportRow, PreviewSource, SignConvention,
};
pub use ledger::{
    approve, assert_balanced, blank_draft, category_totals, correct, current_period, duplicates,
    event, missing_details, save_draft, totals, validation, CategoryTotal, ACTOR,
};
pub use money::{amount_input, parse_money, Currency, Minor, MAX_MINOR};
pub use settings::{save_rule, update_settings, RuleInput, SettingsChange};
pub use suggestions::{suggest, SuggestionInput};
pub use transfers::{
    balances, correct_transfer, opening_balances_name, record_opening_balance, record_transfer,
    transfer_type_label, Balance, OpeningBalanceInput, TransferInput,
};
pub use types::{
    Account, AccountKind, Approval, AuditEvent, Business, BusinessData, Category,
    ClassificationRule, DocumentKind, EventType, ExpenseDraft, ExpenseStatus, ExtractedField,
    ExtractedLine, ExtractedLines, ExtractionFieldName, ExtractionFields, ExtractionOutcome,
    ExtractionRecord, FieldStatus, FinancialEvent, ImportProvider, ImportedExpenseSource,
    ImportedTransaction, JournalEntry, JournalKind, JournalLine, PayloadValue, SchemaVersion,
    SourceDocument, StoredFile, Suggestion, TransferType, UncheckedJournal, Vendor, Workspace,
};
