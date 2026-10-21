//! HTTP access to a running credits server's JSON API.

use std::time::Duration;

use serde::de::DeserializeOwned;

use super::api::{
    ApiError, Episode, Kind, List, MarksUpdate, Person, RecentKind, RecentView, RecentViews,
    SearchHit, SearchResults, Season, Title, TitleCredits, Watched, WatchedList,
};
use super::app::{ListName, ListView};

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

    /// Recently viewed title and person pages, most recent first.
    pub async fn recent(&self) -> Result<Vec<RecentView>, ClientError> {
        let views: RecentViews = self.get("/api/recent", &[]).await?;
        Ok(views.recent)
    }

    /// Counts a title or person page as viewed, as opening it on the web does.
    pub async fn record_view(
        &self,
        kind: RecentKind,
        id: u64,
    ) -> Result<Vec<RecentView>, ClientError> {
        let body = serde_json::json!({ "kind": kind.slug(), "id": id });
        let views: RecentViews = self
            .send(
                self.http
                    .post(format!("{}/api/recent", self.base))
                    .json(&body),
            )
            .await?;
        Ok(views.recent)
    }

    pub async fn title(&self, kind: Kind, id: u64) -> Result<Title, ClientError> {
        self.get(&format!("/api/{}/{id}", kind.slug()), &[]).await
    }

    /// A title's whole cast and crew.
    pub async fn credits(&self, kind: Kind, id: u64) -> Result<TitleCredits, ClientError> {
        self.get(&format!("/api/{}/{id}/credits", kind.slug()), &[])
            .await
    }

    pub async fn person(&self, id: u64) -> Result<Person, ClientError> {
        self.get(&format!("/api/person/{id}"), &[]).await
    }

    pub async fn season(&self, show: u64, number: u32) -> Result<Season, ClientError> {
        self.get(&format!("/api/tv/{show}/season/{number}"), &[])
            .await
    }

    pub async fn episode(
        &self,
        show: u64,
        season: u32,
        number: u32,
    ) -> Result<Episode, ClientError> {
        self.get(
            &format!("/api/tv/{show}/season/{season}/episode/{number}"),
            &[],
        )
        .await
    }

    /// The To watch list or Favorites, with the same filters as the web page.
    pub async fn list(&self, list: ListName, view: ListView) -> Result<List, ClientError> {
        let path = match list {
            ListName::Watchlist => "/api/watchlist",
            ListName::Favorites => "/api/favorites",
            ListName::Watched => {
                return Err(ClientError::Server("Watched isn't a saved list".to_owned()));
            }
        };
        let mut query = Vec::new();
        if let Some(kind) = view.kind {
            query.push(("kind", kind.slug()));
        }
        if view.alt_sort {
            query.push(("sort", "rating"));
        }
        if view.mine && list == ListName::Watchlist {
            query.push(("services", "mine"));
        }
        self.get(path, &query).await
    }

    /// Every watched movie and show, with the same filters as the web page.
    pub async fn watched(&self, view: ListView) -> Result<WatchedList, ClientError> {
        let mut query = Vec::new();
        if let Some(kind) = view.kind {
            query.push(("kind", kind.slug()));
        }
        if let Some(source) = view.source.query() {
            query.push(("source", source));
        }
        if view.alt_sort {
            query.push(("sort", "title"));
        }
        self.get("/api/watched", &query).await
    }

    /// Puts a title on To watch or Favorites, or takes it off.
    pub async fn set_listed(
        &self,
        list: ListName,
        kind: Kind,
        id: u64,
        listed: bool,
    ) -> Result<MarksUpdate, ClientError> {
        let list = match list {
            ListName::Watchlist => "watchlist",
            ListName::Favorites => "favorite",
            ListName::Watched => {
                return Err(ClientError::Server("that can't be listed".to_owned()));
            }
        };
        let path = format!("/api/{}/{id}/{list}", kind.slug());
        self.put(&path, serde_json::json!({ "listed": listed }))
            .await
    }

    /// Marks a movie, or a TV show as a whole, watched or not.
    pub async fn set_title_watched(
        &self,
        kind: Kind,
        id: u64,
        watched: bool,
    ) -> Result<MarksUpdate, ClientError> {
        let path = format!("/api/{}/{id}/watched", kind.slug());
        self.put(&path, serde_json::json!({ "watched": watched }))
            .await
    }

    /// Marks a season, or one of its episodes, watched or not; answers with
    /// the show's marks.
    pub async fn set_watched(
        &self,
        show: u64,
        season: u32,
        episode: Option<u32>,
        watched: bool,
    ) -> Result<Watched, ClientError> {
        let path = match episode {
            Some(number) => format!("/api/tv/{show}/season/{season}/episode/{number}/watched"),
            None => format!("/api/tv/{show}/season/{season}/watched"),
        };
        self.put(&path, serde_json::json!({ "watched": watched }))
            .await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T, ClientError> {
        let request = self.http.get(format!("{}{path}", self.base)).query(query);
        self.send(request).await
    }

    async fn put<T: DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let request = self.http.put(format!("{}{path}", self.base)).json(&body);
        self.send(request).await
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ClientError> {
        let response = request
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
