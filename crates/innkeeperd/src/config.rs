use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use innkeeper_session::SessionConfig;
use innkeeper_world::RetentionPolicy;
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
    /// SPKI PEM of the operator RSA key sealed connection records are encrypted to.
    #[arg(long)]
    pub(crate) audit_public_key: Option<PathBuf>,
    /// Key identifier stored on each sealed record. In production, the escrow key version URL.
    #[arg(long)]
    pub(crate) audit_key_id: Option<String>,
    /// Days a sealed connection record is kept, unless a hold covers it.
    #[arg(long, default_value_t = 90)]
    pub(crate) audit_retention_days: u32,
    /// Days a letter is kept after the player deletes it.
    #[arg(long, default_value_t = 90)]
    pub(crate) retain_mail_days: u32,
    /// Days a bulletin post is kept after it is deleted.
    #[arg(long, default_value_t = 90)]
    pub(crate) retain_post_days: u32,
    /// Days an account is kept after it is deleted on request.
    #[arg(long, default_value_t = 30)]
    pub(crate) retain_account_days: u32,
    /// Days a complaint is kept, counted from when it arrived.
    #[arg(long, default_value_t = 365)]
    pub(crate) retain_complaint_days: u32,
    /// Days an expired session token is kept. The sessions table is not built yet.
    #[arg(long, default_value_t = 30)]
    pub(crate) retain_session_days: u32,
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
    /// Sealed connection records (docs/server/privacy.md).
    Audit {
        #[command(subcommand)]
        action: AuditAction,
    },
    /// Hold an account's sealed records and its content until a date.
    Hold {
        #[arg(long)]
        account: u32,
        /// Last calendar day covered, as YYYY-MM-DD.
        #[arg(long)]
        until: String,
        #[arg(long)]
        data_dir: PathBuf,
    },
    /// Clear a hold on an account.
    Release {
        #[arg(long)]
        account: u32,
        #[arg(long)]
        data_dir: PathBuf,
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
    /// Undo an account deletion within the retention window.
    Restore {
        /// The account number.
        id: u32,
        /// Directory holding innkeeper.db.
        #[arg(long)]
        data_dir: PathBuf,
    },
    /// Disable the account and everything it owns. Rows stay until the retention purge.
    Delete {
        /// The account number.
        id: u32,
        /// Directory holding innkeeper.db.
        #[arg(long)]
        data_dir: PathBuf,
        /// Actually disable. Without this flag, print the plan and exit 1.
        #[arg(long)]
        yes: bool,
        /// Hard-delete now. Refuses while a hold applies. Also requires --yes.
        #[arg(long)]
        purge_now: bool,
    },
}

#[derive(Clone, Debug, clap::Subcommand)]
pub(crate) enum AuditAction {
    /// Write one account's sealed rows as JSON lines.
    Export {
        #[arg(long)]
        account: u32,
        /// Include rows from this UTC date, inclusive.
        #[arg(long)]
        since: Option<String>,
        /// Include rows before this UTC date, exclusive of the next midnight.
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        data_dir: PathBuf,
    },
    /// Decrypt an export offline.
    Open {
        /// JSON lines from `audit export` or `audit seal`.
        #[arg(long = "in")]
        r#in: PathBuf,
        /// DEVELOPMENT ONLY: decrypt with a local PEM. Production uses --data-keys.
        #[arg(
            long,
            conflicts_with = "data_keys",
            required_unless_present = "data_keys"
        )]
        private_key: Option<PathBuf>,
        /// JSON lines of data keys unwrapped by the escrow vault.
        #[arg(
            long,
            conflicts_with = "private_key",
            required_unless_present = "private_key"
        )]
        data_keys: Option<PathBuf>,
    },
    /// Same as `innkeeperd hold`.
    Hold {
        #[arg(long)]
        account: u32,
        #[arg(long)]
        until: String,
        #[arg(long)]
        data_dir: PathBuf,
    },
    /// Same as `innkeeperd release`.
    Release {
        #[arg(long)]
        account: u32,
        #[arg(long)]
        data_dir: PathBuf,
    },
    /// Write a local RSA pair. The private key is a development key.
    DevKeygen {
        #[arg(long)]
        out_dir: PathBuf,
    },
    /// Seal one JSON record. tools/audit_roundtrip.sh uses this.
    Seal {
        #[arg(long)]
        public_key: PathBuf,
        #[arg(long)]
        key_id: String,
        /// The JSON object to seal, as one argument.
        #[arg(long)]
        record: String,
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
        let key = self.audit_public_key.is_some();
        let kid = self.audit_key_id.is_some();
        if key != kid {
            let text = "--audit-public-key and --audit-key-id must be given together\n";
            return Err(clap::Error::raw(ErrorKind::ArgumentConflict, text));
        }
        Ok(self)
    }

    pub(crate) fn retention(&self) -> RetentionPolicy {
        RetentionPolicy {
            mail_days: self.retain_mail_days,
            post_days: self.retain_post_days,
            account_days: self.retain_account_days,
            complaint_days: self.retain_complaint_days,
            audit_days: self.audit_retention_days,
            session_days: self.retain_session_days,
        }
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

    #[test]
    fn retention_flags_match_the_policy_defaults() {
        let config = Config::parse_from(["innkeeperd"]);
        assert_eq!(config.retention(), RetentionPolicy::default());
        assert!(config.audit_public_key.is_none());
        assert!(config.audit_key_id.is_none());
    }

    #[test]
    fn the_audit_public_key_and_its_id_are_required_together() {
        let error = Config::try_load_from(["innkeeperd", "--audit-public-key", "a.pem"]);
        assert!(error.unwrap_err().to_string().contains("together"));
        let config = Config::try_load_from([
            "innkeeperd",
            "--audit-public-key",
            "a.pem",
            "--audit-key-id",
            "https://escrow.vault.azure.net/keys/audit/v1",
        ])
        .unwrap();
        assert_eq!(
            config.audit_key_id.as_deref(),
            Some("https://escrow.vault.azure.net/keys/audit/v1")
        );
    }
}
