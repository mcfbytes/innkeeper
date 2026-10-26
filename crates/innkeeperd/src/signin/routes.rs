//! The `/auth/*` endpoints of the page contract, with CORS for the page origins.

use std::collections::HashMap;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use hyper::body::Incoming;
use hyper::header::{
    HeaderValue, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE, AUTHORIZATION, ORIGIN, VARY,
};
use hyper::{Method, Request, StatusCode};
use innkeeper_world::saves::{canonical_path, SaveMeta};
use innkeeper_world::{AccountId, AccountRecord, Store};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::keys::{self, IdentityKey};
use super::pages::Outcome;
use super::rp::{Audience, PendingRp};
use super::state::LinkSeal;
use super::store::{self, Attested, Created};
use super::{Mode, PageReturn, SignIn};
use crate::dates;
use crate::http::{self, Reply};

use crate::dates::DAY;
const MAX_PERSONA: usize = 10;
/// The longest `return` address taken; it travels inside the sealed `state` to the provider.
const MAX_RETURN: usize = 1024;
/// The most file bytes the saves listing carries inline; later files are fetched one by one.
const PREFETCH_INLINE_BYTES: usize = 2 * 1024 * 1024;

impl SignIn {
    /// Answers one request under `/auth/`.
    pub(crate) async fn serve(self: Arc<Self>, request: Request<Incoming>) -> Reply {
        let origin = request
            .headers()
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let query = query_map(request.uri().query());
        match (&method, path.as_str()) {
            (&Method::GET, "/auth/login") => return self.login(&query).await,
            (&Method::GET, "/auth/callback") => return self.callback(&query).await,
            _ => {}
        }
        let session = self.bearer(&request);
        let identity = session.map(|session| session.identity);
        let reply = match (method, path.as_str()) {
            (Method::OPTIONS, _) => http::empty(StatusCode::NO_CONTENT),
            (Method::POST, "/auth/identities/ticket") => match session {
                Some(session) => self.link_ticket(&session),
                None => unauthorized(),
            },
            (Method::POST, "/auth/identities") => match session {
                Some(session) => self.redeem_link(&session, http::read_json(request).await),
                None => unauthorized(),
            },
            (Method::POST, "/auth/identities/unlink") => match session {
                Some(session) => self.unlink(&session, http::read_json(request).await),
                None => unauthorized(),
            },
            (Method::GET, other) if saves_target(other).is_some() => {
                match (identity, saves_target(other)) {
                    (Some(key), Some(account)) => self.saves(&key, account, &query),
                    _ => unauthorized(),
                }
            }
            (Method::GET, "/auth/providers") => self.providers(),
            (Method::GET, "/auth/session") => self.session(identity),
            (Method::POST, "/auth/logout") => self.logout(&request),
            (Method::POST, "/auth/attest") => match identity {
                Some(key) => self.attest(&key, http::read_json(request).await),
                None => unauthorized(),
            },
            (Method::POST, "/auth/accounts") => match identity {
                Some(key) => self.create_account(&key, http::read_json(request).await),
                None => unauthorized(),
            },
            (Method::POST, other) => match credential_target(other) {
                Some(account) => match identity {
                    Some(key) => self.credential(&key, account),
                    None => unauthorized(),
                },
                None => not_found(),
            },
            _ => not_found(),
        };
        self.with_cors(reply, origin.as_deref())
    }

    fn with_cors(&self, mut reply: Reply, origin: Option<&str>) -> Reply {
        let headers = reply.headers_mut();
        headers.insert(VARY, HeaderValue::from_static("Origin"));
        let Some(allowed) = origin.and_then(|origin| self.origins.allowed(origin)) else {
            return reply;
        };
        if let Ok(value) = HeaderValue::from_str(allowed) {
            headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, value);
            headers.insert(
                ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("GET, POST, OPTIONS"),
            );
            headers.insert(
                ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("Authorization, Content-Type"),
            );
            headers.insert(ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        }
        reply
    }

    fn bearer(&self, request: &Request<Incoming>) -> Option<Session> {
        let header = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
        let token = header.strip_prefix("Bearer ")?.trim();
        let hash = keys::token_hash(token);
        match store::session_row(&self.store, &hash, dates::now_ms()) {
            Ok(found) => found.map(|(identity, created_at)| Session {
                identity,
                token_hash: hash,
                created_at,
            }),
            Err(error) => {
                warn!(%error, "the sign-in store failed");
                crate::metrics::counters().store_error();
                None
            }
        }
    }

    fn providers(&self) -> Reply {
        let list: Vec<Value> = self
            .rp
            .providers()
            .iter()
            .map(|provider| {
                json!({
                    "id": provider.id,
                    "name": provider.name,
                    "url": format!("/auth/login?provider={}", provider.id),
                })
            })
            .collect();
        let reply = json!({ "signInRequired": self.required, "providers": list });
        http::json_reply(StatusCode::OK, &reply)
    }

    async fn login(&self, query: &HashMap<String, String>) -> Reply {
        let provider = query.get("provider").map(String::as_str).unwrap_or("");
        if self.rp.provider(provider).is_none() {
            return http::error(StatusCode::BAD_REQUEST, "bad_provider", "Unknown provider.");
        }
        let address = query.get("return").map(String::as_str).unwrap_or("");
        let checked = (address.len() <= MAX_RETURN)
            .then(|| self.origins.check_return(address))
            .flatten();
        let Some((url, origin)) = checked else {
            return http::error(
                StatusCode::BAD_REQUEST,
                "bad_return",
                "The return address is not an allowed page origin.",
            );
        };
        let mode = match query.get("mode").map(String::as_str) {
            Some("redirect") => Mode::Redirect,
            _ => Mode::Popup,
        };
        let attest = query.get("attest").and_then(|text| parse_attest(text));
        let page_nonce = match query.get("nonce").map(String::as_str) {
            None | Some("") if self.required => {
                return http::error(
                    StatusCode::BAD_REQUEST,
                    "bad_nonce",
                    "This server needs the page's nonce on every login.",
                )
            }
            None | Some("") => None,
            Some(text) if page_nonce_ok(text) => Some(text.to_owned()),
            Some(_) => {
                return http::error(
                    StatusCode::BAD_REQUEST,
                    "bad_nonce",
                    "The nonce must be 16 to 64 of A-Z, a-z, 0-9, - and _.",
                )
            }
        };
        let page = PageReturn {
            url,
            origin,
            mode,
            attest,
            page_nonce,
        };
        let now = dates::now_ms();
        let link = match query.get("link") {
            None => None,
            Some(ticket) => match self.open_ticket(ticket, now) {
                Some(link) => Some(link),
                None => return super::pages::finish(&page, Outcome::Error("link_expired")),
            },
        };
        let expires_at = now.saturating_add(super::LOGIN_TTL);
        let seal = |pending: &PendingRp| {
            let mut state = pending.to_state(expires_at, Some(page.to_state()));
            state.link.clone_from(&link);
            self.states().seal(&state)
        };
        // Linking wants a second account, which the provider would otherwise quietly reuse.
        match self
            .rp
            .begin(provider, Audience::Player, link.is_some(), seal)
            .await
        {
            Ok(authorize) => super::pages::redirect(authorize.as_str()),
            Err(error) => {
                warn!(provider, %error, "sign-in could not start");
                super::pages::finish(&page, Outcome::Error("server_error"))
            }
        }
    }

    async fn callback(&self, query: &HashMap<String, String>) -> Reply {
        let state = query.get("state").map(String::as_str).unwrap_or("");
        let opened = self.states().open(state, Audience::Player, dates::now_ms());
        let Some(login) = opened.and_then(|state| {
            Some(Login {
                page: PageReturn::from_state(state.page.as_ref()?)?,
                rp: PendingRp::from_state(&state),
                link: state.link,
            })
        }) else {
            return super::pages::expired();
        };
        if let Some(code) = query.get("error") {
            let code = if code == "access_denied" {
                "access_denied"
            } else {
                "provider_error"
            };
            return super::pages::finish(&login.page, Outcome::Error(code));
        }
        let Some(code) = query.get("code").cloned() else {
            return super::pages::finish(&login.page, Outcome::Error("provider_error"));
        };
        let verified = match self.rp.finish(login.rp, code).await {
            Ok(verified) => verified,
            Err(error) => {
                warn!(%error, "sign-in did not finish");
                return super::pages::finish(&login.page, Outcome::Error("provider_error"));
            }
        };
        if let Some(link) = &login.link {
            return self.finish_link(link, &verified, &login.page);
        }
        match self.open_session(&verified, login.page.attest) {
            Ok(token) => super::pages::finish(&login.page, Outcome::Session(&token)),
            Err(error) => {
                warn!(%error, "the sign-in store failed");
                crate::metrics::counters().store_error();
                super::pages::finish(&login.page, Outcome::Error("server_error"))
            }
        }
    }

    /// The provider's identity as a row (made on first sign-in), with the page's age and terms
    /// answer when the login carried one.
    pub(super) fn signed_in(
        &self,
        verified: &super::rp::Verified,
        attest: Option<(u32, u32)>,
        now: i64,
    ) -> Result<store::SignedIn, innkeeper_world::StoreError> {
        let signed = store::sign_in(
            &self.store,
            &self.keys,
            &verified.issuer,
            &verified.subject,
            &verified.provider,
            || self.seal_subject(&verified.provider, &verified.issuer, &verified.subject),
            now,
        )?;
        if let Some((age, terms)) = attest {
            store::record_attestation(
                &self.store,
                &signed.key,
                &Attested {
                    age,
                    terms,
                    at: now,
                },
            )?;
        }
        Ok(signed)
    }

    fn open_session(
        &self,
        verified: &super::rp::Verified,
        attest: Option<(u32, u32)>,
    ) -> Result<zeroize::Zeroizing<String>, innkeeper_world::StoreError> {
        let now = dates::now_ms();
        let signed = self.signed_in(verified, attest, now)?;
        let token = keys::new_token();
        let expiry = now.saturating_add(self.session_lifetime);
        store::create_session(
            &self.store,
            &signed.key,
            &keys::token_hash(&token),
            now,
            expiry,
        )?;
        crate::metrics::counters().signin();
        info!(provider = %verified.provider, new_identity = signed.created, "signed in");
        Ok(token)
    }

    fn session(&self, identity: Option<IdentityKey>) -> Reply {
        let Some(key) = identity else {
            return unauthorized();
        };
        let answer = || -> Result<Value, innkeeper_world::StoreError> {
            let row = store::identity(&self.store, &key)?;
            let accounts = store::owned_accounts(&self.store, &key)?;
            let accounts: Vec<Value> = accounts
                .into_iter()
                .map(|(id, persona)| json!({"id": id, "persona": named(&persona)}))
                .collect();
            let provider = row
                .as_ref()
                .map(|row| row.provider.clone())
                .unwrap_or_default();
            let attested = row
                .and_then(|row| row.attested)
                .map(|at| json!({"age": at.age, "terms": at.terms}));
            let mut identity = json!({"handle": keys::handle(&key), "provider": provider});
            if let (true, Some(fields)) = (
                self.enrolment == super::EnrolmentMode::Invite,
                identity.as_object_mut(),
            ) {
                fields.insert("key".to_owned(), json!(crate::admin::allowlist::hex(&key)));
            }
            Ok(json!({
                "identity": identity,
                "identities": self.methods_json(&key)?,
                "accounts": accounts,
                "canCreate": self.can_create(&key)?,
                "attested": attested,
            }))
        };
        match answer() {
            Ok(value) => http::json_reply(StatusCode::OK, &value),
            Err(error) => server_error(&error),
        }
    }

    fn can_create(&self, key: &IdentityKey) -> Result<bool, innkeeper_world::StoreError> {
        if !self.enrolment.web_creates()
            || !self.invited(key)?
            || store::owns_suspended(&self.store, key)?
        {
            return Ok(false);
        }
        let since = dates::now_ms().saturating_sub(DAY);
        Ok(store::accounts_created_since(&self.store, key, since)? < self.accounts_per_day)
    }

    fn attest(&self, key: &IdentityKey, body: Option<Value>) -> Reply {
        let parsed = body
            .as_ref()
            .and_then(|body| body.get("attest"))
            .and_then(Value::as_str)
            .and_then(parse_attest);
        let Some((age, terms)) = parsed else {
            return http::error(
                StatusCode::BAD_REQUEST,
                "bad_attest",
                "Use <age>+v<terms>, for example 18+v1.",
            );
        };
        let at = dates::now_ms();
        match store::record_attestation(&self.store, key, &Attested { age, terms, at }) {
            Ok(()) => http::empty(StatusCode::NO_CONTENT),
            Err(error) => server_error(&error),
        }
    }

    /// The refusal for an identity whose stored attestation is missing or too old, else `None`.
    fn attestation_gate(&self, key: &IdentityKey) -> Option<Reply> {
        match self.attested_enough(key) {
            Ok(true) => return None,
            Ok(false) => {}
            Err(error) => return Some(server_error(&error)),
        }
        let message = format!(
            "The desk needs your word first: you are {} or older, and you accept terms version {} \
             or later. Say so, then ask again.",
            self.minimum_age, self.terms_version
        );
        Some(http::error(
            StatusCode::FORBIDDEN,
            "attest_required",
            &message,
        ))
    }

    /// The refusal for an identity that owns a suspended account, else `None`.
    fn suspension_gate(&self, key: &IdentityKey) -> Option<Reply> {
        match store::owns_suspended(&self.store, key) {
            Ok(false) => None,
            Ok(true) => {
                info!(handle = %keys::handle(key), "refused: the identity owns a suspended account");
                Some(http::error(
                    StatusCode::FORBIDDEN,
                    "suspended",
                    "An account of this sign-in is suspended. Write to Member Services.",
                ))
            }
            Err(error) => Some(server_error(&error)),
        }
    }

    fn create_account(&self, key: &IdentityKey, body: Option<Value>) -> Reply {
        if !self.enrolment.web_creates() {
            return http::error(
                StatusCode::FORBIDDEN,
                "enrolment_closed",
                "New accounts are not being taken.",
            );
        }
        match self.invited(key) {
            Ok(true) => {}
            Ok(false) => {
                info!(handle = %keys::handle(key), "account refused: not on the invite list");
                return not_invited(key);
            }
            Err(error) => return server_error(&error),
        }
        if let Some(refusal) = self
            .attestation_gate(key)
            .or_else(|| self.suspension_gate(key))
        {
            return refusal;
        }
        // No body, `{}` or a null persona opens an account with no persona yet: the client's own
        // Select Player screen names one at its first Login (40/4). A given persona is checked.
        let given = body.as_ref().and_then(|body| body.get("persona"));
        let persona = match given {
            None | Some(Value::Null) => None,
            Some(value) => match value.as_str().filter(|persona| persona_ok(persona)) {
                Some(persona) => Some(persona),
                None => {
                    return http::error(
                        StatusCode::BAD_REQUEST,
                        "bad_persona",
                        "Use 1 to 10 letters or digits, and not a staff name.",
                    )
                }
            },
        };
        match self.can_create(key) {
            Ok(true) => {}
            Ok(false) => {
                return http::error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "This sign-in has opened enough lines for today.",
                )
            }
            Err(error) => return server_error(&error),
        }
        // Nobody learns this password; the credential call issues the one the page writes.
        let unknown = keys::encode_password(&keys::new_password());
        let Some(unknown) = unknown else {
            return http::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "Try again.",
            );
        };
        let record = AccountRecord::new(unknown, persona.unwrap_or_default());
        match store::create_owned_account(&self.store, key, &record, dates::now_ms()) {
            Ok(Created::Account(id)) => {
                crate::metrics::counters().account_created();
                info!(account = id.0, "account created through sign-in");
                http::json_reply(
                    StatusCode::CREATED,
                    &json!({"id": id.0, "persona": persona}),
                )
            }
            Ok(Created::PersonaTaken) => http::error(
                StatusCode::CONFLICT,
                "persona_taken",
                &format!(
                    "The name {} is already in use.",
                    persona.unwrap_or_default()
                ),
            ),
            Err(error) => server_error(&error),
        }
    }

    fn credential(&self, key: &IdentityKey, account: AccountId) -> Reply {
        match store::owns(&self.store, key, account) {
            Ok(true) => {}
            Ok(false) => {
                return http::error(
                    StatusCode::NOT_FOUND,
                    "no_such_account",
                    "That account does not belong to this sign-in.",
                )
            }
            Err(error) => return server_error(&error),
        }
        if let Some(refusal) = self
            .attestation_gate(key)
            .or_else(|| self.suspension_gate(key))
        {
            return refusal;
        }
        let password = keys::new_password();
        let Some(encoded) = keys::encode_password(&password) else {
            return http::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "Try again.",
            );
        };
        match self.store.update_password(account, encoded) {
            Ok(()) => {
                info!(account = account.0, "credential issued");
                http::json_reply(
                    StatusCode::OK,
                    &json!({"account": account.0, "password": password.as_str()}),
                )
            }
            Err(error) => server_error(&error),
        }
    }

    /// The files the save channel would hand `account` at logon, for a page that has not dialled
    /// yet (docs/server/saves.md section 8). Without `path` the answer lists every current file
    /// and carries the bytes of as many as fit under `PREFETCH_INLINE_BYTES`; with `path` it is
    /// that one file, always with its bytes. Only counts and sizes are logged.
    fn saves(
        &self,
        key: &IdentityKey,
        account: AccountId,
        query: &HashMap<String, String>,
    ) -> Reply {
        match store::owns(&self.store, key, account) {
            Ok(true) => {}
            Ok(false) => {
                return http::error(
                    StatusCode::NOT_FOUND,
                    "no_such_account",
                    "That account does not belong to this sign-in.",
                )
            }
            Err(error) => return server_error(&error),
        }
        if let Some(raw) = query.get("path") {
            return self.one_save(account, raw);
        }
        let metas = match self.store.saves(account) {
            Ok(metas) => metas,
            Err(error) => return server_error(&error),
        };
        let mut inline = 0usize;
        let mut files = Vec::with_capacity(metas.len());
        for meta in &metas {
            let mut entry = save_entry(meta);
            if inline + meta.size <= PREFETCH_INLINE_BYTES {
                match self.store.save_read(account, &meta.path, None) {
                    Ok(Some((_, data))) => {
                        inline += data.len();
                        entry.insert("data".to_owned(), json!(BASE64.encode(data)));
                    }
                    Ok(None) => {}
                    Err(error) => return server_error(&error),
                }
            }
            files.push(Value::Object(entry));
        }
        info!(
            account = account.0,
            files = files.len(),
            inline_bytes = inline,
            "saves prefetched"
        );
        http::json_reply(
            StatusCode::OK,
            &json!({"account": account.0, "files": files}),
        )
    }

    fn one_save(&self, account: AccountId, raw: &str) -> Reply {
        let Ok(path) = canonical_path(raw) else {
            return http::error(StatusCode::BAD_REQUEST, "bad_path", "Not a save file path.");
        };
        match self.store.save_read(account, &path, None) {
            Ok(Some((meta, data))) => {
                info!(
                    account = account.0,
                    size = data.len(),
                    "one save prefetched"
                );
                let mut entry = save_entry(&meta);
                entry.insert("data".to_owned(), json!(BASE64.encode(data)));
                http::json_reply(StatusCode::OK, &Value::Object(entry))
            }
            Ok(None) => http::error(StatusCode::NOT_FOUND, "no_such_file", "No such save file."),
            Err(error) => server_error(&error),
        }
    }

    fn logout(&self, request: &Request<Incoming>) -> Reply {
        let token = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|header| header.strip_prefix("Bearer "));
        if let Some(token) = token {
            let hash = keys::token_hash(token.trim());
            if let Err(error) = store::revoke_session(&self.store, &hash, dates::now_ms()) {
                return server_error(&error);
            }
        }
        http::empty(StatusCode::NO_CONTENT)
    }
}

/// A login back from the provider, opened from its sealed `state`.
struct Login {
    rp: PendingRp,
    page: PageReturn,
    link: Option<LinkSeal>,
}

/// The live session behind a bearer token.
#[derive(Clone, Copy, Debug)]
pub(super) struct Session {
    pub(super) identity: IdentityKey,
    pub(super) token_hash: [u8; 32],
    /// When its sign-in happened: changing sign-in methods needs a recent one.
    pub(super) created_at: i64,
}

/// The page's login nonce: 16 to 64 base64url characters, so it is echoed back unchanged.
fn page_nonce_ok(text: &str) -> bool {
    (16..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `<age>+v<terms>`, for example `18+v1`.
pub(crate) fn parse_attest(text: &str) -> Option<(u32, u32)> {
    let (age, terms) = text.split_once("+v")?;
    let digits = |part: &str, max: usize| {
        (!part.is_empty() && part.len() <= max && part.bytes().all(|b| b.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    Some((digits(age, 3)?, digits(terms, 6)?))
}

/// An account's persona for JSON: null until the client has named one.
fn named(persona: &str) -> Option<&str> {
    (!persona.trim().is_empty()).then_some(persona)
}

/// 1 to 10 of A-Z, a-z and 0-9 and no staff or service name, the rule for a persona given when
/// the account is opened.
pub(crate) fn persona_ok(persona: &str) -> bool {
    !persona.is_empty()
        && persona.len() <= MAX_PERSONA
        && persona.bytes().all(|b| b.is_ascii_alphanumeric())
        && !innkeeper_world::is_reserved_name(persona)
}

/// `/auth/accounts/<id>/saves`.
fn saves_target(path: &str) -> Option<AccountId> {
    let number = path
        .strip_prefix("/auth/accounts/")?
        .strip_suffix("/saves")?;
    account_number(number)
}

fn save_entry(meta: &SaveMeta) -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "path": meta.path,
        "version": meta.version,
        "hash": meta.hash,
        "size": meta.size,
        "mtime": meta.modified,
        "current": meta.current,
    }) else {
        return serde_json::Map::new();
    };
    map
}

fn credential_target(path: &str) -> Option<AccountId> {
    let number = path
        .strip_prefix("/auth/accounts/")?
        .strip_suffix("/credential")?;
    account_number(number)
}

fn account_number(number: &str) -> Option<AccountId> {
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number
        .parse::<u32>()
        .ok()
        .filter(|id| *id > 0)
        .map(AccountId)
}

fn query_map(query: Option<&str>) -> HashMap<String, String> {
    let pairs = url::form_urlencoded::parse(query.unwrap_or("").as_bytes());
    let mut map = HashMap::new();
    for (key, value) in pairs {
        map.entry(key.into_owned())
            .or_insert_with(|| value.into_owned());
    }
    map
}

pub(super) fn unauthorized() -> Reply {
    http::error(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "The session is missing or has expired.",
    )
}

/// The `enrolment_closed` answer the page already shows, with the identity's own key to send the
/// operator, as the portal's refusal page does. The key is not a secret, and only its holder sees it.
fn not_invited(key: &IdentityKey) -> Reply {
    let hex = crate::admin::allowlist::hex(key);
    let message = format!(
        "New accounts are by invitation only. To be invited, send the operator your identity key: {hex}"
    );
    http::json_reply(
        StatusCode::FORBIDDEN,
        &json!({"error": "enrolment_closed", "message": message, "identityKey": hex}),
    )
}

fn not_found() -> Reply {
    http::error(StatusCode::NOT_FOUND, "not_found", "No such endpoint.")
}

pub(super) fn server_error(error: &innkeeper_world::StoreError) -> Reply {
    warn!(%error, "the sign-in store failed");
    crate::metrics::counters().store_error();
    http::error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "The server could not answer. Try again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attest_is_age_plus_v_terms() {
        assert_eq!(parse_attest("18+v1"), Some((18, 1)));
        assert_eq!(parse_attest("21+v123456"), Some((21, 123_456)));
        for bad in [
            "18",
            "18+1",
            "+v1",
            "18+v",
            "1800+v1",
            "18+v1234567",
            "-1+v1",
            "18+v1 ",
        ] {
            assert_eq!(parse_attest(bad), None, "{bad}");
        }
    }

    #[test]
    fn personas_follow_the_page_rule() {
        for good in ["Guybrush", "A", "Abcdefghij", "R2D2"] {
            assert!(persona_ok(good), "{good}");
        }
        for bad in [
            "",
            "Abcdefghijk",
            "two words",
            "a-b",
            "é",
            "Sysop",
            "TSNstaff",
        ] {
            assert!(!persona_ok(bad), "{bad}");
        }
    }

    #[test]
    fn the_saves_path_names_one_account() {
        assert_eq!(
            saves_target("/auth/accounts/100002/saves"),
            Some(AccountId(100_002))
        );
        assert_eq!(saves_target("/auth/accounts/0/saves"), None);
        assert_eq!(saves_target("/auth/accounts/x/saves"), None);
        assert_eq!(saves_target("/auth/accounts/1/saves/INN"), None);
    }

    #[test]
    fn the_credential_path_names_one_account() {
        assert_eq!(
            credential_target("/auth/accounts/100002/credential"),
            Some(AccountId(100_002))
        );
        assert_eq!(credential_target("/auth/accounts/0/credential"), None);
        assert_eq!(credential_target("/auth/accounts/+1/credential"), None);
        assert_eq!(
            credential_target("/auth/accounts/99999999999/credential"),
            None
        );
        assert_eq!(credential_target("/auth/accounts/1/credential/x"), None);
    }
}
