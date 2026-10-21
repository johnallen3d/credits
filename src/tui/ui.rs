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
    Credit, Episode, EpisodeRatings, KeyCrew, Kind, Metadata, Person, PersonCredit, Rating,
    Ratings, SearchHit, Season, State, Title, UnavailableReason, Watched, WhereToWatch,
};
use super::app::{App, Content, Load, Screen, Target};

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
const EPISODE_COLUMN: usize = 36;
/// Stands in for an unwatched episode's score.
const HIDDEN_SCORE: &str = "▒▒▒▒▒▒";

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
    let server = Line::styled(format!("{} ", app.server), DIM).right_aligned();
    // Deep trails lose their oldest crumbs rather than run into the server.
    let room = (area.width as usize).saturating_sub(trail[0].width() + 1 + server.width() + 1);
    let mut text = crumbs.join(" › ");
    while text.width() > room && crumbs.len() > 1 {
        crumbs.remove(0);
        text = format!("… › {}", crumbs.join(" › "));
    }
    trail.push(Span::styled(pad(&text, room.min(text.width())), DIM));
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
    let hints = match app.stack.last().map(|s| s.target) {
        _ if app.editing => "type to search · ↓/Enter results · Ctrl-U clear · Ctrl-C quit",
        Some(Target::Season(..)) => {
            "j/k move · Enter open · s peek · Esc back · / search · r reload · q quit"
        }
        Some(Target::Episode(..)) => {
            "j/k move · Enter open · s peek · [/] prev/next · Esc back · / search · r reload · q quit"
        }
        _ => "j/k move · Enter open · Esc back · / search · r reload · q quit",
    };
    frame.render_widget(Line::styled(format!(" {hints}"), DIM), area);
    let busy = match app.stack.last() {
        None if app.search.searching => "Searching… ",
        Some(Screen {
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
    match (&screen.load, screen.target) {
        (Load::Ready(Content::Title(t)), _) => t.title.clone(),
        (Load::Ready(Content::Person(p)), _) => p.name.clone(),
        (Load::Ready(Content::Season(s)), _) => s.name.clone(),
        (Load::Ready(Content::Episode(e)), _) => format!("{}. {}", e.number, e.title),
        (_, Target::Title(kind, _)) => kind.label().to_owned(),
        (_, Target::Person(_)) => "Person".to_owned(),
        (_, Target::Season(_, n)) => season_label(n),
        (_, Target::Episode(_, _, n)) => format!("Episode {n}"),
    }
}

fn page(app: &App, width: u16) -> Page {
    let Some(screen) = app.stack.last() else {
        return search_page(app);
    };
    match &screen.load {
        Load::Ready(Content::Title(t)) => title_page(t, width),
        Load::Ready(Content::Person(p)) => person_page(p, width),
        Load::Ready(Content::Season(s)) => season_page(s, screen, width),
        Load::Ready(Content::Episode(e)) => episode_page(e, screen, width),
        Load::Failed(message) => status_page(screen_name(screen), Some(message), width),
        Load::Loading => status_page(screen_name(screen), None, width),
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
    if t.kind == Kind::Tv {
        season_rows(&mut page, t, width);
    }
    key_crew(&mut page, &t.key_crew);

    if !t.cast.is_empty() {
        let count = if t.cast_total > t.cast.len() {
            format!("Cast ({} of {})", t.cast.len(), t.cast_total)
        } else {
            "Cast".to_owned()
        };
        page.heading(count);
        cast_rows(&mut page, &t.cast);
    }
    page
}

fn season_rows(page: &mut Page, t: &Title, width: u16) {
    if t.season_list.is_empty() {
        return;
    }
    page.heading("Seasons");
    let watched = t
        .watched
        .as_ref()
        .filter(|_| Watched::readable(t.watched.as_ref()));
    if watched.is_none() {
        page.warn("  Watched marks couldn't be read.", width);
    }
    let column = column_width(t.season_list.iter().map(|s| s.name.as_str()), NAME_COLUMN);
    for s in &t.season_list {
        let episodes = s
            .episode_count
            .map(|n| plural(n, "episode"))
            .unwrap_or_default();
        let mut spans = vec![
            Span::styled(format!("  {:>2}  ", s.number), DIM),
            Span::raw(format!("{}  ", pad(&s.name, column))),
            Span::styled(
                format!("{episodes:<13}{:<6}", s.year.as_deref().unwrap_or("")),
                DIM,
            ),
        ];
        if watched.is_some_and(|w| w.season(s.number)) {
            spans.push(Span::styled("✓ Watched", Style::new().fg(MINE)));
        }
        page.link(Line::from(spans), Target::Season(t.id, s.number));
    }
}

fn season_page(s: &Season, screen: &Screen, width: u16) -> Page {
    let mut page = Page::new("Season");
    page.line(Line::raw(format!("{} · {}", s.show_title, s.name)).bold());
    let watched = s
        .watched
        .as_ref()
        .filter(|_| Watched::readable(s.watched.as_ref()));
    let mut facts = vec!["TV season".to_owned()];
    facts.extend(s.air_date.as_deref().map(day));
    facts.push(plural(s.episodes.len() as u32, "episode"));
    if watched.is_some_and(|w| w.season(s.number)) {
        facts.push("✓ Watched".to_owned());
    }
    page.line(Line::styled(facts.join(" · "), DIM));
    stale_notice(&mut page, &s.metadata, width);
    if let Some(overview) = s.overview.as_ref().filter(|o| !o.is_empty()) {
        page.blank();
        page.para(overview, width, None);
    }

    page.heading("Episodes");
    if s.episodes.is_empty() {
        page.line(Line::styled(
            "  TMDB has no episodes listed for this season yet.",
            DIM,
        ));
        return page;
    }
    if watched.is_none() {
        page.warn(
            "  Watched marks couldn't be read, so every episode's score stays hidden.",
            width,
        );
    } else if s
        .episodes
        .iter()
        .any(|e| e.rating.is_some() && !watched_episode(watched, s.number, e.number))
    {
        page.line(Line::styled(
            "  Scores of unwatched episodes are hidden; s peeks the highlighted one.",
            DIM,
        ));
    }
    episode_ratings_notes(&mut page, &s.episode_ratings, "episode ratings", width);

    let column = column_width(s.episodes.iter().map(|e| e.title.as_str()), EPISODE_COLUMN);
    for e in &s.episodes {
        let seen = watched_episode(watched, s.number, e.number);
        let shown = seen || screen.peeking(s.number, e.number);
        let aired = e
            .air_date
            .as_deref()
            .map(day)
            .unwrap_or_else(|| "Not announced".to_owned());
        let minutes = e.runtime_minutes.map(runtime).unwrap_or_default();
        let mut spans = vec![
            Span::styled(format!("  {:>2}. ", e.number), DIM),
            Span::raw(format!("{}  ", pad(&e.title, column))),
            Span::styled(format!("{aired:<14}{minutes:<7} "), DIM),
        ];
        spans.push(match &e.rating {
            Some(r) if shown => Span::raw(format!("{:<12}", score(r))),
            Some(_) => Span::styled(format!("{:<12}", format!("IMDb {HIDDEN_SCORE}")), DIM),
            None => Span::raw(" ".repeat(12)),
        });
        if seen {
            let mark = if watched.is_some_and(|w| w.from_apple_tv(s.number, e.number)) {
                "✓ Apple TV"
            } else {
                "✓"
            };
            spans.push(Span::styled(mark, Style::new().fg(MINE)));
        } else if shown {
            spans.push(Span::styled("peeking", Style::new().fg(WARN)));
        }
        page.link(
            Line::from(spans),
            Target::Episode(s.show_id, s.number, e.number),
        );
    }
    page
}

fn episode_page(e: &Episode, screen: &Screen, width: u16) -> Page {
    let mut page = Page::new("Episode");
    page.line(Line::raw(e.title.clone()).bold());
    page.line(Line::styled(
        format!(
            "{} · {} · Episode {}",
            e.show_title, e.season_name, e.number
        ),
        DIM,
    ));
    let watched = e
        .watched
        .as_ref()
        .filter(|_| Watched::readable(e.watched.as_ref()));
    let seen = watched_episode(watched, e.season_number, e.number);
    let shown = seen || screen.peeking(e.season_number, e.number);
    let mut facts = vec![
        e.air_date
            .as_deref()
            .map(day)
            .unwrap_or_else(|| "Air date not announced".to_owned()),
    ];
    if let Some(minutes) = e.runtime_minutes.filter(|&m| m > 0) {
        facts.push(runtime(minutes));
    }
    if let Some(w) = watched.filter(|_| seen) {
        facts.push(if w.from_apple_tv(e.season_number, e.number) {
            "✓ Watched on Apple TV".to_owned()
        } else if w.season(e.season_number) {
            "✓ Watched with the season".to_owned()
        } else {
            "✓ Watched".to_owned()
        });
    }
    page.line(Line::styled(facts.join(" · "), DIM));
    let mut neighbours = Vec::new();
    if let Some(previous) = e.previous {
        neighbours.push(format!("[ Episode {previous}"));
    }
    if let Some(next) = e.next {
        neighbours.push(format!("] Episode {next}"));
    }
    if !neighbours.is_empty() {
        page.line(Line::styled(neighbours.join("   "), DIM));
    }
    stale_notice(&mut page, &e.metadata, width);
    if watched.is_none() {
        page.blank();
        page.warn(
            "Watched marks couldn't be read, so this episode's summary and score stay hidden.",
            width,
        );
    }

    page.heading("Rating");
    if e.episode_ratings.state == State::Unavailable {
        page.line(Line::styled(
            format!("  {}", episode_ratings_message(&e.episode_ratings)),
            DIM,
        ));
    } else {
        match &e.rating {
            Some(r) if shown => page.line(Line::raw(format!("  {}", score(r))).bold()),
            Some(_) => page.line(Line::styled(
                format!("  IMDb {HIDDEN_SCORE}  hidden until watched; s peeks"),
                DIM,
            )),
            None => page.line(Line::styled("  No IMDb score", DIM)),
        }
        episode_ratings_notes(&mut page, &e.episode_ratings, "ratings", width);
    }

    if let Some(overview) = e.overview.as_ref().filter(|o| !o.is_empty()) {
        page.blank();
        if shown {
            page.para(overview, width, None);
        } else {
            page.line(Line::styled(
                "Summary hidden until you watch this episode; s peeks.",
                DIM,
            ));
        }
    }

    key_crew(&mut page, &e.key_crew);
    page.heading("Guest stars");
    if e.guest_stars.is_empty() {
        page.line(Line::styled(
            "  TMDB lists no guest stars for this episode.",
            DIM,
        ));
    }
    cast_rows(&mut page, &e.guest_stars);
    page.heading("Series regulars");
    if e.cast.is_empty() {
        page.line(Line::styled(
            "  TMDB lists no series regulars for this episode.",
            DIM,
        ));
    }
    cast_rows(&mut page, &e.cast);
    page
}

fn watched_episode(watched: Option<&Watched>, season: u32, number: u32) -> bool {
    watched.is_some_and(|w| w.episode(season, number))
}

/// An episode's IMDb score, like `IMDb 9.0/10`.
fn score(r: &Rating) -> String {
    format!("{} {}", r.source.label(), r.display())
}

fn episode_ratings_message(r: &EpisodeRatings) -> &'static str {
    match r.unavailable_reason {
        Some(UnavailableReason::NotConfigured) => "Ratings aren't configured on the server.",
        Some(UnavailableReason::NoImdbId) => "No IMDb entry is linked to this show.",
        Some(UnavailableReason::ProviderError) | None => {
            "Episode ratings couldn't be loaded right now."
        }
    }
}

/// Says why scores are missing or old, if they are.
fn episode_ratings_notes(page: &mut Page, r: &EpisodeRatings, what: &str, width: u16) {
    match r.state {
        State::Unavailable => page.line(Line::styled(
            format!("  {}", episode_ratings_message(r)),
            DIM,
        )),
        State::Stale => {
            let day = fetched_day(&r.fetched_at)
                .map(|d| format!("; showing scores from {d}"))
                .unwrap_or_default();
            page.warn(format!("  Couldn't refresh {what}{day}."), width);
        }
        _ => {}
    }
}

fn key_crew(page: &mut Page, key_crew: &[KeyCrew]) {
    let crew: Vec<(&str, &Credit)> = key_crew
        .iter()
        .flat_map(|k| k.people.iter().map(|p| (k.role.label(), p)))
        .collect();
    if crew.is_empty() {
        return;
    }
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

fn cast_rows(page: &mut Page, cast: &[Credit]) {
    let column = column_width(cast.iter().map(|c| c.name.as_str()), NAME_COLUMN);
    for credit in cast {
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

fn season_label(number: u32) -> String {
    if number == 0 {
        "Specials".to_owned()
    } else {
        format!("Season {number}")
    }
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
