//! `cargo run -p ledger-service -- --key-file <path> --public-url <url>`
//!
//! Reads `DATABASE_URL` (or `--database-url`). Listens on 127.0.0.1:8787 by
//! default; put it behind a TLS-terminating proxy.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ledger_service::clock::{uuid, SystemClock};
use ledger_service::http::{router, AppState};
use ledger_service::ratelimit::RateLimit;
use ledger_service::{key, Ledger};
use sqlx::postgres::PgPoolOptions;

const USAGE: &str = "Usage: ledger-service --public-url <https://…> [--key-file <path>] [--database-url <url>] [--listen <addr>]
       ledger-service generate-key <path>
       ledger-service public-key <path>";

/// Writes a new service key to `path` (which must not exist) with mode
/// 0600, and prints its public key. The secret is never printed.
fn generate_key(path: &std::path::Path) -> ExitCode {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let keys = nostr::prelude::Keys::generate();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let written = options.open(path).and_then(|mut f| {
        writeln!(f, "{}", keys.secret_key().to_secret_hex())?;
        f.sync_all()
    });
    match written {
        Ok(()) => {
            println!("{}", keys.public_key().to_hex());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Can't write {}: {e}", path.display());
            ExitCode::from(1)
        }
    }
}

struct Config {
    key_file: Option<PathBuf>,
    database_url: String,
    public_url: String,
    listen: SocketAddr,
}

fn config() -> Result<Config, String> {
    let mut args = std::env::args().skip(1);
    let mut key_file = None;
    let mut database_url = std::env::var("DATABASE_URL").ok();
    let mut public_url = None;
    let mut listen: SocketAddr = ([127, 0, 0, 1], 8787).into();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} needs a value.\n{USAGE}"))
        };
        match arg.as_str() {
            "--key-file" => key_file = Some(PathBuf::from(value()?)),
            "--database-url" => database_url = Some(value()?),
            "--public-url" => public_url = Some(value()?),
            "--listen" => {
                listen = value()?.parse().map_err(|_| {
                    format!("--listen needs an address like 127.0.0.1:8787.\n{USAGE}")
                })?
            }
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("Unknown option {other}.\n{USAGE}")),
        }
    }
    Ok(Config {
        key_file,
        database_url: database_url.ok_or_else(|| format!("Set DATABASE_URL.\n{USAGE}"))?,
        public_url: public_url.ok_or_else(|| format!("--public-url is required.\n{USAGE}"))?,
        listen,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1), args.len()) {
        (Some("generate-key"), Some(path), 2) => return generate_key(std::path::Path::new(path)),
        // Checks a key file (permissions included) and prints its public
        // key, for testing a restored backup.
        (Some("public-key"), Some(path), 2) => {
            return match key::load(Some(std::path::Path::new(path))) {
                Ok(k) => {
                    println!("{}", k.public_key().to_hex());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::from(2)
                }
            };
        }
        (Some("generate-key" | "public-key"), _, _) => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
        _ => {}
    }
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let config = match config() {
        Ok(c) => c,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let keys = match key::load(config.key_file.as_deref()) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let pool = match PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Can't connect to the database: {e}");
            return ExitCode::from(1);
        }
    };
    if let Err(e) = Ledger::migrate(&pool).await {
        eprintln!("Can't migrate the database: {e}");
        return ExitCode::from(1);
    }
    let ledger = Arc::new(Ledger::new(
        pool,
        keys,
        Arc::new(SystemClock),
        Arc::new(uuid),
    ));
    tracing::info!(service = %ledger.public_key(), listen = %config.listen, "starting");
    let state = Arc::new(AppState {
        ledger,
        public_url: config.public_url,
        writes: RateLimit::new(60, 10),
        reads: RateLimit::new(120, 20),
    });
    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Can't listen on {}: {e}", config.listen);
            return ExitCode::from(1);
        }
    };
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    match axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("The server stopped: {e}");
            ExitCode::from(1)
        }
    }
}
