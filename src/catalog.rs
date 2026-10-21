//! Application service: turns provider responses into `model` types.

use std::cmp::{Ordering, Reverse};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::cache::ProviderCache;
use crate::config::Config;
use crate::model::{
    Availability, Department, Episode, EpisodeDetail, FilmographyEntry, FilmographyGroup,
    FilmographySort, KeyCrew, KeyRole, KnownFor, ListServices, MediaKind, Metadata, MetadataState,
    MovieWatched, OfferGroup, OfferKind, PersonCredit, PersonDetail, PersonHit, Rating, Ratings,
    RecentView, SearchHit, SeasonDetail, SeasonSummary, Service, ServiceGroup, ServiceOption,
    ServicesDetail, ServicesState, SortRatings, TitleCredits, TitleDetail, TitleHit, TitleMarks,
    UnavailableReason, WatchProvider, WatchTarget, Watched, WatchedMarks, WatchlistDetail,
    WatchlistSort, WhereToWatch, WhereToWatchState,
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
            ratings: RatingsService::new(omdb, cache.clone(), config.ratings_ttl),
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

    /// Remembers that a title or person page was opened. Failures are logged,
    /// never shown: the page itself already loaded.
    pub async fn record_view(&self, view: RecentView) {
        if let Err(e) = self.recent.record(&view, Timestamp::now()).await {
            tracing::warn!(error = %e, path = view.path(), "recording recent view failed");
        }
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

    async fn read_watched(&self, tv_id: u64) -> Option<Watched> {
        self.watched
            .get(tv_id)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, tv_id, "reading watched state failed"))
            .ok()
    }

    /// Marks a whole TV show, a season or an episode as watched or not.
    pub async fn set_watched(
        &self,
        tv_id: u64,
        target: WatchTarget,
        watched: bool,
    ) -> Result<(), sqlx::Error> {
        self.watched
            .set(tv_id, target, watched, Timestamp::now())
            .await
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
        self.watched
            .mark_episode(tv_id, season, episode, "apple_tv", Timestamp::now())
            .await
    }

    pub(crate) async fn mark_movie_from_apple_tv(&self, movie_id: u64) -> Result<(), sqlx::Error> {
        self.watched
            .mark_movie(movie_id, "apple_tv", Timestamp::now())
            .await
    }

    /// Whether a movie is marked watched; not if that can't be read.
    pub async fn movie_watched(&self, movie_id: u64) -> MovieWatched {
        self.watched.movie(movie_id).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, movie_id, "reading watched state failed");
            MovieWatched::default()
        })
    }

    pub async fn set_movie_watched(&self, movie_id: u64, watched: bool) -> Result<(), sqlx::Error> {
        self.watched
            .set_movie(movie_id, watched, Timestamp::now())
            .await
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
        let sort_ratings = match sort {
            WatchlistSort::Added => None,
            WatchlistSort::Rating => Some(self.sort_by_rating(&mut [&mut titles]).await),
        };
        Ok(WatchlistDetail {
            titles,
            sort,
            sort_ratings,
            services: None,
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
        Ok(page
            .results
            .into_iter()
            .filter_map(|result| match result {
                MultiResult::Movie(m) => Some(SearchHit::Movie(TitleHit {
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
        let mine = self.service_ids().await;
        d.where_to_watch.mark_services(mine.as_ref());
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
        Ok(TitleCredits {
            kind,
            id,
            title,
            year,
            cast: people.cast,
            crew: departments(people.crew),
            metadata,
        })
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
        let mut filmography: Vec<FilmographyGroup> = groups
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
        let sort_ratings = match sort {
            FilmographySort::Date => None,
            FilmographySort::Rating => {
                let mut lists: Vec<_> = filmography.iter_mut().map(|g| &mut g.entries).collect();
                Some(self.sort_by_rating(&mut lists).await)
            }
        };

        Ok(PersonDetail {
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
            sort_ratings,
            metadata,
        })
    }

    /// Reorders each list (e.g. a filmography department) best rated first,
    /// by Rotten Tomatoes or else IMDb; unscored entries follow in their
    /// existing order.
    async fn sort_by_rating(&self, lists: &mut [&mut Vec<FilmographyEntry>]) -> SortRatings {
        let entries = || lists.iter().flat_map(|l| l.iter());
        if !self.ratings.is_configured() {
            return SortRatings {
                unrated: entries().count(),
                stale_since: None,
                unavailable_reason: Some(UnavailableReason::NotConfigured),
            };
        }
        let ratings = self
            .title_ratings(entries().map(|e| (e.kind, e.id)).collect())
            .await;

        let mut summary = SortRatings::default();
        for r in ratings.values() {
            if let (true, Some(at)) = (r.is_stale(), r.fetched_at) {
                summary.stale_since = Some(summary.stale_since.map_or(at, |since| since.min(at)));
            }
            if r.unavailable_reason == Some(UnavailableReason::ProviderError) {
                summary.unavailable_reason = Some(UnavailableReason::ProviderError);
            }
        }
        for list in lists {
            for e in list.iter_mut() {
                e.rating = ratings.get(&(e.kind, e.id)).and_then(Rating::for_sorting);
                summary.unrated += usize::from(e.rating.is_none());
            }
            list.sort_by(|a, b| match (&a.rating, &b.rating) {
                (Some(a), Some(b)) => b.percent().total_cmp(&a.percent()),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            });
        }
        summary
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

    async fn movie_raw(&self, id: u64) -> Result<(tmdb::Movie, Metadata), CatalogError> {
        self.cached_if(format!("movie/{id}"), self.tmdb.movie(id), |m| {
            m.watch_providers.is_some()
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
    match into {
        Some(existing) if !existing.split(separator.as_str()).any(|r| r == other) => {
            existing.push_str(separator.as_str());
            existing.push_str(&other);
        }
        Some(_) => {}
        None => *into = Some(other),
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
    fn roles_merge_without_duplicates() {
        let mut role = Some("Director".to_owned());
        join_role(&mut role, Some("Writer".to_owned()), Separator::Jobs);
        join_role(&mut role, Some("Director".to_owned()), Separator::Jobs);
        join_role(&mut role, None, Separator::Jobs);
        assert_eq!(role.as_deref(), Some("Director, Writer"));
    }

    #[test]
    fn known_departments_lead_and_the_rest_are_alphabetical() {
        let mut names = vec!["Sound", "Writing", "Art", "Directing", "Creators"];
        names.sort_by_key(|n| department_rank(n));
        assert_eq!(names, ["Creators", "Directing", "Writing", "Art", "Sound"]);
    }
}
