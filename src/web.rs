//! HTTP layer: full HTML pages (HTMX-enhanced) and a small JSON API, both
//! backed by the same `Catalog`.

use askama::Template;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::{Catalog, CatalogError};
use crate::model::{MediaKind, PersonDetail, SearchHit, SeasonDetail, TitleCredits, TitleDetail};

pub fn app(catalog: Catalog) -> Router {
    Router::new()
        .route("/", get(search_page))
        .route(
            "/movie/{id}",
            get(|s, p| detail_page(s, p, MediaKind::Movie)),
        )
        .route("/tv/{id}", get(|s, p| detail_page(s, p, MediaKind::Tv)))
        .route(
            "/movie/{id}/credits",
            get(|s, p| credits_page(s, p, MediaKind::Movie)),
        )
        .route(
            "/tv/{id}/credits",
            get(|s, p| credits_page(s, p, MediaKind::Tv)),
        )
        .route("/tv/{id}/season/{number}", get(season_page))
        .route("/person/{id}", get(person_page))
        .route("/api/search", get(api_search))
        .route(
            "/api/movie/{id}",
            get(|s, p| api_detail(s, p, MediaKind::Movie)),
        )
        .route("/api/tv/{id}", get(|s, p| api_detail(s, p, MediaKind::Tv)))
        .route(
            "/api/movie/{id}/credits",
            get(|s, p| api_credits(s, p, MediaKind::Movie)),
        )
        .route(
            "/api/tv/{id}/credits",
            get(|s, p| api_credits(s, p, MediaKind::Tv)),
        )
        .route("/api/tv/{id}/season/{number}", get(api_season))
        .route("/api/person/{id}", get(api_person))
        .route(
            "/static/htmx.min.js",
            get(|| asset("text/javascript", include_str!("../static/htmx.min.js"))),
        )
        .route(
            "/static/app.css",
            get(|| asset("text/css", include_str!("../static/app.css"))),
        )
        .route(
            "/static/tmdb-logo.svg",
            get(|| asset("image/svg+xml", include_str!("../static/tmdb-logo.svg"))),
        )
        .fallback(|| async {
            error_page(
                StatusCode::NOT_FOUND,
                "Page not found",
                "There's nothing at this address.",
            )
        })
        .with_state(catalog)
}

pub enum Results {
    Idle,
    Empty,
    Hits(Vec<SearchHit>),
    Failed(&'static str),
}

#[derive(Template)]
#[template(path = "search.html")]
struct SearchPage {
    query: String,
    results: Results,
}

#[derive(Template)]
#[template(path = "_results.html")]
struct ResultsPartial {
    query: String,
    results: Results,
}

#[derive(Template)]
#[template(path = "detail.html")]
struct DetailPage {
    d: TitleDetail,
}

#[derive(Template)]
#[template(path = "credits.html")]
struct CreditsPage {
    c: TitleCredits,
}

#[derive(Template)]
#[template(path = "season.html")]
struct SeasonPage {
    s: SeasonDetail,
}

#[derive(Template)]
#[template(path = "person.html")]
struct PersonPage {
    p: PersonDetail,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    heading: &'static str,
    message: &'static str,
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
}

async fn search_page(
    State(catalog): State<Catalog>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let query = params.q.unwrap_or_default().trim().to_owned();
    let (status, results) = if query.is_empty() {
        (StatusCode::OK, Results::Idle)
    } else {
        match catalog.search(&query).await {
            Ok(hits) if hits.is_empty() => (StatusCode::OK, Results::Empty),
            Ok(hits) => (StatusCode::OK, Results::Hits(hits)),
            Err(e) => {
                tracing::warn!(error = %e, %query, "search failed");
                (status_for(&e), Results::Failed(user_message(&e)))
            }
        }
    };

    let body = if is_htmx_fragment(&headers) {
        ResultsPartial { query, results }.render()
    } else {
        SearchPage { query, results }.render()
    };
    let mut response = html(status, body);
    // Same URL serves a full page or an HTMX fragment.
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("HX-Request"));
    response
}

async fn detail_page(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    kind: MediaKind,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_title(kind);
    };
    let result = catalog.title(kind, id).await.map(|d| DetailPage { d });
    page(result, || not_found_title(kind), "Couldn't load this title")
}

async fn credits_page(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    kind: MediaKind,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_title(kind);
    };
    let result = catalog.credits(kind, id).await.map(|c| CreditsPage { c });
    page(
        result,
        || not_found_title(kind),
        "Couldn't load these credits",
    )
}

async fn season_page(
    State(catalog): State<Catalog>,
    Path((id, number)): Path<(String, String)>,
) -> Response {
    let (Ok(id), Ok(number)) = (id.parse::<u64>(), number.parse::<u32>()) else {
        return not_found_season();
    };
    let result = catalog.season(id, number).await.map(|s| SeasonPage { s });
    page(result, not_found_season, "Couldn't load this season")
}

async fn person_page(State(catalog): State<Catalog>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_person();
    };
    let result = catalog.person(id).await.map(|p| PersonPage { p });
    page(result, not_found_person, "Couldn't load this person")
}

async fn api_search(
    State(catalog): State<Catalog>,
    Query(params): Query<SearchParams>,
) -> Response {
    let query = params.q.unwrap_or_default().trim().to_owned();
    if query.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "query parameter `q` is required");
    }
    match catalog.search(&query).await {
        Ok(results) => Json(json!({ "query": query, "results": results })).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, %query, "api search failed");
            api_error(status_for(&e), user_message(&e))
        }
    }
}

async fn api_detail(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    kind: MediaKind,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    api_result(catalog.title(kind, id).await, "title")
}

async fn api_credits(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    kind: MediaKind,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    api_result(catalog.credits(kind, id).await, "credits")
}

async fn api_season(
    State(catalog): State<Catalog>,
    Path((id, number)): Path<(String, String)>,
) -> Response {
    let (Ok(id), Ok(number)) = (id.parse::<u64>(), number.parse::<u32>()) else {
        return api_not_found();
    };
    api_result(catalog.season(id, number).await, "season")
}

async fn api_person(State(catalog): State<Catalog>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    api_result(catalog.person(id).await, "person")
}

/// Renders a page, or the matching error page when the catalog lookup failed.
fn page(
    result: Result<impl Template, CatalogError>,
    not_found: impl FnOnce() -> Response,
    failure_heading: &'static str,
) -> Response {
    match result {
        Ok(template) => html(StatusCode::OK, template.render()),
        Err(CatalogError::NotFound) => not_found(),
        Err(e) => {
            tracing::warn!(error = %e, failure_heading, "page lookup failed");
            error_page(status_for(&e), failure_heading, user_message(&e))
        }
    }
}

fn api_result(result: Result<impl Serialize, CatalogError>, what: &'static str) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(CatalogError::NotFound) => api_not_found(),
        Err(e) => {
            tracing::warn!(error = %e, what, "api lookup failed");
            api_error(status_for(&e), user_message(&e))
        }
    }
}

fn api_not_found() -> Response {
    api_error(StatusCode::NOT_FOUND, "not found")
}

fn is_htmx_fragment(headers: &HeaderMap) -> bool {
    headers.get("HX-Request").is_some_and(|v| v == "true")
        && !headers.contains_key("HX-History-Restore-Request")
}

fn status_for(error: &CatalogError) -> StatusCode {
    match error {
        CatalogError::NotFound => StatusCode::NOT_FOUND,
        CatalogError::Unauthorized | CatalogError::Provider(_) => StatusCode::BAD_GATEWAY,
    }
}

fn user_message(error: &CatalogError) -> &'static str {
    match error {
        CatalogError::NotFound => "Not found.",
        CatalogError::Unauthorized => {
            "TMDB rejected this server's API token. Check TMDB_API_TOKEN and restart the server."
        }
        CatalogError::Provider(_) => {
            "TMDB isn't responding right now. Please try again in a moment."
        }
    }
}

fn not_found_title(kind: MediaKind) -> Response {
    let message = match kind {
        MediaKind::Movie => "TMDB has no movie with that ID.",
        MediaKind::Tv => "TMDB has no TV show with that ID.",
    };
    error_page(StatusCode::NOT_FOUND, "Title not found", message)
}

fn not_found_season() -> Response {
    error_page(
        StatusCode::NOT_FOUND,
        "Season not found",
        "TMDB has no such season for that TV show.",
    )
}

fn not_found_person() -> Response {
    error_page(
        StatusCode::NOT_FOUND,
        "Person not found",
        "TMDB has no person with that ID.",
    )
}

fn error_page(status: StatusCode, heading: &'static str, message: &'static str) -> Response {
    html(status, ErrorPage { heading, message }.render())
}

fn api_error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn html(status: StatusCode, body: askama::Result<String>) -> Response {
    match body {
        Ok(body) => (status, Html(body)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}

async fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
        .into_response()
}
