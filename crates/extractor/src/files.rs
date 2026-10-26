//! The same file rules as `src/adapters/attachments.ts`: nonempty, at most
//! 10 MB, and the type taken from the first bytes, never the name.

use crate::error::ExtractError;

/// The largest receipt accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Png,
    Jpeg,
    Webp,
    Pdf,
}

impl FileKind {
    pub fn is_pdf(self) -> bool {
        self == FileKind::Pdf
    }
}

/// Checks size and detects the type from the file's first bytes.
pub fn detect(bytes: &[u8]) -> Result<FileKind, ExtractError> {
    if bytes.is_empty() {
        return Err(ExtractError::EmptyFile);
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ExtractError::TooLarge);
    }
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if starts(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        Ok(FileKind::Png)
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Ok(FileKind::Jpeg)
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Ok(FileKind::Webp)
    } else if starts(b"%PDF-") {
        Ok(FileKind::Pdf)
    } else {
        Err(ExtractError::UnsupportedType)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_not_name() {
        assert_eq!(detect(b"\x89PNG\r\n\x1a\nrest"), Ok(FileKind::Png));
        assert_eq!(detect(b"\xff\xd8\xff\xe0"), Ok(FileKind::Jpeg));
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 "), Ok(FileKind::Webp));
        assert_eq!(detect(b"%PDF-1.7"), Ok(FileKind::Pdf));
        assert_eq!(
            detect(b"RIFF\0\0\0\0WAVE"),
            Err(ExtractError::UnsupportedType)
        );
        assert_eq!(detect(b"<html>"), Err(ExtractError::UnsupportedType));
        assert_eq!(detect(b""), Err(ExtractError::EmptyFile));
        let big = vec![0x25; MAX_FILE_BYTES + 1];
        assert_eq!(detect(&big), Err(ExtractError::TooLarge));
    }
}
