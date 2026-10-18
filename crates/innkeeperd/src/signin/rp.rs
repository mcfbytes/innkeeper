//! The OpenID Connect and plain OAuth relying party. It runs one login for an audience and
//! returns the provider's issuer and subject; what a login grants is the caller's business.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use oauth2::basic::BasicClient;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    TokenUrl,
};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

use super::config::{ProviderEntry, ProviderKind};
use super::state::{LoginState, PageState};

/// Discovery and the signing keys are fetched again after this, or at once when a token fails.
const METADATA_TTL: Duration = Duration::from_secs(3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const USER_AGENT: &str = concat!("innkeeperd/", env!("CARGO_PKG_VERSION"));

/// Who a login is for. Each has its own callback path, registered with the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Audience {
    Player,
    /// The operator portal (docs/server/innkeeperd.md section 9).
    Admin,
}

impl Audience {
    pub(crate) fn callback_path(self) -> &'static str {
        match self {
            Audience::Player => "/auth/callback",
            Audience::Admin => "/admin/auth/callback",
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum RpError {
    #[error("unknown provider")]
    UnknownProvider,
    #[error("provider discovery failed: {0}")]
    Discovery(String),
    #[error("the token exchange failed: {0}")]
    Token(String),
    #[error("the provider returned no ID token")]
    NoIdToken,
    #[error("the ID token did not verify: {0}")]
    Claims(String),
    #[error("the profile request failed: {0}")]
    Profile(String),
    #[error("the profile answer has no {0} field")]
    NoUserId(String),
    #[error("{0}")]
    Config(String),
}

enum Flavour {
    Oidc {
        issuer: IssuerUrl,
        metadata: Mutex<Option<(Instant, Box<CoreProviderMetadata>)>>,
    },
    OAuth2(Box<OAuthEndpoints>),
}

struct OAuthEndpoints {
    authorize: AuthUrl,
    token: TokenUrl,
    userinfo: Url,
    id_field: String,
    scopes: Vec<String>,
}

/// One configured provider with its client secret, held in memory only.
pub(crate) struct Provider {
    pub(crate) id: String,
    pub(crate) name: String,
    issuer: String,
    client_id: ClientId,
    secret: Zeroizing<String>,
    flavour: Flavour,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("id", &self.id)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

impl Provider {
    pub(crate) fn from_entry(
        entry: &ProviderEntry,
        secret: Zeroizing<String>,
    ) -> Result<Self, RpError> {
        let bad = |what: &str| RpError::Config(format!("provider {}: bad {what}", entry.id));
        let flavour = match entry.kind {
            ProviderKind::Oidc => Flavour::Oidc {
                issuer: IssuerUrl::new(entry.issuer.clone()).map_err(|_| bad("issuer"))?,
                metadata: Mutex::new(None),
            },
            ProviderKind::Oauth2 => {
                let field = |value: &Option<String>| value.clone().unwrap_or_default();
                Flavour::OAuth2(Box::new(OAuthEndpoints {
                    authorize: AuthUrl::new(field(&entry.authorize_url))
                        .map_err(|_| bad("authorize_url"))?,
                    token: TokenUrl::new(field(&entry.token_url)).map_err(|_| bad("token_url"))?,
                    userinfo: Url::parse(&field(&entry.userinfo_url))
                        .map_err(|_| bad("userinfo_url"))?,
                    id_field: entry.id_field.clone(),
                    scopes: entry.scopes.clone(),
                }))
            }
        };
        Ok(Provider {
            id: entry.id.clone(),
            name: entry.name.clone(),
            issuer: entry.issuer.clone(),
            client_id: ClientId::new(entry.client_id.clone()),
            secret,
            flavour,
        })
    }

    fn client_secret(&self) -> ClientSecret {
        ClientSecret::new(self.secret.to_string())
    }
}

/// A login between the redirect to the provider and its callback. It travels sealed inside the
/// `state` value (`super::state`), so the server keeps nothing for it.
pub(crate) struct PendingRp {
    provider: String,
    audience: Audience,
    verifier: PkceCodeVerifier,
    nonce: Option<Nonce>,
}

impl PendingRp {
    /// What the sealed `state` carries for this login.
    pub(crate) fn to_state(&self, expires_at: i64, page: Option<PageState>) -> LoginState {
        LoginState {
            provider: self.provider.clone(),
            audience: self.audience,
            verifier: self.verifier.secret().clone(),
            nonce: self.nonce.as_ref().map(|nonce| nonce.secret().clone()),
            expires_at,
            page,
        }
    }

    pub(crate) fn from_state(state: &LoginState) -> Self {
        PendingRp {
            provider: state.provider.clone(),
            audience: state.audience,
            verifier: PkceCodeVerifier::new(state.verifier.clone()),
            nonce: state.nonce.clone().map(Nonce::new),
        }
    }
}

/// The provider account, as the caller needs it for the identity HMAC.
pub(crate) struct Verified {
    pub(crate) provider: String,
    pub(crate) issuer: String,
    pub(crate) subject: Zeroizing<String>,
}

pub(crate) struct Rp {
    providers: Vec<Provider>,
    http: reqwest::Client,
    base: Url,
    /// Where the portal's callback is, when that is not `base`.
    admin_base: Option<Url>,
}

impl std::fmt::Debug for Rp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rp")
            .field("providers", &self.providers)
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl Rp {
    /// `base` is the auth origin as providers see it; callbacks are paths under it.
    pub(crate) fn new(providers: Vec<Provider>, base: Url) -> Result<Self, RpError> {
        // Following redirects would let a provider answer point the server elsewhere.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| RpError::Config(format!("the HTTP client: {error}")))?;
        Ok(Rp {
            providers,
            http,
            base,
            admin_base: None,
        })
    }

    /// Where the portal is as browsers reach it: its callback's origin.
    pub(crate) fn admin_origin(&self) -> &Url {
        self.admin_base.as_ref().unwrap_or(&self.base)
    }

    /// The origin the portal's callback hangs off, when it differs from the players'.
    pub(crate) fn with_admin_base(mut self, base: Url) -> Self {
        self.admin_base = Some(base);
        self
    }

    pub(crate) fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub(crate) fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    fn redirect(&self, audience: Audience) -> Result<RedirectUrl, RpError> {
        let base = match audience {
            Audience::Admin => self.admin_base.as_ref().unwrap_or(&self.base),
            Audience::Player => &self.base,
        };
        let url = base
            .join(audience.callback_path())
            .map_err(|error| RpError::Config(format!("the callback URL: {error}")))?;
        Ok(RedirectUrl::from_url(url))
    }

    /// Where to send the browser, with PKCE, `state` and (for OIDC) a nonce. Scope is `openid`.
    /// `seal` turns the pending login into the `state` value; `None` from it fails the login.
    /// The answer is the provider's authorize URL.
    pub(crate) async fn begin(
        &self,
        provider_id: &str,
        audience: Audience,
        seal: impl FnOnce(&PendingRp) -> Option<String>,
    ) -> Result<Url, RpError> {
        let provider = self.provider(provider_id).ok_or(RpError::UnknownProvider)?;
        let redirect = self.redirect(audience)?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let nonce = matches!(provider.flavour, Flavour::Oidc { .. }).then(Nonce::new_random);
        let pending = PendingRp {
            provider: provider.id.clone(),
            audience,
            verifier,
            nonce,
        };
        let state = seal(&pending)
            .ok_or_else(|| RpError::Config("the login state could not be sealed".into()))?;
        let csrf = CsrfToken::new(state);
        let authorize = match &provider.flavour {
            Flavour::Oidc { .. } => {
                let metadata = self.metadata(provider, false).await?;
                let client = CoreClient::from_provider_metadata(
                    metadata,
                    provider.client_id.clone(),
                    Some(provider.client_secret()),
                )
                .set_redirect_uri(redirect);
                let nonce = pending.nonce.clone().unwrap_or_else(Nonce::new_random);
                let (url, _, _) = client
                    .authorize_url(
                        CoreAuthenticationFlow::AuthorizationCode,
                        move || csrf,
                        move || nonce,
                    )
                    .set_pkce_challenge(challenge)
                    .url();
                url
            }
            Flavour::OAuth2(endpoints) => {
                let client = BasicClient::new(provider.client_id.clone())
                    .set_auth_uri(endpoints.authorize.clone())
                    .set_redirect_uri(redirect);
                let (url, _) = client
                    .authorize_url(move || csrf)
                    .add_scopes(endpoints.scopes.iter().cloned().map(Scope::new))
                    .set_pkce_challenge(challenge)
                    .url();
                url
            }
        };
        Ok(authorize)
    }

    /// Exchanges the code and returns the verified subject. Profile fields are dropped unread.
    pub(crate) async fn finish(
        &self,
        pending: PendingRp,
        code: String,
    ) -> Result<Verified, RpError> {
        let provider = self
            .provider(&pending.provider)
            .ok_or(RpError::UnknownProvider)?;
        let redirect = self.redirect(pending.audience)?;
        let subject = match &provider.flavour {
            Flavour::Oidc { .. } => {
                let nonce = pending.nonce.unwrap_or_else(|| Nonce::new(String::new()));
                return self
                    .finish_oidc(provider, redirect, pending.verifier, &nonce, code)
                    .await;
            }
            Flavour::OAuth2(endpoints) => {
                let client = BasicClient::new(provider.client_id.clone())
                    .set_client_secret(provider.client_secret())
                    .set_token_uri(endpoints.token.clone())
                    .set_redirect_uri(redirect);
                let answer = client
                    .exchange_code(AuthorizationCode::new(code))
                    .set_pkce_verifier(pending.verifier)
                    .request_async(&self.http)
                    .await
                    .map_err(|error| RpError::Token(error.to_string()))?;
                self.profile_id(
                    &endpoints.userinfo,
                    &endpoints.id_field,
                    answer.access_token().secret(),
                )
                .await?
            }
        };
        Ok(Verified {
            provider: provider.id.clone(),
            issuer: provider.issuer.clone(),
            subject,
        })
    }

    async fn finish_oidc(
        &self,
        provider: &Provider,
        redirect: RedirectUrl,
        verifier: PkceCodeVerifier,
        nonce: &Nonce,
        code: String,
    ) -> Result<Verified, RpError> {
        let metadata = self.metadata(provider, false).await?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            provider.client_id.clone(),
            Some(provider.client_secret()),
        )
        .set_redirect_uri(redirect);
        let answer = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|error| RpError::Token(error.to_string()))?
            .set_pkce_verifier(verifier)
            .request_async(&self.http)
            .await
            .map_err(|error| RpError::Token(error.to_string()))?;
        let id_token = answer.id_token().ok_or(RpError::NoIdToken)?;
        let checked = match id_token.claims(&client.id_token_verifier(), nonce) {
            Ok(claims) => Ok((claims.issuer().to_string(), claims.subject().to_string())),
            Err(first) => Err(first.to_string()),
        };
        let (issuer, subject) = match checked {
            Ok(pair) => pair,
            Err(_) => {
                // The provider may have rotated its signing keys since discovery.
                let fresh = self.metadata(provider, true).await?;
                let client = CoreClient::from_provider_metadata(
                    fresh,
                    provider.client_id.clone(),
                    Some(provider.client_secret()),
                );
                let claims = id_token
                    .claims(&client.id_token_verifier(), nonce)
                    .map_err(|error| RpError::Claims(error.to_string()))?;
                (claims.issuer().to_string(), claims.subject().to_string())
            }
        };
        Ok(Verified {
            provider: provider.id.clone(),
            issuer,
            subject: Zeroizing::new(subject),
        })
    }

    async fn metadata(
        &self,
        provider: &Provider,
        refresh: bool,
    ) -> Result<CoreProviderMetadata, RpError> {
        let Flavour::Oidc { issuer, metadata } = &provider.flavour else {
            return Err(RpError::Config(format!(
                "provider {} has no discovery",
                provider.id
            )));
        };
        if !refresh {
            let cached = metadata
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((at, value)) = cached.as_ref() {
                if at.elapsed() < METADATA_TTL {
                    return Ok(CoreProviderMetadata::clone(value));
                }
            }
        }
        let fetched = CoreProviderMetadata::discover_async(issuer.clone(), &self.http)
            .await
            .map_err(|error| RpError::Discovery(error.to_string()))?;
        let mut cached = metadata
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cached = Some((Instant::now(), Box::new(fetched.clone())));
        Ok(fetched)
    }

    /// The stable user id from a plain-OAuth profile endpoint. Nothing else is read or kept.
    async fn profile_id(
        &self,
        url: &Url,
        field: &str,
        access_token: &str,
    ) -> Result<Zeroizing<String>, RpError> {
        let response = self
            .http
            .get(url.clone())
            .bearer_auth(access_token)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|error| RpError::Profile(error.without_url().to_string()))?;
        if !response.status().is_success() {
            return Err(RpError::Profile(format!("status {}", response.status())));
        }
        let profile: Value = response
            .json()
            .await
            .map_err(|error| RpError::Profile(error.without_url().to_string()))?;
        let id = match profile.get(field) {
            Some(Value::String(text)) if !text.is_empty() => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => return Err(RpError::NoUserId(field.to_owned())),
        };
        Ok(Zeroizing::new(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_audience_has_its_own_callback() {
        assert_eq!(Audience::Player.callback_path(), "/auth/callback");
        assert_eq!(Audience::Admin.callback_path(), "/admin/auth/callback");
    }

    fn oidc_entry() -> ProviderEntry {
        let text = r#"
[[provider]]
id = "dev"
name = "Dev"
issuer = "http://127.0.0.1:1"
client_id = "client"
client_secret_name = "dev-secret"
"#;
        super::super::config::AuthFile::parse(text)
            .unwrap()
            .providers
            .remove(0)
    }

    #[tokio::test]
    async fn an_admin_login_carries_its_own_callback_and_the_openid_scope_only() {
        let provider = Provider::from_entry(&oidc_entry(), Zeroizing::new("s".into())).unwrap();
        let Flavour::Oidc { metadata, .. } = &provider.flavour else {
            panic!("oidc");
        };
        let document = serde_json::json!({
            "issuer": "http://127.0.0.1:1",
            "authorization_endpoint": "http://127.0.0.1:1/authorize",
            "token_endpoint": "http://127.0.0.1:1/token",
            "jwks_uri": "http://127.0.0.1:1/jwks",
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        });
        let parsed: CoreProviderMetadata = serde_json::from_value(document).unwrap();
        *metadata.lock().unwrap() = Some((Instant::now(), Box::new(parsed)));
        let rp = Rp::new(
            vec![provider],
            Url::parse("https://inn.example.org").unwrap(),
        )
        .unwrap();
        for audience in [Audience::Player, Audience::Admin] {
            let mut sealed = None;
            let authorize = rp
                .begin("dev", audience, |pending| {
                    sealed = Some(pending.to_state(1, None));
                    Some("SEALED".to_owned())
                })
                .await
                .unwrap();
            let query: Vec<(String, String)> = authorize.query_pairs().into_owned().collect();
            let get = |name: &str| {
                query
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            };
            assert_eq!(get("scope").as_deref(), Some("openid"));
            assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
            assert_eq!(get("state").as_deref(), Some("SEALED"));
            let sealed = sealed.unwrap();
            assert_eq!(get("nonce"), sealed.nonce);
            assert_eq!(sealed.audience, audience);
            let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
                sealed.verifier.clone(),
            ));
            assert_eq!(get("code_challenge").as_deref(), Some(challenge.as_str()));
            let expected = format!("https://inn.example.org{}", audience.callback_path());
            assert_eq!(get("redirect_uri"), Some(expected));
        }
        assert!(matches!(
            rp.begin("nope", Audience::Player, |_| Some(String::new()))
                .await,
            Err(RpError::UnknownProvider)
        ));
    }
}
