//! innkeeperd: wires one sans-IO session per TCP connection; usage is in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod capture;
mod config;
mod connection;
mod server;

use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    match server::serve(config::Config::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "innkeeperd stopped");
            ExitCode::FAILURE
        }
    }
}
