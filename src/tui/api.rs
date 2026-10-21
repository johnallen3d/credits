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

/// The `{"error": "…"}` body the server sends with a failure status.
#[derive(Debug, Deserialize)]
pub struct ApiError {
    pub error: String,
}
