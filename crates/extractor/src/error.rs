//! Errors with the plain-language message the user sees.

use thiserror::Error;

/// Everything that can stop an extraction. `Display` is shown to the user,
/// so it never contains document contents.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExtractError {
    #[error("Choose a receipt file to read.")]
    MissingFile,
    #[error("The receipt file is empty.")]
    EmptyFile,
    #[error("The receipt is larger than 10 MB. Choose a smaller file.")]
    TooLarge,
    #[error(
        "Supported receipts: PNG, JPEG, WebP, or PDF. This file does not match a supported format."
    )]
    UnsupportedType,
    #[error("The receipt image is over 40 megapixels. Choose a smaller image.")]
    ImageTooLarge,
    #[error("The request was not understood: {0}")]
    BadRequest(&'static str),
    #[error("Reading PDF receipts needs Poppler. Install it with: brew install poppler")]
    PopplerMissing,
    #[error("This PDF couldn't be opened. Try a photo or screenshot of the receipt instead.")]
    PdfUnreadable,
    #[error("The local model isn't running. Start Ollama, or enter the details manually.")]
    OllamaUnavailable,
    #[error("The model {0} isn't installed. Run: ollama pull {0}")]
    ModelMissing(String),
    #[error("The model couldn't read this receipt. Try again, or enter the details manually.")]
    ModelFailed,
    #[error(
        "The model's answer couldn't be understood. Try again, or enter the details manually."
    )]
    MalformedAnswer,
    #[error(
        "The model took too long to read this receipt. Try again, or enter the details manually."
    )]
    Timeout,
    #[error(
        "The model is still finishing an earlier receipt. Try again in a minute, or enter the details manually."
    )]
    Busy,
    #[error("Receipt reading failed on this computer. Enter the details manually.")]
    Internal,
}

impl ExtractError {
    /// The HTTP status the service answers with.
    pub fn status(&self) -> u16 {
        match self {
            Self::MissingFile | Self::EmptyFile | Self::BadRequest(_) => 400,
            Self::TooLarge | Self::ImageTooLarge => 413,
            Self::UnsupportedType => 415,
            Self::PdfUnreadable => 422,
            Self::PopplerMissing | Self::OllamaUnavailable | Self::ModelMissing(_) | Self::Busy => {
                503
            }
            Self::ModelFailed | Self::MalformedAnswer => 502,
            Self::Timeout => 504,
            Self::Internal => 500,
        }
    }
}
