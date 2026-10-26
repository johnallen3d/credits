use std::net::SocketAddr;
use std::time::Duration;

pub const TOKEN_VAR: &str = "TMDB_API_TOKEN";
pub const OMDB_KEY_VAR: &str = "OMDB_API_KEY";
pub const BIND_VAR: &str = "CREDITS_BIND";
pub const DB_VAR: &str = "CREDITS_DB";
pub const REGION_VAR: &str = "CREDITS_REGION";
pub const HA_URL_VAR: &str = "HA_URL";
pub const HA_TOKEN_VAR: &str = "HA_TOKEN";
pub const HA_PLAYERS_VAR: &str = "CREDITS_HA_PLAYERS";
const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_DB: &str = "credits.db";
const DEFAULT_REGION: &str = "US";
const DEFAULT_HA_PLAYERS: &str = "media_player.living_room_appletv";
/// How long fetched ratings count as fresh before OMDb is asked again.
const RATINGS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long TMDB title, season and person data counts as fresh.
const METADATA_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const PLACEHOLDER: &str = "REPLACE_ME";

#[derive(Clone, Debug)]
pub struct Config {
    pub tmdb_token: String,
    pub tmdb_api_base: String,
    pub tmdb_image_base: String,
    /// Ratings are optional: without a key every title reports them as unavailable.
    pub omdb_api_key: Option<String>,
    pub omdb_api_base: String,
    /// IMDb's daily ratings dataset, gzipped TSV.
    pub imdb_ratings_url: String,
    pub ratings_ttl: Duration,
    pub metadata_ttl: Duration,
    /// SQLite file for the provider cache, or `cache::IN_MEMORY`.
    pub database_path: String,
    /// ISO 3166-1 country code, uppercase, whose streaming and rental offers
    /// are shown.
    pub watch_region: String,
    pub bind: SocketAddr,
    /// Where to read what played on the Apple TVs; `None` turns
    /// auto-marking off.
    pub home_assistant: Option<HomeAssistantConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomeAssistantConfig {
    /// Base URL without a trailing slash, e.g. `http://homeassistant:8123`.
    pub url: String,
    pub token: String,
    /// `media_player` entity IDs to watch.
    pub players: Vec<String>,
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
    #[error("{REGION_VAR}={0:?} is not a two-letter country code (expected e.g. {DEFAULT_REGION})")]
    InvalidRegion(String),
    #[error("{HA_URL_VAR}={0:?} is not an http(s) URL (expected e.g. http://homeassistant:8123)")]
    InvalidHaUrl(String),
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

        let watch_region = match lookup(REGION_VAR).map(|r| r.trim().to_owned()) {
            None => DEFAULT_REGION.to_owned(),
            Some(r) if r.is_empty() => DEFAULT_REGION.to_owned(),
            Some(r) if r.len() == 2 && r.bytes().all(|b| b.is_ascii_alphabetic()) => {
                r.to_ascii_uppercase()
            }
            Some(r) => return Err(ConfigError::InvalidRegion(r)),
        };

        let home_assistant = home_assistant(&lookup)?;

        Ok(Self {
            tmdb_token: token,
            tmdb_api_base: "https://api.themoviedb.org/3".to_owned(),
            tmdb_image_base: "https://image.tmdb.org/t/p".to_owned(),
            omdb_api_key,
            omdb_api_base: "https://www.omdbapi.com".to_owned(),
            imdb_ratings_url: "https://datasets.imdbws.com/title.ratings.tsv.gz".to_owned(),
            ratings_ttl: RATINGS_TTL,
            metadata_ttl: METADATA_TTL,
            database_path,
            watch_region,
            bind,
            home_assistant,
        })
    }
}

/// Home Assistant settings, if both its URL and token are set.
fn home_assistant(
    lookup: &impl Fn(&str) -> Option<String>,
) -> Result<Option<HomeAssistantConfig>, ConfigError> {
    let value = |key| {
        lookup(key)
            .map(|v: String| v.trim().to_owned())
            .filter(|v| !v.is_empty() && v != PLACEHOLDER)
    };
    let (Some(url), Some(token)) = (value(HA_URL_VAR), value(HA_TOKEN_VAR)) else {
        return Ok(None);
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ConfigError::InvalidHaUrl(url));
    }
    let players = value(HA_PLAYERS_VAR).unwrap_or_else(|| DEFAULT_HA_PLAYERS.to_owned());
    Ok(Some(HomeAssistantConfig {
        url: url.trim_end_matches('/').to_owned(),
        token,
        players: players
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect(),
    }))
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

    #[test]
    fn home_assistant_needs_url_and_token() {
        let token = (TOKEN_VAR, "eyJ.token.sig");
        assert_eq!(config(&[token]).unwrap().home_assistant, None);
        assert_eq!(
            config(&[token, (HA_URL_VAR, "http://ha:8123")])
                .unwrap()
                .home_assistant,
            None
        );
        let ha = config(&[
            token,
            (HA_URL_VAR, "http://ha:8123/"),
            (HA_TOKEN_VAR, " t "),
        ])
        .unwrap()
        .home_assistant
        .unwrap();
        assert_eq!(ha.url, "http://ha:8123");
        assert_eq!(ha.token, "t");
        assert_eq!(ha.players, ["media_player.living_room_appletv"]);
        let ha = config(&[
            token,
            (HA_URL_VAR, "https://ha"),
            (HA_TOKEN_VAR, "t"),
            (HA_PLAYERS_VAR, "media_player.a, media_player.b,"),
        ])
        .unwrap()
        .home_assistant
        .unwrap();
        assert_eq!(ha.players, ["media_player.a", "media_player.b"]);
        assert_eq!(
            config(&[
                token,
                (HA_URL_VAR, "homeassistant:8123"),
                (HA_TOKEN_VAR, "t")
            ])
            .unwrap_err(),
            ConfigError::InvalidHaUrl("homeassistant:8123".to_owned())
        );
    }

    #[test]
    fn watch_region_defaults_to_us_and_must_be_a_country_code() {
        let token = (TOKEN_VAR, "eyJ.token.sig");
        assert_eq!(config(&[token]).unwrap().watch_region, "US");
        assert_eq!(
            config(&[token, (REGION_VAR, "")]).unwrap().watch_region,
            "US"
        );
        assert_eq!(
            config(&[token, (REGION_VAR, " gb ")]).unwrap().watch_region,
            "GB"
        );
        assert_eq!(
            config(&[token, (REGION_VAR, "USA")]).unwrap_err(),
            ConfigError::InvalidRegion("USA".to_owned())
        );
    }
}
