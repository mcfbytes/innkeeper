use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use innkeeper_session::SessionConfig;
use pad_thai::{HayesConfig, LineKind};

/// Command line of the daemon.
#[derive(Debug, Parser)]
#[command(
    name = "innkeeperd",
    version,
    about = "Look behind you, a three-headed modem!"
)]
pub(crate) struct Config {
    /// Address to accept client connections on.
    #[arg(long, default_value = "127.0.0.1:2314")]
    pub(crate) bind: SocketAddr,
    /// Address to accept INT 14h transport clients on (docs/protocol/int14h-transport.md).
    #[arg(long, default_value = "127.0.0.1:2315")]
    pub(crate) int14h_bind: SocketAddr,
    /// Do not serve the INT 14h transport.
    #[arg(long)]
    pub(crate) no_int14h: bool,
    /// Address to accept browser clients on: each WebSocket is one line, binary messages its bytes.
    #[arg(long)]
    pub(crate) ws_bind: Option<SocketAddr>,
    /// Directory for one hex capture file per session.
    #[arg(long, default_value = "work/captures")]
    pub(crate) capture_dir: PathBuf,
    /// Write no capture files.
    #[arg(long)]
    pub(crate) no_capture: bool,
    /// Public host: no captures, no message fields, no peer addresses (docs/server/privacy.md).
    #[arg(long)]
    pub(crate) public: bool,
    /// Directory holding the account database; without it every account number is admitted.
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    /// What the client's serial port reaches: a modem emulator, a raw line, or auto-detect.
    #[arg(long, value_enum, default_value_t = LineArg::Auto)]
    pub(crate) line: LineArg,
    /// Rate announced in the modem's CONNECT line on raw lines.
    #[arg(long, default_value_t = 2400)]
    pub(crate) connect_rate: u32,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// Offline tools; with no subcommand the daemon serves.
#[derive(Clone, Debug, clap::Subcommand)]
pub(crate) enum Command {
    /// Export or delete one stored account (docs/server/privacy.md).
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
}

#[derive(Clone, Debug, clap::Subcommand)]
pub(crate) enum AccountAction {
    /// Print everything stored for the account as JSON on stdout.
    Export {
        /// The account number.
        id: u32,
        /// Directory holding innkeeper.db.
        #[arg(long)]
        data_dir: PathBuf,
        /// Include save file bytes as base64.
        #[arg(long)]
        with_saves: bool,
    },
    /// Delete the account and everything that belongs to it.
    Delete {
        /// The account number.
        id: u32,
        /// Directory holding innkeeper.db.
        #[arg(long)]
        data_dir: PathBuf,
        /// Actually delete. Without this flag, print the plan and exit 1.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LineArg {
    Auto,
    Hayes,
    Pad,
}

impl Config {
    /// Parse argv and refuse `--public` together with an explicit `--capture-dir`.
    pub(crate) fn load() -> Self {
        Self::try_load_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    pub(crate) fn try_load_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(args)?;
        let config = Self::from_arg_matches(&matches)?;
        config.validated(&matches)
    }

    fn validated(self, matches: &ArgMatches) -> Result<Self, clap::Error> {
        let capture_given = matches.value_source("capture_dir") == Some(ValueSource::CommandLine);
        if self.public && capture_given {
            let text =
                "--public cannot be used with --capture-dir: public mode writes no capture files\n";
            return Err(clap::Error::raw(ErrorKind::ArgumentConflict, text));
        }
        Ok(self)
    }

    pub(crate) fn session_config(&self) -> SessionConfig {
        let line = match self.line {
            LineArg::Auto => LineKind::Auto,
            LineArg::Hayes => LineKind::Hayes,
            LineArg::Pad => LineKind::Pad,
        };
        let hayes = HayesConfig {
            connect_rate: self.connect_rate,
            ..HayesConfig::default()
        };
        SessionConfig {
            line,
            hayes,
            ..SessionConfig::default()
        }
    }

    pub(crate) fn capture_dir(&self) -> Option<PathBuf> {
        (!self.no_capture && !self.public).then(|| self.capture_dir.clone())
    }

    pub(crate) fn int14h_bind(&self) -> Option<SocketAddr> {
        (!self.no_int14h).then_some(self.int14h_bind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_listen_locally_and_capture_under_work() {
        let config = Config::parse_from(["innkeeperd"]);
        assert_eq!(config.bind, "127.0.0.1:2314".parse().unwrap());
        assert_eq!(config.capture_dir(), Some(PathBuf::from("work/captures")));
        assert_eq!(config.session_config().line, LineKind::Auto);
        assert_eq!(config.data_dir, None);
        assert_eq!(
            config.int14h_bind(),
            Some("127.0.0.1:2315".parse().unwrap())
        );
    }

    #[test]
    fn the_int14h_transport_can_move_or_be_switched_off() {
        let moved = Config::parse_from(["innkeeperd", "--int14h-bind", "0.0.0.0:4000"]);
        assert_eq!(moved.int14h_bind(), Some("0.0.0.0:4000".parse().unwrap()));
        let off = Config::parse_from(["innkeeperd", "--no-int14h"]);
        assert_eq!(off.int14h_bind(), None);
    }

    #[test]
    fn websocket_lines_are_off_unless_bound() {
        assert_eq!(Config::parse_from(["innkeeperd"]).ws_bind, None);
        let bound = Config::parse_from(["innkeeperd", "--ws-bind", "127.0.0.1:2316"]);
        assert_eq!(bound.ws_bind, Some("127.0.0.1:2316".parse().unwrap()));
    }

    #[test]
    fn a_data_dir_names_where_accounts_are_kept() {
        let config = Config::parse_from(["innkeeperd", "--data-dir", "work/data"]);
        assert_eq!(config.data_dir, Some(PathBuf::from("work/data")));
    }

    #[test]
    fn options_reach_the_session() {
        let args = [
            "innkeeperd",
            "--line",
            "pad",
            "--no-capture",
            "--connect-rate",
            "9600",
        ];
        let config = Config::parse_from(args);
        assert_eq!(config.capture_dir(), None);
        assert_eq!(config.session_config().line, LineKind::Pad);
        assert_eq!(config.session_config().hayes.connect_rate, 9600);
    }

    #[test]
    fn public_mode_turns_capture_off() {
        let config = Config::try_load_from(["innkeeperd", "--public"]).unwrap();
        assert!(config.public);
        assert_eq!(config.capture_dir(), None);
    }

    #[test]
    fn public_mode_rejects_an_explicit_capture_dir() {
        let error = Config::try_load_from(["innkeeperd", "--public", "--capture-dir", "x"]);
        let text = error.unwrap_err().to_string();
        assert!(text.contains("--public"));
        assert!(text.contains("--capture-dir"));
        assert!(text.contains("no capture"));
    }

    #[test]
    fn account_export_parses_the_data_dir_and_saves_flag() {
        let config = Config::parse_from([
            "innkeeperd",
            "account",
            "export",
            "100001",
            "--data-dir",
            "work/data",
            "--with-saves",
        ]);
        match config.command {
            Some(Command::Account {
                action:
                    AccountAction::Export {
                        id,
                        data_dir,
                        with_saves,
                    },
            }) => {
                assert_eq!(id, 100_001);
                assert_eq!(data_dir, PathBuf::from("work/data"));
                assert!(with_saves);
            }
            other => panic!("expected export, got {other:?}"),
        }
    }

    #[test]
    fn account_delete_is_a_dry_run_without_yes() {
        let config = Config::parse_from([
            "innkeeperd",
            "account",
            "delete",
            "100001",
            "--data-dir",
            "work/data",
        ]);
        match config.command {
            Some(Command::Account {
                action: AccountAction::Delete { yes, .. },
            }) => assert!(!yes),
            other => panic!("expected delete, got {other:?}"),
        }
    }
}
