//! Recently viewed title and person pages, stored next to the provider cache.

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::{RecentKind, RecentView};

/// Most entries kept; older ones are dropped as new pages are viewed.
pub const CAPACITY: i64 = 12;

/// kind, id, name, year, image_url
type Row = (String, i64, String, Option<String>, Option<String>);

#[derive(Clone)]
pub struct RecentlyViewed {
    pool: SqlitePool,
}

impl RecentlyViewed {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Moves `view` to the front, replacing any earlier visit to the same page.
    pub async fn record(&self, view: &RecentView, at: Timestamp) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT OR REPLACE INTO recently_viewed (kind, id, name, year, image_url, viewed_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(view.kind.slug())
        .bind(view.id as i64)
        .bind(&view.name)
        .bind(&view.year)
        .bind(&view.image_url)
        .bind(at.as_second())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM recently_viewed WHERE rowid NOT IN (
                 SELECT rowid FROM recently_viewed ORDER BY viewed_at DESC, rowid DESC LIMIT ?
             )",
        )
        .bind(CAPACITY)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    /// Most recent first.
    pub async fn list(&self) -> Result<Vec<RecentView>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT kind, id, name, year, image_url FROM recently_viewed
             ORDER BY viewed_at DESC, rowid DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(kind, id, name, year, image_url)| {
                Some(RecentView {
                    kind: RecentKind::from_slug(&kind)?,
                    id: u64::try_from(id).ok()?,
                    name,
                    year,
                    image_url,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{IN_MEMORY, ProviderCache};

    fn movie(id: u64) -> RecentView {
        RecentView {
            kind: RecentKind::Movie,
            id,
            name: format!("Movie {id}"),
            year: None,
            image_url: None,
        }
    }

    #[tokio::test]
    async fn keeps_only_the_most_recent_entries() {
        let recent = RecentlyViewed::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        for id in 1..=CAPACITY as u64 + 3 {
            recent.record(&movie(id), at).await.unwrap();
        }
        recent.record(&movie(5), at).await.unwrap();

        let ids: Vec<u64> = recent.list().await.unwrap().iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), CAPACITY as usize);
        assert_eq!(ids[..3], [5, 15, 14], "same-second visits keep their order");
        assert_eq!(ids.last(), Some(&4), "the three oldest are dropped");
        assert_eq!(ids.iter().filter(|&&id| id == 5).count(), 1);
    }
}
