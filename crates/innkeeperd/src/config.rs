use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
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
    /// Directory for one hex capture file per session.
    #[arg(long, default_value = "work/captures")]
    pub(crate) capture_dir: PathBuf,
    /// Write no capture files.
    #[arg(long)]
    pub(crate) no_capture: bool,
    /// Directory holding the account database; without it every account number is admitted.
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    /// What the client's serial port reaches: a modem emulator, a raw line, or auto-detect.
    #[arg(long, value_enum, default_value_t = LineArg::Auto)]
    pub(crate) line: LineArg,
    /// Rate announced in the modem's CONNECT line on raw lines.
    #[arg(long, default_value_t = 2400)]
    pub(crate) connect_rate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum LineArg {
    Auto,
    Hayes,
    Pad,
}

impl Config {
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
        (!self.no_capture).then(|| self.capture_dir.clone())
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
}
