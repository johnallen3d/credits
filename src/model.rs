//! Presentation-independent types shared by the HTML pages and the JSON API.

use std::collections::HashSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Movie,
    Tv,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Movie => "Movie",
            MediaKind::Tv => "TV show",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            MediaKind::Movie => "movie",
            MediaKind::Tv => "tv",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "movie" => Some(MediaKind::Movie),
            "tv" => Some(MediaKind::Tv),
            _ => None,
        }
    }
}

/// One search result. Serialized with a `kind` of "movie", "tv" or "person"
/// alongside the variant's fields.
#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchHit {
    Movie(TitleHit),
    Tv(TitleHit),
    Person(PersonHit),
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TitleHit {
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    /// Movies only: playing in the region's theaters now.
    pub in_theaters: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PersonHit {
    pub id: u64,
    pub name: String,
    pub known_for_department: Option<String>,
    /// The few titles TMDB associates with the person, most notable first.
    pub known_for: Vec<KnownFor>,
    pub profile_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct KnownFor {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
}

impl SearchHit {
    /// The movie or TV show this hit is, or `None` for a person.
    pub fn title(&self) -> Option<(MediaKind, &TitleHit)> {
        match self {
            SearchHit::Movie(t) => Some((MediaKind::Movie, t)),
            SearchHit::Tv(t) => Some((MediaKind::Tv, t)),
            SearchHit::Person(_) => None,
        }
    }

    pub fn detail_path(&self) -> String {
        match self {
            SearchHit::Movie(t) => title_path(MediaKind::Movie, t.id),
            SearchHit::Tv(t) => title_path(MediaKind::Tv, t.id),
            SearchHit::Person(p) => person_path(p.id),
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            SearchHit::Movie(_) => MediaKind::Movie.slug(),
            SearchHit::Tv(_) => MediaKind::Tv.slug(),
            SearchHit::Person(_) => "person",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SearchHit::Movie(_) => MediaKind::Movie.label(),
            SearchHit::Tv(_) => MediaKind::Tv.label(),
            SearchHit::Person(_) => "Person",
        }
    }

    pub fn heading(&self) -> &str {
        match self {
            SearchHit::Movie(t) | SearchHit::Tv(t) => &t.title,
            SearchHit::Person(p) => &p.name,
        }
    }

    pub fn year(&self) -> Option<&str> {
        match self {
            SearchHit::Movie(t) | SearchHit::Tv(t) => t.year.as_deref(),
            SearchHit::Person(_) => None,
        }
    }

    pub fn image_url(&self) -> Option<&str> {
        match self {
            SearchHit::Movie(t) | SearchHit::Tv(t) => t.poster_url.as_deref(),
            SearchHit::Person(p) => p.profile_url.as_deref(),
        }
    }

    pub fn missing_image_label(&self) -> &'static str {
        match self {
            SearchHit::Person(_) => "No photo",
            _ => "No artwork",
        }
    }

    /// The overview of a title, or a person's department and best-known titles.
    pub fn summary(&self) -> Option<String> {
        match self {
            SearchHit::Movie(t) | SearchHit::Tv(t) => t.overview.clone(),
            SearchHit::Person(p) => {
                let titles = p
                    .known_for
                    .iter()
                    .map(|k| match &k.year {
                        Some(year) => format!("{} ({year})", k.title),
                        None => k.title.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                match (&p.known_for_department, titles.is_empty()) {
                    (Some(d), false) => Some(format!("{d} · known for {titles}")),
                    (Some(d), true) => Some(d.clone()),
                    (None, false) => Some(format!("Known for {titles}")),
                    (None, true) => None,
                }
            }
        }
    }
}

pub fn title_path(kind: MediaKind, id: u64) -> String {
    format!("/{}/{id}", kind.slug())
}

pub fn person_path(id: u64) -> String {
    format!("/person/{id}")
}

/// Which kind of page a recently viewed entry links to: "movie", "tv" or
/// "person".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecentKind {
    Movie,
    Tv,
    Person,
}

impl RecentKind {
    pub fn slug(self) -> &'static str {
        match self {
            RecentKind::Movie => "movie",
            RecentKind::Tv => "tv",
            RecentKind::Person => "person",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "person" => Some(RecentKind::Person),
            _ => MediaKind::from_slug(slug).map(Self::from),
        }
    }

    /// The title kind, or `None` for a person.
    pub fn title(self) -> Option<MediaKind> {
        match self {
            RecentKind::Movie => Some(MediaKind::Movie),
            RecentKind::Tv => Some(MediaKind::Tv),
            RecentKind::Person => None,
        }
    }
}

impl From<MediaKind> for RecentKind {
    fn from(kind: MediaKind) -> Self {
        match kind {
            MediaKind::Movie => RecentKind::Movie,
            MediaKind::Tv => RecentKind::Tv,
        }
    }
}

/// A title or person page as listed under "Recently viewed" on the home page.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RecentView {
    pub kind: RecentKind,
    pub id: u64,
    pub name: String,
    pub year: Option<String>,
    pub image_url: Option<String>,
}

impl RecentView {
    pub fn of_title(d: &TitleDetail) -> Self {
        Self {
            kind: d.kind.into(),
            id: d.id,
            name: d.title.clone(),
            year: d.year.clone(),
            image_url: d.poster_url.clone(),
        }
    }

    pub fn of_person(p: &PersonDetail) -> Self {
        Self {
            kind: RecentKind::Person,
            id: p.id,
            name: p.name.clone(),
            year: None,
            image_url: p.profile_url.clone(),
        }
    }

    pub fn path(&self) -> String {
        match self.kind.title() {
            Some(kind) => title_path(kind, self.id),
            None => person_path(self.id),
        }
    }

    pub fn slug(&self) -> &'static str {
        self.kind.slug()
    }

    pub fn label(&self) -> &'static str {
        match self.kind.title() {
            Some(kind) => kind.label(),
            None => "Person",
        }
    }

    pub fn missing_image_label(&self) -> &'static str {
        match self.kind {
            RecentKind::Person => "No photo",
            _ => "No artwork",
        }
    }
}

/// Recently opened title and person pages, most recent first.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RecentViews {
    pub recent: Vec<RecentView>,
}

/// Which parts of a TV show have been marked as watched. Season pages blur
/// an episode's summary and rating until it, its season or the show is watched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watched {
    /// The whole show, every season included.
    pub show: bool,
    /// Seasons marked one at a time, ascending.
    pub seasons: Vec<u32>,
    /// (season, episode) pairs marked one at a time, ascending.
    pub episodes: Vec<(u32, u32)>,
    /// Those of `episodes` marked from what played on an Apple TV.
    pub apple_tv: Vec<(u32, u32)>,
}

impl Watched {
    pub fn season(&self, number: u32) -> bool {
        self.show || self.seasons.contains(&number)
    }

    pub fn episode(&self, season: u32, number: u32) -> bool {
        self.season(season) || self.episodes.contains(&(season, number))
    }

    /// Whether the episode's own mark came from the Apple TV.
    pub fn from_apple_tv(&self, season: u32, number: u32) -> bool {
        self.apple_tv.contains(&(season, number))
    }
}

/// A TV show's watched marks as the JSON API reports them, so a client can
/// hide spoilers the way the pages do.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WatchedMarks {
    pub state: WatchedState,
    /// The whole show, every season included.
    pub show: bool,
    /// Seasons marked one at a time, ascending.
    pub seasons: Vec<u32>,
    /// Episodes marked one at a time, ascending.
    pub episodes: Vec<EpisodeMark>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchedState {
    Read,
    /// The marks couldn't be read, so nothing is reported as watched and
    /// spoilers should stay hidden.
    Unreadable,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EpisodeMark {
    pub season: u32,
    pub number: u32,
    /// Marked from what played on an Apple TV.
    pub apple_tv: bool,
}

impl WatchedMarks {
    /// `None` when the marks couldn't be read.
    pub fn of(watched: Option<Watched>) -> Self {
        let state = if watched.is_some() {
            WatchedState::Read
        } else {
            WatchedState::Unreadable
        };
        let w = watched.unwrap_or_default();
        Self {
            state,
            show: w.show,
            seasons: w.seasons.clone(),
            episodes: w
                .episodes
                .iter()
                .map(|&(season, number)| EpisodeMark {
                    season,
                    number,
                    apple_tv: w.from_apple_tv(season, number),
                })
                .collect(),
        }
    }
}

/// A movie with whether it's saved and watched.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct MovieWithMarks {
    #[serde(flatten)]
    pub title: TitleDetail,
    pub marks: TitleMarks,
}

/// A TV show with its watched marks.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TvWithWatched {
    #[serde(flatten)]
    pub title: TitleDetail,
    pub watched: WatchedMarks,
    pub marks: TitleMarks,
}

/// Whether a title is on To watch and Favorites and marked watched (a TV
/// show as a whole). Each is `null` when it couldn't be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct TitleMarks {
    pub to_watch: Option<bool>,
    pub favorite: Option<bool>,
    pub watched: Option<bool>,
}

/// A title's marks after a change. TV shows also carry the show's watched
/// marks, since favoriting one marks its aired seasons.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TitleMarksUpdate {
    pub marks: TitleMarks,
    /// TV only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched: Option<WatchedMarks>,
}

/// A season with its show's watched marks.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SeasonWithWatched {
    #[serde(flatten)]
    pub season: SeasonDetail,
    pub watched: WatchedMarks,
}

/// An episode with its show's watched marks.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EpisodeWithWatched {
    #[serde(flatten)]
    pub episode: EpisodeDetail,
    pub watched: WatchedMarks,
}

/// Whether a movie is marked watched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, ToSchema)]
pub struct MovieWatched {
    pub watched: bool,
    /// Marked from what played on an Apple TV.
    pub apple_tv: bool,
}

/// What a watched mark applies to, within one TV show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchTarget {
    Show,
    Season(u32),
    Episode { season: u32, number: u32 },
}

/// A timestamp as a local calendar day, e.g. "Oct 19, 2026".
fn local_day(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%b %-d, %Y")
        .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetadataState {
    /// Within the freshness window.
    Fresh,
    /// An expired cached copy, served because TMDB could not be reached.
    Stale,
}

/// How current the TMDB data behind a response is.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
pub struct Metadata {
    pub state: MetadataState,
    pub fetched_at: Timestamp,
}

impl Metadata {
    pub fn fresh(fetched_at: Timestamp) -> Self {
        Self {
            state: MetadataState::Fresh,
            fetched_at,
        }
    }

    pub fn stale(fetched_at: Timestamp) -> Self {
        Self {
            state: MetadataState::Stale,
            fetched_at,
        }
    }

    /// For a response built from two TMDB payloads: as current as the older one.
    pub fn combine(self, other: Self) -> Self {
        Self {
            state: if self.is_stale() || other.is_stale() {
                MetadataState::Stale
            } else {
                MetadataState::Fresh
            },
            fetched_at: self.fetched_at.min(other.fetched_at),
        }
    }

    pub fn is_stale(&self) -> bool {
        self.state == MetadataState::Stale
    }

    pub fn fetched_on(&self) -> String {
        local_day(self.fetched_at)
    }
}

/// How many episodes an actor is in gives away how long their character
/// lasts, so cast counts wait until the whole show is watched.
pub fn hide_cast_episode_counts(cast: &mut [PersonCredit]) {
    for p in cast {
        p.episode_count = None;
    }
}

/// "1 episode", "62 episodes".
pub fn episodes_label(count: u32) -> String {
    if count == 1 {
        "1 episode".to_owned()
    } else {
        format!("{count} episodes")
    }
}

/// "2008-01-20" as "Jan 20, 2008"; anything unparseable is passed through.
pub fn display_date(date: &str) -> String {
    date.parse::<jiff::civil::Date>()
        .map(|d| d.strftime("%b %-d, %Y").to_string())
        .unwrap_or_else(|_| date.to_owned())
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TitleDetail {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub original_title: Option<String>,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub year: Option<String>,
    pub status: Option<String>,
    pub genres: Vec<String>,
    /// Movies only.
    pub runtime_minutes: Option<u32>,
    /// Movies only; `None` when TMDB has no dates at all.
    pub releases: Option<Releases>,
    /// Movies only: playing in the region's theaters now (see
    /// `Catalog::in_theaters`).
    pub in_theaters: bool,
    /// TV only.
    pub seasons: Option<u32>,
    /// TV only.
    pub episodes: Option<u32>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub tmdb_url: String,
    pub ratings: Ratings,
    pub where_to_watch: WhereToWatch,
    /// Directors and writers (movies), creators and principal directors (TV).
    pub key_crew: Vec<KeyCrew>,
    /// Top-billed cast only; the full list is in `TitleCredits`.
    pub cast: Vec<PersonCredit>,
    pub cast_total: usize,
    pub crew_total: usize,
    /// TV only, regular seasons first and specials (season 0) last.
    pub season_list: Vec<SeasonSummary>,
    pub metadata: Metadata,
}

impl TitleDetail {
    pub fn path(&self) -> String {
        title_path(self.kind, self.id)
    }

    /// Where the form marking this TV show watched posts to.
    pub fn watched_path(&self) -> String {
        format!("{}/watched", self.path())
    }

    pub fn has_more_credits(&self) -> bool {
        self.cast_total > self.cast.len() || self.crew_total > 0
    }
}

/// When a movie comes out in the configured region, from TMDB.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Releases {
    /// The ISO 3166-1 region the dates and certification are for.
    pub region: String,
    /// The region's age rating, like "PG-13".
    pub certification: Option<String>,
    /// In theaters, digital, then physical, each its earliest date. When the
    /// region has none of those, just TMDB's primary release date.
    pub dates: Vec<ReleaseDate>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReleaseDate {
    pub kind: ReleaseKind,
    /// "2010-07-16".
    pub date: String,
    /// Whether the date is after today.
    pub upcoming: bool,
}

impl ReleaseDate {
    pub fn label(&self) -> &'static str {
        self.kind.label(self.upcoming)
    }

    /// "Jul 16, 2010", or "Coming Oct 16" for a date still to come.
    pub fn display(&self) -> String {
        release_day(&self.date, local_today())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    /// Wide theatrical, or limited when there is no wide release.
    Theatrical,
    Digital,
    Physical,
    /// TMDB's primary release date, used when the region has no dates.
    Primary,
}

impl ReleaseKind {
    pub fn label(self, upcoming: bool) -> &'static str {
        match self {
            ReleaseKind::Theatrical => "In theaters",
            ReleaseKind::Digital => "Digital",
            ReleaseKind::Physical => "Physical",
            ReleaseKind::Primary if upcoming => "Release",
            ReleaseKind::Primary => "Released",
        }
    }
}

/// Today's date in the local time zone.
pub fn local_today() -> jiff::civil::Date {
    Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::system())
        .date()
}

/// A release date as "Jul 16, 2010", or "Coming Oct 16" once `today` is
/// before it, with the year only when it isn't this year's.
pub fn release_day(date: &str, today: jiff::civil::Date) -> String {
    match date.parse::<jiff::civil::Date>() {
        Ok(d) if d > today && d.year() == today.year() => {
            format!("Coming {}", d.strftime("%b %-d"))
        }
        Ok(d) if d > today => format!("Coming {}", d.strftime("%b %-d, %Y")),
        _ => display_date(date),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WhereToWatchState {
    /// From TMDB data within its freshness window.
    Fresh,
    /// From an expired cached copy, served because TMDB could not be reached.
    Stale,
    /// Not known: TMDB could not be reached and nothing cached includes it.
    Unavailable,
}

/// How a title can be watched in one region, from TMDB's JustWatch data.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WhereToWatch {
    /// ISO 3166-1 country code.
    pub region: String,
    pub state: WhereToWatchState,
    /// TMDB's page of every offer in the region, which links on to each service.
    pub link: Option<String>,
    /// Non-empty groups, in `OfferKind` order. Empty when nothing is offered
    /// in the region, or when unavailable.
    pub offers: Vec<OfferGroup>,
    pub fetched_at: Option<Timestamp>,
    /// Whether `WatchProvider::mine` could be set.
    pub my_services: ServicesState,
}

impl WhereToWatch {
    pub fn unavailable(region: String) -> Self {
        Self {
            region,
            state: WhereToWatchState::Unavailable,
            link: None,
            offers: Vec::new(),
            fetched_at: None,
            my_services: ServicesState::NotChosen,
        }
    }

    /// Marks the providers among `services` (the user's choice, or `None`
    /// when it couldn't be read).
    pub fn mark_services(&mut self, services: Option<&HashSet<u64>>) {
        self.my_services = ServicesState::of(services);
        for p in self.offers.iter_mut().flat_map(|g| &mut g.providers) {
            p.mine = services.is_some_and(|s| s.contains(&p.id));
        }
    }

    /// The user's services that include this title (streaming, free or with
    /// ads; renting or buying costs extra), once each, in TMDB's order.
    pub fn on_my_services(&self) -> Vec<&WatchProvider> {
        let mut seen = HashSet::new();
        self.offers
            .iter()
            .filter(|g| g.kind.is_included())
            .flat_map(|g| &g.providers)
            .filter(|p| p.mine && seen.insert(p.id))
            .collect()
    }

    /// "Netflix, Max", for the collapsed section.
    pub fn my_services_summary(&self) -> String {
        self.on_my_services()
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn is_unavailable(&self) -> bool {
        self.state == WhereToWatchState::Unavailable
    }

    pub fn is_stale(&self) -> bool {
        self.state == WhereToWatchState::Stale
    }

    pub fn fetched_on(&self) -> Option<String> {
        self.fetched_at.map(local_day)
    }

    /// "1 to stream · 7 to rent · 5 to buy", for the collapsed section.
    pub fn summary(&self) -> String {
        self.offers
            .iter()
            .map(|g| format!("{} {}", g.providers.len(), g.kind.summary_label()))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OfferKind {
    /// Included with a subscription.
    Stream,
    Free,
    /// Free with ads.
    Ads,
    Rent,
    Buy,
}

impl OfferKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stream => "Stream",
            Self::Free => "Free",
            Self::Ads => "Free with ads",
            Self::Rent => "Rent",
            Self::Buy => "Buy",
        }
    }

    /// Watchable at no extra cost: with a subscription, free, or with ads.
    pub fn is_included(self) -> bool {
        matches!(self, Self::Stream | Self::Free | Self::Ads)
    }

    fn summary_label(self) -> &'static str {
        match self {
            Self::Stream => "to stream",
            Self::Free => "free",
            Self::Ads => "free with ads",
            Self::Rent => "to rent",
            Self::Buy => "to buy",
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OfferGroup {
    pub kind: OfferKind,
    /// In TMDB's display priority order.
    pub providers: Vec<WatchProvider>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WatchProvider {
    pub id: u64,
    pub name: String,
    pub logo_url: Option<String>,
    /// One of the user's services.
    pub mine: bool,
}

/// Whether the user's streaming services are known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServicesState {
    NotChosen,
    Chosen,
    /// The saved choice couldn't be read, so nothing is marked as theirs.
    Unreadable,
}

impl ServicesState {
    pub fn of(services: Option<&HashSet<u64>>) -> Self {
        match services {
            None => Self::Unreadable,
            Some(s) if s.is_empty() => Self::NotChosen,
            Some(_) => Self::Chosen,
        }
    }

    pub fn is_chosen(self) -> bool {
        self == Self::Chosen
    }

    pub fn is_unreadable(self) -> bool {
        self == Self::Unreadable
    }
}

/// A streaming service, as TMDB lists it.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Service {
    pub id: u64,
    pub name: String,
    pub logo_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServiceOption {
    #[serde(flatten)]
    pub service: Service,
    pub chosen: bool,
    pub group: ServiceGroup,
}

/// Where a service appears on the services page, in page order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceGroup {
    /// The user's services.
    Mine,
    /// The region's major services, besides the user's.
    Popular,
    /// Every other service.
    More,
}

impl ServiceGroup {
    pub fn heading(self) -> &'static str {
        match self {
            Self::Mine => "Your services",
            Self::Popular => "Popular",
            Self::More => "More services",
        }
    }
}

/// The services page: the user's choice and every service to choose from.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServicesDetail {
    pub region: String,
    /// The saved choice, by name.
    pub chosen: Vec<Service>,
    /// In group order, alphabetical within each group, without add-on
    /// channels the user hasn't chosen. Only the chosen ones when TMDB's list
    /// is unavailable.
    pub options: Vec<ServiceOption>,
    /// Of TMDB's list of services.
    pub state: WhereToWatchState,
    /// False when only one of TMDB's movie and TV lists could be loaded.
    pub complete: bool,
    /// Oldest fetch behind the list.
    pub fetched_at: Option<Timestamp>,
}

impl ServicesDetail {
    pub fn is_unavailable(&self) -> bool {
        self.state == WhereToWatchState::Unavailable
    }

    pub fn is_stale(&self) -> bool {
        self.state == WhereToWatchState::Stale
    }

    pub fn fetched_on(&self) -> Option<String> {
        self.fetched_at.map(local_day)
    }

    /// The options split into their groups, for headings.
    pub fn groups(&self) -> Vec<(ServiceGroup, &[ServiceOption])> {
        self.options
            .chunk_by(|a, b| a.group == b.group)
            .map(|options| (options[0].group, options))
            .collect()
    }
}

/// Which of the user's services include a title on the To watch list.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Availability {
    /// Of the title's offers: stale or unavailable when TMDB couldn't be reached.
    pub state: WhereToWatchState,
    /// In TMDB's order; empty when unavailable.
    pub services: Vec<Service>,
    pub fetched_at: Option<Timestamp>,
}

impl Availability {
    pub fn is_unavailable(&self) -> bool {
        self.state == WhereToWatchState::Unavailable
    }

    pub fn is_stale(&self) -> bool {
        self.state == WhereToWatchState::Stale
    }

    pub fn fetched_on(&self) -> Option<String> {
        self.fetched_at.map(local_day)
    }
}

/// What stood behind the To watch list's service badges.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ListServices {
    pub state: ServicesState,
    /// Titles whose offers couldn't be loaded.
    pub unknown: usize,
    /// Oldest fetch among offers served stale because TMDB failed.
    pub stale_since: Option<Timestamp>,
}

impl ListServices {
    pub fn stale_on(&self) -> Option<String> {
        self.stale_since.map(local_day)
    }
}

/// Whether the To watch list shows every title or only those on the user's services.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ServicesFilter {
    #[default]
    Any,
    Mine,
}

impl ServicesFilter {
    /// The `services` query parameter; anything unrecognised means any.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("mine") => ServicesFilter::Mine,
            _ => ServicesFilter::Any,
        }
    }

    pub fn is_mine(self) -> bool {
        self == ServicesFilter::Mine
    }

    /// Titles of unknown availability are left out of `Mine`.
    pub fn matches(self, e: &FilmographyEntry) -> bool {
        match self {
            ServicesFilter::Any => true,
            ServicesFilter::Mine => e
                .availability
                .as_ref()
                .is_some_and(|a| !a.services.is_empty()),
        }
    }
}

/// A person's part in one title: a character for cast, job titles for crew.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PersonCredit {
    pub person_id: u64,
    pub name: String,
    pub role: Option<String>,
    /// TV only: episodes this person is credited on. Left out for cast until
    /// the show is watched as a whole (see `hide_cast_episode_counts`).
    pub episode_count: Option<u32>,
    pub profile_url: Option<String>,
}

impl PersonCredit {
    pub fn person_path(&self) -> String {
        person_path(self.person_id)
    }

    pub fn episodes_label(&self) -> Option<String> {
        self.episode_count.map(episodes_label)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KeyRole {
    Creator,
    Director,
    Writer,
}

impl KeyRole {
    pub fn label(self) -> &'static str {
        match self {
            KeyRole::Creator => "Created by",
            KeyRole::Director => "Directed by",
            KeyRole::Writer => "Written by",
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct KeyCrew {
    pub role: KeyRole,
    pub people: Vec<PersonCredit>,
}

/// Everyone credited on a title, crew grouped by department.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct TitleCredits {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    pub cast: Vec<PersonCredit>,
    pub crew: Vec<Department>,
    pub metadata: Metadata,
}

impl TitleCredits {
    pub fn title_path(&self) -> String {
        title_path(self.kind, self.id)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Department {
    pub name: String,
    pub people: Vec<PersonCredit>,
}

impl Department {
    /// Fragment identifier for in-page navigation, e.g. "dept-visual-effects".
    pub fn anchor(&self) -> String {
        anchor(&self.name)
    }
}

fn anchor(name: &str) -> String {
    format!("dept-{}", slug(name))
}

/// Lowercase ASCII letters and digits joined by single hyphens.
fn slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    slug.join("-")
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SeasonSummary {
    pub number: u32,
    pub name: String,
    pub year: Option<String>,
    pub episode_count: Option<u32>,
    pub poster_url: Option<String>,
}

impl SeasonSummary {
    pub fn episodes_label(&self) -> Option<String> {
        self.episode_count.map(episodes_label)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SeasonDetail {
    pub show_id: u64,
    pub show_title: String,
    pub number: u32,
    pub name: String,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Vec<Episode>,
    /// Where the episodes' `rating`s came from, and why they're missing if they are.
    pub episode_ratings: EpisodeRatings,
    /// Neighbouring seasons in the show's season order, for navigation.
    pub previous: Option<SeasonSummary>,
    pub next: Option<SeasonSummary>,
    pub metadata: Metadata,
}

impl SeasonDetail {
    pub fn show_path(&self) -> String {
        title_path(MediaKind::Tv, self.show_id)
    }

    pub fn path(&self) -> String {
        format!("/tv/{}/season/{}", self.show_id, self.number)
    }

    /// Where the form marking this season watched posts to.
    pub fn watched_path(&self) -> String {
        format!("{}/watched", self.path())
    }

    pub fn show_watched_path(&self) -> String {
        format!("{}/watched", self.show_path())
    }

    pub fn season_path(&self, season: &SeasonSummary) -> String {
        format!("/tv/{}/season/{}", self.show_id, season.number)
    }

    pub fn air_date_display(&self) -> Option<String> {
        self.air_date.as_deref().map(display_date)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Episode {
    pub number: u32,
    /// TMDB's name, or "Episode N" when it has none.
    pub title: String,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub still_url: Option<String>,
    /// IMDb's episode score; None when IMDb has none or ratings are unavailable.
    pub rating: Option<Rating>,
}

impl Episode {
    pub fn air_date_display(&self) -> Option<String> {
        self.air_date.as_deref().map(display_date)
    }
}

/// One TV episode with everyone credited on it.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EpisodeDetail {
    pub show_id: u64,
    pub show_title: String,
    pub season_number: u32,
    pub season_name: String,
    pub number: u32,
    /// TMDB's name, or "Episode N" when it has none.
    pub title: String,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub still_url: Option<String>,
    /// IMDb's episode score; None when IMDb has none or ratings are unavailable.
    pub rating: Option<Rating>,
    /// Where `rating` came from, and why it's missing if it is.
    pub episode_ratings: EpisodeRatings,
    /// Directors and writers of this episode.
    pub key_crew: Vec<KeyCrew>,
    /// Series regulars who appear in this episode, in billing order.
    pub cast: Vec<PersonCredit>,
    pub guest_stars: Vec<PersonCredit>,
    pub crew: Vec<Department>,
    /// Neighbouring episode numbers within the season, for navigation.
    pub previous: Option<u32>,
    pub next: Option<u32>,
    pub metadata: Metadata,
}

impl EpisodeDetail {
    pub fn show_path(&self) -> String {
        title_path(MediaKind::Tv, self.show_id)
    }

    pub fn season_path(&self) -> String {
        format!("/tv/{}/season/{}", self.show_id, self.season_number)
    }

    pub fn path(&self) -> String {
        format!("{}/episode/{}", self.season_path(), self.number)
    }

    /// Where the form marking this episode watched posts to.
    pub fn watched_path(&self) -> String {
        format!("{}/watched", self.path())
    }

    pub fn air_date_display(&self) -> Option<String> {
        self.air_date.as_deref().map(display_date)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct PersonDetail {
    pub id: u64,
    pub name: String,
    pub known_for: Option<String>,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    pub profile_url: Option<String>,
    pub tmdb_url: String,
    /// The person's most-voted titles, from their main department first.
    pub known_for_titles: Vec<FilmographyEntry>,
    /// Grouped by department, the person's main department first.
    pub filmography: Vec<FilmographyGroup>,
    pub sort: FilmographySort,
    /// How complete the credits' scores are (behind the order, when sorted
    /// by rating). Always present.
    pub sort_ratings: Option<SortRatings>,
    pub metadata: Metadata,
}

impl PersonDetail {
    pub fn path(&self) -> String {
        person_path(self.id)
    }

    /// Leaves out episode counts of acting credits on shows not watched as a
    /// whole (all of them if `watched` couldn't be read); see
    /// `hide_cast_episode_counts`. Known-for entries don't say which credit
    /// their count is from, so they're treated as acting.
    pub fn hide_acting_episode_counts(&mut self, watched: Option<&HashSet<u64>>) {
        let unwatched = |e: &FilmographyEntry| {
            e.kind == MediaKind::Tv && !watched.is_some_and(|shows| shows.contains(&e.id))
        };
        let acting = self
            .filmography
            .iter_mut()
            .filter(|g| g.is_acting())
            .flat_map(|g| &mut g.entries);
        for e in self.known_for_titles.iter_mut().chain(acting) {
            if unwatched(e) {
                e.episode_count = None;
            }
        }
    }

    pub fn biography_paragraphs(&self) -> Vec<&str> {
        self.biography
            .as_deref()
            .map(|b| b.lines().map(str::trim).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default()
    }

    pub fn birthday_display(&self) -> Option<String> {
        self.birthday.as_deref().map(display_date)
    }

    pub fn deathday_display(&self) -> Option<String> {
        self.deathday.as_deref().map(display_date)
    }
}

/// How a person's filmography is ordered within each department.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilmographySort {
    /// Newest first; undated (usually unreleased) entries lead.
    #[default]
    Date,
    /// Highest `sort_score` first, then unscored entries by date.
    Rating,
}

impl FilmographySort {
    /// The `sort` query parameter; anything unrecognised means date.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("rating") => FilmographySort::Rating,
            _ => FilmographySort::Date,
        }
    }

    pub fn is_rating(self) -> bool {
        self == FilmographySort::Rating
    }
}

/// Which titles a person's filmography page lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KindFilter {
    #[default]
    All,
    Movie,
    Tv,
}

impl KindFilter {
    pub const ALL: [KindFilter; 3] = [KindFilter::All, KindFilter::Movie, KindFilter::Tv];

    /// The `kind` query parameter; anything unrecognised means all.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("movie") => KindFilter::Movie,
            Some("tv") => KindFilter::Tv,
            _ => KindFilter::All,
        }
    }

    /// The `kind` query value selecting this filter; none for all.
    pub fn query(self) -> Option<&'static str> {
        match self {
            KindFilter::All => None,
            KindFilter::Movie => Some("movie"),
            KindFilter::Tv => Some("tv"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            KindFilter::All => "All",
            KindFilter::Movie => "Movies",
            KindFilter::Tv => "TV",
        }
    }

    pub fn matches(self, kind: MediaKind) -> bool {
        match self {
            KindFilter::All => true,
            KindFilter::Movie => kind == MediaKind::Movie,
            KindFilter::Tv => kind == MediaKind::Tv,
        }
    }
}

/// The "To watch" list: saved movies and TV shows in the chosen order.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WatchlistDetail {
    pub titles: Vec<FilmographyEntry>,
    pub sort: WatchlistSort,
    /// Only when sorted by rating: how complete the scores behind the order are.
    pub sort_ratings: Option<SortRatings>,
    /// To watch only: the user's services and how complete the titles' offers are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<ListServices>,
}

/// How the "To watch" list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchlistSort {
    /// Most recently added first.
    #[default]
    Added,
    /// Highest `sort_score` first, then unscored titles most recently added first.
    Rating,
}

impl WatchlistSort {
    /// The `sort` query parameter; anything unrecognised means added.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("rating") => WatchlistSort::Rating,
            _ => WatchlistSort::Added,
        }
    }

    pub fn is_rating(self) -> bool {
        self == WatchlistSort::Rating
    }
}

/// What stood behind a list's scores (and a by-rating order): credits
/// without a score, and whether some scores were stale or couldn't be loaded.
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct SortRatings {
    pub unrated: usize,
    /// Oldest fetch among scores served stale because OMDb failed.
    pub stale_since: Option<Timestamp>,
    /// Set when ratings aren't configured or some lookups failed outright.
    pub unavailable_reason: Option<UnavailableReason>,
}

impl SortRatings {
    pub fn stale_on(&self) -> Option<String> {
        self.stale_since.map(local_day)
    }

    pub fn not_configured(&self) -> bool {
        self.unavailable_reason == Some(UnavailableReason::NotConfigured)
    }

    pub fn unavailable_message(&self) -> Option<&'static str> {
        match self.unavailable_reason? {
            UnavailableReason::NotConfigured => {
                Some("Ratings aren't configured on this server, so credits stay in date order.")
            }
            UnavailableReason::NoImdbId | UnavailableReason::ProviderError => {
                Some("Some ratings couldn't be loaded right now.")
            }
        }
    }
}

/// Every movie and TV show with a watched mark, however it was made: by
/// hand, by favoriting a show, or from what played on an Apple TV.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WatchedList {
    /// In `sort` order.
    pub titles: Vec<WatchedTitle>,
    pub sort: WatchedSort,
    /// Titles whose name, year and poster couldn't be loaded from TMDB.
    pub unavailable: usize,
    /// Oldest fetch among title details served stale because TMDB failed.
    pub stale_since: Option<Timestamp>,
}

impl WatchedList {
    pub fn stale_on(&self) -> Option<String> {
        self.stale_since.map(local_day)
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct WatchedTitle {
    pub kind: MediaKind,
    pub id: u64,
    /// `null` when TMDB couldn't be reached and nothing was cached.
    pub title: Option<String>,
    pub year: Option<String>,
    pub poster_url: Option<String>,
    /// When the latest of its marks was made.
    pub last_watched_at: Timestamp,
    /// TV only: what is marked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ShowProgress>,
    /// How its marks were made; at least one.
    pub sources: Vec<WatchedSource>,
}

impl WatchedTitle {
    pub fn path(&self) -> String {
        title_path(self.kind, self.id)
    }

    /// The title, or what it is when TMDB couldn't say.
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| format!("{} #{}", self.kind.label(), self.id))
    }

    pub fn watched_on(&self) -> String {
        local_day(self.last_watched_at)
    }

    pub fn from_apple_tv(&self) -> bool {
        self.sources.contains(&WatchedSource::AppleTv)
    }

    /// "Whole show", "Seasons 1, 2 · 3 episodes", "1 episode"; none for movies.
    pub fn progress_label(&self) -> Option<String> {
        self.progress.as_ref().map(ShowProgress::label)
    }
}

/// What of a TV show is marked watched. Marks are independent, so episodes
/// may also lie within marked seasons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct ShowProgress {
    /// The whole show, every season included.
    pub show: bool,
    /// Seasons marked one at a time, ascending.
    pub seasons: Vec<u32>,
    /// How many episodes are marked one at a time.
    pub episodes: u32,
}

impl ShowProgress {
    pub fn label(&self) -> String {
        if self.show {
            return "Whole show".to_owned();
        }
        let mut parts = Vec::new();
        if let Some(seasons) = seasons_label(&self.seasons) {
            parts.push(seasons);
        }
        if self.episodes > 0 {
            parts.push(episodes_label(self.episodes));
        }
        parts.join(" · ")
    }
}

/// Ascending seasons with runs collapsed and season 0 as specials:
/// "Season 2", "Seasons 1–3, 5", "Seasons 1–9, specials", "Specials".
pub fn seasons_label(seasons: &[u32]) -> Option<String> {
    let specials = seasons.contains(&0);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &n in seasons.iter().filter(|&&n| n > 0) {
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == n => *end = n,
            _ => runs.push((n, n)),
        }
    }
    let mut parts: Vec<String> = runs
        .iter()
        .map(|&(start, end)| match end - start {
            0 => start.to_string(),
            1 => format!("{start}, {end}"),
            _ => format!("{start}–{end}"),
        })
        .collect();
    let numbered = !parts.is_empty();
    let one = runs.len() == 1 && runs[0].0 == runs[0].1;
    if specials {
        parts.push(if numbered { "specials" } else { "Specials" }.to_owned());
    }
    match (numbered, one) {
        (false, _) if !specials => None,
        (false, _) => Some(parts.join(", ")),
        (true, true) => Some(format!("Season {}", parts.join(", "))),
        (true, false) => Some(format!("Seasons {}", parts.join(", "))),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchedSource {
    /// Marked in the web UI or terminal client, or by favoriting a show.
    Manual,
    /// Marked from what played on an Apple TV.
    AppleTv,
}

/// How the Watched list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WatchedSort {
    /// Most recently watched first.
    #[default]
    Recent,
    /// A to Z; titles TMDB couldn't name last.
    Title,
}

impl WatchedSort {
    /// The `sort` query parameter; anything unrecognised means recent.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("title") => WatchedSort::Title,
            _ => WatchedSort::Recent,
        }
    }

    pub fn is_title(self) -> bool {
        self == WatchedSort::Title
    }
}

/// Which marks the Watched list shows titles for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceFilter {
    #[default]
    Any,
    Manual,
    AppleTv,
}

impl SourceFilter {
    pub const ALL: [SourceFilter; 3] = [
        SourceFilter::Any,
        SourceFilter::Manual,
        SourceFilter::AppleTv,
    ];

    /// The `source` query parameter; anything unrecognised means any.
    pub fn from_query(value: Option<&str>) -> Self {
        match value {
            Some("manual") => SourceFilter::Manual,
            Some("apple_tv") => SourceFilter::AppleTv,
            _ => SourceFilter::Any,
        }
    }

    /// The `source` query value selecting this filter; none for any.
    pub fn query(self) -> Option<&'static str> {
        match self {
            SourceFilter::Any => None,
            SourceFilter::Manual => Some("manual"),
            SourceFilter::AppleTv => Some("apple_tv"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceFilter::Any => "Any way",
            SourceFilter::Manual => "By hand",
            SourceFilter::AppleTv => "Apple TV",
        }
    }

    pub fn matches(self, t: &WatchedTitle) -> bool {
        match self {
            SourceFilter::Any => true,
            SourceFilter::Manual => t.sources.contains(&WatchedSource::Manual),
            SourceFilter::AppleTv => t.sources.contains(&WatchedSource::AppleTv),
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct FilmographyGroup {
    pub department: String,
    /// In the page's `FilmographySort` order.
    pub entries: Vec<FilmographyEntry>,
}

impl FilmographyGroup {
    pub fn anchor(&self) -> String {
        anchor(&self.department)
    }

    /// The department as the person page's `dept` query value.
    pub fn slug(&self) -> String {
        slug(&self.department)
    }

    pub fn is_acting(&self) -> bool {
        self.department == "Acting"
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct FilmographyEntry {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    /// Characters for acting credits, job titles otherwise.
    pub role: Option<String>,
    pub episode_count: Option<u32>,
    pub poster_url: Option<String>,
    /// When sorted by rating: the score it was sorted by, Rotten Tomatoes if
    /// there is one, else IMDb.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Rating>,
    /// To watch only, once services are chosen: which of them include it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<Availability>,
    /// Lists only: whether it's saved and watched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marks: Option<TitleMarks>,
    /// To watch and Favorites only: a movie playing in the region's
    /// theaters now. Omitted when not.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub in_theaters: bool,
}

impl FilmographyEntry {
    pub fn path(&self) -> String {
        title_path(self.kind, self.id)
    }

    pub fn episodes_label(&self) -> Option<String> {
        self.episode_count.map(episodes_label)
    }
}

/// Where a score comes from. Each source has its own scale; never compare
/// raw scores across sources (see `Rating::percent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RatingSource {
    Imdb,
    RottenTomatoes,
}

impl RatingSource {
    pub fn label(self) -> &'static str {
        match self {
            RatingSource::Imdb => "IMDb",
            RatingSource::RottenTomatoes => "Rotten Tomatoes",
        }
    }

    /// The score shown first for a kind of title.
    pub fn preferred_for(kind: MediaKind) -> Self {
        match kind {
            MediaKind::Movie => RatingSource::RottenTomatoes,
            MediaKind::Tv => RatingSource::Imdb,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Rating {
    pub source: RatingSource,
    /// Out of `scale`: IMDb is 0–10, the Rotten Tomatoes Tomatometer 0–100 (%).
    pub score: f64,
    pub scale: u16,
    pub votes: Option<u64>,
}

impl Rating {
    pub fn display(&self) -> String {
        match self.source {
            RatingSource::RottenTomatoes => format!("{}%", self.score),
            RatingSource::Imdb => format!("{:.1}/{}", self.score, self.scale),
        }
    }

    /// The score as a share of its scale, 0–100, so a filmography can rank
    /// a Rotten Tomatoes score against an IMDb fallback.
    pub fn percent(&self) -> f64 {
        self.score / f64::from(self.scale.max(1)) * 100.0
    }

    /// `percent` as a whole number, the figure inside a score ring: IMDb's
    /// one decimal makes 9.2/10 exactly 92.
    pub fn ring_percent(&self) -> u32 {
        self.percent().round().clamp(0.0, 100.0) as u32
    }

    /// The ring's color class: the source, and for Rotten Tomatoes the verdict.
    pub fn tone(&self) -> &'static str {
        match self.source {
            RatingSource::Imdb => "imdb",
            RatingSource::RottenTomatoes if self.is_fresh() => "rt-fresh",
            RatingSource::RottenTomatoes => "rt-rotten",
        }
    }

    /// Accessible name and tooltip for a ring, with the score in its own
    /// form: "Rotten Tomatoes: Fresh, 83%" or "IMDb, 9.2/10".
    pub fn ring_label(&self) -> String {
        format!("{}, {}", self.icon_label(), self.display())
    }

    /// The line under a title page's source name: IMDb's own score and
    /// votes ("8.7/10 · 2,300,000 votes"), or the Tomatometer verdict.
    pub fn note(&self) -> String {
        match self.source {
            RatingSource::RottenTomatoes if self.is_fresh() => "Fresh".to_owned(),
            RatingSource::RottenTomatoes => "Rotten".to_owned(),
            RatingSource::Imdb => match self.votes_display() {
                Some(votes) => format!("{} · {votes} votes", self.display()),
                None => self.display(),
            },
        }
    }

    /// The score a credit is sorted by: Rotten Tomatoes, falling back to IMDb.
    pub fn for_sorting(ratings: &Ratings) -> Option<Rating> {
        let scores = || ratings.preferred.iter().chain(&ratings.others);
        scores()
            .find(|r| r.source == RatingSource::RottenTomatoes)
            .or_else(|| scores().find(|r| r.source == RatingSource::Imdb))
            .cloned()
    }

    /// Rotten Tomatoes calls a Tomatometer of 60% or more "Fresh".
    fn is_fresh(&self) -> bool {
        self.score >= 60.0
    }

    /// Path of the icon shown in place of the source's label. Rotten Tomatoes
    /// scores get the fresh tomato or the rotten splat, as on the site.
    pub fn icon(&self) -> &'static str {
        match self.source {
            RatingSource::Imdb => "/static/imdb.svg",
            RatingSource::RottenTomatoes if self.is_fresh() => "/static/rt-fresh.svg",
            RatingSource::RottenTomatoes => "/static/rt-rotten.svg",
        }
    }

    /// Accessible name for `icon`.
    pub fn icon_label(&self) -> &'static str {
        match self.source {
            RatingSource::Imdb => "IMDb",
            RatingSource::RottenTomatoes if self.is_fresh() => "Rotten Tomatoes: Fresh",
            RatingSource::RottenTomatoes => "Rotten Tomatoes: Rotten",
        }
    }

    /// Vote count with thousands separators, e.g. "2,123,456".
    pub fn votes_display(&self) -> Option<String> {
        let digits = self.votes?.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        Some(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RatingsState {
    /// Within the freshness window.
    Fresh,
    /// Cached data served because the provider could not be reached.
    Stale,
    /// No data at all; see `unavailable_reason`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The server has no ratings provider key.
    NotConfigured,
    /// TMDB has no IMDb identifier for the title, so there is nothing safe to look up.
    NoImdbId,
    /// The provider failed (timeout, quota, outage) and nothing was cached.
    ProviderError,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct Ratings {
    pub state: RatingsState,
    pub unavailable_reason: Option<UnavailableReason>,
    /// Always `omdb` today; scores' own origins are in `Rating::source`.
    pub provider: &'static str,
    pub imdb_id: Option<String>,
    pub preferred_source: RatingSource,
    /// None when the preferred source has no score; never substituted.
    pub preferred: Option<Rating>,
    /// Scores from non-preferred sources, each labeled with its source.
    pub others: Vec<Rating>,
    pub fetched_at: Option<Timestamp>,
}

impl Ratings {
    pub fn unavailable(
        kind: MediaKind,
        reason: UnavailableReason,
        imdb_id: Option<String>,
    ) -> Self {
        Self {
            state: RatingsState::Unavailable,
            unavailable_reason: Some(reason),
            provider: "omdb",
            imdb_id,
            preferred_source: RatingSource::preferred_for(kind),
            preferred: None,
            others: Vec::new(),
            fetched_at: None,
        }
    }

    pub fn from_scores(
        kind: MediaKind,
        state: RatingsState,
        imdb_id: String,
        scores: Vec<Rating>,
        fetched_at: Timestamp,
    ) -> Self {
        let preferred_source = RatingSource::preferred_for(kind);
        let (preferred, others): (Vec<_>, Vec<_>) = scores
            .into_iter()
            .partition(|r| r.source == preferred_source);
        Self {
            state,
            unavailable_reason: None,
            provider: "omdb",
            imdb_id: Some(imdb_id),
            preferred_source,
            preferred: preferred.into_iter().next(),
            others,
            fetched_at: Some(fetched_at),
        }
    }

    pub fn is_unavailable(&self) -> bool {
        self.state == RatingsState::Unavailable
    }

    pub fn is_stale(&self) -> bool {
        self.state == RatingsState::Stale
    }

    /// Short human description of `fetched_at`, e.g. "Oct 19, 2026".
    pub fn fetched_on(&self) -> Option<String> {
        self.fetched_at.map(local_day)
    }

    pub fn unavailable_message(&self) -> &'static str {
        match self.unavailable_reason {
            Some(UnavailableReason::NotConfigured) => "Ratings aren't configured on this server.",
            Some(UnavailableReason::NoImdbId) => "No IMDb entry is linked to this title.",
            Some(UnavailableReason::ProviderError) | None => {
                "Ratings couldn't be loaded right now."
            }
        }
    }
}

/// Provenance of a season's per-episode IMDb ratings.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct EpisodeRatings {
    pub state: RatingsState,
    pub unavailable_reason: Option<UnavailableReason>,
    /// Always `omdb` today.
    pub provider: &'static str,
    /// The show's IMDb identifier, which the season was looked up by.
    pub imdb_id: Option<String>,
    pub fetched_at: Option<Timestamp>,
}

impl EpisodeRatings {
    pub fn unavailable(reason: UnavailableReason, imdb_id: Option<String>) -> Self {
        Self {
            state: RatingsState::Unavailable,
            unavailable_reason: Some(reason),
            provider: "omdb",
            imdb_id,
            fetched_at: None,
        }
    }

    pub fn found(state: RatingsState, imdb_id: String, fetched_at: Timestamp) -> Self {
        Self {
            state,
            unavailable_reason: None,
            provider: "omdb",
            imdb_id: Some(imdb_id),
            fetched_at: Some(fetched_at),
        }
    }

    pub fn is_unavailable(&self) -> bool {
        self.state == RatingsState::Unavailable
    }

    pub fn is_stale(&self) -> bool {
        self.state == RatingsState::Stale
    }

    pub fn fetched_on(&self) -> Option<String> {
        self.fetched_at.map(local_day)
    }

    pub fn unavailable_message(&self) -> &'static str {
        match self.unavailable_reason {
            Some(UnavailableReason::NotConfigured) => "Ratings aren't configured on this server.",
            Some(UnavailableReason::NoImdbId) => "No IMDb entry is linked to this show.",
            Some(UnavailableReason::ProviderError) | None => {
                "Episode ratings couldn't be loaded right now."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_days_read_as_coming_until_they_pass() {
        let today = "2026-10-01".parse().unwrap();
        assert_eq!(release_day("2010-07-16", today), "Jul 16, 2010");
        assert_eq!(release_day("2026-10-01", today), "Oct 1, 2026");
        assert_eq!(release_day("2026-10-16", today), "Coming Oct 16");
        assert_eq!(release_day("2027-01-08", today), "Coming Jan 8, 2027");
        assert_eq!(release_day("soon", today), "soon");
    }

    #[test]
    fn seasons_collapse_into_runs_with_specials_last() {
        let label = |s: &[u32]| seasons_label(s);
        assert_eq!(label(&[]), None);
        assert_eq!(label(&[2]).as_deref(), Some("Season 2"));
        assert_eq!(label(&[1, 2]).as_deref(), Some("Seasons 1, 2"));
        assert_eq!(label(&[1, 2, 3, 5]).as_deref(), Some("Seasons 1–3, 5"));
        assert_eq!(
            label(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]).as_deref(),
            Some("Seasons 1–9, specials")
        );
        assert_eq!(label(&[0]).as_deref(), Some("Specials"));
    }

    fn rt(score: f64) -> Rating {
        Rating {
            source: RatingSource::RottenTomatoes,
            score,
            scale: 100,
            votes: None,
        }
    }

    #[test]
    fn sorting_prefers_rotten_tomatoes_and_falls_back_to_imdb() {
        let imdb = Rating {
            source: RatingSource::Imdb,
            score: 8.7,
            scale: 10,
            votes: None,
        };
        let tv = Ratings::from_scores(
            MediaKind::Tv,
            RatingsState::Fresh,
            "tt1".into(),
            vec![imdb.clone(), rt(91.0)],
            Timestamp::UNIX_EPOCH,
        );
        assert_eq!(Rating::for_sorting(&tv), Some(rt(91.0)));
        let movie = Ratings::from_scores(
            MediaKind::Movie,
            RatingsState::Fresh,
            "tt1".into(),
            vec![imdb.clone()],
            Timestamp::UNIX_EPOCH,
        );
        assert_eq!(Rating::for_sorting(&movie), Some(imdb.clone()));
        assert!((imdb.percent() - 87.0).abs() < 1e-9);
        assert_eq!(
            Rating::for_sorting(&Ratings::unavailable(
                MediaKind::Movie,
                UnavailableReason::NoImdbId,
                None
            )),
            None
        );
    }

    #[test]
    fn rotten_tomatoes_icon_follows_the_fresh_threshold() {
        assert_eq!(rt(60.0).icon(), "/static/rt-fresh.svg");
        assert_eq!(rt(59.0).icon(), "/static/rt-rotten.svg");
        assert_eq!(rt(59.0).icon_label(), "Rotten Tomatoes: Rotten");
    }
}
