use std::net::SocketAddr;
use std::time::Duration;

pub const TOKEN_VAR: &str = "TMDB_API_TOKEN";
pub const OMDB_KEY_VAR: &str = "OMDB_API_KEY";
pub const BIND_VAR: &str = "CREDITS_BIND";
pub const DB_VAR: &str = "CREDITS_DB";
const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_DB: &str = "credits.db";
/// How long fetched ratings count as fresh before OMDb is asked again.
const RATINGS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const PLACEHOLDER: &str = "REPLACE_ME";

#[derive(Clone, Debug)]
pub struct Config {
    pub tmdb_token: String,
    pub tmdb_api_base: String,
    pub tmdb_image_base: String,
    /// Ratings are optional: without a key every title reports them as unavailable.
    pub omdb_api_key: Option<String>,
    pub omdb_api_base: String,
    pub ratings_ttl: Duration,
    /// SQLite file for the provider cache, or `cache::IN_MEMORY`.
    pub database_path: String,
    pub bind: SocketAddr,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    #[error(
        "{TOKEN_VAR} is not set. Run the server with `mise run dev` \
         (Doppler project `credits`) or export it. Use the TMDB \"API Read Access Token\" \
         from https://www.themoviedb.org/settings/api"
    )]
    MissingToken,
    #[error(
        "{TOKEN_VAR} is still the placeholder value. Set the real token with \
         `doppler secrets set {TOKEN_VAR} --project credits --config dev`"
    )]
    PlaceholderToken,
    #[error(
        "{TOKEN_VAR} looks like a v3 API key. Use the longer \"API Read Access Token\" \
         from https://www.themoviedb.org/settings/api instead"
    )]
    V3ApiKey,
    #[error("{BIND_VAR}={0:?} is not a valid socket address (expected e.g. {DEFAULT_BIND})")]
    InvalidBind(String),
    #[error(
        "{BIND_VAR}={0} is not a loopback address. The server has no authentication, \
         so it only listens on localhost"
    )]
    NonLoopbackBind(SocketAddr),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let token = lookup(TOKEN_VAR)
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .ok_or(ConfigError::MissingToken)?;
        if token == PLACEHOLDER {
            return Err(ConfigError::PlaceholderToken);
        }
        if token.len() == 32 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ConfigError::V3ApiKey);
        }

        let bind_raw = lookup(BIND_VAR).unwrap_or_else(|| DEFAULT_BIND.to_owned());
        let bind: SocketAddr = bind_raw
            .parse()
            .map_err(|_| ConfigError::InvalidBind(bind_raw.clone()))?;
        if !bind.ip().is_loopback() {
            return Err(ConfigError::NonLoopbackBind(bind));
        }

        let omdb_api_key = lookup(OMDB_KEY_VAR)
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty() && k != PLACEHOLDER);

        let database_path = lookup(DB_VAR)
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| DEFAULT_DB.to_owned());

        Ok(Self {
            tmdb_token: token,
            tmdb_api_base: "https://api.themoviedb.org/3".to_owned(),
            tmdb_image_base: "https://image.tmdb.org/t/p".to_owned(),
            omdb_api_key,
            omdb_api_base: "https://www.omdbapi.com".to_owned(),
            ratings_ttl: RATINGS_TTL,
            database_path,
            bind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_lookup(|key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn missing_or_blank_token_is_rejected() {
        assert_eq!(config(&[]).unwrap_err(), ConfigError::MissingToken);
        assert_eq!(
            config(&[(TOKEN_VAR, "  ")]).unwrap_err(),
            ConfigError::MissingToken
        );
    }

    #[test]
    fn placeholder_and_v3_key_are_rejected() {
        assert_eq!(
            config(&[(TOKEN_VAR, "REPLACE_ME")]).unwrap_err(),
            ConfigError::PlaceholderToken
        );
        assert_eq!(
            config(&[(TOKEN_VAR, "0123456789abcdef0123456789abcdef")]).unwrap_err(),
            ConfigError::V3ApiKey
        );
    }

    #[test]
    fn bind_defaults_to_localhost_and_rejects_public_addresses() {
        let ok = config(&[(TOKEN_VAR, "eyJ.token.sig")]).unwrap();
        assert_eq!(ok.bind.to_string(), DEFAULT_BIND);
        assert!(matches!(
            config(&[(TOKEN_VAR, "eyJ.token.sig"), (BIND_VAR, "0.0.0.0:3000")]),
            Err(ConfigError::NonLoopbackBind(_))
        ));
        assert!(matches!(
            config(&[(TOKEN_VAR, "eyJ.token.sig"), (BIND_VAR, "nope")]),
            Err(ConfigError::InvalidBind(_))
        ));
    }

    #[test]
    fn omdb_key_is_optional_and_placeholder_means_unset() {
        let token = (TOKEN_VAR, "eyJ.token.sig");
        assert_eq!(config(&[token]).unwrap().omdb_api_key, None);
        assert_eq!(
            config(&[token, (OMDB_KEY_VAR, "REPLACE_ME")])
                .unwrap()
                .omdb_api_key,
            None
        );
        assert_eq!(
            config(&[token, (OMDB_KEY_VAR, " abc123 ")])
                .unwrap()
                .omdb_api_key,
            Some("abc123".to_owned())
        );
    }

    #[test]
    fn database_defaults_to_a_local_file() {
        let token = (TOKEN_VAR, "eyJ.token.sig");
        assert_eq!(config(&[token]).unwrap().database_path, DEFAULT_DB);
        assert_eq!(
            config(&[token, (DB_VAR, "/tmp/x.db")])
                .unwrap()
                .database_path,
            "/tmp/x.db"
        );
    }
}
