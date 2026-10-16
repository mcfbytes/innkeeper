//! Startup for sign-in and the audit key: the auth file, the vault or the development keys, read
//! once before the sandbox (docs/server/innkeeperd.md section 8).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tracing::{info, warn};
use url::Url;
use zeroize::Zeroizing;

use super::config::{AuthConfigError, AuthFile, PageOrigins};
use super::keys::{HmacKey, KeyError, KeyRing, DEV_KEY_FILE};
use super::rp::{Provider, Rp, RpError};
use super::{SignIn, SignInSetup};
use crate::config::Config;
use crate::keyvault::{self, Credential, Endpoints, StartupSecrets, VaultError};
use crate::seal::{self, Recipient};
use crate::sqlite_store::SqliteStore;

/// A development client secret may come from the environment under this prefix.
const SECRET_ENV_PREFIX: &str = "INNKEEPERD_SECRET_";

#[derive(Debug, Error)]
pub(crate) enum StartupError {
    #[error("{0}")]
    AuthConfig(#[from] AuthConfigError),
    #[error("the server vault: {0}")]
    Vault(#[from] VaultError),
    #[error("{0}")]
    Key(#[from] KeyError),
    #[error("{0}")]
    Rp(#[from] RpError),
    #[error("{0}")]
    Admin(#[from] crate::admin::allowlist::AllowlistError),
    #[error("the audit public key cannot be used: {0}")]
    Audit(String),
    #[error("{0}")]
    Config(String),
}

/// What startup hands the server: sign-in when configured, and the audit recipient.
pub(crate) struct Prepared {
    pub(crate) sign_in: Option<Arc<SignIn>>,
    pub(crate) recipient: Option<Recipient>,
    /// The portal's staff keys from the vault secret `admin-identities`, when the vault has one.
    pub(crate) admin_vault: Option<HashSet<super::keys::IdentityKey>>,
}

pub(crate) fn prepare(
    config: &Config,
    store: Option<&Arc<SqliteStore>>,
) -> Result<Prepared, StartupError> {
    let file = config
        .auth_config
        .as_deref()
        .map(AuthFile::load)
        .transpose()?;
    let names: Vec<String> = file
        .iter()
        .flat_map(|file| &file.providers)
        .map(|provider| provider.client_secret_name.clone())
        .collect();
    let mut vault = match &config.key_vault {
        Some(url) => {
            let endpoints = Endpoints::new(
                url,
                config.test_vault_endpoint.as_deref(),
                config.test_imds_endpoint.as_deref(),
                config.test_authority_endpoint.as_deref(),
            )?;
            let secrets = keyvault::fetch_startup(
                &endpoints,
                Credential::from_env()?,
                &names,
                config.admin_bind.is_some(),
            )?;
            info!(vault = %url, "server vault secrets read");
            Some(secrets)
        }
        None => None,
    };
    let recipient = audit_recipient(config, vault.as_ref())?;
    let admin_vault = vault
        .as_ref()
        .and_then(|secrets| secrets.admin_identities.as_ref())
        .map(|text| {
            crate::admin::allowlist::parse_strict("the vault secret admin-identities", text)
        })
        .transpose()?;
    let sign_in = match (file, store) {
        (Some(file), Some(store)) => {
            let built = build(
                config,
                file,
                vault.as_mut(),
                Arc::clone(store),
                recipient.clone(),
            )?;
            Some(Arc::new(built))
        }
        _ => None,
    };
    Ok(Prepared {
        sign_in,
        recipient,
        admin_vault,
    })
}

fn audit_recipient(
    config: &Config,
    vault: Option<&StartupSecrets>,
) -> Result<Option<Recipient>, StartupError> {
    let audit = |error: seal::SealError| StartupError::Audit(error.to_string());
    let (key, kid) = match (vault, &config.audit_public_key, &config.audit_key_id) {
        (Some(vault), _, _) => (
            seal::parse_public_key(&vault.audit_public_key).map_err(audit)?,
            vault.audit_key_id.clone(),
        ),
        (None, Some(path), Some(kid)) => {
            let key = seal::load_public_key(path).map_err(audit)?;
            warn!(
                "DEVELOPMENT KEY: the audit public key was loaded from a file; \
                 production reads it from the server vault and the private key stays in the escrow vault"
            );
            (key, kid.clone())
        }
        _ => return Ok(None),
    };
    let bits = seal::key_bits(&key);
    if bits < seal::RECOMMENDED_KEY_BITS {
        warn!(bits, "the audit public key is shorter than 3072 bits");
    }
    Ok(Some(Recipient { key, kid }))
}

fn build(
    config: &Config,
    file: AuthFile,
    vault: Option<&mut StartupSecrets>,
    store: Arc<SqliteStore>,
    recipient: Option<Recipient>,
) -> Result<SignIn, StartupError> {
    let Some(websocket) = config.ws_bind else {
        return Err(StartupError::Config(
            "--auth-config requires --ws-bind: sign-in is served there".into(),
        ));
    };
    let (keys, secrets) = key_ring(config, &file, vault)?;
    let mut providers = Vec::new();
    for entry in &file.providers {
        let secret = client_secret(config, &file, &secrets, &entry.client_secret_name)?;
        providers.push(Provider::from_entry(entry, secret)?);
    }
    let base = file
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{websocket}"));
    let base =
        Url::parse(&base).map_err(|error| StartupError::Config(format!("public_url: {error}")))?;
    let origins = PageOrigins::parse(&config.page_origins).map_err(StartupError::Config)?;
    if origins.is_empty() {
        warn!("sign-in has no --page-origin, so no page can sign in");
    }
    if recipient.is_none() {
        warn!("no audit public key: identities are stored without their sealed subject");
    }
    info!(providers = providers.len(), callback = %base, "sign-in is on");
    let mut rp = Rp::new(providers, base)?;
    if let Some(admin) = config.admin_bind {
        let origin = file
            .admin_public_url
            .clone()
            .or_else(|| file.public_url.clone())
            .unwrap_or_else(|| format!("http://{admin}"));
        let origin = Url::parse(&origin)
            .map_err(|error| StartupError::Config(format!("admin_public_url: {error}")))?;
        info!(callback = %origin, "the operator portal's sign-in callback is under /admin/auth/callback");
        rp = rp.with_admin_base(origin);
    }
    Ok(SignIn::new(SignInSetup {
        store,
        keys,
        rp,
        origins,
        enrolment: config.enrolment(),
        session_lifetime: Duration::from_secs(u64::from(config.session_hours) * 3600),
        accounts_per_day: config.accounts_per_day,
        minimum_age: config.minimum_age,
        terms_version: config.terms_version,
        recipient,
        required: config.public,
    }))
}

/// The identity HMAC keys and the client secrets: from the vault, or the development files.
fn key_ring(
    config: &Config,
    file: &AuthFile,
    vault: Option<&mut StartupSecrets>,
) -> Result<(KeyRing, HashMap<String, Zeroizing<String>>), StartupError> {
    let Some(vault) = vault else {
        return Ok((dev_key_ring(config)?, HashMap::new()));
    };
    if !file.dev_secrets.is_empty() {
        return Err(StartupError::Config(
            "dev_secrets in the auth file cannot be used with --key-vault".into(),
        ));
    }
    let mut ring = Vec::new();
    for (version, value) in &vault.hmac_keys {
        ring.push(HmacKey::from_base64(version, value)?);
    }
    Ok((
        KeyRing::new(ring)?,
        std::mem::take(&mut vault.client_secrets),
    ))
}

fn client_secret(
    config: &Config,
    file: &AuthFile,
    secrets: &HashMap<String, Zeroizing<String>>,
    name: &str,
) -> Result<Zeroizing<String>, StartupError> {
    match secrets.get(name) {
        Some(secret) => Ok(secret.clone()),
        None if config.key_vault.is_none() => dev_secret(file, name),
        None => Err(StartupError::Config(format!(
            "secret {name} was not read from the vault"
        ))),
    }
}

fn dev_key_ring(config: &Config) -> Result<KeyRing, StartupError> {
    let Some(dir) = config.data_dir.as_deref() else {
        return Err(StartupError::Config(
            "--auth-config requires --data-dir".into(),
        ));
    };
    let ring = KeyRing::from_dev_file(dir)?;
    warn!(
        path = %dir.join(DEV_KEY_FILE).display(),
        "DEVELOPMENT KEY: the identity HMAC key is a local file; production reads identity-hmac-key \
         from the server vault"
    );
    Ok(ring)
}

/// The auth file's `[dev_secrets]`, or `INNKEEPERD_SECRET_<NAME>` with `-` as `_`.
fn dev_secret(file: &AuthFile, name: &str) -> Result<Zeroizing<String>, StartupError> {
    let from_file = file.dev_secrets.get(name).cloned();
    let variable = format!(
        "{SECRET_ENV_PREFIX}{}",
        name.to_ascii_uppercase().replace('-', "_")
    );
    let Some(value) = from_file.or_else(|| std::env::var(&variable).ok()) else {
        return Err(StartupError::Config(format!(
            "client secret {name}: not in [dev_secrets] and {variable} is not set"
        )));
    };
    warn!(
        secret = name,
        "DEVELOPMENT KEY: a client secret came from the auth file or the environment"
    );
    Ok(Zeroizing::new(value))
}
