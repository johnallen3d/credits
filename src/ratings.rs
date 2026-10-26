//! Preferred ratings for a title, and IMDb ratings for a season's episodes: OMDb looked up by IMDb identifier, behind
//! the persistent provider cache, with episodes OMDb has no score for filled in from IMDb's dataset. Never fails;
//! problems become `Ratings` states.

use std::collections::HashMap;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Serialize, de::DeserializeOwned};

use crate::cache::ProviderCache;
use crate::imdb::ImdbRatings;
use crate::model::{EpisodeRatings, MediaKind, Rating, Ratings, RatingsState, UnavailableReason};
use crate::omdb::{OmdbClient, OmdbError, OmdbSeason};

const PROVIDER: &str = "omdb";

#[derive(Clone)]
pub struct RatingsService {
    omdb: Option<OmdbClient>,
    imdb: ImdbRatings,
    cache: ProviderCache,
    ttl: Duration,
}

impl RatingsService {
    pub fn new(
        omdb: Option<OmdbClient>,
        imdb: ImdbRatings,
        cache: ProviderCache,
        ttl: Duration,
    ) -> Self {
        Self {
            omdb,
            imdb,
            cache,
            ttl,
        }
    }

    /// IMDb's dataset, which fills in episodes OMDb has no score for.
    pub fn imdb(&self) -> &ImdbRatings {
        &self.imdb
    }

    /// Whether an OMDb key is set; without one nothing new can be fetched.
    pub fn is_configured(&self) -> bool {
        self.omdb.is_some()
    }

    pub async fn for_title(&self, kind: MediaKind, imdb_id: Option<String>) -> Ratings {
        let Some(imdb_id) = imdb_id.filter(|id| is_imdb_title_id(id)) else {
            return Ratings::unavailable(kind, UnavailableReason::NoImdbId, None);
        };
        let lookup = self
            .lookup(
                &imdb_id,
                |_| true,
                |omdb| {
                    let imdb_id = imdb_id.clone();
                    async move { omdb.title(&imdb_id).await }
                },
            )
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
            .lookup(
                &format!("{imdb_id}/season/{number}"),
                // Not when cached before episode identifiers were kept, so
                // nothing could be filled in from IMDb's dataset.
                |season: &OmdbSeason| {
                    season.episodes.is_empty()
                        || season.episodes.iter().any(|e| e.imdb_id.is_some())
                },
                |omdb| {
                    let imdb_id = imdb_id.clone();
                    async move { omdb.season(&imdb_id, number).await }
                },
            )
            .await;
        match lookup {
            Lookup::Found(state, season, fetched_at) => {
                let unscored: Vec<&str> = season
                    .episodes
                    .iter()
                    .filter(|e| e.rating.is_none())
                    .filter_map(|e| e.imdb_id.as_deref())
                    .collect();
                let mut dataset = self.imdb.scores(&unscored).await.unwrap_or_else(|e| {
                    tracing::warn!(error = %e, imdb_id, number, "reading IMDb ratings failed");
                    HashMap::new()
                });
                let scores = season
                    .episodes
                    .into_iter()
                    .filter_map(|e| {
                        let rating = e.rating.or_else(|| dataset.remove(e.imdb_id.as_deref()?))?;
                        Some((e.number, rating))
                    })
                    .collect();
                (EpisodeRatings::found(state, imdb_id, fetched_at), scores)
            }
            Lookup::Failed(reason) => (
                EpisodeRatings::unavailable(reason, Some(imdb_id)),
                HashMap::new(),
            ),
        }
    }

    /// The cached value while fresh and `complete`, else a refetch, else
    /// whatever is cached.
    async fn lookup<T, F, Fut>(
        &self,
        key: &str,
        complete: impl Fn(&T) -> bool,
        fetch: F,
    ) -> Lookup<T>
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
            Some(c) if c.is_fresh(now) && complete(&c.value) => {
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
