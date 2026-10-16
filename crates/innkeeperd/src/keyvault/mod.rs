//! Reads the server Key Vault's secrets once at startup, over the REST API with a managed
//! identity or a service principal (docs/server/privacy.md section 7).

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

mod credential;
pub(crate) use credential::Credential;

pub(crate) const HMAC_SECRET: &str = "identity-hmac-key";
pub(crate) const AUDIT_KEY_SECRET: &str = "audit-public-key";
pub(crate) const AUDIT_KID_SECRET: &str = "audit-key-id";
/// The operator portal's staff allowlist (docs/server/innkeeperd.md section 9). Optional.
pub(crate) const ADMIN_SECRET: &str = "admin-identities";
const API_VERSION: &str = "7.4";
const IMDS: &str = "http://169.254.169.254";
const AUTHORITY: &str = "https://login.microsoftonline.com";
const VAULT_HOST_SUFFIX: &str = ".vault.azure.net";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_VERSION_PAGES: usize = 50;

#[derive(Debug, Error)]
pub(crate) enum VaultError {
    #[error("the vault URL {0}")]
    Url(String),
    #[error("service principal settings are incomplete: {0}")]
    Credential(String),
    #[error("cannot read the client certificate: {0}")]
    Certificate(String),
    #[error("cannot get a vault token: {0}")]
    Token(String),
    #[error("secret {name}: {why}")]
    Secret { name: String, why: String },
    #[error("cannot start the vault client: {0}")]
    Client(String),
}

/// `https://NAME.vault.azure.net` with nothing after the host.
pub(crate) fn check_vault_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|_| "is not a URL".to_owned())?;
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let named = host.len() > VAULT_HOST_SUFFIX.len() && host.ends_with(VAULT_HOST_SUFFIX);
    let bare = url.path() == "/" && url.query().is_none() && url.port().is_none();
    if url.scheme() != "https" || !named || !bare {
        return Err("must be https://NAME.vault.azure.net".to_owned());
    }
    Ok(url)
}

/// Where requests go. Tests point all three at a local mock.
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    pub(crate) vault: Url,
    pub(crate) imds: Url,
    pub(crate) authority: Url,
}

impl Endpoints {
    pub(crate) fn new(
        vault: &str,
        test_vault: Option<&str>,
        test_imds: Option<&str>,
        test_authority: Option<&str>,
    ) -> Result<Self, VaultError> {
        let vault = check_vault_url(vault).map_err(VaultError::Url)?;
        let parse =
            |text: &str| Url::parse(text).map_err(|error| VaultError::Url(error.to_string()));
        Ok(Endpoints {
            vault: test_vault.map(parse).transpose()?.unwrap_or(vault),
            imds: parse(test_imds.unwrap_or(IMDS))?,
            authority: parse(test_authority.unwrap_or(AUTHORITY))?,
        })
    }
}

/// What startup takes from the vault. Values are wiped when dropped.
pub(crate) struct StartupSecrets {
    /// `identity-hmac-key`, every enabled version, the current one first.
    pub(crate) hmac_keys: Vec<(String, Zeroizing<String>)>,
    pub(crate) client_secrets: HashMap<String, Zeroizing<String>>,
    pub(crate) audit_public_key: String,
    pub(crate) audit_key_id: String,
    /// `admin-identities`: asked for only when the portal is on; `None` when the vault has none.
    pub(crate) admin_identities: Option<Zeroizing<String>>,
}

/// Fetches the startup secrets on a runtime of its own, before the sandbox and the server's
/// runtime exist.
pub(crate) fn fetch_startup(
    endpoints: &Endpoints,
    credential: Credential,
    client_secret_names: &[String],
    want_admin: bool,
) -> Result<StartupSecrets, VaultError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| VaultError::Client(error.to_string()))?;
    let result = runtime.block_on(async {
        VaultClient::new(endpoints.clone(), credential)?
            .startup(client_secret_names, want_admin)
            .await
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

pub(crate) struct VaultClient {
    http: reqwest::Client,
    endpoints: Endpoints,
    credential: Credential,
}

struct SecretValue {
    version: String,
    value: Zeroizing<String>,
}

impl VaultClient {
    pub(crate) fn new(endpoints: Endpoints, credential: Credential) -> Result<Self, VaultError> {
        // IMDS must never go through a proxy, and nothing here follows redirects.
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|error| VaultError::Client(error.to_string()))?;
        Ok(VaultClient {
            http,
            endpoints,
            credential,
        })
    }

    pub(crate) async fn startup(
        &self,
        client_secret_names: &[String],
        want_admin: bool,
    ) -> Result<StartupSecrets, VaultError> {
        let token = self.token().await?;
        let hmac_keys = self.all_versions(&token, HMAC_SECRET).await?;
        let mut client_secrets = HashMap::new();
        for name in client_secret_names {
            if !client_secrets.contains_key(name) {
                let secret = self.secret(&token, name, None).await?;
                client_secrets.insert(name.clone(), secret.value);
            }
        }
        let audit_public_key = self
            .secret(&token, AUDIT_KEY_SECRET, None)
            .await?
            .value
            .to_string();
        let audit_key_id = self
            .secret(&token, AUDIT_KID_SECRET, None)
            .await?
            .value
            .trim()
            .to_owned();
        let admin_identities = match want_admin {
            true => self.optional_secret(&token, ADMIN_SECRET).await?,
            false => None,
        };
        Ok(StartupSecrets {
            hmac_keys,
            client_secrets,
            audit_public_key,
            audit_key_id,
            admin_identities,
        })
    }

    /// A secret that may be absent: a 404 is `None`, any other failure stops startup.
    async fn optional_secret(
        &self,
        token: &str,
        name: &str,
    ) -> Result<Option<Zeroizing<String>>, VaultError> {
        match self.secret(token, name, None).await {
            Ok(found) => Ok(Some(found.value)),
            Err(VaultError::Secret { why, .. }) if why.starts_with("status 404") => Ok(None),
            Err(other) => Err(other),
        }
    }

    async fn token(&self) -> Result<Zeroizing<String>, VaultError> {
        let failed = |why: String| VaultError::Token(why);
        let request = self
            .credential
            .token_request(&self.http, &self.endpoints)
            .map_err(failed)?;
        let response = request
            .send()
            .await
            .map_err(|error| failed(error.without_url().to_string()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let code = body
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("no error code");
            return Err(failed(format!("status {status}, {code}")));
        }
        body.get("access_token")
            .and_then(Value::as_str)
            .map(|token| Zeroizing::new(token.to_owned()))
            .ok_or_else(|| failed("the answer has no access_token".to_owned()))
    }

    async fn get(&self, token: &str, url: Url, name: &str) -> Result<Value, VaultError> {
        let failed = |why: String| VaultError::Secret {
            name: name.to_owned(),
            why,
        };
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| failed(error.without_url().to_string()))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("no error code");
            return Err(failed(format!("status {status}, {code}")));
        }
        Ok(body)
    }

    fn secret_url(&self, name: &str, tail: &str) -> Result<Url, VaultError> {
        crate::signin::config::check_secret_name(name).map_err(|why| VaultError::Secret {
            name: name.to_owned(),
            why,
        })?;
        let mut url = join(&self.endpoints.vault, &format!("secrets/{name}{tail}"))
            .map_err(VaultError::Url)?;
        url.query_pairs_mut()
            .append_pair("api-version", API_VERSION);
        Ok(url)
    }

    async fn secret(
        &self,
        token: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<SecretValue, VaultError> {
        let tail = match version {
            Some(version) => format!("/{version}"),
            None => String::new(),
        };
        let body = self.get(token, self.secret_url(name, &tail)?, name).await?;
        let missing = |what: &str| VaultError::Secret {
            name: name.to_owned(),
            why: format!("the answer has no {what}"),
        };
        let value = body
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("value"))?;
        let id = body
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("id"))?;
        Ok(SecretValue {
            version: last_segment(id)
                .ok_or_else(|| missing("version"))?
                .to_owned(),
            value: Zeroizing::new(value.to_owned()),
        })
    }

    /// The current version first, then every other enabled version, newest first.
    async fn all_versions(
        &self,
        token: &str,
        name: &str,
    ) -> Result<Vec<(String, Zeroizing<String>)>, VaultError> {
        let current = self.secret(token, name, None).await?;
        let mut versions = self.enabled_versions(token, name).await?;
        versions.retain(|(version, _)| *version != current.version);
        versions.sort_by_key(|(_, created)| std::cmp::Reverse(*created));
        let mut out = vec![(current.version, current.value)];
        for (version, _) in versions {
            if !version.bytes().all(|b| b.is_ascii_alphanumeric()) {
                continue;
            }
            let older = self.secret(token, name, Some(&version)).await?;
            out.push((older.version, older.value));
        }
        Ok(out)
    }

    async fn enabled_versions(
        &self,
        token: &str,
        name: &str,
    ) -> Result<Vec<(String, i64)>, VaultError> {
        let mut next = Some(self.secret_url(name, "/versions")?);
        let mut found = Vec::new();
        for _ in 0..MAX_VERSION_PAGES {
            let Some(url) = next.take() else {
                break;
            };
            let page = self.get(token, url, name).await?;
            for item in page
                .get("value")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let enabled =
                    item.pointer("/attributes/enabled").and_then(Value::as_bool) == Some(true);
                let version = item
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(last_segment);
                let created = item
                    .pointer("/attributes/created")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                if let (true, Some(version)) = (enabled, version) {
                    found.push((version.to_owned(), created));
                }
            }
            next = page
                .get("nextLink")
                .and_then(Value::as_str)
                .and_then(|link| Url::parse(link).ok())
                .filter(|link| link.origin() == self.endpoints.vault.origin());
        }
        Ok(found)
    }
}

fn join(base: &Url, path: &str) -> Result<Url, String> {
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        let with_slash = format!("{}/", base.path());
        base.set_path(&with_slash);
    }
    base.join(path).map_err(|error| error.to_string())
}

fn last_segment(id: &str) -> Option<&str> {
    let path = id.split('?').next()?;
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
}

#[cfg(test)]
mod tests;
