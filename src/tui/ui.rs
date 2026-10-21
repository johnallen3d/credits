//! Drawing. Every screen becomes a `Page`: lines of text, some of which link
//! to a title or person. j/k move between links and the page scrolls to
//! keep the highlighted one in view.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::api::{
    Credit, Kind, Metadata, Person, PersonCredit, Ratings, SearchHit, State, Title,
    UnavailableReason, WhereToWatch,
};
use super::app::{App, Load, Screen, Target};

const ACCENT: Color = Color::Cyan;
const WARN: Color = Color::Yellow;
const MINE: Color = Color::Green;
const SELECTED: Color = Color::Indexed(237);
const DIM: Style = Style::new().fg(Color::DarkGray);
/// Width the link list is built at when only the links matter.
const LINK_WIDTH: u16 = 80;
const BIO_LINES: usize = 6;
const NAME_COLUMN: usize = 28;
const TITLE_COLUMN: usize = 40;

#[derive(Default)]
pub struct Page {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub links: Vec<(usize, Target)>,
}

impl Page {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    fn line(&mut self, line: impl Into<Line<'static>>) {
        self.lines.push(line.into());
    }

    fn link(&mut self, line: impl Into<Line<'static>>, target: Target) {
        self.links.push((self.lines.len(), target));
        self.lines.push(line.into());
    }

    fn blank(&mut self) {
        self.lines.push(Line::default());
    }

    fn heading(&mut self, text: impl Into<String>) {
        if self.lines.last().is_some_and(|l| l.width() > 0) {
            self.blank();
        }
        self.line(Line::styled(text.into(), Style::new().fg(ACCENT).bold()));
    }

    /// A warning, wrapped under its own leading indent.
    fn warn(&mut self, text: impl Into<String>, width: u16) {
        let text = text.into();
        let body = text.trim_start();
        let indent = &text[..text.len() - body.len()];
        for line in wrap(body, (width as usize).saturating_sub(indent.len())) {
            self.line(Line::styled(
                format!("{indent}{line}"),
                Style::new().fg(WARN),
            ));
        }
    }

    fn para(&mut self, text: &str, width: u16, max_lines: Option<usize>) {
        let mut lines = wrap(text, width as usize);
        if let Some(max) = max_lines.filter(|&max| lines.len() > max) {
            lines.truncate(max);
            if let Some(last) = lines.last_mut() {
                last.push('…');
            }
        }
        for line in lines {
            self.line(line);
        }
    }
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let on_search = app.stack.is_empty();
    let [header, search, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(if on_search { 3 } else { 0 }),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, app, header);
    if on_search {
        draw_search_box(frame, app, search);
    }
    draw_body(frame, app, body);
    draw_footer(frame, app, footer);
}

/// The link the highlighted line points at, if any.
pub fn selected_target(app: &App) -> Option<Target> {
    let page = page(app, LINK_WIDTH);
    page.links.get(app.cursor().selected).map(|(_, t)| *t)
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let mut trail = vec![Span::styled(" credits ", Style::new().reversed().bold())];
    let mut crumbs = vec!["Search".to_owned()];
    crumbs.extend(app.stack.iter().map(screen_name));
    trail.push(Span::raw(" "));
    trail.push(Span::styled(crumbs.join(" › "), DIM));
    let server = Line::styled(format!("{} ", app.server), DIM).right_aligned();
    frame.render_widget(Line::from(trail), area);
    frame.render_widget(server, area);
}

fn draw_search_box(frame: &mut Frame, app: &App, area: Rect) {
    let style = if app.editing {
        Style::new().fg(ACCENT)
    } else {
        DIM
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(style)
        .title(" Search ");
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new(app.search.query.as_str()).block(block), area);
    if app.editing {
        let x = inner.x + (app.search.query.width() as u16).min(inner.width.saturating_sub(1));
        frame.set_cursor_position((x, inner.y));
    }
}

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::new().borders(Borders::ALL).border_style(DIM);
    let inner = block.inner(area);
    let mut page = page(app, inner.width);
    let height = inner.height as usize;
    let highlight = !app.editing;

    let cursor = app.cursor_mut();
    cursor.links = page.links.len();
    cursor.selected = cursor.selected.min(cursor.links.saturating_sub(1));
    match page.links.get(cursor.selected) {
        Some(&(line, _)) => {
            if cursor.selected == 0 && line < height {
                cursor.scroll = 0;
            } else if line < cursor.scroll {
                cursor.scroll = line;
            } else if line >= cursor.scroll + height {
                cursor.scroll = line + 1 - height;
            }
            let marked = std::mem::take(&mut page.lines[line]);
            page.lines[line] = mark(marked, highlight, inner.width);
        }
        None => cursor.scroll = 0,
    }
    let position = match page.links.len() {
        0 => String::new(),
        n => format!(" {}/{n} ", cursor.selected + 1),
    };
    let scroll = cursor.scroll as u16;

    let block = block
        .title(Line::styled(
            format!(" {} ", page.title),
            Style::new().bold(),
        ))
        .title_bottom(Line::styled(position, DIM).right_aligned());
    frame.render_widget(
        Paragraph::new(page.lines).scroll((scroll, 0)).block(block),
        area,
    );
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let hints = if app.editing {
        "type to search · ↓/Enter results · Ctrl-U clear · Ctrl-C quit"
    } else {
        "j/k move · Enter open · Esc back · / search · r reload · q quit"
    };
    frame.render_widget(Line::styled(format!(" {hints}"), DIM), area);
    let busy = match app.stack.last() {
        None if app.search.searching => "Searching… ",
        Some(Screen::Title {
            load: Load::Loading,
            ..
        })
        | Some(Screen::Person {
            load: Load::Loading,
            ..
        }) => "Loading… ",
        _ => "",
    };
    frame.render_widget(
        Line::styled(busy, Style::new().fg(ACCENT)).right_aligned(),
        area,
    );
}

/// Highlights a link line: a marker, and a full-width bar once keys move it.
fn mark(line: Line<'static>, active: bool, width: u16) -> Line<'static> {
    let mut spans = vec![Span::styled("▸", Style::new().fg(ACCENT).bold())];
    // Link lines start with a two-space gutter; the marker takes its first cell.
    let mut rest = line.spans.into_iter();
    if let Some(first) = rest.next() {
        let text = first
            .content
            .strip_prefix(' ')
            .unwrap_or(&first.content)
            .to_owned();
        spans.push(Span::styled(text, first.style));
    }
    spans.extend(rest);
    if !active {
        return Line::from(spans);
    }
    let used: usize = spans.iter().map(|s| s.width()).sum();
    spans.push(Span::raw(" ".repeat((width as usize).saturating_sub(used))));
    for span in &mut spans {
        // Dim text would vanish on the bar, so it brightens a step.
        if span.style.fg == DIM.fg {
            span.style.fg = Some(Color::Gray);
        }
        span.style = span.style.bg(SELECTED);
    }
    Line::from(spans)
}

fn screen_name(screen: &Screen) -> String {
    match screen {
        Screen::Title {
            load: Load::Ready(t),
            ..
        } => t.title.clone(),
        Screen::Person {
            load: Load::Ready(p),
            ..
        } => p.name.clone(),
        Screen::Title {
            target: (kind, _), ..
        } => kind.label().to_owned(),
        Screen::Person { .. } => "Person".to_owned(),
    }
}

fn page(app: &App, width: u16) -> Page {
    match app.stack.last() {
        None => search_page(app),
        Some(screen) => {
            let name = screen_name(screen);
            match screen {
                Screen::Title {
                    load: Load::Ready(t),
                    ..
                } => title_page(t, width),
                Screen::Person {
                    load: Load::Ready(p),
                    ..
                } => person_page(p, width),
                Screen::Title { load, .. } => status_page(name, load_message(load), width),
                Screen::Person { load, .. } => status_page(name, load_message(load), width),
            }
        }
    }
}

fn load_message<T>(load: &Load<T>) -> Option<&str> {
    match load {
        Load::Failed(message) => Some(message),
        _ => None,
    }
}

fn status_page(title: String, failure: Option<&str>, width: u16) -> Page {
    let mut page = Page::new(title);
    match failure {
        Some(message) => {
            page.warn(message, width);
            page.line(Line::styled("Press r to try again or Esc to go back.", DIM));
        }
        None => page.line(Line::styled("Loading…", DIM)),
    }
    page
}

fn search_page(app: &App) -> Page {
    let search = &app.search;
    let mut page = Page::new("Results");
    if let Some(error) = &search.error {
        page.line(Line::styled(error.clone(), Style::new().fg(WARN)));
        page.line(Line::styled("Press Enter to try again.", DIM));
        return page;
    }
    if search.results.is_empty() {
        let text = if search.query.trim().is_empty() {
            "Type to search movies, TV shows and people.".to_owned()
        } else if search.searching || search.answered.is_empty() {
            "Searching…".to_owned()
        } else {
            format!("No results for “{}”.", search.answered)
        };
        page.line(Line::styled(text, DIM));
        return page;
    }
    for hit in &search.results {
        let (kind, target, name, detail) = match hit {
            SearchHit::Movie(t) | SearchHit::Tv(t) => {
                let kind = if matches!(hit, SearchHit::Movie(_)) {
                    Kind::Movie
                } else {
                    Kind::Tv
                };
                let name = match &t.year {
                    Some(year) => format!("{} ({year})", t.title),
                    None => t.title.clone(),
                };
                (kind.label(), Target::Title(kind, t.id), name, String::new())
            }
            SearchHit::Person(p) => {
                let titles: Vec<&str> = p.known_for.iter().map(|k| k.title.as_str()).collect();
                let mut detail = p.known_for_department.clone().unwrap_or_default();
                if !titles.is_empty() {
                    if !detail.is_empty() {
                        detail.push_str(" · ");
                    }
                    detail.push_str(&titles.join(", "));
                }
                ("Person", Target::Person(p.id), p.name.clone(), detail)
            }
        };
        page.link(
            Line::from(vec![
                Span::styled(format!("  {kind:<7}"), Style::new().fg(ACCENT)),
                Span::raw(name).bold(),
                Span::styled(
                    if detail.is_empty() {
                        detail
                    } else {
                        format!("  {detail}")
                    },
                    DIM,
                ),
            ]),
            target,
        );
    }
    page
}

fn title_page(t: &Title, width: u16) -> Page {
    let mut page = Page::new(t.kind.label());
    page.line(Line::raw(t.title.clone()).bold());

    let mut facts = vec![t.kind.label().to_owned()];
    facts.extend(t.year.clone());
    if let Some(minutes) = t.runtime_minutes.filter(|&m| m > 0) {
        facts.push(runtime(minutes));
    }
    if let Some(seasons) = t.seasons {
        facts.push(plural(seasons, "season"));
    }
    if let Some(episodes) = t.episodes {
        facts.push(plural(episodes, "episode"));
    }
    if !t.genres.is_empty() {
        facts.push(t.genres.join(", "));
    }
    if let Some(status) = t.status.as_ref().filter(|s| *s != "Released") {
        facts.push(status.clone());
    }
    page.line(Line::styled(facts.join(" · "), DIM));
    if let Some(tagline) = t.tagline.as_ref().filter(|s| !s.is_empty()) {
        page.line(Line::styled(tagline.clone(), DIM.italic()));
    }
    stale_notice(&mut page, &t.metadata, width);

    ratings(&mut page, &t.ratings, width);
    if let Some(overview) = t.overview.as_ref().filter(|s| !s.is_empty()) {
        page.blank();
        page.para(overview, width, None);
    }
    where_to_watch(&mut page, &t.where_to_watch, width);

    let crew: Vec<(&str, &Credit)> = t
        .key_crew
        .iter()
        .flat_map(|k| k.people.iter().map(|p| (k.role.label(), p)))
        .collect();
    if !crew.is_empty() {
        page.heading("Key crew");
        let mut previous = "";
        for (role, person) in crew {
            let label = if role == previous { "" } else { role };
            previous = role;
            page.link(
                Line::from(vec![
                    Span::styled(format!("  {label:<13}"), DIM),
                    Span::raw(person.name.clone()),
                ]),
                Target::Person(person.person_id),
            );
        }
    }

    if !t.cast.is_empty() {
        let count = if t.cast_total > t.cast.len() {
            format!("Cast ({} of {})", t.cast.len(), t.cast_total)
        } else {
            "Cast".to_owned()
        };
        page.heading(count);
        let column = column_width(t.cast.iter().map(|c| c.name.as_str()), NAME_COLUMN);
        for credit in &t.cast {
            let mut role = credit.role.clone().unwrap_or_default();
            if let Some(n) = credit.episode_count {
                role = join_dot(&role, &plural(n, "ep"));
            }
            page.link(
                Line::from(vec![
                    Span::raw(format!("  {}  ", pad(&credit.name, column))),
                    Span::styled(role, DIM),
                ]),
                Target::Person(credit.person_id),
            );
        }
    }
    page
}

fn person_page(p: &Person, width: u16) -> Page {
    let mut page = Page::new("Person");
    page.line(Line::raw(p.name.clone()).bold());
    let mut facts: Vec<String> = p.known_for.iter().cloned().collect();
    if let Some(born) = &p.birthday {
        facts.push(format!("Born {}", day(born)));
    }
    if let Some(died) = &p.deathday {
        facts.push(format!("Died {}", day(died)));
    }
    facts.extend(p.place_of_birth.clone());
    if !facts.is_empty() {
        page.line(Line::styled(facts.join(" · "), DIM));
    }
    stale_notice(&mut page, &p.metadata, width);
    if let Some(bio) = p.biography.as_ref().filter(|s| !s.is_empty()) {
        page.blank();
        let first = bio.split("\n\n").next().unwrap_or(bio);
        page.para(first, width, Some(BIO_LINES));
    }

    if !p.known_for_titles.is_empty() {
        page.heading("Known for");
        credit_rows(&mut page, &p.known_for_titles);
    }
    for group in &p.filmography {
        page.heading(format!("{} ({})", group.department, group.entries.len()));
        credit_rows(&mut page, &group.entries);
    }
    if p.known_for_titles.is_empty() && p.filmography.is_empty() {
        page.blank();
        page.line(Line::styled("No credits listed on TMDB.", DIM));
    }
    page
}

fn credit_rows(page: &mut Page, credits: &[PersonCredit]) {
    let column = column_width(credits.iter().map(|c| c.title.as_str()), TITLE_COLUMN);
    for c in credits {
        let year = c.year.as_deref().unwrap_or("");
        let mut role = c.role.clone().unwrap_or_default();
        if let Some(n) = c.episode_count {
            role = join_dot(&role, &plural(n, "ep"));
        }
        page.link(
            Line::from(vec![
                Span::styled(format!("  {year:<4}  "), DIM),
                Span::styled(format!("{:<5} ", c.kind.label()), Style::new().fg(ACCENT)),
                Span::raw(format!("{}  ", pad(&c.title, column))),
                Span::styled(role, DIM),
            ]),
            Target::Title(c.kind, c.id),
        );
    }
}

fn stale_notice(page: &mut Page, metadata: &Metadata, width: u16) {
    if metadata.state == State::Stale {
        let saved = fetched_day(&metadata.fetched_at)
            .map(|d| format!(" saved on {d}"))
            .unwrap_or_default();
        page.blank();
        page.warn(
            format!(
                "TMDB isn't responding, so this shows details{saved}. They may be out of date."
            ),
            width,
        );
    }
}

fn ratings(page: &mut Page, r: &Ratings, width: u16) {
    page.heading("Ratings");
    if r.state == State::Unavailable {
        let message = match r.unavailable_reason {
            Some(UnavailableReason::NotConfigured) => "Ratings aren't configured on the server.",
            Some(UnavailableReason::NoImdbId) => "No IMDb entry is linked to this title.",
            Some(UnavailableReason::ProviderError) | None => {
                "Ratings couldn't be loaded right now."
            }
        };
        page.line(Line::styled(format!("  {message}"), DIM));
        return;
    }
    match &r.preferred {
        Some(p) => {
            let mut spans = vec![
                Span::raw(format!("  {:<8}", p.display())).bold(),
                Span::raw(p.source.label()),
            ];
            if let Some(votes) = p.votes {
                spans.push(Span::styled(format!(" ({} votes)", thousands(votes)), DIM));
            }
            page.line(Line::from(spans));
        }
        None => page.line(Line::styled(
            format!("  No {} score", r.preferred_source.label()),
            DIM,
        )),
    }
    for other in &r.others {
        page.line(Line::styled(
            format!("  {:<8}{}", other.display(), other.source.label()),
            DIM,
        ));
    }
    if r.state == State::Stale {
        let day = fetched_day(&r.fetched_at)
            .map(|d| format!("; showing scores from {d}"))
            .unwrap_or_default();
        page.warn(format!("  Couldn't refresh ratings{day}."), width);
    }
}

fn where_to_watch(page: &mut Page, w: &WhereToWatch, width: u16) {
    page.heading(format!("Where to watch in {}", w.region));
    if w.offers.is_empty() {
        if w.state == State::Unavailable {
            page.warn(
                "  Availability unknown: where to watch couldn't be loaded right now.",
                width,
            );
        } else {
            page.line(Line::styled(
                format!(
                    "  No streaming, rental or purchase options are listed for {}.",
                    w.region
                ),
                DIM,
            ));
        }
        return;
    }
    const LABEL: usize = 15;
    for group in &w.offers {
        // Wrap provider names under their label, keeping each name whole.
        let room = (width as usize).saturating_sub(LABEL + 2).max(10);
        let mut rows: Vec<Vec<Span<'static>>> = vec![Vec::new()];
        let mut used = 0;
        for (i, provider) in group.providers.iter().enumerate() {
            let sep = if i + 1 < group.providers.len() {
                ", "
            } else {
                ""
            };
            let name = if provider.mine {
                format!("★ {}", provider.name)
            } else {
                provider.name.clone()
            };
            let cell = name.width() + sep.width();
            if used > 0 && used + cell > room {
                rows.push(Vec::new());
                used = 0;
            }
            used += cell;
            let style = if provider.mine {
                Style::new().fg(MINE).bold()
            } else {
                Style::new()
            };
            let row = rows.last_mut().expect("rows starts non-empty");
            row.push(Span::styled(name, style));
            row.push(Span::raw(sep));
        }
        for (i, row) in rows.into_iter().enumerate() {
            let label = if i == 0 { group.kind.label() } else { "" };
            let mut spans = vec![Span::styled(format!("  {label:<LABEL$}"), DIM)];
            spans.extend(row);
            page.line(Line::from(spans));
        }
    }
    if w.state == State::Stale {
        let day = fetched_day(&w.fetched_at)
            .map(|d| format!("; showing what was listed on {d}"))
            .unwrap_or_default();
        page.warn(format!("  Couldn't refresh where to watch{day}."), width);
    }
}

/// Word-wraps to `width` columns, keeping paragraph breaks.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

fn column_width<'a>(names: impl Iterator<Item = &'a str>, cap: usize) -> usize {
    names.map(|n| n.width()).max().unwrap_or(0).min(cap)
}

/// Pads or truncates to exactly `width` columns.
fn pad(text: &str, width: usize) -> String {
    if text.width() <= width {
        return format!("{text}{}", " ".repeat(width - text.width()));
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    let fill = width.saturating_sub(out.width());
    out + &" ".repeat(fill)
}

fn join_dot(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_owned()
    } else {
        format!("{a} · {b}")
    }
}

fn plural(n: u32, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn runtime(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "1943-02-05" as "Feb 5, 1943".
fn day(date: &str) -> String {
    crate::model::display_date(date)
}

/// A fetch timestamp as the local calendar day, like the web pages.
fn fetched_day(at: &Option<String>) -> Option<String> {
    let t: jiff::Timestamp = at.as_ref()?.parse().ok()?;
    Some(
        t.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%b %-d, %Y")
            .to_string(),
    )
}
