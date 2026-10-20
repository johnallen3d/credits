//! Preferred ratings for a title, and IMDb ratings for a season's episodes: OMDb looked up by IMDb identifier, behind
//! the persistent provider cache. Never fails; problems become `Ratings` states.

use std::collections::HashMap;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Serialize, de::DeserializeOwned};

use crate::cache::ProviderCache;
use crate::model::{EpisodeRatings, MediaKind, Rating, Ratings, RatingsState, UnavailableReason};
use crate::omdb::{OmdbClient, OmdbError};

const PROVIDER: &str = "omdb";

#[derive(Clone)]
pub struct RatingsService {
    omdb: Option<OmdbClient>,
    cache: ProviderCache,
    ttl: Duration,
}

impl RatingsService {
    pub fn new(omdb: Option<OmdbClient>, cache: ProviderCache, ttl: Duration) -> Self {
        Self { omdb, cache, ttl }
    }

    pub async fn for_title(&self, kind: MediaKind, imdb_id: Option<String>) -> Ratings {
        let Some(imdb_id) = imdb_id.filter(|id| is_imdb_title_id(id)) else {
            return Ratings::unavailable(kind, UnavailableReason::NoImdbId, None);
        };
        let lookup = self
            .lookup(&imdb_id, |omdb| {
                let imdb_id = imdb_id.clone();
                async move { omdb.title(&imdb_id).await }
            })
            .await;
        match lookup {
            Lookup::Found(state, title, fetched_at) => {
                Ratings::from_scores(kind, state, imdb_id, title.ratings, fetched_at)
            }
            Lookup::Failed(reason) => Ratings::unavailable(kind, reason, Some(imdb_id)),
        }
    }

    /// IMDb ratings of a season's episodes, keyed by episode number, looked up
    /// by the show's IMDb identifier in one request.
    pub async fn for_season(
        &self,
        show_imdb_id: Option<String>,
        number: u32,
    ) -> (EpisodeRatings, HashMap<u32, Rating>) {
        let Some(imdb_id) = show_imdb_id.filter(|id| is_imdb_title_id(id)) else {
            return (
                EpisodeRatings::unavailable(UnavailableReason::NoImdbId, None),
                HashMap::new(),
            );
        };
        let lookup = self
            .lookup(&format!("{imdb_id}/season/{number}"), |omdb| {
                let imdb_id = imdb_id.clone();
                async move { omdb.season(&imdb_id, number).await }
            })
            .await;
        match lookup {
            Lookup::Found(state, season, fetched_at) => (
                EpisodeRatings::found(state, imdb_id, fetched_at),
                season
                    .episodes
                    .into_iter()
                    .filter_map(|e| Some((e.number, e.rating?)))
                    .collect(),
            ),
            Lookup::Failed(reason) => (
                EpisodeRatings::unavailable(reason, Some(imdb_id)),
                HashMap::new(),
            ),
        }
    }

    /// The cached value while fresh, else a refetch, else whatever is cached.
    async fn lookup<T, F, Fut>(&self, key: &str, fetch: F) -> Lookup<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce(OmdbClient) -> Fut,
        Fut: Future<Output = Result<T, OmdbError>>,
    {
        let cached = self
            .cache
            .get::<T>(PROVIDER, key)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, key, "ratings cache read failed");
                None
            });
        let now = Timestamp::now();
        let cached = match cached {
            Some(c) if c.is_fresh(now) => {
                return Lookup::Found(RatingsState::Fresh, c.value, c.fetched_at);
            }
            other => other,
        };

        let failure = match &self.omdb {
            Some(omdb) => match fetch(omdb.clone()).await {
                Ok(value) => {
                    if let Err(e) = self.cache.put(PROVIDER, key, &value, now, self.ttl).await {
                        tracing::warn!(error = %e, key, "ratings cache write failed");
                    }
                    return Lookup::Found(RatingsState::Fresh, value, now);
                }
                Err(e) => {
                    tracing::warn!(error = %e, key, "OMDb lookup failed");
                    UnavailableReason::ProviderError
                }
            },
            None => UnavailableReason::NotConfigured,
        };

        match cached {
            Some(c) => Lookup::Found(RatingsState::Stale, c.value, c.fetched_at),
            None => Lookup::Failed(failure),
        }
    }
}

enum Lookup<T> {
    Found(RatingsState, T, Timestamp),
    Failed(UnavailableReason),
}

/// IMDb title identifiers look like `tt0133093`; anything else is not looked up.
fn is_imdb_title_id(id: &str) -> bool {
    id.strip_prefix("tt")
        .is_some_and(|digits| digits.len() >= 7 && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_imdb_title_ids_are_looked_up() {
        assert!(is_imdb_title_id("tt0133093"));
        assert!(is_imdb_title_id("tt10872600"));
        assert!(!is_imdb_title_id(""));
        assert!(!is_imdb_title_id("nm0000206"));
        assert!(!is_imdb_title_id("tt12ab"));
    }
}
