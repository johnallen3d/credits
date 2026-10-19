//! Presentation-independent types shared by the HTML pages and the JSON API.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub kind: MediaKind,
    pub id: u64,
    pub title: String,
    pub year: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
}

impl SearchHit {
    pub fn detail_path(&self) -> String {
        format!("/{}/{}", self.kind.slug(), self.id)
    }
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
        self.fetched_at.map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%b %-d, %Y")
                .to_string()
        })
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
