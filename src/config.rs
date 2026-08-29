use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};
use thiserror::Error;

const DEFAULT_LISTEN_ADDRESS: &str = "0.0.0.0:8080";
const DEFAULT_DATABASE_PATH: &str = "/data/sync.db";
const DEFAULT_RATE_LIMIT: u32 = 120;

#[derive(Debug)]
pub struct Config {
    pub listen_address: SocketAddr,
    pub database_path: PathBuf,
    pub token: String,
    pub requests_per_minute: u32,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("STATE_SYNC_LISTEN_ADDR is invalid")]
    ListenAddress,
    #[error("STATE_SYNC_DATABASE_PATH is invalid")]
    DatabasePath,
    #[error("configure exactly one of STATE_SYNC_TOKEN or STATE_SYNC_TOKEN_FILE")]
    TokenSource,
    #[error("STATE_SYNC_TOKEN_FILE could not be read")]
    TokenFile(#[source] std::io::Error),
    #[error("StateSync token must contain 32 to 4096 protocol-safe bytes")]
    Token,
    #[error("STATE_SYNC_RATE_LIMIT_PER_MINUTE must be a positive integer")]
    RateLimit,
}

impl Config {
    pub fn from_environment() -> Result<Self, ConfigError> {
        Self::load(|name| std::env::var(name).ok())
    }

    fn load(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let listen_address = get("STATE_SYNC_LISTEN_ADDR")
            .unwrap_or_else(|| DEFAULT_LISTEN_ADDRESS.to_owned())
            .parse()
            .map_err(|_| ConfigError::ListenAddress)?;
        let database_path = PathBuf::from(
            get("STATE_SYNC_DATABASE_PATH").unwrap_or_else(|| DEFAULT_DATABASE_PATH.to_owned()),
        );
        if database_path.as_os_str().is_empty() {
            return Err(ConfigError::DatabasePath);
        }
        let direct_token = get("STATE_SYNC_TOKEN");
        let token_file = get("STATE_SYNC_TOKEN_FILE");
        let token = match (direct_token, token_file) {
            (Some(token), None) => token,
            (None, Some(path)) if !path.is_empty() => read_token(Path::new(&path))?,
            _ => return Err(ConfigError::TokenSource),
        };
        validate_token(&token)?;
        let requests_per_minute = get("STATE_SYNC_RATE_LIMIT_PER_MINUTE")
            .map(|value| value.parse().map_err(|_| ConfigError::RateLimit))
            .transpose()?
            .unwrap_or(DEFAULT_RATE_LIMIT);
        if requests_per_minute == 0 {
            return Err(ConfigError::RateLimit);
        }
        Ok(Self {
            listen_address,
            database_path,
            token,
            requests_per_minute,
        })
    }
}

fn read_token(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path)
        .map(|value| value.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(ConfigError::TokenFile)
}

pub(crate) fn validate_token(token: &str) -> Result<(), ConfigError> {
    if token.len() < 32
        || token.len() > 4_096
        || !token.bytes().all(|value| {
            value.is_ascii_alphanumeric()
                || matches!(value, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
        })
    {
        return Err(ConfigError::Token);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn defaults_are_safe_for_the_container() {
        let values = HashMap::from([(
            "STATE_SYNC_TOKEN".to_owned(),
            "0123456789abcdef0123456789abcdef".to_owned(),
        )]);

        let config = Config::load(|name| values.get(name).cloned()).unwrap();

        assert_eq!(config.listen_address.to_string(), "0.0.0.0:8080");
        assert_eq!(config.database_path, Path::new("/data/sync.db"));
        assert_eq!(config.requests_per_minute, 120);
    }

    #[test]
    fn ambiguous_or_short_tokens_are_rejected() {
        let ambiguous = HashMap::from([
            ("STATE_SYNC_TOKEN".to_owned(), "x".repeat(32)),
            ("STATE_SYNC_TOKEN_FILE".to_owned(), "/secret".to_owned()),
        ]);
        assert!(matches!(
            Config::load(|name| ambiguous.get(name).cloned()),
            Err(ConfigError::TokenSource)
        ));
        let short = HashMap::from([("STATE_SYNC_TOKEN".to_owned(), "short".to_owned())]);
        assert!(matches!(
            Config::load(|name| short.get(name).cloned()),
            Err(ConfigError::Token)
        ));
    }
}
