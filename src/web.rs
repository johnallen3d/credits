//! HTTP layer: full HTML pages (HTMX-enhanced) and a small JSON API, both
//! backed by the same `Catalog`.

use askama::Template;
use axum::{
    Json, Router,
    extract::{Form, Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use utoipa::{IntoResponses, OpenApi, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::catalog::{Catalog, CatalogError, RecentError, ServicesError, WatchlistError};
use crate::model::{
    EpisodeDetail, EpisodeWithWatched, FilmographyEntry, FilmographyGroup, FilmographySort,
    KindFilter, MediaKind, Metadata, MovieWatched, MovieWithMarks, PersonDetail, RecentKind,
    RecentView, RecentViews, ReleaseFilter, ReleaseList, ReleasesDetail, SearchHit, SeasonDetail,
    SeasonWithWatched, ServicesDetail, ServicesFilter, SourceFilter, TitleCredits, TitleDetail,
    TitleMarksUpdate, TvWithWatched, WatchTarget, Watched, WatchedList, WatchedMarks, WatchedSort,
    WatchedSource, WatchedTitle, WatchlistDetail, WatchlistSort, encode_query_value, title_path,
};
use crate::playback::{App, Kind, LiveStatus, PlaybackDetail};

pub fn app(catalog: Catalog) -> Router {
    let (api, spec) = api().split_for_parts();
    let spec = spec.to_pretty_json().expect("the OpenAPI spec serializes");
    Router::new()
        .route("/", get(search_page))
        .route(
            "/about",
            get(|State(catalog): State<Catalog>| async move {
                html(
                    StatusCode::OK,
                    AboutPage {
                        live: catalog.playback_live(),
                    }
                    .render(),
                )
            }),
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
        .route("/movie/{id}/watched", post(set_movie_watched))
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
        .route("/watched", get(watched_page))
        .route(
            "/now-playing",
            get(|s, q| releases_page(s, q, ReleaseList::NowPlaying)),
        )
        .route(
            "/coming-soon",
            get(|s, q| releases_page(s, q, ReleaseList::ComingSoon)),
        )
        .route("/services", get(services_page).post(save_services))
        .route("/playback", get(playback_page))
        .route("/playback/decide", post(decide_playback_show))
        .route("/person/{id}", get(person_page))
        .route("/person/{id}/ratings", get(person_ratings))
        .merge(api)
        .route(
            "/api/openapi.json",
            get(|| async move { ([(header::CONTENT_TYPE, "application/json")], spec) }),
        )
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

/// The JSON API. Each handler's `#[utoipa::path]` both routes it and
/// describes it, so the spec can't drift from what is served.
fn api() -> OpenApiRouter<Catalog> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(api_search))
        .routes(routes!(api_recent, api_record_view))
        .routes(routes!(api_movie))
        .routes(routes!(api_tv))
        .routes(routes!(api_movie_credits))
        .routes(routes!(api_tv_credits))
        .routes(routes!(api_season))
        .routes(routes!(api_episode))
        .routes(routes!(api_person))
        .routes(routes!(api_watchlist))
        .routes(routes!(api_favorites))
        .routes(routes!(api_watched))
        .routes(routes!(api_services))
        .routes(routes!(api_playback))
        .routes(routes!(api_now_playing))
        .routes(routes!(api_coming_soon))
        .routes(routes!(api_movie_watchlist))
        .routes(routes!(api_tv_watchlist))
        .routes(routes!(api_movie_favorite))
        .routes(routes!(api_tv_favorite))
        .routes(routes!(api_movie_watched))
        .routes(routes!(api_tv_watched))
        .routes(routes!(api_season_watched))
        .routes(routes!(api_episode_watched))
}

/// The OpenAPI document for the JSON API, as committed in `openapi.json`.
pub fn openapi() -> utoipa::openapi::OpenApi {
    api().into_openapi()
}

#[derive(OpenApi)]
#[openapi(info(
    title = "credits",
    description = "JSON twins of the credits pages. No authentication; the server binds to loopback.",
    license(name = "MIT", identifier = "MIT")
))]
struct ApiDoc;

/// Every JSON error response.
#[derive(Serialize, ToSchema)]
struct ApiError {
    error: String,
}

/// The ways a TMDB-backed lookup fails.
#[derive(IntoResponses)]
#[allow(dead_code)] // Only describes responses for the spec.
enum LookupErrors {
    /// TMDB has nothing with that ID, or the ID isn't a number.
    #[response(status = 404)]
    NotFound(#[to_schema] ApiError),
    /// TMDB rejected the token, or failed with no cached copy to fall back on.
    #[response(status = 502)]
    Provider(#[to_schema] ApiError),
}

/// The ways saving a list or watched mark fails.
#[derive(IntoResponses)]
#[allow(dead_code)] // Only describes responses for the spec.
enum WriteErrors {
    /// TMDB has nothing with that ID, or an ID isn't a number.
    #[response(status = 404)]
    NotFound(#[to_schema] ApiError),
    /// The database couldn't be read or updated; nothing was saved.
    #[response(status = 500)]
    Database(#[to_schema] ApiError),
    /// Adding needs the title from TMDB, which rejected the token or failed
    /// with no cached copy to fall back on.
    #[response(status = 502)]
    Provider(#[to_schema] ApiError),
}

/// Puts a title on a list or takes it off.
#[derive(Deserialize, ToSchema)]
struct ListedBody {
    listed: bool,
}

/// A title or person page a client opened.
#[derive(Deserialize, ToSchema)]
struct ViewedBody {
    kind: RecentKind,
    id: u64,
}

/// Marks something watched or not.
#[derive(Deserialize, ToSchema)]
struct WatchedBody {
    watched: bool,
}

/// The database couldn't be read.
#[derive(IntoResponses)]
#[allow(dead_code)] // Only describes responses for the spec.
enum ReadError {
    #[response(status = 500)]
    Database(#[to_schema] ApiError),
}

#[derive(Serialize, ToSchema)]
struct SearchResults {
    /// The trimmed query.
    query: String,
    results: Vec<SearchHit>,
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
    saved: SavedTitles,
}

#[derive(Template)]
#[template(path = "_results.html")]
struct ResultsPartial {
    query: String,
    results: Results,
    saved: SavedTitles,
}

/// Which titles are on the To watch and Favorites lists, for toggles on
/// search results. `favorited` is `None` when Favorites couldn't be read.
struct SavedTitles {
    listed: HashSet<(MediaKind, u64)>,
    favorited: Option<HashSet<(MediaKind, u64)>>,
}

impl SavedTitles {
    fn is_listed(&self, kind: &MediaKind, id: &u64) -> bool {
        self.listed.contains(&(*kind, *id))
    }

    fn is_favorite(&self, kind: &MediaKind, id: &u64) -> Option<bool> {
        self.favorited
            .as_ref()
            .map(|titles| titles.contains(&(*kind, *id)))
    }

    /// The search for `query`, scrolled to the toggle `id`.
    fn back(&self, query: &str, id: &str) -> String {
        format!("/?q={}#{id}", encode_query_value(query))
    }
}

#[derive(Template)]
#[template(path = "about.html")]
struct AboutPage {
    live: LiveStatus,
}

#[derive(Template)]
#[template(path = "detail.html")]
struct DetailPage {
    d: TitleDetail,
    /// Always unwatched for movies.
    watched: Watched,
    /// Always unwatched for TV shows.
    movie_watched: MovieWatched,
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

/// A person page's ratings, swapped into the page once they've loaded.
#[derive(Template)]
#[template(path = "person_ratings.html")]
struct PersonRatings(PersonPage);

impl std::ops::Deref for PersonRatings {
    type Target = PersonPage;

    fn deref(&self) -> &PersonPage {
        &self.0
    }
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

    /// Where this page, as it is, loads its ratings from.
    fn ratings_href(&self) -> String {
        let page = self.href(self.p.sort, self.kind, self.current_dept());
        let page = page.split_once('#').map_or(page.as_str(), |(page, _)| page);
        match page.split_once('?') {
            Some((path, query)) => format!("{path}/ratings?{query}"),
            None => format!("{page}/ratings"),
        }
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
    /// To watch only; always `Any` for Favorites.
    services: ServicesFilter,
    /// To watch only; always `All` for Favorites.
    release: ReleaseFilter,
    /// The titles of `kind`, `services` and `release`, in the list's order.
    shown: Vec<FilmographyEntry>,
    favorites: bool,
    listed: HashSet<(MediaKind, u64)>,
    favorited: Option<HashSet<(MediaKind, u64)>>,
}

impl WatchlistPage {
    #[allow(clippy::too_many_arguments)]
    fn new(
        w: WatchlistDetail,
        kind: KindFilter,
        services: ServicesFilter,
        release: ReleaseFilter,
        favorites: bool,
        listed: HashSet<(MediaKind, u64)>,
        favorited: Option<HashSet<(MediaKind, u64)>>,
    ) -> Self {
        // Without chosen services there's nothing to filter by.
        let services = if w.services.as_ref().is_some_and(|s| s.state.is_chosen()) {
            services
        } else {
            ServicesFilter::Any
        };
        let release = if favorites {
            ReleaseFilter::All
        } else {
            release
        };
        let shown = w
            .titles
            .iter()
            .filter(|e| kind.matches(e.kind) && services.matches(e) && release.matches(e))
            .cloned()
            .collect();
        WatchlistPage {
            w,
            kind,
            services,
            release,
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

    /// Titles matching `kind`, `services` and `release`.
    fn count(&self, kind: KindFilter, services: ServicesFilter, release: ReleaseFilter) -> usize {
        self.w
            .titles
            .iter()
            .filter(|e| kind.matches(e.kind) && services.matches(e) && release.matches(e))
            .count()
    }

    /// Titles of `kind` within the other current filters.
    fn kind_count(&self, kind: &KindFilter) -> usize {
        self.count(*kind, self.services, self.release)
    }

    /// Titles within `services` and the other current filters.
    fn services_count(&self, services: ServicesFilter) -> usize {
        self.count(self.kind, services, self.release)
    }

    /// Titles within `release` and the other current filters.
    fn release_count(&self, release: &ReleaseFilter) -> usize {
        self.count(self.kind, self.services, *release)
    }

    fn href(
        &self,
        sort: WatchlistSort,
        kind: KindFilter,
        services: ServicesFilter,
        release: ReleaseFilter,
    ) -> String {
        watchlist_href(sort, kind, services, release, self.favorites)
    }

    fn kind_href(&self, kind: &KindFilter) -> String {
        self.href(self.w.sort, *kind, self.services, self.release)
    }

    fn sort_href(&self, sort: WatchlistSort) -> String {
        self.href(sort, self.kind, self.services, self.release)
    }

    fn services_href(&self, services: ServicesFilter) -> String {
        self.href(self.w.sort, self.kind, services, self.release)
    }

    fn release_href(&self, release: &ReleaseFilter) -> String {
        self.href(self.w.sort, self.kind, self.services, *release)
    }

    /// Where a removal returns to: this page as it is.
    fn back(&self) -> String {
        self.href(self.w.sort, self.kind, self.services, self.release)
    }

    /// Titles on the list whose release dates couldn't be loaded.
    fn release_unknown(&self) -> usize {
        self.w.release_unknown.unwrap_or(0)
    }

    /// Says what the filters left out, e.g. "None of the movies on your list are out now."
    fn empty_message(&self) -> String {
        let titles = match self.kind {
            KindFilter::All => "titles",
            KindFilter::Movie => "movies",
            KindFilter::Tv => "TV shows",
        };
        let mut what = Vec::new();
        if self.services.is_mine() {
            what.push("on your services");
        }
        match self.release {
            ReleaseFilter::All => {}
            ReleaseFilter::OutNow => what.push("out now"),
            ReleaseFilter::ComingSoon => what.push("coming soon"),
        }
        if what.is_empty() {
            format!("No {titles} on your list.")
        } else {
            format!(
                "None of the {titles} on your list are {}.",
                what.join(" and ")
            )
        }
    }
}

fn watchlist_href(
    sort: WatchlistSort,
    kind: KindFilter,
    services: ServicesFilter,
    release: ReleaseFilter,
    favorites: bool,
) -> String {
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
    if services.is_mine() {
        query.push("services=mine".to_owned());
    }
    if let Some(release) = release.query() {
        query.push(format!("release={release}"));
    }
    if query.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", query.join("&"))
    }
}

#[derive(Template)]
#[template(path = "watched.html")]
struct WatchedPage {
    w: WatchedList,
    kind: KindFilter,
    source: SourceFilter,
    /// The titles of `kind` and `source`, in the list's order.
    shown: Vec<WatchedTitle>,
}

impl WatchedPage {
    fn new(w: WatchedList, kind: KindFilter, source: SourceFilter) -> Self {
        let shown = w
            .titles
            .iter()
            .filter(|t| kind.matches(t.kind) && source.matches(t))
            .cloned()
            .collect();
        WatchedPage {
            w,
            kind,
            source,
            shown,
        }
    }

    /// Titles of `kind` within the current source filter.
    fn kind_count(&self, kind: &KindFilter) -> usize {
        self.w
            .titles
            .iter()
            .filter(|t| kind.matches(t.kind) && self.source.matches(t))
            .count()
    }

    /// Titles of the current kind marked the `source` way.
    fn source_count(&self, source: &SourceFilter) -> usize {
        self.w
            .titles
            .iter()
            .filter(|t| self.kind.matches(t.kind) && source.matches(t))
            .count()
    }

    fn kind_href(&self, kind: &KindFilter) -> String {
        watched_href(self.w.sort, *kind, self.source)
    }

    fn source_href(&self, source: &SourceFilter) -> String {
        watched_href(self.w.sort, self.kind, *source)
    }

    fn sort_href(&self, sort: WatchedSort) -> String {
        watched_href(sort, self.kind, self.source)
    }

    /// Shown titles TMDB couldn't name.
    fn unavailable(&self) -> usize {
        self.shown.iter().filter(|t| t.title.is_none()).count()
    }

    /// Says what the filters left out, e.g. "No TV shows marked from the Apple TV."
    fn empty_message(&self) -> String {
        let titles = match self.kind {
            KindFilter::All => "titles",
            KindFilter::Movie => "movies",
            KindFilter::Tv => "TV shows",
        };
        let how = match self.source {
            SourceFilter::Any => " watched",
            SourceFilter::Manual => " marked watched by hand",
            SourceFilter::AppleTv => " marked watched from the Apple TV",
        };
        format!("No {titles}{how}.")
    }
}

fn watched_href(sort: WatchedSort, kind: KindFilter, source: SourceFilter) -> String {
    let mut query = Vec::new();
    if sort.is_title() {
        query.push("sort=title".to_owned());
    }
    if let Some(kind) = kind.query() {
        query.push(format!("kind={kind}"));
    }
    if let Some(source) = source.query() {
        query.push(format!("source={source}"));
    }
    if query.is_empty() {
        "/watched".to_owned()
    } else {
        format!("/watched?{}", query.join("&"))
    }
}

/// The element id of a title's watchlist toggle, unique on any page.
fn watchlist_toggle_id(e: &FilmographyEntry) -> String {
    title_toggle_id("watchlist", &e.kind, &e.id)
}

fn favorite_toggle_id(e: &FilmographyEntry) -> String {
    title_toggle_id("favorite", &e.kind, &e.id)
}

/// The element id of a filmography row's rating, filled in once it loads.
fn rating_slot_id(e: &FilmographyEntry) -> String {
    title_toggle_id("rating", &e.kind, &e.id)
}

/// The element id of a title's toggle for `list` ("watchlist" or "favorite").
fn title_toggle_id(list: &str, kind: &MediaKind, id: &u64) -> String {
    format!("{list}-{}-{id}", kind.slug())
}

/// The original upload behind a sized TMDB image URL
/// (`…/t/p/w342/x.jpg` to `…/t/p/original/x.jpg`), for viewing it full size.
fn full_size(url: &str) -> String {
    let Some((sized_dir, file)) = url.rsplit_once('/') else {
        return url.to_owned();
    };
    let Some((base, _size)) = sized_dir.rsplit_once('/') else {
        return url.to_owned();
    };
    format!("{base}/original/{file}")
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
#[template(path = "services.html")]
struct ServicesPage {
    s: ServicesDetail,
    /// After an HTMX save: confirms it beside the status.
    saved: bool,
}

#[derive(Template)]
#[template(path = "playback.html")]
struct PlaybackPage {
    p: PlaybackDetail,
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
    /// To watch only: "mine" shows titles on the user's services.
    services: Option<String>,
    /// To watch only: "out_now" or "coming_soon".
    release: Option<String>,
}

#[derive(Deserialize)]
struct WatchedParams {
    sort: Option<String>,
    kind: Option<String>,
    /// "manual" or "apple_tv": only titles with a mark made that way.
    source: Option<String>,
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

    // Only title hits have toggles, so the lists are read only for them.
    let saved = match &results {
        Results::Hits(hits) if hits.iter().any(|h| h.title().is_some()) => SavedTitles {
            listed: catalog.watchlist_titles().await,
            favorited: favorite_titles(&catalog).await,
        },
        _ => SavedTitles {
            listed: HashSet::new(),
            favorited: Some(HashSet::new()),
        },
    };
    let body = if is_htmx_fragment(&headers) {
        ResultsPartial {
            query,
            results,
            saved,
        }
        .render()
    } else {
        SearchPage {
            query,
            results,
            saved,
        }
        .render()
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
    let movie_watched = match kind {
        MediaKind::Movie if result.is_ok() => catalog.movie_watched(id).await,
        _ => MovieWatched::default(),
    };
    let listed = result.is_ok() && catalog.watchlist_titles().await.contains(&(kind, id));
    let favorited = favorite_titles(&catalog)
        .await
        .map(|titles| titles.contains(&(kind, id)));
    page(
        result.map(|d| DetailPage {
            d,
            watched,
            movie_watched,
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

async fn set_movie_watched(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Form(form): Form<WatchedForm>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_title(MediaKind::Movie);
    };
    match catalog.set_movie_watched(id, form.watched).await {
        Ok(()) => Redirect::to(&title_path(MediaKind::Movie, id)).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, id, "saving watched state failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't save watched state",
                "The database couldn't be updated. Please try again.",
            )
        }
    }
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
    // Rating every credit can take seconds, so the page loads its ratings
    // after it shows, unless there is no key and so nothing to wait for.
    let mut result = catalog.person_unrated(id, sort).await;
    if let Ok(p) = &mut result {
        if !catalog.ratings_configured() {
            catalog.rate_filmography(p).await;
        }
        catalog.record_view(RecentView::of_person(p)).await;
    }
    person_template(&catalog, result, &params, |page| page).await
}

/// The ratings a person page loads after it shows: its rating notes, and
/// the rated rows of the department shown, or the whole list when sorted
/// by rating, all swapped in out of band.
async fn person_ratings(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    Query(params): Query<PersonParams>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return not_found_person();
    };
    let sort = FilmographySort::from_query(params.sort.as_deref());
    let result = catalog.person(id, sort).await;
    person_template(&catalog, result, &params, PersonRatings).await
}

async fn person_template<T: Template>(
    catalog: &Catalog,
    result: Result<PersonDetail, CatalogError>,
    params: &PersonParams,
    template: impl FnOnce(PersonPage) -> T,
) -> Response {
    let listed = match &result {
        Ok(_) => catalog.watchlist_titles().await,
        Err(_) => HashSet::new(),
    };
    let favorited = favorite_titles(catalog).await;
    page(
        result.map(|p| {
            let kind = KindFilter::from_query(params.kind.as_deref());
            template(PersonPage::new(
                p,
                kind,
                params.dept.as_deref(),
                listed,
                favorited,
            ))
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
    let services = ServicesFilter::from_query(params.services.as_deref());
    let release = ReleaseFilter::from_query(params.release.as_deref());
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
                WatchlistPage::new(w, kind, services, release, favorites, listed, favorited)
                    .render(),
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

/// The To watch list.
#[utoipa::path(
    get,
    path = "/api/watchlist",
    params(
        ("kind" = Option<MediaKind>, Query, description = "Only movies or only TV; anything else means all"),
        ("sort" = Option<WatchlistSort>, Query, description = "Defaults to `added`"),
        ("services" = Option<String>, Query, description = "`mine`: only titles on your services, once any are chosen"),
        ("release" = Option<String>, Query, description = "`out_now`: only titles watchable at home today; `coming_soon`: only those with a release still ahead. Titles whose dates couldn't be loaded match neither"),
    ),
    responses((status = OK, body = WatchlistDetail), ReadError)
)]
async fn api_watchlist(state: State<Catalog>, query: Query<WatchlistParams>) -> Response {
    api_list(state, query, false).await
}

/// Favorites.
#[utoipa::path(
    get,
    path = "/api/favorites",
    params(
        ("kind" = Option<MediaKind>, Query, description = "Only movies or only TV; anything else means all"),
        ("sort" = Option<WatchlistSort>, Query, description = "Defaults to `added`"),
    ),
    responses((status = OK, body = WatchlistDetail), ReadError)
)]
async fn api_favorites(state: State<Catalog>, query: Query<WatchlistParams>) -> Response {
    api_list(state, query, true).await
}

async fn api_list(
    State(catalog): State<Catalog>,
    Query(params): Query<WatchlistParams>,
    favorites: bool,
) -> Response {
    let sort = WatchlistSort::from_query(params.sort.as_deref());
    let kind = KindFilter::from_query(params.kind.as_deref());
    let services = ServicesFilter::from_query(params.services.as_deref());
    let release = ReleaseFilter::from_query(params.release.as_deref());
    let result = if favorites {
        catalog.favorites(sort).await
    } else {
        catalog.watchlist(sort).await
    };
    match result {
        Ok(mut w) => {
            let chosen = w.services.as_ref().is_some_and(|s| s.state.is_chosen());
            w.titles.retain(|e| {
                kind.matches(e.kind)
                    && (!chosen || services.matches(e))
                    && (favorites || release.matches(e))
            });
            catalog.mark_entries(&mut w.titles).await;
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

#[derive(Template)]
#[template(path = "releases.html")]
struct ReleasesPage {
    detail: ReleasesDetail,
    metadata: Metadata,
}

#[derive(Deserialize)]
struct PageParams {
    page: Option<String>,
}

impl PageParams {
    /// The requested page; the first when missing or not a number.
    fn page(&self) -> u32 {
        self.page
            .as_deref()
            .and_then(|p| p.trim().parse().ok())
            .unwrap_or(1)
    }
}

async fn releases_page(
    State(catalog): State<Catalog>,
    Query(params): Query<PageParams>,
    list: ReleaseList,
) -> Response {
    match catalog.releases(list, params.page()).await {
        Ok((detail, metadata)) => html(StatusCode::OK, ReleasesPage { detail, metadata }.render()),
        Err(e) => {
            tracing::warn!(error = %e, ?list, "reading releases failed");
            error_page(
                status_for(&e),
                match list {
                    ReleaseList::NowPlaying => "Couldn't load Now playing",
                    ReleaseList::ComingSoon => "Couldn't load Coming soon",
                },
                user_message(&e),
            )
        }
    }
}

/// Movies now playing in the region's theaters, most popular first, with
/// their ratings.
#[utoipa::path(
    get,
    path = "/api/now-playing",
    params(("page" = Option<u32>, Query, description = "From 1, 20 movies each; defaults to 1, and a page past the end gives the last")),
    responses((status = OK, body = ReleasesDetail), ReadError)
)]
async fn api_now_playing(
    State(catalog): State<Catalog>,
    Query(params): Query<PageParams>,
) -> Response {
    api_releases(&catalog, ReleaseList::NowPlaying, params.page()).await
}

/// Movies coming to the region's theaters soon, soonest first, with their
/// ratings.
#[utoipa::path(
    get,
    path = "/api/coming-soon",
    params(("page" = Option<u32>, Query, description = "From 1, 20 movies each; defaults to 1, and a page past the end gives the last")),
    responses((status = OK, body = ReleasesDetail), ReadError)
)]
async fn api_coming_soon(
    State(catalog): State<Catalog>,
    Query(params): Query<PageParams>,
) -> Response {
    api_releases(&catalog, ReleaseList::ComingSoon, params.page()).await
}

async fn api_releases(catalog: &Catalog, list: ReleaseList, page: u32) -> Response {
    api_result(
        catalog.releases(list, page).await.map(|(detail, _)| detail),
        "releases",
    )
}

async fn watched_page(
    State(catalog): State<Catalog>,
    Query(params): Query<WatchedParams>,
) -> Response {
    let sort = WatchedSort::from_query(params.sort.as_deref());
    let kind = KindFilter::from_query(params.kind.as_deref());
    let source = SourceFilter::from_query(params.source.as_deref());
    match catalog.watched_titles(sort).await {
        Ok(w) => html(StatusCode::OK, WatchedPage::new(w, kind, source).render()),
        Err(e) => {
            tracing::warn!(error = %e, "reading watched marks failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't load what you've watched",
                "The database couldn't be read. Please try again.",
            )
        }
    }
}

/// Every watched movie and TV show, however it was marked.
#[utoipa::path(
    get,
    path = "/api/watched",
    params(
        ("kind" = Option<MediaKind>, Query, description = "Only movies or only TV; anything else means all"),
        ("source" = Option<WatchedSource>, Query, description = "Only titles with a mark made this way; anything else means any"),
        ("sort" = Option<WatchedSort>, Query, description = "Defaults to `recent`"),
    ),
    responses((status = OK, body = WatchedList), ReadError)
)]
async fn api_watched(
    State(catalog): State<Catalog>,
    Query(params): Query<WatchedParams>,
) -> Response {
    let sort = WatchedSort::from_query(params.sort.as_deref());
    let kind = KindFilter::from_query(params.kind.as_deref());
    let source = SourceFilter::from_query(params.source.as_deref());
    match catalog.watched_titles(sort).await {
        Ok(mut w) => {
            w.titles
                .retain(|t| kind.matches(t.kind) && source.matches(t));
            w.unavailable = w.titles.iter().filter(|t| t.title.is_none()).count();
            Json(w).into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "api watched read failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "watched marks couldn't be read",
            )
        }
    }
}

async fn services_page(State(catalog): State<Catalog>) -> Response {
    match catalog.services().await {
        Ok(s) => html(StatusCode::OK, ServicesPage { s, saved: false }.render()),
        Err(e) => {
            tracing::warn!(error = %e, "reading your services failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't load your services",
                "The database couldn't be read. Please try again.",
            )
        }
    }
}

async fn playback_page(State(catalog): State<Catalog>) -> Response {
    html(
        StatusCode::OK,
        PlaybackPage {
            p: catalog.playback().await,
        }
        .render(),
    )
}

#[derive(Deserialize)]
struct DecideForm {
    app: String,
    /// "episode" (the default: `show` is a show) or "movie".
    kind: Option<String>,
    show: String,
    /// "use" (the chosen `tmdb_id`, or `other_id` if given) or "ignore".
    action: String,
    tmdb_id: Option<String>,
    other_id: Option<String>,
}

/// Sets which TMDB show or movie something seen on the Apple TV is, or
/// ignores it.
async fn decide_playback_show(
    State(catalog): State<Catalog>,
    Form(form): Form<DecideForm>,
) -> Response {
    let Some(app) = App::from_id(&form.app) else {
        return error_page(
            StatusCode::BAD_REQUEST,
            "Unknown app",
            "That app isn't tracked.",
        );
    };
    let Some(kind) = Kind::from_id(form.kind.as_deref().unwrap_or("episode")) else {
        return error_page(
            StatusCode::BAD_REQUEST,
            "Unknown kind",
            "Only shows and movies are matched.",
        );
    };
    let id = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::parse::<u64>)
    };
    let tmdb_id = match form.action.as_str() {
        "ignore" => None,
        _ => match id(&form.other_id).or_else(|| id(&form.tmdb_id)) {
            Some(Ok(id)) => Some(id),
            Some(Err(_)) => {
                return error_page(
                    StatusCode::BAD_REQUEST,
                    "Not a TMDB ID",
                    match kind {
                        Kind::Episode => {
                            "A TMDB TV ID is a number, like the 1396 in themoviedb.org/tv/1396."
                        }
                        Kind::Movie => {
                            "A TMDB movie ID is a number, like the 105 in themoviedb.org/movie/105."
                        }
                    },
                );
            }
            None => {
                let (title, message) = match kind {
                    Kind::Episode => (
                        "Choose a show",
                        "Pick one of the TMDB shows, enter a TMDB TV ID, or ignore the show.",
                    ),
                    Kind::Movie => (
                        "Choose a movie",
                        "Pick one of the TMDB movies, enter a TMDB movie ID, or ignore the movie.",
                    ),
                };
                return error_page(StatusCode::BAD_REQUEST, title, message);
            }
        },
    };
    match catalog
        .decide_playback_show(app, kind, &form.show, tmdb_id)
        .await
    {
        Ok(()) => Redirect::to("/playback").into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "saving a playback show choice failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't save your choice",
                "The database couldn't be written. Please try again.",
            )
        }
    }
}

/// Replaces the user's services with the checked `service` boxes. HTMX
/// saves on each change and gets the page back to refresh the status.
async fn save_services(
    State(catalog): State<Catalog>,
    headers: HeaderMap,
    Form(form): Form<Vec<(String, String)>>,
) -> Response {
    let ids = form
        .iter()
        .filter(|(key, _)| key == "service")
        .filter_map(|(_, id)| id.parse().ok())
        .collect();
    match catalog.set_services(&ids).await {
        Ok(()) if is_htmx_fragment(&headers) => match catalog.services().await {
            Ok(s) => html(StatusCode::OK, ServicesPage { s, saved: true }.render()),
            Err(e) => {
                // The save went through; only the status can't be refreshed.
                tracing::warn!(error = %e, "reading your saved services failed");
                Html(r#"<div id="services-status" class="services-status" role="status"><p><span class="saved">Saved.</span> Reload to see your services.</p></div>"#).into_response()
            }
        },
        Ok(()) => Redirect::to("/services").into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "saving your services failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Couldn't save your services",
                match e {
                    ServicesError::Read(_) => {
                        "Your current services couldn't be read, so nothing was changed. Please try again."
                    }
                    ServicesError::Write(_) => {
                        "The database couldn't be updated. Please try again."
                    }
                },
            )
        }
    }
}

/// Streaming services for `CREDITS_REGION`, grouped, with the ones you chose.
#[utoipa::path(
    get,
    path = "/api/services",
    responses((status = OK, body = ServicesDetail), ReadError)
)]
async fn api_services(State(catalog): State<Catalog>) -> Response {
    match catalog.services().await {
        Ok(s) => Json(s).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "api services read failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "your services couldn't be read",
            )
        }
    }
}

/// Movies, TV shows and people matching a query. Never cached.
#[utoipa::path(
    get,
    path = "/api/search",
    params(("q" = String, Query, description = "What to search for")),
    responses(
        (status = OK, body = SearchResults),
        (status = BAD_REQUEST, description = "`q` is missing or blank", body = ApiError),
        (status = BAD_GATEWAY, description = "TMDB rejected the token or isn't responding", body = ApiError),
    )
)]
async fn api_search(
    State(catalog): State<Catalog>,
    Query(params): Query<SearchParams>,
) -> Response {
    let query = params.q.unwrap_or_default().trim().to_owned();
    if query.is_empty() {
        return api_error(StatusCode::BAD_REQUEST, "query parameter `q` is required");
    }
    match catalog.search(&query).await {
        Ok(results) => Json(SearchResults { query, results }).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, %query, "api search failed");
            api_error(status_for(&e), user_message(&e))
        }
    }
}

/// Recently viewed title and person pages, most recent first, as listed on
/// the home page.
#[utoipa::path(
    get,
    path = "/api/recent",
    responses((status = OK, body = RecentViews), ReadError)
)]
async fn api_recent(State(catalog): State<Catalog>) -> Response {
    match catalog.read_recently_viewed().await {
        Ok(recent) => Json(RecentViews { recent }).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "api recent views read failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "recently viewed pages couldn't be read",
            )
        }
    }
}

/// Records that a title or person page was opened, as opening its web page
/// does; the other `/api/…` reads never count. Answers with the new list.
#[utoipa::path(
    post,
    path = "/api/recent",
    request_body = ViewedBody,
    responses((status = OK, body = RecentViews), WriteErrors)
)]
async fn api_record_view(
    State(catalog): State<Catalog>,
    body: Result<Json<ViewedBody>, JsonRejection>,
) -> Response {
    let body = match body {
        Ok(Json(body)) => body,
        Err(e) => return api_error(e.status(), &e.body_text()),
    };
    match catalog.view(body.kind, body.id).await {
        Ok(recent) => Json(RecentViews { recent }).into_response(),
        Err(RecentError::Page(CatalogError::NotFound)) => api_not_found(),
        Err(RecentError::Page(e)) => {
            tracing::warn!(error = %e, kind = ?body.kind, id = body.id, "api view lookup failed");
            api_error(status_for(&e), user_message(&e))
        }
        Err(e @ RecentError::Database(_)) => {
            tracing::warn!(error = %e, kind = ?body.kind, id = body.id, "api view save failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "recently viewed pages couldn't be updated, so nothing was saved",
            )
        }
    }
}

/// A movie: metadata, key crew, cast, ratings and where to watch.
#[utoipa::path(
    get,
    path = "/api/movie/{id}",
    params(("id" = u64, Path, description = "TMDB movie ID")),
    responses((status = OK, body = MovieWithMarks), LookupErrors)
)]
async fn api_movie(State(catalog): State<Catalog>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let result = match catalog.title(MediaKind::Movie, id).await {
        Ok(title) => Ok(MovieWithMarks {
            title,
            marks: catalog.title_marks(MediaKind::Movie, id).await,
        }),
        Err(e) => Err(e),
    };
    api_result(result, "title")
}

/// A TV show: metadata, key crew, cast, seasons, ratings, where to watch and
/// watched marks.
#[utoipa::path(
    get,
    path = "/api/tv/{id}",
    params(("id" = u64, Path, description = "TMDB TV ID")),
    responses((status = OK, body = TvWithWatched), LookupErrors)
)]
async fn api_tv(State(catalog): State<Catalog>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let result = match catalog.title(MediaKind::Tv, id).await {
        Ok(title) => Ok(TvWithWatched {
            title,
            watched: catalog.watched_marks(id).await,
            marks: catalog.title_marks(MediaKind::Tv, id).await,
        }),
        Err(e) => Err(e),
    };
    api_result(result, "title")
}

/// A movie's whole cast and crew by department.
#[utoipa::path(
    get,
    path = "/api/movie/{id}/credits",
    params(("id" = u64, Path, description = "TMDB movie ID")),
    responses((status = OK, body = TitleCredits), LookupErrors)
)]
async fn api_movie_credits(state: State<Catalog>, id: Path<String>) -> Response {
    api_credits(state, id, MediaKind::Movie).await
}

/// A TV show's whole cast and crew by department.
#[utoipa::path(
    get,
    path = "/api/tv/{id}/credits",
    params(("id" = u64, Path, description = "TMDB TV ID")),
    responses((status = OK, body = TitleCredits), LookupErrors)
)]
async fn api_tv_credits(state: State<Catalog>, id: Path<String>) -> Response {
    api_credits(state, id, MediaKind::Tv).await
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

/// A season's episodes with air dates, IMDb episode scores and the show's
/// watched marks.
#[utoipa::path(
    get,
    path = "/api/tv/{id}/season/{number}",
    params(
        ("id" = u64, Path, description = "TMDB TV ID"),
        ("number" = u32, Path, description = "Season number; 0 is specials"),
    ),
    responses((status = OK, body = SeasonWithWatched), LookupErrors)
)]
async fn api_season(
    State(catalog): State<Catalog>,
    Path((id, number)): Path<(String, String)>,
) -> Response {
    let (Ok(id), Ok(number)) = (id.parse::<u64>(), number.parse::<u32>()) else {
        return api_not_found();
    };
    let result = match catalog.season(id, number).await {
        Ok(season) => Ok(SeasonWithWatched {
            season,
            watched: catalog.watched_marks(id).await,
        }),
        Err(e) => Err(e),
    };
    api_result(result, "season")
}

/// An episode: guest stars, regulars credited on it, crew, neighbours and the
/// show's watched marks.
#[utoipa::path(
    get,
    path = "/api/tv/{id}/season/{number}/episode/{episode}",
    params(
        ("id" = u64, Path, description = "TMDB TV ID"),
        ("number" = u32, Path, description = "Season number; 0 is specials"),
        ("episode" = u32, Path, description = "Episode number within the season"),
    ),
    responses((status = OK, body = EpisodeWithWatched), LookupErrors)
)]
async fn api_episode(
    State(catalog): State<Catalog>,
    Path((id, season, number)): Path<(String, String, String)>,
) -> Response {
    let Some((id, season, number)) = episode_ids(&id, &season, &number) else {
        return api_not_found();
    };
    let result = match catalog.episode(id, season, number).await {
        Ok(episode) => Ok(EpisodeWithWatched {
            episode,
            watched: catalog.watched_marks(id).await,
        }),
        Err(e) => Err(e),
    };
    api_result(result, "episode")
}

/// A person: known for, and filmography by department.
#[utoipa::path(
    get,
    path = "/api/person/{id}",
    params(
        ("id" = u64, Path, description = "TMDB person ID"),
        ("sort" = Option<FilmographySort>, Query, description = "Defaults to `date`"),
    ),
    responses((status = OK, body = PersonDetail), LookupErrors)
)]
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

/// Puts a movie on To watch or takes it off.
#[utoipa::path(
    put,
    path = "/api/movie/{id}/watchlist",
    params(("id" = u64, Path, description = "TMDB movie ID")),
    request_body = ListedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_movie_watchlist(
    state: State<Catalog>,
    id: Path<String>,
    body: Result<Json<ListedBody>, JsonRejection>,
) -> Response {
    api_set_listed(state, id, body, MediaKind::Movie, false).await
}

/// Puts a TV show on To watch or takes it off.
#[utoipa::path(
    put,
    path = "/api/tv/{id}/watchlist",
    params(("id" = u64, Path, description = "TMDB TV ID")),
    request_body = ListedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_tv_watchlist(
    state: State<Catalog>,
    id: Path<String>,
    body: Result<Json<ListedBody>, JsonRejection>,
) -> Response {
    api_set_listed(state, id, body, MediaKind::Tv, false).await
}

/// Hearts a movie or unhearts it.
#[utoipa::path(
    put,
    path = "/api/movie/{id}/favorite",
    params(("id" = u64, Path, description = "TMDB movie ID")),
    request_body = ListedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_movie_favorite(
    state: State<Catalog>,
    id: Path<String>,
    body: Result<Json<ListedBody>, JsonRejection>,
) -> Response {
    api_set_listed(state, id, body, MediaKind::Movie, true).await
}

/// Hearts a TV show, marking its aired seasons watched, or unhearts it,
/// keeping what's watched.
#[utoipa::path(
    put,
    path = "/api/tv/{id}/favorite",
    params(("id" = u64, Path, description = "TMDB TV ID")),
    request_body = ListedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_tv_favorite(
    state: State<Catalog>,
    id: Path<String>,
    body: Result<Json<ListedBody>, JsonRejection>,
) -> Response {
    api_set_listed(state, id, body, MediaKind::Tv, true).await
}

async fn api_set_listed(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    body: Result<Json<ListedBody>, JsonRejection>,
    kind: MediaKind,
    favorites: bool,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let listed = match body {
        Ok(Json(body)) => body.listed,
        Err(e) => return api_error(e.status(), &e.body_text()),
    };
    let result = match (favorites, listed) {
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
        Ok(()) => api_title_marks(&catalog, kind, id).await,
        Err(WatchlistError::Title(CatalogError::NotFound)) => api_not_found(),
        Err(WatchlistError::Title(e)) => {
            tracing::warn!(error = %e, ?kind, id, "api title lookup for list failed");
            api_error(status_for(&e), user_message(&e))
        }
        Err(e @ WatchlistError::Database(_)) => {
            tracing::warn!(error = %e, ?kind, id, "api list save failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                if favorites {
                    "your favorites couldn't be updated, so nothing was saved"
                } else {
                    "the watchlist couldn't be updated, so nothing was saved"
                },
            )
        }
    }
}

/// Marks a movie watched or not. Marking takes it off To watch; unmarking
/// one the Apple TV marked sticks.
#[utoipa::path(
    put,
    path = "/api/movie/{id}/watched",
    params(("id" = u64, Path, description = "TMDB movie ID")),
    request_body = WatchedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_movie_watched(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    body: Result<Json<WatchedBody>, JsonRejection>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let watched = match body {
        Ok(Json(body)) => body.watched,
        Err(e) => return api_error(e.status(), &e.body_text()),
    };
    match catalog.set_movie_watched(id, watched).await {
        Ok(()) => api_title_marks(&catalog, MediaKind::Movie, id).await,
        Err(e) => api_watched_failed(e, id),
    }
}

/// Marks a whole TV show watched or not. Marking takes it off To watch.
#[utoipa::path(
    put,
    path = "/api/tv/{id}/watched",
    params(("id" = u64, Path, description = "TMDB TV ID")),
    request_body = WatchedBody,
    responses((status = OK, body = TitleMarksUpdate), WriteErrors)
)]
async fn api_tv_watched(
    State(catalog): State<Catalog>,
    Path(id): Path<String>,
    body: Result<Json<WatchedBody>, JsonRejection>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return api_not_found();
    };
    let watched = match body {
        Ok(Json(body)) => body.watched,
        Err(e) => return api_error(e.status(), &e.body_text()),
    };
    match catalog.set_watched(id, WatchTarget::Show, watched).await {
        Ok(()) => api_title_marks(&catalog, MediaKind::Tv, id).await,
        Err(e) => api_watched_failed(e, id),
    }
}

/// Marks a season watched or not, answering with the show's watched marks.
#[utoipa::path(
    put,
    path = "/api/tv/{id}/season/{number}/watched",
    params(
        ("id" = u64, Path, description = "TMDB TV ID"),
        ("number" = u32, Path, description = "Season number; 0 is specials"),
    ),
    request_body = WatchedBody,
    responses((status = OK, body = WatchedMarks), WriteErrors)
)]
async fn api_season_watched(
    state: State<Catalog>,
    Path((id, number)): Path<(String, String)>,
    body: Result<Json<WatchedBody>, JsonRejection>,
) -> Response {
    let (Ok(id), Ok(number)) = (id.parse::<u64>(), number.parse::<u32>()) else {
        return api_not_found();
    };
    api_set_watched(state, id, WatchTarget::Season(number), body).await
}

/// Marks an episode watched or not, answering with the show's watched marks.
/// Unmarking one the Apple TV marked sticks.
#[utoipa::path(
    put,
    path = "/api/tv/{id}/season/{number}/episode/{episode}/watched",
    params(
        ("id" = u64, Path, description = "TMDB TV ID"),
        ("number" = u32, Path, description = "Season number; 0 is specials"),
        ("episode" = u32, Path, description = "Episode number within the season"),
    ),
    request_body = WatchedBody,
    responses((status = OK, body = WatchedMarks), WriteErrors)
)]
async fn api_episode_watched(
    state: State<Catalog>,
    Path((id, season, number)): Path<(String, String, String)>,
    body: Result<Json<WatchedBody>, JsonRejection>,
) -> Response {
    let Some((id, season, number)) = episode_ids(&id, &season, &number) else {
        return api_not_found();
    };
    api_set_watched(state, id, WatchTarget::Episode { season, number }, body).await
}

async fn api_set_watched(
    State(catalog): State<Catalog>,
    id: u64,
    target: WatchTarget,
    body: Result<Json<WatchedBody>, JsonRejection>,
) -> Response {
    let watched = match body {
        Ok(Json(body)) => body.watched,
        Err(e) => return api_error(e.status(), &e.body_text()),
    };
    match catalog.set_watched(id, target, watched).await {
        Ok(()) => Json(catalog.watched_marks(id).await).into_response(),
        Err(e) => api_watched_failed(e, id),
    }
}

/// A title's marks after a change; a show's also carry its watched marks.
async fn api_title_marks(catalog: &Catalog, kind: MediaKind, id: u64) -> Response {
    let watched = match kind {
        MediaKind::Movie => None,
        MediaKind::Tv => Some(catalog.watched_marks(id).await),
    };
    Json(TitleMarksUpdate {
        marks: catalog.title_marks(kind, id).await,
        watched,
    })
    .into_response()
}

fn api_watched_failed(error: sqlx::Error, id: u64) -> Response {
    tracing::warn!(error = %error, id, "api watched save failed");
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "watched marks couldn't be updated, so nothing was saved",
    )
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

/// Apple TV playback seen through Home Assistant: connection, history
/// reads, shows waiting on a match, and recent detections.
#[utoipa::path(
    get,
    path = "/api/playback",
    responses((status = OK, body = PlaybackDetail))
)]
async fn api_playback(State(catalog): State<Catalog>) -> Json<PlaybackDetail> {
    Json(catalog.playback().await)
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
    (
        status,
        Json(ApiError {
            error: message.to_owned(),
        }),
    )
        .into_response()
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
