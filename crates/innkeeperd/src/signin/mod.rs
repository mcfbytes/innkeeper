//! Sign-in with common providers: the page contract in INN-Decomp `docs/web.md`, "Sign-in (server
//! contract)", and the tables in docs/server/privacy.md.

pub(crate) mod config;
pub(crate) mod keys;
mod pages;
mod routes;
pub(crate) mod rp;
pub(crate) mod startup;
pub(crate) mod state;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use innkeeper_world::{AccountId, StoreError};
use serde_json::json;
use tracing::warn;
use url::Url;

use crate::dates;
use crate::seal::{self, Recipient};
use crate::sqlite_store::SqliteStore;
use config::PageOrigins;
use keys::{IdentityKey, KeyRing};
use rp::Rp;
use state::{LoginStates, PageState};

/// A login that has not come back from its provider within this is refused at its callback.
pub(crate) const LOGIN_TTL: i64 = 10 * dates::MINUTE;

/// Who may create accounts (docs/server/innkeeperd.md section 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum EnrolmentMode {
    /// A Login for an unknown number creates it, and sign-in creates accounts too.
    Open,
    /// Only sign-in creates accounts; a Login never does.
    WebOnly,
    /// As web-only, but only for an identity on the invite list (`--invite-list`, the vault
    /// secret `invite-identities`): a test publish open to named testers.
    Invite,
    /// Nobody creates accounts.
    Closed,
}

impl EnrolmentMode {
    pub(crate) fn login_enrols(self) -> bool {
        self == EnrolmentMode::Open
    }

    pub(crate) fn web_creates(self) -> bool {
        self != EnrolmentMode::Closed
    }
}

/// Who `--enrolment invite` lets open an account: the vault secret and the file, merged.
#[derive(Debug, Default)]
pub(crate) struct Invites {
    pub(crate) vault: Option<std::collections::HashSet<IdentityKey>>,
    pub(crate) file: Option<crate::admin::allowlist::KeyFile>,
}

impl Invites {
    fn lists(&self, key: &IdentityKey) -> bool {
        let in_vault = self.vault.as_ref().is_some_and(|keys| keys.contains(key));
        in_vault
            || self
                .file
                .as_ref()
                .is_some_and(|file| file.keys("the invite list").contains(key))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.vault.is_none() && self.file.is_none()
    }
}

/// A WebSocket that presented a live session token, tied to its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) identity: IdentityKey,
    pub(crate) token_hash: [u8; 32],
}

/// Everything sign-in needs, gathered at startup before the sandbox.
pub(crate) struct SignInSetup {
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) keys: KeyRing,
    pub(crate) rp: Rp,
    pub(crate) origins: PageOrigins,
    pub(crate) enrolment: EnrolmentMode,
    /// The invite list; read only under `EnrolmentMode::Invite`.
    pub(crate) invites: Invites,
    pub(crate) session_lifetime: Duration,
    pub(crate) accounts_per_day: u32,
    /// An account or password needs an attestation of at least this age ...
    pub(crate) minimum_age: u32,
    /// ... and at least this terms version.
    pub(crate) terms_version: u32,
    pub(crate) recipient: Option<Recipient>,
    /// Every line must carry a session (public mode); the page then offers no unsigned way in.
    pub(crate) required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Popup,
    Redirect,
}

/// Where the page wants the result: its address (checked), its origin and the handshake.
struct PageReturn {
    url: Url,
    origin: String,
    mode: Mode,
    attest: Option<(u32, u32)>,
    /// The page's own random value for this login, echoed with the result so the page can tell
    /// its login from one an attacker started (login CSRF).
    page_nonce: Option<String>,
}

impl PageReturn {
    fn to_state(&self) -> PageState {
        PageState {
            url: self.url.as_str().to_owned(),
            origin: self.origin.clone(),
            redirect: self.mode == Mode::Redirect,
            attest: self.attest,
            page_nonce: self.page_nonce.clone(),
        }
    }

    fn from_state(state: &PageState) -> Option<Self> {
        Some(PageReturn {
            url: Url::parse(&state.url).ok()?,
            origin: state.origin.clone(),
            mode: match state.redirect {
                true => Mode::Redirect,
                false => Mode::Popup,
            },
            attest: state.attest,
            page_nonce: state.page_nonce.clone(),
        })
    }
}

pub(crate) struct SignIn {
    store: Arc<SqliteStore>,
    keys: KeyRing,
    rp: Rp,
    origins: PageOrigins,
    enrolment: EnrolmentMode,
    invites: Invites,
    session_lifetime: i64,
    accounts_per_day: u32,
    minimum_age: u32,
    terms_version: u32,
    recipient: Option<Recipient>,
    required: bool,
    states: LoginStates,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn")
            .field("rp", &self.rp)
            .field("origins", &self.origins)
            .field("enrolment", &self.enrolment)
            .finish_non_exhaustive()
    }
}

impl SignIn {
    pub(crate) fn new(setup: SignInSetup) -> Self {
        SignIn {
            store: setup.store,
            keys: setup.keys,
            rp: setup.rp,
            origins: setup.origins,
            enrolment: setup.enrolment,
            invites: setup.invites,
            session_lifetime: i64::try_from(setup.session_lifetime.as_millis()).unwrap_or(i64::MAX),
            accounts_per_day: setup.accounts_per_day,
            minimum_age: setup.minimum_age,
            terms_version: setup.terms_version,
            recipient: setup.recipient,
            required: setup.required,
            states: LoginStates::new(),
        }
    }

    /// The relying party, for the portal's own login.
    pub(crate) fn rp(&self) -> &Rp {
        &self.rp
    }

    /// The identity key ring, for the portal's allowlist check.
    pub(crate) fn keys(&self) -> &KeyRing {
        &self.keys
    }

    /// The directory the sandbox must keep readable for the invite file.
    pub(crate) fn invite_dir(&self) -> Option<std::path::PathBuf> {
        self.invites
            .file
            .as_ref()
            .map(crate::admin::allowlist::KeyFile::dir)
    }

    /// True unless enrolment is by invitation and `key` is on neither invite list.
    fn invited(&self, key: &IdentityKey) -> bool {
        self.enrolment != EnrolmentMode::Invite || self.invites.lists(key)
    }

    pub(crate) fn has_providers(&self) -> bool {
        !self.rp.providers().is_empty()
    }

    /// The identity behind a WebSocket's `?session=` token, or `None` when the token is dead.
    pub(crate) fn bind(&self, token: &str) -> Result<Option<Binding>, StoreError> {
        let token_hash = keys::token_hash(token);
        let identity = store::session_identity(&self.store, &token_hash, dates::now_ms())?;
        Ok(identity.map(|identity| Binding {
            identity,
            token_hash,
        }))
    }

    /// A Login on a bound line opens only an account the identity owns, while the token lives.
    pub(crate) fn may_log_in(&self, binding: &Binding, account: AccountId) -> bool {
        let now = dates::now_ms();
        let check = || -> Result<bool, StoreError> {
            let live = store::session_identity(&self.store, &binding.token_hash, now)?;
            if live != Some(binding.identity) {
                return Ok(false);
            }
            store::owns(&self.store, &binding.identity, account)
        };
        check().unwrap_or_else(|error| {
            warn!(%error, "the sign-in store failed; the Login is refused");
            crate::metrics::counters().store_error();
            false
        })
    }

    /// Notes which account the session's line is playing, for the session purge and holds.
    pub(crate) fn note_login(&self, binding: &Binding, account: AccountId) {
        if let Err(error) = store::bind_session_account(&self.store, &binding.token_hash, account) {
            warn!(%error, "the session was not linked to its account");
        }
    }

    /// The sealed `state` values of every login in flight, the portal's included.
    pub(crate) fn states(&self) -> &LoginStates {
        &self.states
    }

    /// The sealed `issuer`/`subject` pair, when an audit recipient is configured.
    fn seal_subject(&self, provider: &str, issuer: &str, subject: &str) -> Option<Vec<u8>> {
        let recipient = self.recipient.as_ref()?;
        let record = json!({"provider": provider, "issuer": issuer, "subject": subject});
        match seal::seal(recipient, record.to_string().as_bytes()) {
            Ok(sealed) => Some(sealed),
            Err(error) => {
                warn!(%error, "the identity's subject was not sealed");
                None
            }
        }
    }
}
