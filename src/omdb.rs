//! OMDb access by IMDb identifier only (never by title), normalized into
//! `model::Rating`s. Response types stay private to this module.

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::model::{Rating, RatingSource};

#[derive(Clone)]
pub struct OmdbClient {
    http: reqwest::Client,
    api_base: String,
    key: String,
}

/// What OMDb knows about one IMDb identifier. This is the cached payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct OmdbTitle {
    /// False when OMDb has no entry for the identifier.
    pub listed: bool,
    pub ratings: Vec<Rating>,
}

#[derive(Debug, thiserror::Error)]
pub enum OmdbError {
    #[error("OMDb rejected the API key")]
    Unauthorized,
    #[error("OMDb daily request limit reached")]
    QuotaExhausted,
    #[error("{0}")]
    Unavailable(String),
}

impl OmdbClient {
    pub fn new(api_base: &str, key: &str) -> Self {
        let http = reqwest::Client::builder()
            // Ratings are optional; don't hold the page hostage to a slow provider.
            .timeout(Duration::from_secs(4))
            .user_agent(concat!("credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static settings");
        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            key: key.to_owned(),
        }
    }

    pub async fn title(&self, imdb_id: &str) -> Result<OmdbTitle, OmdbError> {
        let response = self
            .http
            .get(format!("{}/", self.api_base))
            .query(&[("i", imdb_id), ("apikey", &self.key)])
            .send()
            .await
            // reqwest includes the URL, and with it the key, in its errors.
            .map_err(|e| {
                OmdbError::Unavailable(format!("request to OMDb failed: {}", e.without_url()))
            })?;

        let status = response.status();
        let body: Response = match response.json().await {
            Ok(body) => body,
            Err(_) if !status.is_success() => {
                return Err(OmdbError::Unavailable(format!("OMDb returned {status}")));
            }
            Err(e) => {
                return Err(OmdbError::Unavailable(format!(
                    "unexpected OMDb response: {}",
                    e.without_url()
                )));
            }
        };

        if body.response != "True" {
            let error = body.error.unwrap_or_default();
            let lowered = error.to_ascii_lowercase();
            return if lowered.contains("limit") {
                Err(OmdbError::QuotaExhausted)
            } else if status == StatusCode::UNAUTHORIZED || lowered.contains("api key") {
                Err(OmdbError::Unauthorized)
            } else if lowered.contains("incorrect imdb id") || lowered.contains("not found") {
                Ok(OmdbTitle {
                    listed: false,
                    ratings: Vec::new(),
                })
            } else {
                Err(OmdbError::Unavailable(format!("OMDb error: {error}")))
            };
        }

        let mut ratings = Vec::new();
        if let Some(score) = body.imdb_rating.as_deref().and_then(parse_decimal) {
            ratings.push(Rating {
                source: RatingSource::Imdb,
                score,
                scale: 10,
                votes: body.imdb_votes.as_deref().and_then(parse_count),
            });
        }
        if let Some(score) = body
            .ratings
            .iter()
            .find(|r| r.source == "Rotten Tomatoes")
            .and_then(|r| r.value.strip_suffix('%'))
            .and_then(parse_decimal)
        {
            ratings.push(Rating {
                source: RatingSource::RottenTomatoes,
                score,
                scale: 100,
                votes: None,
            });
        }
        Ok(OmdbTitle {
            listed: true,
            ratings,
        })
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "Response")]
    response: String,
    #[serde(rename = "Error")]
    error: Option<String>,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    #[serde(rename = "imdbVotes")]
    imdb_votes: Option<String>,
    #[serde(rename = "Ratings", default)]
    ratings: Vec<SourceValue>,
}

#[derive(Deserialize)]
struct SourceValue {
    #[serde(rename = "Source")]
    source: String,
    #[serde(rename = "Value")]
    value: String,
}

/// OMDb uses "N/A" for missing values; anything unparseable is treated the same.
fn parse_decimal(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
}

fn parse_count(value: &str) -> Option<u64> {
    value.trim().replace(',', "").parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_values_are_not_zero() {
        assert_eq!(parse_decimal("N/A"), None);
        assert_eq!(parse_decimal(""), None);
        assert_eq!(parse_decimal("8.7"), Some(8.7));
        assert_eq!(parse_count("2,123,456"), Some(2_123_456));
        assert_eq!(parse_count("N/A"), None);
    }
}
