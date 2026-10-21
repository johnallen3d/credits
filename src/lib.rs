pub mod cache;
pub mod catalog;
pub mod config;
pub mod model;
mod omdb;
pub mod playback;
mod ratings;
pub mod recent;
mod services;
mod tmdb;
pub mod tui;
mod watched;
mod watchlist;
pub mod web;

pub use catalog::Catalog;
pub use config::Config;
pub use web::app;
