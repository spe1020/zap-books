//! PDF receipts through the Poppler command-line tools: the text layer from
//! `pdftotext` and page images from `pdftoppm`, first three pages only.
//! Files live in a private temporary directory removed when done, and any
//! left by a process that was killed are removed when the service starts.
//!
//! A PDF of a few hundred bytes can declare a page of any size, so nothing
//! here is sized by the page: images are rendered to fit [`MAX_PAGE_PIXELS`]
//! and refused above [`MAX_PAGE_BYTES`], and the text layer is read no
//! further than [`MAX_TEXT_BYTES`].

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::error::ExtractError;
use crate::files::{check_pixels, FileKind};
use crate::prompt::MAX_TEXT_LAYER_CHARS;

/// The most pages read from a PDF.
pub const MAX_PAGES: u32 = 3;
/// Each page is rendered to fit a square this many pixels wide, whatever
/// size the PDF says it is.
pub const MAX_PAGE_PIXELS: u32 = 2000;
/// The largest rendered page accepted. A 2000-pixel page of a real receipt
/// is a few hundred kilobytes; this leaves room for photographs.
pub const MAX_PAGE_BYTES: u64 = 10 * 1024 * 1024;
/// The most text-layer bytes read: enough for the characters the prompt
/// keeps, at four bytes each.
pub const MAX_TEXT_BYTES: u64 = 4 * MAX_TEXT_LAYER_CHARS as u64;
/// Every temporary directory's name starts with this.
const TEMP_PREFIX: &str = "zapbooks-extractor-";
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
            "{TEMP_PREFIX}{}-{}-{nanos}",
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

/// The process id a temporary directory's name records.
fn owner_pid(name: &str) -> Option<u32> {
    name.strip_prefix(TEMP_PREFIX)?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// Whether process `pid` is running, so its directories are still in use
/// (by a bench run, say). Unknown counts as running.
fn running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_or(true, |s| s.success())
}

/// Removes temporary receipt directories left in `dir` by a process that
/// was killed before it could clean up, and returns how many. Called at
/// start-up. Directories of a process still running are left alone.
pub fn sweep_stale(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        // Only directories, never a link to one.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| owner_pid(&e.file_name().to_string_lossy()).is_some_and(|pid| !running(pid)))
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

/// What was read from a PDF.
pub struct PdfPages {
    /// PNG bytes of up to `MAX_PAGES` pages.
    pub images: Vec<Vec<u8>>,
    /// The text layer, if it has real text.
    pub text: Option<String>,
}

/// Reads at most `cap` bytes, and whether there was more.
async fn read_capped(
    reader: impl AsyncRead + Unpin,
    cap: u64,
) -> Result<(Vec<u8>, bool), ExtractError> {
    let mut bytes = Vec::new();
    reader
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| ExtractError::Internal)?;
    let more = bytes.len() as u64 > cap;
    bytes.truncate(usize::try_from(cap).unwrap_or(usize::MAX));
    Ok((bytes, more))
}

async fn text_layer(pdf: &Path) -> Result<Option<String>, ExtractError> {
    let mut child = Command::new("pdftotext")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ExtractError::PopplerMissing)?;
    let stdout = child.stdout.take().ok_or(ExtractError::Internal)?;
    let (bytes, more) = read_capped(stdout, MAX_TEXT_BYTES).await?;
    if more {
        // The rest would be cut by the prompt anyway: stop pdftotext
        // rather than let it write it.
        let _ = child.kill().await;
    } else {
        let status = child.wait().await.map_err(|_| ExtractError::Internal)?;
        if !status.success() {
            return Err(ExtractError::PdfUnreadable);
        }
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let visible = text.chars().filter(|c| !c.is_whitespace()).count();
    Ok((visible >= MIN_TEXT_CHARS).then_some(text))
}

async fn render(pdf: &Path, dir: &Path) -> Result<Vec<Vec<u8>>, ExtractError> {
    let status = Command::new("pdftoppm")
        .args([
            "-f",
            "1",
            "-l",
            &MAX_PAGES.to_string(),
            "-scale-to",
            &MAX_PAGE_PIXELS.to_string(),
            "-png",
        ])
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
        images.push(read_page(&page, MAX_PAGE_BYTES).await?);
    }
    if images.is_empty() {
        return Err(ExtractError::PdfUnreadable);
    }
    Ok(images)
}

/// Reads one rendered page, refusing it above `cap` bytes or
/// [`crate::files::MAX_PIXELS`] before it is loaded.
async fn read_page(page: &Path, cap: u64) -> Result<Vec<u8>, ExtractError> {
    let file = tokio::fs::File::open(page)
        .await
        .map_err(|_| ExtractError::Internal)?;
    let (bytes, more) = read_capped(file, cap).await?;
    if more {
        return Err(ExtractError::PdfUnreadable);
    }
    check_pixels(&bytes, FileKind::Png).map_err(|_| ExtractError::PdfUnreadable)?;
    Ok(bytes)
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reading_stops_at_the_cap() {
        // EXT-3: what isn't read is never buffered.
        let data = vec![b'x'; 1000];
        let (bytes, more) = read_capped(data.as_slice(), 100).await.unwrap();
        assert_eq!((bytes.len(), more), (100, true));
        let (bytes, more) = read_capped(data.as_slice(), 1000).await.unwrap();
        assert_eq!((bytes.len(), more), (1000, false));
    }

    #[tokio::test]
    async fn a_rendered_page_over_the_byte_cap_is_refused() {
        // EXT-3.
        let dir = TempDir::new().unwrap();
        let page = dir.0.join("page-1.png");
        std::fs::write(&page, vec![0u8; 2048]).unwrap();
        assert_eq!(
            read_page(&page, 1024).await.unwrap_err(),
            ExtractError::PdfUnreadable
        );
    }

    /// A process id that has exited.
    fn exited_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn stale_directories_are_swept_and_nothing_else() {
        // EXT-7: a killed process leaves its directory behind.
        let root = TempDir::new().unwrap();
        let stale = root.0.join(format!("{TEMP_PREFIX}{}-0-0", exited_pid()));
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("receipt.pdf"), b"%PDF-").unwrap();
        // This process's own, and those of anything else still running.
        let mine = root
            .0
            .join(format!("{TEMP_PREFIX}{}-0-0", std::process::id()));
        std::fs::create_dir(&mine).unwrap();
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let busy = root.0.join(format!("{TEMP_PREFIX}{}-0-0", sleeper.id()));
        std::fs::create_dir(&busy).unwrap();
        let other = root.0.join("someone-else");
        std::fs::create_dir(&other).unwrap();
        let file = root.0.join(format!("{TEMP_PREFIX}{}-file", exited_pid()));
        std::fs::write(&file, b"").unwrap();
        assert_eq!(sweep_stale(&root.0), 1);
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
        assert!(!stale.exists());
        assert!(mine.exists() && busy.exists() && other.exists() && file.exists());
    }

    #[tokio::test]
    async fn a_pdf_that_fails_leaves_no_files() {
        // EXT-7: every error path removes the temporary directory.
        if !poppler_available().await {
            eprintln!("Poppler not installed; skipping");
            return;
        }
        let before = own_dirs();
        assert_eq!(
            read(b"%PDF-1.4 not really a pdf").await.err(),
            Some(ExtractError::PdfUnreadable)
        );
        assert!(
            own_dirs().is_subset(&before),
            "a temporary directory was left"
        );
    }

    /// This process's temporary directories holding a receipt, which only
    /// `read` writes (other tests here make directories of their own).
    fn own_dirs() -> std::collections::BTreeSet<PathBuf> {
        let mine = format!("{TEMP_PREFIX}{}-", std::process::id());
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(&mine))
            .map(|e| e.path())
            .filter(|p| p.join("receipt.pdf").exists())
            .collect()
    }
}
