use std::ffi::OsString;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use innkeeper_session::SessionConfig;
use innkeeper_world::RetentionPolicy;
use pad_thai::{HayesConfig, LineKind};

use crate::signin::config::PageOrigins;
use crate::signin::EnrolmentMode;

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
    /// Players arrive only on a loopback WebSocket. The in-process sandbox is on unless --no-sandbox.
    #[arg(long)]
    pub(crate) public: bool,
    /// Apply Landlock and seccomp after startup (docs/server/hardening.md). On with --public.
    #[arg(long)]
    pub(crate) sandbox: bool,
    /// Leave the process unrestricted. This is how a public-mode process opts out of the sandbox.
    #[arg(long, conflicts_with_all = ["sandbox", "sandbox_log", "sandbox_probe"])]
    pub(crate) no_sandbox: bool,
    /// Run the sandbox as root. For a local DOSBox setup; refused together with --public.
    #[arg(long)]
    pub(crate) allow_root: bool,
    /// Seccomp logs unknown syscalls instead of killing the process (docs/server/hardening.md).
    #[arg(long)]
    pub(crate) sandbox_log: bool,
    /// Open PATH after the sandbox is installed, print the result, and exit.
    #[arg(long, hide = true, value_name = "PATH")]
    pub(crate) sandbox_probe: Option<PathBuf>,
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
    /// Days an expired or revoked session token is kept, unless a hold covers it.
    #[arg(long, default_value_t = 30)]
    pub(crate) retain_session_days: u32,
    /// TOML file naming the sign-in providers (docs/server/innkeeperd.md section 8). Needs --data-dir.
    #[arg(long, value_name = "FILE")]
    pub(crate) auth_config: Option<PathBuf>,
    /// Origin of the game page allowed to sign in: CORS, `return` and postMessage. Repeatable.
    #[arg(long = "page-origin", value_name = "ORIGIN")]
    pub(crate) page_origins: Vec<String>,
    /// Who creates accounts. Default open, or web-only with --public.
    #[arg(long, value_enum)]
    pub(crate) enrolment: Option<EnrolmentMode>,
    /// Hours a sign-in session token lives.
    #[arg(long, default_value_t = 12)]
    pub(crate) session_hours: u32,
    /// Accounts one signed-in identity may open per day.
    #[arg(long, default_value_t = 3)]
    pub(crate) accounts_per_day: u32,
    /// The server Key Vault, https://NAME.vault.azure.net: identity HMAC key, client secrets, audit key.
    #[arg(long, value_name = "URL")]
    pub(crate) key_vault: Option<String>,
    /// DEVELOPMENT ONLY: no vault; dev-identity.key and file secrets. With --public, loopback pages only.
    #[arg(long, conflicts_with = "key_vault")]
    pub(crate) insecure_dev_keys: bool,
    /// Test only: where Key Vault requests go instead of --key-vault.
    #[arg(long, hide = true, requires = "key_vault")]
    pub(crate) test_vault_endpoint: Option<String>,
    /// Test only: where managed-identity token requests go instead of IMDS.
    #[arg(long, hide = true, requires = "key_vault")]
    pub(crate) test_imds_endpoint: Option<String>,
    /// Test only: where service-principal token requests go instead of Entra ID.
    #[arg(long, hide = true, requires = "key_vault")]
    pub(crate) test_authority_endpoint: Option<String>,
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
    /// DEVELOPMENT ONLY: a mock OpenID Connect provider with an Allow button, on loopback.
    DevOidc {
        /// Loopback address to listen on.
        #[arg(long, default_value = "127.0.0.1:2902")]
        bind: SocketAddr,
        /// The issuer URL it announces. Default http://BIND.
        #[arg(long)]
        issuer: Option<String>,
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
    /// Parse argv. `--public` refuses raw TCP, a non-loopback WebSocket and `--allow-root`
    /// (docs/server/innkeeperd.md).
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

    fn validated(mut self, matches: &ArgMatches) -> Result<Self, clap::Error> {
        let on_cli = |name: &str| matches.value_source(name) == Some(ValueSource::CommandLine);
        if self.public && on_cli("capture_dir") {
            return Err(conflict(
                "--public cannot be used with --capture-dir: public mode writes no capture files\n",
            ));
        }
        let key = self.audit_public_key.is_some();
        let kid = self.audit_key_id.is_some();
        if key != kid {
            return Err(conflict(
                "--audit-public-key and --audit-key-id must be given together\n",
            ));
        }
        if self.public && self.allow_root {
            return Err(conflict(concat!(
                "--public cannot be used with --allow-root: ",
                "public mode refuses to run as root\n"
            )));
        }
        if self.public && on_cli("bind") {
            return Err(conflict(concat!(
                "--public cannot be used with --bind: public mode accepts players only ",
                "over the WebSocket, so there is no raw TCP listener\n"
            )));
        }
        if self.public && on_cli("int14h_bind") {
            return Err(conflict(concat!(
                "--public cannot be used with --int14h-bind: ",
                "public mode does not serve the INT 14h transport\n"
            )));
        }
        if self.public {
            let Some(websocket) = self.ws_bind else {
                return Err(conflict(concat!(
                    "--public requires --ws-bind: ",
                    "public mode accepts players only over the WebSocket\n"
                )));
            };
            if !is_loopback(websocket) {
                return Err(conflict(concat!(
                    "--public requires --ws-bind on a loopback address (127.0.0.0/8 or ::1); ",
                    "a TLS proxy on this host terminates TLS\n"
                )));
            }
            self.no_int14h = true;
        }
        self.check_sign_in()?;
        Ok(self)
    }

    /// The vault and dev-key rules (docs/server/innkeeperd.md section 8).
    fn check_sign_in(&self) -> Result<(), clap::Error> {
        if let Some(url) = &self.key_vault {
            crate::keyvault::check_vault_url(url)
                .map_err(|why| conflict(&format!("--key-vault {why}\n")))?;
            if self.audit_public_key.is_some() {
                return Err(conflict(
                    "--key-vault cannot be used with --audit-public-key: the vault holds audit-public-key\n",
                ));
            }
        }
        let origins =
            PageOrigins::parse(&self.page_origins).map_err(|why| conflict(&format!("{why}\n")))?;
        if self.public && self.key_vault.is_none() {
            if !self.insecure_dev_keys {
                return Err(conflict(concat!(
                    "--public requires --key-vault: the identity key and client secrets live in the ",
                    "server vault (--insecure-dev-keys is for a loopback test)\n"
                )));
            }
            if !origins.all_loopback() {
                return Err(conflict(concat!(
                    "--insecure-dev-keys with --public accepts only loopback --page-origin values; ",
                    "a public page needs --key-vault\n"
                )));
            }
        }
        if self.auth_config.is_some() && self.data_dir.is_none() {
            return Err(conflict(
                "--auth-config requires --data-dir: identities and sessions are stored there\n",
            ));
        }
        if self.session_hours == 0 {
            return Err(conflict("--session-hours must be at least 1\n"));
        }
        Ok(())
    }

    /// `--enrolment`, or its default: open, and web-only with `--public`.
    pub(crate) fn enrolment(&self) -> EnrolmentMode {
        self.enrolment.unwrap_or(if self.public {
            EnrolmentMode::WebOnly
        } else {
            EnrolmentMode::Open
        })
    }

    /// Addresses the process actually binds. Public mode keeps only the WebSocket.
    pub(crate) fn listen_plan(&self) -> ListenPlan {
        if self.public {
            ListenPlan {
                legacy: None,
                int14h: None,
                websocket: self.ws_bind,
            }
        } else {
            ListenPlan {
                legacy: Some(self.bind),
                int14h: self.int14h_bind(),
                websocket: self.ws_bind,
            }
        }
    }

    /// `--public` sandboxes unless `--no-sandbox`. `--sandbox-log` and the probe imply it too.
    pub(crate) fn sandbox_enabled(&self) -> bool {
        if self.no_sandbox {
            return false;
        }
        self.public || self.sandbox || self.sandbox_log || self.sandbox_probe.is_some()
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

/// What `serve` binds. The legacy and INT 14h sockets are absent in public mode.
pub(crate) struct ListenPlan {
    pub(crate) legacy: Option<SocketAddr>,
    pub(crate) int14h: Option<SocketAddr>,
    pub(crate) websocket: Option<SocketAddr>,
}

fn conflict(text: &str) -> clap::Error {
    clap::Error::raw(ErrorKind::ArgumentConflict, text)
}

/// 127.0.0.0/8 or ::1. An IPv4-mapped address is not loopback here.
fn is_loopback(addr: SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
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
        let config = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--insecure-dev-keys",
            "--ws-bind",
            "127.0.0.1:2316",
        ])
        .unwrap();
        assert!(config.public);
        assert_eq!(config.capture_dir(), None);
    }

    #[test]
    fn public_mode_rejects_an_explicit_tcp_bind() {
        let error = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--bind",
            "127.0.0.1:2314",
            "--ws-bind",
            "127.0.0.1:9",
        ]);
        let text = error.unwrap_err().to_string();
        assert!(text.contains("--bind"), "{text}");
        assert!(text.contains("no raw TCP"), "{text}");
    }

    #[test]
    fn public_mode_rejects_the_int14h_transport() {
        let error = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--int14h-bind",
            "127.0.0.1:9",
            "--ws-bind",
            "127.0.0.1:9",
        ]);
        let text = error.unwrap_err().to_string();
        assert!(text.contains("--int14h-bind"), "{text}");
        assert!(text.contains("INT 14h"), "{text}");
    }

    #[test]
    fn public_mode_requires_a_loopback_websocket() {
        let missing = Config::try_load_from(["innkeeperd", "--public"]);
        let text = missing.unwrap_err().to_string();
        assert!(text.contains("requires --ws-bind"), "{text}");
        let remote = Config::try_load_from(["innkeeperd", "--public", "--ws-bind", "0.0.0.0:9"]);
        let text = remote.unwrap_err().to_string();
        assert!(text.contains("loopback"), "{text}");
        let mapped = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--ws-bind",
            "[::ffff:127.0.0.1]:9",
        ]);
        assert!(mapped.unwrap_err().to_string().contains("loopback"));
        let local = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--insecure-dev-keys",
            "--ws-bind",
            "[::1]:9",
        ]);
        assert!(local.is_ok());
    }

    #[test]
    fn public_mode_opens_no_tcp_listener_and_the_default_still_does() {
        let public = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--insecure-dev-keys",
            "--ws-bind",
            "127.0.0.1:9",
            "--line",
            "pad",
            "--no-int14h",
        ])
        .unwrap();
        let plan = public.listen_plan();
        assert_eq!(plan.legacy, None);
        assert_eq!(plan.int14h, None);
        assert_eq!(plan.websocket, Some("127.0.0.1:9".parse().unwrap()));
        assert!(public.sandbox_enabled());

        let defaults = Config::parse_from(["innkeeperd"]);
        let plan = defaults.listen_plan();
        assert_eq!(plan.legacy, Some("127.0.0.1:2314".parse().unwrap()));
        assert_eq!(plan.int14h, Some("127.0.0.1:2315".parse().unwrap()));
        assert_eq!(plan.websocket, None);
        assert!(!defaults.sandbox_enabled());
    }

    #[test]
    fn no_sandbox_turns_the_public_sandbox_off_and_allow_root_is_refused() {
        let open = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--insecure-dev-keys",
            "--no-sandbox",
            "--ws-bind",
            "127.1.2.3:9",
        ])
        .unwrap();
        assert!(!open.sandbox_enabled());
        let rooted = Config::try_load_from([
            "innkeeperd",
            "--public",
            "--allow-root",
            "--ws-bind",
            "127.0.0.1:9",
        ]);
        assert!(rooted.unwrap_err().to_string().contains("--allow-root"));
        let flagged = Config::parse_from(["innkeeperd", "--sandbox"]);
        assert!(flagged.sandbox_enabled());
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

    #[test]
    fn public_mode_needs_the_vault_or_a_loopback_dev_page() {
        let base = ["innkeeperd", "--public", "--ws-bind", "127.0.0.1:9"];
        let bare = Config::try_load_from(base).unwrap_err().to_string();
        assert!(bare.contains("requires --key-vault"), "{bare}");
        let vault = Config::try_load_from(
            base.iter()
                .copied()
                .chain(["--key-vault", "https://inn-server.vault.azure.net"]),
        )
        .unwrap();
        assert_eq!(vault.enrolment(), EnrolmentMode::WebOnly);
        let dev = Config::try_load_from(base.iter().copied().chain([
            "--insecure-dev-keys",
            "--page-origin",
            "http://127.0.0.1:2901",
        ]));
        assert!(dev.is_ok());
        let exposed = Config::try_load_from(base.iter().copied().chain([
            "--insecure-dev-keys",
            "--page-origin",
            "https://play.example.org",
        ]))
        .unwrap_err()
        .to_string();
        assert!(exposed.contains("loopback --page-origin"), "{exposed}");
        let both = Config::try_load_from(base.iter().copied().chain([
            "--insecure-dev-keys",
            "--key-vault",
            "https://inn-server.vault.azure.net",
        ]));
        assert!(both.is_err());
    }

    #[test]
    fn the_vault_url_is_checked_and_replaces_the_audit_key_file() {
        let wrong = Config::try_load_from(["innkeeperd", "--key-vault", "https://evil.test"]);
        assert!(wrong.unwrap_err().to_string().contains("vault.azure.net"));
        let plain =
            Config::try_load_from(["innkeeperd", "--key-vault", "http://x.vault.azure.net"]);
        assert!(plain.is_err());
        let doubled = Config::try_load_from([
            "innkeeperd",
            "--key-vault",
            "https://x.vault.azure.net",
            "--audit-public-key",
            "a.pem",
            "--audit-key-id",
            "kid",
        ]);
        assert!(doubled
            .unwrap_err()
            .to_string()
            .contains("audit-public-key"));
    }

    #[test]
    fn enrolment_defaults_to_open_and_sign_in_needs_a_data_dir() {
        assert_eq!(
            Config::parse_from(["innkeeperd"]).enrolment(),
            EnrolmentMode::Open
        );
        let closed = Config::parse_from(["innkeeperd", "--enrolment", "closed"]);
        assert_eq!(closed.enrolment(), EnrolmentMode::Closed);
        let web = Config::parse_from(["innkeeperd", "--enrolment", "web-only"]);
        assert_eq!(web.enrolment(), EnrolmentMode::WebOnly);
        let missing = Config::try_load_from(["innkeeperd", "--auth-config", "auth.toml"]);
        assert!(missing.unwrap_err().to_string().contains("--data-dir"));
        let bad = Config::try_load_from(["innkeeperd", "--page-origin", "https://x.test/page"]);
        assert!(bad.is_err());
    }
}
