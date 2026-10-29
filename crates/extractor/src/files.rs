//! The same file rules as `src/adapters/attachments.ts`: nonempty, at most
//! 10 MB, and the type taken from the first bytes, never the name. Images
//! must also declare at most [`MAX_PIXELS`]: a small file can declare a huge
//! canvas, which the model's decoder would then allocate.

use crate::error::ExtractError;

/// The largest receipt accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;
/// The most pixels an image may declare: 40 megapixels.
pub const MAX_PIXELS: u64 = 40_000_000;

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
    let kind = kind_of(bytes)?;
    if kind != FileKind::Pdf {
        check_pixels(bytes, kind)?;
    }
    Ok(kind)
}

fn kind_of(bytes: &[u8]) -> Result<FileKind, ExtractError> {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if starts(&PNG) {
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

const PNG: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// Refuses an image over [`MAX_PIXELS`], or one whose header doesn't say
/// its size: a decoder would have to guess, and so can't be bounded.
pub fn check_pixels(bytes: &[u8], kind: FileKind) -> Result<(), ExtractError> {
    let (width, height) = dimensions(bytes, kind).ok_or(ExtractError::UnsupportedType)?;
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ExtractError::ImageTooLarge);
    }
    Ok(())
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_be_bytes([*b.first()?, *b.get(1)?])))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(u16::from_le_bytes([*b.first()?, *b.get(1)?])))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([*b.first()?, *b.get(1)?, *b.get(2)?, 0]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([
        *b.first()?,
        *b.get(1)?,
        *b.get(2)?,
        *b.get(3)?,
    ]))
}

/// The width and height an image's header declares, without decoding it:
/// the PNG `IHDR` chunk, the JPEG start-of-frame segment, or the WebP
/// `VP8 `, `VP8L` or `VP8X` chunk. `None` for a PDF or a header that
/// doesn't say.
pub fn dimensions(bytes: &[u8], kind: FileKind) -> Option<(u32, u32)> {
    match kind {
        FileKind::Png => {
            // The first chunk must be IHDR: length 13, then the type.
            if bytes.get(8..16) != Some(b"\0\0\0\x0dIHDR".as_slice()) {
                return None;
            }
            let w = bytes.get(16..20)?;
            let h = bytes.get(20..24)?;
            Some((
                u32::from_be_bytes(w.try_into().ok()?),
                u32::from_be_bytes(h.try_into().ok()?),
            ))
        }
        FileKind::Jpeg => jpeg_dimensions(bytes),
        FileKind::Webp => {
            let data = 20;
            match bytes.get(12..16)? {
                b"VP8 " => {
                    // Frame tag (3 bytes), start code, then 14-bit sizes.
                    if bytes.get(data + 3..data + 6) != Some([0x9d, 0x01, 0x2a].as_slice()) {
                        return None;
                    }
                    Some((
                        le16(bytes, data + 6)? & 0x3fff,
                        le16(bytes, data + 8)? & 0x3fff,
                    ))
                }
                b"VP8L" => {
                    if bytes.get(data) != Some(&0x2f) {
                        return None;
                    }
                    let bits = le32(bytes, data + 1)?;
                    Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
                }
                b"VP8X" => Some((le24(bytes, 24)? + 1, le24(bytes, 27)? + 1)),
                _ => None,
            }
        }
        FileKind::Pdf => None,
    }
}

/// Walks the JPEG segments to the first start-of-frame marker.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        match marker {
            // Fill bytes before a marker.
            0xff => at += 1,
            // Markers without a length.
            0x01 | 0xd0..=0xd8 => at += 2,
            // Start of scan or end of image, before any frame header.
            0xd9 | 0xda => return None,
            // Every start-of-frame marker except DHT, JPG and DAC.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return Some((be16(bytes, at + 7)?, be16(bytes, at + 5)?));
            }
            _ => at += 2 + usize::try_from(be16(bytes, at + 2)?).ok()?,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_not_name() {
        assert_eq!(detect(&png(640, 480)), Ok(FileKind::Png));
        assert_eq!(detect(&jpeg(640, 480)), Ok(FileKind::Jpeg));
        assert_eq!(detect(&webp_lossy(640, 480)), Ok(FileKind::Webp));
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

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = PNG.to_vec();
        b.extend_from_slice(b"\0\0\0\x0dIHDR");
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    /// SOI, an APP0 segment, then a baseline frame header.
    fn jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut b = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x4a, 0x46];
        b.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        b.extend_from_slice(&height.to_be_bytes());
        b.extend_from_slice(&width.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b
    }

    fn webp(chunk: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend_from_slice(chunk);
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
        b
    }

    fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let mut data = vec![0x10, 0x02, 0x00, 0x9d, 0x01, 0x2a];
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        webp(b"VP8 ", &data)
    }

    fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
        let bits = (width - 1) | ((height - 1) << 14);
        let mut data = vec![0x2f];
        data.extend_from_slice(&bits.to_le_bytes());
        webp(b"VP8L", &data)
    }

    fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        webp(b"VP8X", &data)
    }

    #[test]
    fn image_dimensions_come_from_the_header() {
        assert_eq!(
            dimensions(&png(1234, 567), FileKind::Png),
            Some((1234, 567))
        );
        assert_eq!(
            dimensions(&jpeg(1234, 567), FileKind::Jpeg),
            Some((1234, 567))
        );
        assert_eq!(
            dimensions(&webp_lossy(1234, 567), FileKind::Webp),
            Some((1234, 567))
        );
        assert_eq!(
            dimensions(&webp_lossless(1234, 567), FileKind::Webp),
            Some((1234, 567))
        );
        assert_eq!(
            dimensions(&webp_extended(12_345, 6_789), FileKind::Webp),
            Some((12_345, 6_789))
        );
        // A progressive JPEG's frame header counts too.
        let mut progressive = jpeg(800, 600);
        progressive[9] = 0xc2;
        assert_eq!(dimensions(&progressive, FileKind::Jpeg), Some((800, 600)));
    }

    #[test]
    fn an_image_over_40_megapixels_is_refused() {
        // EXT-3: a few bytes can declare a canvas the decoder would allocate.
        for big in [
            png(60_000, 60_000),
            png(8_000, 5_001),
            jpeg(65_535, 65_535),
            webp_lossless(16_384, 16_384),
            webp_extended(10_000, 4_001),
        ] {
            assert_eq!(detect(&big), Err(ExtractError::ImageTooLarge));
        }
        // Exactly 40 megapixels is allowed.
        assert_eq!(detect(&png(8_000, 5_000)), Ok(FileKind::Png));
        assert_eq!(detect(&webp_extended(8_000, 5_000)), Ok(FileKind::Webp));
    }

    #[test]
    fn an_image_whose_header_hides_its_size_is_refused() {
        // EXT-3: without a size, nothing bounds what decoding allocates.
        let truncated = [
            b"\x89PNG\r\n\x1a\nrest".to_vec(),
            b"\xff\xd8\xff\xe0".to_vec(),
            b"RIFF\0\0\0\0WEBPVP8 ".to_vec(),
            // A JPEG that starts its scan before any frame header.
            vec![0xff, 0xd8, 0xff, 0xda, 0x00, 0x02],
        ];
        for bytes in truncated {
            assert_eq!(detect(&bytes), Err(ExtractError::UnsupportedType));
        }
    }
}
