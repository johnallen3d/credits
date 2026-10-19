//! Presentation-independent types shared by the HTML pages and the JSON API.

use serde::Serialize;

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
}
