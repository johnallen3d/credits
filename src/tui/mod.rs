//! `credits`, the terminal client: browses a running credits server over its
//! JSON API, so it needs no database or TMDB token of its own.

pub mod api;
pub mod app;
pub mod client;
pub mod config;
mod help;
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

use api::RecentKind;
use app::{Change, Content, ListName, Request, Response, Saved, Target};

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
    send(&client, &tx, app.load_recent());
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
            Request::Recent { seq } => (seq, Response::Recent(client.recent().await)),
            Request::Page { seq, target } => (seq, Response::Page(fetch(&client, target).await)),
            Request::Save(change) => (0, Response::Saved(change, save(&client, change).await)),
        };
        // The receiver only goes away when the app has quit.
        let _ = tx.send(answer);
    });
}

/// Sends a toggle; the answer is the new state.
pub async fn save(client: &Client, change: Change) -> Result<Saved, ClientError> {
    Ok(match change {
        Change::Listed {
            list,
            kind,
            id,
            listed,
        } => Saved::Title(client.set_listed(list, kind, id, listed).await?),
        Change::Watched { target, watched } => match target {
            Target::Title(kind, id) => {
                Saved::Title(client.set_title_watched(kind, id, watched).await?)
            }
            Target::Season(show, number) => {
                Saved::Show(client.set_watched(show, number, None, watched).await?)
            }
            Target::Episode(show, season, number) => Saved::Show(
                client
                    .set_watched(show, season, Some(number), watched)
                    .await?,
            ),
            Target::Person(_) | Target::Credits(..) | Target::List(..) => {
                return Err(ClientError::Server(
                    "that can't be marked watched".to_owned(),
                ));
            }
        },
    })
}

/// Loads the page `target` links to. Titles and people then count as
/// recently viewed; like on the web, failing to record that never fails the page.
pub async fn fetch(client: &Client, target: Target) -> Result<Content, ClientError> {
    Ok(match target {
        Target::Title(kind, id) => {
            let title = client.title(kind, id).await?;
            let _ = client.record_view(kind.into(), id).await;
            Content::Title(Box::new(title))
        }
        Target::Credits(kind, id) => Content::Credits(Box::new(client.credits(kind, id).await?)),
        Target::Person(id) => {
            let person = client.person(id).await?;
            let _ = client.record_view(RecentKind::Person, id).await;
            Content::Person(Box::new(person))
        }
        Target::Season(show, number) => {
            Content::Season(Box::new(client.season(show, number).await?))
        }
        Target::Episode(show, season, number) => {
            Content::Episode(Box::new(client.episode(show, season, number).await?))
        }
        Target::List(ListName::Watched, view) => {
            Content::Watched(Box::new(client.watched(view).await?))
        }
        Target::List(list, view) => Content::List(Box::new(client.list(list, view).await?)),
    })
}
