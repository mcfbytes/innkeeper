//! innkeeperd: wires one sans-IO session per TCP connection; usage is in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod audit;
mod capture;
mod config;
mod connection;
mod dates;
mod host;
mod int14h_listener;
mod peer;
mod retention;
mod save_channel;
mod seal;
mod server;
mod sqlite_store;
mod subject;
mod switchboard;
mod websocket;

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
    let mut config = config::Config::load();
    let retention = config.retention();
    match config.command.take() {
        Some(config::Command::Account { action }) => return subject::run(action, retention),
        Some(config::Command::Audit { action }) => return audit::run(action),
        Some(config::Command::Hold {
            account,
            until,
            data_dir,
        }) => return audit::run_hold(account, &until, &data_dir),
        Some(config::Command::Release { account, data_dir }) => {
            return audit::run_release(account, &data_dir)
        }
        None => {}
    }
    match server::serve(config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "innkeeperd stopped");
            ExitCode::FAILURE
        }
    }
}
