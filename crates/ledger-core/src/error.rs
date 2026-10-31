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
    #[error("Imported source evidence can only be added by an import.")]
    ImportSourceNotFromImport,
    #[error("Payment links can only be created by an import.")]
    PaymentNotFromImport,
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
    #[error("This payment belongs to a different expense.")]
    PaymentOfOtherExpense,

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
    #[error(
        "Import rows were previewed for a different business or account. Preview the file again."
    )]
    PreviewedElsewhere,
    #[error("Invalid import row.")]
    InvalidImportRow,
    #[error("Invalid import source.")]
    InvalidImportSource,
    #[error(
        "Import preview does not match this business or payment account. Preview the file again."
    )]
    InvalidImportScope,

    // settings.ts updateSettings
    #[error("Enter a business name and industry.")]
    ProfileRequired,
    #[error("Enter a name.")]
    NameRequired,
    #[error("That name is already in use.")]
    NameInUse,
    #[error("Account not found in this business.")]
    AccountNotFound,
    #[error("Category not found in this business.")]
    CategoryNotFound,
    #[error("Rename an expense account through its category.")]
    ExpenseAccountRename,
    #[error("Choose a bank or card account.")]
    AccountKindInvalid,

    // settings.ts saveRule
    #[error("Vendor and purpose are required.")]
    RuleFieldsRequired,
    #[error("Choose a category in this business.")]
    RuleCategory,
    #[error("A rule for this vendor already exists.")]
    DuplicateRule,

    // suggestions.ts suggest
    #[error("A suggestion needs its author's key.")]
    SuggestionAuthor,
    #[error("Draft not found in this business.")]
    DraftNotFound,
    #[error("Suggestions can only be attached to drafts.")]
    SuggestionNotDraft,
    #[error("A suggestion needs a category, a purpose or a note.")]
    SuggestionEmpty,
    #[error("A suggestion needs its evidence.")]
    SuggestionEvidence,

    // transfers.ts
    #[error("A valid transfer date is required.")]
    InvalidTransferDate,
    #[error("Amount must be greater than zero in whole minor units.")]
    TransferAmount,
    #[error("Purpose is required.")]
    TransferPurpose,
    #[error("Choose accounts in this business.")]
    TransferAccounts,
    #[error("Choose two different accounts.")]
    TransferSameAccount,
    #[error("Both accounts must be in the transfer's currency.")]
    TransferCurrency,
    /// The transfer type's own message: its accounts don't fit it.
    #[error("{0}")]
    TransferShape(&'static str),
    #[error("Receipt does not belong to this business.")]
    TransferReceipt,
    #[error("Record an opening balance from its own step.")]
    OpeningBalanceNotTransfer,
    #[error("A posted transfer and correction reason are required.")]
    TransferCorrectionRequiresPosted,
    #[error("This transfer has already been corrected.")]
    TransferAlreadyCorrected,
    #[error("Correct an opening balance with an opening balance, and a transfer with a transfer.")]
    TransferCorrectionKind,
    #[error("An opening balance correction keeps its account.")]
    OpeningBalanceAccountChanged,
    #[error("Choose an account in this business.")]
    OpeningBalanceAccountNotFound,
    #[error("Opening balances are for bank, card and loan accounts.")]
    OpeningBalanceAccountKind,
    #[error("Enter an opening balance other than zero.")]
    OpeningBalanceZero,
    #[error("This account already has an opening balance. Correct it instead.")]
    OpeningBalanceExists,
    #[error("An opening balance must be dated on or before the first entry, {0}.")]
    OpeningBalanceTooLate(String),
    #[error("Another account is named {0}. Rename it to record opening balances.")]
    OpeningBalancesNameTaken(String),
    #[error("Choose a valid date.")]
    InvalidBalanceDate,

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
