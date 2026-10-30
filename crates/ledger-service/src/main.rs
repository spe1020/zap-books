//! `cargo run -p ledger-service -- --key-file <path> --public-url <url>`
//!
//! Reads `DATABASE_URL` (or `--database-url`). Listens on 127.0.0.1:8787 by
//! default; put it behind a TLS-terminating proxy.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use ledger_service::clock::{uuid, Clock, SystemClock};
use ledger_service::http::{cors, router, AppState};
use ledger_service::{key, scheduler, Ledger};
use sqlx::postgres::PgPoolOptions;

const USAGE: &str = "Usage: ledger-service --public-url <https://…> [--key-file <path>]... [--database-url <url>] [--listen <addr>]
                      [--allowed-origin <https://app.example>]... [--trusted-proxy <ip>]...
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
    /// The first is the primary key; more are for businesses moving to a
    /// new service key.
    key_files: Vec<PathBuf>,
    database_url: String,
    public_url: String,
    listen: SocketAddr,
    allowed_origins: Vec<String>,
    trusted_proxies: Vec<IpAddr>,
}

fn config() -> Result<Config, String> {
    let mut args = std::env::args().skip(1);
    let mut key_files = Vec::new();
    let mut database_url = std::env::var("DATABASE_URL").ok();
    let mut public_url = None;
    let mut listen: SocketAddr = ([127, 0, 0, 1], 8787).into();
    let mut allowed_origins = Vec::new();
    let mut trusted_proxies = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{arg} needs a value.\n{USAGE}"))
        };
        match arg.as_str() {
            "--key-file" => key_files.push(PathBuf::from(value()?)),
            "--database-url" => database_url = Some(value()?),
            "--public-url" => public_url = Some(value()?),
            "--listen" => {
                listen = value()?.parse().map_err(|_| {
                    format!("--listen needs an address like 127.0.0.1:8787.\n{USAGE}")
                })?
            }
            "--allowed-origin" => allowed_origins.push(value()?),
            "--trusted-proxy" => trusted_proxies.push(value()?.parse().map_err(|_| {
                format!("--trusted-proxy needs an IP address like 127.0.0.1.\n{USAGE}")
            })?),
            "--help" | "-h" => return Err(USAGE.to_string()),
            other => return Err(format!("Unknown option {other}.\n{USAGE}")),
        }
    }
    // Refused here, at start, rather than when the first browser calls.
    let _checked = cors(&allowed_origins).map_err(|e| format!("{e}\n{USAGE}"))?;
    Ok(Config {
        allowed_origins,
        trusted_proxies,
        key_files,
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
    let mut keys = match key::load_all(&config.key_files) {
        Ok(k) => k.into_iter(),
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let Some(primary) = keys.next() else {
        eprintln!("{}", key::KeyError::Missing);
        return ExitCode::from(2);
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
    let ledger = keys.fold(
        Ledger::new(pool, primary, clock(), Arc::new(uuid)),
        Ledger::with_key,
    );
    let ledger = Arc::new(ledger);
    // A business whose key isn't loaded could do nothing: refuse to start.
    match ledger.missing_service_keys().await {
        Ok(missing) if missing.is_empty() => {}
        Ok(missing) => {
            for (business, key) in missing {
                eprintln!("Business {business} is signed by service key {key}, which is not loaded. Add its --key-file.");
            }
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("Can't check the service keys: {e}");
            return ExitCode::from(1);
        }
    }
    // One a pending replacement names can be loaded before it falls due.
    match ledger.missing_pending_keys().await {
        Ok(missing) => {
            for (business, key) in missing {
                tracing::error!(business = %business, key = %key, "a pending service key replacement names a key that is not loaded; load it before the change falls due, or it can't activate");
            }
        }
        Err(e) => {
            eprintln!("Can't check the service keys: {e}");
            return ExitCode::from(1);
        }
    }
    let services: Vec<String> = ledger.public_keys().iter().map(|k| k.to_hex()).collect();
    tracing::info!(services = ?services, listen = %config.listen, "starting");
    let scheduler = scheduler::spawn(ledger.clone(), scheduler::INTERVAL);
    // The scheduler isolates each run, so its task should never end; if it
    // does, say so.
    tokio::spawn(async move {
        match scheduler.await {
            Err(e) if e.is_panic() => tracing::error!("the scheduler stopped: it panicked"),
            _ => tracing::error!("the scheduler stopped"),
        }
    });
    let mut state = AppState::new(ledger, config.public_url);
    state.allowed_origins = config.allowed_origins;
    state.trusted_proxies = config.trusted_proxies;
    let app = match router(Arc::new(state)) {
        Ok(app) => app,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Can't listen on {}: {e}", config.listen);
            return ExitCode::from(1);
        }
    };
    match axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("The server stopped: {e}");
            ExitCode::from(1)
        }
    }
}

/// Resolves on SIGINT (Ctrl-C) or SIGTERM (what systemd and Docker send to
/// stop a service), so either drains the listener.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(not(feature = "e2e-clock"))]
fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock)
}

/// The remote e2e tests' clock: moved by the offset in the file
/// `ZAPBOOKS_E2E_CLOCK_FILE` names.
#[cfg(feature = "e2e-clock")]
fn clock() -> Arc<dyn Clock> {
    match std::env::var_os("ZAPBOOKS_E2E_CLOCK_FILE") {
        Some(path) => {
            tracing::warn!("e2e-clock build: the clock follows a test file");
            Arc::new(ledger_service::clock::OffsetClock(path.into()))
        }
        None => Arc::new(SystemClock),
    }
}
