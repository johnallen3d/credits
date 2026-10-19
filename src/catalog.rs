//! Application service: turns provider responses into `model` types.

use crate::cache::ProviderCache;
use crate::config::Config;
use crate::model::{MediaKind, Ratings, SearchHit, TitleDetail};
use crate::omdb::OmdbClient;
use crate::ratings::RatingsService;
use crate::tmdb::{self, MultiResult, TmdbClient};

const POSTER_SIZE: &str = "w342";
const BACKDROP_SIZE: &str = "w1280";

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("not found")]
    NotFound,
    #[error("TMDB rejected the API token")]
    Unauthorized,
    #[error("{0}")]
    Provider(String),
}

#[derive(Clone)]
pub struct Catalog {
    tmdb: TmdbClient,
    image_base: String,
    ratings: RatingsService,
}

impl Catalog {
    /// Opens the provider cache database, so this fails if it can't be created or migrated.
    pub async fn new(config: &Config) -> Result<Self, sqlx::Error> {
        let cache = ProviderCache::open(&config.database_path).await?;
        let omdb = config
            .omdb_api_key
            .as_deref()
            .map(|key| OmdbClient::new(&config.omdb_api_base, key));
        Ok(Self {
            tmdb: TmdbClient::new(&config.tmdb_api_base, &config.tmdb_token),
            image_base: config.tmdb_image_base.trim_end_matches('/').to_owned(),
            ratings: RatingsService::new(omdb, cache, config.ratings_ttl),
        })
    }

    /// Movies and TV shows matching `query`; people and other result types are dropped.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, CatalogError> {
        let page = self.tmdb.search_multi(query).await?;
        Ok(page
            .results
            .into_iter()
            .filter_map(|result| match result {
                MultiResult::Movie(m) => Some(SearchHit {
                    kind: MediaKind::Movie,
                    id: m.id,
                    title: m.title,
                    year: year(m.release_date.as_deref()),
                    overview: non_empty(m.overview),
                    poster_url: self.image(POSTER_SIZE, m.poster_path),
                }),
                MultiResult::Tv(t) => Some(SearchHit {
                    kind: MediaKind::Tv,
                    id: t.id,
                    title: t.name,
                    year: year(t.first_air_date.as_deref()),
                    overview: non_empty(t.overview),
                    poster_url: self.image(POSTER_SIZE, t.poster_path),
                }),
                MultiResult::Other => None,
            })
            .collect())
    }

    pub async fn title(&self, kind: MediaKind, id: u64) -> Result<TitleDetail, CatalogError> {
        match kind {
            MediaKind::Movie => {
                let m = self.tmdb.movie(id).await?;
                let ratings = self
                    .ratings
                    .for_title(kind, m.external_ids.imdb_id.clone())
                    .await;
                Ok(self.movie(m, ratings))
            }
            MediaKind::Tv => {
                let t = self.tmdb.tv(id).await?;
                let ratings = self
                    .ratings
                    .for_title(kind, t.external_ids.imdb_id.clone())
                    .await;
                Ok(self.tv(t, ratings))
            }
        }
    }

    fn movie(&self, m: tmdb::Movie, ratings: Ratings) -> TitleDetail {
        TitleDetail {
            kind: MediaKind::Movie,
            id: m.id,
            original_title: non_empty(m.original_title).filter(|o| *o != m.title),
            title: m.title,
            tagline: non_empty(m.tagline),
            overview: non_empty(m.overview),
            year: year(m.release_date.as_deref()),
            status: non_empty(m.status),
            genres: m.genres.into_iter().map(|g| g.name).collect(),
            runtime_minutes: m.runtime.filter(|r| *r > 0),
            seasons: None,
            episodes: None,
            poster_url: self.image(POSTER_SIZE, m.poster_path),
            backdrop_url: self.image(BACKDROP_SIZE, m.backdrop_path),
            tmdb_url: format!("https://www.themoviedb.org/movie/{}", m.id),
            ratings,
        }
    }

    fn tv(&self, t: tmdb::Tv, ratings: Ratings) -> TitleDetail {
        TitleDetail {
            kind: MediaKind::Tv,
            id: t.id,
            original_title: non_empty(t.original_name).filter(|o| *o != t.name),
            title: t.name,
            tagline: non_empty(t.tagline),
            overview: non_empty(t.overview),
            year: year(t.first_air_date.as_deref()),
            status: non_empty(t.status),
            genres: t.genres.into_iter().map(|g| g.name).collect(),
            runtime_minutes: None,
            seasons: t.number_of_seasons,
            episodes: t.number_of_episodes,
            poster_url: self.image(POSTER_SIZE, t.poster_path),
            backdrop_url: self.image(BACKDROP_SIZE, t.backdrop_path),
            tmdb_url: format!("https://www.themoviedb.org/tv/{}", t.id),
            ratings,
        }
    }

    fn image(&self, size: &str, path: Option<String>) -> Option<String> {
        non_empty(path).map(|p| format!("{}/{size}{p}", self.image_base))
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn year(date: Option<&str>) -> Option<String> {
    date.and_then(|d| d.get(..4)).map(str::to_owned)
}
