//! Raw TMDB v3 HTTP access. Response types mirror TMDB's JSON and stay private
//! to the catalog; everything else uses `crate::model`.

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, de::DeserializeOwned};

use crate::catalog::CatalogError;

#[derive(Clone)]
pub struct TmdbClient {
    http: reqwest::Client,
    api_base: String,
    token: String,
}

impl TmdbClient {
    pub fn new(api_base: &str, token: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static settings");
        Self {
            http,
            api_base: api_base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        }
    }

    pub async fn search_multi(&self, query: &str) -> Result<Paged<MultiResult>, CatalogError> {
        self.get(
            "/search/multi",
            &[("query", query), ("include_adult", "false"), ("page", "1")],
        )
        .await
    }

    pub async fn movie(&self, id: u64) -> Result<Movie, CatalogError> {
        self.get(
            &format!("/movie/{id}"),
            &[("append_to_response", "external_ids")],
        )
        .await
    }

    pub async fn tv(&self, id: u64) -> Result<Tv, CatalogError> {
        self.get(
            &format!("/tv/{id}"),
            &[("append_to_response", "external_ids")],
        )
        .await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, CatalogError> {
        let response = self
            .http
            .get(format!("{}{path}", self.api_base))
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(|e| CatalogError::Provider(format!("request to TMDB {path} failed: {e}")))?;

        match response.status() {
            StatusCode::NOT_FOUND => Err(CatalogError::NotFound),
            StatusCode::UNAUTHORIZED => Err(CatalogError::Unauthorized),
            status if !status.is_success() => Err(CatalogError::Provider(format!(
                "TMDB {path} returned {status}"
            ))),
            _ => response.json().await.map_err(|e| {
                CatalogError::Provider(format!("unexpected TMDB {path} response: {e}"))
            }),
        }
    }
}

#[derive(Deserialize)]
pub struct Paged<T> {
    pub results: Vec<T>,
}

#[derive(Deserialize)]
#[serde(tag = "media_type", rename_all = "lowercase")]
pub enum MultiResult {
    Movie(MovieSummary),
    Tv(TvSummary),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
pub struct MovieSummary {
    pub id: u64,
    pub title: String,
    pub release_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Deserialize)]
pub struct TvSummary {
    pub id: u64,
    pub name: String,
    pub first_air_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ExternalIds {
    pub imdb_id: Option<String>,
}

#[derive(Deserialize)]
pub struct Genre {
    pub name: String,
}

#[derive(Deserialize)]
pub struct Movie {
    pub id: u64,
    pub title: String,
    pub original_title: Option<String>,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub release_date: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    pub runtime: Option<u32>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub external_ids: ExternalIds,
}

#[derive(Deserialize)]
pub struct Tv {
    pub id: u64,
    pub name: String,
    pub original_name: Option<String>,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub first_air_date: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub genres: Vec<Genre>,
    pub number_of_seasons: Option<u32>,
    pub number_of_episodes: Option<u32>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    #[serde(default)]
    pub external_ids: ExternalIds,
}
