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
use crossterm::event::{KeyCode, KeyEvent};
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
    down: Arc<AtomicBool>,
}

impl FakeTmdb {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn set_down(&self, down: bool) {
        self.down.store(down, Ordering::SeqCst);
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
            "/3/movie/{id}",
            get(|Path(id): Path<u64>| async move {
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
                                     "watch/providers": { "results": {} } })),
                    );
                }
                let imdb_id = match id {
                    603 => "tt0133093",
                    // Listed on OMDb with an IMDb score but no Rotten Tomatoes score.
                    6060 => "tt6060000",
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
                    if state.down.load(Ordering::SeqCst) {
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
    assert!(body.contains(r#"<a class="brand" href="/">credits</a>"#));
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
async fn movie_prefers_rotten_tomatoes_and_tv_prefers_imdb() {
    let (_, _, body) = get_page(test_app().await, "/movie/603", false).await;
    assert!(body.contains(
        r#"<p class="rating preferred"><strong class="rating-score">83%</strong><img class="rating-source" src="/static/rt-fresh.svg" alt="Rotten Tomatoes: Fresh" title="Rotten Tomatoes: Fresh"></p>"#
    ));
    assert!(body.contains(r#"<span class="rating-score">8.7/10</span><img class="rating-source" src="/static/imdb.svg" alt="IMDb" title="IMDb">"#));
    assert!(!body.contains("73/100"), "unsupported sources are dropped");

    let (_, _, body) = get_page(test_app().await, "/tv/1396", false).await;
    assert!(body.contains(
        r#"<strong class="rating-score">9.5/10</strong><img class="rating-source" src="/static/imdb.svg" alt="IMDb" title="IMDb"> <span class="muted">(2,300,000 votes)</span>"#
    ));
}

#[tokio::test]
async fn missing_preferred_score_is_unavailable_not_substituted() {
    let (_, _, body) = get_page(test_app().await, "/movie/6060", false).await;
    assert!(body.contains("No Rotten Tomatoes score"));
    assert!(
        body.contains(
            r#"<p class="rating other"><span class="rating-score">6.1/10</span><img class="rating-source" src="/static/imdb.svg" alt="IMDb" title="IMDb"></p>"#
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
async fn expired_ratings_are_refetched_and_served_stale_when_omdb_fails() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::ZERO).await;
    get_page(t.app(), "/tv/1396", false).await;
    get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(t.omdb.calls(), 2, "expired entries are refreshed");

    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/tv/1396", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<strong class="rating-score">9.5/10</strong>"#));
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
    assert!(body.contains(r#"<span class="role">62 episodes</span>"#));
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
    assert_eq!(json["cast"][0]["episode_count"], 62);

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
    assert!(body.contains(r#"<strong class="rating-score">9.0/10</strong>"#));
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
    assert!(body.contains(r#"58 min · <span class="episode-rating" tabindex="0" title="Spoiler: select to peek"><img class="rating-source" src="/static/imdb.svg" alt="IMDb" title="IMDb"> <span class="rating-score">9.0/10</span></span>"#));
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
    assert!(body.contains(r#"<span class="rating-score">9.0/10</span>"#));
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
    assert!(body.contains("· 1 episode"));
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

    let (_, json) = get_json(test_app().await, "/api/person/6384").await;
    assert_eq!(json["sort"], "date");
    assert!(json["sort_ratings"].is_null());
    assert!(json["filmography"][0]["entries"][3].get("rating").is_none());

    let (_, _, body) = get_page(test_app().await, "/person/6384?sort=rating", false).await;
    assert!(body.contains(
        r##"<a href="/person/6384?sort=rating#filmography-heading" aria-current="true">Rating</a>"##
    ));
    assert!(body.contains(r##"<a href="/person/6384#filmography-heading">Date</a>"##));
    assert!(body.contains(r#"<img class="rating-source" src="/static/rt-fresh.svg" alt="Rotten Tomatoes: Fresh" title="Rotten Tomatoes: Fresh"> <span class="rating-score">83%</span>"#));
    assert!(body.contains(r#"<span class="rating-score">9.5/10</span>"#));
    assert!(body.contains("Credits without a Rotten Tomatoes or IMDb score are listed last"));
    let filmography = &body[body.find(r#"id="filmography-heading""#).unwrap()..];
    let bb = filmography.find(r#"<a href="/tv/1396">"#).unwrap();
    let matrix = filmography.find(r#"<a href="/movie/603">"#).unwrap();
    assert!(bb < matrix);

    let (_, _, body) = get_page(test_app().await, "/person/6384", false).await;
    assert!(
        body.contains(
            r##"<a href="/person/6384#filmography-heading" aria-current="true">Date</a>"##
        )
    );
    assert!(!body.contains("credit-rating"));
}

#[tokio::test]
async fn rating_sort_is_cached_and_survives_provider_problems() {
    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    get_json(t.app(), "/api/person/6384").await;
    assert_eq!(t.omdb.calls(), 0, "date order looks nothing up");
    get_json(t.app(), "/api/person/6384?sort=rating").await;
    let (omdb, tmdb) = (t.omdb.calls(), t.tmdb.calls());
    get_json(t.app(), "/api/person/6384?sort=rating").await;
    assert_eq!(t.omdb.calls(), omdb, "second sort reuses cached ratings");
    // Only the two titles TMDB doesn't know are asked for again.
    assert_eq!(t.tmdb.calls(), tmdb + 2);

    let t = build(TOKEN, Some(OMDB_KEY), Duration::from_secs(3600)).await;
    t.omdb.set_down(true);
    let (status, _, body) = get_page(t.app(), "/person/6384?sort=rating", false).await;
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
    get_page(t.app(), "/person/6384?sort=rating", false).await;
    t.omdb.set_down(true);
    let (_, _, body) = get_page(t.app(), "/person/6384?sort=rating", false).await;
    assert!(body.contains("Couldn't refresh some ratings; using scores from as early as "));
    assert!(body.contains(r#"<span class="rating-score">83%</span>"#));

    let t = build(TOKEN, None, Duration::from_secs(3600)).await;
    let (status, _, body) = get_page(t.app(), "/person/6384?sort=rating", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(
            "Ratings aren&#39;t configured on this server, so credits stay in date order."
        )
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

    get_page(t.app(), "/person/6384", false).await;
    get_json(t.app(), "/api/person/6384").await;
    assert_eq!(t.tmdb.calls(), 2);
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
    get_page(t.app(), "/person/6384", false).await;
    assert_eq!(t.tmdb.calls(), 2, "expired entries are refreshed");
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
    let (_, _, body) = get_page(t.app(), "/movie/603", false).await;
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
    assert!(body.contains(r#"<span class="rating-score">83%</span>"#));
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
    p.sync().await.unwrap();
    assert!(p.t.catalog.movie_watched(105).await.apple_tv);
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
        Request::Title { seq, kind, id } => (
            seq,
            Response::Title(client.title(kind, id).await.map(Box::new)),
        ),
        Request::Person { seq, id } => {
            (seq, Response::Person(client.person(id).await.map(Box::new)))
        }
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
        [tui::app::Request::Title {
            kind: tui::api::Kind::Movie,
            id: 603,
            ..
        }]
    ));
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

    let open = app.open(tui::app::Target::Person(424242));
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
