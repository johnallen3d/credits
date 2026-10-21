//! Home Assistant's REST history API: what a media player did over a period.
//! The live feed is in `live`.

use std::time::Duration;

use jiff::Timestamp;
use reqwest::StatusCode;
use serde::Deserialize;

use super::PlaybackEvent;
use crate::config::HomeAssistantConfig;

#[derive(Clone)]
pub struct HomeAssistant {
    http: reqwest::Client,
    pub(super) url: String,
    pub(super) token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HomeAssistantError {
    #[error("Home Assistant rejected the token (HA_TOKEN)")]
    Unauthorized,
    #[error("{0}")]
    Unavailable(String),
}

impl HomeAssistant {
    pub fn new(config: &HomeAssistantConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("credits/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client builds with static settings");
        Self {
            http,
            url: config.url.clone(),
            token: config.token.clone(),
        }
    }

    /// Every state and attribute change of `player` from `start` to `end`,
    /// oldest first. The first row is the state at `start`, so it may be
    /// dated earlier.
    pub async fn history(
        &self,
        player: &str,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<Vec<PlaybackEvent>, HomeAssistantError> {
        let path = format!("/api/history/period/{start}");
        let response = self
            .http
            .get(format!("{}{path}", self.url))
            .bearer_auth(&self.token)
            .query(&[
                ("filter_entity_id", player),
                ("end_time", &end.to_string()),
                // Attribute changes too, such as the next episode starting
                // while the player stays in "playing".
                ("significant_changes_only", "0"),
            ])
            .send()
            .await
            .map_err(|e| {
                HomeAssistantError::Unavailable(format!("request to Home Assistant failed: {e}"))
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(HomeAssistantError::Unauthorized)
            }
            status if !status.is_success() => Err(HomeAssistantError::Unavailable(format!(
                "Home Assistant history returned {status}"
            ))),
            _ => {
                let entities: Vec<Vec<StateRow>> = response.json().await.map_err(|e| {
                    HomeAssistantError::Unavailable(format!(
                        "unexpected Home Assistant history response: {e}"
                    ))
                })?;
                Ok(entities
                    .into_iter()
                    .flatten()
                    .map(PlaybackEvent::from)
                    .collect())
            }
        }
    }
}

/// A media player's state: a row of history, or the new state in a live
/// `state_changed` event.
#[derive(Deserialize)]
pub(super) struct StateRow {
    state: String,
    #[serde(default)]
    attributes: Attributes,
    /// Changes with each attribute update, unlike `last_changed`.
    last_updated: Timestamp,
}

#[derive(Deserialize, Default)]
struct Attributes {
    app_id: Option<String>,
    app_name: Option<String>,
    media_title: Option<String>,
    media_content_id: Option<String>,
    media_duration: Option<f64>,
    /// Live state only.
    media_position: Option<f64>,
    media_position_updated_at: Option<Timestamp>,
}

impl From<StateRow> for PlaybackEvent {
    fn from(row: StateRow) -> Self {
        let a = row.attributes;
        PlaybackEvent {
            at: row.last_updated,
            playing: row.state == "playing",
            app_id: a.app_id,
            app_name: a.app_name,
            title: a.media_title.filter(|t| !t.trim().is_empty()),
            content_id: a.media_content_id.filter(|c| !c.is_empty()),
            duration: a
                .media_duration
                .filter(|d| d.is_finite() && *d > 0.0)
                .map(|d| d.round() as u32),
            position: a
                .media_position
                .filter(|p| p.is_finite() && *p >= 0.0)
                .map(|p| p.round() as u32),
            position_at: a.media_position_updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rows_become_events() {
        let rows: Vec<Vec<StateRow>> = serde_json::from_str(
            r#"[[
              {"entity_id":"media_player.living_room_appletv","state":"playing",
               "attributes":{"volume_level":0.7,"media_content_id":"A0178901003",
                 "media_content_type":"video","media_duration":2461,"media_title":"Widow’s Bay",
                 "app_id":"com.apple.TVWatchList","app_name":"TV","friendly_name":"Living Room AppleTV"},
               "last_changed":"2026-10-11T20:25:21.000000+00:00",
               "last_updated":"2026-10-11T20:27:31.123456+00:00"},
              {"entity_id":"media_player.living_room_appletv","state":"off","attributes":{},
               "last_changed":"2026-10-11T23:00:00+00:00","last_updated":"2026-10-11T23:00:00+00:00"},
              {"state":"playing","attributes":{"media_title":"","app_name":"YouTube","media_duration":993.4},
               "last_updated":"2026-10-12T12:00:40.376841+00:00"}
            ]]"#,
        )
        .unwrap();
        let events: Vec<PlaybackEvent> = rows.into_iter().flatten().map(Into::into).collect();
        assert_eq!(
            events[0],
            PlaybackEvent {
                at: "2026-10-11T20:27:31.123456Z".parse().unwrap(),
                playing: true,
                app_id: Some("com.apple.TVWatchList".to_owned()),
                app_name: Some("TV".to_owned()),
                title: Some("Widow’s Bay".to_owned()),
                content_id: Some("A0178901003".to_owned()),
                duration: Some(2461),
                position: None,
                position_at: None,
            }
        );
        assert!(!events[1].playing);
        assert_eq!(events[1].app_name, None);
        assert_eq!(events[2].title, None);
        assert_eq!(events[2].duration, Some(993));
    }
}
