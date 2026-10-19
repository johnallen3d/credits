//! Presentation-independent types shared by the HTML pages and the JSON API.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
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
}

/// One search result. Serialized with a `kind` of "movie", "tv" or "person"
/// alongside the variant's fields.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchHit {
    Movie(TitleHit),
    Tv(TitleHit),
    Person(PersonHit),
}

#[derive(Clone, Debug, Serialize)]
pub struct TitleHit {
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PersonHit {
    pub id: u64,
    pub name: String,
    pub known_for_department: Option<String>,
    /// The few titles TMDB associates with the person, most notable first.
    pub known_for: Vec<KnownFor>,
    pub profile_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KnownFor {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
}

impl SearchHit {
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

/// A timestamp as a local calendar day, e.g. "Oct 19, 2026".
fn local_day(t: Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%b %-d, %Y")
        .to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataState {
    /// Within the freshness window.
    Fresh,
    /// An expired cached copy, served because TMDB could not be reached.
    Stale,
}

/// How current the TMDB data behind a response is.
#[derive(Clone, Copy, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
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
    /// TV only.
    pub seasons: Option<u32>,
    /// TV only.
    pub episodes: Option<u32>,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub tmdb_url: String,
    pub ratings: Ratings,
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

    pub fn has_more_credits(&self) -> bool {
        self.cast_total > self.cast.len() || self.crew_total > 0
    }
}

/// A person's part in one title: a character for cast, job titles for crew.
#[derive(Clone, Debug, Serialize)]
pub struct PersonCredit {
    pub person_id: u64,
    pub name: String,
    pub role: Option<String>,
    /// TV only: episodes this person is credited on.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct KeyCrew {
    pub role: KeyRole,
    pub people: Vec<PersonCredit>,
}

/// Everyone credited on a title, crew grouped by department.
#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
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
    format!("dept-{}", slug.join("-"))
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct SeasonDetail {
    pub show_id: u64,
    pub show_title: String,
    pub number: u32,
    pub name: String,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Vec<Episode>,
    /// Neighbouring seasons in the show's season order, for navigation.
    pub previous: Option<SeasonSummary>,
    pub next: Option<SeasonSummary>,
    pub metadata: Metadata,
}

impl SeasonDetail {
    pub fn show_path(&self) -> String {
        title_path(MediaKind::Tv, self.show_id)
    }

    pub fn season_path(&self, season: &SeasonSummary) -> String {
        format!("/tv/{}/season/{}", self.show_id, season.number)
    }

    pub fn air_date_display(&self) -> Option<String> {
        self.air_date.as_deref().map(display_date)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Episode {
    pub number: u32,
    /// TMDB's name, or "Episode N" when it has none.
    pub title: String,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<u32>,
    pub still_url: Option<String>,
}

impl Episode {
    pub fn air_date_display(&self) -> Option<String> {
        self.air_date.as_deref().map(display_date)
    }
}

#[derive(Clone, Debug, Serialize)]
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
    /// Grouped by department, the person's main department first.
    pub filmography: Vec<FilmographyGroup>,
    pub metadata: Metadata,
}

impl PersonDetail {
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

#[derive(Clone, Debug, Serialize)]
pub struct FilmographyGroup {
    pub department: String,
    /// Newest first; undated (usually unreleased) entries lead.
    pub entries: Vec<FilmographyEntry>,
}

impl FilmographyGroup {
    pub fn anchor(&self) -> String {
        anchor(&self.department)
    }

    pub fn is_acting(&self) -> bool {
        self.department == "Acting"
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FilmographyEntry {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    /// Characters for acting credits, job titles otherwise.
    pub role: Option<String>,
    pub episode_count: Option<u32>,
    pub poster_url: Option<String>,
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
/// scores across sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RatingsState {
    /// Within the freshness window.
    Fresh,
    /// Cached data served because the provider could not be reached.
    Stale,
    /// No data at all; see `unavailable_reason`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason {
    /// The server has no ratings provider key.
    NotConfigured,
    /// TMDB has no IMDb identifier for the title, so there is nothing safe to look up.
    NoImdbId,
    /// The provider failed (timeout, quota, outage) and nothing was cached.
    ProviderError,
}

#[derive(Clone, Debug, Serialize)]
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
