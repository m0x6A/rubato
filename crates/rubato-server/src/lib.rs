//! `rubato-server` as a library so integration tests build the same router the binary serves.

pub mod api;
pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod github;
pub mod github_local;
pub mod pairing;
pub mod plan_parse;
pub mod poller;
pub mod progress;
pub mod state;
pub mod telemetry;
pub mod ws_companion;
pub mod ws_web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
