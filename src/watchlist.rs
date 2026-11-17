//! Persistent title lists (To watch and Favorites), beside the provider cache.

use std::collections::HashSet;

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::{FilmographyEntry, MediaKind};

/// kind, id, title, year, poster_url, source
type Row = (
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Clone)]
pub struct Watchlist {
    pool: SqlitePool,
    // Only constructors choose the table; never interpolated from user input.
    table: &'static str,
}

impl Watchlist {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            table: "watchlist",
        }
    }

    pub fn favorites(pool: SqlitePool) -> Self {
        Self {
            pool,
            table: "favorites",
        }
    }

    /// Saves `title`. Adding it again refreshes its display fields but keeps
    /// its place in the list.
    pub async fn add(&self, title: &FilmographyEntry, at: Timestamp) -> Result<(), sqlx::Error> {
        self.add_with_watched_seasons(title, &[], at).await
    }

    /// Saves a favorite and its current TV seasons in one transaction. Never
    /// sets the whole-show mark, which would include future seasons.
    pub async fn add_with_watched_seasons(
        &self,
        title: &FilmographyEntry,
        seasons: &[u32],
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "INSERT INTO {} (kind, id, title, year, poster_url, added_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (kind, id) DO UPDATE SET
                 title = excluded.title, year = excluded.year, poster_url = excluded.poster_url",
            self.table,
        ))
        .bind(title.kind.slug())
        .bind(title.id as i64)
        .bind(&title.title)
        .bind(&title.year)
        .bind(&title.poster_url)
        .bind(at.as_second())
        .execute(&mut *tx)
        .await?;
        for season in seasons {
            sqlx::query(
                "INSERT OR IGNORE INTO watched_seasons (tv_id, season, watched_at) VALUES (?, ?, ?)",
            )
            .bind(title.id as i64)
            .bind(season)
            .bind(at.as_second())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    pub async fn remove(&self, kind: MediaKind, id: u64) -> Result<(), sqlx::Error> {
        sqlx::query(&format!(
            "DELETE FROM {} WHERE kind = ? AND id = ?",
            self.table
        ))
        .bind(kind.slug())
        .bind(id as i64)
        .execute(&self.pool)
        .await
        .map(drop)
    }

    /// Most recently added first.
    pub async fn list(&self) -> Result<Vec<FilmographyEntry>, sqlx::Error> {
        // Only To watch records sources.
        let source = if self.has_sources() { "source" } else { "NULL" };
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT kind, id, title, year, poster_url, {source} FROM {}
             ORDER BY added_at DESC, rowid DESC",
            self.table,
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id, title, year, poster_url, source)| {
                Some(FilmographyEntry {
                    kind: MediaKind::from_slug(&kind)?,
                    id: u64::try_from(id).ok()?,
                    title,
                    year,
                    role: None,
                    episode_count: None,
                    poster_url,
                    rating: None,
                    availability: None,
                    marks: None,
                    in_theaters: false,
                    next_release: None,
                    release_status: None,
                    source,
                })
            })
            .collect())
    }

    fn has_sources(&self) -> bool {
        self.table == "watchlist"
    }

    /// Sets who or what recommended a saved title, or clears it with `None`.
    /// A source matching an earlier one but for case takes the earlier
    /// spelling. False, and nothing saved, when the title isn't on the list.
    pub async fn set_source(
        &self,
        kind: MediaKind,
        id: u64,
        source: Option<&str>,
        at: Timestamp,
    ) -> Result<bool, sqlx::Error> {
        debug_assert!(self.has_sources());
        let mut tx = self.pool.begin().await?;
        let source = match source {
            Some(name) => {
                let known: Option<(String,)> =
                    sqlx::query_as("SELECT name FROM watchlist_sources WHERE name = ?")
                        .bind(name)
                        .fetch_optional(&mut *tx)
                        .await?;
                Some(known.map_or_else(|| name.to_owned(), |(known,)| known))
            }
            None => None,
        };
        let updated = sqlx::query("UPDATE watchlist SET source = ? WHERE kind = ? AND id = ?")
            .bind(&source)
            .bind(kind.slug())
            .bind(id as i64)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if updated == 0 {
            return Ok(false);
        }
        if let Some(name) = &source {
            sqlx::query(
                "INSERT INTO watchlist_sources (name, used_at) VALUES (?, ?)
                 ON CONFLICT (name) DO UPDATE SET used_at = excluded.used_at",
            )
            .bind(name)
            .bind(at.as_second())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Who or what recommended a saved title; `None` when unset or not saved.
    pub async fn source(&self, kind: MediaKind, id: u64) -> Result<Option<String>, sqlx::Error> {
        debug_assert!(self.has_sources());
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT source FROM watchlist WHERE kind = ? AND id = ?")
                .bind(kind.slug())
                .bind(id as i64)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(source,)| source))
    }

    /// Every source used so far, including for titles no longer on the
    /// list, most recently used first.
    pub async fn sources(&self) -> Result<Vec<String>, sqlx::Error> {
        debug_assert!(self.has_sources());
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM watchlist_sources ORDER BY used_at DESC, name")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(|(name,)| name).collect())
    }

    /// Which titles are saved, for marking them while browsing.
    pub async fn titles(&self) -> Result<HashSet<(MediaKind, u64)>, sqlx::Error> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as(&format!("SELECT kind, id FROM {}", self.table))
                .fetch_all(&self.pool)
                .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id)| Some((MediaKind::from_slug(&kind)?, u64::try_from(id).ok()?)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{IN_MEMORY, ProviderCache};

    fn title(kind: MediaKind, id: u64, name: &str) -> FilmographyEntry {
        FilmographyEntry {
            kind,
            id,
            title: name.to_owned(),
            year: None,
            role: None,
            episode_count: None,
            poster_url: None,
            rating: None,
            availability: None,
            marks: None,
            in_theaters: false,
            next_release: None,
            release_status: None,
            source: None,
        }
    }

    #[tokio::test]
    async fn newest_first_and_re_adding_keeps_the_original_place() {
        let pool = ProviderCache::open(IN_MEMORY).await.unwrap().pool();
        for list in [
            Watchlist::new(pool.clone()),
            Watchlist::favorites(pool.clone()),
        ] {
            assert_list_order_and_identity(list).await;
        }
    }

    #[tokio::test]
    async fn sources_are_kept_and_offered_after_their_titles_leave() {
        let pool = ProviderCache::open(IN_MEMORY).await.unwrap().pool();
        let list = Watchlist::new(pool);
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        let later = Timestamp::from_second(1_700_000_100).unwrap();
        list.add(&title(MediaKind::Movie, 1, "One"), at)
            .await
            .unwrap();
        list.add(&title(MediaKind::Tv, 2, "Show"), at)
            .await
            .unwrap();

        assert!(
            !list
                .set_source(MediaKind::Movie, 9, Some("Sam"), at)
                .await
                .unwrap()
        );
        assert!(list.sources().await.unwrap().is_empty());

        assert!(
            list.set_source(MediaKind::Movie, 1, Some("Sam"), at)
                .await
                .unwrap()
        );
        assert!(
            list.set_source(MediaKind::Tv, 2, Some("A podcast"), later)
                .await
                .unwrap()
        );
        assert_eq!(list.sources().await.unwrap(), ["A podcast", "Sam"]);

        // Re-adding keeps the source; a different case reuses the first spelling.
        list.add(&title(MediaKind::Movie, 1, "One"), later)
            .await
            .unwrap();
        assert_eq!(
            list.source(MediaKind::Movie, 1).await.unwrap().as_deref(),
            Some("Sam")
        );
        let latest = Timestamp::from_second(1_700_000_200).unwrap();
        assert!(
            list.set_source(MediaKind::Tv, 2, Some("sam"), latest)
                .await
                .unwrap()
        );
        assert_eq!(
            list.source(MediaKind::Tv, 2).await.unwrap().as_deref(),
            Some("Sam")
        );
        assert_eq!(list.sources().await.unwrap(), ["Sam", "A podcast"]);
        let sources: Vec<_> = list
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.source)
            .collect();
        assert_eq!(sources, [Some("Sam".to_owned()), Some("Sam".to_owned())]);

        list.set_source(MediaKind::Movie, 1, None, latest)
            .await
            .unwrap();
        assert_eq!(list.source(MediaKind::Movie, 1).await.unwrap(), None);
        list.remove(MediaKind::Tv, 2).await.unwrap();
        assert_eq!(list.sources().await.unwrap(), ["Sam", "A podcast"]);
    }

    async fn assert_list_order_and_identity(list: Watchlist) {
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        list.add(&title(MediaKind::Movie, 1, "One"), at)
            .await
            .unwrap();
        list.add(&title(MediaKind::Tv, 1, "Show"), at)
            .await
            .unwrap();
        list.add(&title(MediaKind::Movie, 2, "Two"), at)
            .await
            .unwrap();
        let later = Timestamp::from_second(1_700_000_100).unwrap();
        list.add(&title(MediaKind::Movie, 1, "One, renamed"), later)
            .await
            .unwrap();

        let names: Vec<String> = list
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert_eq!(names, ["Two", "Show", "One, renamed"]);
        assert!(list.titles().await.unwrap().contains(&(MediaKind::Tv, 1)));

        list.remove(MediaKind::Movie, 1).await.unwrap();
        list.remove(MediaKind::Movie, 99).await.unwrap();
        let titles = list.titles().await.unwrap();
        assert_eq!(titles.len(), 2);
        assert!(!titles.contains(&(MediaKind::Movie, 1)));
    }
}
