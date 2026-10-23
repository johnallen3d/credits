//! Context-aware key reference. Inactive bindings remain visible but dimmed.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::api::Kind;
use super::app::{App, Content, ListName, Load, Target};
use super::theme::palette;

struct Binding {
    group: &'static str,
    keys: &'static str,
    action: &'static str,
    active: bool,
}

fn bindings(app: &App) -> Vec<Binding> {
    let browsing = !app.editing;
    let search = app.stack.is_empty();
    let screen = app.stack.last();
    let target = screen.map(|s| s.target);
    let ready = screen.and_then(|s| match &s.load {
        Load::Ready(content) => Some(content),
        _ => None,
    });
    let title = app.title_target().is_some();
    let whole_page = matches!(
        ready,
        Some(Content::Title(_) | Content::Season(_) | Content::Episode(_))
    );
    let watched = whole_page || (matches!(ready, Some(Content::List(_))) && title);
    let season = matches!(ready, Some(Content::Season(_)));
    let episode = matches!(ready, Some(Content::Episode(_)));
    let show = matches!(target, Some(Target::Title(Kind::Tv, _)));
    let previous = matches!(ready, Some(Content::Episode(e)) if e.previous.is_some());
    let next = matches!(ready, Some(Content::Episode(e)) if e.next.is_some());
    let mut rows = Vec::new();
    let mut add = |group, keys, action, active| {
        rows.push(Binding {
            group,
            keys,
            action,
            active,
        })
    };
    add("Navigation", "? / Esc", "Close help", true);
    add(
        "Navigation",
        "j/k / ↑/↓",
        "Move between links (scroll this help while open)",
        browsing,
    );
    add(
        "Navigation",
        "Ctrl-D/U / PgDn/PgUp",
        "Move ten links (scroll help while open)",
        browsing,
    );
    add(
        "Navigation",
        "g/G / Home/End",
        "First/last link (top/bottom of help while open)",
        browsing,
    );
    add(
        "Navigation",
        "Enter / l / →",
        "Open highlighted link; Enter retries a failed search or recent list",
        browsing,
    );
    add(
        "Navigation",
        "Esc / Backspace / h / ←",
        "Back; at search results, return to the box",
        browsing,
    );
    add(
        "Navigation",
        "r",
        "Reload the current page or search",
        browsing,
    );
    add("Navigation", "q", "Quit (after closing help)", browsing);
    add(
        "Navigation",
        "Ctrl-C",
        "Quit, including from search or help",
        true,
    );
    add(
        "Navigation",
        "Ctrl-D",
        "Quit from the search box",
        app.editing,
    );
    add(
        "Search",
        "/",
        "Return to the search box, keeping the query",
        browsing,
    );
    add(
        "Search",
        "Tab / Shift-Tab",
        "Switch between search box and results, keeping selection",
        search,
    );
    add(
        "Search",
        "Type / Backspace",
        "Edit the search query",
        app.editing,
    );
    add("Search", "Ctrl-U", "Clear the search query", app.editing);
    add(
        "Search",
        "↓ / Enter",
        "Leave the box / open results; Enter submits pending search or retries",
        app.editing,
    );
    add("Search", "Esc", "Leave the search box", app.editing);
    add(
        "Lists",
        "w / f / v",
        "Open To watch / Favorites / Watched",
        browsing,
    );
    add(
        "Lists",
        "n / u",
        "Open Now playing / Coming soon movies",
        browsing,
    );
    add(
        "Lists",
        "t",
        "Cycle All / Movies / TV (lists and filmographies)",
        browsing && app.has_view(),
    );
    add(
        "Lists",
        "o",
        "Sort by Added / Rating (Watched: Last watched / Title; people: Date / Rating)",
        browsing && app.has_view(),
    );
    add(
        "Lists",
        "a",
        "Cycle marked Any way / By hand / Apple TV (Watched)",
        browsing && app.on_list(ListName::Watched),
    );
    add(
        "Lists",
        "m",
        "Toggle On my services (To watch with chosen services)",
        browsing && app.can_filter_services(),
    );
    add(
        "Toggles",
        "b / F",
        "Toggle To watch / Favorite for the title or list row",
        browsing && title,
    );
    add(
        "Toggles",
        "x",
        "Toggle watched: highlighted season, episode or list row; otherwise this title/episode",
        browsing && watched,
    );
    add(
        "Toggles",
        "X",
        "Toggle watched for the whole show / season (same as x on movies and episodes)",
        browsing && whole_page,
    );
    add(
        "Titles",
        "c",
        "Full cast and crew of this movie or TV show",
        browsing && app.credits_target().is_some(),
    );
    add(
        "Episodes",
        "s",
        "Peek/hide spoilers for the highlighted or open episode",
        browsing && (season || episode),
    );
    add(
        "Episodes",
        "[ / ]",
        "Previous / next episode, or page of Now playing / Coming soon (when available)",
        browsing && (previous || next || app.can_turn_page()),
    );
    // The labels describe selection-sensitive watched behavior without implying
    // that a TV cast link can be marked instead of the show itself.
    if show && browsing {
        if let Some(row) = rows.iter_mut().find(|r| r.keys == "x") {
            row.action = "Toggle watched for the highlighted season, otherwise the whole show";
        }
    }
    rows
}

pub(super) fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let width = area.width.min(94);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + u16::from(area.height > 4),
        width,
        area.height
            .saturating_sub(if area.height > 4 { 2 } else { 0 }),
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(palette().accent))
        .title(" Keyboard help · ? / Esc close ");
    let inner = block.inner(popup);
    let mut lines = vec![Line::styled(
        "Dimmed keys do not apply to the underlying screen.",
        Style::new().fg(palette().dim),
    )];
    let mut group = "";
    for row in bindings(app) {
        if group != row.group {
            group = row.group;
            lines.push(Line::default());
            lines.push(Line::styled(
                group,
                Style::new().fg(palette().accent).bold(),
            ));
        }
        let style = if row.active {
            Style::new()
        } else {
            Style::new().fg(palette().dim)
        };
        // Wrap before scrolling, so even a small terminal can reach every key.
        let text = format!("  {}  —  {}", row.keys, row.action);
        for line in super::ui::wrap(&text, inner.width as usize) {
            lines.push(Line::from(Span::styled(line, style)));
        }
    }
    let help = app.help.as_mut().expect("help is open");
    help.max_scroll = lines.len().saturating_sub(inner.height as usize);
    help.scroll = help.scroll.min(help.max_scroll);
    let block =
        block.title_bottom(Line::from(" ↑/↓ scroll · PgUp/PgDn · Home/End ").right_aligned());
    let mut style = Style::new();
    if let Some(fg) = palette().fg {
        style = style.fg(fg);
    }
    if let Some(bg) = palette().bg {
        style = style.bg(bg);
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .style(style)
            .scroll((help.scroll as u16, 0))
            .block(block),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::super::app::ListView;
    use super::*;

    fn active(app: &App, keys: &str) -> bool {
        bindings(app)
            .iter()
            .find(|r| r.keys == keys)
            .unwrap()
            .active
    }

    #[test]
    fn bindings_follow_search_focus_and_page_context() {
        let mut app = App::new("http://localhost");
        assert!(active(&app, "Ctrl-U"));
        assert!(!active(&app, "w / f / v"));
        assert!(!active(&app, "b / F"));
        app.editing = false;
        assert!(!active(&app, "Ctrl-U"));
        assert!(active(&app, "w / f / v"));
        app.open(Target::List(ListName::Watchlist, ListView::default()));
        assert!(active(&app, "t"));
        assert!(active(&app, "o"));
        assert!(!active(&app, "m"));
        assert!(
            !active(&app, "a"),
            "only Watched filters by how it was marked"
        );
        assert!(!active(&app, "Tab / Shift-Tab"));
        app.stack.last_mut().unwrap().target = Target::List(
            ListName::Watchlist,
            ListView {
                mine: true,
                ..ListView::default()
            },
        );
        assert!(
            active(&app, "m"),
            "an enabled filter can always be disabled"
        );
        assert!(!active(&app, "c"), "lists have no credits");
        app.open(Target::Title(Kind::Movie, 603));
        assert!(active(&app, "c"));
        app.open(Target::Credits(Kind::Movie, 603));
        assert!(!active(&app, "c"), "already on the credits");
        app.open(Target::Person(6384, ListView::default()));
        assert!(active(&app, "t"), "filmographies filter by kind");
        assert!(active(&app, "o"), "filmographies sort by rating");
        assert!(!active(&app, "m"));
        assert!(!active(&app, "s"));
    }
}
