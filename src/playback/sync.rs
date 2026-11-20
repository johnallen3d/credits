//! Reads Home Assistant, adds up playing time, and marks watched episodes
//! and movies.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};

use super::store::{Key, Mapping};
use super::{
    App, Candidate, Detection, DetectionStatus, HomeAssistant, Kind, Live, PlaybackEvent,
    TV_MOVIE_SECONDS, accumulate, content_id_block, is_watched, movie_is_watched, movie_title,
    runtime_fits, show_key,
};
use crate::catalog::{Catalog, CatalogError};

/// The first wait before connecting again after the live feed is lost. It
/// doubles with each failure, up to `MAX_RETRY`.
const MIN_RETRY: Duration = Duration::from_secs(5);
/// The longest wait between attempts. Each attempt also reads the history,
/// so without the live feed Home Assistant is still read this often.
const MAX_RETRY: Duration = Duration::from_secs(5 * 60);
/// TMDB shows or movies offered when asking which one something is.
const MAX_CANDIDATES: usize = 8;
/// How far back the first read goes: Home Assistant's recorder keeps 10 days.
const LOOKBACK_DAYS: i64 = 10;

/// Follows the players live, forever. Each time it connects, it first
/// catches up from the history, so nothing played while disconnected is
/// missed. Changes that arrive while catching up wait in the connection.
pub async fn run(catalog: Catalog, ha: HomeAssistant, players: Vec<String>) {
    let store = catalog.playback_store();
    let mut retry = MIN_RETRY;
    loop {
        let live = Live::connect(&ha, &players).await;
        let caught_up = sync_once(&catalog, &ha, &players).await;
        if let Err(e) = &caught_up {
            tracing::warn!(error = %e, "reading Home Assistant's history failed");
        }
        let error = match (live, caught_up) {
            // Without the history, live changes could be counted against a
            // stale read position.
            (Ok(_), Err(e)) => format!("catching up failed: {e}"),
            (Err(e), _) => e.to_string(),
            (Ok(mut live), Ok(())) => {
                store.record_live(Ok(()), Timestamp::now());
                tracing::info!("following Home Assistant live");
                retry = MIN_RETRY;
                follow(&catalog, &mut live).await
            }
        };
        tracing::warn!(
            error,
            "Home Assistant's live feed is unavailable; retrying in {retry:?}"
        );
        store.record_live(Err(error), Timestamp::now());
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(MAX_RETRY);
    }
}

/// Counts each change as it arrives, until the connection is lost.
async fn follow(catalog: &Catalog, live: &mut Live) -> String {
    loop {
        match live.next().await {
            Ok((player, event)) => {
                if let Err(e) = ingest_live(catalog, &player, event).await {
                    tracing::warn!(error = %e, "saving live playback failed");
                }
            }
            Err(e) => return e.to_string(),
        }
    }
}

/// Adds one live change to the progress, as the next row after the read
/// position, then marks what that finished.
async fn ingest_live(
    catalog: &Catalog,
    player: &str,
    event: PlaybackEvent,
) -> Result<(), sqlx::Error> {
    let store = catalog.playback_store();
    let cursor = store.cursor(player).await?;
    let batch = accumulate(cursor.open, cursor.last, vec![event]);
    store.ingest(player, &batch).await?;
    resolve_ready(catalog).await;
    Ok(())
}

/// Reads each player's history since the last read, then marks episodes
/// that are now watched. The outcome shows on the playback page.
pub async fn sync_once(
    catalog: &Catalog,
    ha: &HomeAssistant,
    players: &[String],
) -> Result<(), String> {
    let result = read_players(catalog, ha, players).await;
    // Even after a failed read, episodes already counted can be resolved.
    resolve_ready(catalog).await;
    catalog
        .playback_store()
        .record_sync(result.clone(), Timestamp::now());
    result
}

async fn read_players(
    catalog: &Catalog,
    ha: &HomeAssistant,
    players: &[String],
) -> Result<(), String> {
    let store = catalog.playback_store();
    for player in players {
        let now = Timestamp::now();
        let cursor = store
            .cursor(player)
            .await
            .map_err(|e| format!("reading the playback position failed: {e}"))?;
        let earliest = now - SignedDuration::from_hours(24 * LOOKBACK_DAYS);
        let start = cursor
            .open
            .as_ref()
            .map_or(earliest, |o| o.at.max(earliest));
        let mut events = ha
            .history(player, start, now)
            .await
            .map_err(|e| e.to_string())?;
        // The first row is the state at `start`, which may have begun earlier.
        for e in &mut events {
            e.at = e.at.max(start);
        }
        let batch = accumulate(cursor.open, cursor.last, events);
        store
            .ingest(player, &batch)
            .await
            .map_err(|e| format!("saving playback progress failed: {e}"))?;
    }
    Ok(())
}

/// What to do with an episode or movie watched far enough.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// Mark the TMDB show's season and episode (0 and 0 for a movie).
    Mark(u64, Place),
    /// What played as a TV app episode is this TMDB movie: see `movie_instead`.
    MarkMovie(u64),
    Ask(DetectionStatus, String),
    Ignore,
    /// TMDB failed; try again on the next read.
    Retry(String),
}

/// Resolves and marks every episode and movie watched far enough. Matching
/// one episode's show can settle earlier episodes that matched several
/// shows, so those get a second pass.
pub(crate) async fn resolve_ready(catalog: &Catalog) {
    let store = catalog.playback_store();
    for _ in 0..2 {
        let mut matched = Vec::new();
        resolve_pass(catalog, &mut matched).await;
        if matched.is_empty() {
            break;
        }
        for (app, kind, show_key) in &matched {
            if let Err(e) = store.reopen_show(*app, *kind, show_key).await {
                tracing::warn!(error = %e, "reopening a show's episodes failed");
            }
        }
    }
}

/// One pass over the ready episodes and movies; shows and movies matched
/// automatically are added to `matched`.
async fn resolve_pass(catalog: &Catalog, matched: &mut Vec<(App, Kind, String)>) {
    let store = catalog.playback_store();
    let ready = match store.ready().await {
        Ok(ready) => ready,
        Err(e) => {
            tracing::warn!(error = %e, "reading playback progress failed");
            return;
        }
    };
    for (mut d, next_started) in ready {
        let Some(duration) = duration(catalog, &d).await else {
            continue;
        };
        let watched = match d.kind {
            Kind::Episode => is_watched(d.progress(), duration, next_started),
            Kind::Movie => movie_is_watched(d.progress(), duration),
        };
        if !watched {
            continue;
        }
        let key = Key {
            player: &d.player,
            app: d.app,
            item_key: &d.item_key,
        };
        let mut outcome = resolve(catalog, &d, matched).await;
        if let Outcome::MarkMovie(movie_id) = outcome {
            if let Err(e) = store.set_movie(&key).await {
                tracing::warn!(error = %e, "saving that an episode is a movie failed");
                continue;
            }
            d.kind = Kind::Movie;
            (d.season, d.episode) = (0, 0);
            outcome = Outcome::Mark(movie_id, (0, 0));
        }
        let saved = match outcome {
            Outcome::MarkMovie(_) => unreachable!("made a movie mark above"),
            Outcome::Mark(tmdb_id, (season, episode)) => {
                let marked = match d.kind {
                    Kind::Episode => {
                        catalog
                            .mark_episode_from_apple_tv(tmdb_id, season, episode)
                            .await
                    }
                    Kind::Movie => catalog.mark_movie_from_apple_tv(tmdb_id).await,
                };
                if (season, episode) != (d.season, d.episode)
                    && let Err(e) = store.set_place(&key, season, episode).await
                {
                    tracing::warn!(error = %e, "saving an episode's season failed");
                }
                match marked {
                    Ok(()) => {
                        tracing::info!(tmdb_id, kind = d.kind.id(), season, episode, title = %d.show_title, "marked watched from Apple TV");
                        store
                            .set_status(&key, DetectionStatus::Marked, Some(tmdb_id), None)
                            .await
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "marking watched failed");
                        continue;
                    }
                }
            }
            Outcome::Ask(status, note) => store.set_status(&key, status, None, Some(&note)).await,
            Outcome::Ignore => {
                store
                    .set_status(&key, DetectionStatus::Ignored, None, None)
                    .await
            }
            Outcome::Retry(note) => {
                store
                    .set_status(&key, DetectionStatus::Watching, None, Some(&note))
                    .await
            }
        };
        if let Err(e) = saved {
            tracing::warn!(error = %e, "saving playback status failed");
        }
    }
}

/// The episode's or movie's length in seconds: as the player reported it,
/// or else TMDB's runtime once its show or movie is matched (or, for a
/// movie, if only one has its name). Infuse doesn't always report it.
async fn duration(catalog: &Catalog, d: &Detection) -> Option<u32> {
    if let Some(duration) = d.duration {
        return Some(duration);
    }
    let mapping = catalog
        .playback_store()
        .mapping(d.app, d.kind, &d.show_key)
        .await
        .ok()?;
    let tmdb_id = match mapping {
        Some(Mapping::Show(id)) => id,
        Some(Mapping::Ignored) => return None,
        None if d.kind == Kind::Movie => match named(catalog, d).await.ok()?.as_slice() {
            [only] => only.id,
            _ => return None,
        },
        None => return None,
    };
    if d.kind == Kind::Movie {
        return Some(catalog.movie_runtime(tmdb_id).await.ok()?? * 60);
    }
    let runtimes = catalog.episode_runtimes(tmdb_id, d.season).await.ok()?;
    let (_, runtime) = runtimes.into_iter().find(|(n, _)| *n == d.episode)?;
    Some(runtime? * 60)
}

async fn resolve(
    catalog: &Catalog,
    d: &Detection,
    matched: &mut Vec<(App, Kind, String)>,
) -> Outcome {
    let store = catalog.playback_store();
    // What has to fit: "S1 E3", or the movie's runtime.
    let what = match d.kind {
        Kind::Episode => format!("S{} E{}", d.season, d.episode),
        Kind::Movie => "the runtime that played".to_owned(),
    };
    let noun = d.kind.noun();
    match store.mapping(d.app, d.kind, &d.show_key).await {
        Err(e) => Outcome::Retry(format!("couldn't read {noun} choices: {e}")),
        Ok(Some(Mapping::Ignored)) => Outcome::Ignore,
        // The movie itself was chosen; there's nothing more to check.
        Ok(Some(Mapping::Show(movie_id))) if d.kind == Kind::Movie => {
            Outcome::Mark(movie_id, (0, 0))
        }
        Ok(Some(Mapping::Show(tv_id))) => match fit(catalog, tv_id, d).await {
            Err(e) => Outcome::Retry(retry_note(&e)),
            Ok((Fit::Yes, place)) => Outcome::Mark(tv_id, place),
            // Infuse names the episode itself; only its runtime differs.
            Ok((Fit::Runtime, place)) if d.app == App::Infuse => Outcome::Mark(tv_id, place),
            Ok((Fit::Runtime, _)) => Outcome::Ask(
                DetectionStatus::NeedsConfirmation,
                format!("{what}'s runtime on TMDB doesn't match what played"),
            ),
            Ok((Fit::Missing, _)) => Outcome::Ask(
                DetectionStatus::NeedsConfirmation,
                format!("TMDB doesn't list {what} for the chosen show"),
            ),
        },
        Ok(None) => {
            let named = match named(catalog, d).await {
                Ok(named) => named,
                Err(e) => return Outcome::Retry(retry_note(&e)),
            };
            if named.is_empty() {
                if let Some(outcome) = movie_instead(catalog, d).await {
                    return outcome;
                }
                return Outcome::Ask(
                    DetectionStatus::NoMatch,
                    format!("No TMDB {noun} is named “{}”", d.show_title),
                );
            }
            let mut fitting = Vec::new();
            for c in &named {
                match fit(catalog, c.id, d).await {
                    Ok((Fit::Yes, place)) => fitting.push((c.id, place)),
                    Ok(_) => {}
                    Err(e) => return Outcome::Retry(retry_note(&e)),
                }
            }
            match fitting.as_slice() {
                [(id, place)] => {
                    if let Err(e) = store
                        .set_mapping(
                            d.app,
                            d.kind,
                            &d.show_key,
                            Mapping::Show(*id),
                            "auto",
                            Timestamp::now(),
                        )
                        .await
                    {
                        return Outcome::Retry(format!("couldn't save the {noun} match: {e}"));
                    }
                    matched.push((d.app, d.kind, d.show_key.clone()));
                    Outcome::Mark(*id, *place)
                }
                [] => movie_instead(catalog, d).await.unwrap_or_else(|| {
                    Outcome::Ask(
                        DetectionStatus::NeedsConfirmation,
                        format!(
                            "No TMDB {noun} named “{}” has {} that fits",
                            d.show_title,
                            match d.kind {
                                Kind::Episode => format!("a {what}"),
                                Kind::Movie => "a runtime".to_owned(),
                            }
                        ),
                    )
                }),
                more => Outcome::Ask(
                    DetectionStatus::NeedsConfirmation,
                    format!(
                        "{} TMDB {noun}s named “{}” fit {what}",
                        more.len(),
                        d.show_title
                    ),
                ),
            }
        }
    }
}

/// Whether a TV app "S1 E1" no show fits is a movie: Apple gives some of
/// its films episode-style content IDs (Mayday's was `A0155401001`). It is
/// when it ran feature-length and exactly one TMDB movie by its name has a
/// runtime that fits; that movie is then remembered as the title's match.
/// `None` leaves it an episode.
async fn movie_instead(catalog: &Catalog, d: &Detection) -> Option<Outcome> {
    let feature = d.duration.is_some_and(|s| s >= TV_MOVIE_SECONDS);
    if d.app != App::Tv || d.kind != Kind::Episode || (d.season, d.episode) != (1, 1) || !feature {
        return None;
    }
    let movies = match catalog.movies_named(&d.show_title).await {
        Ok(movies) => movies,
        Err(e) => return Some(Outcome::Retry(retry_note(&e))),
    };
    let mut fitting = Vec::new();
    for m in movies.iter().filter(|m| show_key(&m.name) == d.show_key) {
        match catalog.movie_runtime(m.id).await {
            Ok(Some(runtime)) if d.duration.is_some_and(|s| runtime_fits(s, runtime)) => {
                fitting.push(m.id);
            }
            Ok(_) => {}
            Err(e) => return Some(Outcome::Retry(retry_note(&e))),
        }
    }
    let [movie_id] = fitting[..] else {
        return None;
    };
    if let Err(e) = catalog
        .playback_store()
        .set_mapping(
            d.app,
            Kind::Movie,
            &d.show_key,
            Mapping::Show(movie_id),
            "auto",
            Timestamp::now(),
        )
        .await
    {
        return Some(Outcome::Retry(format!(
            "couldn't save the movie match: {e}"
        )));
    }
    Some(Outcome::MarkMovie(movie_id))
}

/// TMDB shows or movies with exactly the name that played, and, for a
/// movie whose title gave its year, from that year.
async fn named(catalog: &Catalog, d: &Detection) -> Result<Vec<Candidate>, CatalogError> {
    let found = match d.kind {
        Kind::Episode => catalog.tv_named(&d.show_title).await?,
        Kind::Movie => catalog.movies_named(&d.show_title).await?,
    };
    let year = match d.kind {
        Kind::Movie => movie_title(&d.title).1.map(|y| y.to_string()),
        Kind::Episode => None,
    };
    Ok(found
        .into_iter()
        .filter(|c| show_key(&c.name) == d.show_key)
        .filter(|c| year.is_none() || c.year == year)
        .collect())
}

fn retry_note(e: &CatalogError) -> String {
    format!("TMDB couldn't be reached ({e}); will retry")
}

enum Fit {
    Yes,
    /// The episode exists, but its runtime doesn't match what played.
    Runtime,
    Missing,
}

/// A season and episode on TMDB.
type Place = (u32, u32);

/// A season's number and its episodes' numbers and runtimes in minutes.
type Season = (u32, Vec<(u32, Option<u32>)>);

/// Whether TMDB's show `id` has the episode, or its movie `id` is the
/// movie, with a runtime that fits what played, and where the episode is.
/// Those without a runtime on TMDB fit. A TV app episode goes where its
/// block's other episodes say, if they say (see `block_place`).
async fn fit(catalog: &Catalog, id: u64, d: &Detection) -> Result<(Fit, Place), CatalogError> {
    if d.kind == Kind::Movie {
        let fit = match (catalog.movie_runtime(id).await?, d.duration) {
            (Some(runtime), Some(duration)) if !runtime_fits(duration, runtime) => Fit::Runtime,
            _ => Fit::Yes,
        };
        return Ok((fit, (0, 0)));
    }
    if d.app == App::Tv
        && let Some(place) = block_place(catalog, id, d).await?
    {
        return Ok((Fit::Yes, place));
    }
    let runtimes = match catalog.episode_runtimes(id, d.season).await {
        Ok(runtimes) => runtimes,
        Err(CatalogError::NotFound) => return Ok((Fit::Missing, (d.season, d.episode))),
        Err(e) => return Err(e),
    };
    let fit = match runtimes.iter().find(|(n, _)| *n == d.episode) {
        None => Fit::Missing,
        Some((_, runtime)) => fits(d.duration, *runtime),
    };
    Ok((fit, (d.season, d.episode)))
}

fn fits(duration: Option<u32>, runtime: Option<u32>) -> Fit {
    match (duration, runtime) {
        (Some(duration), Some(runtime)) if !runtime_fits(duration, runtime) => Fit::Runtime,
        _ => Fit::Yes,
    }
}

/// The most seasons a block of TV app episodes is looked for in.
const MAX_BLOCK_SEASONS: u32 = 30;

/// Where a TV app episode is on TMDB's show `id` when its content ID's
/// block isn't simply its season: the block's episodes are numbered
/// straight through from some season on. Once an episode of the block has
/// been marked, that season is known; otherwise it's the one season from
/// which every episode of the block seen so far exists and fits what played.
/// `None` if the block has no single such season and nothing in it was
/// marked, or it does but this episode doesn't fit there.
async fn block_place(
    catalog: &Catalog,
    id: u64,
    d: &Detection,
) -> Result<Option<Place>, CatalogError> {
    let Some((block, first_season, number)) = content_id_block(&d.item_key) else {
        return Ok(None);
    };
    let Ok(episodes) = catalog
        .playback_store()
        .show_episodes(d.app, &d.show_key)
        .await
    else {
        return Ok(None);
    };
    // The block's episodes, by their number in it.
    let block_episodes: Vec<(u32, &Detection)> = episodes
        .iter()
        .filter_map(|e| match content_id_block(&e.item_key) {
            Some((b, _, n)) if b == block => Some((n, e)),
            _ => None,
        })
        .collect();
    let mut seasons = Vec::new();
    for season in first_season.max(1)..first_season.max(1) + MAX_BLOCK_SEASONS {
        match catalog.episode_runtimes(id, season).await {
            Ok(runtimes) if !runtimes.is_empty() => seasons.push((season, runtimes)),
            Ok(_) | Err(CatalogError::NotFound) => break,
            Err(e) => return Err(e),
        }
    }
    let starts = seasons.iter().map(|(s, _)| *s);
    let marked: Vec<u32> = starts
        .clone()
        .filter(|&start| {
            block_episodes.iter().any(|(n, e)| {
                e.status == DetectionStatus::Marked
                    && e.tmdb_id == Some(id)
                    && locate(&seasons, start, *n).map(|(place, _)| place)
                        == Some((e.season, e.episode))
            })
        })
        .collect();
    let start = match marked.as_slice() {
        [start] => *start,
        [] => {
            let fitting: Vec<u32> = starts
                .filter(|&start| {
                    block_episodes.iter().all(|(n, e)| {
                        locate(&seasons, start, *n)
                            .is_some_and(|(_, r)| matches!(fits(e.duration, r), Fit::Yes))
                    })
                })
                .collect();
            match fitting.as_slice() {
                [start] => *start,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    Ok(locate(&seasons, start, number)
        .filter(|(_, r)| matches!(fits(d.duration, *r), Fit::Yes))
        .map(|(place, _)| place))
}

/// The `number`th episode counting straight through `seasons` from `start`,
/// with its runtime.
fn locate(seasons: &[Season], start: u32, number: u32) -> Option<(Place, Option<u32>)> {
    let mut n = number;
    for (season, runtimes) in seasons.iter().filter(|(s, _)| *s >= start) {
        let count = runtimes.len() as u32;
        if n <= count {
            let (episode, runtime) = runtimes.iter().find(|(e, _)| *e == n)?;
            return Some(((*season, *episode), *runtime));
        }
        n -= count;
    }
    None
}

/// Records which TMDB show or movie (or none) an app's show or movie is,
/// then resolves what's waiting again.
pub(crate) async fn decide(
    catalog: &Catalog,
    app: App,
    kind: Kind,
    show_key: &str,
    tmdb_id: Option<u64>,
) -> Result<(), sqlx::Error> {
    let store = catalog.playback_store();
    let mapping = tmdb_id.map_or(Mapping::Ignored, Mapping::Show);
    store
        .set_mapping(app, kind, show_key, mapping, "user", Timestamp::now())
        .await?;
    store.reopen_show(app, kind, show_key).await?;
    resolve_ready(catalog).await;
    Ok(())
}

/// TMDB shows or movies that might be `show_title`, exact name matches
/// first.
pub(crate) async fn candidates(
    catalog: &Catalog,
    kind: Kind,
    show_title: &str,
) -> Option<Vec<Candidate>> {
    let found = match kind {
        Kind::Episode => catalog.tv_named(show_title).await,
        Kind::Movie => catalog.movies_named(show_title).await,
    };
    let mut found = found
        .inspect_err(|e| tracing::warn!(error = %e, kind = kind.id(), "searching TMDB failed"))
        .ok()?;
    let key = show_key(show_title);
    found.sort_by_key(|c| show_key(&c.name) != key);
    found.truncate(MAX_CANDIDATES);
    Some(found)
}
