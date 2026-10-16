//! innkeeperd: wires one sans-IO session per TCP connection; usage is in docs/server/innkeeperd.md.
#![forbid(unsafe_code)]

mod admin;
mod audit;
mod capture;
mod config;
mod connection;
mod connection_gate;
mod dates;
mod dev_oidc;
mod host;
mod http;
mod int14h_listener;
mod keyvault;
mod peer;
mod retention;
mod sandbox;
mod save_channel;
mod seal;
mod server;
mod signin;
mod sqlite_store;
mod subject;
mod switchboard;
mod websocket;

use std::io::{self, Write};
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    init_tracing();
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
        Some(config::Command::Admin { action }) => return admin::cli::run(action),
        Some(config::Command::DevOidc { bind, issuer }) => return dev_oidc::run(bind, issuer),
        None => {}
    }
    let probe = config.sandbox_probe.clone();
    let confine = config.sandbox_enabled();
    let allow_root = config.allow_root;
    let log_only = config.sandbox_log;
    let prepared = match server::prepare(config) {
        Ok(prepared) => prepared,
        Err(error) => return failed(error),
    };
    if confine {
        // Name resolution loads NSS modules. A later dlopen dies once mmap cannot be executable.
        sandbox::preload_resolver();
        let ports = prepared.bound_ports();
        let read_dirs = prepared.read_dirs();
        let request = sandbox::ConfineRequest {
            euid: sandbox::current_euid(),
            allow_root,
            log_only,
            data_dir: prepared.data_dir(),
            capture_dir: prepared.capture_dir(),
            ports: &ports,
            read_dirs: &read_dirs,
        };
        if let Err(error) = sandbox::confine(&request) {
            return failed(error);
        }
    }
    if let Some(path) = probe {
        return sandbox::run_probe(&path);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return failed(error),
    };
    match runtime.block_on(server::run(prepared)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => failed(error),
    }
}

fn failed(error: impl std::fmt::Display) -> ExitCode {
    tracing::error!(%error, "innkeeperd stopped");
    ExitCode::FAILURE
}

/// A pipe holds tracing lines until the process exits unless each write is flushed.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(|| Flush(io::stderr()))
        .init();
}

struct Flush<W>(W);

impl<W: Write> Write for Flush<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.0.write(buf)?;
        self.0.flush()?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
