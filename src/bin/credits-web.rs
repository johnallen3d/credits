use std::process::ExitCode;

use credits::playback::{self, HomeAssistant};
use credits::{Catalog, Config, app};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("credits=info")),
        )
        .init();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("credits: configuration error: {e}");
            return ExitCode::FAILURE;
        }
    };

    if config.omdb_api_key.is_none() {
        tracing::warn!("OMDB_API_KEY is not set; ratings will show as unavailable");
    }
    let catalog = match Catalog::new(&config).await {
        Ok(catalog) => catalog,
        Err(e) => {
            eprintln!(
                "credits: cannot open cache database {:?}: {e}",
                config.database_path
            );
            return ExitCode::FAILURE;
        }
    };

    if config.omdb_api_key.is_some() {
        // OMDb's season listings give each episode's IMDb identifier, which
        // the dataset is looked up by.
        tokio::spawn(catalog.clone().keep_imdb_ratings_fresh());
    }

    match &config.home_assistant {
        Some(ha) => {
            tracing::info!(players = ?ha.players, "marking episodes watched from Home Assistant");
            tokio::spawn(playback::run(
                catalog.clone(),
                HomeAssistant::new(ha),
                ha.players.clone(),
            ));
        }
        None => tracing::info!("HA_URL or HA_TOKEN is not set; not marking episodes from Apple TV"),
    }

    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("credits: cannot listen on {}: {e}", config.bind);
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("listening on http://{}", config.bind);

    if let Err(e) = axum::serve(listener, app(catalog)).await {
        eprintln!("credits: server error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
