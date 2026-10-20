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
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::{Catalog, CatalogError, WatchlistError};
use crate::model::{
    EpisodeDetail, FilmographyEntry, FilmographyGroup, FilmographySort, KindFilter, MediaKind,
    PersonDetail, RecentView, SearchHit, SeasonDetail, TitleCredits, TitleDetail, WatchTarget,
    Watched, WatchlistDetail, WatchlistSort, title_path,
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
        .route(
            "/movie/{id}/watchlist",
            post(|s, p, f| set_on_watchlist(s, p, f, MediaKind::Movie, false)),
        )
        .route(
            "/tv/{id}/watchlist",
            post(|s, p, f| set_on_watchlist(s, p, f, MediaKind::Tv, false)),
        )
        .route(
            "/movie/{id}/favorite",
            post(|s, p, f| set_on_watchlist(s, p, f, MediaKind::Movie, true)),
        )
        .route(
            "/tv/{id}/favorite",
            post(|s, p, f| set_on_watchlist(s, p, f, MediaKind::Tv, true)),
        )
        .route("/watchlist", get(|s, q| watchlist_page(s, q, false)))
        .route("/favorites", get(|s, q| watchlist_page(s, q, true)))
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
        .route("/api/watchlist", get(|s, q| api_watchlist(s, q, false)))
        .route("/api/favorites", get(|s, q| api_watchlist(s, q, true)))
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
    /// On the "To watch" list.
    listed: bool,
    favorited: Option<bool>,
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
    kind: KindFilter,
    /// Departments with credits of `kind`, holding only those credits.
    groups: Vec<FilmographyGroup>,
    /// The department tab shown, an index into `groups`.
    tab: usize,
    /// Titles on the "To watch" list.
    listed: HashSet<(MediaKind, u64)>,
    favorited: Option<HashSet<(MediaKind, u64)>>,
}

impl PersonPage {
    /// Shows the department whose slug is `dept`, or the first one left.
    fn new(
        p: PersonDetail,
        kind: KindFilter,
        dept: Option<&str>,
        listed: HashSet<(MediaKind, u64)>,
        favorited: Option<HashSet<(MediaKind, u64)>>,
    ) -> Self {
        let groups: Vec<FilmographyGroup> = p
            .filmography
            .iter()
            .map(|g| FilmographyGroup {
                department: g.department.clone(),
                entries: g
                    .entries
                    .iter()
                    .filter(|e| kind.matches(e.kind))
                    .cloned()
                    .collect(),
            })
            .filter(|g| !g.entries.is_empty())
            .collect();
        let tab = dept
            .and_then(|d| groups.iter().position(|g| g.slug() == d))
            .unwrap_or(0);
        PersonPage {
            p,
            kind,
            groups,
            tab,
            listed,
            favorited,
        }
    }

    fn is_listed(&self, e: &FilmographyEntry) -> bool {
        self.listed.contains(&(e.kind, e.id))
    }

    fn is_favorite(&self, e: &FilmographyEntry) -> Option<bool> {
        self.favorited
            .as_ref()
            .map(|titles| titles.contains(&(e.kind, e.id)))
    }

    fn favorite_back_to(&self, e: &FilmographyEntry) -> String {
        self.back_to(e).replace("#watchlist-", "#favorite-")
    }

    /// This page as it is, scrolled to `e`'s row, for its watchlist toggle.
    fn back_to(&self, e: &FilmographyEntry) -> String {
        let page = self.href(self.p.sort, self.kind, self.current_dept());
        let page = page.split_once('#').map_or(page.as_str(), |(page, _)| page);
        format!("{page}#{}", watchlist_toggle_id(e))
    }

    fn shown(&self) -> Option<&FilmographyGroup> {
        self.groups.get(self.tab)
    }

    /// Credits of `kind` across every department.
    fn kind_count(&self, kind: &KindFilter) -> usize {
        self.p
            .filmography
            .iter()
            .flat_map(|g| &g.entries)
            .filter(|e| kind.matches(e.kind))
            .count()
    }

    fn tab_href(&self, index: &usize) -> String {
        let dept = (*index > 0).then(|| self.groups[*index].slug());
        self.href(self.p.sort, self.kind, dept)
    }

    /// Keeps the current tab; a kind without that department shows the first.
    fn kind_href(&self, kind: &KindFilter) -> String {
        self.href(self.p.sort, *kind, self.current_dept())
    }

    fn sort_href(&self, sort: FilmographySort) -> String {
        self.href(sort, self.kind, self.current_dept())
    }

    fn current_dept(&self) -> Option<String> {
        (self.tab > 0).then(|| self.groups[self.tab].slug())
    }

    fn href(&self, sort: FilmographySort, kind: KindFilter, dept: Option<String>) -> String {
        let mut query = Vec::new();
        if sort.is_rating() {
            query.push("sort=rating".to_owned());
        }
        if let Some(kind) = kind.query() {
            query.push(format!("kind={kind}"));
        }
        if let Some(dept) = dept {
            query.push(format!("dept={dept}"));
        }
        let query = if query.is_empty() {
            String::new()
        } else {
            format!("?{}", query.join("&"))
        };
        format!("{}{query}#filmography-heading", self.p.path())
    }
}

#[derive(Template)]
#[template(path = "watchlist.html")]
struct WatchlistPage {
    w: WatchlistDetail,
    kind: KindFilter,
    /// The titles of `kind`, in the list's order.
    shown: Vec<FilmographyEntry>,
    favorites: bool,
    listed: HashSet<(MediaKind, u64)>,
    favorited: Option<HashSet<(MediaKind, u64)>>,
}

impl WatchlistPage {
    fn new(
        w: WatchlistDetail,
        kind: KindFilter,
        favorites: bool,
        listed: HashSet<(MediaKind, u64)>,
        favorited: Option<HashSet<(MediaKind, u64)>>,
    ) -> Self {
        let shown = w
            .titles
            .iter()
            .filter(|e| kind.matches(e.kind))
            .cloned()
            .collect();
        WatchlistPage {
            w,
            kind,
            shown,
            favorites,
            listed,
            favorited,
        }
    }

    fn label(&self) -> &'static str {
        if self.favorites {
            "Favorites"
        } else {
            "To watch"
        }
    }

    fn is_listed(&self, e: &FilmographyEntry) -> bool {
        self.listed.contains(&(e.kind, e.id))
    }

    fn is_favorite(&self, e: &FilmographyEntry) -> Option<bool> {
        self.favorited
            .as_ref()
            .map(|titles| titles.contains(&(e.kind, e.id)))
    }

    fn kind_count(&self, kind: &KindFilter) -> usize {
        self.w
            .titles
            .iter()
            .filter(|e| kind.matches(e.kind))
            .count()
    }

    fn kind_href(&self, kind: &KindFilter) -> String {
        watchlist_href(self.w.sort, *kind, self.favorites)
    }

    fn sort_href(&self, sort: WatchlistSort) -> String {
        watchlist_href(sort, self.kind, self.favorites)
    }

    /// Where a removal returns to: this page as it is.
    fn back(&self) -> String {
        watchlist_href(self.w.sort, self.kind, self.favorites)
    }
}

fn watchlist_href(sort: WatchlistSort, kind: KindFilter, favorites: bool) -> String {
    let path = if favorites {
        "/favorites"
    } else {
        "/watchlist"
    };
    let mut query = Vec::new();
    if sort.is_rating() {
        query.push("sort=rating".to_owned());
    }
    if let Some(kind) = kind.query() {
        query.push(format!("kind={kind}"));
    }
    if query.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", query.join("&"))
    }
}

/// The element id of a title's watchlist toggle, unique on any page.
fn watchlist_toggle_id(e: &FilmographyEntry) -> String {
    format!("watchlist-{}-{}", e.kind.slug(), e.id)
}

fn favorite_toggle_id(e: &FilmographyEntry) -> String {
    format!("favorite-{}-{}", e.kind.slug(), e.id)
}

/// Unknown state disables hearts instead of offering a misleading toggle.
async fn favorite_titles(catalog: &Catalog) -> Option<HashSet<(MediaKind, u64)>> {
    match catalog.favorite_titles().await {
        Ok(titles) => Some(titles),
        Err(e) => {
            tracing::warn!(error = %e, "reading favorites failed");
            None
        }
    }
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
    /// Page only: "movie" or "tv" narrows the filmography.
    kind: Option<String>,
    /// Page only: the department tab to show, by `FilmographyGroup::slug`.
    dept: Option<String>,
}

#[derive(Deserialize)]
struct WatchlistParams {
    sort: Option<String>,
    kind: Option<String>,
}

#[derive(Deserialize)]
struct WatchlistForm {
    listed: bool,
    /// Page to return to; only paths on this site are honored.
    back: Option<String>,
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
    let listed = result.is_ok() && catalog.watchlist_titles().await.contains(&(kind, id));
    let favorited = favorite_titles(&catalog)
        .await
        .map(|titles| titles.contains(&(kind, id)));
    page(
        result.map(|d| DetailPage {
            d,
            watched,
            listed,
            favorited,
        }),
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
    let listed = match &result {
        Ok(p) => {
            catalog.record_view(RecentView::of_person(p)).await;
            catalog.watchlist_titles().await
        }
        Err(_) => HashSet::new(),
    };
    let favorited = favorite_titles(&catalog).await;
    page(
        result.map(|p| {
            let kind = KindFilter::from_query(params.kind.as_deref());
            PersonPage::new(p, kind, params.dept.as_deref(), listed, favorited)
        }),
        not_found_person,
        "Couldn't load this person",
    )
}

async fn set_on_watchlist(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Form(form): Form<WatchlistForm>,
    kind: MediaKind,
    favorites: bool,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_title(kind);
    };
    let result = match (favorites, form.listed) {
        (true, true) => catalog.add_favorite(kind, id).await,
        (false, true) => catalog.add_to_watchlist(kind, id).await,
        (true, false) => catalog
            .remove_favorite(kind, id)
            .await
            .map_err(WatchlistError::from),
        (false, false) => catalog
            .remove_from_watchlist(kind, id)
            .await
            .map_err(WatchlistError::from),
    };
    match result {
        Ok(()) => {
            let back = form
                .back
                .filter(|back| is_local_path(back))
                .unwrap_or_else(|| title_path(kind, id));
            Redirect::to(&back).into_response()
        }
        Err(WatchlistError::Title(CatalogError::NotFound)) => not_found_title(kind),
        Err(WatchlistError::Title(e)) => {
            tracing::warn!(error = %e, ?kind, id, "title lookup for watchlist failed");
            error_page(
                status_for(&e),
                if favorites {
                    "Couldn't add to your Favorites"
                } else {
                    "Couldn't add to your To watch list"
                },
                user_message(&e),
            )
        }
        Err(e @ WatchlistError::Database(_)) => {
            tracing::warn!(error = %e, ?kind, id, "saving watchlist failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                if favorites {
                    "Couldn't update your Favorites"
                } else {
                    "Couldn't update your To watch list"
                },
                "The database couldn't be updated. Please try again.",
            )
        }
    }
}

/// A path on this site, so a form can't redirect elsewhere.
fn is_local_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
}

async fn watchlist_page(
    State(catalog): State<Catalog>,
    Query(params): Query<WatchlistParams>,
    favorites: bool,
) -> Response {
    let sort = WatchlistSort::from_query(params.sort.as_deref());
    let kind = KindFilter::from_query(params.kind.as_deref());
    let result = if favorites {
        catalog.favorites(sort).await
    } else {
        catalog.watchlist(sort).await
    };
    match result {
        Ok(w) => {
            let listed = catalog.watchlist_titles().await;
            let favorited = favorite_titles(&catalog).await;
            html(
                StatusCode::OK,
                WatchlistPage::new(w, kind, favorites, listed, favorited).render(),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "reading the watchlist failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                if favorites {
                    "Couldn't load your Favorites"
                } else {
                    "Couldn't load your To watch list"
                },
                "The database couldn't be read. Please try again.",
            )
        }
    }
}

async fn api_watchlist(
    State(catalog): State<Catalog>,
    Query(params): Query<WatchlistParams>,
    favorites: bool,
) -> Response {
    let sort = WatchlistSort::from_query(params.sort.as_deref());
    let kind = KindFilter::from_query(params.kind.as_deref());
    let result = if favorites {
        catalog.favorites(sort).await
    } else {
        catalog.watchlist(sort).await
    };
    match result {
        Ok(mut w) => {
            w.titles.retain(|e| kind.matches(e.kind));
            Json(w).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "api watchlist read failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                if favorites {
                    "favorites couldn't be read"
                } else {
                    "the watchlist couldn't be read"
                },
            )
        }
    }
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
