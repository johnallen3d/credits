use std::process::ExitCode;

use credits::tui::{self, TuiConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match TuiConfig::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("credits: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = tui::run(config).await {
        eprintln!("credits: terminal error: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
