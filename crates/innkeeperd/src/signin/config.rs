//! The `--auth-config` file and the page origins (docs/server/innkeeperd.md section 8).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use thiserror::Error;
use url::Url;

/// Scopes that would hand the server personal data; the PII-free contract refuses them.
const REFUSED_SCOPES: [&str; 4] = ["email", "profile", "address", "phone"];
const MAX_PROVIDER_ID: usize = 32;
const MAX_PROVIDER_NAME: usize = 64;

#[derive(Debug, Error)]
pub(crate) enum AuthConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Toml {
        path: String,
        source: Box<toml::de::Error>,
    },
    #[error("{0}")]
    Invalid(String),
}

/// What the file says, checked.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthFile {
    /// The auth origin as browsers and providers reach it; the callback URL hangs off it.
    pub(crate) public_url: Option<String>,
    /// The portal's origin when it differs from `public_url` (a local run, where it has a port of its
    /// own); the portal's callback is `<admin_public_url>/admin/auth/callback`.
    pub(crate) admin_public_url: Option<String>,
    #[serde(default, rename = "provider")]
    pub(crate) providers: Vec<ProviderEntry>,
    /// DEVELOPMENT ONLY: client secrets by name when there is no vault.
    #[serde(default)]
    pub(crate) dev_secrets: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProviderKind {
    #[default]
    Oidc,
    Oauth2,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) kind: ProviderKind,
    /// The OIDC issuer, or for plain OAuth the stable name that keys the identity HMAC.
    pub(crate) issuer: String,
    pub(crate) authorize_url: Option<String>,
    pub(crate) token_url: Option<String>,
    pub(crate) userinfo_url: Option<String>,
    /// The field of the plain-OAuth profile answer that holds the stable user id.
    #[serde(default = "default_id_field")]
    pub(crate) id_field: String,
    /// Plain OAuth only: whatever the provider needs to return its user id, nothing more.
    #[serde(default)]
    pub(crate) scopes: Vec<String>,
    pub(crate) client_id: String,
    pub(crate) client_secret_name: String,
}

fn default_id_field() -> String {
    "id".to_owned()
}

impl AuthFile {
    pub(crate) fn load(path: &Path) -> Result<Self, AuthConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| AuthConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text).map_err(|error| match error {
            AuthConfigError::Toml { source, .. } => AuthConfigError::Toml {
                path: path.display().to_string(),
                source,
            },
            other => other,
        })
    }

    pub(crate) fn parse(text: &str) -> Result<Self, AuthConfigError> {
        let file: AuthFile = toml::from_str(text).map_err(|source| AuthConfigError::Toml {
            path: String::new(),
            source: Box::new(source),
        })?;
        file.validated()
    }

    fn validated(self) -> Result<Self, AuthConfigError> {
        let mut seen = Vec::new();
        for provider in &self.providers {
            provider.check()?;
            if seen.contains(&provider.id.as_str()) {
                return Err(invalid(format!(
                    "provider id {} is listed twice",
                    provider.id
                )));
            }
            seen.push(provider.id.as_str());
        }
        if let Some(url) = &self.public_url {
            parse_origin(url).map_err(|why| invalid(format!("public_url {url}: {why}")))?;
        }
        if let Some(url) = &self.admin_public_url {
            parse_origin(url).map_err(|why| invalid(format!("admin_public_url {url}: {why}")))?;
        }
        Ok(self)
    }
}

impl ProviderEntry {
    fn check(&self) -> Result<(), AuthConfigError> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= MAX_PROVIDER_ID
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !id_ok {
            return Err(invalid(format!(
                "provider id {:?} must be 1 to 32 of A-Z, a-z, 0-9, - and _",
                self.id
            )));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_PROVIDER_NAME {
            return Err(invalid(format!(
                "provider {}: name must be 1 to 64 characters",
                self.id
            )));
        }
        if self.client_id.is_empty() || self.client_secret_name.is_empty() {
            return Err(invalid(format!(
                "provider {}: client_id and client_secret_name are required",
                self.id
            )));
        }
        check_secret_name(&self.client_secret_name)
            .map_err(|why| invalid(format!("provider {}: {why}", self.id)))?;
        match self.kind {
            ProviderKind::Oidc => self.check_oidc(),
            ProviderKind::Oauth2 => self.check_oauth2(),
        }
    }

    fn check_oidc(&self) -> Result<(), AuthConfigError> {
        parse_url(&self.issuer)
            .map_err(|why| invalid(format!("provider {}: issuer {why}", self.id)))?;
        if !self.scopes.is_empty() {
            return Err(invalid(format!(
                "provider {}: an OpenID Connect provider asks for the openid scope only",
                self.id
            )));
        }
        if self.authorize_url.is_some() || self.token_url.is_some() || self.userinfo_url.is_some() {
            return Err(invalid(format!(
                "provider {}: endpoints come from discovery; set kind = \"oauth2\" to list them",
                self.id
            )));
        }
        Ok(())
    }

    fn check_oauth2(&self) -> Result<(), AuthConfigError> {
        if self.issuer.trim().is_empty() {
            return Err(invalid(format!(
                "provider {}: issuer names the provider",
                self.id
            )));
        }
        for (field, value) in [
            ("authorize_url", &self.authorize_url),
            ("token_url", &self.token_url),
            ("userinfo_url", &self.userinfo_url),
        ] {
            let Some(value) = value else {
                return Err(invalid(format!(
                    "provider {}: {field} is required",
                    self.id
                )));
            };
            parse_url(value)
                .map_err(|why| invalid(format!("provider {}: {field} {why}", self.id)))?;
        }
        if let Some(scope) = self
            .scopes
            .iter()
            .find(|scope| REFUSED_SCOPES.contains(&scope.to_ascii_lowercase().as_str()))
        {
            return Err(invalid(format!(
                "provider {}: the {scope} scope would share personal data",
                self.id
            )));
        }
        Ok(())
    }
}

/// Key Vault secret names are 1 to 127 of letters, digits and `-`; the name becomes a URL path.
pub(crate) fn check_secret_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 127
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "secret name {name:?} must be 1 to 127 of letters, digits and -"
        ))
    }
}

fn parse_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|error| format!("is not a URL: {error}"))?;
    match url.scheme() {
        "https" | "http" => Ok(url),
        other => Err(format!("has scheme {other}; use https")),
    }
}

/// `scheme://host[:port]` with nothing after it, as browsers write `Origin`.
pub(crate) fn parse_origin(text: &str) -> Result<String, String> {
    let url = parse_url(text)?;
    let bare = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
    if !bare || !url.username().is_empty() || url.password().is_some() {
        return Err("must be an origin: scheme, host and port only".to_owned());
    }
    Ok(url.origin().ascii_serialization())
}

/// True when the origin is on this machine: `localhost`, `127.0.0.0/8` or `[::1]`.
pub(crate) fn is_loopback_origin(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// The page origins the server answers: CORS, the `return` address and the postMessage target.
#[derive(Clone, Debug, Default)]
pub(crate) struct PageOrigins {
    origins: Vec<String>,
}

impl PageOrigins {
    pub(crate) fn parse(list: &[String]) -> Result<Self, String> {
        let mut origins = Vec::new();
        for text in list {
            let origin =
                parse_origin(text).map_err(|why| format!("--page-origin {text}: {why}"))?;
            if !origins.contains(&origin) {
                origins.push(origin);
            }
        }
        Ok(PageOrigins { origins })
    }

    /// The `Origin` header value itself when it is one of ours; never a wildcard.
    pub(crate) fn allowed<'a>(&self, origin: &'a str) -> Option<&'a str> {
        self.origins
            .iter()
            .any(|known| known == origin)
            .then_some(origin)
    }

    /// The origin of a `return` address when it is one of ours, with the fragment dropped.
    pub(crate) fn check_return(&self, address: &str) -> Option<(Url, String)> {
        let mut url = Url::parse(address).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let origin = url.origin().ascii_serialization();
        self.allowed(&origin)?;
        url.set_fragment(None);
        Some((url, origin))
    }

    pub(crate) fn all_loopback(&self) -> bool {
        self.origins.iter().all(|origin| is_loopback_origin(origin))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }
}

fn invalid(text: String) -> AuthConfigError {
    AuthConfigError::Invalid(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
public_url = "https://inn.example.org"

[[provider]]
id = "google"
name = "Google"
issuer = "https://accounts.google.com"
client_id = "1234.apps.googleusercontent.com"
client_secret_name = "oidc-google-client-secret"

[[provider]]
id = "discord"
name = "Discord"
kind = "oauth2"
issuer = "https://discord.com"
authorize_url = "https://discord.com/oauth2/authorize"
token_url = "https://discord.com/api/oauth2/token"
userinfo_url = "https://discord.com/api/users/@me"
scopes = ["identify"]
client_id = "5678"
client_secret_name = "oauth-discord-client-secret"
"#;

    #[test]
    fn the_documented_example_parses() {
        let file = AuthFile::parse(EXAMPLE).unwrap();
        assert_eq!(file.providers.len(), 2);
        assert_eq!(file.providers[0].kind, ProviderKind::Oidc);
        assert_eq!(file.providers[1].kind, ProviderKind::Oauth2);
        assert_eq!(file.providers[1].id_field, "id");
    }

    #[test]
    fn personal_scopes_and_extra_oidc_scopes_are_refused() {
        let email = EXAMPLE.replace(
            r#"scopes = ["identify"]"#,
            r#"scopes = ["identify", "email"]"#,
        );
        let text = AuthFile::parse(&email).unwrap_err().to_string();
        assert!(text.contains("email"), "{text}");
        let oidc = EXAMPLE.replacen(
            "client_secret_name = \"oidc-google-client-secret\"",
            "client_secret_name = \"oidc-google-client-secret\"\nscopes = [\"profile\"]",
            1,
        );
        let text = AuthFile::parse(&oidc).unwrap_err().to_string();
        assert!(text.contains("openid scope only"), "{text}");
    }

    #[test]
    fn provider_ids_are_plain_and_unique() {
        let twice = EXAMPLE.replace("id = \"discord\"", "id = \"google\"");
        assert!(AuthFile::parse(&twice)
            .unwrap_err()
            .to_string()
            .contains("twice"));
        let odd = EXAMPLE.replace("id = \"discord\"", "id = \"dis cord\"");
        assert!(AuthFile::parse(&odd).is_err());
        let path = EXAMPLE.replace("oidc-google-client-secret", "../escape");
        assert!(AuthFile::parse(&path).is_err());
    }

    #[test]
    fn origins_are_exact_and_return_addresses_must_match_one() {
        let origins = PageOrigins::parse(&["https://play.example.org/".to_owned()]).unwrap();
        assert_eq!(
            origins.allowed("https://play.example.org"),
            Some("https://play.example.org")
        );
        assert_eq!(origins.allowed("https://play.example.org.evil.test"), None);
        assert_eq!(origins.allowed("http://play.example.org"), None);
        assert_eq!(origins.allowed("*"), None);
        let (url, origin) = origins
            .check_return("https://play.example.org/inn.html?auth=x#frag")
            .unwrap();
        assert_eq!(origin, "https://play.example.org");
        assert_eq!(url.as_str(), "https://play.example.org/inn.html?auth=x");
        assert!(origins.check_return("https://evil.test/inn.html").is_none());
        assert!(origins.check_return("javascript:alert(1)").is_none());
        assert!(origins
            .check_return("https://play.example.org@evil.test/")
            .is_none());
        assert!(PageOrigins::parse(&["https://play.example.org/inn.html".to_owned()]).is_err());
    }

    #[test]
    fn loopback_origins_are_recognised() {
        assert!(is_loopback_origin("http://127.0.0.1:2901"));
        assert!(is_loopback_origin("http://localhost:8080"));
        assert!(is_loopback_origin("http://[::1]:9"));
        assert!(!is_loopback_origin("https://play.example.org"));
        let mixed =
            PageOrigins::parse(&["http://127.0.0.1:1".to_owned(), "https://x.test".to_owned()]);
        assert!(!mixed.unwrap().all_loopback());
    }
}
