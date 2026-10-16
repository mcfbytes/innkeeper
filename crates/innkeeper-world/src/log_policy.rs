//! How much of a session the host may write down. Public mode is docs/server/privacy.md.

use std::fmt;

/// Verbose is the development default and must keep today's log format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogPolicy {
    #[default]
    Verbose,
    /// No message fields, names or addresses: a public host (docs/server/privacy.md).
    Public,
}

impl LogPolicy {
    pub fn is_public(self) -> bool {
        matches!(self, Self::Public)
    }

    /// `Debug` of `value` when verbose; only the type or variant name when public.
    pub fn debug<T: fmt::Debug>(self, value: &T) -> LoggedDebug<'_, T> {
        LoggedDebug {
            policy: self,
            value,
        }
    }

    /// The persona string when verbose; the account number when public.
    pub fn persona(self, account: u32, name: &str) -> LoggedPersona<'_> {
        LoggedPersona {
            policy: self,
            account,
            name,
        }
    }

    /// A span that names the player by persona, or by account number when public.
    pub fn player_span(self, account: u32, persona: &str) -> tracing::Span {
        match self {
            Self::Verbose => tracing::info_span!("player", persona = %persona),
            Self::Public => tracing::info_span!("player", account),
        }
    }
}

/// `Debug` or a kind token, chosen by [`LogPolicy`].
#[derive(Debug)]
pub struct LoggedDebug<'a, T: fmt::Debug> {
    policy: LogPolicy,
    value: &'a T,
}

impl<T: fmt::Debug> fmt::Display for LoggedDebug<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.policy {
            LogPolicy::Verbose => write!(f, "{:?}", self.value),
            LogPolicy::Public => f.write_str(debug_kind(&format!("{:?}", self.value))),
        }
    }
}

/// A persona name, or the account number that owns it.
#[derive(Debug)]
pub struct LoggedPersona<'a> {
    policy: LogPolicy,
    account: u32,
    name: &'a str,
}

impl fmt::Display for LoggedPersona<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.policy {
            LogPolicy::Verbose => f.write_str(self.name),
            LogPolicy::Public => write!(f, "{}", self.account),
        }
    }
}

/// The identifier `Debug` prints before fields: `Login(...)` and `Foo { .. }` become `Login` / `Foo`.
fn debug_kind(debug: &str) -> &str {
    let end = debug.find(['{', '(', ' ', ',']).unwrap_or(debug.len());
    debug.get(..end).unwrap_or(debug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountId, ClientMessage, ClientVersion, EncodedPassword, LandType, Login};

    fn login() -> Login {
        Login {
            land_type: LandType(1),
            version: ClientVersion::new(2, 3, 18),
            account: AccountId(100_001),
            password_source: crate::PasswordSource::StoredFile,
            password: EncodedPassword([0x12, 0xa1, 0x86, 0x01, 0x00, 0x01, 0x1d, 0x7a, 0x01, 0x66]),
            name: "guybrush".into(),
            prodigy_id: None,
        }
    }

    #[test]
    fn public_debug_of_a_login_is_the_kind() {
        let message = ClientMessage::Login(login());
        let public = format!("{}", LogPolicy::Public.debug(&message));
        assert_eq!(public, "Login");
        assert!(!public.contains("guybrush"));
        assert!(!public.contains("12a1"));
        let verbose = format!("{}", LogPolicy::Verbose.debug(&message));
        assert!(verbose.contains("guybrush"));
        assert!(verbose.starts_with("Login"));
    }

    #[test]
    fn public_persona_is_the_account_number() {
        assert_eq!(
            format!("{}", LogPolicy::Public.persona(100_001, "guybrush")),
            "100001"
        );
        assert_eq!(
            format!("{}", LogPolicy::Verbose.persona(100_001, "guybrush")),
            "guybrush"
        );
    }
}
