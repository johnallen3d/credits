//! OMDb access by IMDb identifier only (never by title), normalized into
//! `model::Rating`s. Response types stay private to this module.

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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

/// IMDb ratings of one season's episodes. This is the cached payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct OmdbSeason {
    /// False when OMDb has no entry for the show or season.
    pub listed: bool,
    pub episodes: Vec<OmdbEpisode>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OmdbEpisode {
    pub number: u32,
    /// None when OMDb has no score, e.g. for an unaired episode, or one it
    /// hasn't caught up with.
    pub rating: Option<Rating>,
    /// The episode's own IMDb identifier; missing from payloads cached
    /// before it was kept.
    #[serde(default)]
    pub imdb_id: Option<String>,
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
        let Some(body) = self.get::<Response>(&[("i", imdb_id)]).await? else {
            return Ok(OmdbTitle {
                listed: false,
                ratings: Vec::new(),
            });
        };

        let mut ratings = Vec::new();
        if let Some(rating) = imdb_rating(body.imdb_rating.as_deref(), body.imdb_votes.as_deref()) {
            ratings.push(rating);
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

    /// IMDb ratings of one season's episodes, looked up by the show's IMDb identifier.
    pub async fn season(&self, show_imdb_id: &str, number: u32) -> Result<OmdbSeason, OmdbError> {
        let number = number.to_string();
        let Some(body) = self
            .get::<SeasonResponse>(&[("i", show_imdb_id), ("Season", &number)])
            .await?
        else {
            return Ok(OmdbSeason {
                listed: false,
                episodes: Vec::new(),
            });
        };
        Ok(OmdbSeason {
            listed: true,
            episodes: body
                .episodes
                .into_iter()
                .filter_map(|e| {
                    Some(OmdbEpisode {
                        number: e.episode.trim().parse().ok()?,
                        rating: imdb_rating(e.imdb_rating.as_deref(), None),
                        imdb_id: e.imdb_id.filter(|id| id.starts_with("tt")),
                    })
                })
                .collect(),
        })
    }

    /// One OMDb request; `None` when OMDb has no entry for it.
    async fn get<R: DeserializeOwned + Envelope>(
        &self,
        params: &[(&str, &str)],
    ) -> Result<Option<R>, OmdbError> {
        let response = self
            .http
            .get(format!("{}/", self.api_base))
            .query(params)
            .query(&[("apikey", &self.key)])
            .send()
            .await
            // reqwest includes the URL, and with it the key, in its errors.
            .map_err(|e| {
                OmdbError::Unavailable(format!("request to OMDb failed: {}", e.without_url()))
            })?;

        let status = response.status();
        let body: R = match response.json().await {
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

        let (response, error) = body.status();
        if response == "True" {
            return Ok(Some(body));
        }
        let error = error.unwrap_or_default();
        let lowered = error.to_ascii_lowercase();
        if lowered.contains("limit") {
            Err(OmdbError::QuotaExhausted)
        } else if status == StatusCode::UNAUTHORIZED || lowered.contains("api key") {
            Err(OmdbError::Unauthorized)
        } else if lowered.contains("incorrect imdb id")
            || lowered.contains("not found")
            // OMDb's answer for identifiers it has no record of, e.g. recent titles.
            || lowered.contains("error getting data")
        {
            Ok(None)
        } else {
            Err(OmdbError::Unavailable(format!("OMDb error: {error}")))
        }
    }
}

fn imdb_rating(score: Option<&str>, votes: Option<&str>) -> Option<Rating> {
    Some(Rating {
        source: RatingSource::Imdb,
        score: parse_decimal(score?)?,
        scale: 10,
        votes: votes.and_then(parse_count),
    })
}

/// The status fields every OMDb response carries.
trait Envelope {
    fn status(&self) -> (&str, Option<String>);
}

impl Envelope for Response {
    fn status(&self) -> (&str, Option<String>) {
        (&self.response, self.error.clone())
    }
}

impl Envelope for SeasonResponse {
    fn status(&self) -> (&str, Option<String>) {
        (&self.response, self.error.clone())
    }
}

#[derive(Deserialize)]
struct SeasonResponse {
    #[serde(rename = "Response")]
    response: String,
    #[serde(rename = "Error")]
    error: Option<String>,
    #[serde(rename = "Episodes", default)]
    episodes: Vec<SeasonEpisode>,
}

#[derive(Deserialize)]
struct SeasonEpisode {
    #[serde(rename = "Episode")]
    episode: String,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    #[serde(rename = "imdbID")]
    imdb_id: Option<String>,
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
