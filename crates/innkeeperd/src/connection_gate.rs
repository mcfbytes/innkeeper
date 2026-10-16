//! Whether a WebSocket upgrade may go ahead, and which signed-in identity it carries
//! (docs/server/hardening.md section 1).

use std::fmt;

use hyper::Uri;

use crate::connection::ConnectionSettings;
pub(crate) use crate::signin::Binding;

/// Logged once at startup when public mode has nobody to sign players in.
pub(crate) const NO_PROVIDERS: &str = "\
public mode: no sign-in providers are configured (--auth-config), so every WebSocket is refused";

/// Why an upgrade was refused. The token itself is never part of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Public mode needs a session and the request carried none.
    MissingSession,
    /// The token is unknown, expired or revoked.
    DeadSession,
    /// The session store could not be read.
    StoreFailed,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::MissingSession => "no session token",
            Refusal::DeadSession => "the session token is unknown or expired",
            Refusal::StoreFailed => "the session store failed",
        })
    }
}

/// `?session=<token>` on the upgrade. Public mode refuses without a live one; elsewhere a token
/// binds when sign-in is configured and is ignored when it is not.
pub(crate) fn admit(uri: &Uri, settings: &ConnectionSettings) -> Result<Option<Binding>, Refusal> {
    let token = session_token(uri.query());
    let (Some(sign_in), Some(token)) = (settings.sign_in.as_ref(), token) else {
        return match settings.require_session {
            true => Err(Refusal::MissingSession),
            false => Ok(None),
        };
    };
    match sign_in.bind(&token) {
        Ok(Some(binding)) => Ok(Some(binding)),
        Ok(None) => Err(Refusal::DeadSession),
        Err(error) => {
            tracing::warn!(%error, "the session store failed");
            Err(Refusal::StoreFailed)
        }
    }
}

fn session_token(query: Option<&str>) -> Option<String> {
    let pairs = url::form_urlencoded::parse(query?.as_bytes());
    pairs
        .filter(|(key, _)| key == "session")
        .map(|(_, value)| value.into_owned())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_is_read_from_the_query_only() {
        assert_eq!(session_token(Some("session=abc")), Some("abc".into()));
        assert_eq!(session_token(Some("x=1&session=a%2Bb")), Some("a+b".into()));
        assert_eq!(session_token(Some("session=")), None);
        assert_eq!(session_token(Some("sessions=abc")), None);
        assert_eq!(session_token(None), None);
    }

    #[test]
    fn a_refusal_never_names_the_token() {
        for refusal in [
            Refusal::MissingSession,
            Refusal::DeadSession,
            Refusal::StoreFailed,
        ] {
            assert!(!refusal.to_string().contains("session="));
        }
    }
}
