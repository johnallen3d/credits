//! Movies and TV shows saved to watch later, stored next to the provider cache.

use std::collections::HashSet;

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::{FilmographyEntry, MediaKind};

/// kind, id, title, year, poster_url
type Row = (String, i64, String, Option<String>, Option<String>);

#[derive(Clone)]
pub struct Watchlist {
    pool: SqlitePool,
}

impl Watchlist {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Saves `title`. Adding it again refreshes its display fields but keeps
    /// its place in the list.
    pub async fn add(&self, title: &FilmographyEntry, at: Timestamp) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO watchlist (kind, id, title, year, poster_url, added_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (kind, id) DO UPDATE SET
                 title = excluded.title, year = excluded.year, poster_url = excluded.poster_url",
        )
        .bind(title.kind.slug())
        .bind(title.id as i64)
        .bind(&title.title)
        .bind(&title.year)
        .bind(&title.poster_url)
        .bind(at.as_second())
        .execute(&self.pool)
        .await
        .map(drop)
    }

    pub async fn remove(&self, kind: MediaKind, id: u64) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM watchlist WHERE kind = ? AND id = ?")
            .bind(kind.slug())
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map(drop)
    }

    /// Most recently added first.
    pub async fn list(&self) -> Result<Vec<FilmographyEntry>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT kind, id, title, year, poster_url FROM watchlist
             ORDER BY added_at DESC, rowid DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id, title, year, poster_url)| {
                Some(FilmographyEntry {
                    kind: MediaKind::from_slug(&kind)?,
                    id: u64::try_from(id).ok()?,
                    title,
                    year,
                    role: None,
                    episode_count: None,
                    poster_url,
                    rating: None,
                })
            })
            .collect())
    }

    /// Which titles are saved, for marking them while browsing.
    pub async fn titles(&self) -> Result<HashSet<(MediaKind, u64)>, sqlx::Error> {
        let rows: Vec<(String, i64)> = sqlx::query_as("SELECT kind, id FROM watchlist")
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
        }
    }

    #[tokio::test]
    async fn newest_first_and_re_adding_keeps_the_original_place() {
        let list = Watchlist::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
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
