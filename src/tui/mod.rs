//! `credits`, the terminal client: browses a running credits server over its
//! JSON API, so it needs no database or TMDB token of its own.

pub mod api;
pub mod app;
pub mod client;
pub mod config;
pub mod ui;

use std::io;
use std::time::Instant;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

pub use app::App;
pub use client::Client;
pub use config::TuiConfig;

use app::{Request, Response};

/// Takes over the terminal until the user quits.
pub async fn run(config: TuiConfig) -> io::Result<()> {
    let client = Client::new(&config.url);
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
            Request::Title { seq, kind, id } => (
                seq,
                Response::Title(client.title(kind, id).await.map(Box::new)),
            ),
            Request::Person { seq, id } => {
                (seq, Response::Person(client.person(id).await.map(Box::new)))
            }
        };
        // The receiver only goes away when the app has quit.
        let _ = tx.send(answer);
    });
}
