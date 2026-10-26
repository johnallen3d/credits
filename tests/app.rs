//! End-to-end tests against local fake TMDB and OMDb servers.

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, Request, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use credits::{Catalog, Config, app, cache::IN_MEMORY, tui};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use http_body_util::BodyExt;
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tower::ServiceExt;

const TOKEN: &str = "test.token.value";
const OMDB_KEY: &str = "omdb-key";

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer test.token.value")
}

#[derive(Clone, Default)]
struct FakeTmdb {
    calls: Arc<AtomicUsize>,
    now_playing_calls: Arc<AtomicUsize>,
    upcoming_calls: Arc<AtomicUsize>,
    down: Arc<AtomicBool>,
    /// Fails every movie details request, but not the lists.
    details_down: Arc<AtomicBool>,
}

impl FakeTmdb {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn now_playing_calls(&self) -> usize {
        self.now_playing_calls.load(Ordering::SeqCst)
    }

    fn upcoming_calls(&self) -> usize {
        self.upcoming_calls.load(Ordering::SeqCst)
    }

    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    fn set_details_down(&self, down: bool) {
        self.details_down.store(down, Ordering::SeqCst);
    }
}

async fn fake_tmdb(state: FakeTmdb) -> String {
    let counted = state.clone();
    let fake = Router::new()
        .route(
            "/3/search/multi",
            get(
                |headers: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    if !authorized(&headers) {
                        return (StatusCode::UNAUTHORIZED, Json(json!({})));
                    }
                    match q.get("query").map(String::as_str) {
                        Some("boom") => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({}))),
                        Some("nothing") => (StatusCode::OK, Json(json!({ "results": [] }))),
                        Some("theaters") => (
                            StatusCode::OK,
                            Json(json!({ "results": theaters_hits() })),
                        ),
                        _ => (
                            StatusCode::OK,
                            Json(json!({ "results": [
                                { "media_type": "movie", "id": 603, "title": "The Matrix",
                                  "release_date": "1999-03-31", "overview": "A hacker learns the truth.",
                                  "poster_path": "/matrix.jpg" },
                                { "media_type": "person", "id": 6384, "name": "Keanu Reeves",
                                  "known_for_department": "Acting", "profile_path": "/keanu.jpg",
                                  "known_for": [
                                    { "media_type": "movie", "id": 603, "title": "The Matrix",
                                      "release_date": "1999-03-31" },
                                    { "media_type": "tv", "id": 9999, "name": "Swedish Dicks",
                                      "first_air_date": "" }
                                  ] },
                                { "media_type": "person", "id": 7777, "name": "Unknown Extra",
                                  "known_for_department": "", "profile_path": null },
                                { "media_type": "collection", "id": 2344, "name": "The Matrix Collection" },
                                { "media_type": "tv", "id": 1396, "name": "Breaking Bad",
                                  "first_air_date": "2008-01-20", "overview": "",
                                  "poster_path": null }
                            ]})),
                        ),
                    }
                },
            ),
        )
        .route(
            "/3/search/tv",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let results = match q.get("query").map(String::as_str) {
                    Some("Breaking Bad") => json!([
                        { "id": 1396, "name": "Breaking Bad", "first_air_date": "2008-01-20",
                          "overview": "A teacher turns to crime." }
                    ]),
                    Some("Dark Matter") => json!([
                        { "id": 196322, "name": "Dark Matter", "first_air_date": "2024-05-07",
                          "overview": "A physicist is abducted into an alternate version of his life." },
                        { "id": 62425, "name": "Dark Matter", "first_air_date": "2015-06-12",
                          "overview": "A spaceship crew wakes with no memory." },
                        { "id": 334042, "name": "Dark Matter: The Official Podcast",
                          "first_air_date": "2026-08-27" }
                    ]),
                    _ => json!([]),
                };
                Json(json!({ "results": results }))
            }),
        )
        .route(
            "/3/search/movie",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                let results = match q.get("query").map(String::as_str) {
                    Some("Back to the Future") => json!([
                        { "id": 105, "title": "Back to the Future", "release_date": "1985-07-03",
                          "overview": "Marty McFly is sent back in time." },
                        { "id": 165, "title": "Back to the Future Part II",
                          "release_date": "1989-11-22" }
                    ]),
                    Some("The Thing") => json!([
                        { "id": 1091, "title": "The Thing", "release_date": "1982-06-25",
                          "overview": "Researchers in Antarctica meet a shape-shifter." },
                        { "id": 60935, "title": "The Thing", "release_date": "2011-10-12" }
                    ]),
                    Some("Good Will Hunting") => json!([
                        { "id": 489, "title": "Good Will Hunting", "release_date": "1997-12-05" }
                    ]),
                    _ => json!([]),
                };
                Json(json!({ "results": results }))
            }),
        )
        .route(
            "/3/movie/now_playing",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("region").map(String::as_str) != Some("US") {
                    return Json(json!({ "results": [], "total_pages": 1 }));
                }
                // 7002 is still listed long after opening; 7003 is listed
                // but already out on digital; 7100 is listed before it opens.
                let ids: &[u64] = match q.get("page").map(String::as_str) {
                    Some("1") => &[7003, 7001, 7100],
                    _ => &[7002, 7001],
                };
                let results: Vec<Value> = ids
                    .iter()
                    .filter_map(|&id| in_theaters_movie(id).or_else(|| upcoming_movie(id)))
                    .map(|m| json!({ "id": m["id"], "title": m["title"],
                                     "release_date": m["release_date"] }))
                    .collect();
                Json(json!({ "results": results, "total_pages": 2 }))
            }),
        )
        .route(
            "/3/movie/upcoming",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("region").map(String::as_str) != Some("US") {
                    return Json(json!({ "results": [], "total_pages": 1 }));
                }
                // Most popular first, which is latest opening first here;
                // 7131 has no date at all.
                let ids: Vec<u64> = match q.get("page").map(String::as_str) {
                    Some("2") => (7100..=7109).rev().collect(),
                    _ => (7110..=7124).rev().chain([7130, 7131, 7132]).collect(),
                };
                let results: Vec<Value> = ids
                    .into_iter()
                    .map(|id| match upcoming_movie(id) {
                        Some(m) => json!({ "id": id, "title": m["title"],
                                           "release_date": m["release_date"] }),
                        None => json!({ "id": id, "title": "Undated Film" }),
                    })
                    .collect();
                Json(json!({ "results": results, "total_pages": 2 }))
            }),
        )
        .route(
            "/3/movie/{id}",
            get(|Path(id): Path<u64>| async move {
                if let Some(movie) = in_theaters_movie(id).or_else(|| upcoming_movie(id)) {
                    return (StatusCode::OK, Json(movie));
                }
                // Movies played on the Apple TV: only what matching needs.
                let played = match id {
                    105 => Some(("Back to the Future", "1985-07-03", 116)),
                    1091 => Some(("The Thing", "1982-06-25", 109)),
                    60935 => Some(("The Thing", "2011-10-12", 103)),
                    489 => Some(("Good Will Hunting", "1997-12-05", 127)),
                    _ => None,
                };
                if let Some((title, release_date, runtime)) = played {
                    return (
                        StatusCode::OK,
                        Json(json!({ "id": id, "title": title, "release_date": release_date,
                                     "runtime": runtime, "external_ids": {},
                                     "watch/providers": { "results": {} },
                                     "release_dates": { "results": [] } })),
                    );
                }
                let imdb_id = match id {
                    603 => "tt0133093",
                    // Listed on OMDb with an IMDb score but no Rotten Tomatoes score.
                    6060 => "tt6060000",
                    // Unknown to OMDb, which answers "Error getting data.".
                    4040 => "tt4040000",
                    _ => return (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 }))),
                };
                let watch_providers = if id == 603 {
                    json!({ "results": {
                        "US": {
                            "link": "https://www.themoviedb.org/movie/603-the-matrix/watch?locale=US",
                            "flatrate": [
                                { "provider_id": 8, "provider_name": "Netflix",
                                  "logo_path": "/netflix.jpg", "display_priority": 4 },
                                { "provider_id": 1899, "provider_name": "Max",
                                  "logo_path": "/max.jpg", "display_priority": 1 },
                                { "provider_id": 8, "provider_name": "Netflix",
                                  "logo_path": "/netflix.jpg", "display_priority": 4 }
                            ],
                            "rent": [
                                { "provider_id": 2, "provider_name": "Apple TV",
                                  "logo_path": null, "display_priority": 3 }
                            ]
                        },
                        "GB": {
                            "link": "https://www.themoviedb.org/movie/603-the-matrix/watch?locale=GB",
                            "buy": [
                                { "provider_id": 10, "provider_name": "Amazon Video",
                                  "logo_path": "/amazon.jpg", "display_priority": 7 }
                            ]
                        }
                    }})
                } else {
                    // Offered elsewhere, but not in the default region.
                    json!({ "results": { "GB": { "link": "https://www.themoviedb.org/movie/6060/watch?locale=GB",
                        "flatrate": [{ "provider_id": 8, "provider_name": "Netflix",
                                       "logo_path": "/netflix.jpg", "display_priority": 4 }] } } })
                };
                let release_dates = match id {
                    603 => json!({ "results": [
                        { "iso_3166_1": "US", "release_dates": [
                            { "type": 1, "release_date": "1999-03-24T00:00:00.000Z", "certification": "" },
                            { "type": 3, "release_date": "1999-03-31T00:00:00.000Z", "certification": "R" },
                            { "type": 4, "release_date": "1999-09-21T00:00:00.000Z", "certification": "R" },
                            { "type": 5, "release_date": "1999-09-21T00:00:00.000Z", "certification": "" }
                        ] },
                        { "iso_3166_1": "GB", "release_dates": [
                            { "type": 3, "release_date": "1999-06-11T00:00:00.000Z", "certification": "15" }
                        ] }
                    ] }),
                    // Not out yet in the default region.
                    4040 => json!({ "results": [
                        { "iso_3166_1": "US", "release_dates": [
                            { "type": 3, "release_date": "2099-10-16T00:00:00.000Z", "certification": "" }
                        ] }
                    ] }),
                    // Dated elsewhere only.
                    _ => json!({ "results": [
                        { "iso_3166_1": "GB", "release_dates": [
                            { "type": 3, "release_date": "1999-06-11T00:00:00.000Z", "certification": "15" }
                        ] }
                    ] }),
                };
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": id, "title": "The Matrix", "original_title": "The Matrix",
                        "tagline": "Welcome to the Real World.", "overview": "A hacker learns the truth.",
                        "release_date": "1999-03-31", "status": "Released", "runtime": 136,
                        "genres": [{ "id": 28, "name": "Action" }, { "id": 878, "name": "Science Fiction" }],
                        "poster_path": "/matrix.jpg", "backdrop_path": null,
                        "external_ids": { "imdb_id": imdb_id },
                        "watch/providers": watch_providers,
                        "release_dates": release_dates,
                        "credits": {
                            "cast": [
                                { "id": 6384, "name": "Keanu Reeves", "character": "Neo",
                                  "profile_path": "/keanu.jpg" },
                                { "id": 2975, "name": "Laurence Fishburne", "character": "Morpheus",
                                  "profile_path": null },
                                { "id": 6384, "name": "Keanu Reeves", "character": "Thomas Anderson",
                                  "profile_path": "/keanu.jpg" }
                            ],
                            "crew": [
                                { "id": 9340, "name": "Lana Wachowski", "department": "Directing",
                                  "job": "Director", "profile_path": null },
                                { "id": 9340, "name": "Lana Wachowski", "department": "Writing",
                                  "job": "Writer", "profile_path": null },
                                { "id": 9339, "name": "Lilly Wachowski", "department": "Directing",
                                  "job": "Director", "profile_path": null },
                                { "id": 1091, "name": "Joel Silver", "department": "Production",
                                  "job": "Producer", "profile_path": null }
                            ]
                        }
                    })),
                )
            }),
        )
        .route(
            "/3/tv/{id}",
            get(|Path(id): Path<u64>| async move {
                if id == 500 {
                    return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
                }
                // Announced, not aired yet.
                if id == 9100 {
                    return (
                        StatusCode::OK,
                        Json(json!({
                            "id": id, "name": "Unaired Show", "first_air_date": "",
                            "status": "In Production", "seasons": [],
                            "watch/providers": { "results": {} }
                        })),
                    );
                }
                let imdb_id = match id {
                    1396 => json!("tt0903747"),
                    // OMDb answers with its quota error for this one.
                    5150 => json!("tt5150000"),
                    _ => Value::Null,
                };
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": id, "name": "Breaking Bad", "original_name": "Breaking Bad",
                        "tagline": "", "overview": "A teacher turns to crime.",
                        "first_air_date": "2008-01-20", "status": "Ended",
                        "number_of_seasons": 5, "number_of_episodes": 62,
                        "genres": [{ "id": 18, "name": "Drama" }],
                        "poster_path": null, "backdrop_path": "/bb-back.jpg",
                        "external_ids": { "imdb_id": imdb_id },
                        "watch/providers": { "results": { "US": {
                            "link": "https://www.themoviedb.org/tv/1396-breaking-bad/watch?locale=US",
                            "ads": [{ "provider_id": 300, "provider_name": "Pluto TV",
                                      "logo_path": "/pluto.jpg", "display_priority": 9 }]
                        } } },
                        "created_by": [{ "id": 66633, "name": "Vince Gilligan", "profile_path": null }],
                        "seasons": [
                            { "season_number": 0, "name": "Specials", "episode_count": 9,
                              "air_date": "2009-02-17", "poster_path": null },
                            { "season_number": 1, "name": "Season 1", "episode_count": 7,
                              "air_date": "2008-01-20", "poster_path": "/s1.jpg" },
                            { "season_number": 2, "name": "", "episode_count": 13,
                              "air_date": null, "poster_path": null }
                        ],
                        "aggregate_credits": {
                            "cast": [
                                { "id": 17419, "name": "Bryan Cranston", "profile_path": "/bryan.jpg",
                                  "roles": [{ "character": "Walter White", "episode_count": 62 }],
                                  "total_episode_count": 62 }
                            ],
                            "crew": [
                                { "id": 66633, "name": "Vince Gilligan", "profile_path": null,
                                  "department": "Directing",
                                  "jobs": [{ "job": "Director", "episode_count": 5 }],
                                  "total_episode_count": 5 },
                                { "id": 29924, "name": "Michelle MacLaren", "profile_path": null,
                                  "department": "Directing",
                                  "jobs": [{ "job": "Director", "episode_count": 11 }],
                                  "total_episode_count": 11 }
                            ]
                        }
                    })),
                )
            }),
        )
        .route(
            "/3/watch/providers/{kind}",
            get(
                |Path(kind): Path<String>, Query(q): Query<HashMap<String, String>>| async move {
                    if q.get("watch_region").map(String::as_str) != Some("US") {
                        return (StatusCode::OK, Json(json!({ "results": [] })));
                    }
                    let mut results = vec![
                        json!({ "provider_id": 8, "provider_name": "Netflix", "logo_path": "/netflix.jpg",
                                "display_priority": 5, "display_priorities": { "US": 0, "GB": 3 } }),
                        json!({ "provider_id": 1899, "provider_name": "Max", "logo_path": "/max.jpg",
                                "display_priority": 2, "display_priorities": { "US": 4 } }),
                        json!({ "provider_id": 2, "provider_name": "Apple TV", "logo_path": null,
                                "display_priority": 9 }),
                        json!({ "provider_id": 204, "provider_name": "Shudder Amazon Channel",
                                "logo_path": null, "display_priorities": { "US": 3 } }),
                    ];
                    if kind == "tv" {
                        results.push(json!({ "provider_id": 300, "provider_name": "Pluto TV",
                            "logo_path": "/pluto.jpg", "display_priorities": { "US": 6 } }));
                        // Listed in both, ranked higher for TV.
                        results.push(json!({ "provider_id": 1899, "provider_name": "Max",
                            "logo_path": "/max.jpg", "display_priorities": { "US": 1 } }));
                    }
                    (StatusCode::OK, Json(json!({ "results": results })))
                },
            ),
        )
        .route(
            "/3/tv/{id}/season/{number}",
            get(|Path((id, number)): Path<(u64, u32)>| async move {
                // Dark Matter (2024) and (2015): both have a first episode of about 48 min.
                let runtime = match (id, number) {
                    (196322, 1) => Some(48),
                    (62425, 1) => Some(44),
                    _ => None,
                };
                if let Some(runtime) = runtime {
                    return (
                        StatusCode::OK,
                        Json(json!({
                            "season_number": 1, "name": "Season 1", "episodes": [
                                { "episode_number": 1, "name": "One", "runtime": runtime },
                                { "episode_number": 2, "name": "Two",
                                  "runtime": if id == 196322 { 58 } else { 30 } }
                            ]
                        })),
                    );
                }
                if id != 1396 || number > 2 {
                    return (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 })));
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "season_number": number, "name": "Season 1",
                        "overview": "", "air_date": "2008-01-20", "poster_path": null,
                        "episodes": [
                            { "episode_number": 1, "name": "Pilot", "air_date": "2008-01-20",
                              "overview": "A chemistry teacher is diagnosed.", "runtime": 58,
                              "still_path": "/pilot.jpg" },
                            { "episode_number": 2, "name": "", "air_date": null,
                              "overview": "", "runtime": null, "still_path": null }
                        ]
                    })),
                )
            }),
        )
        .route(
            "/3/tv/{id}/season/{season}/episode/{number}",
            get(|Path((id, season, number)): Path<(u64, u32, u32)>| async move {
                if (id, season, number) != (1396, 1, 1) {
                    return (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 })));
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "season_number": 1, "episode_number": 1, "name": "Pilot",
                        "air_date": "2008-01-20", "runtime": 58, "still_path": "/pilot.jpg",
                        "overview": "A chemistry teacher is diagnosed.",
                        "credits": {
                            "cast": [
                                { "id": 17419, "name": "Bryan Cranston", "character": "Walter White",
                                  "profile_path": "/bryan.jpg" }
                            ],
                            "guest_stars": [
                                { "id": 61535, "name": "Steven Michael Quezada",
                                  "character": "Steven Gomez", "profile_path": null },
                                { "id": 61535, "name": "Steven Michael Quezada",
                                  "character": "Gomie", "profile_path": null }
                            ],
                            "crew": [
                                { "id": 66633, "name": "Vince Gilligan", "department": "Directing",
                                  "job": "Director" },
                                { "id": 66633, "name": "Vince Gilligan", "department": "Writing",
                                  "job": "Writer" },
                                { "id": 5, "name": "Lynne Willingham", "department": "Editing",
                                  "job": "Editor" }
                            ]
                        }
                    })),
                )
            }),
        )
        .route(
            "/3/person/{id}",
            get(|Path(id): Path<u64>| async move {
                match id {
                    6384 => (
                        StatusCode::OK,
                        Json(json!({
                            "id": 6384, "name": "Keanu Reeves",
                            "biography": "Keanu Charles Reeves is an actor.\n\nHe was born in Beirut.",
                            "birthday": "1964-09-02", "deathday": null,
                            "place_of_birth": "Beirut, Lebanon",
                            "known_for_department": "Acting", "profile_path": "/keanu.jpg",
                            "combined_credits": {
                                "cast": [
                                    { "id": 603, "media_type": "movie", "title": "The Matrix",
                                      "release_date": "1999-03-31", "character": "Neo",
                                      "poster_path": "/matrix.jpg", "vote_count": 26000 },
                                    { "id": 603, "media_type": "movie", "title": "The Matrix",
                                      "release_date": "1999-03-31", "character": "Thomas Anderson",
                                      "poster_path": "/matrix.jpg", "vote_count": 26000 },
                                    { "id": 999001, "media_type": "movie", "title": "Untitled Project",
                                      "release_date": "", "character": null, "poster_path": null,
                                      "vote_count": 0 },
                                    { "id": 1396, "media_type": "tv", "name": "Breaking Bad",
                                      "first_air_date": "2008-01-20", "character": "Himself",
                                      "episode_count": 1, "vote_count": 16000 },
                                    { "id": 59941, "media_type": "tv", "name": "The Tonight Show",
                                      "first_air_date": "2014-02-17", "character": "Guest",
                                      "episode_count": 3, "vote_count": 30000, "genre_ids": [10767, 35] }
                                ],
                                "crew": [
                                    { "id": 777, "media_type": "movie", "title": "Man of Tai Chi",
                                      "release_date": "2013-07-05", "department": "Directing",
                                      "job": "Director", "vote_count": 500 }
                                ]
                            }
                        })),
                    ),
                    // A sparse profile: no biography, photo or credits.
                    1 => (
                        StatusCode::OK,
                        Json(json!({
                            "id": 1, "name": "Nobody", "biography": "", "birthday": null,
                            "deathday": null, "place_of_birth": null,
                            "known_for_department": null, "profile_path": null,
                            "combined_credits": { "cast": [], "crew": [] }
                        })),
                    ),
                    _ => (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 }))),
                }
            }),
        )
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let state = counted.clone();
                async move {
                    state.calls.fetch_add(1, Ordering::SeqCst);
                    if request.uri().path().ends_with("/now_playing") {
                        state.now_playing_calls.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.uri().path().ends_with("/upcoming") {
                        state.upcoming_calls.fetch_add(1, Ordering::SeqCst);
                    }
                    if state.down.load(Ordering::SeqCst) {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    let details = request
                        .uri()
                        .path()
                        .strip_prefix("/3/movie/")
                        .is_some_and(|id| id.bytes().all(|b| b.is_ascii_digit()));
                    if details && state.details_down.load(Ordering::SeqCst) {
                        return StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    next.run(request).await
                }
            },
        ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    format!("http://{addr}/3")
}

/// Movies dated relative to today, for telling what's in theaters: 7001
/// opened 10 days ago, 7002 opened 100 days ago but is still now playing,
/// 7003 is now playing but out on digital since yesterday, and 7004 opened 60
/// days ago and is gone.
fn in_theaters_movie(id: u64) -> Option<Value> {
    let today = jiff::Zoned::now().date();
    let ago = |days: i64| {
        format!(
            "{}T00:00:00.000Z",
            today.checked_sub(jiff::Span::new().days(days)).unwrap()
        )
    };
    let (title, theatrical, digital) = match id {
        7001 => ("Opening Night", 10, None),
        7002 => ("Long Run", 100, None),
        7003 => ("Gone Digital", 10, Some(1)),
        7004 => ("Closed Early", 60, None),
        _ => return None,
    };
    let mut dates =
        vec![json!({ "type": 3, "release_date": ago(theatrical), "certification": "PG" })];
    if let Some(days) = digital {
        dates.push(json!({ "type": 4, "release_date": ago(days), "certification": "" }));
    }
    Some(json!({
        "id": id, "title": title, "release_date": &ago(theatrical)[..10], "runtime": 100,
        "external_ids": {}, "watch/providers": { "results": {} },
        "release_dates": { "results": [{ "iso_3166_1": "US", "release_dates": dates }] }
    }))
}

/// Movies TMDB lists as upcoming: 7100–7124 ("Premiere 01"–"Premiere 25")
/// open in US theaters 1–25 days from now, 7100 with Rotten Tomatoes and
/// IMDb scores and 7121 with IMDb only; 7130 opens only in Canada; 7132
/// opened in the US three days ago.
fn upcoming_movie(id: u64) -> Option<Value> {
    let today = jiff::Zoned::now().date();
    let on = |days: i64| today.checked_add(jiff::Span::new().days(days)).unwrap();
    let (title, region, date) = match id {
        7100..=7124 => (
            format!("Premiere {:02}", id - 7099),
            "US",
            on(id as i64 - 7099),
        ),
        7130 => ("Canada First".to_owned(), "CA", on(5)),
        7132 => ("Already Out".to_owned(), "US", on(-3)),
        // Opened 20 days ago; out on digital in 12 days.
        7133 => ("Digital Next".to_owned(), "US", on(-20)),
        _ => return None,
    };
    let mut dates = vec![json!({
        "type": 3, "release_date": format!("{date}T00:00:00.000Z"), "certification": ""
    })];
    if id == 7133 {
        dates.push(json!({
            "type": 4, "release_date": format!("{}T00:00:00.000Z", on(12)), "certification": ""
        }));
    }
    let imdb_id = match id {
        7100 => json!("tt0133093"),
        7121 => json!("tt6060000"),
        _ => Value::Null,
    };
    Some(json!({
        "id": id, "title": title, "release_date": date.to_string(), "runtime": 100,
        "external_ids": { "imdb_id": imdb_id }, "watch/providers": { "results": {} },
        "release_dates": { "results": [{ "iso_3166_1": region, "release_dates": dates }] }
    }))
}

/// Search hits for "theaters": the dated movies, then The Matrix.
fn theaters_hits() -> Vec<Value> {
    [7001, 7002, 7003, 7004]
        .into_iter()
        .filter_map(in_theaters_movie)
        .map(|m| {
            json!({ "media_type": "movie", "id": m["id"], "title": m["title"],
                         "release_date": m["release_date"] })
        })
        .chain([
            json!({ "media_type": "movie", "id": 603, "title": "The Matrix",
                        "release_date": "1999-03-31" }),
        ])
        .collect()
}

#[derive(Clone, Default)]
struct FakeOmdb {
    calls: Arc<AtomicUsize>,
    down: Arc<AtomicBool>,
}

impl FakeOmdb {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
    }

    async fn start(&self) -> String {
        let state = self.clone();
        let fake = Router::new().route(
            "/",
            get(move |Query(q): Query<HashMap<String, String>>| {
                let state = state.clone();
                async move {
                    state.calls.fetch_add(1, Ordering::SeqCst);
                    if state.down.load(Ordering::SeqCst) {
                        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
                    }
                    if q.get("apikey").map(String::as_str) != Some(OMDB_KEY) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({ "Response": "False", "Error": "Invalid API key!" })),
                        );
                    }
                    if let Some(season) = q.get("Season") {
                        let body = match (q.get("i").map(String::as_str), season.as_str()) {
                            (Some("tt0903747"), "1") => json!({
                                "Response": "True", "Title": "Breaking Bad", "Season": "1",
                                "Episodes": [
                                    { "Title": "Pilot", "Episode": "1", "imdbRating": "9.0", "imdbID": "tt0959621" },
                                    { "Title": "Episode 2", "Episode": "2", "imdbRating": "N/A", "imdbID": "tt1054724" }
                                ]
                            }),
                            _ => json!({ "Response": "False", "Error": "Series or season not found!" }),
                        };
                        return (StatusCode::OK, Json(body));
                    }
                    let body = match q.get("i").map(String::as_str) {
                        Some("tt0133093") => json!({
                            "Response": "True", "imdbID": "tt0133093",
                            "imdbRating": "8.7", "imdbVotes": "2,123,456",
                            "Ratings": [
                                { "Source": "Internet Movie Database", "Value": "8.7/10" },
                                { "Source": "Rotten Tomatoes", "Value": "83%" },
                                { "Source": "Metacritic", "Value": "73/100" }
                            ]
                        }),
                        Some("tt0903747") => json!({
                            "Response": "True", "imdbID": "tt0903747",
                            "imdbRating": "9.5", "imdbVotes": "2,300,000",
                            "Ratings": [{ "Source": "Internet Movie Database", "Value": "9.5/10" }]
                        }),
                        Some("tt6060000") => json!({
                            "Response": "True", "imdbID": "tt6060000",
                            "imdbRating": "6.1", "imdbVotes": "N/A", "Ratings": []
                        }),
                        Some("tt5150000") => {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({ "Response": "False", "Error": "Request limit reached!" })),
                            );
                        }
                        Some("tt4040000") => {
                            json!({ "Response": "False", "Error": "Error getting data." })
                        }
                        _ => json!({ "Response": "False", "Error": "Incorrect IMDb ID." }),
                    };
                    (StatusCode::OK, Json(body))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        format!("http://{addr}")
    }
}

struct TestApp {
    router: Router,
    catalog: Catalog,
    omdb: FakeOmdb,
    tmdb: FakeTmdb,
}

impl TestApp {
    fn app(&self) -> Router {
        self.router.clone()
    }
}

async fn build(token: &str, omdb_key: Option<&str>, ratings_ttl: Duration) -> TestApp {
    build_with(token, omdb_key, ratings_ttl, Duration::from_secs(3600)).await
}

async fn build_with(
    token: &str,
    omdb_key: Option<&str>,
    ratings_ttl: Duration,
    metadata_ttl: Duration,
) -> TestApp {
    build_with_database(token, omdb_key, ratings_ttl, metadata_ttl, IN_MEMORY).await
}

async fn build_with_database(
    token: &str,
    omdb_key: Option<&str>,
    ratings_ttl: Duration,
    metadata_ttl: Duration,
    database: &str,
) -> TestApp {
    let config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(token.to_owned()),
        "OMDB_API_KEY" => omdb_key.map(str::to_owned),
        "CREDITS_DB" => Some(database.to_owned()),
        _ => None,
    })
    .unwrap();
    build_from(config, ratings_ttl, metadata_ttl).await
}

async fn build_from(mut config: Config, ratings_ttl: Duration, metadata_ttl: Duration) -> TestApp {
    let omdb = FakeOmdb::default();
    let tmdb = FakeTmdb::default();
    config.tmdb_api_base = fake_tmdb(tmdb.clone()).await;
    config.tmdb_image_base = "https://img.test/t/p".to_owned();
    config.omdb_api_base = omdb.start().await;
    config.ratings_ttl = ratings_ttl;
    config.metadata_ttl = metadata_ttl;
    let catalog = Catalog::new(&config).await.unwrap();
    TestApp {
        router: app(catalog.clone()),
        catalog,
        omdb,
        tmdb,
    }
}

async fn test_app_with_token(token: &str) -> Router {
    build(token, Some(OMDB_KEY), Duration::from_secs(3600))
        .await
        .router
}

async fn test_app() -> Router {
    test_app_with_token(TOKEN).await
}

async fn get_page(app: Router, uri: &str, htmx: bool) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::get(uri);
    if htmx {
        request = request.header("HX-Request", "true");
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

async fn post_form(app: Router, uri: &str, form: &str) -> (StatusCode, HeaderMap) {
    let request = Request::post(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form.to_owned()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    (response.status(), response.headers().clone())
}

async fn post_htmx_form(app: Router, uri: &str, form: &str) -> (StatusCode, String) {
    let request = Request::post(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("HX-Request", "true")
        .body(Body::from(form.to_owned()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn get_json(app: Router, uri: &str) -> (StatusCode, Value) {
    let (status, _, body) = get_page(app, uri, false).await;
    (status, serde_json::from_str(&body).unwrap())
}

#[tokio::test]
async fn home_page_renders_search_form_and_footer_nav() {
    let (status, _, body) = get_page(test_app().await, "/", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<form class="search" action="/" method="get""#));
    assert!(body.contains("Search TMDB for a movie, TV show or person."));
    assert!(body.contains(r#"<a class="brand" href="/"><img src="/static/favicon.svg" alt="credits" width="32" height="32"></a>"#));
    assert!(
        !body.contains(r#"<a href="/">Search</a>"#),
        "the logo links home"
    );
    let footer = &body[body.find(r#"<footer class="site">"#).expect("site footer")..];
    assert!(footer.contains(r#"<nav class="site-nav" aria-label="Site">"#));
    assert!(footer.contains(r#"<a href="/about">About</a>"#));
    assert!(
        !body.contains("not endorsed or certified"),
        "attribution lives on /about"
    );
}

#[tokio::test]
async fn home_page_lists_recently_viewed_titles_and_people() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, _, body) = get_page(t.app(), "/", false).await;
    assert!(body.contains("Pages you open will be listed here."));
    assert!(!body.contains("Recently viewed"));

    get_page(t.app(), "/movie/603", false).await;
    get_page(t.app(), "/tv/1396", false).await;
    get_page(t.app(), "/person/6384", false).await;
    get_page(t.app(), "/movie/603", false).await;
    // Not recorded: failed lookups, sub-pages and the JSON API.
    get_page(t.app(), "/movie/404", false).await;
    get_page(t.app(), "/movie/603/credits", false).await;
    get_json(t.app(), "/api/person/1").await;

    let (status, _, body) = get_page(t.app(), "/", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<h2 id="recent-heading">Recently viewed</h2>"#));
    assert!(!body.contains("Pages you open will be listed here."));
    let matrix = body.find(r#"<a href="/movie/603">"#).unwrap();
    let keanu = body.find(r#"<a href="/person/6384">"#).unwrap();
    let breaking_bad = body.find(r#"<a href="/tv/1396">"#).unwrap();
    assert!(matrix < keanu && keanu < breaking_bad, "most recent first");
    assert_eq!(body.matches(r#"<a href="/movie/603">"#).count(), 1);
    assert!(!body.contains("/movie/404") && !body.contains("/person/1\""));
    assert!(body.contains("https://img.test/t/p/w342/matrix.jpg"));
    assert!(body.contains("https://img.test/t/p/w185/keanu.jpg"));
    assert!(body.contains(
        r#"<span class="role"><span class="badge badge-movie">Movie</span> · 1999</span>"#
    ));
    assert!(body.contains(r#"<span class="badge badge-tv">TV show</span>"#));
    assert!(body.contains(r#"<span class="badge badge-person">Person</span></span>"#));

    let (_, _, fragment) = get_page(t.app(), "/?q=", true).await;
    assert!(!fragment.contains("<html"));
    assert!(
        fragment.contains("Recently viewed"),
        "clearing the search shows recents"
    );
}

#[tokio::test]
async fn json_api_lists_and_records_recently_viewed_pages() {
    let database = temp_database("api-recent");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    let (status, json) = get_json(t.app(), "/api/recent").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, json!({ "recent": [] }));

    get_page(t.app(), "/movie/603", false).await;
    let (status, json) = post_json(
        t.app(),
        "/api/recent",
        json!({ "kind": "person", "id": 6384 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["recent"][0]["name"], "Keanu Reeves");
    let (_, listed) = get_json(t.app(), "/api/recent").await;
    assert_eq!(listed, json, "answers with the new list");
    assert_eq!(
        json["recent"][1],
        json!({
            "kind": "movie",
            "id": 603,
            "name": "The Matrix",
            "year": "1999",
            "image_url": "https://img.test/t/p/w342/matrix.jpg",
        })
    );
    post_json(t.app(), "/api/recent", json!({ "kind": "tv", "id": 1396 })).await;
    let (_, _, body) = get_page(t.app(), "/", false).await;
    let breaking_bad = body.find(r#"<a href="/tv/1396">"#).unwrap();
    let keanu = body.find(r#"<a href="/person/6384">"#).unwrap();
    assert!(breaking_bad < keanu, "the home page shows API views too");

    let (status, _) = post_json(
        t.app(),
        "/api/recent",
        json!({ "kind": "movie", "id": 404 }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_json(
        t.app(),
        "/api/recent",
        json!({ "kind": "episode", "id": 1 }),
    )
    .await;
    assert!(status.is_client_error());
    let (_, json) = get_json(t.app(), "/api/recent").await;
    assert_eq!(json["recent"].as_array().unwrap().len(), 3);

    // An unreadable list is an error, never an empty one.
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE recently_viewed RENAME TO recently_viewed_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (status, json) = get_json(t.app(), "/api/recent").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json["error"].as_str().unwrap().contains("couldn't be read"));
    let (status, json) = post_json(
        t.app(),
        "/api/recent",
        json!({ "kind": "movie", "id": 603 }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("nothing was saved")
    );
    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn about_page_carries_tmdb_and_omdb_attribution() {
    let (status, _, body) = get_page(test_app().await, "/about", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("This product uses the TMDB API but is not endorsed or certified by TMDB.")
    );
    assert!(body.contains("/static/tmdb-logo.svg"));
    assert!(body.contains("OMDb API"));
    assert!(body.contains(r#"Where to watch data by <a href="https://www.justwatch.com/""#));
}

#[tokio::test]
async fn full_page_search_distinguishes_movies_tv_and_people_and_links_to_detail() {
    let (status, headers, body) = get_page(test_app().await, "/?q=matrix", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["vary"], "HX-Request");
    assert!(body.starts_with("<!doctype html>"));
    assert!(body.contains(r#"href="/movie/603""#));
    assert!(body.contains(r#"href="/tv/1396""#));
    assert!(body.contains(r#"<span class="badge badge-movie">Movie</span>"#));
    assert!(body.contains(r#"<span class="badge badge-tv">TV show</span>"#));
    assert!(body.contains(r#"href="/person/6384""#));
    assert!(body.contains(r#"<span class="badge badge-person">Person</span>"#));
    assert!(body.contains("Acting · known for The Matrix (1999), Swedish Dicks"));
    assert!(body.contains("https://img.test/t/p/w185/keanu.jpg"));
    assert!(
        body.contains("No photo"),
        "missing profile has a placeholder"
    );
    assert!(
        !body.contains("Collection"),
        "other result types are dropped"
    );
    assert!(body.contains("https://img.test/t/p/w342/matrix.jpg"));
    assert!(
        body.contains("No artwork"),
        "missing poster has a placeholder"
    );
}

#[tokio::test]
async fn htmx_search_returns_only_the_results_fragment() {
    let (status, _, body) = get_page(test_app().await, "/?q=matrix", true).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("<html"));
    assert!(body.contains(r#"href="/movie/603""#));
}

#[tokio::test]
async fn title_results_offer_to_watch_and_favorite_toggles() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    post_form(t.app(), "/tv/1396/favorite", "listed=true").await;

    let (_, _, body) = get_page(t.app(), "/?q=the%20matrix", true).await;
    assert!(body.contains(r##"id="watchlist-movie-603" method="post" action="/movie/603/watchlist" hx-post="/movie/603/watchlist" hx-target="#watchlist-movie-603" hx-select="#watchlist-movie-603""##));
    assert!(body.contains(r#"name="back" value="/?q=the%20matrix#watchlist-movie-603""#));
    assert!(body.contains(r#"name="back" value="/?q=the%20matrix#favorite-tv-1396""#));
    assert!(body.contains(r#"aria-pressed="true" aria-label="To watch: The Matrix""#));
    assert!(body.contains(r#"aria-pressed="false" aria-label="Favorite: The Matrix""#));
    assert!(body.contains(r#"aria-pressed="false" aria-label="To watch: Breaking Bad""#));
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: Breaking Bad""#));
    assert!(
        !body.contains("/person/6384/watchlist") && !body.contains("/person/6384/favorite"),
        "people have no list toggles"
    );

    let (_, headers) = post_form(
        t.app(),
        "/movie/603/favorite",
        "listed=true&back=%2F%3Fq%3Dthe%2520matrix%23favorite-movie-603",
    )
    .await;
    assert_eq!(
        headers[header::LOCATION],
        "/?q=the%20matrix#favorite-movie-603"
    );
    let (_, _, body) = get_page(t.app(), "/?q=the%20matrix", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: The Matrix""#));
}

#[tokio::test]
async fn empty_results_and_provider_failure_have_clear_states() {
    let (status, _, body) = get_page(test_app().await, "/?q=nothing", true).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("No movies, TV shows or people match “nothing”."));

    let (status, _, body) = get_page(test_app().await, "/?q=boom", false).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("Search failed."));
    assert!(body.contains("responding right now"));
}

#[tokio::test]
async fn rejected_token_explains_the_configuration_problem() {
    let (status, _, body) = get_page(test_app_with_token("wrong.token").await, "/?q=x", true).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("Check TMDB_API_TOKEN"));
}

#[tokio::test]
async fn movie_detail_page_shows_metadata_and_artwork() {
    let (status, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<title>The Matrix · credits</title>"));
    assert!(body.contains("Welcome to the Real World."));
    assert!(body.contains("Action, Science Fiction"));
    assert!(body.contains("136 min"));
    assert!(body.contains("https://img.test/t/p/w342/matrix.jpg"));
    assert!(
        !body.contains(r#"class="backdrop""#),
        "no backdrop when TMDB has none"
    );
    assert!(
        !body.contains(r#"class="original""#),
        "original title hidden when identical"
    );
}

#[tokio::test]
async fn tv_detail_page_shows_seasons_and_missing_poster_placeholder() {
    let (status, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("TV show"));
    assert!(body.contains("<dt>Seasons</dt><dd>5</dd>"));
    assert!(body.contains("<dt>Episodes</dt><dd>62</dd>"));
    assert!(body.contains("https://img.test/t/p/w1280/bb-back.jpg"));
    assert!(body.contains(r#"aria-label="No artwork available""#));
    assert!(
        !body.contains(r#"class="tagline""#),
        "blank tagline is omitted"
    );
}

#[tokio::test]
async fn unknown_titles_and_provider_errors_render_error_pages() {
    let (status, _, body) = get_page(test_app().await, "/movie/999", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("TMDB has no movie with that ID."));

    let (status, _, _) = get_page(test_app().await, "/movie/not-a-number", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, body) = get_page(test_app().await, "/tv/500", false).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("load this title"));
}

#[tokio::test]
async fn json_api_reuses_the_same_models() {
    let (status, json) = get_json(test_app().await, "/api/search?q=matrix").await;
    assert_eq!(status, StatusCode::OK);
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0]["kind"], "movie");
    assert_eq!(results[0]["year"], "1999");
    assert_eq!(results[1]["kind"], "person");
    assert_eq!(results[1]["name"], "Keanu Reeves");
    assert_eq!(results[1]["known_for_department"], "Acting");
    assert_eq!(
        results[1]["known_for"],
        json!([
            { "kind": "movie", "id": 603, "title": "The Matrix", "year": "1999" },
            { "kind": "tv", "id": 9999, "title": "Swedish Dicks", "year": null }
        ])
    );
    assert_eq!(
        results[1]["profile_url"],
        "https://img.test/t/p/w185/keanu.jpg"
    );
    assert_eq!(results[2]["known_for_department"], Value::Null);
    assert_eq!(results[2]["known_for"], json!([]));
    assert_eq!(results[3]["kind"], "tv");
    assert_eq!(results[3]["overview"], Value::Null);

    let (status, json) = get_json(test_app().await, "/api/tv/1396").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["seasons"], 5);
    assert_eq!(json["poster_url"], Value::Null);

    let (status, json) = get_json(test_app().await, "/api/search?q=").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json["error"].is_string());

    let (status, _) = get_json(test_app().await, "/api/movie/999").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn static_assets_are_served() {
    let (status, headers, body) = get_page(test_app().await, "/static/htmx.min.js", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/javascript");
    assert!(body.starts_with("var htmx="));
}

#[tokio::test]
async fn pages_link_the_favicon() {
    let (_, _, body) = get_page(test_app().await, "/", false).await;
    assert!(body.contains(r#"<link rel="icon" href="/static/favicon.svg" type="image/svg+xml">"#));

    let (status, headers, body) = get_page(test_app().await, "/static/favicon.svg", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/svg+xml");
    assert!(body.starts_with("<svg"));
}

#[tokio::test]
async fn stylesheet_url_changes_with_its_content() {
    let (_, _, body) = get_page(test_app().await, "/", false).await;
    let href = format!(
        r#"href="/static/app.css?v={}""#,
        credits::web::APP_CSS_VERSION
    );
    assert!(body.contains(&href), "stylesheet link is versioned");

    let (status, headers, _) = get_page(test_app().await, "/static/app.css?v=1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/css");
}

#[tokio::test]
async fn every_theme_in_the_picker_has_a_stylesheet_palette() {
    let (_, _, body) = get_page(test_app().await, "/", false).await;
    let (_, _, css) = get_page(test_app().await, "/static/app.css", false).await;
    assert!(
        body.contains(r#"<label class="theme-picker" hidden>"#),
        "shown only by script"
    );
    for theme in ["rose-pine", "tokyo-night", "catppuccin", "classic"] {
        assert!(
            body.contains(&format!(r#"<option value="{theme}">"#)),
            "{theme} offered"
        );
        assert!(
            css.contains(&format!(r#":root[data-theme="{theme}"]"#)),
            "{theme} styled"
        );
    }
    assert!(css.starts_with(r#"/* Themes: Rosé Pine is the default"#));
}

#[tokio::test]
async fn movie_prefers_rotten_tomatoes_and_tv_prefers_imdb() {
    let (_, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert!(body.contains(
        r#"<p class="rating preferred"><span class="ring rt-fresh" style="--p:83" role="img" aria-label="Rotten Tomatoes: Fresh, 83%" title="Rotten Tomatoes: Fresh, 83%"><span class="ring-score" aria-hidden="true">83</span><img class="ring-mark" src="/static/rt-fresh.svg" alt=""></span><span class="rating-label">Rotten Tomatoes<span class="muted">Fresh</span></span></p>"#
    ));
    assert!(body.contains(r#"<p class="rating other"><span class="ring imdb" style="--p:87" role="img" aria-label="IMDb, 8.7/10""#));
    assert!(!body.contains("73/100"), "unsupported sources are dropped");

    let (_, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert!(body.contains(
        r#"aria-label="IMDb, 9.5/10" title="IMDb, 9.5/10"><span class="ring-score" aria-hidden="true">95</span><img class="ring-mark" src="/static/imdb.svg" alt=""></span><span class="rating-label">IMDb<span class="muted">9.5/10 · 2,300,000 votes</span></span>"#
    ));
}

#[tokio::test]
async fn missing_preferred_score_is_unavailable_not_substituted() {
    let (_, _, body) = get_page(test_app().await, "/movie/6060", false).await;
    assert!(body.contains("No Rotten Tomatoes score"));
    assert!(
        body.contains(
            r#"<p class="rating other"><span class="ring imdb" style="--p:61" role="img" aria-label="IMDb, 6.1/10""#
        )
    );

    let (status, json) = get_json(test_app().await, "/api/movie/6060").await;
    assert_eq!(status, StatusCode::OK);
    let ratings = &json["ratings"];
    assert_eq!(ratings["state"], "fresh");
    assert_eq!(ratings["preferred_source"], "rotten_tomatoes");
    assert_eq!(ratings["preferred"], Value::Null);
    assert_eq!(ratings["others"][0]["source"], "imdb");
    assert_eq!(ratings["others"][0]["votes"], Value::Null);
}

#[tokio::test]
async fn json_ratings_carry_source_scale_and_provenance() {
    let (_, json) = get_json(test_app().await, "/api/movie/603").await;
    let ratings = &json["ratings"];
    assert_eq!(ratings["provider"], "omdb");
    assert_eq!(ratings["imdb_id"], "tt0133093");
    assert_eq!(
        ratings["preferred"],
        json!({ "source": "rotten_tomatoes", "score": 83.0, "scale": 100, "votes": null })
    );
    assert_eq!(ratings["others"][0]["scale"], 10);
    assert!(ratings["fetched_at"].as_str().unwrap().ends_with('Z'));
}

#[tokio::test]
async fn ratings_are_cached_between_requests() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    get_page(t.app(), "/movie/603", false).await;
    get_page(t.app(), "/movie/603", false).await;
    get_json(t.app(), "/api/movie/603").await;
    assert_eq!(t.omdb.calls(), 1);
}

#[tokio::test]
async fn titles_omdb_has_no_data_for_are_cached_as_unrated() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, json) = get_json(t.app(), "/api/movie/4040").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["ratings"]["state"], "fresh");
    assert!(json["ratings"]["preferred"].is_null());
    get_json(t.app(), "/api/movie/4040").await;
    assert_eq!(
        t.omdb.calls(),
        1,
        "the miss is cached, not retried per view"
    );
}

#[tokio::test]
async fn expired_ratings_are_refetched_and_served_stale_when_omdb_fails() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::ZERO).await;
    get_page(t.app(), "/tv/1396", false).await;
    get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(t.omdb.calls(), 2, "expired entries are refreshed");

    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"aria-label="IMDb, 9.5/10""#));
    assert!(body.contains("Couldn't refresh ratings; showing scores from "));

    let (_, json) = get_json(t.app(), "/api/tv/1396").await;
    assert_eq!(json["ratings"]["state"], "stale");
    assert!(json["ratings"]["fetched_at"].is_string());
}

#[tokio::test]
async fn provider_problems_never_break_the_page() {
    // Quota exhausted, nothing cached.
    let (status, _, body) = get_page(test_app().await, "/tv/5150", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Ratings couldn&#39;t be loaded right now."));

    // OMDb down, nothing cached.
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.omdb.set_down(true);
    let (status, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["ratings"]["state"], "unavailable");
    assert_eq!(json["ratings"]["unavailable_reason"], "provider_error");

    // Rejected key.
    let t = build(TOKEN, Some("wrong"), Duration::from_secs(3600)).await;
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["ratings"]["unavailable_reason"], "provider_error");

    // No IMDb identifier: OMDb is never asked.
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, json) = get_json(t.app(), "/api/tv/4242").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["ratings"]["unavailable_reason"], "no_imdb_id");
    assert_eq!(t.omdb.calls(), 0);
}

#[tokio::test]
async fn missing_omdb_key_reports_ratings_as_not_configured() {
    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("configured on this server"));
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["ratings"]["unavailable_reason"], "not_configured");
    assert_eq!(t.omdb.calls(), 0);
}

#[tokio::test]
async fn movie_page_links_key_crew_and_top_cast_to_people() {
    let (status, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(
        r#"<dt>Directed by</dt><dd><a href="/person/9340">Lana Wachowski</a>, <a href="/person/9339">Lilly Wachowski</a></dd>"#
    ));
    assert!(
        body.contains(r#"<dt>Written by</dt><dd><a href="/person/9340">Lana Wachowski</a></dd>"#)
    );
    assert!(body.contains(r#"<a href="/person/6384">"#));
    assert!(
        body.contains(r#"<span class="role">Neo / Thomas Anderson</span>"#),
        "one tile per actor, characters merged"
    );
    assert!(body.contains("https://img.test/t/p/w185/keanu.jpg"));
    assert!(
        body.contains("No photo"),
        "missing headshot has a placeholder"
    );
    assert!(body.contains(r#"<a href="/movie/603/credits">Full cast &amp; crew</a>"#));
    assert!(body.contains("(2 cast, 3 crew)"));
}

#[tokio::test]
async fn credits_page_lists_everyone_grouped_by_department() {
    let (status, _, body) = get_page(test_app().await, "/movie/603/credits", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<a href="/movie/603">← The Matrix (1999)</a>"#));
    let directing = body.find(r#"id="dept-directing""#).unwrap();
    let writing = body.find(r#"id="dept-writing""#).unwrap();
    let production = body.find(r#"id="dept-production""#).unwrap();
    assert!(directing < writing && writing < production);
    assert!(
        body.contains(r##"<a href="#dept-directing">Directing <span class="muted">2</span></a>"##)
    );
    assert!(body.contains(
        r#"<li><a href="/person/2975">Laurence Fishburne</a> <span class="muted">as Morpheus</span></li>"#
    ));

    let (status, json) = get_json(test_app().await, "/api/movie/603/credits").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["cast"].as_array().unwrap().len(), 2);
    assert_eq!(json["crew"][0]["name"], "Directing");
    assert_eq!(json["crew"][0]["people"][0]["person_id"], 9340);

    let (status, _, _) = get_page(test_app().await, "/movie/999/credits", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cast_episode_counts_show_once_the_whole_show_is_watched() {
    let database = temp_database("cast-episodes");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;

    // A season alone isn't enough: the count spans the whole show.
    post_form(t.app(), "/tv/1396/season/1/watched", "watched=true").await;
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert!(!body.contains("62 episodes"));

    // The show toggle swaps the cast in too, so counts appear without a reload.
    assert!(body.contains(r##"<div class="watch-state" hx-select-oob="#cast">"##));
    post_form(t.app(), "/tv/1396/watched", "watched=true").await;
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert!(body.contains(r#"<section class="section" id="cast""#));
    assert!(body.contains(r#"<span class="role">62 episodes</span>"#));
    let (_, json) = get_json(t.app(), "/api/tv/1396").await;
    assert_eq!(json["cast"][0]["episode_count"], 62);
    let (_, _, body) = get_page(t.app(), "/tv/1396/credits", false).await;
    assert!(body.contains("· 62 episodes"));
    let (_, _, body) = get_page(t.app(), "/person/6384", false).await;
    assert!(body.contains("· 1 episode"));
    let (_, json) = get_json(t.app(), "/api/person/6384").await;
    let acting = json["filmography"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["department"] == "Acting")
        .unwrap();
    let show = |id: u64| {
        acting["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap()["episode_count"]
            .clone()
    };
    assert_eq!(show(1396), 1);
    assert!(show(59941).is_null(), "other shows stay hidden");

    // Marks that can't be read keep them hidden.
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE watched_shows RENAME TO watched_shows_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (_, json) = get_json(t.app(), "/api/tv/1396").await;
    assert!(json["cast"][0]["episode_count"].is_null());
    let (_, _, body) = get_page(t.app(), "/person/6384", false).await;
    assert!(!body.contains("· 1 episode"));
    drop(t);
    let _ = std::fs::remove_file(&database);
}

#[tokio::test]
async fn tv_page_shows_creators_directors_and_seasons() {
    let (status, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<dt>Created by</dt><dd><a href="/person/66633">Vince Gilligan</a></dd>"#)
    );
    assert!(
        body.contains(
            r#"<dt>Directed by</dt><dd><a href="/person/29924">Michelle MacLaren</a>, <a href="/person/66633">Vince Gilligan</a></dd>"#
        ),
        "most episodes first"
    );
    assert!(
        !body.contains("62 episodes"),
        "an actor's episode count waits for the show to be watched"
    );
    let season1 = body.find(r#"href="/tv/1396/season/1""#).unwrap();
    let season2 = body.find(r#"href="/tv/1396/season/2""#).unwrap();
    let specials = body.find(r#"href="/tv/1396/season/0""#).unwrap();
    assert!(season1 < season2 && season2 < specials, "specials last");
    assert!(
        body.contains(r#"<span class="name">Season 2</span>"#),
        "blank name falls back"
    );
    assert!(body.contains("2008 · 7 episodes"));

    let (_, json) = get_json(test_app().await, "/api/tv/1396").await;
    assert_eq!(json["key_crew"][0]["role"], "creator");
    assert_eq!(json["season_list"][0]["number"], 1);
    assert!(json["cast"][0]["episode_count"].is_null());

    let (_, _, body) = get_page(test_app().await, "/tv/1396/credits", false).await;
    let creators = body.find(r#"id="dept-creators""#).unwrap();
    let directing = body.find(r#"id="dept-directing""#).unwrap();
    assert!(creators < directing);
}

#[tokio::test]
async fn season_page_lists_episodes_with_fallbacks() {
    let (status, _, body) = get_page(test_app().await, "/tv/1396/season/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<title>Season 1 · Breaking Bad · credits</title>"));
    assert!(body.contains(r#"<a href="/tv/1396">← Breaking Bad</a>"#));
    assert!(body.contains(r#"<span class="muted">1.</span> Pilot"#));
    assert!(body.contains("Jan 20, 2008 · 58 min"));
    assert!(body.contains("A chemistry teacher is diagnosed."));
    assert!(body.contains("https://img.test/t/p/w300/pilot.jpg"));
    assert!(body.contains(r#"<span class="muted">2.</span> Episode 2"#));
    assert!(body.contains("Air date not announced"));
    assert!(body.contains("No summary available."));
    assert!(body.contains(r#"<a href="/tv/1396/season/2">Season 2 →</a>"#));
    assert!(!body.contains("←  "), "no previous link before season 1");

    let (status, json) = get_json(test_app().await, "/api/tv/1396/season/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["show_title"], "Breaking Bad");
    assert_eq!(json["episodes"][0]["air_date"], "2008-01-20");
    assert_eq!(json["episodes"][1]["title"], "Episode 2");
    assert_eq!(json["episodes"][1]["overview"], Value::Null);
    assert_eq!(json["previous"], Value::Null);
    assert_eq!(json["next"]["number"], 2);

    let (status, _, body) = get_page(test_app().await, "/tv/1396/season/9", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("Season not found"));
    let (status, _) = get_json(test_app().await, "/api/tv/1396/season/x").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn episode_page_shows_episode_cast_and_crew() {
    let (_, _, body) = get_page(test_app().await, "/tv/1396/season/1", false).await;
    assert!(body.contains(
        r#"<h3><a href="/tv/1396/season/1/episode/1"><span class="muted">1.</span> Pilot</a></h3>"#
    ));

    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1/episode/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<title>Pilot · Season 1 · Breaking Bad · credits</title>"));
    assert!(body.contains(
        r#"<a href="/tv/1396">← Breaking Bad</a> · <a href="/tv/1396/season/1">Season 1</a>"#
    ));
    assert!(body.contains("Season 1 · Episode 1"));
    assert!(body.contains("https://img.test/t/p/w300/pilot.jpg"));
    assert!(body.contains("<dt>Runtime</dt><dd>58 min</dd>"));
    assert!(
        body.contains(r#"<dt>Directed by</dt><dd><a href="/person/66633">Vince Gilligan</a></dd>"#)
    );
    assert!(
        body.contains(r#"<dt>Written by</dt><dd><a href="/person/66633">Vince Gilligan</a></dd>"#)
    );
    let guests = body.find("Guest stars").unwrap();
    let regulars = body.find("Series cast").unwrap();
    assert!(guests < regulars);
    assert!(
        body.contains(r#"<span class="role">Steven Gomez / Gomie</span>"#),
        "roles merged"
    );
    assert_eq!(body.matches(r#"href="/person/61535""#).count(), 1);
    assert!(body.contains(r#"<a href="/person/17419">"#));
    assert!(body.contains(r#"<span class="role">Walter White</span>"#));
    assert!(body.contains(r#"id="dept-editing""#));
    assert!(body.contains(r#"<a href="/tv/1396/season/1/episode/2">Episode 2 →</a>"#));
    assert!(
        !body.contains("← Episode"),
        "no previous link before episode 1"
    );
    // Unwatched: score and summary blurred.
    assert!(body.contains(r#"class="meta spoiler" id="episode""#));
    assert!(body.contains(r#"aria-label="IMDb, 9.0/10""#));
    assert!(body.contains(r#"<p class="overview" tabindex="0" title="Spoiler: select to peek">A chemistry teacher is diagnosed.</p>"#));

    let (status, json) = get_json(t.app(), "/api/tv/1396/season/1/episode/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["title"], "Pilot");
    assert_eq!(json["season_name"], "Season 1");
    assert_eq!(json["rating"]["score"], 9.0);
    assert_eq!(json["guest_stars"][0]["role"], "Steven Gomez / Gomie");
    assert_eq!(json["cast"][0]["name"], "Bryan Cranston");
    assert_eq!(json["key_crew"][0]["role"], "director");
    assert_eq!(json["previous"], Value::Null);
    assert_eq!(json["next"], 2);

    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1/episode/99", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("Episode not found"));
    let (status, _) = get_json(t.app(), "/api/tv/1396/season/1/episode/x").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tv_season_and_episode_json_carry_watched_marks() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "credits-watched-{}-{unique}.db",
        std::process::id()
    ));
    let database = path.to_str().unwrap();
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json.get("watched"), None, "movies carry no TV marks");
    post_form(t.app(), "/tv/1396/season/2/watched", "watched=true").await;
    post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=true",
    )
    .await;
    for uri in [
        "/api/tv/1396",
        "/api/tv/1396/season/1",
        "/api/tv/1396/season/1/episode/1",
    ] {
        let (status, json) = get_json(t.app(), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            json["watched"],
            json!({
                "state": "read",
                "show": false,
                "seasons": [2],
                "episodes": [{"season": 1, "number": 1, "apple_tv": false}],
            }),
            "{uri}"
        );
    }
    let (_, json) = get_json(t.app(), "/api/tv/1396/season/1").await;
    assert_eq!(
        json["episodes"][0]["title"], "Pilot",
        "season fields stay top-level"
    );

    let pool = credits::cache::ProviderCache::open(database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE watched_episodes RENAME TO watched_episodes_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (status, json) = get_json(t.app(), "/api/tv/1396/season/1/episode/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["title"], "Pilot");
    assert_eq!(json["watched"]["state"], "unreadable");
    assert_eq!(json["watched"]["seasons"], json!([]));
    drop(t);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn marking_an_episode_watched_from_its_page_returns_there() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, headers) = post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=true&back=%2Ftv%2F1396%2Fseason%2F1%2Fepisode%2F1",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/tv/1396/season/1/episode/1");
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1/episode/1", false).await;
    assert!(body.contains(r#"class="meta" id="episode""#));
    assert!(body.contains(r#"<p class="overview">A chemistry teacher is diagnosed.</p>"#));

    // Any other page falls back to the episode on its season page.
    let (_, headers) = post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=false&back=%2Ftv%2F1396%2Fseason%2F1%2Fepisode%2F2",
    )
    .await;
    assert_eq!(headers[header::LOCATION], "/tv/1396/season/1#episode-1");
}

#[tokio::test]
async fn season_page_shows_imdb_episode_ratings() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"58 min · <span class="episode-rating" tabindex="0" title="Spoiler: select to peek"><span class="ring imdb" style="--p:90" role="img" aria-label="IMDb, 9.0/10""#));
    assert_eq!(
        body.matches("episode-rating\"").count(),
        1,
        "no score, no rating for episode 2"
    );
    assert!(!body.contains("episode-ratings-note"));

    let (_, json) = get_json(t.app(), "/api/tv/1396/season/1").await;
    assert_eq!(json["episodes"][0]["rating"]["source"], "imdb");
    assert_eq!(json["episodes"][0]["rating"]["score"], 9.0);
    assert_eq!(json["episodes"][1]["rating"], Value::Null);
    assert_eq!(json["episode_ratings"]["state"], "fresh");
    assert_eq!(json["episode_ratings"]["imdb_id"], "tt0903747");
    assert_eq!(t.omdb.calls(), 1, "one cached OMDb request per season");

    // OMDb has no such season: fresh, just no scores.
    let (_, json) = get_json(t.app(), "/api/tv/1396/season/2").await;
    assert_eq!(json["episode_ratings"]["state"], "fresh");
    assert_eq!(json["episodes"][0]["rating"], Value::Null);
}

#[tokio::test]
async fn episode_rating_problems_never_break_the_season_page() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::ZERO).await;
    get_page(t.app(), "/tv/1396/season/1", false).await;
    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"aria-label="IMDb, 9.0/10""#));
    assert!(body.contains("Couldn't refresh episode ratings; showing scores from "));

    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Episode ratings couldn&#39;t be loaded right now."));
    assert!(!body.contains("episode-rating\""));

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    let (_, json) = get_json(t.app(), "/api/tv/1396/season/1").await;
    assert_eq!(
        json["episode_ratings"]["unavailable_reason"],
        "not_configured"
    );
    assert_eq!(t.omdb.calls(), 0);
}

#[tokio::test]
async fn episode_spoilers_are_blurred_until_watched() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(body.contains(r#"<li class="episode spoiler" id="episode-1">"#));
    assert!(body.contains(r#"<p class="overview" tabindex="0" title="Spoiler: select to peek">A chemistry teacher is diagnosed.</p>"#));
    assert!(body.contains(r#"action="/tv/1396/watched""#));
    assert!(body.contains(r#"action="/tv/1396/season/1/watched""#));
    assert!(body.contains(r#"action="/tv/1396/season/1/episode/1/watched""#));
    assert_eq!(
        body.matches(r#"aria-pressed="false""#).count(),
        4,
        "show, season, two episodes"
    );
    assert!(body.contains(r#"name="watched" value="true""#));

    // One episode.
    let (status, headers) = post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=true",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/tv/1396/season/1#episode-1");
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(body.contains(r#"<li class="episode" id="episode-1">"#));
    assert!(body.contains(r#"<p class="overview">A chemistry teacher is diagnosed.</p>"#));
    assert!(body.contains(r#"<li class="episode spoiler" id="episode-2">"#));
    post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=false",
    )
    .await;
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(body.contains(r#"<li class="episode spoiler" id="episode-1">"#));

    // The season covers its episodes, whose toggles can't undo it.
    let (status, headers) = post_form(t.app(), "/tv/1396/season/1/watched", "watched=true").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/tv/1396/season/1");
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(!body.contains("episode spoiler"));
    assert_eq!(
        body.matches(r#"title="Watched with the season" disabled"#)
            .count(),
        2
    );
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/2", false).await;
    assert!(
        body.contains("episode spoiler"),
        "other seasons stay blurred"
    );
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(body.matches("✓ Watched").count(), 1);
    assert!(body.contains(r#"aria-pressed="false""#));

    // The whole show, from a season page, returns to that season page.
    let (status, headers) = post_form(
        t.app(),
        "/tv/1396/watched",
        "watched=true&back=%2Ftv%2F1396%2Fseason%2F2",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/tv/1396/season/2");
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/2", false).await;
    assert!(!body.contains("episode spoiler"));
    assert!(body.contains(r#"title="Watched with the whole show" disabled"#));
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert!(body.contains(r#"aria-pressed="true""#));
    assert!(body.contains(r#"name="watched" value="false""#));

    // Unwatching the show keeps the season marked on its own.
    let (_, headers) = post_form(t.app(), "/tv/1396/watched", "watched=false").await;
    assert_eq!(headers[header::LOCATION], "/tv/1396");
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/2", false).await;
    assert!(body.contains("episode spoiler"));
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(!body.contains("episode spoiler"));

    // Only pages of the same show are valid places to return to.
    for back in [
        "https%3A%2F%2Fevil.test",
        "%2Ftv%2F13960",
        "%2Ftv%2F1396%2F%2Fevil.test",
    ] {
        let (_, headers) = post_form(
            t.app(),
            "/tv/1396/watched",
            &format!("watched=false&back={back}"),
        )
        .await;
        assert_eq!(headers[header::LOCATION], "/tv/1396", "{back}");
    }
    let (status, _) = post_form(t.app(), "/tv/x/watched", "watched=true").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_form(
        t.app(),
        "/tv/1396/season/1/episode/x/watched",
        "watched=true",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = post_form(t.app(), "/tv/1396/watched", "watched=maybe").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn marking_the_last_episode_or_season_rolls_up_to_the_show() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    // Breaking Bad has ended: specials (9), season 1 (7) and season 2 (13).
    for e in 1..=6 {
        post_form(
            t.app(),
            &format!("/tv/1396/season/1/episode/{e}/watched"),
            "watched=true",
        )
        .await;
    }
    let watched = t.catalog.watched(1396).await;
    assert!(watched.seasons.is_empty(), "one episode short");

    let (_, json) = put_json(
        t.app(),
        "/api/tv/1396/season/1/episode/7/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(
        json["seasons"],
        json!([1]),
        "the last episode marks its season"
    );
    assert_eq!(json["show"], false);

    post_form(t.app(), "/tv/1396/season/2/watched", "watched=true").await;
    let watched = t.catalog.watched(1396).await;
    assert!(watched.show, "the last regular season marks the show");
    assert_eq!(watched.seasons, [1, 2], "specials aren't needed");
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert!(body.contains(r#"name="watched" value="false""#));

    // Unwatching the show sticks until another mark completes it again,
    // from cached seasons while TMDB is down.
    post_form(t.app(), "/tv/1396/watched", "watched=false").await;
    assert!(!t.catalog.watched(1396).await.show);
    t.tmdb.set_down(true);
    post_form(t.app(), "/tv/1396/season/2/watched", "watched=true").await;
    assert!(t.catalog.watched(1396).await.show);

    // A show TMDB can't describe still saves the mark, without rolling up.
    let (status, _) = post_form(t.app(), "/tv/500/season/1/watched", "watched=true").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let watched = t.catalog.watched(500).await;
    assert_eq!((watched.show, watched.seasons), (false, vec![1]));
}

#[tokio::test]
async fn watching_a_title_takes_it_off_to_watch() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    for path in ["/movie/603", "/tv/1396", "/tv/5150"] {
        post_form(t.app(), &format!("{path}/watchlist"), "listed=true").await;
    }
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let to_watch = |json: Value| -> Vec<String> {
        json["titles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{}/{}", e["kind"].as_str().unwrap(), e["id"]))
            .collect()
    };

    post_form(t.app(), "/movie/603/watched", "watched=true").await;
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(to_watch(json), ["tv/5150", "tv/1396"]);
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"][0]["id"], 603, "favorites are untouched");

    // A season alone keeps the show; the one that completes it doesn't.
    post_form(t.app(), "/tv/1396/season/1/watched", "watched=true").await;
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(to_watch(json), ["tv/5150", "tv/1396"]);
    post_form(t.app(), "/tv/1396/season/2/watched", "watched=true").await;
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(to_watch(json), ["tv/5150"]);

    // Marking the whole show over JSON answers with it gone.
    let (_, json) = put_json(t.app(), "/api/tv/5150/watched", json!({ "watched": true })).await;
    assert_eq!(json["marks"]["to_watch"], false);

    // Unwatching doesn't put anything back.
    post_form(t.app(), "/movie/603/watched", "watched=false").await;
    post_form(t.app(), "/tv/5150/watched", "watched=false").await;
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(json["titles"], json!([]));
}

#[tokio::test]
async fn person_page_shows_biography_and_filmography_linked_to_titles() {
    let (status, _, body) = get_page(test_app().await, "/person/6384", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<title>Keanu Reeves · credits</title>"));
    assert!(body.contains("Known for Acting"));
    assert!(body.contains("<dt>Born</dt><dd>Sep 2, 1964 in Beirut, Lebanon</dd>"));
    assert!(body.contains(r#"<p class="biography">He was born in Beirut.</p>"#));
    assert!(body.contains("https://img.test/t/p/w185/keanu.jpg"));
    assert!(body.contains(r#"<a href="/movie/603">The Matrix</a>"#));
    assert!(body.contains(r#"<a href="/tv/1396">Breaking Bad</a>"#));
    assert!(body.contains("as Neo / Thomas Anderson"));
    assert!(
        !body.contains("· 1 episode"),
        "acting episode counts wait for the show to be watched"
    );
    assert!(body.contains(r#"<img class="poster thumb" src="https://img.test/t/p/w92/matrix.jpg" alt="" loading="lazy""#));
    assert!(body.contains(r#"<span class="poster thumb missing">No artwork</span>"#));
    assert!(
        body.contains(r#"id="dept-acting""#),
        "known-for department shown first"
    );
    assert!(
        !body.contains("Man of Tai Chi</a>"),
        "other departments wait behind tabs"
    );
    let filmography = &body[body.find(r#"id="filmography-heading""#).unwrap()..];
    let tba = filmography.find("TBA").unwrap();
    let matrix = filmography.find(r#"<a href="/movie/603">"#).unwrap();
    assert!(tba < matrix, "undated entries lead, then newest first");

    let (status, json) = get_json(test_app().await, "/api/person/6384").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["metadata"]["state"], "fresh");
    let acting = &json["filmography"][0];
    assert_eq!(acting["department"], "Acting");
    assert_eq!(acting["entries"].as_array().unwrap().len(), 4);
    assert_eq!(acting["entries"][2]["kind"], "tv");
    assert_eq!(acting["entries"][3]["role"], "Neo / Thomas Anderson");
}

#[tokio::test]
async fn person_page_tabs_departments_and_filters_by_kind() {
    let (_, _, body) = get_page(test_app().await, "/person/6384", false).await;
    let tabs = &body[body.find(r#"<nav class="tabs""#).unwrap()..];
    let tabs = &tabs[..tabs.find("</nav>").unwrap()];
    assert!(tabs.contains(
        r##"<a href="/person/6384#filmography-heading" aria-current="page">Acting <span class="count">4</span></a>"##
    ));
    assert!(
        tabs.contains(r##"<a href="/person/6384?dept=directing#filmography-heading">Directing"##)
    );
    assert!(body.contains(
        r##"<a href="/person/6384#filmography-heading" aria-current="true">All <span class="count">"##
    ));

    let (status, _, body) = get_page(test_app().await, "/person/6384?dept=directing", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"id="dept-directing""#));
    assert!(body.contains(r#"<a href="/movie/777">Man of Tai Chi</a>"#));
    assert!(!body.contains(r#"<a href="/movie/603">The Matrix</a>"#));
    assert!(body.contains(
        r##"<a href="/person/6384?sort=rating&#38;dept=directing#filmography-heading">Rating</a>"##
    ));
    assert!(
        body.contains(
            r##"<a href="/person/6384?kind=tv&#38;dept=directing#filmography-heading">TV"##
        )
    );

    let (_, _, body) = get_page(test_app().await, "/person/6384?kind=tv", false).await;
    assert!(body.contains(r#"<a href="/tv/1396">Breaking Bad</a>"#));
    assert!(!body.contains(r#"<a href="/movie/603">The Matrix</a>"#));
    assert!(body.contains(
        r##"<a href="/person/6384?kind=tv#filmography-heading" aria-current="true">TV"##
    ));
    assert!(
        !body.contains(">Directing <"),
        "departments without TV credits drop out"
    );

    // A department the kind leaves empty falls back to the first tab.
    let (status, _, body) = get_page(
        test_app().await,
        "/person/6384?kind=tv&dept=directing",
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"id="dept-acting""#));
}

#[tokio::test]
async fn person_page_shows_most_voted_titles_as_known_for() {
    let (status, json) = get_json(test_app().await, "/api/person/6384").await;
    assert_eq!(status, StatusCode::OK);
    let titles: Vec<&str> = json["known_for_titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    // Main department by votes, then other departments; no talk shows or
    // appearances as themselves.
    assert_eq!(titles, ["The Matrix", "Untitled Project", "Man of Tai Chi"]);
    assert_eq!(json["known_for_titles"][0]["role"], "Neo / Thomas Anderson");
    assert_eq!(
        json["known_for_titles"][0]["poster_url"],
        "https://img.test/t/p/w342/matrix.jpg"
    );

    let (_, _, body) = get_page(test_app().await, "/person/6384", false).await;
    let known_for = body
        .find(r#"<h2 id="known-for-heading">Known for</h2>"#)
        .unwrap();
    let filmography = body.find(r#"id="filmography-heading""#).unwrap();
    assert!(known_for < filmography);
    let row = &body[known_for..filmography];
    assert!(row.contains(r#"<li><a href="/movie/603">"#));
    assert!(row.contains(r#"<img class="poster" src="https://img.test/t/p/w342/matrix.jpg""#));
    assert!(row.contains(r#"<span class="role">1999 · Neo / Thomas Anderson</span>"#));
    assert!(row.contains(r#"<span class="role">TBA</span>"#));
    assert!(row.contains(r#"<div class="poster missing" aria-hidden="true">No artwork</div>"#));
    assert!(!row.contains("/tv/1396") && !row.contains("/tv/59941"));

    let (_, _, body) = get_page(test_app().await, "/person/1", false).await;
    assert!(!body.contains("known-for-heading"));
}

fn entry_titles(group: &Value) -> Vec<&str> {
    group["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn filmography_sorts_by_rotten_tomatoes_then_imdb() {
    let (status, json) = get_json(test_app().await, "/api/person/6384?sort=rating").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["sort"], "rating");
    let acting = &json["filmography"][0];
    // Breaking Bad has only IMDb's 9.5 (95%), The Matrix 83% on Rotten
    // Tomatoes; unscored credits follow in date order.
    assert_eq!(
        entry_titles(acting),
        [
            "Breaking Bad",
            "The Matrix",
            "Untitled Project",
            "The Tonight Show"
        ]
    );
    assert_eq!(acting["entries"][0]["rating"]["source"], "imdb");
    assert_eq!(acting["entries"][1]["rating"]["source"], "rotten_tomatoes");
    assert_eq!(acting["entries"][1]["rating"]["score"], 83.0);
    assert!(acting["entries"][2].get("rating").is_none());
    assert_eq!(json["sort_ratings"]["unrated"], 3);
    assert!(json["sort_ratings"]["unavailable_reason"].is_null());

    // Date order is rated too, just not ranked.
    let (_, json) = get_json(test_app().await, "/api/person/6384").await;
    assert_eq!(json["sort"], "date");
    assert_eq!(json["sort_ratings"]["unrated"], 3);
    let matrix = json["filmography"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["title"] == "The Matrix")
        .unwrap();
    assert_eq!(matrix["rating"]["score"], 83.0);

    let app = test_app().await;
    let (_, _, body) = get_page(app.clone(), "/person/6384?sort=rating", false).await;
    assert!(body.contains(
        r##"<a href="/person/6384?sort=rating#filmography-heading" aria-current="true">Rating</a>"##
    ));
    assert!(body.contains(r##"<a href="/person/6384#filmography-heading">Date</a>"##));
    assert!(body.contains(r#"hx-get="/person/6384/ratings?sort=rating" hx-trigger="load""#));
    assert!(body.contains("Loading ratings to sort by…"));
    assert!(
        body.contains(
            r#"<ul class="filmography with-toggle columns pending" id="filmography-list">"#
        )
    );
    assert!(
        !body.contains(r#"class="ring "#),
        "ratings load after the page"
    );
    // The ratings swap in the whole list, now in rating order.
    let (status, _, body) = get_page(app, "/person/6384/ratings?sort=rating", true).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<div class="rating-notes" id="rating-notes" hx-swap-oob="true">"#));
    assert!(body.contains(
        r#"<ul class="filmography with-toggle columns" id="filmography-list" hx-swap-oob="true">"#
    ));
    assert!(body.contains(r#"<span class="ring rt-fresh" style="--p:83" role="img" aria-label="Rotten Tomatoes: Fresh, 83%" title="Rotten Tomatoes: Fresh, 83%"><span class="ring-score" aria-hidden="true">83</span><img class="ring-mark" src="/static/rt-fresh.svg" alt=""></span>"#));
    assert!(body.contains(r#"aria-label="IMDb, 9.5/10""#));
    assert!(body.contains("Credits without a Rotten Tomatoes or IMDb score are listed last"));
    let bb = body.find(r#"<a href="/tv/1396">"#).unwrap();
    let matrix = body.find(r#"<a href="/movie/603">"#).unwrap();
    assert!(bb < matrix);

    let app = test_app().await;
    let (_, _, body) = get_page(app.clone(), "/person/6384", false).await;
    assert!(
        body.contains(
            r##"<a href="/person/6384#filmography-heading" aria-current="true">Date</a>"##
        )
    );
    assert!(body.contains(r#"hx-get="/person/6384/ratings" hx-trigger="load""#));
    assert!(body.contains(r#"<span class="credit-rating" id="rating-movie-603"></span>"#));
    assert!(!body.contains(r#"filmography-list" hx-swap-oob"#));
    // In date order each rated row's ring swaps into its slot.
    let (_, _, body) = get_page(app, "/person/6384/ratings", true).await;
    assert!(body.contains(r#"<span class="credit-rating" id="rating-movie-603" hx-swap-oob="true"><span class="ring rt-fresh""#));
    assert!(!body.contains("filmography-list"), "the list stays put");
    assert!(!body.contains("are listed last"));
}

#[tokio::test]
async fn rating_sort_is_cached_and_survives_provider_problems() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    get_json(t.app(), "/api/person/6384").await;
    let (omdb, tmdb) = (t.omdb.calls(), t.tmdb.calls());
    assert!(omdb > 0, "date order is rated too");
    get_json(t.app(), "/api/person/6384?sort=rating").await;
    assert_eq!(t.omdb.calls(), omdb, "the sort reuses cached ratings");
    // Only the two titles TMDB doesn't know are asked for again.
    assert_eq!(t.tmdb.calls(), tmdb + 2);

    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/person/6384/ratings?sort=rating", true).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Some ratings couldn&#39;t be loaded right now."));
    let (_, json) = get_json(t.app(), "/api/person/6384?sort=rating").await;
    assert_eq!(json["sort_ratings"]["unavailable_reason"], "provider_error");
    assert_eq!(
        entry_titles(&json["filmography"][0]),
        [
            "Untitled Project",
            "The Tonight Show",
            "Breaking Bad",
            "The Matrix"
        ],
        "nothing scored, so date order"
    );

    let t = build(TOKEN, Some(OMDB_KEY), Duration::ZERO).await;
    get_page(t.app(), "/person/6384/ratings?sort=rating", true).await;
    t.omdb.set_down(true);
    let (_, _, body) = get_page(t.app(), "/person/6384/ratings?sort=rating", true).await;
    assert!(body.contains("Couldn't refresh some ratings; using scores from as early as "));
    assert!(body.contains(r#"aria-label="Rotten Tomatoes: Fresh, 83%""#));

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/person/6384?sort=rating", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(
            "Ratings aren&#39;t configured on this server, so credits stay in date order."
        )
    );
    assert!(
        !body.contains("/person/6384/ratings"),
        "nothing to load later"
    );
    assert_eq!(t.omdb.calls(), 0);
    assert_eq!(t.tmdb.calls(), 1, "no title lookups without a ratings key");
}

#[tokio::test]
async fn sparse_and_unknown_people_have_clear_states() {
    let (status, _, body) = get_page(test_app().await, "/person/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("No biography available."));
    assert!(body.contains(r#"aria-label="No photo available""#));
    assert!(body.contains("TMDB has no credits listed for this person."));

    let (status, _, body) = get_page(test_app().await, "/person/404", false).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("TMDB has no person with that ID."));
    let (status, _) = get_json(test_app().await, "/api/person/404").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tmdb_responses_are_cached_and_shared_by_html_and_json() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    get_page(t.app(), "/movie/603", false).await;
    get_page(t.app(), "/movie/603/credits", false).await;
    get_json(t.app(), "/api/movie/603").await;
    get_json(t.app(), "/api/movie/603/credits").await;
    assert_eq!(
        t.tmdb.calls(),
        1,
        "title and credits share one cached response"
    );

    let omdb = t.omdb.calls();
    get_page(t.app(), "/person/6384", false).await;
    assert_eq!(t.tmdb.calls(), 2, "the page looks up only the person");
    assert_eq!(t.omdb.calls(), omdb, "ratings load after the page");
    get_page(t.app(), "/person/6384/ratings", true).await;
    let calls = t.tmdb.calls();
    get_json(t.app(), "/api/person/6384").await;
    // Only the two credited titles TMDB doesn't know are asked for again.
    assert_eq!(t.tmdb.calls(), calls + 2);
}

#[tokio::test]
async fn expired_tmdb_data_is_served_when_tmdb_is_down() {
    let t = build_with(
        TOKEN,
        Some(OMDB_KEY),
        Duration::from_secs(3600),
        Duration::ZERO,
    )
    .await;
    let (_, _, body) = get_page(t.app(), "/person/6384", false).await;
    assert!(!body.contains("stale-notice"), "fresh data has no notice");
    let calls = t.tmdb.calls();
    get_page(t.app(), "/person/6384", false).await;
    assert_eq!(t.tmdb.calls(), 2 * calls, "expired entries are refreshed");
    get_page(t.app(), "/tv/1396/season/1", false).await;

    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/person/6384", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Keanu Reeves"));
    assert!(body.contains("TMDB isn't responding, so this page shows details saved on "));

    let (_, json) = get_json(t.app(), "/api/person/6384").await;
    assert_eq!(json["metadata"]["state"], "stale");
    assert!(
        json["metadata"]["fetched_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    let (status, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("stale-notice"),
        "season page shows the notice too"
    );
    let (_, _, body) = get_page(t.app(), "/movie/6060", false).await;
    assert!(
        !body.contains("stale-notice"),
        "never cached, so it fails instead"
    );

    let (status, _, body) = get_page(t.app(), "/person/1", false).await;
    assert_eq!(
        status,
        StatusCode::BAD_GATEWAY,
        "nothing cached to fall back on"
    );
    assert!(body.contains("Couldn&#39;t load this person"));
}

#[tokio::test]
async fn titles_are_added_to_and_removed_from_the_watchlist_while_browsing() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<a href="/watchlist">To watch</a>"#),
        "linked from the header"
    );
    assert!(body.contains("Nothing saved yet."));
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"<form class="watch-toggle watchlist-toggle" id="watchlist-toggle" method="post" action="/movie/603/watchlist""#));
    assert!(body.contains(r#"aria-pressed="false" aria-label="To watch: The Matrix""#));

    let (status, headers) = post_form(
        t.app(),
        "/movie/603/watchlist",
        "listed=true&back=%2Fmovie%2F603",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/movie/603");
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="To watch: The Matrix""#));

    // From a filmography row, back to that row.
    let (_, headers) = post_form(
        t.app(),
        "/tv/1396/watchlist",
        "listed=true&back=%2Fperson%2F6384%3Fkind%3Dtv%23watchlist-tv-1396",
    )
    .await;
    assert_eq!(
        headers[header::LOCATION],
        "/person/6384?kind=tv#watchlist-tv-1396"
    );
    let (_, _, body) = get_page(t.app(), "/person/6384", false).await;
    assert!(body.contains(r##"id="watchlist-movie-603" method="post" action="/movie/603/watchlist" hx-post="/movie/603/watchlist" hx-target="#watchlist-movie-603" hx-select="#watchlist-movie-603""##));
    assert!(
        body.contains(
            r#"<input type="hidden" name="back" value="/person/6384#watchlist-movie-603">"#
        )
    );
    assert!(body.contains(r#"aria-pressed="true" aria-label="To watch: The Matrix""#));
    assert!(body.contains(r#"aria-pressed="false" aria-label="To watch: The Tonight Show""#));

    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    let bb = body.find(r#"<a href="/tv/1396">Breaking Bad</a>"#).unwrap();
    let matrix = body.find(r#"<a href="/movie/603">The Matrix</a>"#).unwrap();
    assert!(bb < matrix, "most recently added first");
    assert!(body.contains(
        r#"<a href="/watchlist" aria-current="true">All <span class="count">2</span></a>"#
    ));
    assert!(
        body.contains(r#"<a href="/watchlist?kind=movie">Movies <span class="count">1</span></a>"#)
    );
    assert!(
        body.contains(r#"<img class="poster thumb" src="https://img.test/t/p/w92/matrix.jpg""#)
    );

    let (_, _, body) = get_page(t.app(), "/watchlist?kind=tv", false).await;
    assert!(body.contains("Breaking Bad") && !body.contains("The Matrix</a>"));
    assert!(body.contains(r#"<a href="/watchlist?sort=rating&#38;kind=tv">Rating</a>"#));
    let (_, json) = get_json(t.app(), "/api/watchlist?kind=movie").await;
    assert_eq!(json["sort"], "added");
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
    assert_eq!(json["titles"][0]["title"], "The Matrix");
    assert_eq!(json["titles"][0]["year"], "1999");

    // Removing from the list page goes back to it; other sites are refused.
    let (_, headers) = post_form(
        t.app(),
        "/movie/603/watchlist",
        "listed=false&back=%2Fwatchlist%3Fkind%3Dmovie",
    )
    .await;
    assert_eq!(headers[header::LOCATION], "/watchlist?kind=movie");
    let (_, _, body) = get_page(t.app(), "/watchlist?kind=movie", false).await;
    assert!(body.contains("No movies on your list."));
    for back in [
        "https%3A%2F%2Fexample.com",
        "%2F%2Fexample.com",
        "%2F%5Cexample.com",
    ] {
        let (_, headers) = post_form(
            t.app(),
            "/tv/1396/watchlist",
            &format!("listed=false&back={back}"),
        )
        .await;
        assert_eq!(headers[header::LOCATION], "/tv/1396");
    }
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(json["titles"], json!([]));
}

#[tokio::test]
async fn favorites_are_independent_filterable_and_toggle_while_browsing() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/favorites", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<a href="/favorites">Favorites</a>"#));
    assert!(body.contains("No favorites yet."));
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"aria-pressed="false" aria-label="Favorite: The Matrix""#));
    assert!(body.contains(r##"hx-post="/movie/603/favorite" hx-target="#favorite-toggle" hx-select="#favorite-toggle""##));
    assert!(body.contains("hx-on::before-on-load="));
    assert!(body.contains(r#"role="alert" hidden>Couldn't update Favorites. Please try again."#));

    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let (status, headers) = post_form(
        t.app(),
        "/tv/1396/favorite",
        "listed=true&back=%2Fperson%2F6384%3Fkind%3Dtv%23favorite-tv-1396",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        headers[header::LOCATION],
        "/person/6384?kind=tv#favorite-tv-1396"
    );
    // Repeated requests keep one entry and its original position.
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"].as_array().unwrap().len(), 2);
    assert_eq!(json["titles"][0]["title"], "Breaking Bad");
    assert_eq!(
        json["titles"][1]["poster_url"],
        "https://img.test/t/p/w92/matrix.jpg"
    );
    let (_, watchlist) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(watchlist["titles"], json!([]));

    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: The Matrix""#));
    assert!(body.contains(r#"aria-pressed="false" aria-label="To watch: The Matrix""#));
    let (_, _, body) = get_page(t.app(), "/person/6384", true).await;
    assert!(body.contains(r##"id="favorite-movie-603" method="post" action="/movie/603/favorite" hx-post="/movie/603/favorite" hx-target="#favorite-movie-603" hx-select="#favorite-movie-603""##));
    assert!(body.contains(r#"name="back" value="/person/6384#favorite-movie-603""#));
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: The Matrix""#));

    let (_, _, body) = get_page(t.app(), "/favorites?sort=rating&kind=movie", false).await;
    assert!(body.contains("The Matrix</a>") && !body.contains("Breaking Bad</a>"));
    assert!(body.contains(
        r#"<a href="/favorites?sort=rating&#38;kind=movie" aria-current="true">Rating</a>"#
    ));
    assert!(body.contains(r#"<a href="/favorites?kind=movie">Added</a>"#));
    assert!(body.contains(
        r#"<a href="/favorites?sort=rating&#38;kind=tv">TV <span class="count">1</span></a>"#
    ));
    assert!(body.contains(r#"name="back" value="/favorites?sort=rating&#38;kind=movie""#));
    let (_, filtered) = get_json(t.app(), "/api/favorites?kind=tv").await;
    assert_eq!(filtered["titles"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["titles"][0]["kind"], "tv");
    let (_, fallback) = get_json(t.app(), "/api/favorites?kind=unknown&sort=unknown").await;
    assert_eq!(fallback["sort"], "added");
    assert_eq!(fallback["titles"].as_array().unwrap().len(), 2);

    // Each list offers both toggles, without changing membership in the other.
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: The Matrix""#));
    let (_, _, body) = get_page(t.app(), "/favorites", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="To watch: The Matrix""#));
    let (_, headers) = post_form(
        t.app(),
        "/movie/603/favorite",
        "listed=false&back=%2Ffavorites%3Fkind%3Dmovie",
    )
    .await;
    assert_eq!(headers[header::LOCATION], "/favorites?kind=movie");
    let (_, _, body) = get_page(t.app(), "/favorites?kind=movie", false).await;
    assert!(body.contains("No movies on your list."));
    let (_, watchlist) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(watchlist["titles"][0]["id"], 603);

    for back in [
        "https%3A%2F%2Fexample.com",
        "%2F%2Fexample.com",
        "%2F%5Cexample.com",
    ] {
        let (_, headers) = post_form(
            t.app(),
            "/tv/1396/favorite",
            &format!("listed=false&back={back}"),
        )
        .await;
        assert_eq!(headers[header::LOCATION], "/tv/1396");
    }
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"], json!([]));
}

#[tokio::test]
async fn favoriting_a_show_watches_current_seasons_but_not_the_whole_show() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    post_form(t.app(), "/tv/1396/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    assert!(t.catalog.watched(1396).await.seasons.is_empty());
    assert!(t.catalog.watched(603).await.seasons.is_empty());
    post_form(
        t.app(),
        "/tv/1396/season/2/episode/1/watched",
        "watched=true",
    )
    .await;

    let (status, headers) = post_form(
        t.app(),
        "/tv/1396/favorite",
        "listed=true&back=%2F%3Fq%3Dbreaking%23favorite-tv-1396",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/?q=breaking#favorite-tv-1396");
    let watched = t.catalog.watched(1396).await;
    assert!(!watched.show, "no blanket mark covering future seasons");
    assert_eq!(
        watched.seasons,
        [0, 1, 2],
        "includes specials and undated seasons"
    );
    assert_eq!(watched.episodes, [(2, 1)], "keeps existing episode marks");
    assert!(!watched.season(3), "new seasons remain unwatched");
    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(body.matches("✓ Watched").count(), 3);
    assert!(
        body.contains(r#"hx-target="closest article" hx-select="article.detail""#),
        "TV detail swaps the article so season badges update with the heart"
    );
    let (_, _, body) = get_page(t.app(), "/tv/1396/season/1", false).await;
    assert!(
        !body.contains(r#"class="episode spoiler""#),
        "watched season reveals spoilers"
    );

    post_form(t.app(), "/tv/1396/favorite", "listed=true").await;
    assert_eq!(
        t.catalog.watched(1396).await,
        watched,
        "repeated add is idempotent"
    );
    t.tmdb.set_down(true);
    let (status, _) = post_form(t.app(), "/tv/1396/favorite", "listed=false").await;
    assert_eq!(status, StatusCode::SEE_OTHER, "removal needs no provider");
    assert_eq!(
        t.catalog.watched(1396).await,
        watched,
        "unheart keeps watched history"
    );
    let (_, list) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(list["titles"][0]["id"], 1396);
}

#[tokio::test]
async fn favorite_ratings_and_provider_failures_are_explicit() {
    let t = build_with(TOKEN, Some(OMDB_KEY), Duration::ZERO, Duration::ZERO).await;
    post_form(t.app(), "/tv/1396/favorite", "listed=true").await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let (_, json) = get_json(t.app(), "/api/favorites?sort=rating").await;
    assert_eq!(json["sort"], "rating");
    assert_eq!(json["titles"][0]["title"], "Breaking Bad");
    assert_eq!(json["titles"][1]["rating"]["source"], "rotten_tomatoes");
    t.tmdb.set_down(true);
    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/favorites?sort=rating", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Couldn't refresh some ratings"));
    // Removing never needs an upstream lookup.
    let (status, _) = post_form(t.app(), "/movie/603/favorite", "listed=false").await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let (_, _, body) = get_page(t.app(), "/favorites?sort=rating", false).await;
    assert!(body.contains("Ratings aren't configured on this server"));
    for uri in ["/movie/1/favorite", "/movie/not-an-id/favorite"] {
        let (status, _) = post_form(t.app(), uri, "listed=true").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    let (status, _) = post_form(t.app(), "/tv/500/favorite", "listed=true").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let (status, _) = post_form(t.app(), "/movie/603/favorite", "listed=invalid").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn favorites_persist_and_storage_failures_never_look_empty() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "credits-favorites-{}-{unique}.db",
        std::process::id()
    ));
    let database = path.to_str().unwrap();
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    post_form(t.app(), "/tv/1396/favorite", "listed=true").await;
    drop(t);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"][0]["id"], 1396);
    assert_eq!(json["titles"][1]["id"], 603);
    assert_eq!(t.catalog.watched(1396).await.seasons, [0, 1, 2]);
    assert!(!t.catalog.watched(1396).await.show);
    let pool = credits::cache::ProviderCache::open(database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE favorites RENAME TO favorites_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, body) = get_page(t.app(), "/favorites", false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Couldn&#39;t load your Favorites"));
    assert!(!body.contains("No favorites yet."));
    let (status, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json["error"].as_str().unwrap().contains("couldn't be read"));
    for uri in ["/movie/603", "/person/6384", "/watchlist", "/?q=matrix"] {
        let (status, _, body) = get_page(t.app(), uri, false).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Favorites couldn't be read."));
        assert!(body.contains(r#"disabled aria-label="Favorites unavailable: The Matrix""#));
        assert!(!body.contains(r#"aria-pressed="false" aria-label="Favorite: The Matrix""#));
    }
    for listed in ["true", "false"] {
        let (status, _) =
            post_form(t.app(), "/movie/603/favorite", &format!("listed={listed}")).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }
    // A failure partway through season writes must roll back the favorite
    // and every season mark, while leaving the existing show untouched.
    sqlx::query("ALTER TABLE favorites_unavailable RENAME TO favorites")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER reject_second_season BEFORE INSERT ON watched_seasons
         WHEN NEW.tv_id = 999 AND NEW.season = 1
         BEGIN SELECT RAISE(ABORT, 'season storage failed'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let response = t
        .app()
        .oneshot(
            Request::post("/tv/999/favorite")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("listed=true"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(
        String::from_utf8(body.to_vec())
            .unwrap()
            .contains("Couldn&#39;t update your Favorites")
    );
    assert!(t.catalog.watched(999).await.seasons.is_empty());
    assert!(
        !t.catalog
            .favorite_titles()
            .await
            .unwrap()
            .contains(&(credits::model::MediaKind::Tv, 999))
    );
    assert_eq!(t.catalog.watched(1396).await.seasons, [0, 1, 2]);
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"aria-pressed="true" aria-label="Favorite: The Matrix""#));
    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

async fn put_json(app: Router, uri: &str, body: Value) -> (StatusCode, Value) {
    send_json(app, Request::put(uri), body).await
}

async fn post_json(app: Router, uri: &str, body: Value) -> (StatusCode, Value) {
    send_json(app, Request::post(uri), body).await
}

async fn send_json(
    app: Router,
    request: axum::http::request::Builder,
    body: Value,
) -> (StatusCode, Value) {
    let request = request
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn json_writes_mirror_the_forms_and_answer_with_the_new_state() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(
        json["marks"],
        json!({ "to_watch": false, "favorite": false, "watched": false })
    );

    let (status, json) = put_json(
        t.app(),
        "/api/movie/603/watchlist",
        json!({ "listed": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json,
        json!({ "marks": { "to_watch": true, "favorite": false, "watched": false } }),
        "movies carry no show marks"
    );
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["marks"]["to_watch"], true);

    // Favoriting a show marks its aired seasons, as the form does.
    put_json(
        t.app(),
        "/api/tv/1396/season/2/episode/1/watched",
        json!({ "watched": true }),
    )
    .await;
    let (status, json) =
        put_json(t.app(), "/api/tv/1396/favorite", json!({ "listed": true })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["marks"],
        json!({ "to_watch": false, "favorite": true, "watched": false })
    );
    assert_eq!(json["watched"]["seasons"], json!([0, 1, 2]));
    assert_eq!(json["watched"]["episodes"][0]["season"], 2);
    assert!(!t.catalog.watched(1396).await.show);

    // Lists carry each row's marks. Watching takes a title off To watch;
    // adding it back keeps both.
    let (_, json) = put_json(
        t.app(),
        "/api/movie/603/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(json["marks"]["to_watch"], false);
    put_json(
        t.app(),
        "/api/movie/603/watchlist",
        json!({ "listed": true }),
    )
    .await;
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(
        json["titles"][0]["marks"],
        json!({ "to_watch": true, "favorite": false, "watched": true })
    );
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(
        json["titles"][0]["marks"],
        json!({ "to_watch": false, "favorite": true, "watched": false })
    );
    let (_, json) = get_json(t.app(), "/api/tv/1396").await;
    assert_eq!(json["marks"]["favorite"], true);

    // Unhearting keeps watched history; removing needs no provider.
    t.tmdb.set_down(true);
    let (status, json) =
        put_json(t.app(), "/api/tv/1396/favorite", json!({ "listed": false })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["marks"]["favorite"], false);
    assert_eq!(json["watched"]["seasons"], json!([0, 1, 2]));
    let (_, json) = put_json(
        t.app(),
        "/api/movie/603/watchlist",
        json!({ "listed": false }),
    )
    .await;
    assert_eq!(json["marks"]["to_watch"], false);
    t.tmdb.set_down(false);

    let (_, json) = put_json(t.app(), "/api/tv/1396/watched", json!({ "watched": true })).await;
    assert_eq!(json["marks"]["watched"], true);
    assert_eq!(json["watched"]["show"], true);
    let (status, json) = put_json(
        t.app(),
        "/api/tv/1396/season/3/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["state"], "read");
    assert_eq!(json["seasons"], json!([0, 1, 2, 3]));
    let (_, json) = put_json(
        t.app(),
        "/api/tv/1396/season/2/episode/1/watched",
        json!({ "watched": false }),
    )
    .await;
    assert_eq!(json["episodes"], json!([]));
    let (_, json) = put_json(
        t.app(),
        "/api/movie/603/watched",
        json!({ "watched": false }),
    )
    .await;
    assert_eq!(json["marks"]["watched"], false);

    // Failures answer with the API's error body.
    for uri in ["/api/movie/1/favorite", "/api/movie/not-an-id/watchlist"] {
        let (status, json) = put_json(t.app(), uri, json!({ "listed": true })).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(json["error"].is_string());
    }
    let (status, json) = put_json(t.app(), "/api/tv/500/favorite", json!({ "listed": true })).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(json["error"].as_str().unwrap().contains("TMDB"));
    let (status, json) = put_json(
        t.app(),
        "/api/movie/603/favorite",
        json!({ "listed": "maybe" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(json["error"].is_string());
    let (status, _) = put_json(
        t.app(),
        "/api/tv/1396/season/x/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn json_writes_are_refused_when_storage_fails() {
    let database = temp_database("api-writes");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    for table in ["favorites", "watched_movies", "watched_seasons"] {
        sqlx::query(&format!(
            "ALTER TABLE {table} RENAME TO {table}_unavailable"
        ))
        .execute(&pool)
        .await
        .unwrap();
    }
    for listed in [true, false] {
        let (status, json) = put_json(
            t.app(),
            "/api/movie/603/favorite",
            json!({ "listed": listed }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            json["error"]
                .as_str()
                .unwrap()
                .contains("nothing was saved"),
            "{json}"
        );
    }
    let (status, json) = put_json(
        t.app(),
        "/api/movie/603/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(json["error"].as_str().unwrap().contains("watched marks"));
    let (status, _) = put_json(
        t.app(),
        "/api/tv/1396/season/1/watched",
        json!({ "watched": true }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    // Unreadable marks are null, never passed off as false.
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(
        json["marks"],
        json!({ "to_watch": false, "favorite": null, "watched": null })
    );
    let (status, json) = put_json(
        t.app(),
        "/api/movie/603/watchlist",
        json!({ "listed": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "To watch still works");
    assert_eq!(json["marks"]["favorite"], Value::Null);
    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn watchlist_sorts_by_rating_and_reports_problems() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    post_form(t.app(), "/tv/1396/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let (_, json) = get_json(t.app(), "/api/watchlist?sort=rating").await;
    assert_eq!(json["sort"], "rating");
    // Breaking Bad's IMDb 9.5 (95%) beats The Matrix's 83% Tomatometer.
    assert_eq!(json["titles"][0]["title"], "Breaking Bad");
    assert_eq!(json["titles"][1]["rating"]["source"], "rotten_tomatoes");
    let (_, _, body) = get_page(t.app(), "/watchlist?sort=rating", false).await;
    assert!(body.contains(r#"<a href="/watchlist?sort=rating" aria-current="true">Rating</a>"#));
    assert!(body.contains(r#"aria-label="Rotten Tomatoes: Fresh, 83%""#));
    assert!(!body.contains("listed last"), "every title is scored");

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let (_, _, body) = get_page(t.app(), "/watchlist?sort=rating", false).await;
    assert!(body.contains(
        "Ratings aren't configured on this server, so titles stay in the order they were added."
    ));

    // Titles TMDB doesn't know or can't load aren't saved.
    let (status, _) = post_form(t.app(), "/movie/1/watchlist", "listed=true").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, body) = {
        let request = Request::post("/tv/500/watchlist")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("listed=true"))
            .unwrap();
        let response = t.app().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, (), String::from_utf8(body.to_vec()).unwrap())
    };
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(body.contains("Couldn&#39;t add to your To watch list"));
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
}

fn temp_database(name: &str) -> String {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("credits-{name}-{}-{unique}.db", std::process::id()))
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn title_pages_show_where_to_watch_in_the_region() {
    let (_, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert!(body.contains(r#"<h2 id="where-to-watch-heading">Where to watch <span class="muted region">in US</span></h2>"#));
    assert!(
        body.contains(
            r#"<span class="muted offer-summary">2 to stream · 1 to rent</span></summary>"#
        ),
        "collapsed behind a summary of the offers"
    );
    assert!(!body.contains("<details open"));
    let stream = body.find("<h3>Stream</h3>").unwrap();
    let rent = body.find("<h3>Rent</h3>").unwrap();
    assert!(stream < rent, "subscription offers come first");
    let max = body.find("Max</span>").unwrap();
    let netflix = body.find("Netflix</span>").unwrap();
    assert!(max < netflix, "services in TMDB's display priority order");
    assert_eq!(
        body.matches("Netflix</span>").count(),
        1,
        "duplicates dropped"
    );
    assert!(body.contains(r#"<img src="https://img.test/t/p/w92/max.jpg""#));
    assert!(
        body.contains(r#"<span class="provider-name">Apple TV</span>"#),
        "named even without a logo"
    );
    assert!(
        body.contains(r#"href="https://www.themoviedb.org/movie/603-the-matrix/watch?locale=US""#)
    );
    assert!(body.contains(r#"Availability from <a href="https://www.justwatch.com/""#));
    assert!(
        !body.contains("Amazon Video"),
        "other regions are not shown"
    );

    let (_, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert!(body.contains("<h3>Free with ads</h3>"));
    assert!(body.contains("Pluto TV"));

    let (_, _, body) = get_page(test_app().await, "/movie/6060", false).await;
    assert!(body.contains("No streaming, rental or purchase options are listed for US."));
    assert!(body.contains("Availability from"));

    let (_, json) = get_json(test_app().await, "/api/movie/603").await;
    assert_eq!(
        json["where_to_watch"]["link"],
        "https://www.themoviedb.org/movie/603-the-matrix/watch?locale=US"
    );
    assert_eq!(json["where_to_watch"]["region"], "US");
    assert_eq!(json["where_to_watch"]["state"], "fresh");
    assert!(json["where_to_watch"]["fetched_at"].is_string());
    assert_eq!(
        json["where_to_watch"]["offers"],
        json!([
            { "kind": "stream", "providers": [
                { "id": 1899, "name": "Max", "logo_url": "https://img.test/t/p/w92/max.jpg", "mine": false },
                { "id": 8, "name": "Netflix", "logo_url": "https://img.test/t/p/w92/netflix.jpg", "mine": false }
            ] },
            { "kind": "rent", "providers": [{ "id": 2, "name": "Apple TV", "logo_url": null, "mine": false }] }
        ])
    );
    assert_eq!(json["where_to_watch"]["my_services"], "not_chosen");

    let ttl = Duration::from_secs(3600);
    let config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(TOKEN.to_owned()),
        "CREDITS_DB" => Some(IN_MEMORY.to_owned()),
        "CREDITS_REGION" => Some("gb".to_owned()),
        _ => None,
    })
    .unwrap();
    let t = build_from(config, ttl, ttl).await;
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"<span class="muted region">in GB</span>"#));
    assert!(body.contains("<h3>Buy</h3>"));
    assert!(body.contains("Amazon Video"));
    assert!(!body.contains("Netflix"));
}

#[tokio::test]
async fn where_to_watch_survives_tmdb_outages_and_older_cache_entries() {
    let database = temp_database("where-to-watch");
    let t = build_with(
        TOKEN,
        Some(OMDB_KEY),
        Duration::from_secs(3600),
        Duration::ZERO,
    )
    .await;
    get_page(t.app(), "/movie/603", false).await;
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Netflix"), "expired offers are still shown");
    assert!(body.contains("Couldn't refresh where to watch; showing what was listed on "));
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["where_to_watch"]["state"], "stale");

    // Entries cached before where to watch was requested lack it.
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    get_page(t.app(), "/movie/603", false).await;
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    let drop_providers =
        r#"UPDATE provider_cache SET payload = json_remove(payload, '$."watch/providers"')"#;
    sqlx::query(drop_providers).execute(&pool).await.unwrap();
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(
        t.tmdb.calls(),
        2,
        "a fresh entry without offers is refetched"
    );
    assert!(body.contains("Netflix"));
    get_page(t.app(), "/movie/603", false).await;
    assert_eq!(t.tmdb.calls(), 2, "and then served from the cache");

    sqlx::query(drop_providers).execute(&pool).await.unwrap();
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("stale-notice"));
    assert!(body.contains("Where to watch couldn't be loaded right now."));
    assert!(!body.contains("Availability from"));
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["where_to_watch"]["state"], "unavailable");
    assert_eq!(json["where_to_watch"]["offers"], json!([]));

    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn movie_pages_show_release_dates_in_the_region() {
    let (_, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert!(body.contains("<dt>In theaters</dt><dd>Mar 31, 1999</dd>"));
    assert!(body.contains("<dt>Digital</dt><dd>Sep 21, 1999</dd>"));
    assert!(body.contains("<dt>Physical</dt><dd>Sep 21, 1999</dd>"));
    assert!(body.contains("<dt>Rated</dt><dd>R</dd>"));
    assert!(
        !body.contains("Jun 11, 1999"),
        "other regions are not shown"
    );

    let (_, _, body) = get_page(test_app().await, "/movie/4040", false).await;
    assert!(body.contains(r#"<dt>In theaters</dt><dd class="upcoming">Coming Oct 16, 2099</dd>"#));
    assert!(!body.contains("<dt>Rated</dt>"));

    let (_, _, body) = get_page(test_app().await, "/movie/6060", false).await;
    assert!(
        body.contains("<dt>Released</dt><dd>Mar 31, 1999</dd>"),
        "TMDB's primary date when the region has none"
    );
    assert!(!body.contains("<dt>In theaters</dt>"));

    let (_, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert!(!body.contains("<dt>Released</dt>"), "movies only");

    let (_, json) = get_json(test_app().await, "/api/movie/603").await;
    assert_eq!(
        json["releases"],
        json!({ "region": "US", "certification": "R", "dates": [
            { "kind": "theatrical", "date": "1999-03-31", "upcoming": false },
            { "kind": "digital", "date": "1999-09-21", "upcoming": false },
            { "kind": "physical", "date": "1999-09-21", "upcoming": false }
        ] })
    );
    let (_, json) = get_json(test_app().await, "/api/movie/4040").await;
    assert_eq!(json["releases"]["dates"][0]["upcoming"], true);
    let (_, json) = get_json(test_app().await, "/api/tv/1396").await;
    assert_eq!(json["releases"], Value::Null);

    let ttl = Duration::from_secs(3600);
    let config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(TOKEN.to_owned()),
        "CREDITS_DB" => Some(IN_MEMORY.to_owned()),
        "CREDITS_REGION" => Some("gb".to_owned()),
        _ => None,
    })
    .unwrap();
    let t = build_from(config, ttl, ttl).await;
    let (_, _, body) = get_page(t.app(), "/movie/6060", false).await;
    assert!(body.contains("<dt>In theaters</dt><dd>Jun 11, 1999</dd>"));
    assert!(body.contains("<dt>Rated</dt><dd>15</dd>"));
}

#[tokio::test]
async fn movies_cached_before_release_dates_are_refetched_once() {
    let database = temp_database("release-dates");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    get_page(t.app(), "/movie/603", false).await;
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("UPDATE provider_cache SET payload = json_remove(payload, '$.release_dates')")
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(
        t.tmdb.calls(),
        2,
        "a fresh entry without dates is refetched"
    );
    assert!(body.contains("<dt>In theaters</dt><dd>Mar 31, 1999</dd>"));
    get_page(t.app(), "/movie/603", false).await;
    assert_eq!(t.tmdb.calls(), 2, "and then served from the cache");

    sqlx::query("UPDATE provider_cache SET payload = json_remove(payload, '$.release_dates')")
        .execute(&pool)
        .await
        .unwrap();
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert_eq!(status, StatusCode::OK, "the page survives without dates");
    assert!(
        body.contains("<dt>Released</dt><dd>Mar 31, 1999</dd>"),
        "falls back to the primary date"
    );

    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn movie_pages_mark_movies_in_theaters() {
    let badge = r#"<span class="badge in-theaters">In theaters</span>"#;
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;

    let (_, _, body) = get_page(t.app(), "/movie/7001", false).await;
    assert!(body.contains(&format!(
        r#"<span class="badge badge-movie">Movie</span> {badge}"#
    )));
    assert!(
        body.contains(&format!(
            r#"<p class="in-theaters-note">{badge} Not streaming or for rent or sale in US yet.</p>"#
        )),
        "where to watch says so rather than looking empty"
    );
    assert!(!body.contains("No streaming, rental or purchase options"));
    assert_eq!(
        t.tmdb.now_playing_calls(),
        0,
        "opened 10 days ago: the dates decide"
    );

    let (_, _, body) = get_page(t.app(), "/movie/7002", false).await;
    assert!(
        body.contains(badge),
        "still on now playing, on its second page"
    );
    assert_eq!(t.tmdb.now_playing_calls(), 2, "every page is read");

    for (id, why) in [
        (7003, "out on digital, though now playing"),
        (7004, "opened 60 days ago and no longer playing"),
        (603, "long since out"),
    ] {
        let (_, _, body) = get_page(t.app(), &format!("/movie/{id}"), false).await;
        assert!(!body.contains(badge), "{id}: {why}");
    }
    let (_, _, body) = get_page(t.app(), "/movie/7004", false).await;
    assert!(body.contains("No streaming, rental or purchase options are listed for US."));
    assert_eq!(t.tmdb.now_playing_calls(), 2, "the list is cached");

    let (_, json) = get_json(t.app(), "/api/movie/7001").await;
    assert_eq!(json["in_theaters"], true);
    let (_, json) = get_json(t.app(), "/api/movie/7003").await;
    assert_eq!(json["in_theaters"], false);
    let (_, json) = get_json(t.app(), "/api/tv/1396").await;
    assert_eq!(json["in_theaters"], false);
}

#[tokio::test]
async fn movies_in_theaters_link_to_showtimes() {
    let link = "https://www.fandango.com/search?q=Opening%20Night&mode=all";
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;

    let (_, _, body) = get_page(t.app(), "/movie/7001", false).await;
    assert!(
        body.contains(
            r#"<a class="showtimes-link" href="https://www.fandango.com/search?q=Opening%20Night&#38;mode=all" rel="noopener">Showtimes</a>"#
        ),
        "{body}"
    );

    for id in [7003, 7004, 603] {
        let (_, _, body) = get_page(t.app(), &format!("/movie/{id}"), false).await;
        assert!(!body.contains("showtimes-link"), "{id}: not in theaters");
    }

    let (_, json) = get_json(t.app(), "/api/movie/7001").await;
    assert_eq!(json["showtimes_url"], link);
    let (_, json) = get_json(t.app(), "/api/movie/7003").await;
    assert_eq!(json["showtimes_url"], Value::Null);
}

#[tokio::test]
async fn movies_are_judged_by_their_dates_when_now_playing_fails() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    // Cache both movies without loading now playing.
    get_page(t.app(), "/movie/7001/credits", false).await;
    get_page(t.app(), "/movie/7002/credits", false).await;
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/movie/7002", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("in-theaters"), "now playing is unknown");
    let (_, _, body) = get_page(t.app(), "/movie/7001", false).await;
    assert!(body.contains("in-theaters"), "the dates still say so");
}

#[tokio::test]
async fn search_and_saved_list_rows_mark_movies_in_theaters() {
    let badge = r#"<span class="badge in-theaters">In theaters</span>"#;
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, _, body) = get_page(t.app(), "/?q=theaters", false).await;
    let rows: Vec<&str> = body.split(r#"<li class="result"#).skip(1).collect();
    let marked: Vec<bool> = rows.iter().map(|r| r.contains(badge)).collect();
    assert_eq!(marked, [true, true, false, false, false], "{body}");

    let (_, json) = get_json(t.app(), "/api/search?q=theaters").await;
    let marked: Vec<(u64, bool)> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["id"].as_u64().unwrap(),
                h["in_theaters"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        marked,
        [
            (7001, true),
            (7002, true),
            (7003, false),
            (7004, false),
            (603, false)
        ]
    );

    for id in [7001, 7004, 603] {
        post_form(t.app(), &format!("/movie/{id}/watchlist"), "listed=true").await;
        post_form(t.app(), &format!("/movie/{id}/favorite"), "listed=true").await;
    }
    for list in ["watchlist", "favorites"] {
        let (_, _, body) = get_page(t.app(), &format!("/{list}"), false).await;
        assert_eq!(body.matches(badge).count(), 1, "{list}: {body}");
        assert!(
            body.contains(&format!(
                r#">Opening Night</a> <span class="badge badge-movie">Movie</span> {badge}"#
            )),
            "{list}"
        );
        let (_, json) = get_json(t.app(), &format!("/api/{list}")).await;
        let marked: Vec<(u64, bool)> = json["titles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["id"].as_u64().unwrap(), e.get("in_theaters").is_some()))
            .collect();
        assert_eq!(
            marked,
            [(603, false), (7004, false), (7001, true)],
            "{list}: only movies in theaters carry it"
        );
    }
}

fn chosen_names(json: &Value) -> Vec<&str> {
    json["chosen"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn services_are_chosen_and_highlighted_on_title_pages() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/services", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<a href="/services">My services</a>"#),
        "linked from the footer"
    );
    let footer = &body[body.find("<footer").unwrap()..];
    assert!(footer.contains(r#"<a href="/services">My services</a>"#));
    let header = &body[..body.find("</header>").unwrap()];
    assert!(!header.contains(r#"href="/services""#));
    assert!(body.contains("No services chosen yet."));
    let at = |needle: &str| body.find(needle).unwrap();
    let order = [
        at(r#"<h2 class="service-group">Popular</h2>"#),
        at(r#"value="1899">"#), // Max
        at(r#"value="8">"#),    // Netflix
        at(r#"<h2 class="service-group">More services</h2>"#),
        at(r#"value="2">"#),   // Apple TV
        at(r#"value="300">"#), // Pluto TV
    ];
    assert!(
        order.is_sorted(),
        "the major US services, then the rest, each A to Z"
    );
    assert!(!body.contains(r#"value="204""#), "add-on channels hidden");
    assert!(!body.contains("Your services</h2>"));
    assert_eq!(body.matches(r#"value="1899">"#).count(), 1, "listed once");
    assert!(body.contains(r#"<img src="https://img.test/t/p/w92/netflix.jpg""#));

    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(
        r#"Choose <a href="/services">your streaming services</a> to highlight them here."#
    ));
    assert!(!body.contains("On your services"));

    // Unknown IDs are ignored.
    let (status, headers) = post_form(
        t.app(),
        "/services",
        "service=8&service=300&service=99999&service=x",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/services");
    let (_, json) = get_json(t.app(), "/api/services").await;
    assert_eq!(chosen_names(&json), ["Netflix", "Pluto TV"]);
    assert_eq!(json["state"], "fresh");
    assert_eq!(json["complete"], true);
    assert_eq!(json["options"][0]["name"], "Netflix");
    assert_eq!(json["options"][0]["chosen"], true);
    assert_eq!(json["options"][0]["group"], "mine");
    assert_eq!(json["options"][1]["name"], "Pluto TV");
    assert_eq!(json["options"][2]["name"], "Max");
    assert_eq!(json["options"][2]["chosen"], false);
    assert_eq!(json["options"][2]["group"], "popular");
    assert_eq!(json["options"][3]["name"], "Apple TV");
    assert_eq!(json["options"][3]["group"], "more");
    assert_eq!(json["options"].as_array().unwrap().len(), 4);
    let (_, _, body) = get_page(t.app(), "/services", false).await;
    assert!(body.contains(r#"<input type="checkbox" name="service" value="8" checked>"#));
    assert!(body.contains(r#"<input type="checkbox" name="service" value="1899">"#));
    assert!(
        body.find(r#"<h2 class="service-group">Your services</h2>"#)
            .unwrap()
            < body.find(r#"value="8" checked>"#).unwrap(),
        "your services come first, under their own heading"
    );
    assert!(body.contains("Your services: <strong>Netflix</strong>, <strong>Pluto TV</strong>."));
    assert!(!body.contains("Saved."), "only after an HTMX save");
    assert!(body.contains(r##"hx-post="/services" hx-trigger="change""##));
    assert!(body.contains(r#"<noscript><div class="service-save"><button type="submit">Save services</button></div></noscript>"#));

    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains(r#"<span class="on-my-services">On your services: Netflix</span>"#));
    assert!(body.contains(r#"<a class="provider mine" href="#));
    assert_eq!(
        body.matches("provider mine").count(),
        1,
        "only Netflix is theirs"
    );
    assert!(body.contains(r#"<span class="visually-hidden">(your service)</span>"#));
    assert!(!body.contains("to highlight them here"));
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["where_to_watch"]["my_services"], "chosen");
    assert_eq!(
        json["where_to_watch"]["offers"][0]["providers"][1]["mine"],
        true
    );
    assert_eq!(
        json["where_to_watch"]["offers"][0]["providers"][0]["mine"],
        false
    );

    let (_, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert!(
        body.contains("On your services: Pluto TV"),
        "free with ads counts as included"
    );

    // HTMX saves on each change and gets the refreshed status back.
    let (status, body) = post_htmx_form(t.app(), "/services", "service=1899").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<div id="services-status" class="services-status" role="status">"#));
    assert!(
        body.contains(r#"Your services: <strong>Max</strong>. <span class="saved">Saved.</span>"#)
    );

    // An empty form clears the choice.
    post_form(t.app(), "/services", "").await;
    let (_, json) = get_json(t.app(), "/api/services").await;
    assert_eq!(json["chosen"], json!([]));
}

#[tokio::test]
async fn to_watch_filters_by_my_services_with_badges() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    for title in ["/movie/603", "/tv/1396", "/movie/6060"] {
        post_form(t.app(), &format!("{title}/watchlist"), "listed=true").await;
    }
    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert!(body.contains(r#"Choose <a href="/services">your streaming services</a> to see which titles you can watch on them."#));
    assert!(!body.contains("On my services"));
    assert!(!body.contains("service-badges"));
    let (_, _, body) = get_page(t.app(), "/favorites", false).await;
    assert!(!body.contains("your streaming services"), "To watch only");

    post_form(t.app(), "/services", "service=8&service=300").await;
    let calls = t.tmdb.calls();
    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert_eq!(t.tmdb.calls(), calls, "offers come from the cached titles");
    assert!(body.contains(
        r#"<a href="/watchlist?services=mine">On my services <span class="count">2</span></a>"#
    ));
    assert!(body.contains(
        r#"<a href="/watchlist" aria-current="true">Anywhere <span class="count">3</span></a>"#
    ));
    assert!(body.contains(
        r#"<img class="service-badge" src="https://img.test/t/p/w92/netflix.jpg" alt="Netflix""#
    ));
    assert!(body.contains(r#"alt="Pluto TV""#));
    assert_eq!(body.matches(r#"<span class="service-badges">"#).count(), 2);

    let (_, _, body) = get_page(t.app(), "/watchlist?services=mine", false).await;
    assert!(body.contains("The Matrix</a>") && body.contains("Breaking Bad</a>"));
    assert_eq!(
        body.matches(r#"<a href="/movie/6060">"#).count(),
        0,
        "not on their services"
    );
    assert!(body.contains(
        r#"<a href="/watchlist?kind=movie&#38;services=mine">Movies <span class="count">1</span></a>"#
    ));
    assert!(body.contains(r#"<a href="/watchlist?sort=rating&#38;services=mine">Rating</a>"#));
    assert!(body.contains(r#"<input type="hidden" name="back" value="/watchlist?services=mine">"#));

    post_form(t.app(), "/services", "service=1899").await;
    let (_, _, body) = get_page(t.app(), "/watchlist?kind=tv&services=mine", false).await;
    assert!(body.contains("None of the TV shows on your list are on your services."));

    let (_, json) = get_json(t.app(), "/api/watchlist?services=mine&sort=rating").await;
    assert_eq!(json["services"]["state"], "chosen");
    assert_eq!(json["services"]["unknown"], 0);
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
    assert_eq!(json["titles"][0]["title"], "The Matrix");
    assert_eq!(json["titles"][0]["availability"]["state"], "fresh");
    assert_eq!(
        json["titles"][0]["availability"]["services"][0]["name"],
        "Max"
    );

    // Favorites never filter by service.
    post_form(t.app(), "/movie/6060/favorite", "listed=true").await;
    let (_, json) = get_json(t.app(), "/api/favorites?services=mine").await;
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
    assert!(json.get("services").is_none());
}

#[tokio::test]
async fn to_watch_rows_show_the_next_release_still_to_come() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    for title in [
        "/movie/7100",
        "/movie/7133",
        "/movie/7132",
        "/movie/603",
        "/tv/1396",
    ] {
        post_form(t.app(), &format!("{title}/watchlist"), "listed=true").await;
    }
    let today = jiff::Zoned::now().date();
    let on = |days: i64| today.checked_add(jiff::Span::new().days(days)).unwrap();
    let row = |body: &str, title: &str| -> String {
        let start = body.find(&format!(">{title}</a>")).expect(title);
        let end = start + body[start..].find("</li>").unwrap();
        body[start..end].to_owned()
    };

    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    let theatrical = format!(
        r#"<span class="next-release">In theaters {}</span>"#,
        credits::model::release_day(&on(1).to_string(), today)
    );
    assert!(row(&body, "Premiere 01").contains(&theatrical), "{body}");
    let digital = format!(
        r#"<span class="next-release">Digital {}</span>"#,
        credits::model::release_day(&on(12).to_string(), today)
    );
    assert!(row(&body, "Digital Next").contains(&digital), "{body}");
    for title in ["Already Out", "The Matrix", "Breaking Bad"] {
        assert!(
            !row(&body, title).contains("next-release"),
            "{title}: nothing still to come"
        );
    }

    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    let entries = json["titles"].as_array().unwrap();
    let next = |id: u64| {
        let entry = entries.iter().find(|e| e["id"] == id).expect("listed");
        entry.get("next_release").cloned()
    };
    assert_eq!(
        next(7100),
        Some(json!({ "kind": "theatrical", "date": on(1).to_string(), "upcoming": true }))
    );
    assert_eq!(
        next(7133),
        Some(json!({ "kind": "digital", "date": on(12).to_string(), "upcoming": true }))
    );
    assert_eq!(next(7132), None);
    assert_eq!(next(603), None);
    assert_eq!(next(1396), None, "TV has no theatrical or digital dates");

    let (_, _, body) = get_page(t.app(), "/favorites", false).await;
    assert!(!body.contains("next-release"), "To watch only");
}

#[tokio::test]
async fn movies_are_not_in_theaters_before_they_open() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, json) = get_json(t.app(), "/api/movie/7100").await;
    assert_eq!(
        json["in_theaters"], false,
        "now playing, but opens tomorrow"
    );
    let (_, _, body) = get_page(t.app(), "/movie/7100", false).await;
    assert!(!body.contains("badge in-theaters"), "{body}");
    assert!(t.tmdb.now_playing_calls() == 0, "its dates decide");
    let (_, json) = get_json(t.app(), "/api/movie/7001").await;
    assert_eq!(json["in_theaters"], true, "opened 10 days ago");
    let (_, json) = get_json(t.app(), "/api/movie/7002").await;
    assert_eq!(json["in_theaters"], true, "long run, still now playing");
}

#[tokio::test]
async fn to_watch_filters_by_out_now_and_coming_soon() {
    let database = temp_database("release-filter");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    // Coming soon: opens tomorrow; in theaters until digital in 12 days;
    // opened 3 days ago with no digital date; a show not aired yet.
    // Out now: on digital; gone from theaters; an old movie; an old show.
    for title in [
        "/movie/7100",
        "/movie/7133",
        "/movie/7132",
        "/tv/9100",
        "/movie/7003",
        "/movie/7004",
        "/movie/603",
        "/tv/1396",
    ] {
        post_form(t.app(), &format!("{title}/watchlist"), "listed=true").await;
    }
    let titles = |json: &Value| -> Vec<u64> {
        let mut ids: Vec<u64> = json["titles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_u64().unwrap())
            .collect();
        ids.sort();
        ids
    };

    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert!(body.contains(r#"<nav class="sort" aria-label="Filter by release">"#));
    assert!(body.contains(
        r#"<a href="/watchlist" aria-current="true">All <span class="count">8</span></a>"#
    ));
    assert!(body.contains(
        r#"<a href="/watchlist?release=out_now">Out now <span class="count">4</span></a>"#
    ));
    assert!(body.contains(
        r#"<a href="/watchlist?release=coming_soon">Coming soon <span class="count">4</span></a>"#
    ));
    assert!(!body.contains("Couldn't check the release dates"));

    let (_, _, body) = get_page(t.app(), "/watchlist?release=coming_soon&kind=movie", false).await;
    assert!(body.contains(">Premiere 01</a>") && body.contains(">Digital Next</a>"));
    assert!(
        body.contains(">Already Out</a>"),
        "in theaters, not on digital"
    );
    assert!(!body.contains(">The Matrix</a>") && !body.contains(">Gone Digital</a>"));
    assert!(body.contains(
        r#"<a href="/watchlist?kind=movie&#38;release=coming_soon" aria-current="true">Movies <span class="count">3</span></a>"#
    ));
    assert!(body.contains(
        r#"<a href="/watchlist?sort=rating&#38;kind=movie&#38;release=coming_soon">Rating</a>"#
    ));
    assert!(body.contains(
        r#"<input type="hidden" name="back" value="/watchlist?kind=movie&#38;release=coming_soon">"#
    ));

    let (_, json) = get_json(t.app(), "/api/watchlist?release=coming_soon").await;
    assert_eq!(titles(&json), [7100, 7132, 7133, 9100]);
    assert_eq!(json["release_unknown"], 0);
    let (_, json) = get_json(t.app(), "/api/watchlist?release=out_now").await;
    assert_eq!(titles(&json), [603, 1396, 7003, 7004]);
    assert!(
        json["titles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["release_status"] == "out_now")
    );

    // Favorites have no release filter.
    post_form(t.app(), "/movie/7100/favorite", "listed=true").await;
    let (_, _, body) = get_page(t.app(), "/favorites?release=out_now", false).await;
    assert!(!body.contains("Filter by release") && body.contains(">Premiere 01</a>"));
    let (_, json) = get_json(t.app(), "/api/favorites?release=out_now").await;
    assert_eq!(titles(&json), [7100]);

    // A movie whose dates can't be loaded is unknown, and left out of either filter.
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("DELETE FROM provider_cache WHERE key = 'movie/7004'")
        .execute(&pool)
        .await
        .unwrap();
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Couldn't check the release dates of 1 title right now."));
    assert!(body.contains(">Closed Early</a>"));
    let (_, _, body) = get_page(t.app(), "/watchlist?release=out_now", false).await;
    assert!(
        body.contains("Couldn't check the release dates of 1 title right now, so it's left out.")
    );
    assert!(!body.contains(">Closed Early</a>"));
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(json["release_unknown"], 1);
    let closed = json["titles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == 7004)
        .unwrap();
    assert!(closed.get("release_status").is_none());

    // Nothing matching says which filter left it all out.
    post_form(t.app(), "/tv/9100/watchlist", "listed=false").await;
    let (_, _, body) = get_page(t.app(), "/watchlist?kind=tv&release=coming_soon", false).await;
    assert!(
        body.contains("None of the TV shows on your list are coming soon."),
        "{body}"
    );
}

#[tokio::test]
async fn my_services_degrade_when_tmdb_or_storage_fail() {
    let database = temp_database("my-services");
    let t = build_with_database(
        TOKEN,
        Some(OMDB_KEY),
        Duration::from_secs(3600),
        Duration::ZERO,
        &database,
    )
    .await;
    post_form(t.app(), "/services", "service=8").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    post_form(t.app(), "/tv/1396/watchlist", "listed=true").await;

    // Expired offers and service lists are served stale.
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/services", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Couldn't refresh TMDB's list of services; showing the list from "));
    assert!(body.contains(r#"value="1899">"#));
    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert!(body.contains(
        "Couldn't refresh where some titles are available; using listings from as early as "
    ));
    assert!(body.contains(r#"<span class="service-badges stale" title="Listed on "#));

    // A title whose cached entry lacks offers is unknown, and left out of the filter.
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query(
        r#"UPDATE provider_cache SET payload = json_remove(payload, '$."watch/providers"') WHERE key = 'movie/603'"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let (_, _, body) = get_page(t.app(), "/watchlist", false).await;
    assert!(body.contains(r#"<span class="service-badges muted">Availability unknown</span>"#));
    assert!(body.contains("Couldn't check where 1 title is available right now."));
    let (_, _, body) = get_page(t.app(), "/watchlist?services=mine", false).await;
    assert!(
        body.contains("Couldn't check where 1 title is available right now, so it's left out.")
    );
    assert!(!body.contains("The Matrix</a>"));
    let (_, json) = get_json(t.app(), "/api/watchlist").await;
    assert_eq!(json["services"]["unknown"], 1);
    assert_eq!(
        json["titles"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["availability"]["state"] == "unavailable")
            .count(),
        1
    );

    // Without TMDB's list, the current choice can still be seen and removed.
    sqlx::query("DELETE FROM provider_cache WHERE key LIKE 'watch-providers/%'")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, body) = get_page(t.app(), "/services", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("TMDB's list of services couldn't be loaded right now, so only your current services are listed. You can still remove them."));
    assert!(body.contains(r#"<input type="checkbox" name="service" value="8" checked>"#));
    assert!(!body.contains(r#"value="1899""#));
    assert!(!body.contains("Services from"));
    let (_, json) = get_json(t.app(), "/api/services").await;
    assert_eq!(json["state"], "unavailable");
    post_form(t.app(), "/services", "service=8").await;
    let (_, json) = get_json(t.app(), "/api/services").await;
    assert_eq!(
        chosen_names(&json),
        ["Netflix"],
        "saving unchanged keeps it"
    );
    post_form(t.app(), "/services", "").await;
    let (_, _, body) = get_page(t.app(), "/services", false).await;
    assert!(body.contains("TMDB's list of services couldn't be loaded right now."));
    assert!(!body.contains("<form class=\"service-form\""));
    t.tmdb.set_down(false);
    post_form(t.app(), "/services", "service=8").await;

    // Unreadable services never look like none chosen, and are never saved over.
    sqlx::query("ALTER TABLE my_services RENAME TO my_services_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, body) = get_page(t.app(), "/services", false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Couldn&#39;t load your services"));
    let (status, _, body) = {
        let request = Request::post("/services")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("service=1899"))
            .unwrap();
        let response = t.app().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, (), String::from_utf8(body.to_vec()).unwrap())
    };
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("Your current services couldn&#39;t be read, so nothing was changed."));
    let (status, _) = get_json(t.app(), "/api/services").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
    assert!(body.contains("Your services couldn't be read, so none are highlighted."));
    assert!(!body.contains("provider mine"));
    let (_, json) = get_json(t.app(), "/api/movie/603").await;
    assert_eq!(json["where_to_watch"]["my_services"], "unreadable");
    let (_, _, body) = get_page(t.app(), "/watchlist?services=mine", false).await;
    assert!(body.contains("Your services couldn't be read, so titles can't be filtered by them."));
    assert!(body.contains("The Matrix</a>"), "unfiltered");
    sqlx::query("ALTER TABLE my_services_unavailable RENAME TO my_services")
        .execute(&pool)
        .await
        .unwrap();
    let (_, json) = get_json(t.app(), "/api/services").await;
    assert_eq!(
        chosen_names(&json),
        ["Netflix"],
        "unchanged by the failed save"
    );

    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

// Apple TV playback, read from a fake Home Assistant.

#[derive(Clone)]
struct FakeHa {
    rows: Arc<std::sync::Mutex<Vec<Value>>>,
    /// 0: answers; 401: rejects the token; other: that error status.
    failing: Arc<AtomicUsize>,
    /// `state_changed` event data for every WebSocket connection; `null`
    /// closes them.
    events: tokio::sync::broadcast::Sender<Value>,
}

impl Default for FakeHa {
    fn default() -> Self {
        Self {
            rows: Arc::default(),
            failing: Arc::default(),
            events: tokio::sync::broadcast::channel(64).0,
        }
    }
}

/// Home Assistant's WebSocket API: sign in, subscribe, then events.
async fn fake_ha_socket(state: FakeHa, mut socket: axum::extract::ws::WebSocket) {
    use axum::extract::ws::Message;
    let message = |v: Value| Message::Text(v.to_string().into());
    let mut events = state.events.subscribe();
    let _ = socket
        .send(message(json!({ "type": "auth_required" })))
        .await;
    let Some(Ok(Message::Text(auth))) = socket.recv().await else {
        return;
    };
    let auth: Value = serde_json::from_str(&auth).unwrap();
    assert_eq!(auth["type"], "auth");
    if state.failing.load(Ordering::SeqCst) == 401 || auth["access_token"] != "ha.token" {
        let invalid = json!({ "type": "auth_invalid", "message": "Invalid access token" });
        let _ = socket.send(message(invalid)).await;
        return;
    }
    let _ = socket.send(message(json!({ "type": "auth_ok" }))).await;
    let Some(Ok(Message::Text(subscribe))) = socket.recv().await else {
        return;
    };
    let subscribe: Value = serde_json::from_str(&subscribe).unwrap();
    assert_eq!(subscribe["type"], "subscribe_events");
    assert_eq!(subscribe["event_type"], "state_changed");
    let id = subscribe["id"].clone();
    let result = json!({ "id": id, "type": "result", "success": true, "result": null });
    let _ = socket.send(message(result)).await;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(Value::Null) | Err(_) => return,
                Ok(data) => {
                    let event = json!({ "id": id, "type": "event", "event": {
                        "event_type": "state_changed", "data": data } });
                    if socket.send(message(event)).await.is_err() {
                        return;
                    }
                }
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let ping: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(ping["type"], "ping");
                    let pong = json!({ "id": ping["id"], "type": "pong" });
                    let _ = socket.send(message(pong)).await;
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

impl FakeHa {
    async fn start(&self) -> String {
        let state = self.clone();
        let socket_state = self.clone();
        let fake = Router::new().route(
            "/api/websocket",
            get(move |ws: axum::extract::ws::WebSocketUpgrade| {
                let state = socket_state.clone();
                async move {
                    match state.failing.load(Ordering::SeqCst) {
                        0 | 401 => ws.on_upgrade(move |socket| fake_ha_socket(state, socket)),
                        status => StatusCode::from_u16(status as u16).unwrap().into_response(),
                    }
                }
            }),
        );
        let fake = fake.route(
            "/api/history/period/{start}",
            get(
                move |headers: HeaderMap, Query(q): Query<HashMap<String, String>>| {
                    let state = state.clone();
                    async move {
                        let failing = state.failing.load(Ordering::SeqCst) as u16;
                        if failing != 0 {
                            return (StatusCode::from_u16(failing).unwrap(), Json(json!({})));
                        }
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer ha.token",
                            "sends the HA token"
                        );
                        assert_eq!(q["significant_changes_only"], "0");
                        assert!(q.contains_key("end_time"));
                        let rows = state.rows.lock().unwrap().clone();
                        if q["filter_entity_id"] != "media_player.living_room_appletv" {
                            return (StatusCode::OK, Json(json!([])));
                        }
                        (StatusCode::OK, Json(json!([rows])))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
        format!("http://{addr}")
    }

    fn set_rows(&self, rows: Vec<Value>) {
        *self.rows.lock().unwrap() = rows;
    }

    fn fail_with(&self, status: u16) {
        self.failing.store(status as usize, Ordering::SeqCst);
    }

    /// Sends a player's new state over every WebSocket connection.
    fn change(&self, new_state: Value) {
        let entity_id = new_state["entity_id"].clone();
        self.events
            .send(json!({ "entity_id": entity_id, "old_state": null, "new_state": new_state }))
            .unwrap();
    }

    fn disconnect(&self) {
        self.events.send(Value::Null).unwrap();
    }
}

/// `row` as the live feed sends it: `position` seconds into what's playing.
fn live_row(mut row: Value, position: u32) -> Value {
    let at = row["last_updated"].clone();
    row["attributes"]["media_position"] = json!(position);
    row["attributes"]["media_position_updated_at"] = at;
    row
}

/// A history row `minutes_ago` minutes before now.
fn ha_row(minutes_ago: i64, state: &str, attributes: Value) -> Value {
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_mins(minutes_ago);
    json!({ "entity_id": "media_player.living_room_appletv", "state": state,
            "attributes": attributes, "last_changed": at.to_string(),
            "last_updated": at.to_string() })
}

fn infuse_row(minutes_ago: i64, state: &str, title: &str, duration: u32) -> Value {
    ha_row(
        minutes_ago,
        state,
        json!({ "app_id": "com.firecore.infuse", "app_name": "Infuse", "media_title": title,
                "media_duration": duration, "media_content_type": "video" }),
    )
}

fn tv_app_row(
    minutes_ago: i64,
    state: &str,
    title: &str,
    content_id: &str,
    duration: u32,
) -> Value {
    ha_row(
        minutes_ago,
        state,
        json!({ "app_id": "com.apple.TVWatchList", "app_name": "TV", "media_title": title,
                "media_content_id": content_id, "media_duration": duration }),
    )
}

struct PlaybackApp {
    t: TestApp,
    ha: FakeHa,
    client: credits::playback::HomeAssistant,
    players: Vec<String>,
}

impl PlaybackApp {
    async fn new() -> Self {
        let ha = FakeHa::default();
        let mut config = Config::from_lookup(|key| match key {
            "TMDB_API_TOKEN" => Some(TOKEN.to_owned()),
            "HA_URL" => Some("http://placeholder".to_owned()),
            "HA_TOKEN" => Some("ha.token".to_owned()),
            "CREDITS_DB" => Some(IN_MEMORY.to_owned()),
            _ => None,
        })
        .unwrap();
        let ha_config = config.home_assistant.as_mut().unwrap();
        ha_config.url = ha.start().await;
        let client = credits::playback::HomeAssistant::new(ha_config);
        let players = ha_config.players.clone();
        let t = build_from(config, Duration::from_secs(3600), Duration::from_secs(3600)).await;
        Self {
            t,
            ha,
            client,
            players,
        }
    }

    async fn sync(&self) -> Result<(), String> {
        credits::playback::sync_once(&self.t.catalog, &self.client, &self.players).await
    }

    async fn page(&self) -> String {
        let (status, _, body) = get_page(self.t.app(), "/playback", false).await;
        assert_eq!(status, StatusCode::OK);
        body
    }

    /// Follows Home Assistant live in the background, as the server does.
    fn run(&self) {
        tokio::spawn(credits::playback::run(
            self.t.catalog.clone(),
            self.client.clone(),
            self.players.clone(),
        ));
    }

    /// Waits until `/api/playback` passes `check`.
    async fn wait_for(&self, what: &str, check: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..400 {
            let (_, json) = get_json(self.t.app(), "/api/playback").await;
            if check(&json) {
                return json;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (_, json) = get_json(self.t.app(), "/api/playback").await;
        panic!("timed out waiting for {what}: {json}");
    }
}

#[tokio::test]
async fn playback_page_says_when_home_assistant_is_not_set_up() {
    let body = get_page(test_app().await, "/playback", false).await.2;
    assert!(body.contains("Home Assistant isn't set up"));
    assert!(body.contains("Nothing yet."));
    let (_, json) = get_json(test_app().await, "/api/playback").await;
    assert_eq!(json["status"]["state"], "not_configured");
    let (_, _, home) = get_page(test_app().await, "/", false).await;
    assert!(home.contains(r#"<a href="/playback">Seen on Apple TV</a>"#));
}

#[tokio::test]
async fn an_episode_played_to_ninety_percent_is_marked_watched() {
    let p = PlaybackApp::new().await;
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    // 58 min episode: 30 min, a pause, then 23 min (53 of 58 min, 91%).
    p.ha.set_rows(vec![
        infuse_row(120, "playing", pilot, 3480),
        infuse_row(90, "paused", pilot, 3480),
        infuse_row(80, "playing", pilot, 3480),
        infuse_row(
            57,
            "playing",
            "Breaking Bad - S1 ∙ E2 - Cat's in the Bag...",
            3000,
        ),
        infuse_row(50, "idle", "", 0),
        ha_row(
            40,
            "playing",
            json!({ "app_id": "com.google.ios.youtube", "app_name": "YouTube" }),
        ),
        ha_row(10, "off", json!({})),
    ]);
    p.sync().await.unwrap();
    let watched = p.t.catalog.watched(1396).await;
    assert!(watched.episode(1, 1), "pilot marked");
    assert!(!watched.episode(1, 2), "7 of 50 min isn't watched");

    let body = p.page().await;
    assert!(body.contains("Last read Home Assistant's history at"));
    assert!(body.contains(r#"<a href="/tv/1396/season/1/episode/1">Breaking Bad S1 E1</a>"#));
    assert!(body.contains("Marked watched"));
    assert!(body.contains("Breaking Bad S1 E2"));
    assert!(body.contains("14%"), "7 of 50 minutes");
    assert!(body.contains("not tracked: YouTube 30 min"));
    assert!(!body.contains("Which show or movie is this?"));

    // Reading the same history again counts nothing twice.
    p.sync().await.unwrap();
    let (_, json) = get_json(p.t.app(), "/api/playback").await;
    let recent = json["recent"].as_array().unwrap();
    let e2 = recent.iter().find(|d| d["episode"] == 2).unwrap();
    assert_eq!(e2["playing_seconds"], 7 * 60);
    assert_eq!(e2["status"], "watching");
    assert_eq!(json["status"]["state"], "ok");

    // A manual unmark sticks: the episode was handled already.
    p.t.catalog
        .set_watched(
            1396,
            credits::model::WatchTarget::Episode {
                season: 1,
                number: 1,
            },
            false,
        )
        .await
        .unwrap();
    p.sync().await.unwrap();
    assert!(!p.t.catalog.watched(1396).await.episode(1, 1));
}

#[tokio::test]
async fn a_movie_played_to_ninety_percent_is_marked_watched_on_its_page() {
    let p = PlaybackApp::new().await;
    let movie = "Back to the Future";
    // 116 min: 60 min, a pause, then 45 min (105 min, 90.5%).
    p.ha.set_rows(vec![
        infuse_row(200, "playing", movie, 6962),
        infuse_row(140, "paused", movie, 6962),
        infuse_row(110, "playing", movie, 6962),
        infuse_row(65, "idle", "", 0),
    ]);
    post_form(p.t.app(), "/movie/105/watchlist", "listed=true").await;
    p.sync().await.unwrap();
    assert!(p.t.catalog.movie_watched(105).await.apple_tv);
    let (_, json) = get_json(p.t.app(), "/api/watchlist").await;
    assert_eq!(json["titles"], json!([]), "watched leaves To watch");
    let body = p.page().await;
    assert!(body.contains(
        r#"<a href="/movie/105">Back to the Future</a> <span class="muted">· movie</span>"#
    ));
    assert!(body.contains("Marked watched"));
    assert!(!body.contains("Which show or movie is this?"));

    let (_, _, page) = get_page(p.t.app(), "/movie/105", false).await;
    assert!(page.contains(r#"action="/movie/105/watched""#));
    assert!(page.contains(r#"aria-pressed="true""#));
    assert!(page.contains("Marked watched from what played on the Apple TV."));

    // Unmarking over the JSON API sticks too.
    let (status, json) = put_json(
        p.t.app(),
        "/api/movie/105/watched",
        json!({ "watched": false }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["marks"]["watched"], false);
    p.sync().await.unwrap();
    assert!(!p.t.catalog.movie_watched(105).await.watched);

    // Unmarking from the page sticks, and marking again is a manual mark.
    let (status, headers) = post_form(p.t.app(), "/movie/105/watched", "watched=false").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/movie/105");
    p.sync().await.unwrap();
    assert!(!p.t.catalog.movie_watched(105).await.watched);
    post_form(p.t.app(), "/movie/105/watched", "watched=true").await;
    let (_, _, page) = get_page(p.t.app(), "/movie/105", false).await;
    assert!(page.contains(r#"aria-pressed="true""#));
    assert!(!page.contains("Marked watched from what played on the Apple TV."));
}

#[tokio::test]
async fn a_movie_played_in_the_tv_app_is_marked_watched_but_trailers_are_not() {
    let p = PlaybackApp::new().await;
    let movie = "Good Will Hunting";
    // 127 min, the TV app's IDs from Oct 21: a trailer, then 115 min (91%).
    p.ha.set_rows(vec![
        tv_app_row(130, "playing", movie, "PMTFY26TRL001", 150),
        tv_app_row(127, "playing", movie, "PMTFY26FTR018", 7599),
        tv_app_row(12, "idle", "", "", 0),
    ]);
    p.sync().await.unwrap();
    assert!(p.t.catalog.movie_watched(489).await.apple_tv);
    let body = p.page().await;
    assert!(body.contains(
        r#"<a href="/movie/489">Good Will Hunting</a> <span class="muted">· movie</span>"#
    ));
    assert!(body.contains("Marked watched"));
    assert!(!body.contains("Which show or movie is this?"));
}

#[tokio::test]
async fn movies_with_one_name_wait_for_a_choice_unless_the_year_or_runtime_settles_it() {
    let p = PlaybackApp::new().await;
    // 107 min fits both: 1982 (109 min) and 2011 (103 min).
    p.ha.set_rows(vec![
        infuse_row(200, "playing", "The Thing", 6400),
        infuse_row(90, "paused", "The Thing", 6400),
    ]);
    p.sync().await.unwrap();
    let body = p.page().await;
    assert!(body.contains("Which show or movie is this?"));
    assert!(body.contains("2 TMDB movies named “The Thing” fit the runtime that played"));
    assert!(body.contains(r#"<input type="radio" name="tmdb_id" value="1091">"#));
    assert!(body.contains(r#"<a href="/movie/60935">details</a>"#));
    assert!(body.contains("Use this movie"));
    assert!(!body.contains("Waiting:"));

    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&kind=movie&show=the+thing&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nothing chosen");
    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&kind=movie&show=the+thing&tmdb_id=1091&other_id=&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(p.t.catalog.movie_watched(1091).await.apple_tv);
    assert!(!p.t.catalog.movie_watched(60935).await.watched);

    // A year in the title picks the movie; a runtime that fits one does too,
    // even with no duration reported (TMDB's runtime is used).
    let p = PlaybackApp::new().await;
    p.ha.set_rows(vec![
        infuse_row(300, "playing", "The Thing (2011)", 6400),
        infuse_row(190, "paused", "The Thing (2011)", 6400),
        infuse_row(180, "playing", "Back to the Future", 0),
        infuse_row(70, "paused", "Back to the Future", 0),
    ]);
    p.sync().await.unwrap();
    assert!(p.t.catalog.movie_watched(60935).await.watched);
    assert!(!p.t.catalog.movie_watched(1091).await.watched);
    assert!(p.t.catalog.movie_watched(105).await.watched);
}

#[tokio::test]
async fn unknown_movies_can_be_ignored() {
    let p = PlaybackApp::new().await;
    p.ha.set_rows(vec![
        infuse_row(100, "playing", "Beach Trip 2019", 1800),
        infuse_row(60, "paused", "Beach Trip 2019", 1800),
    ]);
    p.sync().await.unwrap();
    let body = p.page().await;
    assert!(body.contains("No TMDB movie is named “Beach Trip 2019”"));
    assert!(body.contains("TMDB found no movies by that name."));
    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&kind=movie&show=beach+trip+2019&other_id=abc&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&kind=movie&show=beach+trip+2019&other_id=&action=ignore",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let body = p.page().await;
    assert!(!body.contains("Which show or movie is this?"));
    assert!(body.contains(">Ignored<"));
}

#[tokio::test]
async fn ambiguous_shows_wait_for_a_choice_that_then_applies_to_later_episodes() {
    let p = PlaybackApp::new().await;
    p.ha.set_rows(vec![
        tv_app_row(200, "playing", "Dark Matter", "A0052201001", 2920),
        tv_app_row(150, "paused", "Dark Matter", "A0052201001", 2920),
    ]);
    p.sync().await.unwrap();
    assert!(!p.t.catalog.watched(196322).await.episode(1, 1));
    let body = p.page().await;
    assert!(body.contains("Which show or movie is this?"));
    assert!(body.contains("2 TMDB shows named “Dark Matter” fit S1 E1"));
    assert!(body.contains("Waiting: S1 E1."));
    assert!(body.contains(r#"<input type="radio" name="tmdb_id" value="196322">"#));
    assert!(body.contains(r#"value="62425""#));
    assert!(body.contains("Needs confirmation"));

    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=tv&show=dark+matter&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nothing chosen");

    let (status, headers) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=tv&show=dark+matter&tmdb_id=196322&other_id=&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], "/playback");
    assert!(p.t.catalog.watched(196322).await.episode(1, 1));
    let body = p.page().await;
    assert!(!body.contains("Which show or movie is this?"));
    assert!(body.contains(r#"<a href="/tv/196322/season/1/episode/1">Dark Matter S1 E1</a>"#));

    // The next episode is matched without asking.
    p.ha.set_rows(vec![
        tv_app_row(120, "playing", "Dark Matter", "A0052201002", 3516),
        tv_app_row(60, "paused", "Dark Matter", "A0052201002", 3516),
    ]);
    p.sync().await.unwrap();
    assert!(p.t.catalog.watched(196322).await.episode(1, 2));
}

#[tokio::test]
async fn ignored_and_unknown_shows_are_not_marked() {
    let p = PlaybackApp::new().await;
    p.ha.set_rows(vec![
        infuse_row(100, "playing", "Nowhere Show - S1 ∙ E1 - Start", 1200),
        infuse_row(60, "paused", "Nowhere Show - S1 ∙ E1 - Start", 1200),
    ]);
    p.sync().await.unwrap();
    let body = p.page().await;
    assert!(body.contains("No TMDB show is named “Nowhere Show”"));
    assert!(body.contains("TMDB found no shows by that name."));
    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&show=nowhere+show&other_id=abc&action=use",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post_form(
        p.t.app(),
        "/playback/decide",
        "app=infuse&show=nowhere+show&other_id=&action=ignore",
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let body = p.page().await;
    assert!(!body.contains("Which show or movie is this?"));
    assert!(body.contains(">Ignored<"));
}

#[tokio::test]
async fn home_assistant_and_tmdb_failures_are_retried() {
    let p = PlaybackApp::new().await;
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    p.ha.set_rows(vec![
        infuse_row(120, "playing", pilot, 3480),
        infuse_row(60, "paused", pilot, 3480),
    ]);

    p.ha.fail_with(401);
    assert!(p.sync().await.unwrap_err().contains("rejected the token"));
    let body = p.page().await;
    assert!(body.contains("Couldn't read Home Assistant's history at"));
    assert!(body.contains("Home Assistant rejected the token (HA_TOKEN)"));

    p.ha.fail_with(0);
    p.t.tmdb.set_down(true);
    p.sync().await.unwrap();
    assert!(!p.t.catalog.watched(1396).await.episode(1, 1));
    let body = p.page().await;
    assert!(body.contains("Last read Home Assistant's history at"));
    assert!(body.contains("TMDB couldn&#39;t be reached"));
    assert!(body.contains("will retry"));

    p.ha.fail_with(502);
    p.t.tmdb.set_down(false);
    let error = p.sync().await.unwrap_err();
    assert!(error.contains("502"), "{error}");
    assert!(
        p.t.catalog.watched(1396).await.episode(1, 1),
        "marked from what was already read"
    );
    let body = p.page().await;
    assert!(body.contains("last read at"));
}

#[tokio::test]
async fn credits_skipped_missing_durations_and_later_matches_still_mark() {
    let p = PlaybackApp::new().await;
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    p.ha.set_rows(vec![
        // 85% of the pilot, then the next episode: the credits were skipped.
        infuse_row(300, "playing", pilot, 3480),
        infuse_row(
            251,
            "playing",
            "Breaking Bad - S1 ∙ E2 - Cat's in the Bag...",
            3000,
        ),
        infuse_row(
            240,
            "paused",
            "Breaking Bad - S1 ∙ E2 - Cat's in the Bag...",
            3000,
        ),
        // Infuse didn't say how long this one is; TMDB's 58 min is used.
        infuse_row(
            200,
            "playing",
            "Breaking Bad - S2 ∙ E1 - Seven Thirty-Seven",
            0,
        ),
        infuse_row(
            145,
            "paused",
            "Breaking Bad - S2 ∙ E1 - Seven Thirty-Seven",
            0,
        ),
        // Dark Matter E1 fits two shows, E2 only one: that settles E1 too.
        tv_app_row(140, "playing", "Dark Matter", "A0052201001", 2920),
        tv_app_row(90, "playing", "Dark Matter", "A0052201002", 3516),
        tv_app_row(30, "paused", "Dark Matter", "A0052201002", 3516),
    ]);
    p.sync().await.unwrap();
    let bb = p.t.catalog.watched(1396).await;
    assert!(bb.episode(1, 1), "85% and the next episode started");
    assert!(!bb.episode(1, 2), "11 of 50 min");
    assert!(bb.episode(2, 1), "55 of TMDB's 58 min");
    let dm = p.t.catalog.watched(196322).await;
    assert!(dm.episode(1, 1) && dm.episode(1, 2));
    let body = p.page().await;
    assert!(!body.contains("Which show or movie is this?"));
}

#[tokio::test]
async fn eighty_percent_alone_is_not_watched() {
    let p = PlaybackApp::new().await;
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    p.ha.set_rows(vec![
        infuse_row(300, "playing", pilot, 3480),
        infuse_row(251, "paused", pilot, 3480),
    ]);
    p.sync().await.unwrap();
    assert!(!p.t.catalog.watched(1396).await.episode(1, 1));
    let (_, json) = get_json(p.t.app(), "/api/playback").await;
    assert_eq!(json["recent"][0]["status"], "watching");
}

#[tokio::test]
async fn live_positions_mark_episodes_and_reconnecting_catches_up() {
    let p = PlaybackApp::new().await;
    p.run();
    p.wait_for("connected", |j| j["live"]["state"] == "connected")
        .await;
    let body = p.page().await;
    assert!(body.contains("Following Home Assistant live since"));
    let (_, _, about) = get_page(p.t.app(), "/about", false).await;
    assert!(about.contains("Connected since"));

    // 5 minutes played from 47 minutes in, then a skip to 55 minutes: only
    // the live position shows the pilot was watched.
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    p.ha.change(live_row(infuse_row(60, "playing", pilot, 3480), 2820));
    p.ha.change(
        json!({ "entity_id": "light.kitchen", "state": "on", "attributes": {},
                        "last_updated": jiff::Timestamp::now().to_string() }),
    );
    p.ha.change(live_row(infuse_row(55, "paused", pilot, 3480), 3300));
    let json = p
        .wait_for("the pilot marked", |j| j["recent"][0]["status"] == "marked")
        .await;
    assert_eq!(json["recent"][0]["playing_seconds"], 300);
    assert_eq!(json["recent"][0]["max_position"], 3300);
    assert!(p.t.catalog.watched(1396).await.episode(1, 1));
    assert!(p.page().await.contains("94%"));

    // Played while the connection was down: read from the history once
    // it's back.
    let e2 = "Breaking Bad - S1 ∙ E2 - Cat's in the Bag...";
    p.ha.set_rows(vec![
        infuse_row(55, "paused", pilot, 3480),
        infuse_row(50, "playing", e2, 3000),
        infuse_row(3, "paused", e2, 3000),
    ]);
    p.ha.disconnect();
    p.wait_for("disconnected", |j| j["live"]["state"] == "disconnected")
        .await;
    let body = p.page().await;
    assert!(body.contains("Disconnected from Home Assistant since"));
    assert!(body.contains("connection to Home Assistant lost"));
    p.wait_for("reconnected and caught up", |j| {
        j["live"]["state"] == "connected" && j["recent"][0]["status"] == "marked"
    })
    .await;
    assert!(p.t.catalog.watched(1396).await.episode(1, 2));
}

#[tokio::test]
async fn live_feed_problems_show_on_about_and_playback_pages() {
    let (_, _, about) = get_page(test_app().await, "/about", false).await;
    assert!(about.contains("Not set up, so episodes and movies aren't marked"));

    let p = PlaybackApp::new().await;
    p.ha.fail_with(401);
    p.run();
    let json = p
        .wait_for("disconnected", |j| j["live"]["state"] == "disconnected")
        .await;
    assert_eq!(
        json["live"]["error"],
        "Home Assistant rejected the token (HA_TOKEN)"
    );
    let (_, _, about) = get_page(p.t.app(), "/about", false).await;
    assert!(about.contains("Disconnected since"));
    assert!(about.contains("retrying: Home Assistant rejected the token (HA_TOKEN)"));
    let body = p.page().await;
    assert!(body.contains("Couldn't read Home Assistant's history"));
}

#[tokio::test]
async fn season_and_episode_pages_label_episodes_marked_from_the_apple_tv() {
    let p = PlaybackApp::new().await;
    let pilot = "Breaking Bad - S1 ∙ E1 - Pilot";
    p.ha.set_rows(vec![
        infuse_row(120, "playing", pilot, 3480),
        infuse_row(60, "paused", pilot, 3480),
    ]);
    p.sync().await.unwrap();
    p.t.catalog
        .set_watched(
            1396,
            credits::model::WatchTarget::Episode {
                season: 1,
                number: 2,
            },
            true,
        )
        .await
        .unwrap();
    let (_, _, season) = get_page(p.t.app(), "/tv/1396/season/1", false).await;
    assert_eq!(season.matches("Watched on Apple TV").count(), 1, "only E1");
    let (_, _, episode) = get_page(p.t.app(), "/tv/1396/season/1/episode/1", false).await;
    assert!(episode.contains("Marked watched from what played on the Apple TV"));
    let (_, _, episode) = get_page(p.t.app(), "/tv/1396/season/1/episode/2", false).await;
    assert!(!episode.contains("Marked watched from what played on the Apple TV"));
}

#[tokio::test]
async fn now_playing_lists_only_movies_in_the_regions_theaters_by_date() {
    let (status, _, body) = get_page(test_app().await, "/now-playing", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<h1 id="releases-heading">Now playing</h1>"#));
    assert!(
        body.contains(r#"<a href="/now-playing">Now playing</a>"#),
        "header link"
    );
    assert!(
        body.contains(r#"<a href="/coming-soon">Coming soon</a>"#),
        "header link"
    );
    assert!(body.contains("Opening Night"), "opened 10 days ago");
    for (title, why) in [
        ("Long Run", "TMDB lists it, but it opened 100 days ago"),
        ("Gone Digital", "out on digital"),
    ] {
        assert!(!body.contains(title), "{title}: {why}");
    }
    assert_eq!(body.matches(r#"<li class="result">"#).count(), 1);
    assert!(
        !body.contains(r#"class="controls pager""#),
        "one page, no pager"
    );
}

#[tokio::test]
async fn coming_soon_lists_twenty_regional_openings_a_page_soonest_first() {
    let app = test_app().await;
    let (status, _, body) = get_page(app.clone(), "/coming-soon", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<h1 id="releases-heading">Coming soon</h1>"#));
    assert_eq!(body.matches(r#"<li class="result">"#).count(), 20);
    let first = body.find("Premiere 01").expect("opens tomorrow");
    assert!(first < body.find("Premiere 02").unwrap(), "soonest first");
    assert!(body.contains("Premiere 20") && !body.contains("Premiere 21"));
    assert_eq!(
        body.matches(r#"<p class="release-date">Coming "#).count(),
        20
    );
    for (title, why) in [
        ("Canada First", "opens only outside the region"),
        ("Undated Film", "no date"),
        ("Already Out", "opened three days ago"),
    ] {
        assert!(!body.contains(title), "{title}: {why}");
    }
    assert!(body.contains(r#"<a href="/coming-soon?page=2" rel="next">Next →</a>"#));
    assert!(!body.contains(r#"rel="prev""#));
    assert!(body.contains("Page 1 of 2 · 25 movies"));
    assert!(
        body.contains(r#"hx-boost="true""#),
        "pages swap in over htmx"
    );

    let (_, _, body) = get_page(app.clone(), "/coming-soon?page=2", false).await;
    assert_eq!(body.matches(r#"<li class="result">"#).count(), 5);
    assert!(body.contains("Premiere 21") && body.contains("Premiere 25"));
    assert!(body.contains(r#"<a href="/coming-soon" rel="prev">← Previous</a>"#));
    assert!(!body.contains(r#"rel="next""#));

    let (_, _, body) = get_page(app.clone(), "/coming-soon?page=9", false).await;
    assert!(body.contains("Page 2 of 2"), "past the end: the last page");
    let (_, _, body) = get_page(app, "/coming-soon?page=x", false).await;
    assert!(body.contains("Page 1 of 2"), "not a number: the first page");
}

#[tokio::test]
async fn release_lists_page_as_json_and_rate_only_the_page_shown() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let (_, json) = get_json(t.app(), "/api/coming-soon").await;
    assert_eq!(json["list"], "coming_soon");
    assert_eq!(json["region"], "US");
    assert_eq!(
        (&json["page"], &json["total_pages"], &json["total"]),
        (&json!(1), &json!(2), &json!(25))
    );
    let titles = json["titles"].as_array().unwrap();
    let ids: Vec<u64> = titles.iter().map(|t| t["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, (7100..=7119).collect::<Vec<_>>());
    assert_eq!(titles[0]["rating"]["source"], "rotten_tomatoes");
    assert_eq!(titles[0]["rating"]["score"], 83.0);
    assert_eq!(json["ratings"]["unrated"], 19);
    assert_eq!(json["unchecked"], 0);
    assert_eq!(t.omdb.calls(), 1, "only this page's movies are rated");

    let (_, json) = get_json(t.app(), "/api/coming-soon?page=2").await;
    assert_eq!(json["page"], 2);
    assert_eq!(json["titles"].as_array().unwrap().len(), 5);
    assert_eq!(json["titles"][1]["id"], 7121);
    assert_eq!(json["titles"][1]["rating"]["source"], "imdb");
    assert_eq!(t.omdb.calls(), 2);

    let (_, json) = get_json(t.app(), "/api/now-playing").await;
    assert_eq!(json["list"], "now_playing");
    let ids: Vec<u64> = json["titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [7001]);
    assert_eq!(json["total_pages"], 1);
}

#[tokio::test]
async fn release_lists_are_cached_and_degrade_when_tmdb_fails() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    get_page(t.app(), "/coming-soon", false).await;
    assert_eq!(t.tmdb.upcoming_calls(), 2, "both pages are read");
    let calls = t.tmdb.calls();
    get_page(t.app(), "/coming-soon?page=2", false).await;
    assert_eq!(t.tmdb.upcoming_calls(), 2, "the list is cached for the day");
    assert_eq!(t.tmdb.calls(), calls, "and so are the dates");

    // Details failing: the movies are left out and the page says so, and
    // the list is checked again next time.
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.tmdb.set_details_down(true);
    let (status, _, body) = get_page(t.app(), "/coming-soon", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(
            "Couldn't check the release dates of 27 movies right now, so they're left out."
        ),
        "{body}"
    );
    assert!(body.contains("TMDB lists no movies coming to US theaters."));
    t.tmdb.set_details_down(false);
    let (_, _, body) = get_page(t.app(), "/coming-soon", false).await;
    assert_eq!(body.matches(r#"<li class="result">"#).count(), 20);
    assert!(!body.contains("Couldn't check"));

    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.tmdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/coming-soon", false).await;
    assert_ne!(status, StatusCode::OK);
    assert!(body.contains("Couldn&#39;t load Coming soon"), "{body}");
}

#[tokio::test]
async fn api_serves_its_openapi_spec() {
    let (status, json) = get_json(test_app().await, "/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json, serde_json::to_value(credits::web::openapi()).unwrap());
    assert!(json["paths"]["/api/movie/{id}"]["get"].is_object());
}

/// Serves the app on a loopback port, as the terminal client sees it.
async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn fulfil(client: &tui::Client, app: &mut tui::App, request: tui::app::Request) {
    use tui::app::{Request, Response};
    let (seq, response) = match request {
        Request::Search { seq, query } => (seq, Response::Search(client.search(&query).await)),
        Request::Recent { seq } => (seq, Response::Recent(client.recent().await)),
        Request::Page { seq, target } => (seq, Response::Page(tui::fetch(client, target).await)),
        Request::Save(change) => (0, Response::Saved(change, tui::save(client, change).await)),
    };
    app.respond(seq, response);
}

fn press(app: &mut tui::App, code: KeyCode) -> Vec<tui::app::Request> {
    app.key(KeyEvent::from(code), std::time::Instant::now())
}

fn screen(app: &mut tui::App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| tui::ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn terminal_client_searches_and_opens_titles_and_people_over_http() {
    let base = serve(test_app().await).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    for c in "matrix".chars() {
        assert!(
            press(&mut app, KeyCode::Char(c)).is_empty(),
            "typing is debounced"
        );
    }
    let search = app
        .tick(std::time::Instant::now() + tui::app::SEARCH_DEBOUNCE)
        .expect("search goes out after the pause");
    fulfil(&client, &mut app, search).await;
    let results = screen(&mut app);
    assert!(results.contains("Movie  The Matrix (1999)"), "{results}");
    assert!(
        results.contains("Person Keanu Reeves  Acting · The Matrix"),
        "{results}"
    );
    assert!(results.contains("TV     Breaking Bad (2008)"), "{results}");

    // Enter opens the first result: the movie, with its ratings.
    let open = press(&mut app, KeyCode::Enter);
    assert_eq!(open.len(), 1);
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let movie = screen(&mut app);
    assert!(movie.contains("The Matrix"), "{movie}");
    assert!(movie.contains("Search › The Matrix"), "{movie}");
    assert!(movie.contains("Ratings"), "{movie}");
    assert!(
        movie.contains(
            "In theaters Mar 31, 1999 · Digital Sep 21, 1999 · Physical Sep 21, 1999 · Rated R"
        ),
        "{movie}"
    );

    // Back to the results, down to Keanu Reeves, and open his page.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Char('j'));
    let open = press(&mut app, KeyCode::Char('l'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let person = screen(&mut app);
    assert!(person.contains("Keanu Reeves"), "{person}");
    assert!(
        person.contains("Born Sep 2, 1964 · Beirut, Lebanon"),
        "{person}"
    );
    assert!(person.contains("Known for"), "{person}");
    assert!(person.contains("Neo"), "{person}");
    assert!(
        !person.contains("He was born in Beirut"),
        "only the first paragraph: {person}"
    );

    // A filmography entry links back to its title.
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Title(tui::api::Kind::Movie, 603),
            ..
        }]
    ));
}

#[tokio::test]
async fn terminal_client_marks_movies_in_theaters() {
    let base = serve(test_app().await).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    for c in "theaters".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    let search = app
        .tick(std::time::Instant::now() + tui::app::SEARCH_DEBOUNCE)
        .expect("search goes out after the pause");
    fulfil(&client, &mut app, search).await;
    let results = screen(&mut app);
    let year = jiff::Zoned::now().date() - jiff::Span::new().days(10);
    assert!(
        results.contains(&format!(
            "Movie  Opening Night ({})  In theaters",
            year.year()
        )),
        "{results}"
    );
    assert_eq!(results.matches("In theaters").count(), 2, "{results}");

    let open = press(&mut app, KeyCode::Enter);
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let movie = screen(&mut app);
    assert!(movie.contains("Movie · "), "{movie}");
    assert!(movie.contains("1h 40m · In theaters"), "{movie}");
    assert!(
        movie.contains("In theaters. Not streaming or for rent or sale in US yet."),
        "{movie}"
    );
    assert!(!movie.contains("No streaming, rental"), "{movie}");
}

#[tokio::test]
async fn terminal_client_opens_and_pages_now_playing_and_coming_soon() {
    let base = serve(test_app().await).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    press(&mut app, KeyCode::Esc);

    let open = press(&mut app, KeyCode::Char('u'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let coming = screen(&mut app);
    assert!(coming.contains("Page 1 of 2 · 25 movies"), "{coming}");
    assert!(
        coming.find("Premiere 01") < coming.find("Premiere 02"),
        "{coming}"
    );
    assert!(coming.contains("Coming "), "{coming}");
    assert!(!coming.contains("Canada First"), "{coming}");

    let next = press(&mut app, KeyCode::Char(']'));
    fulfil(&client, &mut app, next.into_iter().next().unwrap()).await;
    let page2 = screen(&mut app);
    assert!(page2.contains("Page 2 of 2"), "{page2}");
    assert!(page2.contains("Premiere 25"), "{page2}");
    assert!(
        press(&mut app, KeyCode::Char(']')).is_empty(),
        "no page after the last"
    );
    let back = press(&mut app, KeyCode::Char('['));
    fulfil(&client, &mut app, back.into_iter().next().unwrap()).await;
    assert!(screen(&mut app).contains("Page 1 of 2"));

    let open = press(&mut app, KeyCode::Char('n'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let now = screen(&mut app);
    assert!(now.contains("Now playing"), "{now}");
    assert!(now.contains("Opening Night"), "{now}");
    assert!(!now.contains("Long Run"), "{now}");
}

#[test]
fn terminal_client_help_is_modal_and_restores_search_focus_and_selection() {
    for editing in [true, false] {
        for close in [KeyCode::Esc, KeyCode::Char('?')] {
            let mut app = tui::App::new("http://localhost");
            app.search.query = "matrix".to_owned();
            app.search.answered = "matrix".to_owned();
            app.search.results = (0..60)
                .map(|id| {
                    tui::api::SearchHit::Movie(tui::api::TitleHit {
                        id,
                        title: format!("Result {id}"),
                        year: None,
                        in_theaters: false,
                    })
                })
                .collect();
            app.editing = false;
            screen(&mut app);
            for _ in 0..45 {
                press(&mut app, KeyCode::Down);
            }
            app.editing = editing;
            app.status = Some(tui::app::Status::Done("Saved.".to_owned()));
            let before = screen(&mut app);
            let selected = app.search.cursor.selected;
            let scroll = app.search.cursor.scroll;
            assert!(before.contains("? help"));
            assert!(press(&mut app, KeyCode::Char('?')).is_empty());
            let help = screen(&mut app);
            assert!(help.contains("Keyboard help"), "{help}");
            assert!(help.contains("Navigation"), "{help}");
            assert!(help.contains("Search"), "{help}");
            for key in [
                'b', 'F', 'x', 'X', 'w', 'f', 'r', '/', 'q', 't', 'o', 'm', 's', '[', ']',
            ] {
                assert!(press(&mut app, KeyCode::Char(key)).is_empty());
            }
            for key in [
                KeyCode::Enter,
                KeyCode::Tab,
                KeyCode::BackTab,
                KeyCode::Backspace,
            ] {
                assert!(press(&mut app, key).is_empty());
            }
            assert!(!app.quit);
            assert!(app.stack.is_empty());
            assert_eq!(app.search.query, "matrix");
            assert!(app.search_due().is_none());
            assert_eq!(app.search.cursor.selected, selected);
            assert_eq!(app.search.cursor.scroll, scroll);
            assert_eq!(app.editing, editing);
            press(&mut app, close);
            assert!(app.help.is_none());
            assert_eq!(screen(&mut app), before);
        }
    }
}

#[test]
fn terminal_client_help_scrolls_resizes_and_keeps_ctrl_c_available() {
    let mut app = tui::App::new("http://localhost");
    app.open(tui::app::Target::Person(
        6384,
        tui::app::ListView::default(),
    ));
    app.stack.last_mut().unwrap().load = tui::app::Load::Failed("Provider unavailable".to_owned());
    let before = screen(&mut app);
    press(&mut app, KeyCode::Char('?'));
    let mut terminal = Terminal::new(TestBackend::new(45, 16)).unwrap();
    terminal
        .draw(|frame| tui::ui::draw(frame, &mut app))
        .unwrap();
    assert!(app.help.as_ref().unwrap().max_scroll > 10);
    press(&mut app, KeyCode::PageDown);
    assert_eq!(app.help.as_ref().unwrap().scroll, 10);
    press(&mut app, KeyCode::Up);
    assert_eq!(app.help.as_ref().unwrap().scroll, 9);
    press(&mut app, KeyCode::Home);
    assert_eq!(app.help.as_ref().unwrap().scroll, 0);
    press(&mut app, KeyCode::End);
    let help = app.help.as_ref().unwrap();
    assert_eq!(help.scroll, help.max_scroll);
    terminal
        .draw(|frame| tui::ui::draw(frame, &mut app))
        .unwrap();
    let bottom: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(bottom.contains("Previous / next episode"), "{bottom}");
    // Resizing recalculates wrapping and clamps the independent help cursor.
    screen(&mut app);
    let help = app.help.as_ref().unwrap();
    assert!(help.scroll <= help.max_scroll);
    for size in [(1, 1), (10, 4), (30, 10), (120, 50)] {
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
        terminal
            .draw(|frame| tui::ui::draw(frame, &mut app))
            .unwrap();
    }
    press(&mut app, KeyCode::Esc);
    assert_eq!(
        screen(&mut app),
        before,
        "Esc closes help, not the failed page"
    );
    press(&mut app, KeyCode::Char('?'));
    app.key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        std::time::Instant::now(),
    );
    assert!(app.quit);
}

#[test]
fn terminal_client_ctrl_d_quits_from_search_box_and_pages_elsewhere() {
    let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
    let mut app = tui::App::new("http://localhost");
    app.editing = false;
    app.key(ctrl_d, std::time::Instant::now());
    assert!(!app.quit, "Ctrl-D pages down outside the search box");
    app.editing = true;
    app.key(ctrl_d, std::time::Instant::now());
    assert!(app.quit);
}

#[tokio::test]
async fn terminal_client_help_dims_keys_for_titles_people_lists_and_episodes() {
    use tui::api::Kind;
    use tui::app::{ListName, ListView, Target};
    fn binding(app: &mut tui::App, keys: &str, active: bool) {
        let mut terminal = Terminal::new(TestBackend::new(120, 70)).unwrap();
        terminal.draw(|frame| tui::ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let needle = format!("{keys} —");
        let y = (0..buffer.area.height)
            .find(|&y| {
                let text: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                text.contains(&needle)
            })
            .expect("binding is listed");
        // Popup starts at column 13; wrapping trims its two-space gutter.
        let dim = tui::theme::palette().dim;
        let color = buffer[(14, y)].fg;
        assert_eq!(color != dim, active, "{keys}");
    }
    let base = serve(test_app().await).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    for (target, title, watched, peek, neighbour, list) in [
        (
            Target::Title(Kind::Movie, 603),
            true,
            true,
            false,
            false,
            false,
        ),
        (
            Target::Title(Kind::Tv, 1396),
            true,
            true,
            false,
            false,
            false,
        ),
        (
            Target::Person(6384, tui::app::ListView::default()),
            false,
            false,
            false,
            false,
            false,
        ),
        (Target::Season(1396, 1), false, true, true, false, false),
        (Target::Episode(1396, 1, 1), false, true, true, true, false),
        (
            Target::List(ListName::Watchlist, ListView::default()),
            false,
            false,
            false,
            false,
            true,
        ),
    ] {
        let request = app.open(target);
        fulfil(&client, &mut app, request).await;
        assert!(matches!(
            app.stack.last().unwrap().load,
            tui::app::Load::Ready(_)
        ));
        press(&mut app, KeyCode::Char('?'));
        for (keys, active) in [
            ("b / F", title),
            ("x", watched),
            ("X", watched && !list),
            ("s", peek),
            ("[ / ]", neighbour),
            ("t", list || matches!(target, Target::Person(..))),
        ] {
            binding(&mut app, keys, active);
        }
        press(&mut app, KeyCode::Esc);
    }
    client
        .set_listed(ListName::Watchlist, Kind::Movie, 603, true)
        .await
        .unwrap();
    let request = app.open(Target::List(ListName::Watchlist, ListView::default()));
    fulfil(&client, &mut app, request).await;
    press(&mut app, KeyCode::Char('?'));
    binding(&mut app, "b / F", true);
    binding(&mut app, "x", true);
    binding(&mut app, "X", false);
}

#[test]
fn terminal_client_tabs_between_search_box_and_results_without_resetting_cursor() {
    for query in ["matrix", ""] {
        for back_to_box in [KeyCode::Tab, KeyCode::BackTab] {
            let mut app = tui::App::new("http://localhost");
            app.search.query = query.to_owned();
            app.search.answered = query.to_owned();
            app.search.results = (0..60)
                .map(|id| {
                    tui::api::SearchHit::Movie(tui::api::TitleHit {
                        id,
                        title: format!("Result {id}"),
                        year: Some("1999".to_owned()),
                        in_theaters: false,
                    })
                })
                .collect();
            assert!(screen(&mut app).contains("Tab/↓/Enter results"));
            assert!(press(&mut app, KeyCode::Tab).is_empty());
            assert!(!app.editing);
            for _ in 0..45 {
                press(&mut app, KeyCode::Down);
            }
            let results = screen(&mut app);
            assert!(results.contains("Tab/Shift-Tab search box"));
            let selected = app.search.cursor.selected;
            let scroll = app.search.cursor.scroll;
            assert_eq!(selected, 45);
            assert!(scroll > 0, "exercise a scrolled result list");
            let target = tui::ui::selected_target(&app);

            assert!(press(&mut app, back_to_box).is_empty());
            assert!(app.editing);
            screen(&mut app);
            assert_eq!(app.search.query, query);
            assert_eq!(app.search.answered, query);
            assert_eq!(app.search.results.len(), 60);
            assert_eq!(app.search.cursor.selected, selected);
            assert_eq!(app.search.cursor.scroll, scroll);
            assert!(app.search_due().is_none(), "focus alone never searches");

            assert!(press(&mut app, KeyCode::Tab).is_empty());
            assert!(!app.editing);
            assert_eq!(screen(&mut app), results);
            assert_eq!(tui::ui::selected_target(&app), target);
            assert_eq!(app.search.cursor.selected, selected);
            assert_eq!(app.search.cursor.scroll, scroll);
        }
    }
}

#[test]
fn terminal_client_tabs_handle_empty_search_but_leave_other_pages_alone() {
    let mut app = tui::App::new("http://localhost");
    for key in [KeyCode::Tab, KeyCode::BackTab] {
        assert!(press(&mut app, key).is_empty());
        assert!(!app.editing);
        assert!(press(&mut app, key).is_empty());
        assert!(app.editing);
        assert!(app.search.query.is_empty());
    }

    app.open(tui::app::Target::Person(
        6384,
        tui::app::ListView::default(),
    ));
    for key in [KeyCode::Tab, KeyCode::BackTab] {
        assert!(press(&mut app, key).is_empty());
        assert!(!app.editing);
        assert_eq!(app.stack.len(), 1);
    }
}

#[tokio::test]
async fn terminal_client_shows_degraded_and_failed_states() {
    // No OMDb key: ratings are unavailable but the page still renders.
    let base = serve(build(TOKEN, None, Duration::from_secs(3600)).await.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let open = app.open(tui::app::Target::Title(tui::api::Kind::Movie, 603));
    fulfil(&client, &mut app, open).await;
    let movie = screen(&mut app);
    assert!(
        movie.contains("Ratings aren't configured on the server."),
        "{movie}"
    );

    let open = app.open(tui::app::Target::Person(
        424242,
        tui::app::ListView::default(),
    ));
    fulfil(&client, &mut app, open).await;
    let missing = screen(&mut app);
    assert!(missing.contains("Not found on TMDB."), "{missing}");

    // The server's own explanation of a provider failure comes through.
    let err = client.search("boom").await.unwrap_err();
    assert!(
        matches!(err, tui::client::ClientError::Server(_)),
        "{err:?}"
    );

    let offline = tui::Client::new("http://127.0.0.1:9");
    let err = offline.search("matrix").await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "can't reach the credits server at http://127.0.0.1:9"
    );
}

#[tokio::test]
async fn terminal_client_retries_a_failed_search_with_enter() {
    let mut app = tui::App::new("http://127.0.0.1:9");
    for c in "matrix".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    let search = app
        .tick(std::time::Instant::now() + tui::app::SEARCH_DEBOUNCE)
        .unwrap();
    fulfil(&tui::Client::new("http://127.0.0.1:9"), &mut app, search).await;
    let failed = screen(&mut app);
    assert!(
        failed.contains("Can't reach the credits server at http://127.0.0.1:9"),
        "{failed}"
    );
    assert!(failed.contains("Press Enter to try again."), "{failed}");

    // Still typing in the box: Enter asks again rather than typing or opening.
    let retry = press(&mut app, KeyCode::Enter);
    assert!(
        matches!(retry.as_slice(), [tui::app::Request::Search { query, .. }] if query == "matrix")
    );
}

#[tokio::test]
async fn terminal_client_browses_seasons_and_episodes_hiding_spoilers() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "credits-tui-seasons-{}-{unique}.db",
        std::process::id()
    ));
    let database = path.to_str().unwrap();
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;
    post_form(t.app(), "/tv/1396/season/2/watched", "watched=true").await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    let open = app.open(tui::app::Target::Title(tui::api::Kind::Tv, 1396));
    fulfil(&client, &mut app, open).await;
    let show = screen(&mut app);
    assert!(show.contains("Seasons"), "{show}");
    assert!(show.contains("1  Season 1"), "{show}");
    assert!(show.contains("7 episodes"), "{show}");
    assert!(
        show.contains("2  Season 2  13 episodes        ✓ Watched"),
        "{show}"
    );

    // The first link on a TV page is its first season.
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Season(1396, 1),
            ..
        }]
    ));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let season = screen(&mut app);
    assert!(season.contains("Breaking Bad · Season 1"), "{season}");
    assert!(
        season.contains("Search › Breaking Bad › Season 1"),
        "{season}"
    );
    assert!(season.contains("1. Pilot"), "{season}");
    assert!(season.contains("Jan 20, 2008"), "{season}");
    assert!(season.contains("58m"), "{season}");
    assert!(
        season.contains("◌  ▒▒ IMDb"),
        "unwatched score hidden: {season}"
    );
    assert!(!season.contains(" 90 IMDb"), "{season}");
    assert!(season.contains("s peeks the highlighted one"), "{season}");

    // s peeks the highlighted episode, and again hides it.
    press(&mut app, KeyCode::Char('s'));
    let peeked = screen(&mut app);
    assert!(peeked.contains("●  90 IMDb"), "{peeked}");
    assert!(peeked.contains("peeking"), "{peeked}");
    press(&mut app, KeyCode::Char('s'));
    assert!(!screen(&mut app).contains(" 90 IMDb"));

    // Enter opens the episode, its summary hidden until peeked.
    let open = press(&mut app, KeyCode::Enter);
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let episode = screen(&mut app);
    assert!(
        episode.contains("Breaking Bad · Season 1 · Episode 1"),
        "{episode}"
    );
    assert!(
        episode.contains("Summary hidden until you watch this episode"),
        "{episode}"
    );
    assert!(!episode.contains("A chemistry teacher"), "{episode}");
    assert!(episode.contains("Directed by"), "{episode}");
    assert!(episode.contains("Guest stars"), "{episode}");
    assert!(episode.contains("Steven Michael Quezada"), "{episode}");
    assert!(episode.contains("Series regulars"), "{episode}");
    assert!(episode.contains("Bryan Cranston"), "{episode}");
    assert!(episode.contains("] Episode 2"), "{episode}");
    press(&mut app, KeyCode::Char('s'));
    let peeked = screen(&mut app);
    assert!(
        peeked.contains("A chemistry teacher is diagnosed."),
        "{peeked}"
    );
    assert!(peeked.contains("│ 90 │  9.0/10"), "{peeked}");

    // Enter on a credited person opens them.
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Person(66633, _),
            ..
        }]
    ));
    press(&mut app, KeyCode::Esc);

    // ] swaps in the next episode; [ has nowhere to go from the first.
    assert!(press(&mut app, KeyCode::Char('[')).is_empty());
    let next = press(&mut app, KeyCode::Char(']'));
    assert!(matches!(
        next.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Episode(1396, 1, 2),
            ..
        }]
    ));
    press(&mut app, KeyCode::Esc);
    assert!(screen(&mut app).contains("1. Pilot"), "back to the season");

    // Watching the episode reveals it without a peek.
    post_form(
        t.app(),
        "/tv/1396/season/1/episode/1/watched",
        "watched=true",
    )
    .await;
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let season = screen(&mut app);
    assert!(season.contains("●  90 IMDb"), "{season}");
    assert!(season.contains("✓"), "{season}");

    // Unreadable marks keep everything hidden, and say so.
    let pool = credits::cache::ProviderCache::open(database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE watched_episodes RENAME TO watched_episodes_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let season = screen(&mut app);
    assert!(
        season.contains("Watched marks couldn't be read"),
        "{season}"
    );
    assert!(!season.contains(" 90 IMDb"), "{season}");
    let open = app.open(tui::app::Target::Episode(1396, 1, 1));
    fulfil(&client, &mut app, open).await;
    let episode = screen(&mut app);
    assert!(
        episode.contains("Watched marks couldn't be read"),
        "{episode}"
    );
    assert!(!episode.contains("A chemistry teacher"), "{episode}");
    drop(t);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn terminal_client_shows_to_watch_and_favorites_with_filters() {
    let database = temp_database("tui-lists");
    let t = build_with_database(
        TOKEN,
        Some(OMDB_KEY),
        Duration::from_secs(3600),
        Duration::ZERO,
        &database,
    )
    .await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    // Esc leaves an empty search box, and w opens the empty list.
    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let empty = screen(&mut app);
    assert!(empty.contains("Search › To watch"), "{empty}");
    assert!(empty.contains("Nothing saved yet."), "{empty}");
    press(&mut app, KeyCode::Esc);

    post_form(t.app(), "/services", "service=8").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    post_form(t.app(), "/tv/1396/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains(
            "Show All · Movies · TV   Sort Added · Rating   Available Anywhere · On my services"
        ),
        "{list}"
    );
    let bb = list
        .find("2008  TV    Breaking Bad")
        .expect("Breaking Bad row");
    let matrix = list
        .find("1999  Movie The Matrix    ■♥·  ★ Netflix")
        .expect("The Matrix row");
    assert!(bb < matrix, "most recently added first: {list}");
    assert!(list.contains("m services"), "{list}");

    // t narrows to movies, o sorts by rating, m keeps only my services.
    let movies = press(&mut app, KeyCode::Char('t'));
    assert!(matches!(
        movies.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::List(
                tui::app::ListName::Watchlist,
                tui::app::ListView {
                    kind: Some(tui::api::Kind::Movie),
                    ..
                }
            ),
            ..
        }]
    ));
    fulfil(&client, &mut app, movies.into_iter().next().unwrap()).await;
    let rated = press(&mut app, KeyCode::Char('o'));
    fulfil(&client, &mut app, rated.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(!list.contains("Breaking Bad"), "{list}");
    assert!(
        list.contains("The Matrix  ■♥·  ◕  83 RT    ★ Netflix"),
        "{list}"
    );
    let tv = press(&mut app, KeyCode::Char('t'));
    fulfil(&client, &mut app, tv.into_iter().next().unwrap()).await;
    let mine = press(&mut app, KeyCode::Char('m'));
    fulfil(&client, &mut app, mine.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("None of the TV shows on your list are on your services."),
        "{list}"
    );

    // Enter opens the title; f swaps To watch for Favorites, which has no
    // services filter.
    let all = press(&mut app, KeyCode::Char('t'));
    fulfil(&client, &mut app, all.into_iter().next().unwrap()).await;
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Title(tui::api::Kind::Movie, 603),
            ..
        }]
    ));
    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('f'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let favorites = screen(&mut app);
    assert!(favorites.contains("Search › Favorites"), "{favorites}");
    assert!(favorites.contains("1999  Movie The Matrix"), "{favorites}");
    assert!(!favorites.contains("Breaking Bad"), "{favorites}");
    assert!(!favorites.contains("Available"), "{favorites}");
    assert!(press(&mut app, KeyCode::Char('m')).is_empty());
    assert!(
        press(&mut app, KeyCode::Char('f')).is_empty(),
        "already open"
    );

    // Offers TMDB can't refresh are stale; ones it never had are unknown.
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    t.tmdb.set_down(true);
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("Couldn't refresh where some titles are available; using listings from"),
        "{list}"
    );
    assert!(list.contains("★ Netflix (listed "), "{list}");
    sqlx::query(
        r#"UPDATE provider_cache SET payload = json_remove(payload, '$."watch/providers"') WHERE key = 'movie/603'"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("The Matrix    ■♥·  Availability unknown"),
        "{list}"
    );
    assert!(
        list.contains("Couldn't check where 1 title is available right now."),
        "{list}"
    );

    // An unreadable list is an error, never an empty list.
    sqlx::query("ALTER TABLE watchlist RENAME TO watchlist_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(list.contains("The watchlist couldn't be read"), "{list}");
    assert!(list.contains("Press r to try again"), "{list}");
    assert!(!list.contains("Nothing saved yet"), "{list}");
    drop(t);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn terminal_client_lists_say_when_ratings_are_stale_or_missing() {
    let t = build_with(TOKEN, Some(OMDB_KEY), Duration::ZERO, Duration::ZERO).await;
    post_form(t.app(), "/movie/603/favorite", "listed=true").await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('f'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let rated = press(&mut app, KeyCode::Char('o'));
    fulfil(&client, &mut app, rated.into_iter().next().unwrap()).await;
    t.omdb.set_down(true);
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(list.contains("The Matrix  □♥·  ◕  83 RT"), "{list}");
    assert!(
        list.contains("Couldn't refresh some ratings; using scores from as early as "),
        "{list}"
    );

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let base = serve(t.app()).await;
    let mut app = tui::App::new(&base);
    let open = app.open(tui::app::Target::List(
        tui::app::ListName::Watchlist,
        tui::app::ListView {
            alt_sort: true,
            ..Default::default()
        },
    ));
    fulfil(&tui::Client::new(&base), &mut app, open).await;
    let list = screen(&mut app);
    assert!(
        list.contains(
            "Ratings aren't configured on the server, so titles stay in the order they were added."
        ),
        "{list}"
    );
}

#[tokio::test]
async fn terminal_client_shows_the_next_release_on_to_watch_rows() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/7100/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/7133/watchlist", "listed=true").await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    let today = jiff::Zoned::now().date();
    let day = |days: i64| {
        let date = today + jiff::Span::new().days(days);
        credits::model::release_day(&date.to_string(), today)
    };
    assert!(
        list.contains("Premiere 01 ") && list.contains(&format!("In theaters {}", day(1))),
        "{list}"
    );
    assert!(
        list.contains("Digital Next ") && list.contains(&format!("Digital {}", day(12))),
        "{list}"
    );
}

#[tokio::test]
async fn terminal_client_filters_to_watch_by_release() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    post_form(t.app(), "/movie/7100/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("Release All · Out now · Coming soon"),
        "{list}"
    );
    assert!(
        list.contains("Premiere 01") && list.contains("The Matrix"),
        "{list}"
    );

    let out = press(&mut app, KeyCode::Char('e'));
    fulfil(&client, &mut app, out.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("The Matrix") && !list.contains("Premiere 01"),
        "{list}"
    );

    let coming = press(&mut app, KeyCode::Char('e'));
    fulfil(&client, &mut app, coming.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        !list.contains("The Matrix") && list.contains("Premiere 01"),
        "{list}"
    );

    let tv = press(&mut app, KeyCode::Char('t'));
    fulfil(&client, &mut app, tv.into_iter().next().unwrap()).await;
    let tv = press(&mut app, KeyCode::Char('t'));
    fulfil(&client, &mut app, tv.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        list.contains("None of the TV shows on your list are coming soon."),
        "{list}"
    );
}

#[tokio::test]
async fn terminal_client_toggles_to_watch_favorite_and_watched() {
    let database = temp_database("tui-toggles");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);

    // A movie's marks, toggled in place from the server's answer.
    let open = app.open(tui::app::Target::Title(tui::api::Kind::Movie, 603));
    fulfil(&client, &mut app, open).await;
    let movie = screen(&mut app);
    assert!(
        movie.contains("□ To watch b   ♡ Favorite F   · Watched x"),
        "{movie}"
    );
    let save = press(&mut app, KeyCode::Char('b'));
    assert!(matches!(
        save.as_slice(),
        [tui::app::Request::Save(tui::app::Change::Listed {
            list: tui::app::ListName::Watchlist,
            listed: true,
            ..
        })]
    ));
    assert!(screen(&mut app).contains("Saving…"));
    assert!(
        press(&mut app, KeyCode::Char('F')).is_empty(),
        "one save at a time"
    );
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let movie = screen(&mut app);
    assert!(movie.contains("■ To watch b"), "{movie}");
    assert!(movie.contains("Added to To watch."), "{movie}");
    for key in ['F', 'x'] {
        let save = press(&mut app, KeyCode::Char(key));
        fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    }
    let movie = screen(&mut app);
    assert!(
        movie.contains("□ To watch b   ♥ Favorite F   ✓ Watched x"),
        "watching takes it off To watch: {movie}"
    );
    assert!(movie.contains("Movie marked watched."), "{movie}");
    press(&mut app, KeyCode::Char('j'));
    assert!(
        !screen(&mut app).contains("Movie marked watched."),
        "the outcome clears on the next key"
    );

    // Favoriting a show shows the seasons the server marked watched.
    let open = app.open(tui::app::Target::Title(tui::api::Kind::Tv, 1396));
    fulfil(&client, &mut app, open).await;
    assert!(!screen(&mut app).contains("✓ Watched"));
    let save = press(&mut app, KeyCode::Char('F'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let show = screen(&mut app);
    assert!(show.contains("♥ Favorite F"), "{show}");
    assert!(
        show.contains("2  Season 2  13 episodes        ✓ Watched"),
        "{show}"
    );
    assert!(show.contains("· Watched x"), "not the whole show: {show}");

    // x unmarks the highlighted season; X marks the whole show, which
    // still covers it.
    let save = press(&mut app, KeyCode::Char('x'));
    assert!(matches!(
        save.as_slice(),
        [tui::app::Request::Save(tui::app::Change::Watched {
            target: tui::app::Target::Season(1396, 1),
            watched: false,
        })]
    ));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    assert!(!t.catalog.watched(1396).await.season(1));
    let save = press(&mut app, KeyCode::Char('X'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    assert!(screen(&mut app).contains("✓ Watched x"));
    let save = press(&mut app, KeyCode::Char('x'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let show = screen(&mut app);
    assert!(
        show.contains("Season unmarked, but still watched with its show."),
        "{show}"
    );
    let save = press(&mut app, KeyCode::Char('X'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;

    // On a season, x marks the highlighted episode, revealing its score.
    let open = app.open(tui::app::Target::Season(1396, 1));
    fulfil(&client, &mut app, open).await;
    assert!(screen(&mut app).contains("◌  ▒▒ IMDb"));
    let save = press(&mut app, KeyCode::Char('x'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let season = screen(&mut app);
    assert!(season.contains("●  90 IMDb"), "{season}");
    assert!(season.contains("Episode marked watched."), "{season}");
    assert!(t.catalog.watched(1396).await.episode(1, 1));

    // List rows carry their marks and toggle in place; removing a row from
    // the list on screen drops it and highlights the row that took its place.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    post_form(t.app(), "/movie/105/watchlist", "listed=true").await;
    post_form(t.app(), "/tv/1396/watchlist", "listed=true").await;
    post_form(t.app(), "/movie/603/watchlist", "listed=true").await;
    let open = press(&mut app, KeyCode::Char('w'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    // Marking a row watched takes it off To watch.
    let future = tui::app::Target::Title(tui::api::Kind::Movie, 105);
    for _ in 0..2 {
        screen(&mut app);
        press(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(tui::ui::selected_target(&app), Some(future));
    let save = press(&mut app, KeyCode::Char('x'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(!list.contains("Back to the Future"), "{list}");
    assert!(list.contains("Movie marked watched."), "{list}");
    assert!(
        screen(&mut app).contains("1999  Movie The Matrix    ■♥✓"),
        "{}",
        screen(&mut app)
    );
    let matrix = tui::app::Target::Title(tui::api::Kind::Movie, 603);
    let show = tui::app::Target::Title(tui::api::Kind::Tv, 1396);
    while tui::ui::selected_target(&app) != Some(matrix) {
        press(&mut app, KeyCode::Char('k'));
    }
    let save = press(&mut app, KeyCode::Char('x'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    assert!(screen(&mut app).contains("The Matrix    ■♥·"));
    let save = press(&mut app, KeyCode::Char('b'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(
        !list.contains("Movie The Matrix"),
        "dropped once saved: {list}"
    );
    assert!(list.contains("Breaking Bad"), "{list}");
    assert!(list.contains("Removed from To watch."), "{list}");
    assert_eq!(tui::ui::selected_target(&app), Some(show));

    // A failed removal keeps the row.
    let save = press(&mut app, KeyCode::Char('b'));
    fulfil(
        &tui::Client::new("http://127.0.0.1:9"),
        &mut app,
        save.into_iter().next().unwrap(),
    )
    .await;
    let list = screen(&mut app);
    assert!(list.contains("Couldn't save:"), "{list}");
    assert!(list.contains("Breaking Bad"), "{list}");

    // Failures leave the state alone and say why.
    press(&mut app, KeyCode::Esc);
    let save = press(&mut app, KeyCode::Char('b'));
    fulfil(
        &tui::Client::new("http://127.0.0.1:9"),
        &mut app,
        save.into_iter().next().unwrap(),
    )
    .await;
    let movie = screen(&mut app);
    assert!(
        movie.contains("Couldn't save: can't reach the credits server at http://127.0.0.1:9"),
        "{movie}"
    );
    assert!(movie.contains("□ To watch b"), "{movie}");

    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE favorites RENAME TO favorites_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let save = press(&mut app, KeyCode::Char('F'));
    fulfil(&client, &mut app, save.into_iter().next().unwrap()).await;
    let movie = screen(&mut app);
    assert!(
        movie.contains("Couldn't save: your favorites couldn't be updated, so nothing was saved"),
        "{movie}"
    );
    assert!(movie.contains("♥ Favorite F"), "{movie}");
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    assert!(screen(&mut app).contains("? Favorite unreadable F"));
    assert!(press(&mut app, KeyCode::Char('F')).is_empty());
    assert!(
        screen(&mut app).contains("Favorites couldn't be read, so it can't be changed; r reloads.")
    );
    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn terminal_client_lists_recently_viewed_pages_on_the_empty_search_screen() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let load = app.load_recent();
    fulfil(&client, &mut app, load).await;
    let empty = screen(&mut app);
    assert!(
        empty.contains("Pages you open will be listed here."),
        "{empty}"
    );
    assert!(press(&mut app, KeyCode::Down).is_empty());
    assert!(app.editing, "nothing to move into");

    // Pages opened on the web are listed, most recent first.
    get_page(t.app(), "/tv/1396", false).await;
    get_page(t.app(), "/movie/603", false).await;
    let load = app.load_recent();
    fulfil(&client, &mut app, load).await;
    let recent = screen(&mut app);
    assert!(recent.contains("Recently viewed"), "{recent}");
    let matrix = recent.find("Movie  The Matrix (1999)").expect(&recent);
    let breaking_bad = recent.find("TV     Breaking Bad (2008)").expect(&recent);
    assert!(matrix < breaking_bad, "{recent}");

    // ↓ moves into them and Enter opens one, like search results.
    assert!(press(&mut app, KeyCode::Down).is_empty());
    assert!(!app.editing);
    press(&mut app, KeyCode::Char('j'));
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Title(tui::api::Kind::Tv, 1396),
            ..
        }]
    ));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;

    // Opening a page here counts too: going back lists it first.
    let back = press(&mut app, KeyCode::Esc);
    assert!(matches!(
        back.as_slice(),
        [tui::app::Request::Recent { .. }]
    ));
    fulfil(&client, &mut app, back.into_iter().next().unwrap()).await;
    let recent = screen(&mut app);
    let breaking_bad = recent.find("TV     Breaking Bad (2008)").expect(&recent);
    let matrix = recent.find("Movie  The Matrix (1999)").expect(&recent);
    assert!(breaking_bad < matrix, "{recent}");

    // From the box, Enter opens the highlighted one.
    press(&mut app, KeyCode::Tab);
    assert!(app.editing);
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [tui::app::Request::Page {
            target: tui::app::Target::Title(tui::api::Kind::Tv, 1396),
            ..
        }]
    ));

    // Typing replaces them with results; clearing the box brings them back.
    let mut app = tui::App::new(&base);
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Backspace);
    let load = app
        .tick(std::time::Instant::now() + tui::app::SEARCH_DEBOUNCE)
        .expect("an emptied box rereads the list");
    assert!(matches!(load, tui::app::Request::Recent { .. }));
    fulfil(&client, &mut app, load).await;
    assert!(screen(&mut app).contains("TV     Breaking Bad (2008)"));
}

#[tokio::test]
async fn terminal_client_shows_an_unreadable_recent_list_as_an_error() {
    let mut app = tui::App::new("http://127.0.0.1:9");
    let offline = tui::Client::new("http://127.0.0.1:9");
    let load = app.load_recent();
    fulfil(&offline, &mut app, load).await;
    let failed = screen(&mut app);
    assert!(
        failed.contains("Can't reach the credits server at http://127.0.0.1:9"),
        "{failed}"
    );
    assert!(failed.contains("Press Enter to try again."), "{failed}");
    assert!(!failed.contains("Pages you open"), "{failed}");
    let retry = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        retry.as_slice(),
        [tui::app::Request::Recent { .. }]
    ));

    // The server's own explanation comes through too.
    let database = temp_database("tui-recent");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query("ALTER TABLE recently_viewed RENAME TO recently_viewed_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let load = app.load_recent();
    fulfil(&client, &mut app, load).await;
    let failed = screen(&mut app);
    assert!(
        failed.contains("Recently viewed pages couldn't be read"),
        "{failed}"
    );
    // Opening a page still works when it can't be recorded.
    let open = app.open(tui::app::Target::Title(tui::api::Kind::Movie, 603));
    fulfil(&client, &mut app, open).await;
    assert!(screen(&mut app).contains("The Matrix"));
    drop(t);
    pool.close().await;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn watched_page_lists_every_mark_and_filters_by_kind_and_source() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "credits-watched-list-{}-{unique}.db",
        std::process::id()
    ));
    let database = path.to_str().unwrap();
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;

    let (status, _, body) = get_page(t.app(), "/watched", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"<a href="/watched">Watched</a>"#),
        "linked from the nav"
    );
    assert!(body.contains("Nothing watched yet."));
    let (_, json) = get_json(t.app(), "/api/watched").await;
    assert_eq!(json["titles"], json!([]));

    // By hand: a movie and two seasons (the second stored directly, so it
    // doesn't roll up to the whole show); from the Apple TV: a movie and an
    // episode; and a show TMDB fails on.
    post_form(t.app(), "/movie/603/watched", "watched=true").await;
    post_form(t.app(), "/tv/1396/season/1/watched", "watched=true").await;
    let pool = credits::cache::ProviderCache::open(database)
        .await
        .unwrap()
        .pool();
    for statement in [
        "INSERT INTO watched_seasons (tv_id, season, watched_at) VALUES (1396, 2, 1)",
        "INSERT INTO watched_movies (movie_id, watched_at, source) VALUES (105, 4000000000, 'apple_tv')",
        "INSERT INTO watched_episodes (tv_id, season, episode, watched_at, source)
         VALUES (1396, 3, 1, 3000000000, 'apple_tv')",
        "INSERT INTO watched_shows (tv_id, watched_at) VALUES (500, 1)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    let (_, json) = get_json(t.app(), "/api/watched").await;
    let ids: Vec<_> = json["titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["kind"].as_str().unwrap(), t["id"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        ids,
        [("movie", 105), ("tv", 1396), ("movie", 603), ("tv", 500)],
        "most recently watched first"
    );
    assert_eq!(json["sort"], "recent");
    assert_eq!(json["unavailable"], 1);
    let show = &json["titles"][1];
    assert_eq!(show["title"], "Breaking Bad");
    assert_eq!(
        show["progress"],
        json!({"show": false, "seasons": [1, 2], "episodes": 1})
    );
    assert_eq!(show["sources"], json!(["manual", "apple_tv"]));
    assert_eq!(json["titles"][0]["sources"], json!(["apple_tv"]));
    assert_eq!(
        json["titles"][0].get("progress"),
        None,
        "movies carry no progress"
    );
    assert_eq!(json["titles"][3]["title"], Value::Null);

    let (_, json) = get_json(t.app(), "/api/watched?kind=movie&source=apple_tv").await;
    assert_eq!(json["titles"].as_array().unwrap().len(), 1);
    assert_eq!(json["titles"][0]["id"], 105);
    let (_, json) = get_json(t.app(), "/api/watched?source=manual&sort=title").await;
    let titles: Vec<_> = json["titles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].clone())
        .collect();
    assert_eq!(
        titles,
        [json!("Breaking Bad"), json!("The Matrix"), Value::Null],
        "A to Z, unnamed last"
    );

    let (_, _, body) = get_page(t.app(), "/watched", false).await;
    assert!(
        body.contains(r#"<a href="/movie/105">Back to the Future</a>"#),
        "{body}"
    );
    assert!(
        body.contains("Seasons 1, 2 · 1 episode · Watched"),
        "{body}"
    );
    assert!(body.contains(r#"<span class="watched-source">Apple TV</span>"#));
    assert!(body.contains(r#"<a href="/tv/500">TV show #500</a>"#));
    assert!(body.contains("Couldn't load the details of 1 title from TMDB right now."));
    assert!(body.contains(
        r#"<a href="/watched?source=apple_tv">Apple TV <span class="count">2</span></a>"#
    ));
    let (_, _, body) = get_page(t.app(), "/watched?kind=tv&source=apple_tv", false).await;
    assert!(body.contains(
        r#"<a href="/watched?kind=movie&#38;source=apple_tv">Movies <span class="count">1</span></a>"#
    ), "{body}");
    assert!(body.contains(r#"<a href="/tv/1396">Breaking Bad</a>"#));
    assert!(!body.contains("The Matrix"));
    assert!(
        !body.contains("Couldn't load the details"),
        "only shown titles count"
    );
    let (_, _, body) = get_page(
        t.app(),
        "/watched?kind=movie&source=manual&sort=title",
        false,
    )
    .await;
    assert!(
        body.contains(r#"<a href="/watched?kind=movie&#38;source=manual">Last watched</a>"#),
        "{body}"
    );

    // Unreadable marks are an error, never an empty list.
    sqlx::query("ALTER TABLE watched_movies RENAME TO watched_movies_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, body) = get_page(t.app(), "/watched", false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        body.contains("Couldn&#39;t load what you&#39;ve watched"),
        "{body}"
    );
    assert!(!body.contains("Nothing watched yet"));
    let (status, json) = get_json(t.app(), "/api/watched").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(json["error"], "watched marks couldn't be read");
    drop(t);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn terminal_client_lists_everything_watched_and_filters_it() {
    let database = temp_database("tui-watched");
    let ttl = Duration::from_secs(3600);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, &database).await;
    post_form(t.app(), "/movie/603/watched", "watched=true").await;
    post_form(t.app(), "/tv/1396/season/1/watched", "watched=true").await;
    let pool = credits::cache::ProviderCache::open(&database)
        .await
        .unwrap()
        .pool();
    sqlx::query(
        "INSERT INTO watched_movies (movie_id, watched_at, source) VALUES (105, 4000000000, 'apple_tv')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    press(&mut app, KeyCode::Esc);
    let open = press(&mut app, KeyCode::Char('v'));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(list.contains("Marked Any way"), "{list}");
    assert!(list.contains("Back to the Future"), "{list}");
    assert!(list.contains("Apple TV"), "{list}");
    assert!(list.contains("Season 1 · "), "{list}");
    assert!(
        list.find("Back to the Future").unwrap() < list.find("The Matrix").unwrap(),
        "most recently watched first: {list}"
    );

    // a: by hand only.
    let by_hand = press(&mut app, KeyCode::Char('a'));
    fulfil(&client, &mut app, by_hand.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(!list.contains("Back to the Future"), "{list}");
    assert!(
        list.contains("The Matrix") && list.contains("Breaking Bad"),
        "{list}"
    );
    // t: movies; a: Apple TV.
    let movies = press(&mut app, KeyCode::Char('t'));
    fulfil(&client, &mut app, movies.into_iter().next().unwrap()).await;
    let apple_tv = press(&mut app, KeyCode::Char('a'));
    fulfil(&client, &mut app, apple_tv.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(list.contains("Back to the Future"), "{list}");
    assert!(!list.contains("The Matrix"), "{list}");
    let open = press(&mut app, KeyCode::Enter);
    assert_eq!(
        open,
        [tui::app::Request::Page {
            seq: match &open[0] {
                tui::app::Request::Page { seq, .. } => *seq,
                _ => unreachable!(),
            },
            target: tui::app::Target::Title(tui::api::Kind::Movie, 105),
        }]
    );
    press(&mut app, KeyCode::Esc);

    // Unreadable marks are an error, never an empty list.
    sqlx::query("ALTER TABLE watched_movies RENAME TO watched_movies_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let list = screen(&mut app);
    assert!(list.contains("Watched marks couldn't be read"), "{list}");
    assert!(!list.contains("Nothing watched yet"), "{list}");
    drop(t);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{database}{suffix}"));
    }
}

#[tokio::test]
async fn terminal_client_opens_full_cast_and_crew_from_title_pages() {
    use tui::api::Kind;
    use tui::app::{Request, Target};
    let base = serve(test_app().await).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let open = app.open(Target::Title(Kind::Movie, 603));
    fulfil(&client, &mut app, open).await;
    let movie = screen(&mut app);
    assert!(movie.contains("Cast  c full cast and crew"), "{movie}");

    let open = press(&mut app, KeyCode::Char('c'));
    assert!(matches!(
        open.as_slice(),
        [Request::Page {
            target: Target::Credits(Kind::Movie, 603),
            ..
        }]
    ));
    fulfil(&client, &mut app, open.into_iter().next().unwrap()).await;
    let credits = screen(&mut app);
    assert!(credits.contains("The Matrix › Cast & crew"), "{credits}");
    assert!(credits.contains("The Matrix (1999)"), "{credits}");
    assert!(credits.contains("Cast (2)"), "{credits}");
    assert!(credits.contains("Laurence Fishburne"), "{credits}");
    assert!(credits.contains("Morpheus"), "{credits}");
    let directing = credits.find("Directing (2)").expect("crew by department");
    let writing = credits.find("Writing (").unwrap();
    assert!(
        credits.find("Cast (2)").unwrap() < directing && directing < writing,
        "{credits}"
    );
    assert!(
        press(&mut app, KeyCode::Char('c')).is_empty(),
        "already on the credits"
    );

    // Enter opens the highlighted person; Esc comes back, then to the title.
    let open = press(&mut app, KeyCode::Enter);
    assert!(matches!(
        open.as_slice(),
        [Request::Page {
            target: Target::Person(..),
            ..
        }]
    ));
    press(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.stack.last().map(|s| s.target),
        Some(Target::Credits(Kind::Movie, 603))
    ));
    press(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.stack.last().map(|s| s.target),
        Some(Target::Title(Kind::Movie, 603))
    ));

    // TV shows have them too.
    let open = app.open(Target::Credits(Kind::Tv, 1396));
    fulfil(&client, &mut app, open).await;
    let tv = screen(&mut app);
    assert!(tv.contains("TV · full cast and crew"), "{tv}");
    assert!(tv.contains("Bryan Cranston"), "{tv}");
}

#[tokio::test]
async fn terminal_client_credits_show_degraded_and_failed_states() {
    use tui::api::Kind;
    use tui::app::Target;
    let t = build_with(
        TOKEN,
        Some(OMDB_KEY),
        Duration::from_secs(3600),
        Duration::ZERO,
    )
    .await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let open = app.open(Target::Credits(Kind::Movie, 603));
    fulfil(&client, &mut app, open).await;
    assert!(!screen(&mut app).contains("TMDB isn't responding"));

    t.tmdb.set_down(true);
    let reload = press(&mut app, KeyCode::Char('r'));
    fulfil(&client, &mut app, reload.into_iter().next().unwrap()).await;
    let stale = screen(&mut app);
    assert!(
        stale.contains("TMDB isn't responding, so this shows details saved on"),
        "{stale}"
    );
    assert!(stale.contains("Laurence Fishburne"), "{stale}");
    t.tmdb.set_down(false);

    let open = app.open(Target::Credits(Kind::Movie, 999));
    fulfil(&client, &mut app, open).await;
    let missing = screen(&mut app);
    assert!(missing.contains("Not found on TMDB."), "{missing}");
    assert!(missing.contains("Cast & crew"), "{missing}");

    let offline = tui::Client::new("http://127.0.0.1:9");
    let mut app = tui::App::new("http://127.0.0.1:9");
    let open = app.open(Target::Credits(Kind::Movie, 603));
    fulfil(&offline, &mut app, open).await;
    let down = screen(&mut app);
    assert!(
        down.contains("Can't reach the credits server at http://127.0.0.1:9"),
        "{down}"
    );
    assert!(down.contains("Press r to try again"), "{down}");
}

#[tokio::test]
async fn terminal_client_filters_and_sorts_person_filmography() {
    use tui::api::Kind;
    use tui::app::{ListView, Request, Target};
    let t = build(TOKEN, Some(OMDB_KEY), Duration::ZERO).await;
    let base = serve(t.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    // Known for stays as it is, as on the web.
    let filmography = |app: &mut tui::App| {
        let page = screen(app);
        page[page.find("Filmography").unwrap()..].to_owned()
    };
    let open = app.open(Target::Person(6384, ListView::default()));
    fulfil(&client, &mut app, open).await;
    let page = screen(&mut app);
    assert!(
        page.contains("Show All · Movies · TV   Sort Date · Rating"),
        "{page}"
    );
    assert!(
        page.contains("Breaking Bad") && page.contains("The Matrix"),
        "{page}"
    );
    assert!(page.contains("◕  83 RT"), "date order is rated too");

    // The kind filter is applied here, without a request.
    assert!(press(&mut app, KeyCode::Char('l')).len() == 1);
    press(&mut app, KeyCode::Esc);
    assert!(press(&mut app, KeyCode::Char('t')).is_empty());
    assert_eq!(
        app.stack.last().unwrap().target,
        Target::Person(
            6384,
            ListView {
                kind: Some(Kind::Movie),
                ..ListView::default()
            }
        )
    );
    let movies = filmography(&mut app);
    assert!(movies.contains("The Matrix"), "{movies}");
    assert!(!movies.contains("Breaking Bad"), "{movies}");
    assert_eq!(app.cursor().selected, 0, "the highlight starts over");
    press(&mut app, KeyCode::Char('t'));
    let tv = filmography(&mut app);
    assert!(
        tv.contains("Breaking Bad") && !tv.contains("The Matrix"),
        "{tv}"
    );

    // The order comes from the server, keeping the filter.
    press(&mut app, KeyCode::Char('t'));
    let requests = press(&mut app, KeyCode::Char('o'));
    let [Request::Page { target, .. }] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert_eq!(
        *target,
        Target::Person(
            6384,
            ListView {
                alt_sort: true,
                ..ListView::default()
            }
        )
    );
    for request in requests {
        fulfil(&client, &mut app, request).await;
    }
    let rated = filmography(&mut app);
    assert!(rated.contains("Sort Date · Rating"), "{rated}");
    assert!(
        rated.contains("●  95 IMDb") && rated.contains("◕  83 RT"),
        "{rated}"
    );
    assert!(
        rated.contains("Credits without a Rotten Tomatoes or IMDb score are listed last"),
        "{rated}"
    );
    assert!(rated.find("Breaking Bad").unwrap() < rated.find("The Matrix").unwrap());

    // Scores that couldn't be refreshed are shown with a warning.
    t.omdb.set_down(true);
    let requests = press(&mut app, KeyCode::Char('r'));
    for request in requests {
        fulfil(&client, &mut app, request).await;
    }
    let stale = screen(&mut app);
    assert!(
        stale.contains("Couldn't refresh some ratings; using scores from as early as"),
        "{stale}"
    );
    assert!(stale.contains("◕  83 RT"), "{stale}");

    // Without a ratings key, credits stay in date order.
    let base = serve(build(TOKEN, None, Duration::from_secs(3600)).await.app()).await;
    let client = tui::Client::new(&base);
    let mut app = tui::App::new(&base);
    let open = app.open(Target::Person(
        6384,
        ListView {
            alt_sort: true,
            ..ListView::default()
        },
    ));
    fulfil(&client, &mut app, open).await;
    let unconfigured = screen(&mut app);
    assert!(
        unconfigured
            .contains("Ratings aren't configured on the server, so credits stay in date order."),
        "{unconfigured}"
    );
}
