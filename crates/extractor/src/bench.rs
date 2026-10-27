//! The model benchmark: runs the real extraction pipeline over a corpus of
//! labelled receipts and reports accuracy, unclear rate, confident-wrong rate,
//! and speed per field and model.
//!
//! Corpus layout: any folder tree of receipts (`.pdf`, `.png`, `.jpg`,
//! `.jpeg`, `.webp`). Each receipt is labelled by `<name>.expected.json` next
//! to it, or by `expected.json` in its folder when the folder holds one
//! receipt. An optional `categories.json` at the corpus root lists
//! `{ "id", "name" }` categories sent to the model.
//!
//! The Markdown report contains only aggregate numbers, never receipt values
//! or file names, so it can be pasted into a PR.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ledger_core::{Currency, ExtractedField, ExtractedLines, FieldStatus, Minor};
use serde::Deserialize;
use serde_json::Value;

use crate::error::ExtractError;
use crate::extract::{Category, ExtractionResult, Extractor};
use crate::normalize::parse_amount;

pub const FIELDS: [&str; 9] = [
    "total",
    "date",
    "vendor",
    "currency",
    "reference",
    "subtotal",
    "tax",
    "tip",
    "category",
];

/// One receipt's labels. A missing key is not scored; `null` means the field
/// is not on the receipt (so "found" is wrong).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Expected {
    #[serde(default, deserialize_with = "present")]
    pub vendor: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub date: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub total: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub currency: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub reference: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub subtotal: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub tax: Option<Value>,
    /// `null` when the receipt has no tip line.
    #[serde(default, deserialize_with = "present")]
    pub tip: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    pub category: Option<Value>,
    /// Counts toward "Total on clear receipts". Defaults to true.
    #[serde(default = "yes")]
    pub clear: bool,
    /// A free-form tag such as "thermal" or "pdf", reported as counts.
    #[serde(default)]
    pub kind: Option<String>,
}

fn yes() -> bool {
    true
}

/// Keeps an explicit `null` as `Some(Value::Null)`, unlike `Option`'s default.
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

impl Expected {
    fn get(&self, field: &str) -> Option<&Value> {
        match field {
            "vendor" => self.vendor.as_ref(),
            "date" => self.date.as_ref(),
            "total" => self.total.as_ref(),
            "currency" => self.currency.as_ref(),
            "reference" => self.reference.as_ref(),
            "subtotal" => self.subtotal.as_ref(),
            "tax" => self.tax.as_ref(),
            "tip" => self.tip.as_ref(),
            "category" => self.category.as_ref(),
            _ => None,
        }
    }
}

/// A verdict and whether the receipt shows the field at all.
pub type Scored = (Verdict, bool);

/// How one field of one receipt came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Found and right.
    Correct,
    /// Found but wrong, or found when the receipt doesn't show it.
    ConfidentWrong,
    Unclear,
    /// Absent, or the receipt couldn't be read at all.
    Missed,
    /// Not on the receipt, and not found: the right answer.
    RightlyAbsent,
}

fn letters(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn amount(v: &Value) -> Option<Minor> {
    match v {
        Value::String(s) => parse_amount(s).ok(),
        _ => None,
    }
}

/// Whether a found value matches the label.
fn matches(field: &str, expected: &Value, found: &Value) -> bool {
    match field {
        "total" | "subtotal" | "tax" | "tip" => {
            let got = found.as_i64().and_then(|n| Minor::new(n).ok());
            got.is_some() && got == amount(expected)
        }
        "vendor" => {
            let got = found.as_str().map(letters);
            let options: Vec<String> = match expected {
                Value::Array(list) => list.iter().filter_map(Value::as_str).map(letters).collect(),
                Value::String(s) => vec![letters(s)],
                _ => Vec::new(),
            };
            got.is_some_and(|g| options.contains(&g))
        }
        "reference" => {
            let norm = |s: &str| {
                s.chars()
                    .filter(|c| !c.is_whitespace())
                    .flat_map(char::to_uppercase)
                    .collect::<String>()
            };
            matches!((expected.as_str(), found.as_str()), (Some(e), Some(f)) if norm(e) == norm(f))
        }
        _ => expected == found,
    }
}

fn verdict_for(field: &str, expected: &Value, status: FieldStatus, value: &Value) -> Verdict {
    let on_receipt = !expected.is_null();
    match status {
        FieldStatus::Found if on_receipt && matches(field, expected, value) => Verdict::Correct,
        FieldStatus::Found => Verdict::ConfidentWrong,
        FieldStatus::Unclear => Verdict::Unclear,
        FieldStatus::Absent if on_receipt => Verdict::Missed,
        FieldStatus::Absent => Verdict::RightlyAbsent,
    }
}

fn status_and_value<T: serde::Serialize>(f: &ExtractedField<T>) -> (FieldStatus, Value) {
    (
        f.status,
        f.value
            .as_ref()
            .and_then(|v| serde_json::to_value(v).ok())
            .unwrap_or(Value::Null),
    )
}

/// A list is scored by the sum of its lines.
fn status_and_sum(f: &ExtractedLines) -> (FieldStatus, Value) {
    (
        f.status,
        f.value.map_or(Value::Null, |v| Value::from(v.get())),
    )
}

/// Scores every labelled field of one result.
pub fn score(expected: &Expected, result: &ExtractionResult) -> BTreeMap<&'static str, Scored> {
    let f = &result.fields;
    let mut out = BTreeMap::new();
    for field in FIELDS {
        let Some(label) = expected.get(field) else {
            continue;
        };
        let (status, value) = match field {
            "vendor" => status_and_value(&f.vendor),
            "date" => status_and_value(&f.date),
            "total" => status_and_value(&f.total),
            "currency" => status_and_value(&f.currency),
            "reference" => status_and_value(&f.reference),
            "subtotal" => status_and_value(&f.subtotal),
            "tax" => status_and_sum(&f.tax),
            "tip" => status_and_value(&f.tip),
            _ => status_and_value(&f.category),
        };
        out.insert(
            field,
            (verdict_for(field, label, status, &value), !label.is_null()),
        );
    }
    out
}

/// Receipts read with no result count as missed for every labelled field.
pub fn score_failure(expected: &Expected) -> BTreeMap<&'static str, Scored> {
    FIELDS
        .iter()
        .filter_map(|f| {
            expected.get(f).map(|label| {
                let on_receipt = !label.is_null();
                let verdict = if on_receipt {
                    Verdict::Missed
                } else {
                    Verdict::RightlyAbsent
                };
                (*f, (verdict, on_receipt))
            })
        })
        .collect()
}

/// A receipt in the corpus.
#[derive(Debug, Clone)]
pub struct Receipt {
    pub path: PathBuf,
    pub expected: Expected,
}

fn is_receipt(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("pdf" | "png" | "jpg" | "jpeg" | "webp")
    )
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out)?;
        } else if is_receipt(&path) {
            out.push(path);
        }
    }
    Ok(())
}

/// Finds every labelled receipt. Unlabelled receipts are returned separately.
pub fn load_corpus(dir: &Path) -> Result<(Vec<Receipt>, Vec<PathBuf>), String> {
    let mut files = Vec::new();
    walk(dir, &mut files).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
    let mut receipts = Vec::new();
    let mut unlabelled = Vec::new();
    for path in files {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let folder = path.parent().unwrap_or(dir);
        let named = folder.join(format!("{stem}.expected.json"));
        let shared = folder.join("expected.json");
        let label = if named.exists() {
            named
        } else if shared.exists() {
            shared
        } else {
            unlabelled.push(path);
            continue;
        };
        let text = std::fs::read_to_string(&label)
            .map_err(|e| format!("Could not read {}: {e}", label.display()))?;
        let expected: Expected = serde_json::from_str(&text)
            .map_err(|e| format!("{} is not a valid label: {e}", label.display()))?;
        receipts.push(Receipt { path, expected });
    }
    Ok((receipts, unlabelled))
}

/// Categories sent with every receipt: `categories.json` at the corpus root.
pub fn load_categories(dir: &Path) -> Result<Vec<Category>, String> {
    let path = dir.join("categories.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a category list: {e}", path.display()))
}

/// One receipt's outcome for one model.
#[derive(Debug, Clone)]
pub struct Run {
    pub receipt: usize,
    pub millis: u64,
    pub text_pdf: bool,
    pub error: Option<ExtractError>,
    pub verdicts: BTreeMap<&'static str, Scored>,
}

/// Runs every receipt through the extractor, after one warm-up call that
/// loads the model. Returns the runs and the warm-up time.
pub async fn run_model(
    extractor: &Extractor,
    receipts: &[Receipt],
    categories: &[Category],
    mut progress: impl FnMut(usize, usize),
) -> (Vec<Run>, Option<u64>) {
    let mut warmup = None;
    if let Some(first) = receipts.first() {
        if let Ok(bytes) = std::fs::read(&first.path) {
            let started = Instant::now();
            let _ = extractor.extract(&bytes, categories, &Currency::ALL).await;
            warmup = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        }
    }
    let mut runs = Vec::with_capacity(receipts.len());
    for (i, receipt) in receipts.iter().enumerate() {
        progress(i + 1, receipts.len());
        let started = Instant::now();
        let outcome = match std::fs::read(&receipt.path) {
            Ok(bytes) => extractor.extract(&bytes, categories, &Currency::ALL).await,
            Err(_) => Err(ExtractError::Internal),
        };
        let millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        runs.push(match outcome {
            Ok(result) => Run {
                receipt: i,
                millis,
                text_pdf: result.used_text_layer,
                error: None,
                verdicts: score(&receipt.expected, &result),
            },
            Err(e) => Run {
                receipt: i,
                millis,
                text_pdf: false,
                error: Some(e),
                verdicts: score_failure(&receipt.expected),
            },
        });
    }
    (runs, warmup)
}

/// Accuracy, unclear and confident-wrong rates for one field.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FieldStats {
    /// Receipts where the field is labelled.
    pub scored: usize,
    /// Of those, receipts that show the field.
    pub present: usize,
    pub correct: usize,
    pub unclear: usize,
    pub wrong: usize,
}

impl FieldStats {
    pub fn add(&mut self, (verdict, on_receipt): Scored) {
        self.scored += 1;
        if on_receipt {
            self.present += 1;
        }
        match verdict {
            Verdict::Correct => self.correct += 1,
            Verdict::Unclear => self.unclear += 1,
            Verdict::ConfidentWrong => self.wrong += 1,
            Verdict::Missed | Verdict::RightlyAbsent => {}
        }
    }
}

fn pct(n: usize, d: usize) -> String {
    if d == 0 {
        "n/a".into()
    } else {
        format!("{:.1}% ({n}/{d})", n as f64 * 100.0 / d as f64)
    }
}

fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

fn seconds(ms: Option<u64>) -> String {
    ms.map_or("n/a".into(), |ms| format!("{:.1} s", ms as f64 / 1000.0))
}

/// The starting targets from the stage 1 brief, as (field, accuracy, max
/// confident-wrong). Total accuracy is judged on clear receipts.
pub const TARGETS: [(&str, f64, f64); 4] = [
    ("total", 0.95, 0.01),
    ("date", 0.95, 0.01),
    ("vendor", 0.90, 0.03),
    ("currency", 0.98, 0.01),
];
pub const MEDIAN_TARGET_MS: u64 = 20_000;

/// The Markdown report for one model.
pub fn report(
    model: &str,
    receipts: &[Receipt],
    runs: &[Run],
    warmup: Option<u64>,
    memory_bytes: Option<u64>,
) -> String {
    let mut stats: BTreeMap<&str, FieldStats> = BTreeMap::new();
    let mut clear_total = FieldStats::default();
    for run in runs {
        for (field, verdict) in &run.verdicts {
            stats.entry(field).or_default().add(*verdict);
        }
        let clear = receipts.get(run.receipt).is_some_and(|r| r.expected.clear);
        if let (true, Some(v)) = (clear, run.verdicts.get("total")) {
            clear_total.add(*v);
        }
    }
    let mut md = format!("### {model}\n\n");
    let errors = runs.iter().filter(|r| r.error.is_some()).count();
    md.push_str(&format!(
        "{} receipts, {} could not be read at all.\n\n",
        runs.len(),
        errors
    ));
    md.push_str(
        "| Field | Accuracy (found and correct) | Unclear | Confident-wrong |\n|---|---|---|---|\n",
    );
    let mut rows: Vec<(String, FieldStats)> = vec![("Total, clear receipts".into(), clear_total)];
    for field in FIELDS {
        if let Some(s) = stats.get(field) {
            rows.push((field[..1].to_uppercase() + &field[1..], *s));
        }
    }
    for (name, s) in &rows {
        md.push_str(&format!(
            "| {name} | {} | {} | {} |\n",
            pct(s.correct, s.present),
            pct(s.unclear, s.scored),
            pct(s.wrong, s.scored)
        ));
    }
    md.push_str("\nAccuracy is over receipts that show the field; unclear and confident-wrong are over all labelled receipts (a value found on a receipt that doesn't show the field counts as confident-wrong).\n\n");

    let times = |pick: &dyn Fn(&Run) -> bool| {
        let mut t: Vec<u64> = runs.iter().filter(|r| pick(r)).map(|r| r.millis).collect();
        t.sort_unstable();
        t
    };
    let all = times(&|_| true);
    let text = times(&|r: &Run| r.text_pdf);
    let image = times(&|r: &Run| !r.text_pdf);
    md.push_str("| Time per receipt | Receipts | Median | 95th percentile |\n|---|---|---|---|\n");
    for (name, t) in [
        ("All", &all),
        ("PDF with text layer", &text),
        ("Image (photo, scan, or PDF without text)", &image),
    ] {
        md.push_str(&format!(
            "| {name} | {} | {} | {} |\n",
            t.len(),
            seconds(percentile(t, 0.5)),
            seconds(percentile(t, 0.95))
        ));
    }
    md.push_str(&format!(
        "\nFirst request (loads the model, not counted above): {}. Model memory reported by Ollama: {}.\n\n",
        seconds(warmup),
        memory_bytes.map_or("not measured".into(), |b| format!("{:.1} GB", b as f64 / 1e9))
    ));

    md.push_str("Targets from the brief:\n\n");
    for (field, accuracy, wrong) in TARGETS {
        let s = if field == "total" {
            clear_total
        } else {
            stats.get(field).copied().unwrap_or_default()
        };
        let acc_ok = s.present > 0 && s.correct as f64 >= accuracy * s.present as f64;
        let wrong_ok = s.scored > 0 && s.wrong as f64 <= wrong * s.scored as f64;
        md.push_str(&format!(
            "- {field}: accuracy ≥ {:.0}% {}, confident-wrong ≤ {:.0}% {}\n",
            accuracy * 100.0,
            if acc_ok { "met" } else { "**not met**" },
            wrong * 100.0,
            if wrong_ok { "met" } else { "**not met**" },
        ));
    }
    let median_ok = percentile(&all, 0.5).is_some_and(|m| m < MEDIAN_TARGET_MS);
    md.push_str(&format!(
        "- median time < 20 s: {}\n\n",
        if median_ok { "met" } else { "**not met**" }
    ));

    let kinds: BTreeMap<String, usize> = receipts.iter().fold(BTreeMap::new(), |mut m, r| {
        *m.entry(r.expected.kind.clone().unwrap_or_else(|| "untagged".into()))
            .or_default() += 1;
        m
    });
    let kinds: Vec<String> = kinds.iter().map(|(k, n)| format!("{k} {n}")).collect();
    md.push_str(&format!("Corpus mix: {}.\n", kinds.join(", ")));
    md
}

/// Per-receipt verdicts for local debugging. It names files, so it stays on
/// this computer; it never contains extracted values.
pub fn details(model: &str, receipts: &[Receipt], runs: &[Run]) -> String {
    let mut out = format!("# {model}: per-receipt verdicts (local only, do not share)\n\n");
    for run in runs {
        let name = receipts
            .get(run.receipt)
            .map(|r| r.path.display().to_string())
            .unwrap_or_default();
        let verdicts: Vec<String> = run
            .verdicts
            .iter()
            .map(|(f, (v, _))| format!("{f}={v:?}"))
            .collect();
        out.push_str(&format!(
            "- {name} ({} ms){}: {}\n",
            run.millis,
            run.error
                .as_ref()
                .map(|e| format!(" error: {e}"))
                .unwrap_or_default(),
            verdicts.join(" ")
        ));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    fn expected(v: Value) -> Expected {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn labels_distinguish_missing_keys_from_null() {
        let e = expected(json!({ "total": "42.50", "tax": null }));
        assert_eq!(e.get("total"), Some(&json!("42.50")));
        assert_eq!(e.get("tax"), Some(&Value::Null));
        assert_eq!(e.get("vendor"), None);
        assert!(e.clear);
    }

    #[test]
    fn verdicts() {
        use FieldStatus::*;
        assert_eq!(
            verdict_for("total", &json!("42.50"), Found, &json!(4250)),
            Verdict::Correct
        );
        assert_eq!(
            verdict_for("total", &json!("$1,042.50"), Found, &json!(104_250)),
            Verdict::Correct
        );
        assert_eq!(
            verdict_for("total", &json!("42.50"), Found, &json!(100)),
            Verdict::ConfidentWrong
        );
        assert_eq!(
            verdict_for("tax", &Value::Null, Found, &json!(100)),
            Verdict::ConfidentWrong
        );
        assert_eq!(
            verdict_for("tax", &Value::Null, Absent, &Value::Null),
            Verdict::RightlyAbsent
        );
        assert_eq!(
            verdict_for("tax", &Value::Null, Unclear, &Value::Null),
            Verdict::Unclear
        );
        assert_eq!(
            verdict_for("total", &json!("42.50"), Absent, &Value::Null),
            Verdict::Missed
        );
        assert_eq!(
            verdict_for(
                "vendor",
                &json!(["Paper & Pine", "Paper and Pine"]),
                Found,
                &json!("PAPER AND PINE")
            ),
            Verdict::Correct
        );
        assert_eq!(
            verdict_for("vendor", &json!("Paper & Pine"), Found, &json!("Paper")),
            Verdict::ConfidentWrong
        );
        assert_eq!(
            verdict_for("reference", &json!("qb 901"), Found, &json!("QB901")),
            Verdict::Correct
        );
        assert_eq!(
            verdict_for("currency", &json!("GBP"), Found, &json!("GBP")),
            Verdict::Correct
        );
    }

    #[test]
    fn tips_and_zero_amounts_are_scored() {
        use FieldStatus::*;
        assert_eq!(
            verdict_for("tip", &json!("8.50"), Found, &json!(850)),
            Verdict::Correct
        );
        assert_eq!(
            verdict_for("tip", &Value::Null, Found, &json!(850)),
            Verdict::ConfidentWrong
        );
        assert_eq!(
            verdict_for("tip", &Value::Null, Absent, &Value::Null),
            Verdict::RightlyAbsent
        );
        assert_eq!(
            verdict_for("tax", &json!("0.00"), Found, &json!(0)),
            Verdict::Correct
        );
        let e = expected(json!({ "total": "51.00", "tip": null }));
        assert_eq!(e.get("tip"), Some(&Value::Null));
    }

    #[test]
    fn rates_and_percentiles() {
        let mut s = FieldStats::default();
        for v in [
            (Verdict::Correct, true),
            (Verdict::Correct, true),
            (Verdict::Unclear, true),
            (Verdict::ConfidentWrong, true),
            (Verdict::ConfidentWrong, false),
            (Verdict::RightlyAbsent, false),
            (Verdict::Missed, true),
        ] {
            s.add(v);
        }
        assert_eq!(
            s,
            FieldStats {
                scored: 7,
                present: 5,
                correct: 2,
                unclear: 1,
                wrong: 2
            }
        );
        assert_eq!(pct(1, 4), "25.0% (1/4)");
        assert_eq!(percentile(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.5), Some(5));
        assert_eq!(percentile(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.95), Some(10));
        assert_eq!(percentile(&[], 0.5), None);
    }
}
