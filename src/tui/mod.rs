//! `credits`, the terminal client: browses a running credits server over its
//! JSON API, so it needs no database or TMDB token of its own.

pub mod api;
pub mod app;
pub mod client;
pub mod config;
pub mod theme;
pub mod ui;

use std::io;
use std::time::Instant;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

pub use app::App;
pub use client::{Client, ClientError};
pub use config::TuiConfig;

use app::{Content, Request, Response, Target};

/// Takes over the terminal until the user quits.
pub async fn run(config: TuiConfig) -> io::Result<()> {
    let client = Client::new(&config.url);
    theme::set(config.theme);
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, client).await;
    ratatui::restore();
    result
}

async fn event_loop(terminal: &mut DefaultTerminal, client: Client) -> io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<(u64, Response)>();
    let mut events = EventStream::new();
    let mut app = App::new(client.base());
    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        if app.quit {
            return Ok(());
        }
        let due = app.search_due();
        let requests = tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) => app.key(key, Instant::now()),
                Some(Ok(_)) => Vec::new(),
                Some(Err(e)) => return Err(e),
                None => return Ok(()),
            },
            Some((seq, response)) = rx.recv() => {
                app.respond(seq, response);
                Vec::new()
            }
            () = tokio::time::sleep_until(due.unwrap_or_else(Instant::now).into()), if due.is_some() => {
                app.tick(Instant::now()).into_iter().collect()
            }
        };
        for request in requests {
            send(&client, &tx, request);
        }
    }
}

/// Runs a request in the background; its answer comes back on `tx`.
fn send(client: &Client, tx: &mpsc::UnboundedSender<(u64, Response)>, request: Request) {
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let answer = match request {
            Request::Search { seq, query } => (seq, Response::Search(client.search(&query).await)),
            Request::Page { seq, target } => (seq, Response::Page(fetch(&client, target).await)),
        };
        // The receiver only goes away when the app has quit.
        let _ = tx.send(answer);
    });
}

/// Loads the page `target` links to.
pub async fn fetch(client: &Client, target: Target) -> Result<Content, ClientError> {
    Ok(match target {
        Target::Title(kind, id) => Content::Title(Box::new(client.title(kind, id).await?)),
        Target::Person(id) => Content::Person(Box::new(client.person(id).await?)),
        Target::Season(show, number) => {
            Content::Season(Box::new(client.season(show, number).await?))
        }
        Target::Episode(show, season, number) => {
            Content::Episode(Box::new(client.episode(show, season, number).await?))
        }
    })
}
