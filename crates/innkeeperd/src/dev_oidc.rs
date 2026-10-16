//! DEVELOPMENT ONLY: a mock OpenID Connect provider for local sign-in and the tests. Its consent
//! page has Allow and Deny buttons and signs in anyone; docs/server/innkeeperd.md section 8.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};
use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use url::Url;

const KEY_BITS: usize = 2048;
const GRANT_TTL: Duration = Duration::from_secs(300);
const TOKEN_LIFETIME: u64 = 300;
const DEFAULT_SUBJECT: &str = "dev-subject-1";

struct Grant {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    nonce: Option<String>,
    subject: String,
    created: Instant,
}

/// The provider: one signing key, the codes it has handed out and the access tokens.
pub struct MockProvider {
    issuer: String,
    key: RsaPrivateKey,
    kid: String,
    grants: Mutex<HashMap<String, Grant>>,
    access: Mutex<HashMap<String, String>>,
}

type Reply = Response<Full<Bytes>>;

fn random_text() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    BASE64URL.encode(bytes)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn reply(status: StatusCode, kind: &'static str, body: String) -> Reply {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    if let Ok(value) = kind.parse() {
        response.headers_mut().insert("content-type", value);
    }
    if let Ok(value) = "no-store".parse() {
        response.headers_mut().insert("cache-control", value);
    }
    response
}

fn json_reply(status: StatusCode, value: &Value) -> Reply {
    reply(status, "application/json", value.to_string())
}

fn redirect(location: &str) -> Reply {
    let mut response = reply(StatusCode::FOUND, "text/plain", String::new());
    if let Ok(value) = location.parse() {
        response.headers_mut().insert("location", value);
    }
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

impl MockProvider {
    pub fn new(issuer: String, key: RsaPrivateKey) -> Self {
        MockProvider {
            issuer: issuer.trim_end_matches('/').to_owned(),
            key,
            kid: format!("dev-{}", random_text().chars().take(8).collect::<String>()),
            grants: Mutex::new(HashMap::new()),
            access: Mutex::new(HashMap::new()),
        }
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Serves every connection on `listener` until the task is dropped.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let provider = Arc::clone(&self);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let provider = Arc::clone(&provider);
                    async move { Ok::<_, Infallible>(provider.route(request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    }

    async fn route(&self, request: Request<Incoming>) -> Reply {
        let path = request.uri().path().to_owned();
        let query: HashMap<String, String> =
            url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
                .into_owned()
                .collect();
        match path.as_str() {
            "/.well-known/openid-configuration" => json_reply(StatusCode::OK, &self.discovery()),
            "/jwks" => json_reply(StatusCode::OK, &self.jwks()),
            "/authorize" => self.authorize(&query),
            "/token" => self.token(request).await,
            "/userinfo" => self.userinfo(&request),
            _ => json_reply(StatusCode::NOT_FOUND, &json!({"error": "not_found"})),
        }
    }

    fn discovery(&self) -> Value {
        let issuer = &self.issuer;
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "scopes_supported": ["openid"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
            "code_challenge_methods_supported": ["S256"],
        })
    }

    fn jwks(&self) -> Value {
        json!({"keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": self.kid,
            "n": BASE64URL.encode(self.key.n().to_bytes_be()),
            "e": BASE64URL.encode(self.key.e().to_bytes_be()),
        }]})
    }

    /// The consent page, or with `decision` the redirect back. Only the `openid` scope is
    /// allowed, so a relying party that asks for more is caught here.
    fn authorize(&self, query: &HashMap<String, String>) -> Reply {
        let get = |name: &str| query.get(name).map(String::as_str).unwrap_or("");
        let Ok(mut back) = Url::parse(get("redirect_uri")) else {
            return json_reply(
                StatusCode::BAD_REQUEST,
                &json!({"error": "invalid_request"}),
            );
        };
        let refusal = if get("response_type") != "code" {
            Some("unsupported_response_type")
        } else if get("scope").split(' ').any(|scope| scope != "openid") || get("scope").is_empty()
        {
            Some("invalid_scope")
        } else if get("code_challenge_method") != "S256" || get("code_challenge").is_empty() {
            Some("invalid_request")
        } else {
            None
        };
        if let Some(error) = refusal {
            back.query_pairs_mut()
                .append_pair("error", error)
                .append_pair("state", get("state"));
            return redirect(back.as_str());
        }
        match get("decision") {
            "allow" => {
                let code = random_text();
                let subject = match get("subject") {
                    "" => DEFAULT_SUBJECT,
                    named => named,
                };
                let grant = Grant {
                    client_id: get("client_id").to_owned(),
                    redirect_uri: get("redirect_uri").to_owned(),
                    challenge: get("code_challenge").to_owned(),
                    nonce: query.get("nonce").cloned(),
                    subject: subject.to_owned(),
                    created: Instant::now(),
                };
                self.grants
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(code.clone(), grant);
                back.query_pairs_mut()
                    .append_pair("code", &code)
                    .append_pair("state", get("state"));
                redirect(back.as_str())
            }
            "deny" => {
                back.query_pairs_mut()
                    .append_pair("error", "access_denied")
                    .append_pair("state", get("state"));
                redirect(back.as_str())
            }
            _ => reply(
                StatusCode::OK,
                "text/html; charset=utf-8",
                consent_page(query),
            ),
        }
    }

    async fn token(&self, request: Request<Incoming>) -> Reply {
        let basic = basic_client_id(&request);
        let Ok(body) = Limited::new(request.into_body(), 16 * 1024).collect().await else {
            return json_reply(
                StatusCode::BAD_REQUEST,
                &json!({"error": "invalid_request"}),
            );
        };
        let form: HashMap<String, String> = url::form_urlencoded::parse(&body.to_bytes())
            .into_owned()
            .collect();
        let get = |name: &str| form.get(name).map(String::as_str).unwrap_or("");
        let invalid = || json_reply(StatusCode::BAD_REQUEST, &json!({"error": "invalid_grant"}));
        if get("grant_type") != "authorization_code" {
            return json_reply(
                StatusCode::BAD_REQUEST,
                &json!({"error": "unsupported_grant_type"}),
            );
        }
        let Some(grant) = self
            .grants
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(get("code"))
        else {
            return invalid();
        };
        let client_id = basic.unwrap_or_else(|| get("client_id").to_owned());
        let verifier = BASE64URL.encode(Sha256::digest(get("code_verifier").as_bytes()));
        let fresh = grant.created.elapsed() < GRANT_TTL;
        if !fresh
            || verifier != grant.challenge
            || get("redirect_uri") != grant.redirect_uri
            || client_id != grant.client_id
        {
            return invalid();
        }
        let access = random_text();
        self.access
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(access.clone(), grant.subject.clone());
        let id_token = self.id_token(&grant);
        json_reply(
            StatusCode::OK,
            &json!({"access_token": access, "token_type": "Bearer", "expires_in": TOKEN_LIFETIME, "id_token": id_token}),
        )
    }

    fn id_token(&self, grant: &Grant) -> String {
        let issued = now();
        let mut claims = json!({
            "iss": self.issuer,
            "sub": grant.subject,
            "aud": grant.client_id,
            "iat": issued,
            "exp": issued + TOKEN_LIFETIME,
        });
        if let (Some(nonce), Value::Object(map)) = (&grant.nonce, &mut claims) {
            map.insert("nonce".into(), Value::String(nonce.clone()));
        }
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": self.kid});
        let input = format!(
            "{}.{}",
            BASE64URL.encode(header.to_string()),
            BASE64URL.encode(claims.to_string())
        );
        let signature = SigningKey::<Sha256>::new(self.key.clone()).sign(input.as_bytes());
        format!("{input}.{}", BASE64URL.encode(signature.to_vec()))
    }

    /// A plain-OAuth profile: the stable `id` plus a name a relying party must ignore.
    fn userinfo(&self, request: &Request<Incoming>) -> Reply {
        let token = request
            .headers()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or("");
        let access = self.access.lock().unwrap_or_else(PoisonError::into_inner);
        match access.get(token) {
            Some(subject) => json_reply(
                StatusCode::OK,
                &json!({"id": subject, "sub": subject, "username": "Not To Be Stored"}),
            ),
            None => json_reply(StatusCode::UNAUTHORIZED, &json!({"error": "invalid_token"})),
        }
    }
}

fn basic_client_id(request: &Request<Incoming>) -> Option<String> {
    let header = request.headers().get("authorization")?.to_str().ok()?;
    let decoded = BASE64.decode(header.strip_prefix("Basic ")?).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (id, _) = text.split_once(':')?;
    url::form_urlencoded::parse(format!("x={id}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
}

fn consent_page(query: &HashMap<String, String>) -> String {
    let hidden: String = query
        .iter()
        .filter(|(name, _)| name.as_str() != "decision" && name.as_str() != "subject")
        .map(|(name, value)| {
            format!(
                r#"<input type="hidden" name="{}" value="{}">"#,
                escape(name),
                escape(value)
            )
        })
        .collect();
    format!(
        r#"<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Mock provider</title>
<style>body{{font:16px/1.4 system-ui,sans-serif;max-width:30em;margin:3em auto;padding:0 1em}}
.warn{{background:#fde8c8;border:2px solid #b36a10;padding:.5em .8em}}button{{font:inherit;padding:.4em 1.2em;margin-right:.6em}}</style>
<h1>Mock provider</h1>
<p class="warn">innkeeperd dev-oidc: for development only. Nothing here is real, and Allow signs in anyone.</p>
<p>The game asks for the openid scope only: no email, name or profile.</p>
<form method="get" action="/authorize">{hidden}
<p><label>Subject <input name="subject" value="{DEFAULT_SUBJECT}"></label></p>
<button name="decision" value="allow" autofocus>Allow</button><button name="decision" value="deny">Deny</button>
</form>"#
    )
}

/// `innkeeperd dev-oidc`: refuses a non-loopback address, prints the issuer and serves.
pub fn run(bind: SocketAddr, issuer: Option<String>) -> ExitCode {
    if !bind.ip().is_loopback() {
        eprintln!("dev-oidc binds loopback only: it signs in anyone who clicks Allow");
        return ExitCode::FAILURE;
    }
    let issuer = issuer.unwrap_or_else(|| format!("http://{bind}"));
    let key = match RsaPrivateKey::new(&mut OsRng, KEY_BITS) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("dev-oidc: cannot make a signing key: {error}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("dev-oidc: {error}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let listener = match TcpListener::bind(bind).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("dev-oidc: cannot listen on {bind}: {error}");
                return ExitCode::FAILURE;
            }
        };
        let provider = Arc::new(MockProvider::new(issuer, key));
        eprintln!(
            "dev-oidc: MOCK OpenID Connect provider (development only) on {bind}, issuer {}",
            provider.issuer()
        );
        tokio::select! {
            () = provider.serve(listener) => ExitCode::SUCCESS,
            _ = tokio::signal::ctrl_c() => ExitCode::SUCCESS,
        }
    })
}
