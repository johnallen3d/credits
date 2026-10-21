//! Terminal client state and key handling. No I/O happens here: keys and
//! server responses go in, and `Request`s come out for the runtime to send.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::api::{
    Episode, Kind, List, Marks, MarksUpdate, Person, SearchHit, Season, ServicesState, Title,
    Watched,
};
use super::client::ClientError;

/// Typing pause before a search goes out, so each keystroke isn't a request.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const PAGE_STEP: usize = 10;

/// Something a page links to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Title(Kind, u64),
    Person(u64),
    /// A TV show's season by number.
    Season(u64, u32),
    /// A TV show's episode by season and number.
    Episode(u64, u32, u32),
    List(ListName, ListView),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListName {
    Watchlist,
    Favorites,
}

impl ListName {
    pub fn label(self) -> &'static str {
        match self {
            ListName::Watchlist => "To watch",
            ListName::Favorites => "Favorites",
        }
    }
}

/// A list's filters and order, as the web page's query parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListView {
    /// Only movies or only TV; `None` for all.
    pub kind: Option<Kind>,
    pub by_rating: bool,
    /// Only titles on the user's services (To watch only).
    pub mine: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Search { seq: u64, query: String },
    Page { seq: u64, target: Target },
    Save(Change),
}

#[derive(Debug)]
pub enum Response {
    Search(Result<Vec<SearchHit>, ClientError>),
    Page(Result<Content, ClientError>),
    Saved(Change, Result<Saved, ClientError>),
}

/// A toggle sent to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Listed {
        list: ListName,
        kind: Kind,
        id: u64,
        listed: bool,
    },
    /// A movie, a TV show as a whole (`Target::Title`), a season or an episode.
    Watched { target: Target, watched: bool },
}

/// The server's answer to a `Change`: the new state.
#[derive(Debug)]
pub enum Saved {
    Title(MarksUpdate),
    Show(Watched),
}

/// The line under the page saying how the last toggle went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Saving,
    Done(String),
    Failed(String),
}

/// What a page shows once it has loaded.
#[derive(Debug)]
pub enum Content {
    Title(Box<Title>),
    Person(Box<Person>),
    Season(Box<Season>),
    Episode(Box<Episode>),
    List(Box<List>),
}

#[derive(Debug)]
pub enum Load {
    Loading,
    Ready(Content),
    Failed(String),
}

/// Which link is highlighted and how far the page is scrolled. Rendering
/// records how many links the page has so keys can clamp to it.
#[derive(Debug, Default)]
pub struct Cursor {
    pub selected: usize,
    pub scroll: usize,
    pub links: usize,
}

impl Cursor {
    fn step(&mut self, by: isize) {
        let last = self.links.saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(by).min(last);
    }
}

/// A page opened from search or another page.
#[derive(Debug)]
pub struct Screen {
    pub target: Target,
    seq: u64,
    pub load: Load,
    pub cursor: Cursor,
    /// Unwatched episodes, by (season, number), whose spoilers are shown anyway.
    pub peeked: Vec<(u32, u32)>,
}

impl Screen {
    fn new(target: Target, seq: u64) -> Self {
        Self {
            target,
            seq,
            load: Load::Loading,
            cursor: Cursor::default(),
            peeked: Vec::new(),
        }
    }

    pub fn peeking(&self, season: u32, number: u32) -> bool {
        self.peeked.contains(&(season, number))
    }
}

#[derive(Debug, Default)]
pub struct Search {
    pub query: String,
    /// The query the shown results (or error) answer.
    pub answered: String,
    pub results: Vec<SearchHit>,
    pub error: Option<String>,
    pub searching: bool,
    seq: u64,
    due: Option<Instant>,
    pub cursor: Cursor,
}

/// The help overlay scrolls independently of the page underneath it.
#[derive(Debug, Default)]
pub struct Help {
    pub scroll: usize,
    pub max_scroll: usize,
}

#[derive(Debug)]
pub struct App {
    pub server: String,
    pub search: Search,
    /// Typing goes to the search box; otherwise keys move through the page.
    pub editing: bool,
    /// Pages opened from search, most recent last.
    pub stack: Vec<Screen>,
    pub status: Option<Status>,
    pub quit: bool,
    pub help: Option<Help>,
    next_seq: u64,
}

impl App {
    pub fn new(server: &str) -> Self {
        Self {
            server: server.to_owned(),
            search: Search::default(),
            editing: true,
            stack: Vec::new(),
            status: None,
            quit: false,
            help: None,
            next_seq: 0,
        }
    }

    /// When the pending search should go out, for the runtime's timer.
    pub fn search_due(&self) -> Option<Instant> {
        self.search.due
    }

    /// Sends the debounced search once its pause has passed.
    pub fn tick(&mut self, now: Instant) -> Option<Request> {
        match self.search.due {
            Some(due) if due <= now => self.send_search(),
            _ => None,
        }
    }

    pub fn key(&mut self, key: KeyEvent, now: Instant) -> Vec<Request> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Vec::new();
        }
        // Help is modal, including while typing a search. It must not clear
        // status, navigate, edit the query or send saves underneath the overlay.
        if let Some(help) = &mut self.help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => self.help = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    help.scroll = (help.scroll + 1).min(help.max_scroll);
                }
                KeyCode::Up | KeyCode::Char('k') => help.scroll = help.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char('d') if ctrl || key.code == KeyCode::PageDown => {
                    help.scroll = (help.scroll + PAGE_STEP).min(help.max_scroll);
                }
                KeyCode::PageUp | KeyCode::Char('u') if ctrl || key.code == KeyCode::PageUp => {
                    help.scroll = help.scroll.saturating_sub(PAGE_STEP);
                }
                KeyCode::Home | KeyCode::Char('g') => help.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => help.scroll = help.max_scroll,
                _ => {}
            }
            return Vec::new();
        }
        if key.code == KeyCode::Char('?') && !ctrl {
            self.help = Some(Help::default());
            return Vec::new();
        }
        // A finished toggle's outcome stays until the next key.
        if self.status != Some(Status::Saving) {
            self.status = None;
        }
        if self.editing {
            self.edit_key(key, ctrl, now)
        } else {
            self.page_key(key, ctrl)
        }
    }

    pub fn respond(&mut self, seq: u64, response: Response) {
        match response {
            Response::Search(result) => {
                if seq != self.search.seq {
                    return;
                }
                self.search.searching = false;
                self.search.cursor = Cursor::default();
                match result {
                    Ok(results) => {
                        self.search.results = results;
                        self.search.error = None;
                    }
                    Err(e) => {
                        self.search.results.clear();
                        self.search.error = Some(capitalize(&e.to_string()));
                    }
                }
            }
            Response::Page(result) => {
                if let Some(screen) = self.stack.iter_mut().find(|s| s.seq == seq) {
                    screen.load = loaded(result);
                }
            }
            Response::Saved(change, result) => self.saved(change, result),
        }
    }

    /// The highlighted link on the current page, once a renderer has drawn it.
    pub fn cursor(&self) -> &Cursor {
        match self.stack.last() {
            Some(screen) => &screen.cursor,
            None => &self.search.cursor,
        }
    }

    pub fn cursor_mut(&mut self) -> &mut Cursor {
        match self.stack.last_mut() {
            Some(screen) => &mut screen.cursor,
            None => &mut self.search.cursor,
        }
    }

    /// Opens a link from the current page.
    pub fn open(&mut self, target: Target) -> Request {
        let seq = self.seq();
        self.stack.push(Screen::new(target, seq));
        self.editing = false;
        Request::Page { seq, target }
    }

    fn edit_key(&mut self, key: KeyEvent, ctrl: bool, now: Instant) -> Vec<Request> {
        match key.code {
            KeyCode::Char('u') if ctrl => {
                self.search.query.clear();
                self.query_changed(now);
            }
            KeyCode::Char(c) if !ctrl => {
                self.search.query.push(c);
                self.query_changed(now);
            }
            KeyCode::Backspace => {
                self.search.query.pop();
                self.query_changed(now);
            }
            KeyCode::Down => {
                if !self.search.results.is_empty() {
                    self.editing = false;
                }
            }
            // Leaves the box even without results, so w and f can open the lists.
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => self.editing = false,
            KeyCode::Enter => {
                if self.search.due.is_some() {
                    self.editing = false;
                    return self.send_search().into_iter().collect();
                }
                if self.search.error.is_some() {
                    return self.reload().into_iter().collect();
                }
                if !self.search.searching && !self.search.results.is_empty() {
                    return self.open_selected().into_iter().collect();
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn page_key(&mut self, key: KeyEvent, ctrl: bool) -> Vec<Request> {
        let on_search = self.stack.is_empty();
        match key.code {
            KeyCode::Tab | KeyCode::BackTab if on_search => self.editing = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('/') => {
                self.stack.clear();
                self.editing = true;
            }
            KeyCode::Char('j') | KeyCode::Down => self.cursor_mut().step(1),
            KeyCode::Char('k') | KeyCode::Up => {
                if on_search && self.search.cursor.selected == 0 {
                    self.editing = true;
                } else {
                    self.cursor_mut().step(-1);
                }
            }
            KeyCode::Char('d') if ctrl => self.cursor_mut().step(PAGE_STEP as isize),
            KeyCode::Char('u') if ctrl => self.cursor_mut().step(-(PAGE_STEP as isize)),
            KeyCode::PageDown => self.cursor_mut().step(PAGE_STEP as isize),
            KeyCode::PageUp => self.cursor_mut().step(-(PAGE_STEP as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.cursor_mut().selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                let cursor = self.cursor_mut();
                cursor.selected = cursor.links.saturating_sub(1);
            }
            KeyCode::Enter if on_search && self.search.error.is_some() => {
                return self.reload().into_iter().collect();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                return self.open_selected().into_iter().collect();
            }
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left => {
                if self.stack.pop().is_none() {
                    self.editing = true;
                }
            }
            KeyCode::Char('r') => return self.reload().into_iter().collect(),
            KeyCode::Char('s') => self.peek(),
            KeyCode::Char('[') => return self.neighbour(false).into_iter().collect(),
            KeyCode::Char(']') => return self.neighbour(true).into_iter().collect(),
            KeyCode::Char('w') => return self.open_list(ListName::Watchlist).into_iter().collect(),
            KeyCode::Char('f') => return self.open_list(ListName::Favorites).into_iter().collect(),
            KeyCode::Char('t') => {
                return self
                    .change_view(|v| {
                        v.kind = match v.kind {
                            None => Some(Kind::Movie),
                            Some(Kind::Movie) => Some(Kind::Tv),
                            Some(Kind::Tv) => None,
                        };
                    })
                    .into_iter()
                    .collect();
            }
            KeyCode::Char('o') => {
                return self
                    .change_view(|v| v.by_rating = !v.by_rating)
                    .into_iter()
                    .collect();
            }
            KeyCode::Char('m') if self.can_filter_services() => {
                return self.change_view(|v| v.mine = !v.mine).into_iter().collect();
            }
            KeyCode::Char('b') => return self.toggle_listed(ListName::Watchlist),
            KeyCode::Char('F') => return self.toggle_listed(ListName::Favorites),
            KeyCode::Char('x') => return self.toggle_watched(false),
            KeyCode::Char('X') => return self.toggle_watched(true),
            _ => {}
        }
        Vec::new()
    }

    /// Shows or hides the spoilers of the episode on screen, or the one
    /// highlighted on a season.
    fn peek(&mut self) {
        let selected = super::ui::selected_target(self);
        let Some(screen) = self.stack.last_mut() else {
            return;
        };
        let episode = match (screen.target, selected) {
            (Target::Episode(_, season, number), _)
            | (Target::Season(..), Some(Target::Episode(_, season, number))) => (season, number),
            _ => return,
        };
        match screen.peeked.iter().position(|&e| e == episode) {
            Some(i) => {
                screen.peeked.remove(i);
            }
            None => screen.peeked.push(episode),
        }
    }

    /// Bookmarks or hearts the title on screen, or the highlighted list row.
    fn toggle_listed(&mut self, list: ListName) -> Vec<Request> {
        let Some(Target::Title(kind, id)) = self.title_target() else {
            return Vec::new();
        };
        let current = self.marks_of(kind, id).and_then(|m| match list {
            ListName::Watchlist => m.to_watch,
            ListName::Favorites => m.favorite,
        });
        let Some(current) = current else {
            return self.refuse(format!(
                "{} couldn't be read, so it can't be changed; r reloads.",
                list.label()
            ));
        };
        self.save(Change::Listed {
            list,
            kind,
            id,
            listed: !current,
        })
    }

    /// Marks the highlighted season, episode or list row watched or not, or
    /// the page itself when nothing markable is highlighted or `page` is set.
    fn toggle_watched(&mut self, page: bool) -> Vec<Request> {
        let selected = super::ui::selected_target(self)
            .filter(|_| !page)
            .filter(|t| self.watched_of(*t).is_some());
        let Some(target) = selected.or_else(|| self.subject()) else {
            return Vec::new();
        };
        let Some(current) = self.watched_of(target) else {
            return self
                .refuse("Watched marks couldn't be read, so they can't be changed; r reloads.");
        };
        self.save(Change::Watched {
            target,
            watched: !current,
        })
    }

    fn refuse(&mut self, message: impl Into<String>) -> Vec<Request> {
        self.status = Some(Status::Failed(message.into()));
        Vec::new()
    }

    fn save(&mut self, change: Change) -> Vec<Request> {
        // One at a time, so answers can't arrive out of order.
        if self.status == Some(Status::Saving) {
            return Vec::new();
        }
        self.status = Some(Status::Saving);
        vec![Request::Save(change)]
    }

    /// The page's own title, season or episode.
    fn subject(&self) -> Option<Target> {
        match &self.stack.last()?.load {
            Load::Ready(Content::Title(t)) => Some(Target::Title(t.kind, t.id)),
            Load::Ready(Content::Season(s)) => Some(Target::Season(s.show_id, s.number)),
            Load::Ready(Content::Episode(e)) => {
                Some(Target::Episode(e.show_id, e.season_number, e.number))
            }
            _ => None,
        }
    }

    /// The title b and F act on: the one on screen, or the highlighted row.
    pub(super) fn title_target(&self) -> Option<Target> {
        match &self.stack.last()?.load {
            Load::Ready(Content::Title(t)) => Some(Target::Title(t.kind, t.id)),
            Load::Ready(Content::List(_)) => super::ui::selected_target(self),
            _ => None,
        }
    }

    /// A title's marks as the current page knows them.
    fn marks_of(&self, kind: Kind, id: u64) -> Option<Marks> {
        match &self.stack.last()?.load {
            Load::Ready(Content::Title(t)) if t.kind == kind && t.id == id => t.marks,
            Load::Ready(Content::List(l)) => l
                .titles
                .iter()
                .find(|e| e.kind == kind && e.id == id)
                .and_then(|e| e.marks),
            _ => None,
        }
    }

    /// Whether `target` is watched, as the current page knows it; `None` if
    /// it can't say.
    fn watched_of(&self, target: Target) -> Option<bool> {
        let show = |watched: &Option<Watched>| {
            watched
                .as_ref()
                .filter(|_| Watched::readable(watched.as_ref()))
                .cloned()
        };
        let marks = match (&self.stack.last()?.load, target) {
            (_, Target::Title(kind, id)) => return self.marks_of(kind, id)?.watched,
            (Load::Ready(Content::Title(t)), _) => show(&t.watched),
            (Load::Ready(Content::Season(s)), _) => show(&s.watched),
            (Load::Ready(Content::Episode(e)), _) => show(&e.watched),
            _ => None,
        }?;
        match target {
            Target::Season(_, number) => Some(marks.season(number)),
            Target::Episode(_, season, number) => Some(marks.episode(season, number)),
            _ => None,
        }
    }

    /// Applies the server's answer to every open page showing it; a failure
    /// changes nothing.
    fn saved(&mut self, change: Change, result: Result<Saved, ClientError>) {
        let saved = match result {
            Ok(saved) => saved,
            Err(e) => {
                let e = match e {
                    ClientError::NotFound => "not found".to_owned(),
                    e => e.to_string(),
                };
                self.status = Some(Status::Failed(format!("Couldn't save: {e}")));
                return;
            }
        };
        let show = match change {
            Change::Listed {
                kind: Kind::Tv, id, ..
            }
            | Change::Watched {
                target: Target::Title(Kind::Tv, id),
                ..
            }
            | Change::Watched {
                target: Target::Season(id, _) | Target::Episode(id, ..),
                ..
            } => Some(id),
            _ => None,
        };
        match saved {
            Saved::Title(update) => {
                if let Change::Listed { kind, id, .. }
                | Change::Watched {
                    target: Target::Title(kind, id),
                    ..
                } = change
                {
                    self.apply_marks(kind, id, update.marks);
                }
                if let (Some(show), Some(watched)) = (show, update.watched) {
                    self.apply_watched(show, watched);
                }
            }
            Saved::Show(watched) => {
                if let Some(show) = show {
                    self.apply_watched(show, watched);
                }
            }
        }
        self.status = Some(Status::Done(self.saved_message(change)));
    }

    fn saved_message(&self, change: Change) -> String {
        match change {
            Change::Listed { list, listed, .. } => {
                let verb = if listed { "Added to" } else { "Removed from" };
                format!("{verb} {}.", list.label())
            }
            Change::Watched { target, watched } => {
                let what = match target {
                    Target::Title(Kind::Movie, _) => "Movie",
                    Target::Title(Kind::Tv, _) => "Show",
                    Target::Season(..) => "Season",
                    _ => "Episode",
                };
                // Unmarking a season of a watched show leaves it covered.
                let covered = !watched && self.watched_of(target) == Some(true);
                match (watched, covered) {
                    (true, _) => format!("{what} marked watched."),
                    (false, false) => format!("{what} marked unwatched."),
                    (false, true) => format!(
                        "{what} unmarked, but still watched with its {}.",
                        if matches!(target, Target::Episode(..)) {
                            "season or show"
                        } else {
                            "show"
                        }
                    ),
                }
            }
        }
    }

    fn apply_marks(&mut self, kind: Kind, id: u64, marks: Marks) {
        for screen in &mut self.stack {
            match &mut screen.load {
                Load::Ready(Content::Title(t)) if t.kind == kind && t.id == id => {
                    t.marks = Some(marks);
                }
                Load::Ready(Content::List(l)) => {
                    for e in l.titles.iter_mut().filter(|e| e.kind == kind && e.id == id) {
                        e.marks = Some(marks);
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_watched(&mut self, show: u64, watched: Watched) {
        let whole = Watched::readable(Some(&watched)).then_some(watched.show);
        for screen in &mut self.stack {
            match &mut screen.load {
                Load::Ready(Content::Title(t)) if t.kind == Kind::Tv && t.id == show => {
                    t.watched = Some(watched.clone());
                    if let Some(marks) = &mut t.marks {
                        marks.watched = whole;
                    }
                }
                Load::Ready(Content::Season(s)) if s.show_id == show => {
                    s.watched = Some(watched.clone());
                }
                Load::Ready(Content::Episode(e)) if e.show_id == show => {
                    e.watched = Some(watched.clone());
                }
                Load::Ready(Content::List(l)) => {
                    for e in l
                        .titles
                        .iter_mut()
                        .filter(|e| e.kind == Kind::Tv && e.id == show)
                    {
                        if let Some(marks) = &mut e.marks {
                            marks.watched = whole;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Swaps the episode on screen for the previous or next one in its season.
    fn neighbour(&mut self, next: bool) -> Option<Request> {
        let Some(Screen {
            load: Load::Ready(Content::Episode(e)),
            ..
        }) = self.stack.last()
        else {
            return None;
        };
        let number = if next { e.next } else { e.previous }?;
        let target = Target::Episode(e.show_id, e.season_number, number);
        self.stack.pop();
        Some(self.open(target))
    }

    /// Opens a list over the current page; from the other list, swaps it.
    fn open_list(&mut self, name: ListName) -> Option<Request> {
        if let Some(Screen {
            target: Target::List(open, _),
            ..
        }) = self.stack.last()
        {
            if *open == name {
                return None;
            }
            self.stack.pop();
        }
        Some(self.open(Target::List(name, ListView::default())))
    }

    /// Refetches the list on screen with a changed filter or order.
    fn change_view(&mut self, change: impl FnOnce(&mut ListView)) -> Option<Request> {
        let seq = self.seq();
        let screen = self.stack.last_mut()?;
        let Target::List(name, mut view) = screen.target else {
            return None;
        };
        change(&mut view);
        screen.target = Target::List(name, view);
        screen.seq = seq;
        screen.load = Load::Loading;
        screen.cursor = Cursor::default();
        Some(Request::Page {
            seq,
            target: screen.target,
        })
    }

    /// The services filter only means something once services are chosen;
    /// it can always be turned back off.
    pub(super) fn can_filter_services(&self) -> bool {
        match self.stack.last() {
            Some(Screen {
                target: Target::List(ListName::Watchlist, view),
                load,
                ..
            }) => {
                view.mine
                    || matches!(load, Load::Ready(Content::List(l))
                        if l.services.as_ref().is_some_and(|s| s.state == ServicesState::Chosen))
            }
            _ => false,
        }
    }

    fn open_selected(&mut self) -> Option<Request> {
        let target = super::ui::selected_target(self)?;
        Some(self.open(target))
    }

    /// Fetches the current page again, e.g. after a stale or failed load.
    fn reload(&mut self) -> Option<Request> {
        let seq = self.seq();
        match self.stack.last_mut() {
            None => {
                self.search.answered.clear();
                self.send_search()
            }
            Some(screen) => {
                screen.seq = seq;
                screen.load = Load::Loading;
                Some(Request::Page {
                    seq,
                    target: screen.target,
                })
            }
        }
    }

    fn query_changed(&mut self, now: Instant) {
        self.search.due = Some(now + SEARCH_DEBOUNCE);
    }

    fn send_search(&mut self) -> Option<Request> {
        self.search.due = None;
        let query = self.search.query.trim().to_owned();
        if query.is_empty() {
            // Emptying the box clears the results; a fresh seq drops any in flight.
            let seq = self.seq();
            self.search = Search {
                query: std::mem::take(&mut self.search.query),
                seq,
                ..Search::default()
            };
            return None;
        }
        if query == self.search.answered && !self.search.searching {
            return None;
        }
        let seq = self.seq();
        self.search.seq = seq;
        self.search.searching = true;
        self.search.answered = query.clone();
        Some(Request::Search { seq, query })
    }

    fn seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }
}

fn loaded(result: Result<Content, ClientError>) -> Load {
    match result {
        Ok(value) => Load::Ready(value),
        Err(ClientError::NotFound) => Load::Failed("Not found on TMDB.".to_owned()),
        Err(e) => Load::Failed(capitalize(&e.to_string())),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
