//! TV shows, seasons and episodes marked as watched, stored next to the
//! provider cache.

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::{WatchTarget, Watched};

#[derive(Clone)]
pub struct WatchedShows {
    pool: SqlitePool,
}

impl WatchedShows {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
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
        let episodes: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT season, episode FROM watched_episodes WHERE tv_id = ?
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
            episodes: episodes
                .into_iter()
                .filter_map(|(s, e)| Some((u32::try_from(s).ok()?, u32::try_from(e).ok()?)))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{IN_MEMORY, ProviderCache};

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
                episodes: vec![]
            }
        );
    }
}
