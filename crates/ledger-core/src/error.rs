//! Every error the ledger can raise.
//!
//! Each `Display` string is copied character for character from the `throw new
//! Error(...)` it ports, because the UI shows these messages and the parity
//! fixtures compare them. The source location is noted on each variant.

use thiserror::Error;

/// The single error type for every fallible ledger operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LedgerError {
    // ledger.ts parseMoney / sum
    #[error("Use an amount with up to two decimal places, such as 42.50.")]
    InvalidAmount,
    #[error("Amount is too large.")]
    AmountTooLarge,
    #[error("Financial total exceeds supported range.")]
    TotalOutOfRange,

    // ledger.ts assertBalanced
    #[error("Invalid journal context.")]
    InvalidJournalContext,
    #[error("Invalid journal account or currency.")]
    InvalidJournalAccount,
    #[error("Invalid journal amounts.")]
    InvalidJournalAmounts,
    #[error("Invalid journal category.")]
    InvalidJournalCategory,
    #[error("Journal entry must balance.")]
    Unbalanced,

    // ledger.ts saveDraft
    #[error("Import source belongs to another business.")]
    ForeignImportSource,
    #[error("Imported source evidence cannot be changed.")]
    ImportSourceChanged,
    #[error("Only drafts in this business can be edited.")]
    NotEditable,
    #[error("Invalid receipt reference.")]
    InvalidReceipt,

    // ledger.ts approve
    #[error("Expense not found in this business.")]
    ExpenseNotFound,
    #[error("This expense has already been posted.")]
    AlreadyPosted,
    #[error("Duplicate posting prevented.")]
    DuplicatePosting,
    /// `approve` throws `errors.join(" ")` from `validation`.
    #[error("{}", .0.join(" "))]
    Validation(Vec<String>),
    #[error("This imported source already has a posted expense.")]
    ImportSourceAlreadyPosted,
    #[error("This payment is already linked to a posted expense.")]
    PaymentAlreadyLinked,

    // ledger.ts correct
    #[error("A posted expense and correction reason are required.")]
    CorrectionRequiresPosted,
    #[error("Invalid replacement.")]
    InvalidReplacement,
    #[error("Preserve the payment link when correcting an expense.")]
    PaymentLinkChanged,
    #[error("Preserve import source evidence when correcting an expense.")]
    ImportSourceNotPreserved,

    // import.ts parseCSV
    #[error("CSV must be smaller than 2 MB.")]
    CsvTooLarge,
    #[error("Malformed CSV quote.")]
    MalformedQuote,
    #[error("Unexpected text after a quoted value.")]
    TextAfterQuote,
    #[error("Unclosed quote in CSV.")]
    UnclosedQuote,
    #[error("CSV needs a header and at least one transaction.")]
    CsvTooShort,
    #[error("CSV headers must be unique and nonempty.")]
    InvalidHeaders,
    #[error("Import up to 2,000 transactions at a time.")]
    TooManyRows,

    // import.ts previewImport (row errors)
    #[error("File contains multiple payment accounts. Export one account at a time.")]
    MultipleSourceAccounts,
    #[error("Select a payment account.")]
    SelectPaymentAccount,
    #[error("Column count does not match header.")]
    ColumnCount,
    #[error("Map date, description, and amount.")]
    MappingIncomplete,
    #[error("Date does not match the selected format or is invalid.")]
    InvalidImportDate,
    #[error("Description is required.")]
    DescriptionRequired,
    #[error("Currency must match the selected account.")]
    ImportCurrency,
    #[error(
        "Only paid expense rows are supported; remove bills, income, transfers, and subtotal rows."
    )]
    UnsupportedTransactionType,
    #[error("This is not an expense outflow under the selected sign convention.")]
    NotAnOutflow,
    #[error("Existing reference has conflicting transaction details.")]
    ConflictingReference,
    #[error("Combined import amount exceeds the supported range.")]
    CombinedAmountOutOfRange,

    // import.ts commitImport
    #[error("Resolve invalid CSV rows before importing.")]
    UnresolvedRows,
    #[error("Payment account not found.")]
    PaymentAccountNotFound,
    #[error("Invalid import row.")]
    InvalidImportRow,
    #[error("Invalid import source.")]
    InvalidImportSource,
    #[error(
        "Import preview does not match this business or payment account. Preview the file again."
    )]
    InvalidImportScope,

    // export.ts exportRecords
    #[error("Choose valid export dates.")]
    InvalidExportDates,
    #[error("The start date must be on or before the end date.")]
    ExportRangeReversed,

    /// A state the TypeScript reaches only through a non-null assertion (`!`)
    /// or a property read on `undefined`, where it would crash with a
    /// `TypeError`. Rust returns this instead of panicking. See
    /// PARITY_NOTES.md for each site.
    #[error("Internal ledger error: {0}")]
    Internal(&'static str),
}

pub type Result<T> = std::result::Result<T, LedgerError>;
