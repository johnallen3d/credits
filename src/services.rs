//! The streaming services the user subscribes to, beside the provider cache.

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use crate::model::Service;

#[derive(Clone)]
pub struct MyServices {
    pool: SqlitePool,
}

impl MyServices {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Alphabetical by name.
    pub async fn list(&self) -> Result<Vec<Service>, sqlx::Error> {
        let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT provider_id, name, logo_url FROM my_services ORDER BY name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, name, logo_url)| {
                Some(Service {
                    id: u64::try_from(id).ok()?,
                    name,
                    logo_url,
                })
            })
            .collect())
    }

    /// Replaces the whole choice. Services chosen before keep their
    /// `added_at`; their name and logo are refreshed.
    pub async fn replace(&self, services: &[Service], at: Timestamp) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let ids: Vec<i64> = services.iter().map(|s| s.id as i64).collect();
        sqlx::query(
            "DELETE FROM my_services WHERE provider_id NOT IN (SELECT value FROM json_each(?))",
        )
        .bind(serde_json::to_string(&ids).expect("integers serialize"))
        .execute(&mut *tx)
        .await?;
        for s in services {
            sqlx::query(
                "INSERT INTO my_services (provider_id, name, logo_url, added_at) VALUES (?, ?, ?, ?)
                 ON CONFLICT (provider_id) DO UPDATE SET
                     name = excluded.name, logo_url = excluded.logo_url",
            )
            .bind(s.id as i64)
            .bind(&s.name)
            .bind(&s.logo_url)
            .bind(at.as_second())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{IN_MEMORY, ProviderCache};

    fn service(id: u64, name: &str) -> Service {
        Service {
            id,
            name: name.to_owned(),
            logo_url: None,
        }
    }

    #[tokio::test]
    async fn replacing_keeps_only_the_new_choice_in_name_order() {
        let services = MyServices::new(ProviderCache::open(IN_MEMORY).await.unwrap().pool());
        let at = Timestamp::from_second(1_700_000_000).unwrap();
        services
            .replace(&[service(8, "Netflix"), service(337, "Disney Plus")], at)
            .await
            .unwrap();
        services
            .replace(&[service(8, "Netflix"), service(1899, "max")], at)
            .await
            .unwrap();
        let names: Vec<String> = services
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["max", "Netflix"]);
        services.replace(&[], at).await.unwrap();
        assert!(services.list().await.unwrap().is_empty());
    }
}
