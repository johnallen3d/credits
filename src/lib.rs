pub mod cache;
pub mod catalog;
pub mod config;
pub mod model;
mod omdb;
mod ratings;
pub mod recent;
mod services;
mod tmdb;
mod watched;
mod watchlist;
pub mod web;

pub use catalog::Catalog;
pub use config::Config;
pub use web::app;
