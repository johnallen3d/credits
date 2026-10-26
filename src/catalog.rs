//! Application service: turns provider responses into `model` types.

use std::cmp::{Ordering, Reverse};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::ProviderCache;
use crate::config::Config;
use crate::imdb::{ImdbError, ImdbRatings};
use crate::model::{
    Availability, Department, Episode, EpisodeDetail, FilmographyEntry, FilmographyGroup,
    FilmographySort, KeyCrew, KeyRole, KnownFor, ListServices, MediaKind, Metadata, MetadataState,
    MovieWatched, OfferGroup, OfferKind, PersonCredit, PersonDetail, PersonHit, Rating, Ratings,
    RecentKind, RecentView, ReleaseDate, ReleaseEntry, ReleaseKind, ReleaseList, ReleaseStatus,
    Releases, ReleasesDetail, SearchHit, SeasonDetail, SeasonSummary, Service, ServiceGroup,
    ServiceOption, ServicesDetail, ServicesState, ShowProgress, SortRatings, TitleCredits,
    TitleDetail, TitleHit, TitleMarks, UnavailableReason, WatchProvider, WatchTarget, Watched,
    WatchedList, WatchedMarks, WatchedSort, WatchedSource, WatchedTitle, WatchlistDetail,
    WatchlistSort, WhereToWatch, WhereToWatchState, hide_cast_episode_counts, local_today,
    showtimes_url,
};
use crate::omdb::OmdbClient;
use crate::playback::{
    self, App, Candidate, Kind, LiveStatus, PendingShow, Playback, PlaybackDetail,
};
use crate::ratings::RatingsService;
use crate::recent::RecentlyViewed;
use crate::services::MyServices;
use crate::tmdb::{self, MultiResult, TmdbClient};
use crate::watched::WatchedShows;
use crate::watchlist::Watchlist;

const POSTER_SIZE: &str = "w342";
const BACKDROP_SIZE: &str = "w1280";
const PROFILE_SIZE: &str = "w185";
const STILL_SIZE: &str = "w300";
/// Small posters beside title references in lists, e.g. a filmography.
const THUMB_SIZE: &str = "w92";
/// Streaming service logos, shown at about 40px square.
const LOGO_SIZE: &str = "w92";
const PROVIDER: &str = "tmdb";
/// Cast members shown on a title page; the rest are on its credits page.
const TOP_CAST: usize = 12;
/// Most-frequent directors named on a TV show page.
const TV_KEY_DIRECTORS: usize = 3;
/// A TV cast member's characters listed before the rest are elided.
const MAX_CHARACTERS: usize = 3;
/// How long after its theatrical date a movie counts as in theaters, unless
/// TMDB still lists it as now playing.
const IN_THEATERS_DAYS: i64 = 45;
/// Pages of TMDB's now playing list read at most, 20 movies each.
const MOVIE_LIST_PAGES: u32 = 40;
/// Movies on each page of Now playing and Coming soon.
pub const RELEASES_PAGE_SIZE: usize = 20;
/// How far from today a row's release date (or year) can be for the row to
/// be checked for being in theaters; others aren't looked up.
const IN_THEATERS_CANDIDATE_DAYS: i64 = 365;
/// Titles in a person page's "Known for" row.
const KNOWN_FOR_TITLES: usize = 8;
/// Services listed as popular in regions without a curated list: the most
/// popular in TMDB's order.
const POPULAR_SERVICES: usize = 10;
/// The major US subscription services, by TMDB ID: Netflix, Amazon Prime
/// Video, Disney Plus, HBO Max, Hulu, Apple TV, Paramount Plus Essential and
/// Premium, Peacock Premium and Premium Plus, YouTube TV and ESPN Plus.
const POPULAR_US: [u64; 12] = [8, 9, 337, 1899, 15, 350, 2616, 2303, 386, 387, 2528, 1768];
/// Name endings of channels sold through another service, including a
/// misspelling TMDB uses.
const ADD_ON_SUFFIXES: [&str; 4] = [
    " amazon channel",
    " amzon channel",
    " roku premium channel",
    " apple tv channel",
];
/// Titles looked up at once when sorting by rating or checking which
/// services offer them.
const CONCURRENT_TITLE_LOOKUPS: usize = 8;
/// TMDB's Talk and News genres: appearing on these says little about what
/// someone is known for.
const TALK_AND_NEWS_GENRES: &[u32] = &[10767, 10763];
/// Characters that mean a person appeared as themselves.
const SELF_ROLES: &[&str] = &["Self", "Himself", "Herself", "Themselves"];
const WRITER_JOBS: &[&str] = &["Screenplay", "Writer", "Story", "Teleplay"];
/// Credits pages list these departments first, in this order; the rest follow alphabetically.
const DEPARTMENT_ORDER: &[&str] = &["Creators", "Directing", "Writing", "Production"];

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("not found")]
    NotFound,
    #[error("TMDB rejected the API token")]
    Unauthorized,
    #[error("{0}")]
    Provider(String),
}

/// Why the user's services couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub enum ServicesError {
    /// The current choice couldn't be read, so it can't be safely replaced.
    #[error("reading your services failed: {0}")]
    Read(sqlx::Error),
    #[error("saving your services failed: {0}")]
    Write(sqlx::Error),
}

/// Why a title couldn't be added to a saved list.
#[derive(Debug, thiserror::Error)]
pub enum WatchlistError {
    #[error(transparent)]
    Title(#[from] CatalogError),
    #[error("saving to the title list failed: {0}")]
    Database(#[from] sqlx::Error),
}

/// Why a page couldn't be recorded as viewed.
#[derive(Debug, thiserror::Error)]
pub enum RecentError {
    #[error(transparent)]
    Page(#[from] CatalogError),
    #[error("recording the view failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Catalog {
    tmdb: TmdbClient,
    image_base: String,
    ratings: RatingsService,
    cache: ProviderCache,
    recent: RecentlyViewed,
    watched: WatchedShows,
    watchlist: Watchlist,
    favorites: Watchlist,
    services: MyServices,
    playback: Playback,
    /// Home Assistant media players read for playback; empty if not configured.
    playback_players: Vec<String>,
    metadata_ttl: Duration,
    watch_region: String,
}

/// A filmography entry with its full release date, for sorting.
type DatedEntry = (Option<String>, FilmographyEntry);

/// A person's credits on one title in one department, roles merged.
type MergedCredit = (String, Option<String>, FilmographyEntry, Separator);

/// What a title's TMDB entry says about how well known it is.
struct Notability {
    votes: u64,
    talk_or_news: bool,
    poster_url: Option<String>,
}

/// People credited on a title, before trimming for the detail page.
struct People {
    cast: Vec<PersonCredit>,
    crew: Vec<(String, PersonCredit)>,
    key_crew: Vec<KeyCrew>,
}

impl Catalog {
    /// Opens the provider cache database, so this fails if it can't be created or migrated.
    pub async fn new(config: &Config) -> Result<Self, sqlx::Error> {
        let cache = ProviderCache::open(&config.database_path).await?;
        let omdb = config
            .omdb_api_key
            .as_deref()
            .map(|key| OmdbClient::new(&config.omdb_api_base, key));
        Ok(Self {
            tmdb: TmdbClient::new(&config.tmdb_api_base, &config.tmdb_token),
            image_base: config.tmdb_image_base.trim_end_matches('/').to_owned(),
            ratings: RatingsService::new(
                omdb,
                ImdbRatings::new(&config.imdb_ratings_url, cache.clone()),
                cache.clone(),
                config.ratings_ttl,
            ),
            recent: RecentlyViewed::new(cache.pool()),
            watched: WatchedShows::new(cache.pool()),
            watchlist: Watchlist::new(cache.pool()),
            favorites: Watchlist::favorites(cache.pool()),
            services: MyServices::new(cache.pool()),
            playback: Playback::new(cache.pool(), config.home_assistant.is_some()),
            playback_players: config
                .home_assistant
                .as_ref()
                .map(|ha| ha.players.clone())
                .unwrap_or_default(),
            cache,
            metadata_ttl: config.metadata_ttl,
            watch_region: config.watch_region.clone(),
        })
    }

    /// Keeps IMDb's ratings dataset at most a day old, forever; for the
    /// server to run in the background.
    pub async fn keep_imdb_ratings_fresh(self) {
        self.ratings.imdb().clone().keep_fresh().await
    }

    /// Downloads IMDb's ratings dataset if it's a day old or missing; true
    /// if it did.
    pub async fn refresh_imdb_ratings(&self) -> Result<bool, ImdbError> {
        self.ratings.imdb().refresh(Timestamp::now()).await
    }

    /// Remembers that a title or person page was opened. Failures are logged,
    /// never shown: the page itself already loaded.
    pub async fn record_view(&self, view: RecentView) {
        if let Err(e) = self.recent.record(&view, Timestamp::now()).await {
            tracing::warn!(error = %e, path = view.path(), "recording recent view failed");
        }
    }

    /// Records a page that a client such as the terminal client opened from
    /// the JSON API, looking up its name and artwork; answers with the list.
    pub async fn view(&self, kind: RecentKind, id: u64) -> Result<Vec<RecentView>, RecentError> {
        let view = match kind.title() {
            Some(kind) => RecentView::of_title(&self.title_detail(kind, id).await?),
            None => RecentView::of_person(&self.person(id, FilmographySort::Date).await?),
        };
        self.recent.record(&view, Timestamp::now()).await?;
        Ok(self.recent.list().await?)
    }

    /// Recently opened title and person pages, most recent first.
    pub async fn read_recently_viewed(&self) -> Result<Vec<RecentView>, sqlx::Error> {
        self.recent.list().await
    }

    /// Recently opened title and person pages, most recent first; empty if
    /// they can't be read.
    pub async fn recently_viewed(&self) -> Vec<RecentView> {
        self.recent.list().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "reading recent views failed");
            Vec::new()
        })
    }

    /// Which parts of a TV show are marked watched; nothing if that can't be
    /// read, so spoilers stay hidden.
    pub async fn watched(&self, tv_id: u64) -> Watched {
        self.read_watched(tv_id).await.unwrap_or_default()
    }

    /// A TV show's watched marks for the JSON API, saying whether they could
    /// be read.
    pub async fn watched_marks(&self, tv_id: u64) -> WatchedMarks {
        WatchedMarks::of(self.read_watched(tv_id).await)
    }

    /// Shows marked watched as a whole; `None` if they can't be read, so
    /// spoilers stay hidden.
    pub async fn watched_shows(&self) -> Option<HashSet<u64>> {
        self.watched
            .shows()
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "reading watched shows failed"))
            .ok()
    }

    async fn read_watched(&self, tv_id: u64) -> Option<Watched> {
        self.watched
            .get(tv_id)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, tv_id, "reading watched state failed"))
            .ok()
    }

    /// Marks a whole TV show, a season or an episode as watched or not.
    /// Marking rolls up to whatever that completes; a show marked watched
    /// as a whole leaves To watch.
    pub async fn set_watched(
        &self,
        tv_id: u64,
        target: WatchTarget,
        watched: bool,
    ) -> Result<(), sqlx::Error> {
        let at = Timestamp::now();
        self.watched.set(tv_id, target, watched, at).await?;
        match (watched, target) {
            (false, _) => {}
            (true, WatchTarget::Show) => self.leave_watchlist(MediaKind::Tv, tv_id).await,
            (true, _) => self.roll_up_watched(tv_id, at).await,
        }
        Ok(())
    }

    /// Takes a title just marked watched off To watch. Failures are only
    /// logged: the watched mark is already saved.
    async fn leave_watchlist(&self, kind: MediaKind, id: u64) {
        if let Err(e) = self.watchlist.remove(kind, id).await {
            tracing::warn!(error = %e, ?kind, id, "taking a watched title off To watch failed");
        }
    }

    /// Marks each season whose episodes are all marked, then the show once
    /// it has ended and every regular season is marked. Failures are only
    /// logged: the mark that prompted this is already saved.
    async fn roll_up_watched(&self, tv_id: u64, at: Timestamp) {
        let Some(watched) = self.read_watched(tv_id).await else {
            return;
        };
        if watched.show {
            return;
        }
        let t = match self.tv_raw(tv_id).await {
            Ok((t, _)) => t,
            Err(e) => {
                tracing::warn!(error = %e, tv_id, "reading seasons to roll up watched marks failed");
                return;
            }
        };
        let (seasons, show) = completed_marks(&t, &watched);
        let targets = seasons
            .into_iter()
            .map(WatchTarget::Season)
            .chain(show.then_some(WatchTarget::Show));
        for target in targets {
            if let Err(e) = self.watched.set(tv_id, target, true, at).await {
                tracing::warn!(error = %e, tv_id, ?target, "rolling up watched marks failed");
                return;
            }
        }
        if show {
            self.leave_watchlist(MediaKind::Tv, tv_id).await;
        }
    }

    /// Episodes and movies seen playing on the Apple TVs, and shows and
    /// movies waiting for the user to pick their TMDB match.
    pub async fn playback(&self) -> PlaybackDetail {
        let (recent, waiting) = tokio::join!(self.playback.recent(50), self.playback.waiting());
        let unreadable = recent.is_err() || waiting.is_err();
        if let Err(e) = recent.as_ref().and(waiting.as_ref()) {
            tracing::warn!(error = %e, "reading playback progress failed");
        }
        let mut pending: Vec<PendingShow> = Vec::new();
        for d in waiting.unwrap_or_default() {
            let label = match d.kind {
                Kind::Episode => Some(format!("S{} E{}", d.season, d.episode)),
                Kind::Movie => None,
            };
            match pending
                .iter_mut()
                .find(|p| p.app == d.app && p.kind == d.kind && p.show_key == d.show_key)
            {
                Some(p) => p.episodes.extend(label),
                None => pending.push(PendingShow {
                    app: d.app,
                    kind: d.kind,
                    show_key: d.show_key,
                    show_title: d.show_title,
                    note: d.note,
                    episodes: label.into_iter().collect(),
                    candidates: None,
                }),
            }
        }
        for p in &mut pending {
            p.episodes.reverse();
            p.candidates = playback::candidates(self, p.kind, &p.show_title).await;
        }
        PlaybackDetail {
            live: self.playback.live(),
            status: self.playback.status(),
            players: self.playback_players.clone(),
            pending,
            recent: recent.unwrap_or_default(),
            other: self.playback.other(),
            unreadable,
        }
    }

    /// Sets which TMDB show or movie (`None`: none, ignore it) an app's
    /// show or movie is, and marks what's waiting accordingly.
    pub async fn decide_playback_show(
        &self,
        app: App,
        kind: Kind,
        show_key: &str,
        tmdb_id: Option<u64>,
    ) -> Result<(), sqlx::Error> {
        playback::decide(self, app, kind, show_key, tmdb_id).await
    }

    /// How the live feed from Home Assistant is going.
    pub fn playback_live(&self) -> LiveStatus {
        self.playback.live()
    }

    pub(crate) fn playback_store(&self) -> &Playback {
        &self.playback
    }

    pub(crate) async fn mark_episode_from_apple_tv(
        &self,
        tv_id: u64,
        season: u32,
        episode: u32,
    ) -> Result<(), sqlx::Error> {
        let at = Timestamp::now();
        self.watched
            .mark_episode(tv_id, season, episode, "apple_tv", at)
            .await?;
        self.roll_up_watched(tv_id, at).await;
        Ok(())
    }

    pub(crate) async fn mark_movie_from_apple_tv(&self, movie_id: u64) -> Result<(), sqlx::Error> {
        self.watched
            .mark_movie(movie_id, "apple_tv", Timestamp::now())
            .await?;
        self.leave_watchlist(MediaKind::Movie, movie_id).await;
        Ok(())
    }

    /// Whether a movie is marked watched; not if that can't be read.
    pub async fn movie_watched(&self, movie_id: u64) -> MovieWatched {
        self.watched.movie(movie_id).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, movie_id, "reading watched state failed");
            MovieWatched::default()
        })
    }

    /// Marks a movie watched or not; marked watched, it leaves To watch.
    pub async fn set_movie_watched(&self, movie_id: u64, watched: bool) -> Result<(), sqlx::Error> {
        self.watched
            .set_movie(movie_id, watched, Timestamp::now())
            .await?;
        if watched {
            self.leave_watchlist(MediaKind::Movie, movie_id).await;
        }
        Ok(())
    }

    /// Every movie and show with a watched mark, named from their cached
    /// TMDB entries a few at a time. A failed read is an error, so the page
    /// never passes it off as empty; a failed TMDB lookup leaves that title
    /// unnamed.
    pub async fn watched_titles(&self, sort: WatchedSort) -> Result<WatchedList, sqlx::Error> {
        let records = self.watched.all().await?;
        let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
        let mut lookups = JoinSet::new();
        for (i, r) in records.into_iter().enumerate() {
            let catalog = self.clone();
            let permits = permits.clone();
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let found = match r.kind {
                    MediaKind::Movie => catalog.movie_raw(r.id).await.map(|(m, metadata)| {
                        (m.title, m.release_date, m.poster_path, metadata)
                    }),
                    MediaKind::Tv => catalog.tv_raw(r.id).await.map(|(t, metadata)| {
                        (t.name, t.first_air_date, t.poster_path, metadata)
                    }),
                };
                let found = found
                    .inspect_err(|e| {
                        tracing::warn!(error = %e, kind = ?r.kind, id = r.id, "title lookup for watched list failed");
                    })
                    .ok();
                (i, r, found)
            });
        }
        let mut found = Vec::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok(f) => found.push(f),
                Err(e) => tracing::warn!(error = %e, "watched title lookup task failed"),
            }
        }
        found.sort_by_key(|(i, ..)| *i);

        let mut list = WatchedList {
            titles: Vec::with_capacity(found.len()),
            sort,
            unavailable: 0,
            stale_since: None,
        };
        for (_, r, details) in found {
            let (title, year, poster_url) = match details {
                Some((title, date, poster_path, metadata)) => {
                    if metadata.is_stale() {
                        list.stale_since = Some(
                            list.stale_since
                                .map_or(metadata.fetched_at, |at| at.min(metadata.fetched_at)),
                        );
                    }
                    (
                        Some(title),
                        year(date.as_deref()),
                        self.image(THUMB_SIZE, poster_path),
                    )
                }
                None => {
                    list.unavailable += 1;
                    (None, None, None)
                }
            };
            let sources = [
                (r.manual, WatchedSource::Manual),
                (r.apple_tv, WatchedSource::AppleTv),
            ]
            .into_iter()
            .filter_map(|(on, source)| on.then_some(source))
            .collect();
            list.titles.push(WatchedTitle {
                kind: r.kind,
                id: r.id,
                title,
                year,
                poster_url,
                last_watched_at: Timestamp::from_second(r.last_watched_at)
                    .unwrap_or(Timestamp::UNIX_EPOCH),
                progress: (r.kind == MediaKind::Tv).then_some(ShowProgress {
                    show: r.show,
                    seasons: r.seasons,
                    episodes: r.episodes,
                }),
                sources,
            });
        }
        if sort.is_title() {
            // Stable, so equal names stay most recent first.
            list.titles.sort_by_cached_key(|t| {
                (t.title.is_none(), t.title.as_deref().map(str::to_lowercase))
            });
        }
        Ok(list)
    }

    /// Movies matching `name`, in TMDB's order.
    pub(crate) async fn movies_named(&self, name: &str) -> Result<Vec<Candidate>, CatalogError> {
        let page = self.tmdb.search_movie(name).await?;
        Ok(page
            .results
            .into_iter()
            .map(|m| Candidate {
                id: m.id,
                name: m.title,
                year: year(m.release_date.as_deref()),
                overview: non_empty(m.overview),
            })
            .collect())
    }

    /// A movie's TMDB runtime in minutes, through the same cache as its page.
    pub(crate) async fn movie_runtime(&self, movie_id: u64) -> Result<Option<u32>, CatalogError> {
        let (movie, _) = self.movie_raw(movie_id).await?;
        Ok(movie.runtime.filter(|r| *r > 0))
    }

    /// TV shows matching `name`, in TMDB's order.
    pub(crate) async fn tv_named(&self, name: &str) -> Result<Vec<Candidate>, CatalogError> {
        let page = self.tmdb.search_tv(name).await?;
        Ok(page
            .results
            .into_iter()
            .map(|t| Candidate {
                id: t.id,
                name: t.name,
                year: year(t.first_air_date.as_deref()),
                overview: non_empty(t.overview),
            })
            .collect())
    }

    /// Each episode of a season with its TMDB runtime in minutes, through
    /// the same cache as season pages.
    pub(crate) async fn episode_runtimes(
        &self,
        tv_id: u64,
        season: u32,
    ) -> Result<Vec<(u32, Option<u32>)>, CatalogError> {
        let (season, _) = self
            .cached(
                format!("tv/{tv_id}/season/{season}"),
                self.tmdb.season(tv_id, season),
            )
            .await?;
        Ok(season
            .episodes
            .into_iter()
            .map(|e| (e.episode_number, e.runtime.filter(|r| *r > 0)))
            .collect())
    }

    /// The "To watch" list in `sort` order. Unlike other saved state, a
    /// failed read is an error, so the page never passes it off as empty.
    /// Each title also says which of the user's services include it.
    pub async fn watchlist(&self, sort: WatchlistSort) -> Result<WatchlistDetail, sqlx::Error> {
        let mut w = self.saved_list(&self.watchlist, sort).await?;
        w.release_unknown = Some(self.mark_releases(&mut w.titles).await);
        let mine = self.service_ids().await;
        let state = ServicesState::of(mine.as_ref());
        let mut services = ListServices {
            state,
            unknown: 0,
            stale_since: None,
        };
        if let (true, Some(mine)) = (state.is_chosen(), mine) {
            let titles = w.titles.iter().map(|e| (e.kind, e.id)).collect();
            let mut availability = self.title_availability(titles, Arc::new(mine)).await;
            for e in &mut w.titles {
                let a = availability
                    .remove(&(e.kind, e.id))
                    .unwrap_or(Availability {
                        state: WhereToWatchState::Unavailable,
                        services: Vec::new(),
                        fetched_at: None,
                    });
                services.unknown += usize::from(a.is_unavailable());
                if let (true, Some(at)) = (a.is_stale(), a.fetched_at) {
                    services.stale_since =
                        Some(services.stale_since.map_or(at, |since| since.min(at)));
                }
                e.availability = Some(a);
            }
        }
        w.services = Some(services);
        Ok(w)
    }

    pub async fn favorites(&self, sort: WatchlistSort) -> Result<WatchlistDetail, sqlx::Error> {
        self.saved_list(&self.favorites, sort).await
    }

    async fn saved_list(
        &self,
        list: &Watchlist,
        sort: WatchlistSort,
    ) -> Result<WatchlistDetail, sqlx::Error> {
        let mut titles = list.list().await?;
        self.mark_in_theaters(&mut titles).await;
        let sort_ratings = match sort {
            WatchlistSort::Added => None,
            WatchlistSort::Rating => Some(self.sort_by_rating(&mut [&mut titles]).await),
        };
        Ok(WatchlistDetail {
            titles,
            sort,
            sort_ratings,
            services: None,
            release_unknown: None,
        })
    }

    /// Titles on the "To watch" list; none if that can't be read, so every
    /// toggle offers to add.
    pub async fn watchlist_titles(&self) -> HashSet<(MediaKind, u64)> {
        self.watchlist.titles().await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "reading the watchlist failed");
            HashSet::new()
        })
    }

    /// Saves a title to the "To watch" list, with its name, year and poster
    /// from its cached TMDB entry.
    pub async fn add_to_watchlist(&self, kind: MediaKind, id: u64) -> Result<(), WatchlistError> {
        let (entry, _) = self.title_to_save(kind, id).await?;
        Ok(self.watchlist.add(&entry, Timestamp::now()).await?)
    }

    /// Favorites snapshot the show's current seasons, not future seasons.
    pub async fn add_favorite(&self, kind: MediaKind, id: u64) -> Result<(), WatchlistError> {
        let (entry, seasons) = self.title_to_save(kind, id).await?;
        Ok(self
            .favorites
            .add_with_watched_seasons(&entry, &seasons, Timestamp::now())
            .await?)
    }

    pub async fn remove_favorite(&self, kind: MediaKind, id: u64) -> Result<(), sqlx::Error> {
        self.favorites.remove(kind, id).await
    }

    /// Keep read failures distinct from an empty list, even on browsing pages.
    pub async fn favorite_titles(&self) -> Result<HashSet<(MediaKind, u64)>, sqlx::Error> {
        self.favorites.titles().await
    }

    /// Whether a title is saved and watched, each `None` if it couldn't be read.
    pub async fn title_marks(&self, kind: MediaKind, id: u64) -> TitleMarks {
        let (to_watch, favorites) = self.saved_titles().await;
        TitleMarks {
            to_watch: to_watch.map(|t| t.contains(&(kind, id))),
            favorite: favorites.map(|t| t.contains(&(kind, id))),
            watched: self.title_watched(kind, id).await,
        }
    }

    /// Fills in each list entry's marks.
    pub async fn mark_entries(&self, entries: &mut [FilmographyEntry]) {
        let (to_watch, favorites) = self.saved_titles().await;
        for e in entries {
            let key = (e.kind, e.id);
            e.marks = Some(TitleMarks {
                to_watch: to_watch.as_ref().map(|t| t.contains(&key)),
                favorite: favorites.as_ref().map(|t| t.contains(&key)),
                watched: self.title_watched(e.kind, e.id).await,
            });
        }
    }

    /// Titles on To watch and on Favorites; each `None` if it couldn't be read.
    async fn saved_titles(
        &self,
    ) -> (
        Option<HashSet<(MediaKind, u64)>>,
        Option<HashSet<(MediaKind, u64)>>,
    ) {
        let (to_watch, favorites) = tokio::join!(self.watchlist.titles(), self.favorites.titles());
        let to_watch = to_watch
            .inspect_err(|e| tracing::warn!(error = %e, "reading the watchlist failed"))
            .ok();
        let favorites = favorites
            .inspect_err(|e| tracing::warn!(error = %e, "reading favorites failed"))
            .ok();
        (to_watch, favorites)
    }

    /// A movie's watched mark, or a show's mark as a whole.
    async fn title_watched(&self, kind: MediaKind, id: u64) -> Option<bool> {
        match kind {
            MediaKind::Movie => self
                .watched
                .movie(id)
                .await
                .inspect_err(|e| tracing::warn!(error = %e, id, "reading watched state failed"))
                .ok()
                .map(|m| m.watched),
            MediaKind::Tv => self.read_watched(id).await.map(|w| w.show),
        }
    }

    async fn title_to_save(
        &self,
        kind: MediaKind,
        id: u64,
    ) -> Result<(FilmographyEntry, Vec<u32>), CatalogError> {
        let (title, date, poster_path, seasons) = match kind {
            MediaKind::Movie => {
                let (m, _) = self.movie_raw(id).await?;
                (m.title, m.release_date, m.poster_path, Vec::new())
            }
            MediaKind::Tv => {
                let (t, _) = self.tv_raw(id).await?;
                let today = Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date();
                let seasons = current_seasons(&t.seasons, today);
                (t.name, t.first_air_date, t.poster_path, seasons)
            }
        };
        let entry = FilmographyEntry {
            kind,
            id,
            title,
            year: year(date.as_deref()),
            role: None,
            episode_count: None,
            poster_url: self.image(THUMB_SIZE, poster_path),
            rating: None,
            availability: None,
            marks: None,
            in_theaters: false,
            next_release: None,
            release_status: None,
        };
        Ok((entry, seasons))
    }

    pub async fn remove_from_watchlist(&self, kind: MediaKind, id: u64) -> Result<(), sqlx::Error> {
        self.watchlist.remove(kind, id).await
    }

    /// IDs of the user's services; `None` if they can't be read.
    async fn service_ids(&self) -> Option<HashSet<u64>> {
        match self.services.list().await {
            Ok(services) => Some(services.into_iter().map(|s| s.id).collect()),
            Err(e) => {
                tracing::warn!(error = %e, "reading your services failed");
                None
            }
        }
    }

    /// The user's services and every service offered in the region. Unlike
    /// marking titles, a failed read is an error, so the choice is never
    /// passed off as empty (and then saved over).
    pub async fn services(&self) -> Result<ServicesDetail, sqlx::Error> {
        let chosen = self.services.list().await?;
        let (listed, state, complete, fetched_at) = self.service_list().await;
        let options = service_options(&self.watch_region, listed, &chosen);
        Ok(ServicesDetail {
            region: self.watch_region.clone(),
            chosen,
            options,
            state,
            complete,
            fetched_at,
        })
    }

    /// Replaces the user's services with `ids`. Each must be in TMDB's list
    /// for the region or already chosen; others are ignored.
    pub async fn set_services(&self, ids: &HashSet<u64>) -> Result<(), ServicesError> {
        let current = self.services.list().await.map_err(ServicesError::Read)?;
        let (listed, ..) = self.service_list().await;
        let mut seen = HashSet::new();
        let services: Vec<Service> = listed
            .into_iter()
            .chain(current)
            .filter(|s| ids.contains(&s.id) && seen.insert(s.id))
            .collect();
        self.services
            .replace(&services, Timestamp::now())
            .await
            .map_err(ServicesError::Write)
    }

    /// TMDB's movie and TV service lists for the region, merged in display
    /// priority order, with how fresh and complete they are.
    async fn service_list(&self) -> (Vec<Service>, WhereToWatchState, bool, Option<Timestamp>) {
        let region = &self.watch_region;
        let lists = [
            self.cached(
                format!("watch-providers/movie/{region}"),
                self.tmdb.provider_list("movie", region),
            )
            .await,
            self.cached(
                format!("watch-providers/tv/{region}"),
                self.tmdb.provider_list("tv", region),
            )
            .await,
        ];
        let mut stale = false;
        let mut fetched_at: Option<Timestamp> = None;
        let mut loaded = 0;
        let mut by_id: HashMap<u64, (u32, tmdb::ListedProvider)> = HashMap::new();
        for list in lists {
            let (list, metadata) = match list {
                Ok(list) => list,
                Err(e) => {
                    tracing::warn!(error = %e, "TMDB provider list failed");
                    continue;
                }
            };
            loaded += 1;
            stale |= metadata.is_stale();
            fetched_at =
                Some(fetched_at.map_or(metadata.fetched_at, |at| at.min(metadata.fetched_at)));
            for p in list.results {
                let priority = p
                    .display_priorities
                    .get(region)
                    .copied()
                    .or(p.display_priority)
                    .unwrap_or(u32::MAX);
                match by_id.entry(p.provider_id) {
                    Entry::Occupied(mut o) => o.get_mut().0 = o.get().0.min(priority),
                    Entry::Vacant(v) => {
                        v.insert((priority, p));
                    }
                }
            }
        }
        let mut listed: Vec<_> = by_id.into_values().collect();
        listed.sort_by(|(a, pa), (b, pb)| {
            a.cmp(b).then_with(|| {
                pa.provider_name
                    .to_lowercase()
                    .cmp(&pb.provider_name.to_lowercase())
            })
        });
        let services = listed
            .into_iter()
            .map(|(_, p)| Service {
                id: p.provider_id,
                name: p.provider_name.trim().to_owned(),
                logo_url: self.image(LOGO_SIZE, p.logo_path),
            })
            .collect();
        let state = match (loaded, stale) {
            (0, _) => WhereToWatchState::Unavailable,
            (_, true) => WhereToWatchState::Stale,
            (_, false) => WhereToWatchState::Fresh,
        };
        (services, state, loaded == 2, fetched_at)
    }

    /// Which of `mine` include each title, from its cached TMDB entry, a few
    /// titles at a time. A TMDB failure leaves that title unavailable.
    async fn title_availability(
        &self,
        titles: HashSet<(MediaKind, u64)>,
        mine: Arc<HashSet<u64>>,
    ) -> HashMap<(MediaKind, u64), Availability> {
        let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
        let mut lookups = JoinSet::new();
        for (kind, id) in titles {
            let catalog = self.clone();
            let permits = permits.clone();
            let mine = mine.clone();
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let offers =
                    match kind {
                        MediaKind::Movie => catalog.movie_raw(id).await.map(|(m, metadata)| {
                            catalog.where_to_watch(m.watch_providers, metadata)
                        }),
                        MediaKind::Tv => catalog.tv_raw(id).await.map(|(t, metadata)| {
                            catalog.where_to_watch(t.watch_providers, metadata)
                        }),
                    };
                let mut offers = offers.unwrap_or_else(|e| {
                    tracing::warn!(error = %e, ?kind, id, "title lookup for availability failed");
                    WhereToWatch::unavailable(catalog.watch_region.clone())
                });
                offers.mark_services(Some(&mine));
                let availability = Availability {
                    state: offers.state,
                    services: offers
                        .on_my_services()
                        .into_iter()
                        .map(|p| Service {
                            id: p.id,
                            name: p.name.clone(),
                            logo_url: p.logo_url.clone(),
                        })
                        .collect(),
                    fetched_at: offers.fetched_at,
                };
                ((kind, id), availability)
            });
        }
        let mut availability = HashMap::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((title, a)) => {
                    availability.insert(title, a);
                }
                Err(e) => tracing::warn!(error = %e, "availability lookup task failed"),
            }
        }
        availability
    }

    /// Movies, TV shows and people matching `query`, in TMDB's order; other
    /// result types are dropped.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, CatalogError> {
        let page = self.tmdb.search_multi(query).await?;
        let today = local_today();
        let candidates = page
            .results
            .iter()
            .filter_map(|r| match r {
                MultiResult::Movie(m)
                    if near_today(
                        m.release_date.as_deref().and_then(|d| d.parse().ok()),
                        today,
                    ) =>
                {
                    Some(m.id)
                }
                _ => None,
            })
            .collect();
        let showing = self.movies_in_theaters(candidates).await;
        Ok(page
            .results
            .into_iter()
            .filter_map(|result| match result {
                MultiResult::Movie(m) => Some(SearchHit::Movie(TitleHit {
                    in_theaters: showing.contains(&m.id),
                    id: m.id,
                    title: m.title,
                    year: year(m.release_date.as_deref()),
                    overview: non_empty(m.overview),
                    poster_url: self.image(POSTER_SIZE, m.poster_path),
                })),
                MultiResult::Tv(t) => Some(SearchHit::Tv(TitleHit {
                    id: t.id,
                    title: t.name,
                    year: year(t.first_air_date.as_deref()),
                    overview: non_empty(t.overview),
                    poster_url: self.image(POSTER_SIZE, t.poster_path),
                    in_theaters: false,
                })),
                MultiResult::Person(p) => Some(SearchHit::Person(PersonHit {
                    id: p.id,
                    name: p.name,
                    known_for_department: non_empty(p.known_for_department),
                    known_for: p.known_for.into_iter().filter_map(known_for).collect(),
                    profile_url: self.image(PROFILE_SIZE, p.profile_path),
                })),
                MultiResult::Other => None,
            })
            .collect())
    }

    /// The title with the user's services marked among its offers.
    pub async fn title(&self, kind: MediaKind, id: u64) -> Result<TitleDetail, CatalogError> {
        let mut d = self.title_detail(kind, id).await?;
        if kind == MediaKind::Movie {
            d.in_theaters = self.in_theaters(d.id, d.releases.as_ref()).await;
            d.showtimes_url = d
                .in_theaters
                .then(|| showtimes_url(&self.watch_region, &d.title));
        }
        let mine = self.service_ids().await;
        d.where_to_watch.mark_services(mine.as_ref());
        if !self.show_watched(kind, id).await {
            hide_cast_episode_counts(&mut d.cast);
        }
        Ok(d)
    }

    async fn title_detail(&self, kind: MediaKind, id: u64) -> Result<TitleDetail, CatalogError> {
        match kind {
            MediaKind::Movie => {
                let (m, metadata) = self.movie_raw(id).await?;
                let ratings = self
                    .ratings
                    .for_title(kind, m.external_ids.imdb_id.clone())
                    .await;
                Ok(self.movie(m, ratings, metadata))
            }
            MediaKind::Tv => {
                let (t, metadata) = self.tv_raw(id).await?;
                let ratings = self
                    .ratings
                    .for_title(kind, t.external_ids.imdb_id.clone())
                    .await;
                Ok(self.tv(t, ratings, metadata))
            }
        }
    }

    /// The full cast and crew, from the same cached response as `title`.
    pub async fn credits(&self, kind: MediaKind, id: u64) -> Result<TitleCredits, CatalogError> {
        let (id, title, year, people, metadata) = match kind {
            MediaKind::Movie => {
                let (m, metadata) = self.movie_raw(id).await?;
                let people = self.movie_people(&m);
                (
                    m.id,
                    m.title,
                    year(m.release_date.as_deref()),
                    people,
                    metadata,
                )
            }
            MediaKind::Tv => {
                let (t, metadata) = self.tv_raw(id).await?;
                let people = self.tv_people(&t);
                (
                    t.id,
                    t.name,
                    year(t.first_air_date.as_deref()),
                    people,
                    metadata,
                )
            }
        };
        let mut cast = people.cast;
        if !self.show_watched(kind, id).await {
            hide_cast_episode_counts(&mut cast);
        }
        Ok(TitleCredits {
            kind,
            id,
            title,
            year,
            cast,
            crew: departments(people.crew),
            metadata,
        })
    }

    /// Whether `kind` is TV and the show is marked watched as a whole.
    async fn show_watched(&self, kind: MediaKind, id: u64) -> bool {
        kind == MediaKind::Tv && self.watched(id).await.show
    }

    pub async fn season(&self, tv_id: u64, number: u32) -> Result<SeasonDetail, CatalogError> {
        let ((show, show_metadata), (season, season_metadata)) = tokio::try_join!(
            self.tv_raw(tv_id),
            self.cached(
                format!("tv/{tv_id}/season/{number}"),
                self.tmdb.season(tv_id, number)
            ),
        )?;
        let (episode_ratings, mut episode_scores) = self
            .ratings
            .for_season(show.external_ids.imdb_id.clone(), season.season_number)
            .await;
        let seasons = self.season_list(&show.seasons);
        let position = seasons
            .iter()
            .position(|s| s.number == season.season_number);
        let neighbour = |offset: isize| {
            position
                .and_then(|p| p.checked_add_signed(offset))
                .and_then(|p| seasons.get(p))
                .cloned()
        };
        Ok(SeasonDetail {
            show_id: show.id,
            show_title: show.name.clone(),
            number: season.season_number,
            name: non_empty(season.name).unwrap_or_else(|| season_name(season.season_number)),
            overview: non_empty(season.overview),
            air_date: non_empty(season.air_date),
            poster_url: self
                .image(POSTER_SIZE, season.poster_path)
                .or_else(|| self.image(POSTER_SIZE, show.poster_path.clone())),
            episodes: season
                .episodes
                .into_iter()
                .map(|e| Episode {
                    number: e.episode_number,
                    title: non_empty(e.name)
                        .unwrap_or_else(|| format!("Episode {}", e.episode_number)),
                    air_date: non_empty(e.air_date),
                    overview: non_empty(e.overview),
                    runtime_minutes: e.runtime.filter(|r| *r > 0),
                    still_url: self.image(STILL_SIZE, e.still_path),
                    rating: episode_scores.remove(&e.episode_number),
                })
                .collect(),
            episode_ratings,
            previous: neighbour(-1),
            next: neighbour(1),
            metadata: show_metadata.combine(season_metadata),
        })
    }

    pub async fn episode(
        &self,
        tv_id: u64,
        season: u32,
        number: u32,
    ) -> Result<EpisodeDetail, CatalogError> {
        let ((show, show_metadata), (episode, episode_metadata)) = tokio::try_join!(
            self.tv_raw(tv_id),
            self.cached(
                format!("tv/{tv_id}/season/{season}/episode/{number}"),
                self.tmdb.episode(tv_id, season, number)
            ),
        )?;
        let (episode_ratings, mut episode_scores) = self
            .ratings
            .for_season(show.external_ids.imdb_id.clone(), episode.season_number)
            .await;
        let summary = show
            .seasons
            .iter()
            .find(|s| s.season_number == episode.season_number);
        let episode_count = summary.and_then(|s| s.episode_count).unwrap_or(0);
        let number = episode.episode_number;
        let credits = &episode.credits;
        let cast_of = |members: &[tmdb::CastMember]| {
            merge_credits(
                members.iter().map(|c| {
                    (
                        (),
                        self.credit(c.id, &c.name, c.character.clone(), None, &c.profile_path),
                    )
                }),
                Separator::Characters,
            )
            .into_iter()
            .map(|(_, p)| p)
            .collect::<Vec<_>>()
        };
        Ok(EpisodeDetail {
            show_id: show.id,
            show_title: show.name.clone(),
            season_number: episode.season_number,
            season_name: summary
                .and_then(|s| non_empty(s.name.clone()))
                .unwrap_or_else(|| season_name(episode.season_number)),
            number,
            title: non_empty(episode.name).unwrap_or_else(|| format!("Episode {number}")),
            air_date: non_empty(episode.air_date),
            overview: non_empty(episode.overview),
            runtime_minutes: episode.runtime.filter(|r| *r > 0),
            still_url: self.image(STILL_SIZE, episode.still_path),
            rating: episode_scores.remove(&number),
            episode_ratings,
            key_crew: self.key_crew(&credits.crew),
            cast: cast_of(&credits.cast),
            guest_stars: cast_of(&credits.guest_stars),
            crew: departments(self.crew_by_department(&credits.crew)),
            previous: number.checked_sub(1).filter(|n| *n > 0),
            next: Some(number + 1).filter(|n| *n <= episode_count),
            metadata: show_metadata.combine(episode_metadata),
        })
    }

    pub async fn person(
        &self,
        id: u64,
        sort: FilmographySort,
    ) -> Result<PersonDetail, CatalogError> {
        let mut person = self.person_unrated(id, sort).await?;
        self.rate_filmography(&mut person).await;
        Ok(person)
    }

    /// Whether ratings can be looked up; without a key rating is instant.
    pub fn ratings_configured(&self) -> bool {
        self.ratings.is_configured()
    }

    /// A person with their filmography in date order and no ratings yet,
    /// which needs no title or ratings lookups; `sort` is only recorded.
    pub async fn person_unrated(
        &self,
        id: u64,
        sort: FilmographySort,
    ) -> Result<PersonDetail, CatalogError> {
        let (p, metadata): (tmdb::Person, _) = self
            // v2: entries cached before vote counts were kept can't rank "Known for".
            .cached(format!("person/{id}/v2"), self.tmdb.person(id))
            .await?;
        let known_for = non_empty(p.known_for_department);

        let mut notability: HashMap<(MediaKind, u64), Notability> = HashMap::new();
        for c in p
            .combined_credits
            .cast
            .iter()
            .chain(&p.combined_credits.crew)
        {
            let Some(kind) = media_kind(&c.media_type) else {
                continue;
            };
            notability
                .entry((kind, c.id))
                .or_insert_with(|| Notability {
                    votes: c.vote_count.unwrap_or(0),
                    talk_or_news: c.genre_ids.iter().any(|g| TALK_AND_NEWS_GENRES.contains(g)),
                    poster_url: self.image(POSTER_SIZE, c.poster_path.clone()),
                });
        }

        let credits = p
            .combined_credits
            .cast
            .into_iter()
            .map(|c| ("Acting".to_owned(), c, Separator::Characters))
            .chain(p.combined_credits.crew.into_iter().map(|c| {
                let department =
                    non_empty(c.department.clone()).unwrap_or_else(|| "Crew".to_owned());
                (department, c, Separator::Jobs)
            }))
            .filter_map(|(department, c, separator)| {
                let kind = media_kind(&c.media_type)?;
                let date = non_empty(c.release_date.or(c.first_air_date));
                let title = non_empty(c.title.or(c.name))?;
                let role = match separator {
                    Separator::Characters => non_empty(c.character),
                    Separator::Jobs => non_empty(c.job),
                };
                let entry = FilmographyEntry {
                    kind,
                    id: c.id,
                    title,
                    year: year(date.as_deref()),
                    role,
                    episode_count: c.episode_count.filter(|n| *n > 0),
                    poster_url: self.image(THUMB_SIZE, c.poster_path),
                    rating: None,
                    availability: None,
                    marks: None,
                    in_theaters: false,
                    next_release: None,
                    release_status: None,
                };
                Some((
                    (department.clone(), kind, c.id),
                    (department, date, entry, separator),
                ))
            });
        let merged = merge_by(credits, |into, (_, _, other, separator)| {
            join_role(&mut into.2.role, other.role, separator);
            into.2.episode_count = into.2.episode_count.max(other.episode_count);
        });
        let merged: Vec<_> = merged
            .into_iter()
            .map(|(department, date, mut entry, separator)| {
                if let Separator::Characters = separator {
                    entry.role = entry
                        .role
                        .map(|r| Characters::parse(&r).render(MAX_CHARACTERS));
                }
                (department, date, entry, separator)
            })
            .collect();
        let known_for_titles = known_for_titles(&merged, known_for.as_deref(), &notability);

        let mut groups: Vec<(String, Vec<DatedEntry>)> = Vec::new();
        for (department, date, entry, _) in merged {
            match groups.iter_mut().find(|(d, _)| *d == department) {
                Some((_, entries)) => entries.push((date, entry)),
                None => groups.push((department, vec![(date, entry)])),
            }
        }
        groups.sort_by_key(|(department, entries)| {
            (
                Some(department) != known_for.as_ref(),
                Reverse(entries.len()),
            )
        });
        let filmography: Vec<FilmographyGroup> = groups
            .into_iter()
            .map(|(department, mut entries)| {
                // Undated entries are usually announced but unreleased, so they lead.
                entries.sort_by_key(|(date, _)| (date.is_some(), Reverse(date.clone())));
                FilmographyGroup {
                    department,
                    entries: entries.into_iter().map(|(_, e)| e).collect(),
                }
            })
            .collect();

        let mut person = PersonDetail {
            id: p.id,
            tmdb_url: format!("https://www.themoviedb.org/person/{}", p.id),
            name: p.name,
            known_for,
            known_for_titles,
            biography: non_empty(p.biography),
            birthday: non_empty(p.birthday),
            deathday: non_empty(p.deathday),
            place_of_birth: non_empty(p.place_of_birth),
            profile_url: self.image(PROFILE_SIZE, p.profile_path),
            filmography,
            sort,
            sort_ratings: None,
            metadata,
        };
        person.hide_acting_episode_counts(self.watched_shows().await.as_ref());
        Ok(person)
    }

    /// Gives `person`'s credits their ratings, best rated first when sorted
    /// by rating.
    pub async fn rate_filmography(&self, person: &mut PersonDetail) {
        let mut lists: Vec<_> = person
            .filmography
            .iter_mut()
            .map(|g| &mut g.entries)
            .collect();
        person.sort_ratings = Some(match person.sort {
            FilmographySort::Date => self.rate(&mut lists).await,
            FilmographySort::Rating => self.sort_by_rating(&mut lists).await,
        });
    }

    /// Reorders each list (e.g. a filmography department) best rated first,
    /// by Rotten Tomatoes or else IMDb; unscored entries follow in their
    /// existing order.
    async fn sort_by_rating(&self, lists: &mut [&mut Vec<FilmographyEntry>]) -> SortRatings {
        let summary = self.rate(lists).await;
        for list in lists {
            list.sort_by(|a, b| match (&a.rating, &b.rating) {
                (Some(a), Some(b)) => b.percent().total_cmp(&a.percent()),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            });
        }
        summary
    }

    /// Gives each entry its Rotten Tomatoes or else IMDb score, if any.
    async fn rate(&self, lists: &mut [&mut Vec<FilmographyEntry>]) -> SortRatings {
        let entries = || lists.iter().flat_map(|l| l.iter());
        let (scores, mut summary) = self
            .scores(entries().map(|e| (e.kind, e.id)).collect())
            .await;
        for list in lists {
            for e in list.iter_mut() {
                e.rating = scores.get(&(e.kind, e.id)).cloned();
                summary.unrated += usize::from(e.rating.is_none());
            }
        }
        summary
    }

    /// Each title's Rotten Tomatoes or else IMDb score, where it has one,
    /// and how complete the scores are. Titles without a score are left
    /// out; `unrated` is not counted here.
    async fn scores(
        &self,
        titles: HashSet<(MediaKind, u64)>,
    ) -> (HashMap<(MediaKind, u64), Rating>, SortRatings) {
        if !self.ratings.is_configured() {
            return (
                HashMap::new(),
                SortRatings {
                    unavailable_reason: Some(UnavailableReason::NotConfigured),
                    ..SortRatings::default()
                },
            );
        }
        let ratings = self.title_ratings(titles).await;

        let mut summary = SortRatings::default();
        for r in ratings.values() {
            if let (true, Some(at)) = (r.is_stale(), r.fetched_at) {
                summary.stale_since = Some(summary.stale_since.map_or(at, |since| since.min(at)));
            }
            if r.unavailable_reason == Some(UnavailableReason::ProviderError) {
                summary.unavailable_reason = Some(UnavailableReason::ProviderError);
            }
        }
        let scores = ratings
            .iter()
            .filter_map(|(title, r)| Some((*title, Rating::for_sorting(r)?)))
            .collect();
        (scores, summary)
    }

    /// Each title's ratings, looked up by the IMDb ID on its cached TMDB
    /// entry, a few at a time. A TMDB failure counts as a provider error.
    async fn title_ratings(
        &self,
        titles: HashSet<(MediaKind, u64)>,
    ) -> HashMap<(MediaKind, u64), Ratings> {
        let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
        let mut lookups = JoinSet::new();
        for (kind, id) in titles {
            let catalog = self.clone();
            let permits = permits.clone();
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let imdb_id = match kind {
                    MediaKind::Movie => catalog
                        .movie_raw(id)
                        .await
                        .map(|(m, _)| m.external_ids.imdb_id),
                    MediaKind::Tv => catalog.tv_raw(id).await.map(|(t, _)| t.external_ids.imdb_id),
                };
                let ratings = match imdb_id {
                    Ok(imdb_id) => catalog.ratings.for_title(kind, imdb_id).await,
                    Err(CatalogError::NotFound) => {
                        Ratings::unavailable(kind, UnavailableReason::NoImdbId, None)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, ?kind, id, "title lookup for rating sort failed");
                        Ratings::unavailable(kind, UnavailableReason::ProviderError, None)
                    }
                };
                ((kind, id), ratings)
            });
        }
        let mut ratings = HashMap::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((title, r)) => {
                    ratings.insert(title, r);
                }
                Err(e) => tracing::warn!(error = %e, "rating lookup task failed"),
            }
        }
        ratings
    }

    /// The movies TMDB lists as playing in the region's theaters, most
    /// popular first, through the provider cache (so refreshed daily).
    pub async fn now_playing(&self) -> Result<(Vec<tmdb::MovieSummary>, Metadata), CatalogError> {
        self.region_movies(ReleaseList::NowPlaying).await
    }

    /// The movies TMDB lists as playing in the region's theaters or coming
    /// to them soon, every page up to `MOVIE_LIST_PAGES`, through the provider
    /// cache. Duplicates across pages are dropped, keeping the first.
    async fn region_movies(
        &self,
        list: ReleaseList,
    ) -> Result<(Vec<tmdb::MovieSummary>, Metadata), CatalogError> {
        let region = self.watch_region.clone();
        let tmdb = self.tmdb.clone();
        let key = match list {
            ReleaseList::NowPlaying => "now_playing",
            ReleaseList::ComingSoon => "upcoming",
        };
        self.cached(format!("movie/{key}/{region}"), async move {
            let first = movie_page(&tmdb, list, &region, 1).await?;
            let mut movies = first.results;
            let mut pages = JoinSet::new();
            for page in 2..=first.total_pages.min(MOVIE_LIST_PAGES) {
                let (tmdb, region) = (tmdb.clone(), region.clone());
                pages.spawn(async move { (page, movie_page(&tmdb, list, &region, page).await) });
            }
            let mut rest = Vec::new();
            while let Some(joined) = pages.join_next().await {
                let (page, result) = joined
                    .map_err(|e| CatalogError::Provider(format!("movie list task failed: {e}")))?;
                rest.push((page, result?.results));
            }
            rest.sort_by_key(|(page, _)| *page);
            let mut seen: HashSet<u64> = movies.iter().map(|m| m.id).collect();
            for (_, page) in rest {
                movies.extend(page.into_iter().filter(|m| seen.insert(m.id)));
            }
            Ok(movies)
        })
        .await
    }

    /// One page of Now playing or Coming soon, `RELEASES_PAGE_SIZE` movies
    /// with their ratings. Only movies with a theatrical date in the region
    /// count: Now playing holds those in theaters by #898's dates rule (TMDB
    /// listing alone isn't enough), in TMDB's popularity order; Coming soon
    /// those opening after today, soonest first. A page past the end shows
    /// the last one.
    pub async fn releases(
        &self,
        list: ReleaseList,
        page: u32,
    ) -> Result<(ReleasesDetail, Metadata), CatalogError> {
        let (regional, metadata) = self.regional_movies(list).await?;
        let today = local_today();
        let mut movies: Vec<RegionalMovie> = regional
            .movies
            .into_iter()
            .filter(|m| match list {
                ReleaseList::NowPlaying => {
                    let releases = m.releases(&self.watch_region, today);
                    showing_by_dates(Some(&releases), today) == Some(true)
                }
                ReleaseList::ComingSoon => m.theatrical_date().is_some_and(|d| d > today),
            })
            .collect();
        if list == ReleaseList::ComingSoon {
            // Stable, so a day's openings keep TMDB's popularity order.
            movies.sort_by_key(RegionalMovie::theatrical_date);
        }
        let total = movies.len();
        let total_pages = total.div_ceil(RELEASES_PAGE_SIZE).max(1);
        let page = usize::try_from(page).unwrap_or(1).clamp(1, total_pages);
        let mut titles: Vec<ReleaseEntry> = movies
            .into_iter()
            .skip((page - 1) * RELEASES_PAGE_SIZE)
            .take(RELEASES_PAGE_SIZE)
            .map(|m| ReleaseEntry {
                id: m.id,
                title: m.title,
                release_date: m.theatrical,
                poster_url: self.image(POSTER_SIZE, m.poster_path),
                rating: None,
            })
            .collect();
        let (scores, mut ratings) = self
            .scores(titles.iter().map(|t| (MediaKind::Movie, t.id)).collect())
            .await;
        for t in &mut titles {
            t.rating = scores.get(&(MediaKind::Movie, t.id)).cloned();
            ratings.unrated += usize::from(t.rating.is_none());
        }
        Ok((
            ReleasesDetail {
                list,
                region: self.watch_region.clone(),
                page: page as u32,
                total_pages: total_pages as u32,
                total,
                titles,
                ratings,
                unchecked: regional.unchecked,
            },
            metadata,
        ))
    }

    /// The movies on TMDB's now playing or upcoming list that have a
    /// theatrical date in the region, with that and their digital date, in
    /// TMDB's order. Each listed movie dated within
    /// `IN_THEATERS_CANDIDATE_DAYS` of today is looked up (its details are
    /// cached anyway); the result is cached too, and refetched while any
    /// lookup failed.
    async fn regional_movies(
        &self,
        list: ReleaseList,
    ) -> Result<(RegionalMovies, Metadata), CatalogError> {
        let key = format!("releases/{}/{}", list.slug(), self.watch_region);
        let catalog = self.clone();
        let fetch = async move {
            let (listed, _) = catalog.region_movies(list).await?;
            let today = local_today();
            let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
            let mut lookups = JoinSet::new();
            for (index, m) in listed
                .into_iter()
                .filter(|m| {
                    near_today(
                        m.release_date.as_deref().and_then(|d| d.parse().ok()),
                        today,
                    )
                })
                .enumerate()
            {
                let catalog = catalog.clone();
                let permits = permits.clone();
                lookups.spawn(async move {
                    let _permit = permits.acquire_owned().await;
                    let detail = catalog.movie_raw(m.id).await;
                    (index, m, detail)
                });
            }
            let mut found = Vec::new();
            let mut unchecked = 0;
            while let Some(joined) = lookups.join_next().await {
                match joined {
                    Ok((index, m, Ok((mut detail, _)))) => {
                        let releases = releases(
                            &catalog.watch_region,
                            detail.release_dates.take().unwrap_or_default(),
                            None,
                            today,
                        );
                        let date = |kind| {
                            releases
                                .as_ref()?
                                .dates
                                .iter()
                                .find(|d| d.kind == kind)
                                .map(|d| d.date.clone())
                        };
                        let theatrical = date(ReleaseKind::Theatrical);
                        if theatrical.is_some() {
                            found.push((
                                index,
                                RegionalMovie {
                                    id: m.id,
                                    title: m.title,
                                    poster_path: m.poster_path,
                                    theatrical,
                                    digital: date(ReleaseKind::Digital),
                                },
                            ));
                        }
                    }
                    Ok((_, _, Err(CatalogError::NotFound))) => {}
                    Ok((_, m, Err(e))) => {
                        tracing::warn!(error = %e, id = m.id, "movie lookup for release list failed");
                        unchecked += 1;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "release list lookup task failed");
                        unchecked += 1;
                    }
                }
            }
            found.sort_by_key(|(index, _)| *index);
            Ok(RegionalMovies {
                movies: found.into_iter().map(|(_, m)| m).collect(),
                unchecked,
            })
        };
        self.cached_if(key, fetch, |r: &RegionalMovies| r.unchecked == 0)
            .await
    }

    /// IDs on the now playing list; none, logged, when it can't be loaded,
    /// so movies are then judged by their dates alone.
    async fn now_playing_ids(&self) -> HashSet<u64> {
        match self.now_playing().await {
            Ok((movies, _)) => movies.into_iter().map(|m| m.id).collect(),
            Err(e) => {
                tracing::warn!(error = %e, "loading now playing failed");
                HashSet::new()
            }
        }
    }

    /// Whether movie `id` is in the region's theaters: its theatrical date
    /// has arrived, within `IN_THEATERS_DAYS` or it is on TMDB's now playing
    /// list, and it is not yet out on digital.
    async fn in_theaters(&self, id: u64, releases: Option<&Releases>) -> bool {
        match showing_by_dates(releases, local_today()) {
            Some(showing) => showing,
            None => self.now_playing_ids().await.contains(&id),
        }
    }

    /// Which of the movies `ids` are in theaters, from their cached TMDB
    /// entries, a few at a time. A movie TMDB can't return isn't marked.
    async fn movies_in_theaters(&self, ids: HashSet<u64>) -> HashSet<u64> {
        let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
        let mut lookups = JoinSet::new();
        for id in ids {
            let catalog = self.clone();
            let permits = permits.clone();
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let showing = match catalog.movie_raw(id).await {
                    Ok((mut m, _)) => {
                        let releases = releases(
                            &catalog.watch_region,
                            m.release_dates.take().unwrap_or_default(),
                            m.release_date.as_deref(),
                            local_today(),
                        );
                        showing_by_dates(releases.as_ref(), local_today())
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, id, "movie lookup for in theaters failed");
                        Some(false)
                    }
                };
                (id, showing)
            });
        }
        let mut showing = HashSet::new();
        let mut undecided = Vec::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((id, Some(true))) => {
                    showing.insert(id);
                }
                Ok((_, Some(false))) => {}
                Ok((id, None)) => undecided.push(id),
                Err(e) => tracing::warn!(error = %e, "in theaters lookup task failed"),
            }
        }
        if !undecided.is_empty() {
            let playing = self.now_playing_ids().await;
            showing.extend(undecided.into_iter().filter(|id| playing.contains(id)));
        }
        showing
    }

    /// Marks the movies among `entries` that are in theaters, looking up
    /// only those dated within a year or so of today.
    async fn mark_in_theaters(&self, entries: &mut [FilmographyEntry]) {
        let showing = self.movies_in_theaters(recent_movies(entries)).await;
        for e in entries.iter_mut().filter(|e| e.kind == MediaKind::Movie) {
            e.in_theaters = showing.contains(&e.id);
        }
    }

    /// Gives each of `entries` its release status and, for a movie, its next
    /// release date still to come. Movies with a recent, future or no year
    /// are looked up, and TV shows with a current, future or no year, through
    /// their cached TMDB entries; older titles are out now. Returns how many
    /// couldn't be looked up, which get no status (unless in theaters).
    /// Expects `in_theaters` to be marked already.
    async fn mark_releases(&self, entries: &mut [FilmographyEntry]) -> usize {
        let today = local_today();
        let permits = Arc::new(Semaphore::new(CONCURRENT_TITLE_LOOKUPS));
        let mut lookups = JoinSet::new();
        for e in entries.iter().filter(|e| may_be_pending(e, today)) {
            let (catalog, permits, kind, id) = (self.clone(), permits.clone(), e.kind, e.id);
            lookups.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let found = match kind {
                    MediaKind::Movie => catalog.movie_raw(id).await.map(|(mut m, _)| {
                        let releases = releases(
                            &catalog.watch_region,
                            m.release_dates.take().unwrap_or_default(),
                            m.release_date.as_deref(),
                            today,
                        );
                        let unreleased = m.status.as_deref().is_some_and(|s| s != "Released");
                        MovieRelease::of(releases.as_ref(), unreleased, today)
                    }),
                    MediaKind::Tv => catalog.tv_raw(id).await.map(|(t, _)| {
                        let first = t.first_air_date.as_deref().and_then(|d| d.parse().ok());
                        MovieRelease {
                            next: None,
                            out_on_digital: false,
                            pending: first.is_none_or(|d: jiff::civil::Date| d > today),
                        }
                    }),
                };
                if let Err(e) = &found {
                    tracing::warn!(error = %e, ?kind, id, "title lookup for release status failed");
                }
                ((kind, id), found.ok())
            });
        }
        let mut found = HashMap::new();
        while let Some(joined) = lookups.join_next().await {
            match joined {
                Ok((key, release)) => {
                    found.insert(key, release);
                }
                Err(e) => tracing::warn!(error = %e, "release status lookup task failed"),
            }
        }
        let mut unknown = 0;
        for e in entries.iter_mut() {
            let release = match found.remove(&(e.kind, e.id)) {
                // Not looked up: an older title.
                None if !may_be_pending(e, today) => Some(MovieRelease::default()),
                None | Some(None) => None,
                Some(Some(release)) => Some(release),
            };
            e.release_status = match &release {
                Some(r) if r.out_on_digital => Some(ReleaseStatus::OutNow),
                _ if e.in_theaters => Some(ReleaseStatus::ComingSoon),
                Some(r) if r.pending => Some(ReleaseStatus::ComingSoon),
                Some(_) => Some(ReleaseStatus::OutNow),
                None => None,
            };
            unknown += usize::from(e.release_status.is_none());
            e.next_release = release.and_then(|r| r.next);
        }
        unknown
    }

    async fn movie_raw(&self, id: u64) -> Result<(tmdb::Movie, Metadata), CatalogError> {
        self.cached_if(format!("movie/{id}"), self.tmdb.movie(id), |m| {
            m.watch_providers.is_some() && m.release_dates.is_some()
        })
        .await
    }

    async fn tv_raw(&self, id: u64) -> Result<(tmdb::Tv, Metadata), CatalogError> {
        self.cached_if(format!("tv/{id}"), self.tmdb.tv(id), |t| {
            t.watch_providers.is_some()
        })
        .await
    }

    /// TMDB data through the provider cache. Expired entries are refetched, and
    /// served anyway (marked stale) if TMDB is failing; "not found" and auth
    /// errors still surface.
    async fn cached<T: Serialize + DeserializeOwned>(
        &self,
        key: String,
        fetch: impl Future<Output = Result<T, CatalogError>>,
    ) -> Result<(T, Metadata), CatalogError> {
        self.cached_if(key, fetch, |_| true).await
    }

    /// `cached`, also refetching fresh entries that aren't `complete`: those
    /// saved before the request asked TMDB for more.
    async fn cached_if<T: Serialize + DeserializeOwned>(
        &self,
        key: String,
        fetch: impl Future<Output = Result<T, CatalogError>>,
        complete: impl Fn(&T) -> bool,
    ) -> Result<(T, Metadata), CatalogError> {
        let now = Timestamp::now();
        let stale = match self.cache.get::<T>(PROVIDER, &key).await {
            Ok(Some(c)) if c.is_fresh(now) && complete(&c.value) => {
                return Ok((c.value, Metadata::fresh(c.fetched_at)));
            }
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, %key, "TMDB cache read failed");
                None
            }
        };
        match fetch.await {
            Ok(value) => {
                if let Err(e) = self
                    .cache
                    .put(PROVIDER, &key, &value, now, self.metadata_ttl)
                    .await
                {
                    tracing::warn!(error = %e, %key, "TMDB cache write failed");
                }
                Ok((value, Metadata::fresh(now)))
            }
            Err(CatalogError::Provider(e)) if stale.is_some() => {
                tracing::warn!(error = %e, %key, "TMDB failed; serving expired cache entry");
                let c = stale.expect("checked by guard");
                Ok((c.value, Metadata::stale(c.fetched_at)))
            }
            Err(e) => Err(e),
        }
    }

    fn movie(&self, mut m: tmdb::Movie, ratings: Ratings, metadata: Metadata) -> TitleDetail {
        let people = Summary::from(self.movie_people(&m));
        let where_to_watch = self.where_to_watch(m.watch_providers.take(), metadata);
        let releases = releases(
            &self.watch_region,
            m.release_dates.take().unwrap_or_default(),
            m.release_date.as_deref(),
            local_today(),
        );
        TitleDetail {
            kind: MediaKind::Movie,
            id: m.id,
            original_title: non_empty(m.original_title).filter(|o| *o != m.title),
            title: m.title,
            tagline: non_empty(m.tagline),
            overview: non_empty(m.overview),
            year: year(m.release_date.as_deref()),
            status: non_empty(m.status),
            genres: m.genres.into_iter().map(|g| g.name).collect(),
            runtime_minutes: m.runtime.filter(|r| *r > 0),
            releases,
            in_theaters: false,
            showtimes_url: None,
            seasons: None,
            episodes: None,
            poster_url: self.image(POSTER_SIZE, m.poster_path),
            backdrop_url: self.image(BACKDROP_SIZE, m.backdrop_path),
            tmdb_url: format!("https://www.themoviedb.org/movie/{}", m.id),
            ratings,
            where_to_watch,
            key_crew: people.key_crew,
            cast: people.cast,
            cast_total: people.cast_total,
            crew_total: people.crew_total,
            season_list: Vec::new(),
            metadata,
        }
    }

    fn tv(&self, mut t: tmdb::Tv, ratings: Ratings, metadata: Metadata) -> TitleDetail {
        let people = Summary::from(self.tv_people(&t));
        let where_to_watch = self.where_to_watch(t.watch_providers.take(), metadata);
        TitleDetail {
            kind: MediaKind::Tv,
            id: t.id,
            original_title: non_empty(t.original_name).filter(|o| *o != t.name),
            title: t.name,
            tagline: non_empty(t.tagline),
            overview: non_empty(t.overview),
            year: year(t.first_air_date.as_deref()),
            status: non_empty(t.status),
            genres: t.genres.into_iter().map(|g| g.name).collect(),
            runtime_minutes: None,
            releases: None,
            in_theaters: false,
            showtimes_url: None,
            seasons: t.number_of_seasons,
            episodes: t.number_of_episodes,
            poster_url: self.image(POSTER_SIZE, t.poster_path),
            backdrop_url: self.image(BACKDROP_SIZE, t.backdrop_path),
            tmdb_url: format!("https://www.themoviedb.org/tv/{}", t.id),
            ratings,
            where_to_watch,
            key_crew: people.key_crew,
            cast: people.cast,
            cast_total: people.cast_total,
            crew_total: people.crew_total,
            season_list: self.season_list(&t.seasons),
            metadata,
        }
    }

    fn movie_people(&self, m: &tmdb::Movie) -> People {
        let cast = merge_credits(
            m.credits.cast.iter().map(|c| {
                (
                    (),
                    self.credit(c.id, &c.name, c.character.clone(), None, &c.profile_path),
                )
            }),
            Separator::Characters,
        );
        People {
            cast: cast.into_iter().map(|(_, p)| p).collect(),
            crew: self.crew_by_department(&m.credits.crew),
            key_crew: self.key_crew(&m.credits.crew),
        }
    }

    /// Crew of a movie or episode, one entry per person and department.
    fn crew_by_department(&self, crew: &[tmdb::CrewMember]) -> Vec<(String, PersonCredit)> {
        merge_credits(
            crew.iter().map(|c| {
                let department =
                    non_empty(c.department.clone()).unwrap_or_else(|| "Crew".to_owned());
                (
                    department,
                    self.credit(c.id, &c.name, c.job.clone(), None, &c.profile_path),
                )
            }),
            Separator::Jobs,
        )
    }

    /// Directors and writers of a movie or episode.
    fn key_crew(&self, crew: &[tmdb::CrewMember]) -> Vec<KeyCrew> {
        let with_jobs = |jobs: &[&str]| {
            merge_credits(
                crew.iter()
                    .filter(|c| c.job.as_deref().is_some_and(|j| jobs.contains(&j)))
                    .map(|c| {
                        (
                            (),
                            self.credit(c.id, &c.name, c.job.clone(), None, &c.profile_path),
                        )
                    }),
                Separator::Jobs,
            )
            .into_iter()
            .map(|(_, p)| p)
            .collect::<Vec<_>>()
        };
        [
            (KeyRole::Director, with_jobs(&["Director"])),
            (KeyRole::Writer, with_jobs(WRITER_JOBS)),
        ]
        .into_iter()
        .filter(|(_, people)| !people.is_empty())
        .map(|(role, people)| KeyCrew { role, people })
        .collect()
    }

    fn tv_people(&self, t: &tmdb::Tv) -> People {
        let credits = &t.aggregate_credits;
        let cast = credits
            .cast
            .iter()
            .map(|c| {
                let characters: Vec<&str> = c
                    .roles
                    .iter()
                    .filter_map(|r| r.character.as_deref())
                    .filter(|ch| !ch.trim().is_empty())
                    .collect();
                let mut role = characters
                    .iter()
                    .take(MAX_CHARACTERS)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(Separator::Characters.as_str());
                if characters.len() > MAX_CHARACTERS {
                    role.push_str(" …");
                }
                self.credit(
                    c.id,
                    &c.name,
                    Some(role),
                    c.total_episode_count,
                    &c.profile_path,
                )
            })
            .collect();

        let creators: Vec<PersonCredit> = t
            .created_by
            .iter()
            .map(|c| {
                self.credit(
                    c.id,
                    &c.name,
                    Some("Creator".to_owned()),
                    None,
                    &c.profile_path,
                )
            })
            .collect();
        let mut crew: Vec<(String, PersonCredit)> = creators
            .iter()
            .map(|p| ("Creators".to_owned(), p.clone()))
            .collect();
        crew.extend(merge_credits(
            credits.crew.iter().map(|c| {
                let department =
                    non_empty(c.department.clone()).unwrap_or_else(|| "Crew".to_owned());
                let jobs = c
                    .jobs
                    .iter()
                    .filter_map(|j| j.job.as_deref())
                    .filter(|j| !j.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(Separator::Jobs.as_str());
                (
                    department,
                    self.credit(
                        c.id,
                        &c.name,
                        Some(jobs),
                        c.total_episode_count,
                        &c.profile_path,
                    ),
                )
            }),
            Separator::Jobs,
        ));

        let mut directors: Vec<PersonCredit> = credits
            .crew
            .iter()
            .filter_map(|c| {
                let episodes = c
                    .jobs
                    .iter()
                    .find(|j| j.job.as_deref() == Some("Director"))?
                    .episode_count;
                Some(self.credit(
                    c.id,
                    &c.name,
                    Some("Director".to_owned()),
                    episodes,
                    &c.profile_path,
                ))
            })
            .collect();
        directors.sort_by_key(|p| Reverse(p.episode_count));
        directors.truncate(TV_KEY_DIRECTORS);

        let key_crew = [(KeyRole::Creator, creators), (KeyRole::Director, directors)]
            .into_iter()
            .filter(|(_, people)| !people.is_empty())
            .map(|(role, people)| KeyCrew { role, people })
            .collect();

        People {
            cast,
            crew,
            key_crew,
        }
    }

    fn credit(
        &self,
        person_id: u64,
        name: &str,
        role: Option<String>,
        episode_count: Option<u32>,
        profile_path: &Option<String>,
    ) -> PersonCredit {
        PersonCredit {
            person_id,
            name: name.to_owned(),
            role: non_empty(role),
            episode_count: episode_count.filter(|n| *n > 0),
            profile_url: self.image(PROFILE_SIZE, profile_path.clone()),
        }
    }

    fn season_list(&self, seasons: &[tmdb::SeasonSummary]) -> Vec<SeasonSummary> {
        let mut list: Vec<SeasonSummary> = seasons
            .iter()
            .map(|s| SeasonSummary {
                number: s.season_number,
                name: non_empty(s.name.clone()).unwrap_or_else(|| season_name(s.season_number)),
                year: year(s.air_date.as_deref()),
                episode_count: s.episode_count,
                poster_url: self.image(POSTER_SIZE, s.poster_path.clone()),
            })
            .collect();
        // Specials (season 0) go after the regular seasons.
        list.sort_by_key(|s| (s.number == 0, s.number));
        list
    }

    /// The configured region's offers, each kind's services in TMDB's order.
    fn where_to_watch(
        &self,
        providers: Option<tmdb::WatchProviders>,
        metadata: Metadata,
    ) -> WhereToWatch {
        let region = self.watch_region.clone();
        let Some(mut providers) = providers else {
            return WhereToWatch::unavailable(region);
        };
        let offers = providers.results.remove(&region).unwrap_or_default();
        WhereToWatch {
            state: match metadata.state {
                MetadataState::Fresh => WhereToWatchState::Fresh,
                MetadataState::Stale => WhereToWatchState::Stale,
            },
            link: offers.link.filter(|l| l.starts_with("https://")),
            offers: [
                (OfferKind::Stream, offers.flatrate),
                (OfferKind::Free, offers.free),
                (OfferKind::Ads, offers.ads),
                (OfferKind::Rent, offers.rent),
                (OfferKind::Buy, offers.buy),
            ]
            .into_iter()
            .filter_map(|(kind, mut services)| {
                services.sort_by_key(|p| p.display_priority.unwrap_or(u32::MAX));
                let mut seen = HashSet::new();
                services.retain(|p| seen.insert(p.provider_id));
                (!services.is_empty()).then(|| OfferGroup {
                    kind,
                    providers: services
                        .into_iter()
                        .map(|p| WatchProvider {
                            id: p.provider_id,
                            name: p.provider_name,
                            logo_url: self.image(LOGO_SIZE, p.logo_path),
                            mine: false,
                        })
                        .collect(),
                })
            })
            .collect(),
            fetched_at: Some(metadata.fetched_at),
            my_services: ServicesState::NotChosen,
            region,
        }
    }

    fn image(&self, size: &str, path: Option<String>) -> Option<String> {
        non_empty(path).map(|p| format!("{}/{size}{p}", self.image_base))
    }
}

/// What a title page shows of its people: key crew, top-billed cast, and totals.
struct Summary {
    key_crew: Vec<KeyCrew>,
    cast: Vec<PersonCredit>,
    cast_total: usize,
    crew_total: usize,
}

impl From<People> for Summary {
    fn from(people: People) -> Self {
        let mut crew_ids: Vec<u64> = people.crew.iter().map(|(_, p)| p.person_id).collect();
        crew_ids.sort_unstable();
        crew_ids.dedup();
        let cast_total = people.cast.len();
        let mut cast = people.cast;
        cast.truncate(TOP_CAST);
        Self {
            key_crew: people.key_crew,
            cast,
            cast_total,
            crew_total: crew_ids.len(),
        }
    }
}

#[derive(Clone, Copy)]
enum Separator {
    Characters,
    Jobs,
}

impl Separator {
    fn as_str(self) -> &'static str {
        match self {
            Separator::Characters => " / ",
            Separator::Jobs => ", ",
        }
    }
}

/// Collapses rows that share a key into the first one, in first-seen order.
fn merge_by<K: Eq + Hash, T>(
    rows: impl IntoIterator<Item = (K, T)>,
    mut combine: impl FnMut(&mut T, T),
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    let mut seen: HashMap<K, usize> = HashMap::new();
    for (key, row) in rows {
        match seen.entry(key) {
            Entry::Occupied(e) => combine(&mut out[*e.get()], row),
            Entry::Vacant(e) => {
                e.insert(out.len());
                out.push(row);
            }
        }
    }
    out
}

/// One entry per person and group, e.g. a director who also wrote the film
/// appears once under Directing and once under Writing, and an actor with two
/// characters appears once with both.
fn merge_credits<G: Clone + Eq + Hash>(
    rows: impl IntoIterator<Item = (G, PersonCredit)>,
    separator: Separator,
) -> Vec<(G, PersonCredit)> {
    merge_by(
        rows.into_iter()
            .map(|(group, p)| ((group.clone(), p.person_id), (group, p))),
        |into, (_, other)| {
            join_role(&mut into.1.role, other.role, separator);
            into.1.episode_count = into.1.episode_count.max(other.episode_count);
        },
    )
}

fn join_role(into: &mut Option<String>, other: Option<String>, separator: Separator) {
    let Some(other) = other else { return };
    if let Separator::Characters = separator {
        let mut characters = Characters::parse(into.as_deref().unwrap_or(""));
        characters.extend(Characters::parse(&other));
        *into = Some(characters.render(usize::MAX));
        return;
    }
    match into {
        Some(existing) if !existing.split(separator.as_str()).any(|r| r == other) => {
            existing.push_str(separator.as_str());
            existing.push_str(&other);
        }
        Some(_) => {}
        None => *into = Some(other),
    }
}

const VOICE: &str = "(voice)";

/// Character names in TMDB's form, "Donald / Isotope (voice)", where one
/// trailing "(voice)" covers every name. TV credits arrive as one such string
/// per combination of characters played, so they repeat names.
#[derive(Default)]
struct Characters {
    names: Vec<String>,
    voice: bool,
}

impl Characters {
    fn parse(role: &str) -> Self {
        let mut characters = Characters::default();
        for name in role.split(Separator::Characters.as_str()) {
            let name = name.trim();
            let name = match name.strip_suffix(VOICE) {
                Some(rest) => {
                    characters.voice = true;
                    rest.trim_end()
                }
                None => name,
            };
            if !name.is_empty() && !characters.names.iter().any(|n| n == name) {
                characters.names.push(name.to_owned());
            }
        }
        characters
    }

    fn extend(&mut self, other: Characters) {
        self.voice |= other.voice;
        for name in other.names {
            if !self.names.contains(&name) {
                self.names.push(name);
            }
        }
    }

    /// The first `max` names, with "…" when some are left out.
    fn render(&self, max: usize) -> String {
        let mut role = self
            .names
            .iter()
            .take(max)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(Separator::Characters.as_str());
        if self.names.len() > max {
            role.push_str(" …");
        }
        if self.voice {
            if !role.is_empty() {
                role.push(' ');
            }
            role.push_str(VOICE);
        }
        role
    }
}

fn departments(crew: Vec<(String, PersonCredit)>) -> Vec<Department> {
    let mut departments: Vec<Department> = Vec::new();
    for (name, person) in crew {
        match departments.iter_mut().find(|d| d.name == name) {
            Some(d) => d.people.push(person),
            None => departments.push(Department {
                name,
                people: vec![person],
            }),
        }
    }
    departments.sort_by(|a, b| department_rank(&a.name).cmp(&department_rank(&b.name)));
    departments
}

fn department_rank(name: &str) -> (usize, &str) {
    let position = DEPARTMENT_ORDER.iter().position(|d| *d == name);
    (position.unwrap_or(DEPARTMENT_ORDER.len()), name)
}

/// Aired seasons (including specials), or undated seasons with listed
/// episodes. Explicitly future-dated and empty/unknown seasons stay unwatched.
fn current_seasons(seasons: &[tmdb::SeasonSummary], today: jiff::civil::Date) -> Vec<u32> {
    seasons
        .iter()
        .filter(|s| s.episode_count != Some(0))
        .filter(|s| {
            let air_date = s
                .air_date
                .as_deref()
                .and_then(|date| date.parse::<jiff::civil::Date>().ok());
            match air_date {
                Some(date) => date <= today,
                None => s.episode_count.is_some_and(|count| count > 0),
            }
        })
        .map(|s| s.season_number)
        .collect()
}

/// Seasons not yet marked whose every TMDB episode is, and whether the show
/// is then complete: ended or canceled, with every regular season marked.
/// Specials never hold the show back, and a returning show is never marked
/// whole so its next season stays unwatched.
fn completed_marks(t: &tmdb::Tv, watched: &Watched) -> (Vec<u32>, bool) {
    let seasons: Vec<u32> = t
        .seasons
        .iter()
        .filter(|s| !watched.season(s.season_number))
        .filter(|s| {
            s.episode_count.is_some_and(|count| {
                count > 0 && (1..=count).all(|e| watched.episode(s.season_number, e))
            })
        })
        .map(|s| s.season_number)
        .collect();
    let ended = matches!(t.status.as_deref(), Some("Ended" | "Canceled"));
    let mut regular = t
        .seasons
        .iter()
        .map(|s| s.season_number)
        .filter(|&n| n > 0)
        .peekable();
    let show = ended
        && regular.peek().is_some()
        && regular.all(|n| watched.season(n) || seasons.contains(&n));
    (seasons, show)
}

fn season_name(number: u32) -> String {
    if number == 0 {
        "Specials".to_owned()
    } else {
        format!("Season {number}")
    }
}

fn known_for(result: MultiResult) -> Option<KnownFor> {
    match result {
        MultiResult::Movie(m) => Some(KnownFor {
            kind: MediaKind::Movie,
            id: m.id,
            title: m.title,
            year: year(m.release_date.as_deref()),
        }),
        MultiResult::Tv(t) => Some(KnownFor {
            kind: MediaKind::Tv,
            id: t.id,
            title: t.name,
            year: year(t.first_air_date.as_deref()),
        }),
        MultiResult::Person(_) | MultiResult::Other => None,
    }
}

fn media_kind(media_type: &str) -> Option<MediaKind> {
    match media_type {
        "movie" => Some(MediaKind::Movie),
        "tv" => Some(MediaKind::Tv),
        _ => None,
    }
}

/// The person's most-voted titles, those in their main department ranked
/// first, leaving out talk and news shows and appearances as themselves.
fn known_for_titles(
    credits: &[MergedCredit],
    department: Option<&str>,
    notability: &HashMap<(MediaKind, u64), Notability>,
) -> Vec<FilmographyEntry> {
    let mut ranked: Vec<(bool, u64, &FilmographyEntry, Option<String>)> = credits
        .iter()
        .filter_map(|(dept, _, entry, separator)| {
            let n = notability.get(&(entry.kind, entry.id))?;
            let as_self = matches!(separator, Separator::Characters)
                && entry.role.as_deref().is_some_and(|role| {
                    role.split(separator.as_str())
                        .any(|r| SELF_ROLES.contains(&r))
                });
            (!n.talk_or_news && !as_self).then(|| {
                let elsewhere = Some(dept.as_str()) != department;
                (elsewhere, n.votes, entry, n.poster_url.clone())
            })
        })
        .collect();
    ranked.sort_by_key(|(elsewhere, votes, _, _)| (*elsewhere, Reverse(*votes)));
    merge_by(
        ranked
            .into_iter()
            .map(|(_, _, entry, poster_url)| ((entry.kind, entry.id), (entry, poster_url))),
        |_, _| {},
    )
    .into_iter()
    .take(KNOWN_FOR_TITLES)
    .map(|(entry, poster_url)| FilmographyEntry {
        poster_url,
        ..entry.clone()
    })
    .collect()
}

/// The services page's options: the user's services, then the popular ones,
/// then the rest, each A to Z. Add-on channels are left out unless chosen,
/// since the service itself is nearly always listed alongside them.
/// `listed` is in TMDB's order for `region`.
fn service_options(region: &str, listed: Vec<Service>, chosen: &[Service]) -> Vec<ServiceOption> {
    let chosen_ids: HashSet<u64> = chosen.iter().map(|s| s.id).collect();
    // Chosen services TMDB no longer lists stay, so they can be removed.
    let (mut mine, rest): (Vec<_>, Vec<_>) =
        listed.into_iter().partition(|s| chosen_ids.contains(&s.id));
    let listed_ids: HashSet<u64> = mine.iter().map(|s| s.id).collect();
    mine.extend(
        chosen
            .iter()
            .filter(|s| !listed_ids.contains(&s.id))
            .cloned(),
    );
    let rest = rest.into_iter().filter(|s| !is_add_on(&s.name));
    let curated: Option<&[u64]> = match region {
        "US" => Some(&POPULAR_US),
        _ => None,
    };
    let mut options: Vec<ServiceOption> = mine
        .into_iter()
        .map(|service| ServiceOption {
            service,
            chosen: true,
            group: ServiceGroup::Mine,
        })
        .chain(rest.enumerate().map(|(rank, service)| {
            let popular = match curated {
                Some(ids) => ids.contains(&service.id),
                None => rank < POPULAR_SERVICES,
            };
            ServiceOption {
                service,
                chosen: false,
                group: if popular {
                    ServiceGroup::Popular
                } else {
                    ServiceGroup::More
                },
            }
        }))
        .collect();
    options.sort_by_cached_key(|o| (o.group, o.service.name.to_lowercase()));
    options
}

/// Whether `name` is a channel sold through another service, like
/// "Shudder Amazon Channel".
fn is_add_on(name: &str) -> bool {
    let name = name.to_lowercase();
    ADD_ON_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// `region`'s theatrical, digital and physical dates and certification, or
/// the movie's primary date when the region lists none of those dates.
fn releases(
    region: &str,
    all: tmdb::ReleaseDates,
    primary: Option<&str>,
    today: jiff::civil::Date,
) -> Option<Releases> {
    let listed: Vec<(u8, jiff::civil::Date, Option<String>)> = all
        .results
        .into_iter()
        .filter(|r| r.iso_3166_1.eq_ignore_ascii_case(region))
        .flat_map(|r| r.release_dates)
        .filter_map(|r| {
            let date = r.release_date.as_deref()?.get(..10)?.parse().ok()?;
            Some((r.kind?, date, non_empty(r.certification)))
        })
        .collect();
    let earliest = |kind: u8| {
        listed
            .iter()
            .filter(|(k, _, _)| *k == kind)
            .map(|(_, d, _)| *d)
            .min()
    };
    let release = |kind, date: jiff::civil::Date| ReleaseDate {
        kind,
        date: date.to_string(),
        upcoming: date > today,
    };
    let mut dates: Vec<ReleaseDate> = [
        (ReleaseKind::Theatrical, earliest(3).or_else(|| earliest(2))),
        (ReleaseKind::Digital, earliest(4)),
        (ReleaseKind::Physical, earliest(5)),
    ]
    .into_iter()
    .filter_map(|(kind, date)| Some(release(kind, date?)))
    .collect();
    if dates.is_empty()
        && let Some(date) = primary.and_then(|d| d.parse().ok())
    {
        dates.push(release(ReleaseKind::Primary, date));
    }
    // The theatrical rating, else whichever release carries one.
    let certification = [3, 2]
        .into_iter()
        .find_map(|kind| {
            listed
                .iter()
                .find(|(k, _, c)| *k == kind && c.is_some())
                .and_then(|(_, _, c)| c.clone())
        })
        .or_else(|| listed.iter().find_map(|(_, _, c)| c.clone()));
    (!dates.is_empty() || certification.is_some()).then(|| Releases {
        region: region.to_owned(),
        certification,
        dates,
    })
}

/// Whether a movie with `releases` is in theaters on `today` by its dates:
/// not without a regional theatrical date that has arrived, nor once its
/// digital date has; yes within `IN_THEATERS_DAYS` of its theatrical date,
/// and otherwise `None`, for TMDB's now playing list to decide.
fn showing_by_dates(releases: Option<&Releases>, today: jiff::civil::Date) -> Option<bool> {
    let date = |kind| {
        releases?
            .dates
            .iter()
            .find(|d| d.kind == kind)?
            .date
            .parse::<jiff::civil::Date>()
            .ok()
    };
    if date(ReleaseKind::Digital).is_some_and(|d| d <= today) {
        return Some(false);
    }
    // Not before it opens, even if TMDB lists it as now playing.
    let Some(theatrical) = date(ReleaseKind::Theatrical).filter(|d| *d <= today) else {
        return Some(false);
    };
    let days = (today - theatrical).get_days();
    ((0..=IN_THEATERS_DAYS).contains(&i64::from(days))).then_some(true)
}

/// What a To watch title's dates say about watching it at home.
#[derive(Default)]
struct MovieRelease {
    /// The soonest theatrical or digital date still to come.
    next: Option<ReleaseDate>,
    /// Its regional digital date has passed.
    out_on_digital: bool,
    /// Some release is still ahead (or, undated, not released yet).
    pending: bool,
}

impl MovieRelease {
    /// From a movie's regional `releases`; `unreleased` is TMDB's status
    /// saying so, which only counts when there are no dates at all.
    fn of(releases: Option<&Releases>, unreleased: bool, today: jiff::civil::Date) -> Self {
        let dates = releases.map_or(&[][..], |r| &r.dates[..]);
        let next = dates
            .iter()
            .filter(|d| {
                matches!(d.kind, ReleaseKind::Theatrical | ReleaseKind::Digital) && d.upcoming
            })
            .min_by(|a, b| a.date.cmp(&b.date))
            .cloned();
        let out_on_digital = dates.iter().any(|d| {
            d.kind == ReleaseKind::Digital
                && d.date
                    .parse::<jiff::civil::Date>()
                    .is_ok_and(|d| d <= today)
        });
        let pending = dates.iter().any(|d| d.upcoming) || (dates.is_empty() && unreleased);
        MovieRelease {
            next,
            out_on_digital,
            pending,
        }
    }
}

/// Whether `e` is recent enough, or undated, that a release might still be
/// ahead: a movie from last year on, or a show from this year on.
fn may_be_pending(e: &FilmographyEntry, today: jiff::civil::Date) -> bool {
    let from = match e.kind {
        MediaKind::Movie => today.year() - 1,
        MediaKind::Tv => today.year(),
    };
    e.year
        .as_deref()
        .and_then(|y| y.parse::<i16>().ok())
        .is_none_or(|y| y >= from)
}

/// The movies among `entries` dated within a year or so of today, the only
/// ones whose release dates are looked up.
fn recent_movies(entries: &[FilmographyEntry]) -> HashSet<u64> {
    let today = local_today();
    entries
        .iter()
        .filter(|e| {
            e.kind == MediaKind::Movie
                && e.year
                    .as_deref()
                    .and_then(|y| y.parse::<i16>().ok())
                    .is_some_and(|y| (today.year() - 1..=today.year() + 1).contains(&y))
        })
        .map(|e| e.id)
        .collect()
}

/// Whether `date` is within `IN_THEATERS_CANDIDATE_DAYS` of `today`, so a
/// movie released then might be in theaters.
fn near_today(date: Option<jiff::civil::Date>, today: jiff::civil::Date) -> bool {
    date.is_some_and(|d| {
        let days = i64::from((today - d).get_days());
        days.abs() <= IN_THEATERS_CANDIDATE_DAYS
    })
}

/// A movie from TMDB's now playing or upcoming list with its theatrical
/// and digital dates in the region, as cached for the release lists.
#[derive(Serialize, Deserialize)]
struct RegionalMovie {
    id: u64,
    title: String,
    poster_path: Option<String>,
    /// "2026-11-06".
    theatrical: Option<String>,
    digital: Option<String>,
}

impl RegionalMovie {
    fn theatrical_date(&self) -> Option<jiff::civil::Date> {
        self.theatrical.as_deref()?.parse().ok()
    }

    /// Its dates as `Releases`, for `showing_by_dates`.
    fn releases(&self, region: &str, today: jiff::civil::Date) -> Releases {
        let dates = [
            (ReleaseKind::Theatrical, &self.theatrical),
            (ReleaseKind::Digital, &self.digital),
        ]
        .into_iter()
        .filter_map(|(kind, date)| {
            let date = date.clone()?;
            let upcoming = date.parse::<jiff::civil::Date>().is_ok_and(|d| d > today);
            Some(ReleaseDate {
                kind,
                date,
                upcoming,
            })
        })
        .collect();
        Releases {
            region: region.to_owned(),
            certification: None,
            dates,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct RegionalMovies {
    movies: Vec<RegionalMovie>,
    /// Listed movies whose details couldn't be loaded, so whose dates are
    /// unknown.
    unchecked: usize,
}

/// One page of a region's movie list from TMDB.
async fn movie_page(
    tmdb: &TmdbClient,
    list: ReleaseList,
    region: &str,
    page: u32,
) -> Result<tmdb::Paged<tmdb::MovieSummary>, CatalogError> {
    match list {
        ReleaseList::NowPlaying => tmdb.now_playing(region, page).await,
        ReleaseList::ComingSoon => tmdb.upcoming(region, page).await,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}

fn year(date: Option<&str>) -> Option<String> {
    date.and_then(|d| d.get(..4)).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_options_group_popular_and_hide_add_ons() {
        let service = |id: u64, name: &str| Service {
            id,
            name: name.to_owned(),
            logo_url: None,
        };
        let listed = || {
            vec![
                service(2, "Apple TV Store"),
                service(204, "Shudder Amazon Channel"),
                service(8, "Netflix"),
                service(99, "Shudder"),
                service(582, "Paramount+ Amazon Channel"),
            ]
        };
        let summary = |options: Vec<ServiceOption>| {
            options
                .into_iter()
                .map(|o| (o.group, o.service.name))
                .collect::<Vec<_>>()
        };
        let named = |group, name: &str| (group, name.to_owned());
        use ServiceGroup::{Mine, More, Popular};

        // The US has a curated popular list, whatever TMDB's order.
        let chosen = [service(204, "Shudder Amazon Channel"), service(5, "Gone")];
        assert_eq!(
            summary(service_options("US", listed(), &chosen)),
            [
                named(Mine, "Gone"),
                named(Mine, "Shudder Amazon Channel"),
                named(Popular, "Netflix"),
                named(More, "Apple TV Store"),
                named(More, "Shudder"),
            ]
        );

        // Elsewhere, the most popular in TMDB's order.
        let mut many = listed();
        many.extend((10..20).map(|id| service(id, &format!("Extra {id}"))));
        let options = summary(service_options("GB", many, &[]));
        assert_eq!(options[0], named(Popular, "Apple TV Store"));
        assert_eq!(options.iter().filter(|(g, _)| *g == Popular).count(), 10);
        assert_eq!(options.iter().filter(|(g, _)| *g == More).count(), 3);
        assert!(!options.iter().any(|(_, n)| n.contains("Channel")));
    }

    #[test]
    fn releases_pick_the_regions_earliest_dates_and_fall_back_to_primary() {
        let all: tmdb::ReleaseDates = serde_json::from_value(serde_json::json!({ "results": [
            { "iso_3166_1": "US", "release_dates": [
                { "type": 1, "release_date": "2026-09-01T00:00:00.000Z", "certification": "" },
                { "type": 2, "release_date": "2026-10-09T00:00:00.000Z", "certification": "" },
                { "type": 3, "release_date": "2026-11-20T00:00:00.000Z", "certification": "" },
                { "type": 3, "release_date": "2026-10-16T00:00:00.000Z", "certification": "PG-13" },
                { "type": 4, "release_date": "2026-09-30T00:00:00.000Z", "certification": "R" },
                { "type": 5, "release_date": "bad", "certification": "" }
            ] },
            { "iso_3166_1": "GB", "release_dates": [
                { "type": 6, "release_date": "2027-03-01T00:00:00.000Z", "certification": "12A" }
            ] }
        ]}))
        .unwrap();
        let today = "2026-10-01".parse().unwrap();
        let us = releases("US", all, Some("2026-09-01"), today).unwrap();
        assert_eq!(us.region, "US");
        assert_eq!(
            us.certification.as_deref(),
            Some("PG-13"),
            "the theatrical rating"
        );
        let dates: Vec<_> = us
            .dates
            .iter()
            .map(|d| (d.kind, d.date.as_str(), d.upcoming))
            .collect();
        assert_eq!(
            dates,
            [
                (ReleaseKind::Theatrical, "2026-10-16", true),
                (ReleaseKind::Digital, "2026-09-30", false),
            ],
            "earliest wide release; unparseable dates dropped"
        );

        let limited: tmdb::ReleaseDates = serde_json::from_value(serde_json::json!({ "results": [
            { "iso_3166_1": "US", "release_dates": [
                { "type": 2, "release_date": "2026-10-09T00:00:00.000Z" }
            ] }
        ]}))
        .unwrap();
        let limited = releases("US", limited, None, today).unwrap();
        assert_eq!(limited.dates[0].kind, ReleaseKind::Theatrical);
        assert_eq!(limited.dates[0].date, "2026-10-09");

        let gb: tmdb::ReleaseDates = serde_json::from_value(serde_json::json!({ "results": [
            { "iso_3166_1": "GB", "release_dates": [
                { "type": 6, "release_date": "2027-03-01T00:00:00.000Z", "certification": "12A" }
            ] }
        ]}))
        .unwrap();
        let gb = releases("GB", gb, Some("2026-09-01"), today).unwrap();
        assert_eq!(gb.certification.as_deref(), Some("12A"));
        assert_eq!(gb.dates.len(), 1, "TV airings don't count");
        assert_eq!(gb.dates[0].kind, ReleaseKind::Primary);
        assert_eq!(gb.dates[0].date, "2026-09-01");
        assert!(!gb.dates[0].upcoming);

        assert!(releases("US", Default::default(), None, today).is_none());
        assert!(releases("US", Default::default(), Some(""), today).is_none());
    }

    #[test]
    fn movies_are_in_theaters_for_45_days_until_they_go_digital() {
        let today: jiff::civil::Date = "2026-10-23".parse().unwrap();
        let with = |dates: &[(ReleaseKind, &str)]| Releases {
            region: "US".to_owned(),
            certification: None,
            dates: dates
                .iter()
                .map(|&(kind, date)| ReleaseDate {
                    kind,
                    date: date.to_owned(),
                    upcoming: false,
                })
                .collect(),
        };
        let showing = |dates: &[(ReleaseKind, &str)]| showing_by_dates(Some(&with(dates)), today);
        use ReleaseKind::{Digital, Primary, Theatrical};
        assert_eq!(
            showing(&[(Theatrical, "2026-10-23")]),
            Some(true),
            "opening day"
        );
        assert_eq!(showing(&[(Theatrical, "2026-09-08")]), Some(true), "day 45");
        assert_eq!(
            showing(&[(Theatrical, "2026-09-07")]),
            None,
            "day 46: ask now playing"
        );
        assert_eq!(
            showing(&[(Theatrical, "2026-10-24")]),
            Some(false),
            "not open yet, whatever now playing says"
        );
        assert_eq!(
            showing(&[(Theatrical, "2026-10-01"), (Digital, "2026-10-23")]),
            Some(false),
            "digital today ends it"
        );
        assert_eq!(
            showing(&[(Theatrical, "2026-10-01"), (Digital, "2026-11-01")]),
            Some(true),
            "digital still to come"
        );
        assert_eq!(
            showing(&[(Theatrical, "2026-05-01"), (Digital, "2026-06-01")]),
            Some(false),
            "now playing can't bring back a digital release"
        );
        assert_eq!(
            showing(&[(Primary, "2026-10-20")]),
            Some(false),
            "no regional theatrical date"
        );
        assert_eq!(showing_by_dates(None, today), Some(false));

        assert!(near_today("2025-10-23".parse().ok(), today));
        assert!(near_today("2027-10-23".parse().ok(), today));
        assert!(!near_today("2025-10-22".parse().ok(), today));
        assert!(!near_today(None, today));
    }

    #[test]
    fn favorite_seasons_are_a_snapshot_of_current_seasons() {
        let seasons: Vec<tmdb::SeasonSummary> = serde_json::from_value(serde_json::json!([
            {"season_number": 0, "air_date": "2025-01-01", "episode_count": 2},
            {"season_number": 1, "air_date": "2025-02-01", "episode_count": 10},
            {"season_number": 2, "air_date": null, "episode_count": 8},
            {"season_number": 3, "air_date": "2026-01-01", "episode_count": 1},
            {"season_number": 4, "air_date": "2026-01-02", "episode_count": 10},
            {"season_number": 5, "episode_count": 0},
            {"season_number": 6},
            {"season_number": 7, "air_date": "2025-01-01", "episode_count": 0},
            {"season_number": 8, "air_date": "2025-01-01"}
        ]))
        .unwrap();
        let today = "2026-01-01".parse().unwrap();
        assert_eq!(current_seasons(&seasons, today), [0, 1, 2, 3, 8]);
        assert!(current_seasons(&[], today).is_empty());
    }

    #[test]
    fn watched_marks_roll_up_to_complete_seasons_and_ended_shows() {
        let tv = |status: &str| -> tmdb::Tv {
            serde_json::from_value(serde_json::json!({
                "id": 1, "name": "Show", "status": status,
                "seasons": [
                    {"season_number": 0, "episode_count": 3},
                    {"season_number": 1, "episode_count": 2},
                    {"season_number": 2, "episode_count": 2},
                    {"season_number": 3, "episode_count": null}
                ]
            }))
            .unwrap()
        };
        let watched = |seasons: &[u32], episodes: &[(u32, u32)]| Watched {
            seasons: seasons.to_vec(),
            episodes: episodes.to_vec(),
            ..Watched::default()
        };

        let partway = watched(&[], &[(1, 1), (2, 1), (2, 2)]);
        assert_eq!(completed_marks(&tv("Ended"), &partway), (vec![2], false));
        let marked_out_of_order = watched(&[1], &[(2, 2), (2, 1), (2, 3)]);
        assert_eq!(
            completed_marks(&tv("Ended"), &marked_out_of_order),
            (vec![2], false),
            "a season without an episode count never completes"
        );
        let all = watched(&[1, 3], &[(2, 1), (2, 2)]);
        assert_eq!(completed_marks(&tv("Ended"), &all), (vec![2], true));
        assert_eq!(completed_marks(&tv("Canceled"), &all), (vec![2], true));
        assert_eq!(
            completed_marks(&tv("Returning Series"), &all),
            (vec![2], false),
            "a returning show's next season stays unwatched"
        );
        assert_eq!(
            completed_marks(&tv("Ended"), &watched(&[1, 2, 3], &[])),
            (vec![], true),
            "specials are optional"
        );
        let mut whole = watched(&[], &[]);
        whole.show = true;
        assert_eq!(completed_marks(&tv("Ended"), &whole).0, Vec::<u32>::new());
        let no_regular_seasons: tmdb::Tv = serde_json::from_value(serde_json::json!({
            "id": 1, "name": "Show", "status": "Ended", "seasons": []
        }))
        .unwrap();
        assert_eq!(
            completed_marks(&no_regular_seasons, &watched(&[], &[])),
            (vec![], false)
        );
    }

    #[test]
    fn roles_merge_without_duplicates() {
        let mut role = Some("Director".to_owned());
        join_role(&mut role, Some("Writer".to_owned()), Separator::Jobs);
        join_role(&mut role, Some("Director".to_owned()), Separator::Jobs);
        join_role(&mut role, None, Separator::Jobs);
        assert_eq!(role.as_deref(), Some("Director, Writer"));
    }

    #[test]
    fn repeated_tv_characters_collapse() {
        let mut role = None;
        for r in [
            "Donald (voice)",
            "Donald / Doc Seismic (voice)",
            "Pete / Donald / Todd (voice)",
            "(voice)",
        ] {
            join_role(&mut role, Some(r.to_owned()), Separator::Characters);
        }
        let role = role.unwrap();
        assert_eq!(role, "Donald / Doc Seismic / Pete / Todd (voice)");
        assert_eq!(
            Characters::parse(&role).render(MAX_CHARACTERS),
            "Donald / Doc Seismic / Pete … (voice)"
        );
        assert_eq!(Characters::parse("Brian").render(MAX_CHARACTERS), "Brian");
    }

    #[test]
    fn known_departments_lead_and_the_rest_are_alphabetical() {
        let mut names = vec!["Sound", "Writing", "Art", "Directing", "Creators"];
        names.sort_by_key(|n| department_rank(n));
        assert_eq!(names, ["Creators", "Directing", "Writing", "Art", "Sound"]);
    }
}
