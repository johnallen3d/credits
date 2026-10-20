//! Raw TMDB v3 HTTP access. Response types mirror TMDB's JSON and stay private
//! to the catalog; everything else uses `crate::model`. Detail responses are
//! also the provider cache payload, so they serialize back to the same shape.

use std::time::Duration;

use reqwest::StatusCode;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

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
            &[("append_to_response", "external_ids,credits")],
        )
        .await
    }

    pub async fn tv(&self, id: u64) -> Result<Tv, CatalogError> {
        self.get(
            &format!("/tv/{id}"),
            &[("append_to_response", "external_ids,aggregate_credits")],
        )
        .await
    }

    pub async fn season(&self, tv_id: u64, number: u32) -> Result<Season, CatalogError> {
        self.get(&format!("/tv/{tv_id}/season/{number}"), &[]).await
    }

    pub async fn person(&self, id: u64) -> Result<Person, CatalogError> {
        self.get(
            &format!("/person/{id}"),
            &[("append_to_response", "combined_credits")],
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
    Person(PersonSummary),
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

#[derive(Deserialize)]
pub struct PersonSummary {
    pub id: u64,
    pub name: String,
    pub known_for_department: Option<String>,
    pub profile_path: Option<String>,
    /// Movies and TV shows; TMDB lists the person's most notable few.
    #[serde(default)]
    pub known_for: Vec<MultiResult>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ExternalIds {
    pub imdb_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Genre {
    pub name: String,
}

#[derive(Serialize, Deserialize)]
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
    #[serde(default)]
    pub credits: Credits,
}

#[derive(Serialize, Deserialize)]
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
    #[serde(default)]
    pub aggregate_credits: AggregateCredits,
    #[serde(default)]
    pub created_by: Vec<Creator>,
    #[serde(default)]
    pub seasons: Vec<SeasonSummary>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Credits {
    #[serde(default)]
    pub cast: Vec<CastMember>,
    #[serde(default)]
    pub crew: Vec<CrewMember>,
}

#[derive(Serialize, Deserialize)]
pub struct CastMember {
    pub id: u64,
    pub name: String,
    pub character: Option<String>,
    pub profile_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CrewMember {
    pub id: u64,
    pub name: String,
    pub department: Option<String>,
    pub job: Option<String>,
    pub profile_path: Option<String>,
}

/// TV credits summed over every episode, with per-role episode counts.
#[derive(Serialize, Deserialize, Default)]
pub struct AggregateCredits {
    #[serde(default)]
    pub cast: Vec<AggregateCast>,
    #[serde(default)]
    pub crew: Vec<AggregateCrew>,
}

#[derive(Serialize, Deserialize)]
pub struct AggregateCast {
    pub id: u64,
    pub name: String,
    pub profile_path: Option<String>,
    #[serde(default)]
    pub roles: Vec<Role>,
    pub total_episode_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct Role {
    pub character: Option<String>,
    pub episode_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct AggregateCrew {
    pub id: u64,
    pub name: String,
    pub profile_path: Option<String>,
    pub department: Option<String>,
    #[serde(default)]
    pub jobs: Vec<Job>,
    pub total_episode_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct Job {
    pub job: Option<String>,
    pub episode_count: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct Creator {
    pub id: u64,
    pub name: String,
    pub profile_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SeasonSummary {
    pub season_number: u32,
    pub name: Option<String>,
    pub air_date: Option<String>,
    pub episode_count: Option<u32>,
    pub poster_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Season {
    pub season_number: u32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub poster_path: Option<String>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Serialize, Deserialize)]
pub struct Episode {
    pub episode_number: u32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<u32>,
    pub still_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    pub known_for_department: Option<String>,
    pub profile_path: Option<String>,
    #[serde(default)]
    pub combined_credits: CombinedCredits,
}

#[derive(Serialize, Deserialize, Default)]
pub struct CombinedCredits {
    #[serde(default)]
    pub cast: Vec<PersonCredit>,
    #[serde(default)]
    pub crew: Vec<PersonCredit>,
}

/// One movie or TV credit of a person. Cast credits carry `character`, crew
/// credits `department` and `job`; `media_type` is "movie" or "tv".
#[derive(Serialize, Deserialize)]
pub struct PersonCredit {
    pub id: u64,
    pub media_type: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub release_date: Option<String>,
    pub first_air_date: Option<String>,
    pub poster_path: Option<String>,
    pub character: Option<String>,
    pub department: Option<String>,
    pub job: Option<String>,
    pub episode_count: Option<u32>,
    pub vote_count: Option<u64>,
    #[serde(default)]
    pub genre_ids: Vec<u32>,
}
