//! HTTP layer: full HTML pages (HTMX-enhanced) and a small JSON API, both
//! backed by the same `Catalog`.

use askama::Template;
use axum::{
    Json, Router,
    extract::{Form, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::{Catalog, CatalogError};
use crate::model::{
    EpisodeDetail, FilmographySort, MediaKind, PersonDetail, RecentView, SearchHit, SeasonDetail,
    TitleCredits, TitleDetail, WatchTarget, Watched, title_path,
};

pub fn app(catalog: Catalog) -> Router {
    Router::new()
        .route("/", get(search_page))
        .route(
            "/about",
            get(|| async { html(StatusCode::OK, AboutPage.render()) }),
        )
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
        .route("/tv/{id}/watched", post(set_show_watched))
        .route("/tv/{id}/season/{number}", get(season_page))
        .route("/tv/{id}/season/{number}/watched", post(set_season_watched))
        .route(
            "/tv/{id}/season/{number}/episode/{episode}",
            get(episode_page),
        )
        .route(
            "/tv/{id}/season/{number}/episode/{episode}/watched",
            post(set_episode_watched),
        )
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
        .route(
            "/api/tv/{id}/season/{number}/episode/{episode}",
            get(api_episode),
        )
        .route("/api/person/{id}", get(api_person))
        .route(
            "/static/htmx.min.js",
            get(|| asset("text/javascript", include_str!("../static/htmx.min.js"))),
        )
        .route("/static/app.css", get(|| asset("text/css", APP_CSS)))
        .route(
            "/static/tmdb-logo.svg",
            get(|| asset("image/svg+xml", include_str!("../static/tmdb-logo.svg"))),
        )
        .route(
            "/static/favicon.svg",
            get(|| asset("image/svg+xml", include_str!("../static/favicon.svg"))),
        )
        .route(
            "/static/imdb.svg",
            get(|| asset("image/svg+xml", include_str!("../static/imdb.svg"))),
        )
        .route(
            "/static/rt-fresh.svg",
            get(|| asset("image/svg+xml", include_str!("../static/rt-fresh.svg"))),
        )
        .route(
            "/static/rt-rotten.svg",
            get(|| asset("image/svg+xml", include_str!("../static/rt-rotten.svg"))),
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
    /// No query yet: recently viewed pages, if any.
    Idle(Vec<RecentView>),
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
#[template(path = "about.html")]
struct AboutPage;

#[derive(Template)]
#[template(path = "detail.html")]
struct DetailPage {
    d: TitleDetail,
    /// Always unwatched for movies.
    watched: Watched,
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
    watched: Watched,
}

#[derive(Template)]
#[template(path = "episode.html")]
struct EpisodePage {
    e: EpisodeDetail,
    watched: Watched,
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

#[derive(Deserialize)]
struct PersonParams {
    sort: Option<String>,
}

#[derive(Deserialize)]
struct WatchedForm {
    watched: bool,
    /// Page to return to; only pages of the same show are honored.
    back: Option<String>,
}

async fn search_page(
    State(catalog): State<Catalog>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Response {
    let query = params.q.unwrap_or_default().trim().to_owned();
    let (status, results) = if query.is_empty() {
        (
            StatusCode::OK,
            Results::Idle(catalog.recently_viewed().await),
        )
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
    let result = catalog.title(kind, id).await;
    if let Ok(d) = &result {
        catalog.record_view(RecentView::of_title(d)).await;
    }
    let watched = match kind {
        MediaKind::Tv if result.is_ok() => catalog.watched(id).await,
        _ => Watched::default(),
    };
    page(
        result.map(|d| DetailPage { d, watched }),
        || not_found_title(kind),
        "Couldn't load this title",
    )
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
    let result = catalog.season(id, number).await;
    let watched = match &result {
        Ok(_) => catalog.watched(id).await,
        Err(_) => Watched::default(),
    };
    page(
        result.map(|s| SeasonPage { s, watched }),
        not_found_season,
        "Couldn't load this season",
    )
}

async fn episode_page(
    State(catalog): State<Catalog>,
    Path((id, season, number)): Path<(String, String, String)>,
) -> Response {
    let Some((id, season, number)) = episode_ids(&id, &season, &number) else {
        return not_found_episode();
    };
    let result = catalog.episode(id, season, number).await;
    let watched = match &result {
        Ok(_) => catalog.watched(id).await,
        Err(_) => Watched::default(),
    };
    page(
        result.map(|e| EpisodePage { e, watched }),
        not_found_episode,
        "Couldn't load this episode",
    )
}

async fn set_show_watched(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Form(form): Form<WatchedForm>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_title(MediaKind::Tv);
    };
    let back = form
        .back
        .filter(|back| is_show_page(back, id))
        .unwrap_or_else(|| title_path(MediaKind::Tv, id));
    save_watched(&catalog, id, WatchTarget::Show, form.watched, &back).await
}

async fn set_season_watched(
    State(catalog): State<Catalog>,
    Path((id, number)): Path<(String, String)>,
    Form(form): Form<WatchedForm>,
) -> Response {
    let (Ok(id), Ok(number)) = (id.parse::<u64>(), number.parse::<u32>()) else {
        return not_found_season();
    };
    let back = format!("/tv/{id}/season/{number}");
    save_watched(
        &catalog,
        id,
        WatchTarget::Season(number),
        form.watched,
        &back,
    )
    .await
}

async fn set_episode_watched(
    State(catalog): State<Catalog>,
    Path((id, season, number)): Path<(String, String, String)>,
    Form(form): Form<WatchedForm>,
) -> Response {
    let Some((id, season, number)) = episode_ids(&id, &season, &number) else {
        return not_found_season();
    };
    // Back to the episode's own page if that's where the form was, else its
    // place on the season page.
    let episode_page = format!("/tv/{id}/season/{season}/episode/{number}");
    let back = if form.back.as_deref() == Some(episode_page.as_str()) {
        episode_page
    } else {
        format!("/tv/{id}/season/{season}#episode-{number}")
    };
    let target = WatchTarget::Episode { season, number };
    save_watched(&catalog, id, target, form.watched, &back).await
}

/// Saves a watched mark, then sends the browser back to `back`.
async fn save_watched(
    catalog: &Catalog,
    id: u64,
    target: WatchTarget,
    watched: bool,
    back: &str,
) -> Response {
    match catalog.set_watched(id, target, watched).await {
        Ok(()) => Redirect::to(back).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, id, ?target, "saving watched state failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't save watched state",
                "The database couldn't be updated. Please try again.",
            )
        }
    }
}

/// The show's own page or one of its subpages, so a form can't redirect elsewhere.
fn is_show_page(path: &str, id: u64) -> bool {
    let show = title_path(MediaKind::Tv, id);
    path.strip_prefix(&show).is_some_and(|rest| {
        (rest.is_empty() || rest.starts_with('/'))
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'/')
            && !rest.contains("//")
    })
}

async fn person_page(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Query(params): Query<PersonParams>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_person();
    };
    let sort = FilmographySort::from_query(params.sort.as_deref());
    let result = catalog.person(id, sort).await;
    if let Ok(p) = &result {
        catalog.record_view(RecentView::of_person(p)).await;
    }
    page(
        result.map(|p| PersonPage { p }),
        not_found_person,
        "Couldn't load this person",
    )
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

async fn api_episode(
    State(catalog): State<Catalog>,
    Path((id, season, number)): Path<(String, String, String)>,
) -> Response {
    let Some((id, season, number)) = episode_ids(&id, &season, &number) else {
        return api_not_found();
    };
    api_result(catalog.episode(id, season, number).await, "episode")
}

async fn api_person(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Query(params): Query<PersonParams>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let sort = FilmographySort::from_query(params.sort.as_deref());
    api_result(catalog.person(id, sort).await, "person")
}

fn episode_ids(id: &str, season: &str, number: &str) -> Option<(u64, u32, u32)> {
    Some((id.parse().ok()?, season.parse().ok()?, number.parse().ok()?))
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

fn not_found_episode() -> Response {
    error_page(
        StatusCode::NOT_FOUND,
        "Episode not found",
        "TMDB has no such episode for that TV show.",
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

const APP_CSS: &str = include_str!("../static/app.css");

/// Content hash of `app.css`, used as a query string by `base.html` so a
/// stylesheet change gets a new URL instead of a cached stale copy.
pub const APP_CSS_VERSION: u64 = fnv1a(APP_CSS.as_bytes());

const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    hash
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
