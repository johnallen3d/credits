//! Home Assistant's WebSocket API: each change to a media player as it
//! happens, with the position in what's playing.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::PlaybackEvent;
use super::home_assistant::{HomeAssistant, HomeAssistantError, StateRow};

/// How long connecting and signing in may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// After this long without a message, Home Assistant is pinged; after twice
/// this long, the connection counts as lost.
const PING_AFTER: Duration = Duration::from_secs(30);

/// The subscription's message ID.
const SUBSCRIPTION: u64 = 1;

/// A signed-in connection subscribed to state changes.
pub struct Live {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    players: Vec<String>,
    next_id: u64,
    pinged: bool,
}

#[derive(Deserialize)]
struct Incoming {
    #[serde(rename = "type")]
    kind: String,
    id: Option<u64>,
    success: Option<bool>,
    error: Option<Value>,
    message: Option<String>,
    event: Option<Event>,
}

#[derive(Deserialize)]
struct Event {
    data: EventData,
}

#[derive(Deserialize)]
struct EventData {
    entity_id: String,
    /// Left as JSON: only the players' states are read.
    new_state: Option<Value>,
}

impl Live {
    /// Connects, signs in with the token and subscribes to state changes.
    pub async fn connect(
        ha: &HomeAssistant,
        players: &[String],
    ) -> Result<Self, HomeAssistantError> {
        tokio::time::timeout(CONNECT_TIMEOUT, Self::open(ha, players))
            .await
            .unwrap_or_else(|_| Err(unavailable("connecting to Home Assistant timed out")))
    }

    async fn open(ha: &HomeAssistant, players: &[String]) -> Result<Self, HomeAssistantError> {
        let url = websocket_url(&ha.url);
        let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .map_err(|e| unavailable(&format!("connecting to {url} failed: {e}")))?;
        let mut live = Self {
            ws,
            players: players.to_vec(),
            next_id: SUBSCRIPTION + 1,
            pinged: false,
        };
        live.expect("auth_required").await?;
        live.send(json!({ "type": "auth", "access_token": ha.token }))
            .await?;
        let reply = live.receive().await?;
        match reply.kind.as_str() {
            "auth_ok" => {}
            "auth_invalid" => return Err(HomeAssistantError::Unauthorized),
            other => return Err(unexpected(other)),
        }
        live.send(json!({
            "id": SUBSCRIPTION,
            "type": "subscribe_events",
            "event_type": "state_changed",
        }))
        .await?;
        let reply = live.receive().await?;
        if reply.kind != "result" || reply.id != Some(SUBSCRIPTION) {
            return Err(unexpected(&reply.kind));
        }
        if reply.success != Some(true) {
            return Err(unavailable(&format!(
                "Home Assistant refused the subscription: {}",
                reply.error.unwrap_or_default()
            )));
        }
        Ok(live)
    }

    /// The next change to one of the players, with its entity ID. Fails once
    /// the connection is lost.
    pub async fn next(&mut self) -> Result<(String, PlaybackEvent), HomeAssistantError> {
        loop {
            let message = match tokio::time::timeout(PING_AFTER, self.receive()).await {
                Ok(message) => message?,
                Err(_) if self.pinged => {
                    return Err(unavailable("Home Assistant stopped answering"));
                }
                Err(_) => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.send(json!({ "id": id, "type": "ping" })).await?;
                    self.pinged = true;
                    continue;
                }
            };
            let Some(Event { data }) = message.event else {
                continue;
            };
            if !self.players.contains(&data.entity_id) {
                continue;
            }
            // `None`: the player was removed.
            let Some(state) = data.new_state else {
                continue;
            };
            match serde_json::from_value::<StateRow>(state) {
                Ok(row) => return Ok((data.entity_id, row.into())),
                Err(e) => {
                    tracing::warn!(error = %e, player = data.entity_id, "unreadable player state")
                }
            }
        }
    }

    async fn send(&mut self, message: Value) -> Result<(), HomeAssistantError> {
        self.ws
            .send(Message::text(message.to_string()))
            .await
            .map_err(|e| unavailable(&format!("connection to Home Assistant lost: {e}")))
    }

    /// The next JSON message. Any message shows the connection is alive.
    async fn receive(&mut self) -> Result<Incoming, HomeAssistantError> {
        loop {
            let message = match self.ws.next().await {
                None => return Err(unavailable("Home Assistant closed the connection")),
                Some(Err(e)) => {
                    return Err(unavailable(&format!(
                        "connection to Home Assistant lost: {e}"
                    )));
                }
                Some(Ok(message)) => message,
            };
            self.pinged = false;
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => {
                    return Err(unavailable("Home Assistant closed the connection"));
                }
                _ => continue,
            };
            match serde_json::from_str::<Incoming>(&text) {
                Ok(incoming) => return Ok(incoming),
                Err(e) => tracing::warn!(error = %e, "unreadable Home Assistant message"),
            }
        }
    }

    async fn expect(&mut self, kind: &str) -> Result<(), HomeAssistantError> {
        let message = self.receive().await?;
        if message.kind == kind {
            Ok(())
        } else {
            Err(unexpected(
                message.message.as_deref().unwrap_or(&message.kind),
            ))
        }
    }
}

/// `http://homeassistant:8123` → `ws://homeassistant:8123/api/websocket`.
fn websocket_url(base: &str) -> String {
    let rest = base
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            base.strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })
        .unwrap_or_else(|| base.to_owned());
    format!("{rest}/api/websocket")
}

fn unavailable(message: &str) -> HomeAssistantError {
    HomeAssistantError::Unavailable(message.to_owned())
}

fn unexpected(kind: &str) -> HomeAssistantError {
    unavailable(&format!("unexpected Home Assistant message: {kind}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_follows_the_http_scheme() {
        assert_eq!(
            websocket_url("http://homeassistant:8123"),
            "ws://homeassistant:8123/api/websocket"
        );
        assert_eq!(
            websocket_url("https://ha.example.com"),
            "wss://ha.example.com/api/websocket"
        );
    }
}
