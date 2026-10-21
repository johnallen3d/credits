//! The terminal client's settings: a TOML file naming the credits server
//! and, optionally, a color theme.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::theme::Theme;

/// Overrides where the config file is read from.
pub const CONFIG_VAR: &str = "CREDITS_CONFIG";
const EXAMPLE: &str = "url = \"https://credits.jallen7usa.com\"";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiConfig {
    /// Base URL of a running credits server, e.g. `https://credits.jallen7usa.com`.
    pub url: String,
    /// `rose-pine` (default), `tokyo-night`, `catppuccin` or `classic`.
    #[serde(default)]
    pub theme: Theme,
}

#[derive(Debug, thiserror::Error)]
pub enum TuiConfigError {
    #[error("no config file at {0}. Create it with the server's address, e.g.\n\n    {EXAMPLE}")]
    Missing(PathBuf),
    #[error("cannot read {0}: {1}")]
    Unreadable(PathBuf, std::io::Error),
    #[error("{0} is not valid: {1}")]
    Invalid(PathBuf, toml::de::Error),
    #[error("url = {0:?} in {1} is not an http(s) URL (expected e.g. {EXAMPLE})")]
    InvalidUrl(String, PathBuf),
    #[error("cannot find the config directory: set {CONFIG_VAR}, XDG_CONFIG_HOME or HOME")]
    NoConfigDir,
}

impl TuiConfig {
    /// Reads `$CREDITS_CONFIG`, else `$XDG_CONFIG_HOME/credits/config.toml`,
    /// else `~/.config/credits/config.toml`.
    pub fn load() -> Result<Self, TuiConfigError> {
        let path = config_path(|key| std::env::var_os(key).filter(|v| !v.is_empty()))?;
        Self::from_file(&path)
    }

    pub fn from_file(path: &Path) -> Result<Self, TuiConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(TuiConfigError::Missing(path.to_owned()));
            }
            Err(e) => return Err(TuiConfigError::Unreadable(path.to_owned(), e)),
        };
        Self::parse(&text, path)
    }

    fn parse(text: &str, path: &Path) -> Result<Self, TuiConfigError> {
        let mut config: TuiConfig =
            toml::from_str(text).map_err(|e| TuiConfigError::Invalid(path.to_owned(), e))?;
        let url = config.url.trim().trim_end_matches('/');
        if !(url.starts_with("http://") || url.starts_with("https://"))
            || url.contains(char::is_whitespace)
        {
            return Err(TuiConfigError::InvalidUrl(config.url, path.to_owned()));
        }
        config.url = url.to_owned();
        Ok(config)
    }
}

fn config_path(
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, TuiConfigError> {
    if let Some(path) = lookup(CONFIG_VAR) {
        return Ok(PathBuf::from(path));
    }
    let dir = lookup("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| lookup("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or(TuiConfigError::NoConfigDir)?;
    Ok(dir.join("credits").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn parse(text: &str) -> Result<TuiConfig, TuiConfigError> {
        TuiConfig::parse(text, Path::new("config.toml"))
    }

    #[test]
    fn reads_the_server_url_without_a_trailing_slash() {
        let config = parse("url = \"https://credits.jallen7usa.com/\"\n").unwrap();
        assert_eq!(config.url, "https://credits.jallen7usa.com");
    }

    #[test]
    fn theme_defaults_to_rose_pine_and_rejects_unknown_names() {
        assert_eq!(parse("url = \"http://a\"").unwrap().theme, Theme::RosePine);
        let config = parse("url = \"http://a\"\ntheme = \"tokyo-night\"").unwrap();
        assert_eq!(config.theme, Theme::TokyoNight);
        let err = parse("url = \"http://a\"\ntheme = \"solarized\"").unwrap_err();
        assert!(err.to_string().contains("catppuccin"), "{err}");
    }

    #[test]
    fn rejects_missing_unknown_and_non_http_settings() {
        assert!(matches!(parse(""), Err(TuiConfigError::Invalid(..))));
        assert!(matches!(
            parse("url = \"http://a\"\nport = 3\n"),
            Err(TuiConfigError::Invalid(..))
        ));
        assert!(matches!(
            parse("url = \"credits.jallen7usa.com\""),
            Err(TuiConfigError::InvalidUrl(..))
        ));
    }

    #[test]
    fn missing_file_explains_what_to_write() {
        let err = TuiConfig::from_file(Path::new("/nonexistent/credits/config.toml")).unwrap_err();
        assert!(matches!(err, TuiConfigError::Missing(_)));
        assert!(
            err.to_string()
                .contains("url = \"https://credits.jallen7usa.com\"")
        );
    }

    #[test]
    fn path_prefers_override_then_xdg_then_home() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        assert_eq!(
            config_path(vars(&[(CONFIG_VAR, "/x.toml"), ("HOME", "/h")])).unwrap(),
            PathBuf::from("/x.toml")
        );
        assert_eq!(
            config_path(vars(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")])).unwrap(),
            PathBuf::from("/xdg/credits/config.toml")
        );
        assert_eq!(
            config_path(vars(&[("HOME", "/h")])).unwrap(),
            PathBuf::from("/h/.config/credits/config.toml")
        );
        assert!(config_path(vars(&[])).is_err());
    }
}
