//! End-to-end tests against a local fake TMDB server.

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, Request, StatusCode},
    routing::get,
};
use credits::{Catalog, Config, app};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use tower::ServiceExt;

const TOKEN: &str = "test.token.value";

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer test.token.value")
}

async fn fake_tmdb() -> String {
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
                                { "media_type": "person", "id": 6384, "name": "Keanu Reeves" },
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
                if id != 603 {
                    return (StatusCode::NOT_FOUND, Json(json!({ "status_code": 34 })));
                }
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": 603, "title": "The Matrix", "original_title": "The Matrix",
                        "tagline": "Welcome to the Real World.", "overview": "A hacker learns the truth.",
                        "release_date": "1999-03-31", "status": "Released", "runtime": 136,
                        "genres": [{ "id": 28, "name": "Action" }, { "id": 878, "name": "Science Fiction" }],
                        "poster_path": "/matrix.jpg", "backdrop_path": null
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
                (
                    StatusCode::OK,
                    Json(json!({
                        "id": id, "name": "Breaking Bad", "original_name": "Breaking Bad",
                        "tagline": "", "overview": "A teacher turns to crime.",
                        "first_air_date": "2008-01-20", "status": "Ended",
                        "number_of_seasons": 5, "number_of_episodes": 62,
                        "genres": [{ "id": 18, "name": "Drama" }],
                        "poster_path": null, "backdrop_path": "/bb-back.jpg"
                    })),
                )
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    format!("http://{addr}/3")
}

async fn test_app_with_token(token: &str) -> Router {
    let mut config = Config::from_lookup(|key| match key {
        "TMDB_API_TOKEN" => Some(token.to_owned()),
        _ => None,
    })
    .unwrap();
    config.tmdb_api_base = fake_tmdb().await;
    config.tmdb_image_base = "https://img.test/t/p".to_owned();
    app(Catalog::new(&config))
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
async fn home_page_renders_search_form_and_attribution() {
    let (status, _, body) = get_page(test_app().await, "/", false).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<form class="search" action="/" method="get""#));
    assert!(body.contains("Search TMDB for a movie or TV show."));
    assert!(
        body.contains("This product uses the TMDB API but is not endorsed or certified by TMDB.")
    );
    assert!(body.contains("/static/tmdb-logo.svg"));
}

#[tokio::test]
async fn full_page_search_distinguishes_movies_and_tv_and_links_to_detail() {
    let (status, headers, body) = get_page(test_app().await, "/?q=matrix", false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["vary"], "HX-Request");
    assert!(body.starts_with("<!doctype html>"));
    assert!(body.contains(r#"href="/movie/603""#));
    assert!(body.contains(r#"href="/tv/1396""#));
    assert!(body.contains(r#"<span class="badge badge-movie">Movie</span>"#));
    assert!(body.contains(r#"<span class="badge badge-tv">TV show</span>"#));
    assert!(!body.contains("Keanu"), "people are filtered out");
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
    assert!(body.contains("No movies or TV shows match “nothing”."));

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
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["kind"], "movie");
    assert_eq!(results[0]["year"], "1999");
    assert_eq!(results[1]["kind"], "tv");
    assert_eq!(results[1]["overview"], Value::Null);

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
