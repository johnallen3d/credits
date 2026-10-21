//! HTTP access to a running credits server's JSON API.

use std::time::Duration;

use serde::de::DeserializeOwned;

use super::api::{ApiError, Kind, Person, SearchHit, SearchResults, Title};

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("can't reach the credits server at {0}")]
    Unreachable(String),
    #[error("not found")]
    NotFound,
    /// The server's own explanation, e.g. "TMDB isn't responding".
    #[error("{0}")]
    Server(String),
    #[error("unexpected response from the server: {0}")]
    Decode(String),
}

impl Client {
    /// `base` is the server's URL without a trailing slash.
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client builds with static settings");
        Self {
            http,
            base: base.to_owned(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, ClientError> {
        let results: SearchResults = self.get("/api/search", &[("q", query)]).await?;
        Ok(results.results)
    }

    pub async fn title(&self, kind: Kind, id: u64) -> Result<Title, ClientError> {
        self.get(&format!("/api/{}/{id}", kind.slug()), &[]).await
    }

    pub async fn person(&self, id: u64) -> Result<Person, ClientError> {
        self.get(&format!("/api/person/{id}"), &[]).await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .await
            .map_err(|_| ClientError::Unreachable(self.base.clone()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|_| ClientError::Unreachable(self.base.clone()))?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ClientError::NotFound);
        }
        if !status.is_success() {
            return Err(match serde_json::from_slice::<ApiError>(&body) {
                Ok(e) => ClientError::Server(e.error),
                Err(_) => ClientError::Server(format!("the server answered {status}")),
            });
        }
        serde_json::from_slice(&body).map_err(|e| ClientError::Decode(e.to_string()))
    }
}
