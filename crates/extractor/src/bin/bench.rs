//! `cargo run -p extractor --bin bench -- --corpus <dir> --model <name>`
//!
//! Runs every labelled receipt in the corpus through each model and writes a
//! Markdown report (aggregate numbers only) plus a local per-receipt file.

use std::path::PathBuf;
use std::time::Duration;

use extractor::bench::{details, load_categories, load_corpus, report, run_model, Settings};
use extractor::config::{is_local_url, DEFAULT_MAX_TOKENS, DEFAULT_MODEL, DEFAULT_OLLAMA};
use extractor::ollama::Ollama;
use extractor::{log, Extractor};

const USAGE: &str = "Usage: bench --corpus <dir> [--model <name>]... [--ollama http://127.0.0.1:11434] [--out bench-results.md] [--timeout-seconds 90] [--max-tokens 2048]";

struct Args {
    corpus: PathBuf,
    models: Vec<String>,
    ollama: String,
    out: PathBuf,
    timeout: Duration,
    max_tokens: u32,
}

fn parse() -> Result<Args, String> {
    let mut corpus = None;
    let mut models = Vec::new();
    let mut ollama = DEFAULT_OLLAMA.to_string();
    let mut out = PathBuf::from("bench-results.md");
    let mut timeout = Duration::from_secs(90);
    let mut max_tokens = DEFAULT_MAX_TOKENS;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value.\n{USAGE}"));
        match flag.as_str() {
            "--corpus" => corpus = Some(PathBuf::from(value()?)),
            "--model" => models.push(value()?),
            "--ollama" => ollama = value()?,
            "--out" => out = PathBuf::from(value()?),
            "--timeout-seconds" => {
                let secs: u64 = value()?
                    .parse()
                    .map_err(|_| format!("--timeout-seconds must be a number.\n{USAGE}"))?;
                timeout = Duration::from_secs(secs.max(1));
            }
            "--max-tokens" => {
                max_tokens = value()?
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or(format!("--max-tokens must be a positive number.\n{USAGE}"))?;
            }
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("Unknown flag {other}.\n{USAGE}")),
        }
    }
    if !is_local_url(&ollama) {
        return Err("--ollama must be an http:// address on this computer.".into());
    }
    if models.is_empty() {
        models.push(DEFAULT_MODEL.to_string());
    }
    Ok(Args {
        corpus: corpus.ok_or(format!("--corpus is required.\n{USAGE}"))?,
        models,
        ollama,
        out,
        timeout,
        max_tokens,
    })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    log::init();
    let args = match parse() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    let (receipts, unlabelled) = match load_corpus(&args.corpus) {
        Ok(found) => found,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(1);
        }
    };
    for path in &unlabelled {
        eprintln!("Skipping {} (no expected.json)", path.display());
    }
    if receipts.is_empty() {
        eprintln!("No labelled receipts in {}.", args.corpus.display());
        return std::process::ExitCode::from(1);
    }
    let categories = match load_categories(&args.corpus) {
        Ok(c) => c,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(1);
        }
    };
    let mut markdown = String::from("## Receipt extraction benchmark\n\n");
    let mut local = String::new();
    for model in &args.models {
        let ollama = match Ollama::new(&args.ollama, args.timeout, args.max_tokens) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::from(1);
            }
        };
        match ollama.has_model(model).await {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Skipping {model}: not installed. Run: ollama pull {model}");
                continue;
            }
            Err(e) => {
                eprintln!("{e}");
                return std::process::ExitCode::from(1);
            }
        }
        let extractor = Extractor {
            ollama: ollama.clone(),
            model: model.clone(),
        };
        eprintln!("{model}: {} receipts", receipts.len());
        let (runs, warmup) = run_model(&extractor, &receipts, &categories, |i, n| {
            eprint!("\r{model}: receipt {i}/{n}");
        })
        .await;
        eprintln!();
        let memory = ollama.loaded_size(model).await;
        // Free the memory before the next model so runs don't overlap.
        if let Err(e) = ollama.unload(model).await {
            eprintln!("Could not unload {model}: {e}");
        }
        let settings = Settings {
            thinking_supported: ollama.supports_thinking(model).await,
            max_tokens: ollama.max_tokens(),
            timeout: args.timeout,
            prompt_version: extractor::prompt::PROMPT_VERSION,
        };
        markdown.push_str(&report(model, &receipts, &runs, warmup, memory, &settings));
        markdown.push('\n');
        local.push_str(&details(model, &receipts, &runs));
        local.push('\n');
    }
    let details_path = args.out.with_extension("details.local.md");
    if let Err(e) =
        std::fs::write(&args.out, &markdown).and_then(|()| std::fs::write(&details_path, &local))
    {
        eprintln!("Could not write the report: {e}");
        return std::process::ExitCode::from(1);
    }
    eprintln!(
        "Wrote {} (aggregate numbers, safe to paste) and {} (per-receipt, keep local).",
        args.out.display(),
        details_path.display()
    );
    std::process::ExitCode::SUCCESS
}
