//! Linking sign-in methods: a ticket from a recent session, a login that proves the other
//! identity, and the proof redeemed by that session (docs/server/innkeeperd.md section 8.7).

use hyper::StatusCode;
use innkeeper_world::StoreError;
use serde_json::{json, Value};
use tracing::{info, warn};

use super::groups::{self, Linked, Unlinked};
use super::keys::{self, IdentityKey};
use super::pages::{self, Outcome};
use super::routes::{server_error, Session};
use super::rp::Verified;
use super::state::{LinkSeal, Purpose};
use super::{store, PageReturn, SignIn, LOGIN_TTL, RECENT_SIGN_IN};
use crate::dates;
use crate::http::{self, Reply};

impl SignIn {
    /// `POST /auth/identities/ticket`: leave for one link login, bound to this session.
    pub(super) fn link_ticket(&self, session: &Session) -> Reply {
        let now = dates::now_ms();
        if let Some(refusal) = recent_gate(session, now) {
            return refusal;
        }
        match groups::methods(&self.store, &session.identity) {
            Ok(methods) if methods.len() >= groups::MAX_GROUP_IDENTITIES => return too_many(),
            Ok(_) => {}
            Err(error) => return server_error(&error),
        }
        let ticket = LinkSeal {
            token_hash: session.token_hash,
            identity: session.identity,
            proven: None,
            expires_at: now.saturating_add(LOGIN_TTL),
        };
        match self.states().seal_link(Purpose::Ticket, &ticket) {
            Some(text) => http::json_reply(
                StatusCode::OK,
                &json!({"ticket": text, "expiresIn": LOGIN_TTL / dates::SECOND}),
            ),
            None => http::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "Try again.",
            ),
        }
    }

    /// The ticket on a link login, once, while the session that took it is still live.
    pub(super) fn open_ticket(&self, text: &str, now: i64) -> Option<LinkSeal> {
        let ticket = self.states().open_link(text, Purpose::Ticket, now)?;
        self.session_lives(&ticket, now).then_some(ticket)
    }

    fn session_lives(&self, link: &LinkSeal, now: i64) -> bool {
        match store::session_identity(&self.store, &link.token_hash, now) {
            Ok(identity) => identity == Some(link.identity),
            Err(error) => {
                warn!(%error, "the sign-in store failed");
                false
            }
        }
    }

    /// The callback of a link login: no session for the proven identity, only a proof that the
    /// asking session can redeem. Two groups that both own accounts are refused here already.
    pub(super) fn finish_link(
        &self,
        link: &LinkSeal,
        verified: &Verified,
        page: &PageReturn,
    ) -> Reply {
        let now = dates::now_ms();
        if !self.session_lives(link, now) {
            return pages::finish(page, Outcome::Error("link_expired"));
        }
        let checked = self
            .signed_in(verified, page.attest, now)
            .and_then(|signed| {
                let plan = groups::link_check(&self.store, &link.identity, &signed.key)?;
                Ok((signed.key, plan))
            });
        let code = match checked {
            Ok((proven, Linked::Joined)) => {
                let proof = LinkSeal {
                    proven: Some(proven),
                    expires_at: now.saturating_add(LOGIN_TTL),
                    ..link.clone()
                };
                info!(provider = %verified.provider, "a sign-in method was proven for a link");
                match self.states().seal_link(Purpose::Proof, &proof) {
                    Some(text) => return pages::finish(page, Outcome::Link(&text)),
                    None => "server_error",
                }
            }
            Ok((_, refused)) => refusal_code(refused),
            Err(error) => {
                warn!(%error, "the sign-in store failed");
                crate::metrics::counters().store_error();
                "server_error"
            }
        };
        info!(provider = %verified.provider, code, "a link login was refused");
        pages::finish(page, Outcome::Error(code))
    }

    /// `POST /auth/identities` `{"proof"}`: links the proven identity, for the session that asked only.
    pub(super) fn redeem_link(&self, session: &Session, body: Option<Value>) -> Reply {
        let now = dates::now_ms();
        let text = body
            .as_ref()
            .and_then(|body| body.get("proof"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let opened = self.states().open_link(text, Purpose::Proof, now);
        let ours = opened.filter(|proof| {
            proof.token_hash == session.token_hash && proof.identity == session.identity
        });
        let Some(proven) = ours.and_then(|proof| proof.proven) else {
            return http::error(
                StatusCode::BAD_REQUEST,
                "bad_proof",
                "This link has expired, was already used or belongs to another sign-in. Start again.",
            );
        };
        match groups::link(&self.store, &session.identity, &proven, now) {
            Ok(Linked::Joined | Linked::Already) => {
                info!(handle = %keys::handle(&session.identity), "a sign-in method was linked");
                self.methods_reply(&session.identity)
            }
            Ok(refused) => refusal_reply(refused),
            Err(error) => server_error(&error),
        }
    }

    /// `POST /auth/identities/unlink` `{"id"}`: another method of this group leaves it.
    pub(super) fn unlink(&self, session: &Session, body: Option<Value>) -> Reply {
        let now = dates::now_ms();
        if let Some(refusal) = recent_gate(session, now) {
            return refusal;
        }
        let id = body
            .as_ref()
            .and_then(|body| body.get("id"))
            .and_then(Value::as_str)
            .filter(|id| id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some(id) = id else {
            return http::error(
                StatusCode::BAD_REQUEST,
                "bad_id",
                "Name a sign-in method by its id.",
            );
        };
        let (status, code, message) = match groups::unlink(&self.store, &session.identity, id, now) {
            Ok(Unlinked::Done) => {
                info!(handle = %keys::handle(&session.identity), "a sign-in method was removed");
                return self.methods_reply(&session.identity);
            }
            Ok(Unlinked::Last) => (
                StatusCode::CONFLICT,
                "last_identity",
                "This is the only way to sign in to these accounts; add another before removing it.",
            ),
            Ok(Unlinked::Current) => (
                StatusCode::CONFLICT,
                "current_identity",
                "You are signed in with this method. Sign in with another one to remove it.",
            ),
            Ok(Unlinked::Suspended) => (
                StatusCode::FORBIDDEN,
                "suspended",
                "An account of this sign-in is suspended. Write to Member Services.",
            ),
            Ok(Unlinked::Missing) => (
                StatusCode::NOT_FOUND,
                "no_such_identity",
                "That sign-in method is not linked here.",
            ),
            Err(error) => return server_error(&error),
        };
        http::error(status, code, message)
    }

    /// The group's sign-in methods for the page: provider and link date, never the subject.
    pub(super) fn methods_json(&self, key: &IdentityKey) -> Result<Value, StoreError> {
        let methods = groups::methods(&self.store, key)?;
        let listed: Vec<Value> = methods
            .iter()
            .map(|method| {
                let name = self
                    .rp
                    .provider(&method.provider)
                    .map(|provider| provider.name.clone());
                json!({
                    "id": method.id(),
                    "provider": method.provider,
                    "name": name,
                    "linkedOn": dates::format_date(method.linked_at),
                    "current": method.key == *key,
                })
            })
            .collect();
        Ok(Value::Array(listed))
    }

    fn methods_reply(&self, key: &IdentityKey) -> Reply {
        match self.methods_json(key) {
            Ok(list) => http::json_reply(StatusCode::OK, &json!({ "identities": list })),
            Err(error) => server_error(&error),
        }
    }
}

/// A session older than `RECENT_SIGN_IN` may not change its sign-in methods, so a stale or stolen
/// token cannot add its holder's provider or strip the owner's.
fn recent_gate(session: &Session, now: i64) -> Option<Reply> {
    if now.saturating_sub(session.created_at) <= RECENT_SIGN_IN {
        return None;
    }
    Some(http::error(
        StatusCode::FORBIDDEN,
        "reauth_required",
        "Sign in again first: changing sign-in methods needs a sign-in from the last ten minutes.",
    ))
}

fn refusal_code(refused: Linked) -> &'static str {
    match refused {
        Linked::InUse => "identity_in_use",
        Linked::Already => "already_linked",
        Linked::TooMany => "too_many_identities",
        Linked::Joined | Linked::Missing => "server_error",
    }
}

fn refusal_reply(refused: Linked) -> Reply {
    match refused {
        Linked::InUse => http::error(
            StatusCode::CONFLICT,
            "identity_in_use",
            "That sign-in already has accounts of its own. Two people's accounts are never merged.",
        ),
        Linked::TooMany => too_many(),
        Linked::Joined | Linked::Already | Linked::Missing => http::error(
            StatusCode::BAD_REQUEST,
            "bad_proof",
            "This link has expired, was already used or belongs to another sign-in. Start again.",
        ),
    }
}

fn too_many() -> Reply {
    http::error(
        StatusCode::CONFLICT,
        "too_many_identities",
        &format!(
            "These accounts already have {} sign-in methods, the most there may be.",
            groups::MAX_GROUP_IDENTITIES
        ),
    )
}
