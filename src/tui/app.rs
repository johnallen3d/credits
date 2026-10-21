//! Terminal client state and key handling. No I/O happens here: keys and
//! server responses go in, and `Request`s come out for the runtime to send.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::api::{Kind, Person, SearchHit, Title};
use super::client::ClientError;

/// Typing pause before a search goes out, so each keystroke isn't a request.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const PAGE_STEP: usize = 10;

/// Something a page links to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Title(Kind, u64),
    Person(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Search { seq: u64, query: String },
    Title { seq: u64, kind: Kind, id: u64 },
    Person { seq: u64, id: u64 },
}

#[derive(Debug)]
pub enum Response {
    Search(Result<Vec<SearchHit>, ClientError>),
    Title(Result<Box<Title>, ClientError>),
    Person(Result<Box<Person>, ClientError>),
}

#[derive(Debug)]
pub enum Load<T> {
    Loading,
    Ready(Box<T>),
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

#[derive(Debug)]
pub enum Screen {
    Title {
        target: (Kind, u64),
        seq: u64,
        load: Load<Title>,
        cursor: Cursor,
    },
    Person {
        id: u64,
        seq: u64,
        load: Load<Person>,
        cursor: Cursor,
    },
}

impl Screen {
    pub fn cursor(&self) -> &Cursor {
        match self {
            Screen::Title { cursor, .. } | Screen::Person { cursor, .. } => cursor,
        }
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        match self {
            Screen::Title { cursor, .. } | Screen::Person { cursor, .. } => cursor,
        }
    }

    fn seq(&self) -> u64 {
        match self {
            Screen::Title { seq, .. } | Screen::Person { seq, .. } => *seq,
        }
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

#[derive(Debug)]
pub struct App {
    pub server: String,
    pub search: Search,
    /// Typing goes to the search box; otherwise keys move through the page.
    pub editing: bool,
    /// Pages opened from search, most recent last.
    pub stack: Vec<Screen>,
    pub quit: bool,
    next_seq: u64,
}

impl App {
    pub fn new(server: &str) -> Self {
        Self {
            server: server.to_owned(),
            search: Search::default(),
            editing: true,
            stack: Vec::new(),
            quit: false,
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
            Response::Title(result) => {
                if let Some(Screen::Title { load, .. }) = self.screen_for(seq) {
                    *load = loaded(result);
                }
            }
            Response::Person(result) => {
                if let Some(Screen::Person { load, .. }) = self.screen_for(seq) {
                    *load = loaded(result);
                }
            }
        }
    }

    /// The highlighted link on the current page, once a renderer has drawn it.
    pub fn cursor(&self) -> &Cursor {
        match self.stack.last() {
            Some(screen) => screen.cursor(),
            None => &self.search.cursor,
        }
    }

    pub fn cursor_mut(&mut self) -> &mut Cursor {
        match self.stack.last_mut() {
            Some(screen) => screen.cursor_mut(),
            None => &mut self.search.cursor,
        }
    }

    /// Opens a link from the current page.
    pub fn open(&mut self, target: Target) -> Request {
        let seq = self.seq();
        let (screen, request) = match target {
            Target::Title(kind, id) => (
                Screen::Title {
                    target: (kind, id),
                    seq,
                    load: Load::Loading,
                    cursor: Cursor::default(),
                },
                Request::Title { seq, kind, id },
            ),
            Target::Person(id) => (
                Screen::Person {
                    id,
                    seq,
                    load: Load::Loading,
                    cursor: Cursor::default(),
                },
                Request::Person { seq, id },
            ),
        };
        self.stack.push(screen);
        self.editing = false;
        request
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
            KeyCode::Down | KeyCode::Tab | KeyCode::Esc => {
                if !self.search.results.is_empty() {
                    self.editing = false;
                }
            }
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
            _ => {}
        }
        Vec::new()
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
            Some(Screen::Title {
                target: (kind, id),
                seq: s,
                load,
                ..
            }) => {
                *s = seq;
                *load = Load::Loading;
                Some(Request::Title {
                    seq,
                    kind: *kind,
                    id: *id,
                })
            }
            Some(Screen::Person {
                id, seq: s, load, ..
            }) => {
                *s = seq;
                *load = Load::Loading;
                Some(Request::Person { seq, id: *id })
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

    fn screen_for(&mut self, seq: u64) -> Option<&mut Screen> {
        self.stack.iter_mut().find(|s| s.seq() == seq)
    }

    fn seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }
}

fn loaded<T>(result: Result<Box<T>, ClientError>) -> Load<T> {
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
