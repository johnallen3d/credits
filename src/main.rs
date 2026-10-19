use std::process::ExitCode;

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

    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("credits: cannot listen on {}: {e}", config.bind);
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("listening on http://{}", config.bind);

    if let Err(e) = axum::serve(listener, app(Catalog::new(&config))).await {
        eprintln!("credits: server error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
