//! End-to-end tests against local fake TMDB and OMDb servers.

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, Request, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use credits::{Catalog, Config, app, cache::IN_MEMORY};
use http_body_util::BodyExt;
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
            "/3/movie/{id}",
            get(|Path(id): Path<u64>| async move {
                let imdb_id = match id {
                    603 => "tt0133093",
                    // Listed on OMDb with an IMDb score but no Rotten Tomatoes score.
                    6060 => "tt6060000",
                    _ => return (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 }))),
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
            "/3/tv/{id}/season/{number}",
            get(|Path((id, number)): Path<(u64, u32)>| async move {
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
    let mut config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(token.to_owned()),
        "OMDB_API_KEY" => omdb_key.map(str::to_owned),
        "CREDITS_DB" => Some(database.to_owned()),
        _ => None,
    })
    .unwrap();
    let omdb = FakeOmdb::default();
    let tmdb = FakeTmdb::default();
    config.tmdb_api_base = fake_tmdb(tmdb.clone()).await;
    config.tmdb_image_base = "https://img.test/t/p".to_owned();
    config.omdb_api_base = omdb.start().await;
    config.ratings_ttl = ratings_ttl;
    config.metadata_ttl = metadata_ttl;
    TestApp {
        router: app(Catalog::new(&config).await.unwrap()),
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
    assert!(body.contains("hx-on::before-swap="));
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
    drop(t);
    let t = build_with_database(TOKEN, Some(OMDB_KEY), ttl, ttl, database).await;
    let (_, json) = get_json(t.app(), "/api/favorites").await;
    assert_eq!(json["titles"][0]["id"], 603);
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
    for uri in ["/movie/603", "/person/6384", "/watchlist"] {
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
    sqlx::query("ALTER TABLE favorites_unavailable RENAME TO favorites")
        .execute(&pool)
        .await
        .unwrap();
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
