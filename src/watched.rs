//! TV shows, seasons, episodes and movies marked as watched, stored next to
//! the provider cache.

use std::collections::{BTreeMap, HashSet};

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::{MediaKind, MovieWatched, WatchTarget, Watched};

/// One movie or show's watched marks, summed up for the Watched list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchedRecord {
    pub kind: MediaKind,
    pub id: u64,
    /// Unix seconds of the latest mark.
    pub last_watched_at: i64,
    /// TV: the whole show is marked.
    pub show: bool,
    /// TV: seasons marked one at a time, ascending.
    pub seasons: Vec<u32>,
    /// TV: episodes marked one at a time.
    pub episodes: u32,
    /// Some mark was made in the web UI (shows and seasons always are).
    pub manual: bool,
    /// Some mark came from what played on an Apple TV.
    pub apple_tv: bool,
}

impl WatchedRecord {
    fn new(kind: MediaKind, id: u64) -> Self {
        Self {
            kind,
            id,
            last_watched_at: i64::MIN,
            show: false,
            seasons: Vec::new(),
            episodes: 0,
            manual: false,
            apple_tv: false,
        }
    }

    fn add(&mut self, at: i64, source: &str) {
        self.last_watched_at = self.last_watched_at.max(at);
        if source == "apple_tv" {
            self.apple_tv = true;
        } else {
            self.manual = true;
        }
    }
}

#[derive(Clone)]
pub struct WatchedShows {
    pool: SqlitePool,
}

impl WatchedShows {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Shows marked watched as a whole.
    pub async fn shows(&self) -> Result<HashSet<u64>, sqlx::Error> {
        let shows: Vec<(i64,)> = sqlx::query_as("SELECT tv_id FROM watched_shows")
            .fetch_all(&self.pool)
            .await?;
        Ok(shows
            .into_iter()
            .filter_map(|(id,)| u64::try_from(id).ok())
            .collect())
    }

    pub async fn get(&self, tv_id: u64) -> Result<Watched, sqlx::Error> {
        let show: Option<(i64,)> =
            sqlx::query_as("SELECT tv_id FROM watched_shows WHERE tv_id = ?")
                .bind(tv_id as i64)
                .fetch_optional(&self.pool)
                .await?;
        let seasons: Vec<(i64,)> =
            sqlx::query_as("SELECT season FROM watched_seasons WHERE tv_id = ? ORDER BY season")
                .bind(tv_id as i64)
                .fetch_all(&self.pool)
                .await?;
        let episodes: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT season, episode, source FROM watched_episodes WHERE tv_id = ?
             ORDER BY season, episode",
        )
        .bind(tv_id as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(Watched {
            show: show.is_some(),
            seasons: seasons
                .into_iter()
                .filter_map(|(n,)| u32::try_from(n).ok())
                .collect(),
            apple_tv: episodes
                .iter()
                .filter(|(_, _, source)| source == "apple_tv")
                .filter_map(|(s, e, _)| Some((u32::try_from(*s).ok()?, u32::try_from(*e).ok()?)))
                .collect(),
            episodes: episodes
                .into_iter()
                .filter_map(|(s, e, _)| Some((u32::try_from(s).ok()?, u32::try_from(e).ok()?)))
                .collect(),
        })
    }

    /// Marks `target` watched or not. Each target keeps its own mark, so
    /// unwatching a show leaves the seasons and episodes marked within it.
    pub async fn set(
        &self,
        tv_id: u64,
        target: WatchTarget,
        watched: bool,
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        let tv_id = tv_id as i64;
        let query = match (target, watched) {
            (WatchTarget::Show, true) => {
                sqlx::query("INSERT OR IGNORE INTO watched_shows (tv_id, watched_at) VALUES (?, ?)")
                    .bind(tv_id)
                    .bind(at.as_second())
            }
            (WatchTarget::Show, false) => {
                sqlx::query("DELETE FROM watched_shows WHERE tv_id = ?").bind(tv_id)
            }
            (WatchTarget::Season(season), true) => sqlx::query(
                "INSERT OR IGNORE INTO watched_seasons (tv_id, season, watched_at) VALUES (?, ?, ?)",
            )
            .bind(tv_id)
            .bind(season)
            .bind(at.as_second()),
            (WatchTarget::Season(season), false) => {
                sqlx::query("DELETE FROM watched_seasons WHERE tv_id = ? AND season = ?")
                    .bind(tv_id)
                    .bind(season)
            }
            (WatchTarget::Episode { season, number }, true) => sqlx::query(
                "INSERT OR IGNORE INTO watched_episodes (tv_id, season, episode, watched_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(tv_id)
            .bind(season)
            .bind(number)
            .bind(at.as_second()),
            (WatchTarget::Episode { season, number }, false) => sqlx::query(
                "DELETE FROM watched_episodes WHERE tv_id = ? AND season = ? AND episode = ?",
            )
            .bind(tv_id)
            .bind(season)
            .bind(number),
        };
        query.execute(&self.pool).await.map(drop)
    }

    /// Marks an episode watched by `source` (e.g. "apple_tv"), keeping an
    /// existing mark and its source.
    pub async fn mark_episode(
        &self,
        tv_id: u64,
        season: u32,
        episode: u32,
        source: &str,
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT OR IGNORE INTO watched_episodes (tv_id, season, episode, watched_at, source)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(tv_id as i64)
        .bind(season)
        .bind(episode)
        .bind(at.as_second())
        .bind(source)
        .execute(&self.pool)
        .await
        .map(drop)
    }

    /// Every movie and show with any watched mark, however it was made,
    /// most recently watched first.
    pub async fn all(&self) -> Result<Vec<WatchedRecord>, sqlx::Error> {
        let movies: Vec<(i64, i64, String)> =
            sqlx::query_as("SELECT movie_id, watched_at, source FROM watched_movies")
                .fetch_all(&self.pool)
                .await?;
        let shows: Vec<(i64, i64)> = sqlx::query_as("SELECT tv_id, watched_at FROM watched_shows")
            .fetch_all(&self.pool)
            .await?;
        let seasons: Vec<(i64, i64, i64)> =
            sqlx::query_as("SELECT tv_id, season, watched_at FROM watched_seasons ORDER BY season")
                .fetch_all(&self.pool)
                .await?;
        let episodes: Vec<(i64, i64, String)> =
            sqlx::query_as("SELECT tv_id, watched_at, source FROM watched_episodes")
                .fetch_all(&self.pool)
                .await?;

        let mut records: Vec<WatchedRecord> = movies
            .into_iter()
            .filter_map(|(id, at, source)| {
                let mut r = WatchedRecord::new(MediaKind::Movie, u64::try_from(id).ok()?);
                r.add(at, &source);
                Some(r)
            })
            .collect();
        let mut tv: BTreeMap<u64, WatchedRecord> = BTreeMap::new();
        fn show(tv: &mut BTreeMap<u64, WatchedRecord>, id: i64) -> Option<&mut WatchedRecord> {
            let id = u64::try_from(id).ok()?;
            Some(
                tv.entry(id)
                    .or_insert_with(|| WatchedRecord::new(MediaKind::Tv, id)),
            )
        }
        for (id, at) in shows {
            if let Some(r) = show(&mut tv, id) {
                r.add(at, "manual");
                r.show = true;
            }
        }
        for (id, season, at) in seasons {
            if let (Some(r), Ok(season)) = (show(&mut tv, id), u32::try_from(season)) {
                r.add(at, "manual");
                r.seasons.push(season);
            }
        }
        for (id, at, source) in episodes {
            if let Some(r) = show(&mut tv, id) {
                r.add(at, &source);
                r.episodes += 1;
            }
        }
        records.extend(tv.into_values());
        records.sort_by_key(|r| std::cmp::Reverse(r.last_watched_at));
        Ok(records)
    }

    pub async fn movie(&self, movie_id: u64) -> Result<MovieWatched, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT source FROM watched_movies WHERE movie_id = ?")
                .bind(movie_id as i64)
                .fetch_optional(&self.pool)
                .await?;
        Ok(MovieWatched {
            watched: row.is_some(),
            apple_tv: row.is_some_and(|(source,)| source == "apple_tv"),
        })
    }

    /// Marks a movie watched or not from the web UI. Marking keeps an
    /// existing mark and its source.
    pub async fn set_movie(
        &self,
        movie_id: u64,
        watched: bool,
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        if watched {
            self.mark_movie(movie_id, "manual", at).await
        } else {
            sqlx::query("DELETE FROM watched_movies WHERE movie_id = ?")
                .bind(movie_id as i64)
                .execute(&self.pool)
                .await
                .map(drop)
        }
    }

    /// Marks a movie watched by `source` ("manual" or "apple_tv"), keeping
    /// an existing mark and its source.
    pub async fn mark_movie(
        &self,
        movie_id: u64,
        source: &str,
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT OR IGNORE INTO watched_movies (movie_id, watched_at, source) VALUES (?, ?, ?)",
        )
        .bind(movie_id as i64)
        .bind(at.as_second())
        .bind(source)
        .execute(&self.pool)
        .await
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{IN_MEMORY, ProviderCache};

    #[tokio::test]
    async fn movie_marks_keep_their_source() {
        let watched = WatchedShows::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        assert_eq!(watched.movie(105).await.unwrap(), MovieWatched::default());
        watched.mark_movie(105, "apple_tv", at).await.unwrap();
        watched.set_movie(105, true, at).await.unwrap();
        let apple_tv = MovieWatched {
            watched: true,
            apple_tv: true,
        };
        assert_eq!(watched.movie(105).await.unwrap(), apple_tv);
        assert_eq!(watched.movie(603).await.unwrap(), MovieWatched::default());
        watched.set_movie(105, false, at).await.unwrap();
        watched.set_movie(105, true, at).await.unwrap();
        assert_eq!(
            watched.movie(105).await.unwrap(),
            MovieWatched {
                watched: true,
                apple_tv: false,
            }
        );
    }

    #[tokio::test]
    async fn all_sums_up_each_title_newest_first() {
        let watched = WatchedShows::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
        let at = |s: i64| Timestamp::from_second(1_700_000_000 + s).unwrap();
        watched.mark_movie(105, "apple_tv", at(10)).await.unwrap();
        watched.set_movie(603, true, at(30)).await.unwrap();
        watched
            .set(1, WatchTarget::Season(2), true, at(5))
            .await
            .unwrap();
        watched
            .set(1, WatchTarget::Season(1), true, at(1))
            .await
            .unwrap();
        watched
            .mark_episode(1, 3, 1, "apple_tv", at(20))
            .await
            .unwrap();
        watched
            .mark_episode(1, 3, 2, "apple_tv", at(2))
            .await
            .unwrap();
        watched
            .set(7, WatchTarget::Show, true, at(0))
            .await
            .unwrap();

        let all = watched.all().await.unwrap();
        let order: Vec<_> = all.iter().map(|r| (r.kind, r.id)).collect();
        assert_eq!(
            order,
            [
                (MediaKind::Movie, 603),
                (MediaKind::Tv, 1),
                (MediaKind::Movie, 105),
                (MediaKind::Tv, 7),
            ]
        );
        let show = &all[1];
        assert_eq!(show.last_watched_at, at(20).as_second());
        assert_eq!(
            (show.show, show.seasons.as_slice(), show.episodes),
            (false, &[1, 2][..], 2)
        );
        assert!(show.manual && show.apple_tv);
        assert!(all[2].apple_tv && !all[2].manual);
        assert!(all[3].show && all[3].manual && !all[3].apple_tv);
    }

    #[tokio::test]
    async fn show_season_and_episode_marks_are_independent() {
        let watched = WatchedShows::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        let set = |target, on| watched.set(1, target, on, at);
        assert_eq!(watched.get(1).await.unwrap(), Watched::default());

        set(WatchTarget::Season(2), true).await.unwrap();
        set(WatchTarget::Season(1), true).await.unwrap();
        set(WatchTarget::Season(1), true).await.unwrap();
        set(
            WatchTarget::Episode {
                season: 3,
                number: 4,
            },
            true,
        )
        .await
        .unwrap();
        watched
            .set(9, WatchTarget::Season(3), true, at)
            .await
            .unwrap();
        let w = watched.get(1).await.unwrap();
        assert_eq!(w.seasons, [1, 2]);
        assert_eq!(w.episodes, [(3, 4)]);
        assert!(w.season(2) && !w.season(3));
        assert!(w.episode(2, 7), "a season covers its episodes");
        assert!(w.episode(3, 4) && !w.episode(3, 5));

        set(WatchTarget::Show, true).await.unwrap();
        assert!(
            watched.get(1).await.unwrap().episode(5, 1),
            "the show covers everything"
        );

        set(WatchTarget::Show, false).await.unwrap();
        set(WatchTarget::Season(2), false).await.unwrap();
        set(
            WatchTarget::Episode {
                season: 3,
                number: 4,
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            watched.get(1).await.unwrap(),
            Watched {
                show: false,
                seasons: vec![1],
                episodes: vec![],
                apple_tv: vec![],
            }
        );
    }
}
