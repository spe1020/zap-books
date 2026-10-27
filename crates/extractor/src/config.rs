//! Command-line flags. Every flag has a default, so the service runs with no
//! setup beyond installing Ollama, a model, and Poppler.

use std::time::Duration;

pub use crate::ollama::DEFAULT_MAX_TOKENS;

pub const DEFAULT_PORT: u16 = 8787;
pub const DEFAULT_OLLAMA: &str = "http://127.0.0.1:11434";
pub const DEFAULT_MODEL: &str = "qwen3-vl:30b-a3b-instruct";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// The app's local origins: Vite dev (5173) and preview (4173).
pub const DEFAULT_ORIGINS: [&str; 4] = [
    "http://127.0.0.1:5173",
    "http://localhost:5173",
    "http://127.0.0.1:4173",
    "http://localhost:4173",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub ollama: String,
    pub model: String,
    pub allowed_origins: Vec<String>,
    pub timeout: Duration,
    /// Cap on generated tokens per receipt (`num_predict`).
    pub max_tokens: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            port: DEFAULT_PORT,
            ollama: DEFAULT_OLLAMA.to_string(),
            model: DEFAULT_MODEL.to_string(),
            allowed_origins: DEFAULT_ORIGINS.iter().map(|o| (*o).to_string()).collect(),
            timeout: DEFAULT_TIMEOUT,
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

pub const USAGE: &str = "Usage: extractor [--port 8787] [--ollama http://127.0.0.1:11434] [--model qwen3-vl:30b-a3b-instruct] [--allowed-origin <origin>]... [--timeout-seconds 90] [--max-tokens 2048]";

/// Ollama must be on this computer: documents never leave the machine.
pub fn is_local_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or_default();
    ["127.0.0.1", "localhost", "[::1]"].iter().any(|host| {
        host_port == *host
            || host_port
                .strip_prefix(host)
                .and_then(|p| p.strip_prefix(':'))
                .is_some_and(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
    })
}

impl Config {
    /// Parses flags. `--allowed-origin` replaces the defaults the first time
    /// it is given and adds to the list after that.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Config, String> {
        let mut config = Config::default();
        let mut custom_origins = false;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let mut value = || args.next().ok_or(format!("{flag} needs a value.\n{USAGE}"));
            match flag.as_str() {
                "--port" => {
                    config.port = value()?
                        .parse()
                        .map_err(|_| format!("--port must be a port number.\n{USAGE}"))?;
                }
                "--ollama" => config.ollama = value()?,
                "--model" => config.model = value()?,
                "--allowed-origin" => {
                    let origin = value()?;
                    if !custom_origins {
                        config.allowed_origins.clear();
                        custom_origins = true;
                    }
                    config.allowed_origins.push(origin);
                }
                "--timeout-seconds" => {
                    let secs: u64 = value()?
                        .parse()
                        .map_err(|_| format!("--timeout-seconds must be a number.\n{USAGE}"))?;
                    config.timeout = Duration::from_secs(secs.max(1));
                }
                "--max-tokens" => {
                    config.max_tokens = value()?
                        .parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or(format!("--max-tokens must be a positive number.\n{USAGE}"))?;
                }
                "-h" | "--help" => return Err(USAGE.to_string()),
                other => return Err(format!("Unknown flag {other}.\n{USAGE}")),
            }
        }
        if !is_local_url(&config.ollama) {
            return Err(format!(
                "--ollama must be an http:// address on this computer (127.0.0.1, localhost or [::1]); got {}. Receipts never leave this computer.",
                config.ollama
            ));
        }
        Ok(config)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, String> {
        Config::from_args(args.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn defaults_need_no_flags() {
        assert_eq!(parse(&[]).unwrap(), Config::default());
    }

    #[test]
    fn flags_override_defaults() {
        let c = parse(&[
            "--port",
            "9000",
            "--model",
            "m",
            "--allowed-origin",
            "http://127.0.0.1:3000",
            "--allowed-origin",
            "http://localhost:3000",
            "--timeout-seconds",
            "5",
            "--max-tokens",
            "512",
        ])
        .unwrap();
        assert_eq!(c.port, 9000);
        assert_eq!(c.model, "m");
        assert_eq!(
            c.allowed_origins,
            vec!["http://127.0.0.1:3000", "http://localhost:3000"]
        );
        assert_eq!(c.timeout, Duration::from_secs(5));
        assert_eq!(c.max_tokens, 512);
        assert_eq!(Config::default().max_tokens, 2048);
    }

    #[test]
    fn ollama_must_be_local() {
        assert!(parse(&["--ollama", "http://localhost:11434"]).is_ok());
        assert!(parse(&["--ollama", "http://[::1]:11434/"]).is_ok());
        for remote in [
            "http://192.168.1.5:11434",
            "https://api.example.com",
            "http://127.0.0.1.evil.com:11434",
            "http://localhost.evil.com",
            "http://user@127.0.0.1:11434",
        ] {
            assert!(parse(&["--ollama", remote]).is_err(), "{remote}");
        }
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert!(parse(&["--host", "0.0.0.0"]).is_err());
        assert!(parse(&["--port"]).is_err());
        assert!(parse(&["--max-tokens", "0"]).is_err());
        assert!(parse(&["--max-tokens", "lots"]).is_err());
    }
}
