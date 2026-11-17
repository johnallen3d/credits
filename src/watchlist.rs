//! Persistent title lists (To watch and Favorites), beside the provider cache.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use sqlx::Transaction;
use sqlx::sqlite::{Sqlite, SqlitePool};

use crate::model::{FilmographyEntry, MediaKind};

/// kind, id, title, year, poster_url
type Row = (String, i64, String, Option<String>, Option<String>);

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

    /// Takes a title off the list, with its sources.
    pub async fn remove(&self, kind: MediaKind, id: u64) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(&format!(
            "DELETE FROM {} WHERE kind = ? AND id = ?",
            self.table
        ))
        .bind(kind.slug())
        .bind(id as i64)
        .execute(&mut *tx)
        .await?;
        if self.has_sources() {
            Self::clear_sources(&mut tx, kind, id).await?;
        }
        tx.commit().await
    }

    /// Most recently added first; To watch titles carry their sources.
    pub async fn list(&self) -> Result<Vec<FilmographyEntry>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT kind, id, title, year, poster_url FROM {}
             ORDER BY added_at DESC, rowid DESC",
            self.table,
        ))
        .fetch_all(&self.pool)
        .await?;
        let mut sources = if self.has_sources() {
            self.title_sources().await?
        } else {
            HashMap::new()
        };
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id, title, year, poster_url)| {
                let kind = MediaKind::from_slug(&kind)?;
                let id = u64::try_from(id).ok()?;
                Some(FilmographyEntry {
                    kind,
                    id,
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
                    sources: sources.remove(&(kind, id)).unwrap_or_default(),
                })
            })
            .collect())
    }

    fn has_sources(&self) -> bool {
        self.table == "watchlist"
    }

    /// Every saved title's sources, each in the order added.
    async fn title_sources(&self) -> Result<HashMap<(MediaKind, u64), Vec<String>>, sqlx::Error> {
        let rows: Vec<(String, i64, String)> =
            sqlx::query_as("SELECT kind, id, name FROM watchlist_title_sources ORDER BY rowid")
                .fetch_all(&self.pool)
                .await?;
        let mut sources: HashMap<_, Vec<String>> = HashMap::new();
        for (kind, id, name) in rows {
            if let (Some(kind), Ok(id)) = (MediaKind::from_slug(&kind), u64::try_from(id)) {
                sources.entry((kind, id)).or_default().push(name);
            }
        }
        Ok(sources)
    }

    /// Tags a saved title with each of `names` it doesn't already have. A
    /// name matching an earlier one but for case takes the earlier spelling.
    /// False, and nothing saved, when the title isn't on the list.
    pub async fn add_sources(
        &self,
        kind: MediaKind,
        id: u64,
        names: &[String],
        at: Timestamp,
    ) -> Result<bool, sqlx::Error> {
        debug_assert!(self.has_sources());
        let mut tx = self.pool.begin().await?;
        if !Self::is_saved(&mut tx, kind, id).await? {
            return Ok(false);
        }
        Self::insert_sources(&mut tx, kind, id, names, at).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Replaces a saved title's sources with `names`, in that order. False,
    /// and nothing saved, when the title isn't on the list.
    pub async fn set_sources(
        &self,
        kind: MediaKind,
        id: u64,
        names: &[String],
        at: Timestamp,
    ) -> Result<bool, sqlx::Error> {
        debug_assert!(self.has_sources());
        let mut tx = self.pool.begin().await?;
        if !Self::is_saved(&mut tx, kind, id).await? {
            return Ok(false);
        }
        Self::clear_sources(&mut tx, kind, id).await?;
        Self::insert_sources(&mut tx, kind, id, names, at).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Takes a source off a saved title, matching case-insensitively. It's
    /// still offered for other titles.
    pub async fn remove_source(
        &self,
        kind: MediaKind,
        id: u64,
        name: &str,
    ) -> Result<(), sqlx::Error> {
        debug_assert!(self.has_sources());
        sqlx::query("DELETE FROM watchlist_title_sources WHERE kind = ? AND id = ? AND name = ?")
            .bind(kind.slug())
            .bind(id as i64)
            .bind(name)
            .execute(&self.pool)
            .await
            .map(drop)
    }

    async fn is_saved(
        tx: &mut Transaction<'_, Sqlite>,
        kind: MediaKind,
        id: u64,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM watchlist WHERE kind = ? AND id = ?")
                .bind(kind.slug())
                .bind(id as i64)
                .fetch_optional(&mut **tx)
                .await?;
        Ok(row.is_some())
    }

    async fn clear_sources(
        tx: &mut Transaction<'_, Sqlite>,
        kind: MediaKind,
        id: u64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM watchlist_title_sources WHERE kind = ? AND id = ?")
            .bind(kind.slug())
            .bind(id as i64)
            .execute(&mut **tx)
            .await
            .map(drop)
    }

    async fn insert_sources(
        tx: &mut Transaction<'_, Sqlite>,
        kind: MediaKind,
        id: u64,
        names: &[String],
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        for name in names {
            sqlx::query(
                "INSERT INTO watchlist_sources (name, used_at) VALUES (?, ?)
                 ON CONFLICT (name) DO UPDATE SET used_at = excluded.used_at",
            )
            .bind(name)
            .bind(at.as_second())
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO watchlist_title_sources (kind, id, name)
                 SELECT ?, ?, name FROM watchlist_sources WHERE name = ?",
            )
            .bind(kind.slug())
            .bind(id as i64)
            .bind(name)
            .execute(&mut **tx)
            .await?;
        }
        Ok(())
    }

    /// A saved title's sources in the order added; none when it isn't saved.
    pub async fn sources_of(&self, kind: MediaKind, id: u64) -> Result<Vec<String>, sqlx::Error> {
        debug_assert!(self.has_sources());
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM watchlist_title_sources WHERE kind = ? AND id = ? ORDER BY rowid",
        )
        .bind(kind.slug())
        .bind(id as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(name,)| name).collect())
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
            sources: Vec::new(),
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
    async fn sources_are_tags_kept_and_offered_after_their_titles_leave() {
        let pool = ProviderCache::open(IN_MEMORY).await.unwrap().pool();
        let list = Watchlist::new(pool);
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        let later = Timestamp::from_second(1_700_000_100).unwrap();
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        list.add(&title(MediaKind::Movie, 1, "One"), at)
            .await
            .unwrap();
        list.add(&title(MediaKind::Tv, 2, "Show"), at)
            .await
            .unwrap();

        assert!(
            !list
                .add_sources(MediaKind::Movie, 9, &names(&["sam"]), at)
                .await
                .unwrap()
        );
        assert!(list.sources().await.unwrap().is_empty());

        assert!(
            list.add_sources(MediaKind::Movie, 1, &names(&["Sam", "jazzy"]), at)
                .await
                .unwrap()
        );
        assert!(
            list.add_sources(MediaKind::Tv, 2, &names(&["podcast"]), later)
                .await
                .unwrap()
        );
        assert_eq!(list.sources().await.unwrap(), ["podcast", "jazzy", "Sam"]);

        // Adding again or in another case keeps one tag, in the first spelling.
        let latest = Timestamp::from_second(1_700_000_200).unwrap();
        assert!(
            list.add_sources(MediaKind::Movie, 1, &names(&["JAZZY"]), latest)
                .await
                .unwrap()
        );
        assert!(
            list.add_sources(MediaKind::Tv, 2, &names(&["sam"]), latest)
                .await
                .unwrap()
        );
        assert_eq!(
            list.sources_of(MediaKind::Movie, 1).await.unwrap(),
            ["Sam", "jazzy"]
        );
        let sources: Vec<_> = list
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.sources)
            .collect();
        assert_eq!(
            sources,
            [names(&["podcast", "Sam"]), names(&["Sam", "jazzy"])]
        );

        // Re-adding the title keeps its tags.
        list.add(&title(MediaKind::Movie, 1, "One"), latest)
            .await
            .unwrap();
        list.remove_source(MediaKind::Movie, 1, "SAM")
            .await
            .unwrap();
        assert_eq!(
            list.sources_of(MediaKind::Movie, 1).await.unwrap(),
            ["jazzy"]
        );

        assert!(
            list.set_sources(MediaKind::Movie, 1, &names(&["work", "Podcast"]), latest)
                .await
                .unwrap()
        );
        assert_eq!(
            list.sources_of(MediaKind::Movie, 1).await.unwrap(),
            ["work", "podcast"]
        );

        // Leaving the list drops a title's tags but not the names.
        list.remove(MediaKind::Tv, 2).await.unwrap();
        list.add(&title(MediaKind::Tv, 2, "Show"), latest)
            .await
            .unwrap();
        assert!(list.sources_of(MediaKind::Tv, 2).await.unwrap().is_empty());
        assert_eq!(list.sources().await.unwrap().len(), 4);
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
