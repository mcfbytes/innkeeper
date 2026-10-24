//! The operator portal: a web office under `/admin/` on its own listener, gated by the same
//! sign-in providers as the players and by a staff allowlist (docs/server/innkeeperd.md section 9).
//!
//! Every route lives below `/admin/`. A request is answered in this order: static files, the
//! sign-in routes, then everything else behind a cookie that names a live portal session whose
//! identity is *still* on the allowlist. A POST also needs a matching `Origin` and the session's
//! CSRF token. A GET never changes anything.

mod actions;
pub(crate) mod allowlist;
pub(crate) mod cli;
mod html;
pub(crate) mod store;
mod views;

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use innkeeper_world::RetentionPolicy;
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tracing::{debug, info, warn};
use url::Url;

use crate::connection::ConnectionSettings;
use crate::dates;
use crate::http::{self, Reply, HEADER_TIMEOUT};
use crate::signin::keys::{self, IdentityKey};
use crate::signin::rp::{Audience, PendingRp};
use crate::signin::SignIn;
use crate::sqlite_store::SqliteStore;
use allowlist::{Allowlist, Decision};
use html::{esc, Ctx, Flash, Nav};
use store::Who;

/// A portal session lasts this long in all, and ends sooner when idle.
const SESSION_LIFETIME: i64 = 8 * dates::HOUR;
const SESSION_IDLE: i64 = 30 * dates::MINUTE;
const MAX_FORM: usize = 64 * 1024;

const COOKIE_SECURE: &str = "__Host-inn-admin";
const COOKIE_PLAIN: &str = "inn-admin";

const CSS: &str = include_str!("assets/admin.css");
const ADMIN_JS: &str = include_str!("assets/admin.js");
const THEME_JS: &str = include_str!("assets/theme.js");
const VIGNETTE_JS: &str = include_str!("assets/vignette.js");

/// Everything the portal needs, gathered at startup.
pub(crate) struct AdminSetup {
    pub(crate) settings: Arc<ConnectionSettings>,
    pub(crate) sign_in: Arc<SignIn>,
    pub(crate) store: Arc<SqliteStore>,
    pub(crate) allowlist: Allowlist,
    /// The portal's public origin: callbacks, the `Origin` check and whether cookies are `Secure`.
    pub(crate) origin: Url,
    pub(crate) retention: RetentionPolicy,
}

pub(crate) struct Admin {
    pub(crate) settings: Arc<ConnectionSettings>,
    sign_in: Arc<SignIn>,
    pub(crate) store: Arc<SqliteStore>,
    allowlist: Allowlist,
    origin: String,
    secure: bool,
    pub(crate) retention: RetentionPolicy,
}

impl std::fmt::Debug for Admin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Admin")
            .field("origin", &self.origin)
            .field("allowlist", &self.allowlist)
            .finish_non_exhaustive()
    }
}

/// One request that passed the cookie, allowlist, origin and CSRF checks.
pub(crate) struct Req {
    pub(crate) ctx: Ctx,
    pub(crate) who: Who,
    /// The form fields of a POST, or the query of a GET; the first value of a name wins.
    pub(crate) params: HashMap<String, String>,
}

impl Req {
    pub(crate) fn param(&self, name: &str) -> &str {
        self.params.get(name).map_or("", String::as_str)
    }

    pub(crate) fn confirmed(&self) -> bool {
        self.param("confirm") == "yes"
    }
}

impl Admin {
    pub(crate) fn new(setup: AdminSetup) -> Self {
        let secure = setup.origin.scheme() == "https";
        Admin {
            settings: setup.settings,
            sign_in: setup.sign_in,
            store: setup.store,
            allowlist: setup.allowlist,
            origin: setup.origin.origin().ascii_serialization(),
            secure,
            retention: setup.retention,
        }
    }

    /// The directory the sandbox must keep readable for the allowlist file.
    pub(crate) fn allowlist_dir(&self) -> Option<std::path::PathBuf> {
        self.allowlist.file_dir()
    }

    pub(crate) fn dev_open(&self) -> bool {
        self.allowlist.is_open(&self.store).unwrap_or(false)
    }

    fn cookie_name(&self) -> &'static str {
        match self.secure {
            true => COOKIE_SECURE,
            false => COOKIE_PLAIN,
        }
    }

    /// `max_age` is in seconds, as `Max-Age` is.
    fn cookie(&self, value: &str, max_age: i64) -> HeaderValue {
        let attributes = match self.secure {
            true => "Path=/; Secure; HttpOnly; SameSite=Strict",
            false => "Path=/admin/; HttpOnly; SameSite=Strict",
        };
        let text = format!(
            "{}={value}; {attributes}; Max-Age={max_age}",
            self.cookie_name()
        );
        HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""))
    }
}

/// Serves one accepted socket of the portal listener.
pub(crate) async fn serve(stream: TcpStream, addr: SocketAddr, admin: Arc<Admin>) {
    if let Err(error) = stream.set_nodelay(true) {
        debug!(%error, "cannot disable Nagle on a portal socket");
    }
    let _ = addr;
    let service = service_fn(move |request| {
        let admin = Arc::clone(&admin);
        async move { Ok::<_, Infallible>(admin.handle(request).await) }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);
    if let Err(error) = builder
        .serve_connection(TokioIo::new(stream), service)
        .await
    {
        debug!(%error, "portal connection ended");
    }
}

/// The headers every portal answer carries.
fn harden(mut reply: Reply) -> Reply {
    let headers = reply.headers_mut();
    let set = |headers: &mut hyper::HeaderMap, name: &'static str, value: &'static str| {
        headers.insert(name, HeaderValue::from_static(value));
    };
    set(
        headers,
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self' https://fonts.googleapis.com; \
         font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; \
         object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
    );
    set(headers, "x-content-type-options", "nosniff");
    set(headers, "referrer-policy", "no-referrer");
    set(headers, "x-frame-options", "DENY");
    set(headers, "cross-origin-opener-policy", "same-origin");
    set(headers, "cross-origin-resource-policy", "same-origin");
    set(headers, "cache-control", "no-store");
    set(headers, "pragma", "no-cache");
    reply
}

pub(crate) fn html_reply(status: StatusCode, body: String) -> Reply {
    http::reply(status, Some("text/html; charset=utf-8"), Bytes::from(body))
}

pub(crate) fn see_other(location: &str) -> Reply {
    let mut reply = http::empty(StatusCode::SEE_OTHER);
    if let Ok(value) = HeaderValue::from_str(location) {
        reply.headers_mut().insert(LOCATION, value);
    }
    reply
}

fn asset(kind: &'static str, text: &'static str) -> Reply {
    let mut reply = http::reply(
        StatusCode::OK,
        Some(kind),
        Bytes::from_static(text.as_bytes()),
    );
    // Fixed bytes: a short cache is safe and saves the pixel-font page a few requests.
    reply.headers_mut().insert(
        "cache-control",
        HeaderValue::from_static("private, max-age=300"),
    );
    reply
}

fn query_map(query: Option<&str>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        map.entry(key.into_owned())
            .or_insert_with(|| value.into_owned());
    }
    map
}

/// The CSRF token of a session: derived from its cookie, never stored.
pub(crate) fn csrf_for(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"inn-admin-csrf\0");
    hasher.update(token.as_bytes());
    BASE64URL.encode(hasher.finalize())
}

fn same_bytes(a: &str, b: &str) -> bool {
    // Constant time for equal lengths; the length of a token is not a secret.
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then_some(value)
    })
}

impl Admin {
    async fn handle(self: Arc<Self>, request: Request<Incoming>) -> Reply {
        harden(self.route(request).await)
    }

    async fn route(&self, request: Request<Incoming>) -> Reply {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let query = query_map(request.uri().query());
        if path == "/admin" {
            return see_other("/admin/");
        }
        if !path.starts_with("/admin/") {
            return http::text(StatusCode::NOT_FOUND, "Not found.");
        }
        if method == Method::GET {
            match path.as_str() {
                "/admin/healthz" => return self.healthz(),
                "/admin/static/admin.css" => return asset("text/css; charset=utf-8", CSS),
                "/admin/static/admin.js" => {
                    return asset("text/javascript; charset=utf-8", ADMIN_JS)
                }
                "/admin/static/theme.js" => {
                    return asset("text/javascript; charset=utf-8", THEME_JS)
                }
                "/admin/static/vignette.js" => {
                    return asset("text/javascript; charset=utf-8", VIGNETTE_JS)
                }
                "/admin/login" => return self.login_page(),
                "/admin/auth/login" => return self.auth_login(&query).await,
                "/admin/auth/callback" => return self.auth_callback(&query).await,
                _ => {}
            }
        }
        if method != Method::GET && method != Method::POST {
            return http::text(StatusCode::METHOD_NOT_ALLOWED, "Not allowed.");
        }
        self.authed(request, method, path, query).await
    }

    fn healthz(&self) -> Reply {
        let alive = store::counts(&self.store, dates::now_ms()).is_ok();
        match alive {
            true => http::text(StatusCode::OK, "ok"),
            false => http::text(StatusCode::SERVICE_UNAVAILABLE, "store unavailable"),
        }
    }

    /// Everything behind the cookie.
    async fn authed(
        &self,
        request: Request<Incoming>,
        method: Method,
        path: String,
        query: HashMap<String, String>,
    ) -> Reply {
        let headers = request.headers().clone();
        let token = headers
            .get(COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|header| cookie_value(header, self.cookie_name()))
            .map(str::to_owned);
        let session = match token.as_deref().map(|token| self.session(token)) {
            Some(Ok(Some(session))) => session,
            Some(Ok(None)) | None => return see_other("/admin/login"),
            Some(Err(error)) => return self.unavailable(&error),
        };
        let (token, who, dev_open) = session;
        let mut req = Req {
            ctx: Ctx {
                handle: who.handle.clone(),
                csrf: csrf_for(&token),
                dev_open,
            },
            who,
            params: query,
        };
        if method == Method::POST {
            if !self.origin_ok(&headers) {
                warn!(handle = %req.who.handle, "portal POST refused: wrong Origin");
                return self.forbidden(&req, "That request did not come from this office.");
            }
            let Some(form) = read_form(request).await else {
                return self.forbidden(&req, "That form was too large or unreadable.");
            };
            let sent = form.get("csrf").map_or("", String::as_str);
            if !same_bytes(sent, &req.ctx.csrf) {
                warn!(handle = %req.who.handle, "portal POST refused: CSRF token");
                return self.forbidden(
                    &req,
                    "The form's stamp does not match. Open the page again.",
                );
            }
            req.params = form;
            return self.post(&req, &path, &token).await;
        }
        self.get(&req, &path)
    }

    /// The session behind a cookie, with the allowlist asked again: revoking a key ends access on
    /// the next request.
    fn session(
        &self,
        token: &str,
    ) -> Result<Option<(String, Who, bool)>, innkeeper_world::StoreError> {
        let hash = keys::token_hash(token);
        let now = dates::now_ms();
        let Some(key) = store::live_session(&self.store, &hash, now, SESSION_IDLE)? else {
            return Ok(None);
        };
        let dev_open = match self.allowlist.decide(&self.store, &[key])? {
            Decision::Allowed(_) => false,
            Decision::Open(_) => true,
            Decision::Denied => {
                // Revoked since sign-in: the cookie dies with the permission.
                let _ = store::delete_session(&self.store, &hash);
                warn!(handle = %keys::handle(&key), "portal session ended: identity no longer on the staff list");
                return Ok(None);
            }
        };
        let who = Who {
            handle: keys::handle(&key),
            key,
        };
        Ok(Some((token.to_owned(), who, dev_open)))
    }

    fn unavailable(&self, error: &innkeeper_world::StoreError) -> Reply {
        warn!(%error, "the portal store failed");
        crate::metrics::counters().store_error();
        html_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            html::bare_page(
                false,
                "Out of order",
                &html::narrator(
                    html::Tone::Bad,
                    "The back office could not read its records. Try again in a minute.",
                    Some("The clerk is looking for the key to the filing cabinet."),
                ),
            ),
        )
    }

    /// A POST must come from this office. A real `Origin` must be this office's. A page served with
    /// `Referrer-Policy: no-referrer` makes browsers send `Origin: null` (or none) even for a
    /// same-origin form, so then the `Referer` must be this office, or the browser's own
    /// `Sec-Fetch-Site` must say `same-origin`. A cross-site page, a sandboxed frame and a tool that
    /// sends none of these are refused.
    fn origin_ok(&self, headers: &hyper::HeaderMap) -> bool {
        let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        if let Some(origin) = text("origin").filter(|origin| *origin != "null") {
            return origin == self.origin;
        }
        let referer_ok = text("referer")
            .and_then(|text| Url::parse(text).ok())
            .is_some_and(|url| url.origin().ascii_serialization() == self.origin);
        referer_ok || text("sec-fetch-site") == Some("same-origin")
    }

    fn forbidden(&self, req: &Req, why: &str) -> Reply {
        let body = html::narrator(
            html::Tone::Bad,
            why,
            Some("The clerk squints at the form and slides it back across the counter."),
        );
        html_reply(
            StatusCode::FORBIDDEN,
            html::page(&req.ctx, Nav::None, "Refused", None, &body),
        )
    }

    // ---- signing in ----

    fn login_page(&self) -> Reply {
        let providers = self.sign_in.rp().providers();
        let mut list = String::new();
        for provider in providers {
            list.push_str(&format!(
                "<p><a class=\"abtn primary\" href=\"/admin/auth/login?provider={}\">Sign in with {}</a></p>",
                esc(&provider.id),
                esc(&provider.name)
            ));
        }
        let body = if providers.is_empty() {
            html::narrator(
                html::Tone::Bad,
                "No sign-in provider is configured, so nobody can enter.",
                Some("The doorbell is not connected to anything."),
            )
        } else {
            html::narrator(
                html::Tone::Plain,
                "Staff only. Sign in with the account your identity key was added under.",
                Some("The clerk looks up. \"Name?\""),
            ) + &format!(
                "<div class=\"card\">{}</div>",
                html::window("Staff entrance", &list)
            )
        };
        html_reply(
            StatusCode::OK,
            html::bare_page(self.dev_open(), "Staff entrance", &body),
        )
    }

    async fn auth_login(&self, query: &HashMap<String, String>) -> Reply {
        let provider = query.get("provider").map_or("", String::as_str);
        if self.sign_in.rp().provider(provider).is_none() {
            return html_reply(
                StatusCode::BAD_REQUEST,
                html::bare_page(
                    false,
                    "Unknown provider",
                    &html::narrator(html::Tone::Bad, "There is no such sign-in provider.", None),
                ),
            );
        }
        let expires_at = dates::now_ms().saturating_add(crate::signin::LOGIN_TTL);
        let states = self.sign_in.states();
        let seal = |pending: &PendingRp| states.seal(&pending.to_state(expires_at, None));
        let authorize = match self
            .sign_in
            .rp()
            .begin(provider, Audience::Admin, seal)
            .await
        {
            Ok(authorize) => authorize,
            Err(error) => {
                warn!(provider, %error, "portal sign-in could not start");
                return self.sign_in_failed();
            }
        };
        let mut reply = http::empty(StatusCode::FOUND);
        if let Ok(value) = HeaderValue::from_str(authorize.as_str()) {
            reply.headers_mut().insert(LOCATION, value);
        }
        reply
    }

    fn sign_in_failed(&self) -> Reply {
        html_reply(
            StatusCode::BAD_GATEWAY,
            html::bare_page(
                self.dev_open(),
                "Sign-in failed",
                &(html::narrator(
                    html::Tone::Bad,
                    "The sign-in provider did not answer properly.",
                    Some("The telephone at the other end rings and rings."),
                ) + "<p><a class=\"abtn\" href=\"/admin/login\">Back to the entrance</a></p>"),
            ),
        )
    }

    async fn auth_callback(&self, query: &HashMap<String, String>) -> Reply {
        let state = query.get("state").map_or("", String::as_str);
        let opened = self
            .sign_in
            .states()
            .open(state, Audience::Admin, dates::now_ms());
        let Some(login) = opened.map(|state| PendingRp::from_state(&state)) else {
            return html_reply(
                StatusCode::BAD_REQUEST,
                html::bare_page(
                    self.dev_open(),
                    "Sign-in expired",
                    &(html::narrator(
                        html::Tone::Bad,
                        "This sign-in has expired or was already used.",
                        Some("The ticket has been punched twice."),
                    ) + "<p><a class=\"abtn\" href=\"/admin/login\">Back to the entrance</a></p>"),
                ),
            );
        };
        if query.contains_key("error") {
            return html_reply(
                StatusCode::FORBIDDEN,
                html::bare_page(
                    self.dev_open(),
                    "Turned away",
                    &(html::narrator(
                        html::Tone::Bad,
                        "The sign-in provider did not let you through.",
                        None,
                    ) + "<p><a class=\"abtn\" href=\"/admin/login\">Back to the entrance</a></p>"),
                ),
            );
        }
        let Some(code) = query.get("code").cloned() else {
            return self.sign_in_failed();
        };
        let verified = match self.sign_in.rp().finish(login, code).await {
            Ok(verified) => verified,
            Err(error) => {
                warn!(%error, "portal sign-in did not finish");
                return self.sign_in_failed();
            }
        };
        let ring = self.sign_in.keys();
        let candidates: Vec<IdentityKey> = ring
            .candidates(&verified.issuer, &verified.subject)
            .into_iter()
            .map(|(_, key)| key)
            .collect();
        let Some(current) = candidates.first().copied() else {
            return self.sign_in_failed();
        };
        let decision = match self.allowlist.decide(&self.store, &candidates) {
            Ok(decision) => decision,
            Err(error) => {
                warn!(%error, "the portal store failed");
                crate::metrics::counters().store_error();
                return self.sign_in_failed();
            }
        };
        match decision {
            Decision::Allowed(key) | Decision::Open(key) => self.enter(&key),
            Decision::Denied => {
                // Only the handle is logged: never the key, never the provider's subject.
                warn!(handle = %keys::handle(&current), "portal sign-in refused: not on the staff list");
                self.refusal(&current)
            }
        }
    }

    /// Opens a portal session and lands on the office. The 200 page (not a redirect) makes the
    /// move to `/admin/` a same-site navigation, so the `SameSite=Strict` cookie goes with it.
    fn enter(&self, key: &IdentityKey) -> Reply {
        let token = keys::new_token();
        let now = dates::now_ms();
        let stored = store::create_session(
            &self.store,
            &keys::token_hash(&token),
            key,
            now,
            now.saturating_add(SESSION_LIFETIME),
        );
        if let Err(error) = stored {
            warn!(%error, "the portal store failed");
            crate::metrics::counters().store_error();
            return self.sign_in_failed();
        }
        info!(handle = %keys::handle(key), "portal sign-in");
        let body = html::narrator(
            html::Tone::Good,
            &format!("Signed in as {}.", keys::handle(key)),
            Some("The clerk lifts the counter flap without looking up."),
        ) + "<p><a class=\"abtn primary\" href=\"/admin/\">Enter the office</a></p>\
             <meta http-equiv=\"refresh\" content=\"0;url=/admin/\">";
        let mut reply = html_reply(
            StatusCode::OK,
            html::bare_page(self.dev_open(), "Welcome", &body),
        );
        reply.headers_mut().append(
            SET_COOKIE,
            self.cookie(&token, SESSION_LIFETIME / dates::SECOND),
        );
        reply
    }

    /// The polite refusal: the identity's own key, and how to put it on the list. Nothing else.
    fn refusal(&self, key: &IdentityKey) -> Reply {
        let hex = allowlist::hex(key);
        let intro = html::narrator(
            html::Tone::Bad,
            &format!(
                "You are signed in as {}, but this identity is not on the staff list.",
                keys::handle(key)
            ),
            Some("The clerk runs a finger down the ledger. Twice. \"Not here,\" says the clerk, kindly."),
        );
        let mine = format!(
            "<p>This is your identity key. It is not a password, but give it only to the person who runs this server.</p>\
             <div class=\"keybox\"><code id=\"mykey\">{}</code>\
             <button type=\"button\" class=\"abtn\" data-copy=\"mykey\">Copy</button></div>",
            esc(&hex)
        );
        let how = "<p>To be let in, the operator adds that key to the staff list, in any one of these ways:</p>\
             <ul>\
             <li>the server vault secret <code>admin-identities</code>: one key per line, or separated by commas, \
             with <code>#</code> comments allowed. The vault is read at startup, so restart the server after changing it;</li>\
             <li>the file named by <code>--admin-allowlist</code>, in the same format. It is read again on every request, \
             so a saved change takes effect at once;</li>\
             <li>the command <code>innkeeperd admin allow KEY --data-dir DIR</code>, which works while the server runs.</li>\
             </ul><p>Then sign in again.</p>\
             <p><a class=\"abtn\" href=\"/admin/login\">Back to the entrance</a></p>";
        let body = format!(
            "{intro}<div class=\"card\">{}{}</div>",
            html::window("Your identity key", &mine),
            html::window("How to get on the list", how)
        );
        html_reply(
            StatusCode::FORBIDDEN,
            html::bare_page(false, "Not on the list", &body),
        )
    }

    fn logout(&self, req: &Req, token: &str) -> Reply {
        let _ = store::delete_session(&self.store, &keys::token_hash(token));
        info!(handle = %req.who.handle, "portal sign-out");
        let mut reply = see_other("/admin/login");
        reply.headers_mut().append(SET_COOKIE, self.cookie("", 0));
        reply
    }
}

async fn read_form(request: Request<Incoming>) -> Option<HashMap<String, String>> {
    let kind = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !kind.starts_with("application/x-www-form-urlencoded") {
        return None;
    }
    let collected = Limited::new(request.into_body(), MAX_FORM)
        .collect()
        .await
        .ok()?;
    let bytes = collected.to_bytes();
    let mut map = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(&bytes) {
        map.entry(key.into_owned())
            .or_insert_with(|| value.into_owned());
    }
    Some(map)
}

impl Admin {
    fn get(&self, req: &Req, path: &str) -> Reply {
        let segments: Vec<&str> = path
            .trim_start_matches("/admin/")
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        match segments.as_slice() {
            [] => self.dashboard(req),
            ["accounts"] => self.accounts(req),
            ["accounts", id] => self.account(req, id),
            ["complaints"] => self.complaints(req),
            ["complaints", id] => self.complaint(req, id),
            ["posts"] => self.boards(req),
            ["posts", board] => self.board(req, board),
            ["letters"] => self.mailboxes(req),
            ["letters", mailbox] => self.mailbox(req, mailbox),
            ["letters", mailbox, id] => self.letter_gate(req, mailbox, id),
            ["saves"] => self.save_owners(req),
            ["saves", account] => self.saves_of(req, account),
            ["holds"] => self.holds(req),
            ["audit"] => self.audit(req),
            ["help"] => self.help(req),
            _ => self.not_found(req),
        }
    }

    async fn post(&self, req: &Req, path: &str, token: &str) -> Reply {
        let segments: Vec<&str> = path
            .trim_start_matches("/admin/")
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        match segments.as_slice() {
            ["logout"] => self.logout(req, token),
            ["kick"] => self.kick(req).await,
            ["announce"] => self.announce(req).await,
            ["accounts", id, action] => self.account_action(req, id, action),
            ["complaints", id, "handled"] => self.complaint_handled(req, id),
            ["posts", board, id, "remove"] => self.post_remove(req, board, id),
            ["letters", mailbox, id, "read"] => self.letter_read(req, mailbox, id),
            ["holds", "place"] => self.hold_place(req),
            ["holds", "release"] => self.hold_release(req),
            ["saves", account, "download"] => self.save_download(req, account),
            _ => self.not_found(req),
        }
    }

    pub(crate) fn not_found(&self, req: &Req) -> Reply {
        let body = html::narrator(
            html::Tone::Bad,
            "There is no such page in this office.",
            Some("You open a door. It is a cupboard. It is a very full cupboard."),
        );
        html_reply(
            StatusCode::NOT_FOUND,
            html::page(&req.ctx, Nav::None, "Not found", None, &body),
        )
    }

    pub(crate) fn failed(&self, req: &Req, error: &innkeeper_world::StoreError) -> Reply {
        warn!(%error, "the portal store failed");
        crate::metrics::counters().store_error();
        let body = html::narrator(
            html::Tone::Bad,
            "The store could not answer. Nothing was changed that you were told about.",
            Some("The filing cabinet is stuck."),
        );
        html_reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            html::page(&req.ctx, Nav::None, "Out of order", None, &body),
        )
    }

    pub(crate) fn render(
        &self,
        req: &Req,
        nav: Nav,
        title: &str,
        flash: Option<&Flash>,
        body: &str,
    ) -> Reply {
        html_reply(
            StatusCode::OK,
            html::page(&req.ctx, nav, title, flash, body),
        )
    }

    pub(crate) fn render_status(
        &self,
        req: &Req,
        status: StatusCode,
        nav: Nav,
        title: &str,
        flash: Option<&Flash>,
        body: &str,
    ) -> Reply {
        html_reply(status, html::page(&req.ctx, nav, title, flash, body))
    }
}

#[cfg(test)]
mod tests;
