//! Preferred ratings for a title: OMDb looked up by IMDb identifier, behind
//! the persistent provider cache. Never fails; problems become `Ratings` states.

use std::time::Duration;

use jiff::Timestamp;

use crate::cache::ProviderCache;
use crate::model::{MediaKind, Ratings, RatingsState, UnavailableReason};
use crate::omdb::{OmdbClient, OmdbTitle};

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

        let cached = self
            .cache
            .get::<OmdbTitle>(PROVIDER, &imdb_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, %imdb_id, "ratings cache read failed");
                None
            });
        let now = Timestamp::now();
        if let Some(c) = cached.as_ref().filter(|c| c.is_fresh(now)) {
            return Ratings::from_scores(
                kind,
                RatingsState::Fresh,
                imdb_id,
                c.value.ratings.clone(),
                c.fetched_at,
            );
        }

        let failure = match &self.omdb {
            Some(omdb) => match omdb.title(&imdb_id).await {
                Ok(title) => {
                    if let Err(e) = self
                        .cache
                        .put(PROVIDER, &imdb_id, &title, now, self.ttl)
                        .await
                    {
                        tracing::warn!(error = %e, %imdb_id, "ratings cache write failed");
                    }
                    return Ratings::from_scores(
                        kind,
                        RatingsState::Fresh,
                        imdb_id,
                        title.ratings,
                        now,
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, %imdb_id, "OMDb lookup failed");
                    UnavailableReason::ProviderError
                }
            },
            None => UnavailableReason::NotConfigured,
        };

        match cached {
            Some(c) => Ratings::from_scores(
                kind,
                RatingsState::Stale,
                imdb_id,
                c.value.ratings,
                c.fetched_at,
            ),
            None => Ratings::unavailable(kind, failure, Some(imdb_id)),
        }
    }
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
