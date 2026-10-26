//! PDF receipts through the Poppler command-line tools: the text layer from
//! `pdftotext` and page images from `pdftoppm`, first three pages only.
//! Files live in a private temporary directory removed when done.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::process::Command;

use crate::error::ExtractError;

/// The most pages read from a PDF.
pub const MAX_PAGES: u32 = 3;
/// Rendering resolution for page images.
const DPI: &str = "150";
/// A text layer with fewer non-space characters than this is ignored.
const MIN_TEXT_CHARS: usize = 20;

/// Whether `pdftotext` and `pdftoppm` can be run.
pub async fn poppler_available() -> bool {
    async fn runs(tool: &str) -> bool {
        Command::new(tool)
            .arg("-v")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success())
    }
    runs("pdftotext").await && runs("pdftoppm").await
}

/// A temporary directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<TempDir, ExtractError> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "zapbooks-extractor-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = std::fs::DirBuilder::new();
        // Only this user may read the receipt while it is on disk.
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path).map_err(|_| ExtractError::Internal)?;
        Ok(TempDir(path))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What was read from a PDF.
pub struct PdfPages {
    /// PNG bytes of up to `MAX_PAGES` pages.
    pub images: Vec<Vec<u8>>,
    /// The text layer, if it has real text.
    pub text: Option<String>,
}

async fn text_layer(pdf: &Path) -> Result<Option<String>, ExtractError> {
    let output = Command::new("pdftotext")
        .args([
            "-f",
            "1",
            "-l",
            &MAX_PAGES.to_string(),
            "-layout",
            "-enc",
            "UTF-8",
        ])
        .arg(pdf)
        .arg("-")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|_| ExtractError::PopplerMissing)?;
    if !output.status.success() {
        return Err(ExtractError::PdfUnreadable);
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let visible = text.chars().filter(|c| !c.is_whitespace()).count();
    Ok((visible >= MIN_TEXT_CHARS).then_some(text))
}

async fn render(pdf: &Path, dir: &Path) -> Result<Vec<Vec<u8>>, ExtractError> {
    let status = Command::new("pdftoppm")
        .args(["-f", "1", "-l", &MAX_PAGES.to_string(), "-r", DPI, "-png"])
        .arg(pdf)
        .arg(dir.join("page"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .status()
        .await
        .map_err(|_| ExtractError::PopplerMissing)?;
    if !status.success() {
        return Err(ExtractError::PdfUnreadable);
    }
    let mut pages: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|_| ExtractError::Internal)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("page") && n.ends_with(".png"))
        })
        .collect();
    // pdftoppm pads page numbers to the same width, so names sort in order.
    pages.sort();
    let mut images = Vec::with_capacity(pages.len());
    for page in pages.into_iter().take(MAX_PAGES as usize) {
        images.push(
            tokio::fs::read(page)
                .await
                .map_err(|_| ExtractError::Internal)?,
        );
    }
    if images.is_empty() {
        return Err(ExtractError::PdfUnreadable);
    }
    Ok(images)
}

/// Reads up to the first three pages of a PDF.
pub async fn read(bytes: &[u8]) -> Result<PdfPages, ExtractError> {
    let dir = TempDir::new()?;
    let pdf = dir.0.join("receipt.pdf");
    tokio::fs::write(&pdf, bytes)
        .await
        .map_err(|_| ExtractError::Internal)?;
    let text = text_layer(&pdf).await?;
    let images = render(&pdf, &dir.0).await?;
    Ok(PdfPages { images, text })
}
