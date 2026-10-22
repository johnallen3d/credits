//! Color themes, picked by `theme = "…"` in the config file. Set once at
//! startup; drawing reads the palette from anywhere.

use std::sync::OnceLock;

use ratatui::style::Color;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    #[default]
    RosePine,
    TokyoNight,
    Catppuccin,
    /// The terminal's own colors.
    Classic,
}

pub struct Palette {
    /// Headings, kinds and the highlight marker.
    pub accent: Color,
    pub warn: Color,
    /// Watched marks and your services.
    pub mine: Color,
    /// The highlighted line's bar.
    pub selected: Color,
    pub dim: Color,
    /// Dim text on the highlight bar, a step brighter so it stays legible.
    pub dim_selected: Color,
    /// Score rings, as on the web: Rotten Tomatoes fresh and rotten, IMDb.
    pub rt_fresh: Color,
    pub rt_rotten: Color,
    pub imdb: Color,
    /// Painted behind everything; `None` keeps the terminal's.
    pub fg: Option<Color>,
    pub bg: Option<Color>,
}

impl Theme {
    pub fn palette(self) -> Palette {
        match self {
            Theme::RosePine => Palette {
                accent: Color::from_u32(0xc4a7e7),
                warn: Color::from_u32(0xf6c177),
                mine: Color::from_u32(0x9ccfd8),
                selected: Color::from_u32(0x403d52),
                dim: Color::from_u32(0x6e6a86),
                dim_selected: Color::from_u32(0x908caa),
                rt_fresh: Color::from_u32(0xfa320a),
                rt_rotten: Color::from_u32(0x0ac855),
                imdb: Color::from_u32(0xf5c518),
                fg: Some(Color::from_u32(0xe0def4)),
                bg: Some(Color::from_u32(0x191724)),
            },
            Theme::TokyoNight => Palette {
                accent: Color::from_u32(0x7aa2f7),
                warn: Color::from_u32(0xe0af68),
                mine: Color::from_u32(0x9ece6a),
                selected: Color::from_u32(0x283457),
                dim: Color::from_u32(0x565f89),
                dim_selected: Color::from_u32(0xa9b1d6),
                rt_fresh: Color::from_u32(0xfa320a),
                rt_rotten: Color::from_u32(0x0ac855),
                imdb: Color::from_u32(0xf5c518),
                fg: Some(Color::from_u32(0xc0caf5)),
                bg: Some(Color::from_u32(0x1a1b26)),
            },
            Theme::Catppuccin => Palette {
                accent: Color::from_u32(0xcba6f7),
                warn: Color::from_u32(0xf9e2af),
                mine: Color::from_u32(0xa6e3a1),
                selected: Color::from_u32(0x45475a),
                dim: Color::from_u32(0x7f849c),
                dim_selected: Color::from_u32(0xbac2de),
                rt_fresh: Color::from_u32(0xfa320a),
                rt_rotten: Color::from_u32(0x0ac855),
                imdb: Color::from_u32(0xf5c518),
                fg: Some(Color::from_u32(0xcdd6f4)),
                bg: Some(Color::from_u32(0x1e1e2e)),
            },
            Theme::Classic => Palette {
                accent: Color::Cyan,
                warn: Color::Yellow,
                mine: Color::Green,
                selected: Color::Indexed(237),
                dim: Color::DarkGray,
                dim_selected: Color::Gray,
                rt_fresh: Color::Red,
                rt_rotten: Color::Green,
                imdb: Color::Yellow,
                fg: None,
                bg: None,
            },
        }
    }
}

static PALETTE: OnceLock<Palette> = OnceLock::new();

/// Chooses the theme; only the first call counts.
pub fn set(theme: Theme) {
    let _ = PALETTE.set(theme.palette());
}

/// The chosen theme's palette, or the default theme's if none was set.
pub fn palette() -> &'static Palette {
    PALETTE.get_or_init(|| Theme::default().palette())
}
