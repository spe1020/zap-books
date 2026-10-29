//! `cargo run -p extractor`: the local receipt-reading service.

use std::net::{Ipv4Addr, SocketAddr};

use extractor::{config::Config, log, ollama::Ollama, pdf, server::Service};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    log::init();
    let config = match Config::from_args(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(2);
        }
    };
    let swept = pdf::sweep_stale(&std::env::temp_dir());
    if swept > 0 {
        eprintln!("Removed {swept} temporary receipt folder(s) left by a run that was stopped.");
    }
    if !pdf::poppler_available().await {
        eprintln!(
            "Poppler (pdftotext, pdftoppm) was not found, so PDF receipts can't be read.\nInstall it with: brew install poppler\nImage receipts still work."
        );
    }
    match Ollama::new(&config.ollama, config.timeout, config.max_tokens) {
        Ok(ollama) => match ollama.has_model(&config.model).await {
            Ok(true) => {}
            Ok(false) => eprintln!(
                "The model {0} isn't installed in Ollama. Run: ollama pull {0}",
                config.model
            ),
            Err(_) => eprintln!(
                "Ollama isn't answering at {}. Start it (open the Ollama app, or run: ollama serve).",
                config.ollama
            ),
        },
        Err(e) => eprintln!("{e}"),
    }
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let service = match Service::bind(&config, addr).await {
        Ok(service) => service,
        Err(message) => {
            eprintln!("{message}");
            return std::process::ExitCode::from(1);
        }
    };
    eprintln!(
        "Receipt reading is running at http://{} with model {}. Receipts are read on this computer only.",
        service.addr, config.model
    );
    eprintln!(
        "Loading the model with a fictional sample receipt. Read receipt turns on when it's ready."
    );
    match service.run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("The extractor stopped: {e}");
            std::process::ExitCode::from(1)
        }
    }
}
