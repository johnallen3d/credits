//! End-to-end tests against local fake TMDB and OMDb servers.

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, Request, StatusCode},
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
                                      "poster_path": "/matrix.jpg" },
                                    { "id": 603, "media_type": "movie", "title": "The Matrix",
                                      "release_date": "1999-03-31", "character": "Thomas Anderson",
                                      "poster_path": "/matrix.jpg" },
                                    { "id": 999001, "media_type": "movie", "title": "Untitled Project",
                                      "release_date": "", "character": null, "poster_path": null },
                                    { "id": 1396, "media_type": "tv", "name": "Breaking Bad",
                                      "first_air_date": "2008-01-20", "character": "Himself",
                                      "episode_count": 1 }
                                ],
                                "crew": [
                                    { "id": 777, "media_type": "movie", "title": "Man of Tai Chi",
                                      "release_date": "2013-07-05", "department": "Directing",
                                      "job": "Director" }
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
    let mut config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(token.to_owned()),
        "OMDB_API_KEY" => omdb_key.map(str::to_owned),
        "CREDITS_DB" => Some(IN_MEMORY.to_owned()),
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

async fn get_json(app: Router, uri: &str) -> (StatusCode, Value) {
    let (status, _, body) = get_page(app, uri, false).await;
    (status, serde_json::from_str(&body).unwrap())
}

#[tokio::test]
async fn home_page_renders_search_form_and_site_nav() {
    let (status, _, body) = get_page(test_app().await, "/", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<form class="search" action="/" method="get""#));
    assert!(body.contains("Search TMDB for a movie, TV show or person."));
    assert!(body.contains(r#"<nav class="site-nav" aria-label="Site">"#));
    assert!(body.contains(r#"<a href="/">Search</a>"#));
    assert!(body.contains(r#"<a href="/about">About</a>"#));
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
    assert!(body.contains(r#"<a href="/movie/777">Man of Tai Chi</a>"#));
    assert!(body.contains(r#"<img class="poster thumb" src="https://img.test/t/p/w92/matrix.jpg" alt="" loading="lazy""#));
    assert!(body.contains(r#"<span class="poster thumb missing">No artwork</span>"#));
    let acting = body.find(r#"id="dept-acting""#).unwrap();
    let directing = body.find(r#"id="dept-directing""#).unwrap();
    assert!(acting < directing, "known-for department first");
    let tba = body.find("TBA").unwrap();
    let matrix = body.find(r#"<a href="/movie/603">"#).unwrap();
    assert!(tba < matrix, "undated entries lead, then newest first");

    let (status, json) = get_json(test_app().await, "/api/person/6384").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["metadata"]["state"], "fresh");
    let acting = &json["filmography"][0];
    assert_eq!(acting["department"], "Acting");
    assert_eq!(acting["entries"].as_array().unwrap().len(), 3);
    assert_eq!(acting["entries"][1]["kind"], "tv");
    assert_eq!(acting["entries"][2]["role"], "Neo / Thomas Anderson");
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
