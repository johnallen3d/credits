//! Marks TV episodes and movies watched from what played on an Apple TV, as
//! Home Assistant's history reports it.
//!
//! Home Assistant's WebSocket reports each change as it happens, with the
//! position in the episode. After connecting, the history the recorder kept
//! is read from where the last read stopped, to catch up on time credits
//! wasn't running. Both feed the same pipeline: playing time adds up per
//! episode across sessions, and once it (or the furthest position reached)
//! is 90% of the episode's duration the episode is matched to TMDB and
//! marked watched.
//! Only Infuse (show, season and episode are in the title; any other title is
//! taken to be a movie's) and Apple's TV app (an episode's content ID ends in
//! the season and episode; other feature-length videos are movies) are
//! understood.

pub mod home_assistant;
mod live;
mod store;
mod sync;

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use home_assistant::HomeAssistant;
pub use live::Live;
pub(crate) use store::Playback;
pub(crate) use sync::{candidates, decide};
pub use sync::{run, sync_once};

/// A player's state at one moment: one row of Home Assistant's history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PlaybackEvent {
    pub at: Timestamp,
    pub playing: bool,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub content_id: Option<String>,
    /// Seconds.
    pub duration: Option<u32>,
    /// Seconds into the episode at `position_at`. Only the live feed has it;
    /// the recorder doesn't keep it.
    #[serde(default)]
    pub position: Option<u32>,
    #[serde(default)]
    pub position_at: Option<Timestamp>,
}

/// Apps whose titles say which episode is playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum App {
    Infuse,
    /// Apple's TV app.
    Tv,
}

impl App {
    pub fn id(self) -> &'static str {
        match self {
            App::Infuse => "infuse",
            App::Tv => "tv",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "infuse" => Some(App::Infuse),
            "tv" => Some(App::Tv),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            App::Infuse => "Infuse",
            App::Tv => "TV app",
        }
    }
}

/// Whether something seen playing is a TV episode or a movie.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Episode,
    Movie,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Episode => "episode",
            Kind::Movie => "movie",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "episode" => Some(Kind::Episode),
            "movie" => Some(Kind::Movie),
            _ => None,
        }
    }

    /// What is matched to TMDB: an episode's show, or the movie.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Episode => "show",
            Kind::Movie => "movie",
        }
    }

    pub fn is_movie(self) -> bool {
        self == Kind::Movie
    }
}

/// An episode or movie identified from a history row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub app: App,
    /// The app's own key for it: Infuse's title, the TV app's content ID.
    pub key: String,
    pub kind: Kind,
    /// `show_title` normalized, for matching it to a TMDB show or movie.
    pub show_key: String,
    /// The episode's show, or the movie's title without its year.
    pub show_title: String,
    /// The title as the app reported it.
    pub title: String,
    /// 0 for movies.
    pub season: u32,
    /// 0 for movies.
    pub episode: u32,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Item(Item),
    /// Something else is on: another app, or a title these apps don't
    /// describe as an episode. Named for the playback page.
    Other(String),
    /// The app sent no title. Infuse sometimes does for minutes at a time
    /// after resuming an episode.
    Untitled(App),
    Nothing,
}

const INFUSE_ID: &str = "com.firecore.infuse";
const TV_ID: &str = "com.apple.TVWatchList";
/// The shortest video from the TV app taken to be a movie: 40 minutes, the
/// Academy's line between a short and a feature.
const TV_MOVIE_SECONDS: u32 = 40 * 60;

pub fn parse(event: &PlaybackEvent) -> Parsed {
    let title = event.title.as_deref().map(str::trim).unwrap_or_default();
    match event.app_id.as_deref() {
        Some(INFUSE_ID) if title.is_empty() => Parsed::Untitled(App::Infuse),
        Some(INFUSE_ID) => match infuse_episode(title) {
            Some((show, season, episode)) => {
                Parsed::Item(item(App::Infuse, title, show, title, season, episode))
            }
            None => Parsed::Item(Item {
                kind: Kind::Movie,
                ..item(App::Infuse, title, movie_title(title).0, title, 0, 0)
            }),
        },
        Some(TV_ID) => {
            let content_id = event.content_id.as_deref().unwrap_or_default();
            match content_id_episode(content_id) {
                Some((season, episode)) if !title.is_empty() => {
                    Parsed::Item(item(App::Tv, content_id, title, title, season, episode))
                }
                _ if title.is_empty() => Parsed::Untitled(App::Tv),
                // Movies have other IDs, like "PMTFY26FTR018". Shorter
                // videos are trailers and extras.
                _ if !content_id.is_empty()
                    && event.duration.is_some_and(|d| d >= TV_MOVIE_SECONDS) =>
                {
                    Parsed::Item(Item {
                        kind: Kind::Movie,
                        ..item(App::Tv, content_id, movie_title(title).0, title, 0, 0)
                    })
                }
                _ => Parsed::Other("TV app (not a TV episode or movie)".to_owned()),
            }
        }
        _ => match event.app_name.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => Parsed::Other(name.to_owned()),
            _ => Parsed::Nothing,
        },
    }
}

fn item(app: App, key: &str, show: &str, title: &str, season: u32, episode: u32) -> Item {
    Item {
        app,
        key: key.to_owned(),
        kind: Kind::Episode,
        show_key: show_key(show),
        show_title: show.to_owned(),
        title: title.to_owned(),
        season,
        episode,
    }
}

/// Infuse titles episodes "New Girl - S4 ∙ E8 - Teachers".
fn infuse_episode(title: &str) -> Option<(&str, u32, u32)> {
    for (at, marker) in title.match_indices(" - S") {
        let rest = &title[at + marker.len()..];
        let Some((season, rest)) = leading_number(rest) else {
            continue;
        };
        let Some(rest) = [" ∙ E", " · E", " • E"]
            .iter()
            .find_map(|sep| rest.strip_prefix(sep))
        else {
            continue;
        };
        let Some((episode, rest)) = leading_number(rest) else {
            continue;
        };
        let show = title[..at].trim();
        if (rest.is_empty() || rest.starts_with(" - ")) && !show.is_empty() && episode > 0 {
            return Some((show, season, episode));
        }
    }
    None
}

/// A movie title and the year it may end in: Infuse shows "Back to the
/// Future", or "Back to the Future (1985)" when the file is named that way.
pub fn movie_title(title: &str) -> (&str, Option<u32>) {
    let year = title
        .strip_suffix(')')
        .and_then(|t| t.rsplit_once(" ("))
        .filter(|(name, year)| {
            !name.trim().is_empty() && year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit())
        });
    match year {
        Some((name, year)) => (name.trim(), year.parse().ok()),
        None => (title, None),
    }
}

fn leading_number(s: &str) -> Option<(u32, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// The TV app's content IDs for episodes, like "A0178901003", end in a
/// two-digit season and a three-digit episode number (here S1 E3).
fn content_id_episode(id: &str) -> Option<(u32, u32)> {
    let digits = id.strip_prefix(|c: char| c.is_ascii_uppercase())?;
    if digits.len() < 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = digits.len();
    let season = digits[n - 5..n - 3].parse().ok()?;
    let episode = digits[n - 3..].parse().ok()?;
    (episode > 0).then_some((season, episode))
}

/// Lowercase letters and digits, single-spaced: "Widow’s Bay" and TMDB's
/// "Widow's Bay" both become "widows bay".
pub fn show_key(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `playing_seconds` of an episode `duration` seconds long counts
/// as having watched it: 90% of it, or 80% once a later episode of the same
/// show has started. Players move on to the next episode at the end credits,
/// which can be 5 minutes or more: finished episodes stopped at 82 to 89% in
/// the history checked.
pub fn is_watched(playing_seconds: u32, duration: u32, next_started: bool) -> bool {
    let tenths = if next_started { 8 } else { 9 };
    duration > 0 && u64::from(playing_seconds) * 10 >= u64::from(duration) * tenths
}

/// The end credits of a movie that may be skipped: a 2-hour movie's run 5
/// to 10 minutes.
const MOVIE_CREDITS: u32 = 10 * 60;

/// Whether `progress` seconds of a movie `duration` seconds long counts as
/// having watched it: 90% of it, or all but the last 10 minutes as long as
/// that's 80%.
pub fn movie_is_watched(progress: u32, duration: u32) -> bool {
    is_watched(progress, duration, false)
        || (is_watched(progress, duration, true)
            && progress >= duration.saturating_sub(MOVIE_CREDITS))
}

/// Whether a playing duration fits TMDB's runtime for an episode. Players
/// report a little more than TMDB (recaps, credits): 1 to 3 minutes in the
/// history checked.
pub fn runtime_fits(duration: u32, runtime_minutes: u32) -> bool {
    let (d, r) = (i64::from(duration), i64::from(runtime_minutes) * 60);
    (r - 4 * 60..=r + 8 * 60).contains(&d)
}

/// The longest single stretch of playing counted when the duration isn't
/// known yet.
const MAX_INTERVAL: u32 = 4 * 60 * 60;

/// What a run of history rows says about each episode.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub item: Item,
    pub duration: Option<u32>,
    pub playing_seconds: u32,
    /// The furthest position reached, in seconds, if the live feed said.
    pub max_position: Option<u32>,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

#[derive(Debug, Default, PartialEq)]
pub struct Batch {
    pub seen: Vec<Seen>,
    /// Seconds spent playing anything that isn't a recognized episode, by app.
    pub other: BTreeMap<String, u32>,
    /// The last row: how long it lasts is only known from the next one.
    pub open: Option<PlaybackEvent>,
}

/// Adds up playing time from `events`, continuing from `open`, the last row
/// of the previous run. Rows at or before `open` were already counted.
/// Each row in the playing state counts until the next row, at most the
/// episode's duration, so an interrupted history can't run on for hours.
/// A row without a title continues the episode before it, if it's from the
/// same app and reports the same duration. (The TV app's menus send
/// untitled rows without one.) A row with a position reaches that position
/// plus, while playing, the time until the next row.
pub fn accumulate(open: Option<PlaybackEvent>, mut events: Vec<PlaybackEvent>) -> Batch {
    if let Some(open) = &open {
        events.retain(|e| e.at > open.at);
    }
    events.sort_by_key(|e| e.at);
    let rows: Vec<PlaybackEvent> = open.into_iter().chain(events).collect();
    let mut batch = Batch::default();
    let mut last: Option<(Item, Option<u32>)> = None;
    for (i, row) in rows.iter().enumerate() {
        let next = rows.get(i + 1);
        let elapsed = next.map_or(0, |n| {
            u32::try_from(n.at.as_second() - row.at.as_second()).unwrap_or(u32::MAX)
        });
        let elapsed = if row.playing { elapsed } else { 0 };
        let parsed = match parse(row) {
            Parsed::Untitled(app) => match &last {
                Some((item, duration))
                    if item.app == app && row.duration.is_some() && row.duration == *duration =>
                {
                    Parsed::Item(item.clone())
                }
                _ => Parsed::Nothing,
            },
            parsed => parsed,
        };
        if let Parsed::Item(item) = &parsed {
            let duration = match &last {
                Some((l, d)) if l == item => row.duration.or(*d),
                _ => row.duration,
            };
            last = Some((item.clone(), duration));
            if next.is_none() {
                // Saved for the next run, which can't see the rows before it.
                let mut open = row.clone();
                open.title = Some(match item.app {
                    App::Infuse => item.key.clone(),
                    App::Tv => item.title.clone(),
                });
                if item.app == App::Tv {
                    open.content_id = Some(item.key.clone());
                }
                batch.open = Some(open);
            }
        }
        match parsed {
            Parsed::Item(item) => {
                let index = match batch
                    .seen
                    .iter()
                    .position(|s| s.item.app == item.app && s.item.key == item.key)
                {
                    Some(index) => index,
                    None => {
                        batch.seen.push(Seen {
                            item,
                            duration: None,
                            playing_seconds: 0,
                            max_position: None,
                            first_seen: row.at,
                            last_seen: row.at,
                        });
                        batch.seen.len() - 1
                    }
                };
                let seen = &mut batch.seen[index];
                seen.duration = seen.duration.max(row.duration.filter(|d| *d > 0));
                seen.last_seen = row.at;
                let cap = row.duration.filter(|d| *d > 0).unwrap_or(MAX_INTERVAL);
                seen.playing_seconds = seen.playing_seconds.saturating_add(elapsed.min(cap));
                if let Some(reached) = reached(row, next) {
                    let reached = reached.min(seen.duration.unwrap_or(cap));
                    seen.max_position = seen.max_position.max(Some(reached));
                }
            }
            Parsed::Other(app) if elapsed > 0 => {
                *batch.other.entry(app).or_default() += elapsed.min(MAX_INTERVAL);
            }
            Parsed::Other(_) | Parsed::Untitled(_) | Parsed::Nothing => {}
        }
    }
    if batch.open.is_none() {
        batch.open = rows.into_iter().last();
    }
    batch
}

/// How far into its episode `row` got: its position, plus the time it kept
/// playing until `next`.
fn reached(row: &PlaybackEvent, next: Option<&PlaybackEvent>) -> Option<u32> {
    let position = row.position?;
    let Some(next) = next.filter(|_| row.playing) else {
        return Some(position);
    };
    let since = row.position_at.unwrap_or(row.at);
    let played = (next.at.as_second() - since.as_second()).max(0);
    Some(position.saturating_add(u32::try_from(played).unwrap_or(u32::MAX)))
}

/// How an episode seen playing has been handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DetectionStatus {
    /// Not watched far enough yet, or waiting for TMDB.
    Watching,
    Marked,
    /// Its show matches more than one TMDB show, or the episode doesn't fit.
    NeedsConfirmation,
    /// No TMDB show has its show's name.
    NoMatch,
    /// Its show was set to be ignored.
    Ignored,
}

impl DetectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DetectionStatus::Watching => "watching",
            DetectionStatus::Marked => "marked",
            DetectionStatus::NeedsConfirmation => "needs_confirmation",
            DetectionStatus::NoMatch => "no_match",
            DetectionStatus::Ignored => "ignored",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        [
            DetectionStatus::Watching,
            DetectionStatus::Marked,
            DetectionStatus::NeedsConfirmation,
            DetectionStatus::NoMatch,
            DetectionStatus::Ignored,
        ]
        .into_iter()
        .find(|status| status.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            DetectionStatus::Watching => "Watching",
            DetectionStatus::Marked => "Marked watched",
            DetectionStatus::NeedsConfirmation => "Needs confirmation",
            DetectionStatus::NoMatch => "No TMDB match",
            DetectionStatus::Ignored => "Ignored",
        }
    }
}

/// An episode or movie seen playing, as stored.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Detection {
    pub player: String,
    pub app: App,
    pub item_key: String,
    pub kind: Kind,
    pub show_key: String,
    pub show_title: String,
    pub title: String,
    pub season: u32,
    pub episode: u32,
    pub duration: Option<u32>,
    pub playing_seconds: u32,
    pub max_position: Option<u32>,
    pub last_seen: Timestamp,
    pub status: DetectionStatus,
    /// The TMDB show or movie it was matched to.
    pub tmdb_id: Option<u64>,
    pub note: Option<String>,
}

impl Detection {
    /// Seconds of the episode watched: the time spent playing it, or the
    /// furthest position reached, whichever is more.
    pub fn progress(&self) -> u32 {
        self.playing_seconds.max(self.max_position.unwrap_or(0))
    }

    /// Share of the episode watched, 0 to 100, if its duration is known.
    pub fn percent(&self) -> Option<u32> {
        let d = self.duration.filter(|d| *d > 0)?;
        Some((u64::from(self.progress()) * 100 / u64::from(d)).min(100) as u32)
    }

    pub fn last_seen_label(&self) -> String {
        local_time(self.last_seen)
    }

    /// "Dark Matter S1 E3", or a movie's title as the app reported it.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Episode => format!("{} S{} E{}", self.show_title, self.season, self.episode),
            Kind::Movie => self.title.clone(),
        }
    }

    /// The matched episode's or movie's page.
    pub fn href(&self) -> Option<String> {
        self.tmdb_id.map(|id| match self.kind {
            Kind::Episode => format!("/tv/{id}/season/{}/episode/{}", self.season, self.episode),
            Kind::Movie => format!("/movie/{id}"),
        })
    }
}

/// A show whose episodes, or a movie, wait for the user to pick its TMDB
/// show or movie.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PendingShow {
    pub app: App,
    pub kind: Kind,
    pub show_key: String,
    pub show_title: String,
    /// Why it wasn't matched, from its latest episode.
    pub note: Option<String>,
    /// "S1 E3" for each waiting episode; empty for a movie.
    pub episodes: Vec<String>,
    /// TMDB shows or movies found by name; `None` if TMDB couldn't be searched.
    pub candidates: Option<Vec<Candidate>>,
}

/// A TMDB show or movie.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Candidate {
    pub id: u64,
    pub name: String,
    pub year: Option<String>,
    pub overview: Option<String>,
}

/// How reading Home Assistant is going.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SyncStatus {
    /// HA_URL or HA_TOKEN isn't set.
    NotConfigured,
    /// Configured, but not read yet.
    Starting,
    Ok {
        at: Timestamp,
    },
    Failed {
        at: Timestamp,
        error: String,
        last_ok: Option<Timestamp>,
    },
}

impl SyncStatus {
    pub fn kind(&self) -> &'static str {
        match self {
            SyncStatus::NotConfigured => "not_configured",
            SyncStatus::Starting => "starting",
            SyncStatus::Ok { .. } => "ok",
            SyncStatus::Failed { .. } => "failed",
        }
    }

    pub fn error(&self) -> &str {
        match self {
            SyncStatus::Failed { error, .. } => error,
            _ => "",
        }
    }

    pub fn at_label(&self) -> Option<String> {
        match self {
            SyncStatus::Ok { at } | SyncStatus::Failed { at, .. } => Some(local_time(*at)),
            _ => None,
        }
    }

    pub fn last_ok_label(&self) -> Option<String> {
        match self {
            SyncStatus::Failed {
                last_ok: Some(at), ..
            } => Some(local_time(*at)),
            _ => None,
        }
    }
}

/// The live feed from Home Assistant's WebSocket.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LiveStatus {
    /// HA_URL or HA_TOKEN isn't set.
    NotConfigured,
    /// Configured, not connected yet.
    Connecting,
    Connected {
        since: Timestamp,
    },
    /// Lost or never made since `since`; retrying.
    Disconnected {
        since: Timestamp,
        error: String,
    },
}

impl LiveStatus {
    pub fn kind(&self) -> &'static str {
        match self {
            LiveStatus::NotConfigured => "not_configured",
            LiveStatus::Connecting => "connecting",
            LiveStatus::Connected { .. } => "connected",
            LiveStatus::Disconnected { .. } => "disconnected",
        }
    }

    pub fn error(&self) -> &str {
        match self {
            LiveStatus::Disconnected { error, .. } => error,
            _ => "",
        }
    }

    pub fn since_label(&self) -> Option<String> {
        match self {
            LiveStatus::Connected { since } | LiveStatus::Disconnected { since, .. } => {
                Some(local_time(*since))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PlaybackDetail {
    pub live: LiveStatus,
    /// The last read of Home Assistant's history.
    pub status: SyncStatus,
    /// Players read, by Home Assistant entity ID.
    pub players: Vec<String>,
    pub pending: Vec<PendingShow>,
    pub recent: Vec<Detection>,
    /// Time spent playing other things since the server started, by app,
    /// most first.
    pub other: Vec<(String, u32)>,
    /// `recent` couldn't be read.
    pub unreadable: bool,
}

impl PlaybackDetail {
    pub fn other_label(&self) -> String {
        self.other
            .iter()
            .map(|(app, seconds)| format!("{app} {}", duration_label(*seconds)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// "1 h 05 min", "12 min".
pub fn duration_label(seconds: u32) -> String {
    let minutes = seconds / 60;
    if minutes >= 60 {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

/// A timestamp in local time, e.g. "Oct 11, 20:27".
fn local_time(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%b %-d, %H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_760_000_000 + second).unwrap()
    }

    fn infuse(second: i64, playing: bool, title: &str, duration: Option<u32>) -> PlaybackEvent {
        PlaybackEvent {
            at: at(second),
            playing,
            app_id: Some(INFUSE_ID.to_owned()),
            app_name: Some("Infuse".to_owned()),
            title: Some(title.to_owned()),
            content_id: None,
            duration,
            position: None,
            position_at: None,
        }
    }

    fn tv(second: i64, playing: bool, content_id: &str, duration: u32) -> PlaybackEvent {
        PlaybackEvent {
            at: at(second),
            playing,
            app_id: Some(TV_ID.to_owned()),
            app_name: Some("TV".to_owned()),
            title: Some("Widow’s Bay".to_owned()),
            content_id: Some(content_id.to_owned()),
            duration: Some(duration),
            position: None,
            position_at: None,
        }
    }

    fn other(second: i64, playing: bool, app_id: &str, app_name: &str) -> PlaybackEvent {
        PlaybackEvent {
            at: at(second),
            playing,
            app_id: Some(app_id.to_owned()),
            app_name: Some(app_name.to_owned()),
            title: Some("Something".to_owned()),
            content_id: None,
            duration: Some(600),
            position: None,
            position_at: None,
        }
    }

    fn episode(parsed: Parsed) -> (App, String, String, u32, u32) {
        let Parsed::Item(i) = parsed else {
            panic!("not an episode: {parsed:?}")
        };
        (i.app, i.key, i.show_title, i.season, i.episode)
    }

    // Titles and IDs below are from the Living Room Apple TV's history.
    #[test]
    fn infuse_titles_carry_show_season_and_episode() {
        let (app, key, show, season, number) = episode(parse(&infuse(
            0,
            true,
            "New Girl - S4 ∙ E8 - Teachers",
            None,
        )));
        assert_eq!(app, App::Infuse);
        assert_eq!(key, "New Girl - S4 ∙ E8 - Teachers");
        assert_eq!((show.as_str(), season, number), ("New Girl", 4, 8));
        let (_, _, show, season, number) = episode(parse(&infuse(
            0,
            true,
            "New Girl - S2 ∙ E25 - Elaine's Big Day",
            None,
        )));
        assert_eq!((show.as_str(), season, number), ("New Girl", 2, 25));
        let (_, _, show, ..) = episode(parse(&infuse(
            0,
            true,
            "Star Trek - Strange New Worlds - S1 ∙ E2 - Children of the Comet",
            None,
        )));
        assert_eq!(show, "Star Trek - Strange New Worlds");
    }

    #[test]
    fn other_infuse_titles_are_movies() {
        // Played on Oct 20.
        let Parsed::Item(movie) = parse(&infuse(0, true, "Back to the Future", Some(6962))) else {
            panic!("not parsed")
        };
        assert_eq!(movie.kind, Kind::Movie);
        assert_eq!(movie.key, "Back to the Future");
        assert_eq!(movie.show_key, "back to the future");
        assert_eq!((movie.season, movie.episode), (0, 0));
        let Parsed::Item(movie) = parse(&infuse(0, true, "The Thing (1982)", None)) else {
            panic!("not parsed")
        };
        assert_eq!(movie.show_title, "The Thing");
        assert_eq!(movie.key, "The Thing (1982)");
        assert_eq!(movie_title("The Thing (1982)"), ("The Thing", Some(1982)));
        assert_eq!(
            movie_title("Blade Runner 2049"),
            ("Blade Runner 2049", None)
        );
        assert_eq!(movie_title("(1982)"), ("(1982)", None));
        assert_eq!(
            movie_title("Movie (Director's Cut)"),
            ("Movie (Director's Cut)", None)
        );
    }

    #[test]
    fn tv_app_content_ids_end_in_season_and_episode() {
        let (app, key, show, season, number) = episode(parse(&tv(0, true, "A0178901003", 2461)));
        assert_eq!(app, App::Tv);
        assert_eq!(key, "A0178901003");
        assert_eq!((show.as_str(), season, number), ("Widow’s Bay", 1, 3));
        assert_eq!(content_id_episode("A0052201006"), Some((1, 6)));
        assert_eq!(content_id_episode("A0052202010"), Some((2, 10)));
        assert_eq!(content_id_episode("umc.cmc.123"), None);
        assert_eq!(content_id_episode("A00522"), None);
        assert_eq!(content_id_episode("A0052201000"), None);
        let mut idle = tv(0, true, "", 0);
        idle.title = None;
        idle.content_id = None;
        assert_eq!(parse(&idle), Parsed::Untitled(App::Tv));
    }

    #[test]
    fn other_feature_length_tv_app_videos_are_movies() {
        // Played on Oct 21.
        let mut row = tv(0, true, "PMTFY26FTR018", 7599);
        row.title = Some("Good Will Hunting".to_owned());
        let Parsed::Item(movie) = parse(&row) else {
            panic!("not parsed")
        };
        assert_eq!(movie.app, App::Tv);
        assert_eq!(movie.kind, Kind::Movie);
        assert_eq!(movie.key, "PMTFY26FTR018");
        assert_eq!(movie.show_title, "Good Will Hunting");
        assert_eq!(movie.show_key, "good will hunting");
        assert_eq!((movie.season, movie.episode), (0, 0));
        let other = Parsed::Other("TV app (not a TV episode or movie)".to_owned());
        let mut trailer = tv(0, true, "PMTFY26TRL001", 150);
        trailer.title = Some("Good Will Hunting".to_owned());
        assert_eq!(parse(&trailer), other);
        trailer.duration = None;
        assert_eq!(parse(&trailer), other);
    }

    #[test]
    fn other_apps_are_named_not_parsed() {
        assert_eq!(
            parse(&other(0, true, "com.google.ios.youtube", "YouTube")),
            Parsed::Other("YouTube".to_owned())
        );
        assert_eq!(
            parse(&other(0, true, "com.apple.TVMusic", "Music")),
            Parsed::Other("Music".to_owned())
        );
        assert_eq!(
            parse(&other(0, true, "com.espn.ScoreCenter", "ESPN")),
            Parsed::Other("ESPN".to_owned())
        );
        let off = PlaybackEvent {
            at: at(0),
            playing: false,
            app_id: None,
            app_name: None,
            title: None,
            content_id: None,
            duration: None,
            position: None,
            position_at: None,
        };
        assert_eq!(parse(&off), Parsed::Nothing);
    }

    #[test]
    fn show_keys_ignore_case_punctuation_and_apostrophe_style() {
        assert_eq!(show_key("Widow’s Bay"), "widows bay");
        assert_eq!(show_key("Widow's  Bay"), "widows bay");
        assert_eq!(show_key("Dark Matter"), "dark matter");
    }

    #[test]
    fn watched_at_ninety_percent_or_eighty_once_the_next_episode_started() {
        assert!(!is_watched(1000, 0, true));
        assert!(!is_watched(899, 1000, false));
        assert!(is_watched(900, 1000, false));
        assert!(!is_watched(799, 1000, true));
        assert!(is_watched(800, 1000, true));
    }

    #[test]
    fn movies_are_watched_at_ninety_percent_or_into_the_credits() {
        // Back to the Future, 116 min: 90% comes first.
        assert!(!movie_is_watched(6265, 6962));
        assert!(movie_is_watched(6266, 6962));
        // 90 min: the last 10 minutes start at 89%.
        assert!(!movie_is_watched(4799, 5400));
        assert!(movie_is_watched(4800, 5400));
        // A short film, all of it credits by that rule: at least 80%.
        assert!(!movie_is_watched(479, 600));
        assert!(movie_is_watched(480, 600));
        assert!(!movie_is_watched(100, 0));
    }

    #[test]
    fn runtimes_fit_with_a_few_minutes_of_slack() {
        // Widow's Bay S1 E3: 2461 s played, TMDB says 38 min.
        assert!(runtime_fits(2461, 38));
        // Dark Matter S1 E1 (48 min) against S2 E1 (54 min).
        assert!(runtime_fits(2920, 48));
        assert!(!runtime_fits(2920, 54));
        assert!(!runtime_fits(2461, 30));
    }

    #[test]
    fn playing_time_counts_until_the_next_row() {
        let batch = accumulate(
            None,
            vec![
                tv(100, false, "A0178901004", 2261),
                tv(0, true, "A0178901003", 2461),
                tv(60, true, "A0178901003", 2461),
                other(300, true, "com.google.ios.youtube", "YouTube"),
                tv(400, true, "A0178901004", 2261),
                tv(1000, false, "A0178901004", 2261),
            ],
        );
        let summary: Vec<_> = batch
            .seen
            .iter()
            .map(|s| (s.item.key.as_str(), s.playing_seconds, s.duration))
            .collect();
        assert_eq!(
            summary,
            [
                ("A0178901003", 100, Some(2461)),
                ("A0178901004", 600, Some(2261))
            ]
        );
        assert_eq!(batch.other["YouTube"], 100);
        assert_eq!(batch.seen[1].first_seen, at(100));
        assert_eq!(batch.seen[1].last_seen, at(1000));
        assert_eq!(batch.open, Some(tv(1000, false, "A0178901004", 2261)));
    }

    #[test]
    fn continuing_from_the_open_row_counts_each_stretch_once() {
        let rows = vec![
            infuse(0, true, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
            infuse(500, false, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
            infuse(600, true, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
            infuse(1300, false, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
        ];
        let whole = accumulate(None, rows.clone());
        let first = accumulate(None, rows[..3].to_vec());
        // The next read overlaps: it starts at the open row's time.
        let second = accumulate(first.open.clone(), rows[2..].to_vec());
        assert_eq!(whole.seen[0].playing_seconds, 1200);
        assert_eq!(
            first.seen[0].playing_seconds + second.seen[0].playing_seconds,
            1200
        );
        assert_eq!(second.open, whole.open);
    }

    #[test]
    fn untitled_rows_continue_the_episode_before_them() {
        let untitled = |second, playing, duration| {
            let mut e = infuse(second, playing, "", duration);
            e.title = None;
            e
        };
        // New Girl S4 E6 on Oct 19: resumed, then 16 minutes without a title.
        let e6 = "New Girl - S4 ∙ E6 - Background Check";
        let rows = vec![
            infuse(0, true, e6, Some(1283)),
            infuse(300, false, e6, Some(1283)),
            untitled(400, true, Some(1283)),
            untitled(900, false, Some(1283)),
            untitled(1000, true, None),
            infuse(1300, false, "New Girl - S4 ∙ E7 - Goldmine", None),
            untitled(1400, true, Some(999)),
            infuse(1500, false, "New Girl - S4 ∙ E7 - Goldmine", None),
        ];
        let batch = accumulate(None, rows.clone());
        assert_eq!(
            batch.seen[0].playing_seconds,
            300 + 500,
            "not the row without a duration"
        );
        assert_eq!(
            batch.seen[1].playing_seconds, 0,
            "another duration isn't E7"
        );

        // Across runs: the saved row names the episode it continues.
        let first = accumulate(None, rows[..3].to_vec());
        assert_eq!(first.open.as_ref().unwrap().title.as_deref(), Some(e6));
        let second = accumulate(first.open, rows[2..].to_vec());
        assert_eq!(second.seen[0].playing_seconds, 500);
    }

    #[test]
    fn a_stretch_counts_at_most_the_duration() {
        let batch = accumulate(
            None,
            vec![
                infuse(0, true, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
                infuse(10 * 3600, false, "New Girl - S3 ∙ E5 - The Box", Some(1303)),
            ],
        );
        assert_eq!(batch.seen[0].playing_seconds, 1303);
    }

    #[test]
    fn live_positions_reach_on_while_playing() {
        let at_position = |second: i64, playing: bool, position: u32| PlaybackEvent {
            position: Some(position),
            position_at: Some(at(second)),
            ..tv(second, playing, "A0178901003", 2461)
        };
        // Resumed at 30 min, 5 min played, a skip to 39 min, then paused.
        let batch = accumulate(
            None,
            vec![
                at_position(0, true, 1800),
                at_position(300, true, 2340),
                at_position(310, false, 2350),
            ],
        );
        assert_eq!(batch.seen[0].playing_seconds, 310);
        assert_eq!(batch.seen[0].max_position, Some(2350));

        // The open row's position runs on until the next run's first row,
        // and never past the end.
        let batch = accumulate(
            Some(at_position(0, true, 2400)),
            vec![tv(600, false, "A0178901003", 2461)],
        );
        assert_eq!(batch.seen[0].max_position, Some(2461));

        // History rows have no position.
        let batch = accumulate(
            None,
            vec![
                tv(0, true, "A0178901003", 2461),
                tv(60, false, "A0178901003", 2461),
            ],
        );
        assert_eq!(batch.seen[0].max_position, None);
    }

    #[test]
    fn the_longest_reported_duration_wins() {
        let batch = accumulate(
            None,
            vec![
                infuse(0, false, "New Girl - S2 ∙ E25 - Elaine's Big Day", None),
                infuse(
                    10,
                    true,
                    "New Girl - S2 ∙ E25 - Elaine's Big Day",
                    Some(1304),
                ),
                tv(20, true, "A0178901005", 2333),
                tv(30, true, "A0178901005", 2411),
                tv(40, false, "A0178901005", 2333),
            ],
        );
        assert_eq!(batch.seen[0].duration, Some(1304));
        assert_eq!(batch.seen[1].duration, Some(2411));
    }

    #[test]
    fn labels() {
        assert_eq!(duration_label(59), "0 min");
        assert_eq!(duration_label(12 * 60), "12 min");
        assert_eq!(duration_label(3900), "1 h 05 min");
    }
}
