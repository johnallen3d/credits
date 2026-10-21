//! Playback progress, show mappings and read positions in SQLite, plus how
//! the last read of Home Assistant and the live feed are going (in memory).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use sqlx::sqlite::SqlitePool;

use super::{App, Batch, Detection, DetectionStatus, Kind, LiveStatus, PlaybackEvent, SyncStatus};

#[derive(Clone)]
pub struct Playback {
    pool: SqlitePool,
    status: Arc<Mutex<Status>>,
}

struct Status {
    sync: SyncStatus,
    live: LiveStatus,
    last_ok: Option<Timestamp>,
    other: BTreeMap<String, u32>,
}

/// An episode's or movie's row in `playback_progress`.
pub struct Key<'a> {
    pub player: &'a str,
    pub app: App,
    pub item_key: &'a str,
}

/// Which TMDB show or movie an app's title is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mapping {
    Show(u64),
    Ignored,
}

/// A `playback_progress` row, plus (for `ready`) whether a later episode
/// of its show has started since.
#[derive(sqlx::FromRow)]
struct Row {
    player: String,
    app: String,
    item_key: String,
    kind: String,
    show_key: String,
    show_title: String,
    title: String,
    season: i64,
    episode: i64,
    duration: Option<i64>,
    playing_seconds: i64,
    max_position: Option<i64>,
    last_seen: i64,
    status: String,
    tmdb_id: Option<i64>,
    note: Option<String>,
    #[sqlx(default)]
    next_started: bool,
}

const COLUMNS: &str = "player, app, item_key, kind, show_key, show_title, title, season, \
     episode, duration, playing_seconds, max_position, last_seen, status, tmdb_id, note";

impl Playback {
    pub fn new(pool: SqlitePool, configured: bool) -> Self {
        let (sync, live) = if configured {
            (SyncStatus::Starting, LiveStatus::Connecting)
        } else {
            (SyncStatus::NotConfigured, LiveStatus::NotConfigured)
        };
        Self {
            pool,
            status: Arc::new(Mutex::new(Status {
                sync,
                live,
                last_ok: None,
                other: BTreeMap::new(),
            })),
        }
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().expect("status lock").sync.clone()
    }

    pub fn live(&self) -> LiveStatus {
        self.status.lock().expect("status lock").live.clone()
    }

    /// Records the live feed connecting, or failing with `error`. A failure
    /// keeps the time the feed was first lost.
    pub fn record_live(&self, result: Result<(), String>, at: Timestamp) {
        let mut status = self.status.lock().expect("status lock");
        status.live = match (result, &status.live) {
            (Ok(()), _) => LiveStatus::Connected { since: at },
            (Err(error), LiveStatus::Disconnected { since, .. }) => LiveStatus::Disconnected {
                since: *since,
                error,
            },
            (Err(error), _) => LiveStatus::Disconnected { since: at, error },
        };
    }

    /// Playing time on other apps since the server started, most first.
    pub fn other(&self) -> Vec<(String, u32)> {
        let mut other: Vec<_> = self
            .status
            .lock()
            .expect("status lock")
            .other
            .iter()
            .map(|(app, s)| (app.clone(), *s))
            .collect();
        other.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        other
    }

    pub fn record_sync(&self, result: Result<(), String>, at: Timestamp) {
        let mut status = self.status.lock().expect("status lock");
        status.sync = match result {
            Ok(()) => {
                status.last_ok = Some(at);
                SyncStatus::Ok { at }
            }
            Err(error) => SyncStatus::Failed {
                at,
                error,
                last_ok: status.last_ok,
            },
        };
    }

    /// The last history row read for `player`.
    pub async fn cursor(&self, player: &str) -> Result<Option<PlaybackEvent>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT event FROM playback_cursor WHERE player = ?")
                .bind(player)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.and_then(|(json,)| {
            serde_json::from_str(&json)
                .inspect_err(|e| tracing::warn!(error = %e, player, "unreadable playback cursor"))
                .ok()
        }))
    }

    /// Adds `batch` to each episode's progress and moves the read position,
    /// together, so no stretch of playing is counted twice.
    pub async fn ingest(&self, player: &str, batch: &Batch) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        for seen in &batch.seen {
            sqlx::query(
                "INSERT INTO playback_progress (player, app, item_key, kind, show_key,
                     show_title, title, season, episode, duration, playing_seconds,
                     max_position, first_seen, last_seen)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (player, app, item_key) DO UPDATE SET
                     duration = max(coalesce(duration, 0), coalesce(excluded.duration, 0)),
                     playing_seconds = playing_seconds + excluded.playing_seconds,
                     max_position = nullif(max(coalesce(max_position, 0),
                                               coalesce(excluded.max_position, 0)), 0),
                     first_seen = min(first_seen, excluded.first_seen),
                     last_seen = max(last_seen, excluded.last_seen)",
            )
            .bind(player)
            .bind(seen.item.app.id())
            .bind(&seen.item.key)
            .bind(seen.item.kind.id())
            .bind(&seen.item.show_key)
            .bind(&seen.item.show_title)
            .bind(&seen.item.title)
            .bind(seen.item.season)
            .bind(seen.item.episode)
            .bind(seen.duration)
            .bind(seen.playing_seconds)
            .bind(seen.max_position)
            .bind(seen.first_seen.as_second())
            .bind(seen.last_seen.as_second())
            .execute(&mut *tx)
            .await?;
        }
        if let Some(open) = &batch.open {
            let json = serde_json::to_string(open).expect("events serialize");
            sqlx::query(
                "INSERT INTO playback_cursor (player, event) VALUES (?, ?)
                 ON CONFLICT (player) DO UPDATE SET event = excluded.event",
            )
            .bind(player)
            .bind(json)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        let mut status = self.status.lock().expect("status lock");
        for (app, seconds) in &batch.other {
            *status.other.entry(app.clone()).or_default() += seconds;
        }
        Ok(())
    }

    /// Episodes and movies not handled yet that may have been watched,
    /// oldest first:
    /// 80% of their duration played or reached, or 5 minutes played if the
    /// player didn't say
    /// how long they are. Each comes with whether a later episode of its
    /// show has started since.
    pub async fn ready(&self) -> Result<Vec<(Detection, bool)>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS}, EXISTS (
                 SELECT 1 FROM playback_progress n
                 WHERE n.player = p.player AND n.app = p.app AND n.show_key = p.show_key
                   AND n.kind = 'episode' AND p.kind = 'episode'
                   AND (n.season > p.season OR (n.season = p.season AND n.episode > p.episode))
                   AND n.last_seen > p.last_seen) AS next_started
             FROM playback_progress p
             WHERE status = 'watching'
               AND (max(playing_seconds, coalesce(max_position, 0)) * 10 >= duration * 8
                    OR (coalesce(duration, 0) = 0 AND playing_seconds >= 300))
             ORDER BY last_seen"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let next_started = r.next_started;
                Some((detection(r)?, next_started))
            })
            .collect())
    }

    /// The most recently seen episodes and movies, newest first.
    pub async fn recent(&self, limit: u32) -> Result<Vec<Detection>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM playback_progress ORDER BY last_seen DESC LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(detection).collect())
    }

    /// Episodes and movies waiting for their show or movie to be chosen,
    /// newest first.
    pub async fn waiting(&self) -> Result<Vec<Detection>, sqlx::Error> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM playback_progress
             WHERE status IN ('needs_confirmation', 'no_match')
             ORDER BY last_seen DESC"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(detection).collect())
    }

    pub async fn set_status(
        &self,
        key: &Key<'_>,
        status: DetectionStatus,
        tmdb_id: Option<u64>,
        note: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE playback_progress SET status = ?, tmdb_id = ?, note = ?
             WHERE player = ? AND app = ? AND item_key = ?",
        )
        .bind(status.as_str())
        .bind(tmdb_id.map(|id| id as i64))
        .bind(note)
        .bind(key.player)
        .bind(key.app.id())
        .bind(key.item_key)
        .execute(&self.pool)
        .await
        .map(drop)
    }

    pub async fn mapping(
        &self,
        app: App,
        kind: Kind,
        show_key: &str,
    ) -> Result<Option<Mapping>, sqlx::Error> {
        let row: Option<(Option<i64>,)> = sqlx::query_as(
            "SELECT tmdb_id FROM playback_mappings WHERE app = ? AND kind = ? AND show_key = ?",
        )
        .bind(app.id())
        .bind(kind.id())
        .bind(show_key)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(tv_id,)| match tv_id {
            Some(id) => Mapping::Show(id as u64),
            None => Mapping::Ignored,
        }))
    }

    /// Records which show or movie `show_key` is; `by` is "auto" or "user".
    pub async fn set_mapping(
        &self,
        app: App,
        kind: Kind,
        show_key: &str,
        mapping: Mapping,
        by: &str,
        at: Timestamp,
    ) -> Result<(), sqlx::Error> {
        let tmdb_id = match mapping {
            Mapping::Show(id) => Some(id as i64),
            Mapping::Ignored => None,
        };
        sqlx::query(
            "INSERT INTO playback_mappings (app, kind, show_key, tmdb_id, decided_by, decided_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (app, kind, show_key) DO UPDATE SET
                 tmdb_id = excluded.tmdb_id,
                 decided_by = excluded.decided_by,
                 decided_at = excluded.decided_at",
        )
        .bind(app.id())
        .bind(kind.id())
        .bind(show_key)
        .bind(tmdb_id)
        .bind(by)
        .bind(at.as_second())
        .execute(&self.pool)
        .await
        .map(drop)
    }

    /// Puts a show's waiting and ignored episodes (or a movie's plays) back
    /// to be resolved again.
    pub async fn reopen_show(
        &self,
        app: App,
        kind: Kind,
        show_key: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE playback_progress SET status = 'watching', note = NULL
             WHERE app = ? AND kind = ? AND show_key = ?
               AND status IN ('needs_confirmation', 'no_match', 'ignored')",
        )
        .bind(app.id())
        .bind(kind.id())
        .bind(show_key)
        .execute(&self.pool)
        .await
        .map(drop)
    }
}

fn detection(row: Row) -> Option<Detection> {
    Some(Detection {
        player: row.player,
        app: App::from_id(&row.app)?,
        item_key: row.item_key,
        kind: Kind::from_id(&row.kind)?,
        show_key: row.show_key,
        show_title: row.show_title,
        title: row.title,
        season: u32::try_from(row.season).ok()?,
        episode: u32::try_from(row.episode).ok()?,
        duration: row
            .duration
            .and_then(|d| u32::try_from(d).ok())
            .filter(|d| *d > 0),
        playing_seconds: u32::try_from(row.playing_seconds).unwrap_or(u32::MAX),
        max_position: row.max_position.and_then(|p| u32::try_from(p).ok()),
        last_seen: Timestamp::from_second(row.last_seen).ok()?,
        status: DetectionStatus::from_id(&row.status)?,
        tmdb_id: row.tmdb_id.and_then(|id| u64::try_from(id).ok()),
        note: row.note,
    })
}
