//! What the terminal client reads from the server's `/api/…` JSON. Fields it
//! does not show are left out, and unknown enum values decode as `Other`, so
//! a newer server stays readable.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Movie,
    Tv,
}

impl Kind {
    pub fn slug(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Tv => "tv",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Movie => "Movie",
            Kind::Tv => "TV",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchHit {
    Movie(TitleHit),
    Tv(TitleHit),
    Person(PersonHit),
}

#[derive(Clone, Debug, Deserialize)]
pub struct TitleHit {
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonHit {
    pub id: u64,
    pub name: String,
    pub known_for_department: Option<String>,
    #[serde(default)]
    pub known_for: Vec<KnownForHit>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KnownForHit {
    pub title: String,
}

/// What a recently viewed entry links to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecentKind {
    Movie,
    Tv,
    Person,
    #[serde(other)]
    Other,
}

impl RecentKind {
    pub fn slug(self) -> &'static str {
        match self {
            RecentKind::Movie => "movie",
            RecentKind::Tv => "tv",
            RecentKind::Person => "person",
            RecentKind::Other => "other",
        }
    }
}

impl From<Kind> for RecentKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Movie => RecentKind::Movie,
            Kind::Tv => RecentKind::Tv,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RecentViews {
    pub recent: Vec<RecentView>,
}

/// A title or person page opened on the web or here, most recent first.
#[derive(Clone, Debug, Deserialize)]
pub struct RecentView {
    pub kind: RecentKind,
    pub id: u64,
    pub name: String,
    pub year: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Title {
    pub kind: Kind,
    pub id: u64,
    pub title: String,
    pub tagline: Option<String>,
    pub overview: Option<String>,
    pub year: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    pub runtime_minutes: Option<u32>,
    pub seasons: Option<u32>,
    pub episodes: Option<u32>,
    pub ratings: Ratings,
    pub where_to_watch: WhereToWatch,
    #[serde(default)]
    pub key_crew: Vec<KeyCrew>,
    #[serde(default)]
    pub cast: Vec<Credit>,
    #[serde(default)]
    pub cast_total: usize,
    /// TV only.
    #[serde(default)]
    pub season_list: Vec<SeasonSummary>,
    /// TV only; a server too old to send it can't say what's watched.
    pub watched: Option<Watched>,
    /// A server too old to send them can't say what's saved.
    pub marks: Option<Marks>,
    pub metadata: Metadata,
}

/// Whether a title is on To watch and Favorites and watched (a TV show as a
/// whole); each `None` when the server couldn't read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Marks {
    pub to_watch: Option<bool>,
    pub favorite: Option<bool>,
    pub watched: Option<bool>,
}

/// A title's marks after a change; a TV show's also carry its watched marks.
#[derive(Clone, Debug, Deserialize)]
pub struct MarksUpdate {
    pub marks: Marks,
    pub watched: Option<Watched>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SeasonSummary {
    pub number: u32,
    pub name: String,
    pub year: Option<String>,
    pub episode_count: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchedState {
    Read,
    #[serde(other)]
    Unreadable,
}

/// A show's watched marks. Spoilers stay hidden until an episode, its season
/// or the show is watched, as on the web pages.
#[derive(Clone, Debug, Deserialize)]
pub struct Watched {
    pub state: WatchedState,
    #[serde(default)]
    pub show: bool,
    #[serde(default)]
    pub seasons: Vec<u32>,
    #[serde(default)]
    pub episodes: Vec<EpisodeMark>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EpisodeMark {
    pub season: u32,
    pub number: u32,
    #[serde(default)]
    pub apple_tv: bool,
}

impl Watched {
    /// Whether the marks could be read; `None` (an older server) can't be.
    pub fn readable(watched: Option<&Self>) -> bool {
        watched.is_some_and(|w| w.state == WatchedState::Read)
    }

    pub fn season(&self, number: u32) -> bool {
        self.show || self.seasons.contains(&number)
    }

    pub fn episode(&self, season: u32, number: u32) -> bool {
        self.season(season) || self.mark(season, number).is_some()
    }

    /// Whether the episode's own mark came from the Apple TV.
    pub fn from_apple_tv(&self, season: u32, number: u32) -> bool {
        self.mark(season, number).is_some_and(|m| m.apple_tv)
    }

    fn mark(&self, season: u32, number: u32) -> Option<&EpisodeMark> {
        self.episodes
            .iter()
            .find(|m| m.season == season && m.number == number)
    }
}

/// Where a season's or episode's IMDb scores came from.
#[derive(Clone, Debug, Deserialize)]
pub struct EpisodeRatings {
    pub state: State,
    pub unavailable_reason: Option<UnavailableReason>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Season {
    pub show_id: u64,
    pub show_title: String,
    pub number: u32,
    pub name: String,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    #[serde(default)]
    pub episodes: Vec<SeasonEpisode>,
    pub episode_ratings: EpisodeRatings,
    pub watched: Option<Watched>,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SeasonEpisode {
    pub number: u32,
    pub title: String,
    pub air_date: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub rating: Option<Rating>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Episode {
    pub show_id: u64,
    pub show_title: String,
    pub season_number: u32,
    pub season_name: String,
    pub number: u32,
    pub title: String,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub rating: Option<Rating>,
    pub episode_ratings: EpisodeRatings,
    #[serde(default)]
    pub key_crew: Vec<KeyCrew>,
    /// Series regulars credited on this episode.
    #[serde(default)]
    pub cast: Vec<Credit>,
    #[serde(default)]
    pub guest_stars: Vec<Credit>,
    pub previous: Option<u32>,
    pub next: Option<u32>,
    pub watched: Option<Watched>,
    pub metadata: Metadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Fresh,
    Stale,
    Unavailable,
    #[serde(other)]
    Other,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Metadata {
    pub state: State,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingSource {
    Imdb,
    RottenTomatoes,
    #[serde(other)]
    Other,
}

impl RatingSource {
    pub fn label(self) -> &'static str {
        match self {
            RatingSource::Imdb => "IMDb",
            RatingSource::RottenTomatoes => "Rotten Tomatoes",
            RatingSource::Other => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    NotConfigured,
    NoImdbId,
    #[serde(other)]
    ProviderError,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Ratings {
    pub state: State,
    pub unavailable_reason: Option<UnavailableReason>,
    pub preferred_source: RatingSource,
    pub preferred: Option<Rating>,
    #[serde(default)]
    pub others: Vec<Rating>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rating {
    pub source: RatingSource,
    pub score: f64,
    pub scale: u32,
    pub votes: Option<u64>,
}

impl Rating {
    /// Same form as the web pages: `84%` or `8.3/10`.
    pub fn display(&self) -> String {
        match self.source {
            RatingSource::RottenTomatoes => format!("{}%", self.score),
            _ => format!("{:.1}/{}", self.score, self.scale),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct WhereToWatch {
    pub region: String,
    pub state: State,
    #[serde(default)]
    pub offers: Vec<OfferGroup>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferKind {
    Stream,
    Free,
    Ads,
    Rent,
    Buy,
    #[serde(other)]
    Other,
}

impl OfferKind {
    pub fn label(self) -> &'static str {
        match self {
            OfferKind::Stream => "Stream",
            OfferKind::Free => "Free",
            OfferKind::Ads => "Free with ads",
            OfferKind::Rent => "Rent",
            OfferKind::Buy => "Buy",
            OfferKind::Other => "Other",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct OfferGroup {
    pub kind: OfferKind,
    pub providers: Vec<Provider>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Provider {
    pub name: String,
    #[serde(default)]
    pub mine: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyRole {
    Creator,
    Director,
    Writer,
    #[serde(other)]
    Other,
}

impl KeyRole {
    pub fn label(self) -> &'static str {
        match self {
            KeyRole::Creator => "Created by",
            KeyRole::Director => "Directed by",
            KeyRole::Writer => "Written by",
            KeyRole::Other => "Crew",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyCrew {
    pub role: KeyRole,
    pub people: Vec<Credit>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Credit {
    pub person_id: u64,
    pub name: String,
    pub role: Option<String>,
    pub episode_count: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Person {
    pub id: u64,
    pub name: String,
    pub known_for: Option<String>,
    pub biography: Option<String>,
    pub birthday: Option<String>,
    pub deathday: Option<String>,
    pub place_of_birth: Option<String>,
    #[serde(default)]
    pub known_for_titles: Vec<PersonCredit>,
    #[serde(default)]
    pub filmography: Vec<FilmographyGroup>,
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FilmographyGroup {
    pub department: String,
    pub entries: Vec<PersonCredit>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonCredit {
    pub kind: Kind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    pub role: Option<String>,
    pub episode_count: Option<u32>,
}

/// The To watch list or Favorites, already filtered by the server.
#[derive(Clone, Debug, Deserialize)]
pub struct List {
    pub titles: Vec<ListEntry>,
    /// Only when sorted by rating.
    pub sort_ratings: Option<SortRatings>,
    /// To watch only.
    pub services: Option<ListServices>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ListEntry {
    pub kind: Kind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    /// Only when sorted by rating: Rotten Tomatoes, else IMDb.
    pub rating: Option<Rating>,
    /// To watch only, once services are chosen.
    pub availability: Option<Availability>,
    pub marks: Option<Marks>,
}

/// Which of the user's services include a title.
#[derive(Clone, Debug, Deserialize)]
pub struct Availability {
    pub state: State,
    #[serde(default)]
    pub services: Vec<Service>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Service {
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServicesState {
    NotChosen,
    Chosen,
    #[serde(other)]
    Unreadable,
}

/// What stood behind the To watch list's service badges.
#[derive(Clone, Debug, Deserialize)]
pub struct ListServices {
    pub state: ServicesState,
    /// Titles whose offers couldn't be loaded.
    #[serde(default)]
    pub unknown: usize,
    pub stale_since: Option<String>,
}

/// How complete the scores behind a by-rating order are.
#[derive(Clone, Debug, Deserialize)]
pub struct SortRatings {
    #[serde(default)]
    pub unrated: usize,
    pub stale_since: Option<String>,
    pub unavailable_reason: Option<UnavailableReason>,
}

/// The `{"error": "…"}` body the server sends with a failure status.
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub error: String,
}
