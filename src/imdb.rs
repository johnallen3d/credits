//! IMDb's non-commercial ratings dataset, downloaded once a day into SQLite,
//! for episodes OMDb has no score for: its episode data lags IMDb's, often
//! by weeks. https://developer.imdb.com/non-commercial-datasets/

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::time::Duration;

use flate2::read::GzDecoder;
use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::cache::ProviderCache;
use crate::model::{Rating, RatingSource};

const PROVIDER: &str = "imdb";
const KEY: &str = "title.ratings";
/// IMDb refreshes the dataset daily.
const TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often to check whether the copy is due, and retry after a failure.
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
/// Rows inserted per statement.
const CHUNK: usize = 20_000;

/// IMDb identifier digits, score out of 10, votes.
type Row = (i64, f64, i64);

#[derive(Clone)]
pub struct ImdbRatings {
    http: reqwest::Client,
    url: String,
    cache: ProviderCache,
    pool: SqlitePool,
}

#[derive(Debug, thiserror::Error)]
pub enum ImdbError {
    #[error("downloading IMDb ratings failed: {0}")]
    Download(String),
    #[error("reading IMDb ratings failed: {0}")]
    Read(#[from] std::io::Error),
    #[error("storing IMDb ratings failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Cache(#[from] crate::cache::CacheError),
}

impl ImdbRatings {
    pub fn new(url: &str, cache: ProviderCache) -> Self {
        let http = reqwest::Client::builder()
            // About 9 MB, fetched in the background.
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static settings");
        Self {
            http,
            url: url.to_owned(),
            pool: cache.pool(),
            cache,
        }
    }

    /// IMDb scores of the given titles that the dataset has, by identifier.
    pub async fn scores(&self, imdb_ids: &[&str]) -> Result<HashMap<String, Rating>, sqlx::Error> {
        let titles: Vec<i64> = imdb_ids.iter().filter_map(|id| title_number(id)).collect();
        if titles.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT title, score, votes FROM imdb_ratings
             WHERE title IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(&titles).expect("integers serialize"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(title, score, votes)| {
                let rating = Rating {
                    source: RatingSource::Imdb,
                    score,
                    scale: 10,
                    votes: u64::try_from(votes).ok(),
                };
                (format!("tt{title:07}"), rating)
            })
            .collect())
    }

    /// Downloads the dataset if the stored copy is a day old or missing;
    /// true if it did. A failed download keeps the stored copy.
    pub async fn refresh(&self, now: Timestamp) -> Result<bool, ImdbError> {
        if let Some(c) = self.cache.get::<usize>(PROVIDER, KEY).await?
            && c.is_fresh(now)
        {
            return Ok(false);
        }
        let response = self
            .http
            .get(&self.url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| ImdbError::Download(e.to_string()))?;
        let body = response
            .bytes()
            .await
            .map_err(|e| ImdbError::Download(e.to_string()))?;
        let rows = tokio::task::spawn_blocking(move || parse(&body[..]))
            .await
            .expect("parsing doesn't panic")?;
        self.replace(&rows).await?;
        self.cache.put(PROVIDER, KEY, &rows.len(), now, TTL).await?;
        tracing::info!(titles = rows.len(), "downloaded IMDb ratings");
        Ok(true)
    }

    /// Keeps the stored copy at most a day old, forever.
    pub async fn keep_fresh(self) {
        loop {
            if let Err(e) = self.refresh(Timestamp::now()).await {
                tracing::warn!(error = %e, "refreshing IMDb ratings failed; keeping the stored copy");
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    }

    async fn replace(&self, rows: &[Row]) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM imdb_ratings")
            .execute(&mut *tx)
            .await?;
        for chunk in rows.chunks(CHUNK) {
            sqlx::query(
                "INSERT OR REPLACE INTO imdb_ratings (title, score, votes)
                 SELECT value ->> 0, value ->> 1, value ->> 2 FROM json_each(?)",
            )
            .bind(serde_json::to_string(chunk).expect("numbers serialize"))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

/// Rows of the gzipped `tconst averageRating numVotes` file; malformed
/// lines, the header among them, are skipped.
fn parse(gzipped: impl std::io::Read) -> std::io::Result<Vec<Row>> {
    let mut rows = Vec::new();
    for line in BufReader::new(GzDecoder::new(gzipped)).lines() {
        let line = line?;
        let mut fields = line.split('\t');
        let (Some(id), Some(score), Some(votes)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let (Some(title), Ok(score), Ok(votes)) =
            (title_number(id), score.parse::<f64>(), votes.parse::<i64>())
        else {
            continue;
        };
        if score.is_finite() {
            rows.push((title, score, votes));
        }
    }
    Ok(rows)
}

/// `tt0133093` → 133093.
fn title_number(imdb_id: &str) -> Option<i64> {
    imdb_id.strip_prefix("tt")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::GzEncoder};

    use super::*;
    use crate::cache::IN_MEMORY;

    fn gzip(text: &str) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(text.as_bytes()).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn parses_rows_and_skips_the_header_and_junk() {
        let file = gzip(
            "tconst\taverageRating\tnumVotes\n\
             tt0000001\t5.7\t2195\n\
             tt33131991\t7.5\t1648\n\
             nm0000206\t1.0\t1\n\
             tt0000002\t\\N\t3\n",
        );
        assert_eq!(
            parse(&file[..]).unwrap(),
            [(1, 5.7, 2195), (33_131_991, 7.5, 1648)]
        );
    }

    #[tokio::test]
    async fn stored_scores_are_found_by_imdb_id_and_replaced_whole() {
        let cache = ProviderCache::open(IN_MEMORY).await.unwrap();
        let imdb = ImdbRatings::new("http://unused.test", cache);
        imdb.replace(&[(959_621, 9.0, 40_000), (33_131_991, 7.5, 1648)])
            .await
            .unwrap();
        let scores = imdb
            .scores(&["tt33131991", "tt0959621", "tt1054724", "bad"])
            .await
            .unwrap();
        assert_eq!(scores.len(), 2);
        assert_eq!(scores["tt0959621"].score, 9.0);
        assert_eq!(scores["tt33131991"].votes, Some(1648));

        imdb.replace(&[(1_054_724, 8.6, 30_000)]).await.unwrap();
        let scores = imdb.scores(&["tt0959621", "tt1054724"]).await.unwrap();
        assert_eq!(scores.keys().collect::<Vec<_>>(), ["tt1054724"]);
    }
}
