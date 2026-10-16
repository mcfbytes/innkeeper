//! How innkeeperd proves itself to Entra ID for the vault: managed identity, client secret or
//! certificate (docs/server/innkeeperd.md section 8.3).

use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use super::{join, Endpoints, VaultError};

const IMDS_API_VERSION: &str = "2018-02-01";
const VAULT_RESOURCE: &str = "https://vault.azure.net";
const ASSERTION_LIFETIME: i64 = 600;

/// How innkeeperd proves itself to Entra ID.
pub(crate) enum Credential {
    /// The VM's managed identity; a user-assigned one is named by its client id.
    Managed { client_id: Option<String> },
    Secret {
        tenant: String,
        client_id: String,
        secret: Zeroizing<String>,
    },
    Certificate {
        tenant: String,
        client_id: String,
        key: Box<RsaPrivateKey>,
        thumbprint: String,
    },
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Credential::Managed { .. } => "Credential::Managed",
            Credential::Secret { .. } => "Credential::Secret",
            Credential::Certificate { .. } => "Credential::Certificate",
        })
    }
}

impl Credential {
    /// `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and a secret or certificate path pick a service
    /// principal; otherwise the managed identity (user-assigned when `AZURE_CLIENT_ID` is set).
    pub(crate) fn from_env() -> Result<Self, VaultError> {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let secret = var("AZURE_CLIENT_SECRET").map(Zeroizing::new);
        let certificate = var("AZURE_CLIENT_CERTIFICATE_PATH");
        match (
            var("AZURE_TENANT_ID"),
            var("AZURE_CLIENT_ID"),
            secret,
            certificate,
        ) {
            (Some(tenant), Some(client_id), Some(secret), _) => {
                check_tenant(&tenant)?;
                Ok(Credential::Secret {
                    tenant,
                    client_id,
                    secret,
                })
            }
            (Some(tenant), Some(client_id), None, Some(path)) => {
                check_tenant(&tenant)?;
                let pem = Zeroizing::new(
                    std::fs::read_to_string(&path)
                        .map_err(|error| VaultError::Certificate(format!("{path}: {error}")))?,
                );
                Credential::from_certificate_pem(tenant, client_id, &pem)
            }
            (None, client_id, None, None) => Ok(Credential::Managed { client_id }),
            _ => Err(VaultError::Credential(
                "set AZURE_TENANT_ID, AZURE_CLIENT_ID and AZURE_CLIENT_SECRET or \
                 AZURE_CLIENT_CERTIFICATE_PATH, or none of them for the managed identity"
                    .to_owned(),
            )),
        }
    }

    /// A PEM holding the certificate and its RSA private key, as Azure tooling writes it.
    pub(crate) fn from_certificate_pem(
        tenant: String,
        client_id: String,
        pem: &str,
    ) -> Result<Self, VaultError> {
        let bad = |why: &str| VaultError::Certificate(why.to_owned());
        let der = pem_body(pem, "CERTIFICATE").ok_or_else(|| bad("no CERTIFICATE block"))?;
        let key = match (
            pem_block(pem, "PRIVATE KEY"),
            pem_block(pem, "RSA PRIVATE KEY"),
        ) {
            (Some(block), _) => RsaPrivateKey::from_pkcs8_pem(block)
                .map_err(|_| bad("the PRIVATE KEY is not RSA"))?,
            (None, Some(block)) => {
                RsaPrivateKey::from_pkcs1_pem(block).map_err(|_| bad("bad RSA PRIVATE KEY"))?
            }
            (None, None) => return Err(bad("no private key block")),
        };
        Ok(Credential::Certificate {
            tenant,
            client_id,
            key: Box::new(key),
            thumbprint: BASE64URL.encode(Sha1::digest(&der)),
        })
    }
}

fn check_tenant(tenant: &str) -> Result<(), VaultError> {
    let ok = tenant
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    if ok && !tenant.is_empty() {
        Ok(())
    } else {
        Err(VaultError::Credential(
            "AZURE_TENANT_ID is not a tenant id".to_owned(),
        ))
    }
}

fn pem_block<'a>(pem: &'a str, label: &str) -> Option<&'a str> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = pem.find(&begin)?;
    let stop = pem.get(start..)?.find(&end)? + start + end.len();
    pem.get(start..stop)
}

fn pem_body(pem: &str, label: &str) -> Option<Vec<u8>> {
    let block = pem_block(pem, label)?;
    let inner: String = block
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    BASE64.decode(inner.trim()).ok()
}

impl Credential {
    /// The token request for this credential; the caller sends it and reads `access_token`.
    pub(super) fn token_request(
        &self,
        http: &reqwest::Client,
        endpoints: &Endpoints,
    ) -> Result<reqwest::RequestBuilder, String> {
        Ok(match self {
            Credential::Managed { client_id } => {
                let mut url = join(&endpoints.imds, "metadata/identity/oauth2/token")?;
                url.query_pairs_mut()
                    .append_pair("api-version", IMDS_API_VERSION)
                    .append_pair("resource", VAULT_RESOURCE);
                if let Some(id) = client_id {
                    url.query_pairs_mut().append_pair("client_id", id);
                }
                http.get(url).header("Metadata", "true")
            }
            Credential::Secret {
                tenant,
                client_id,
                secret,
            } => {
                let url = token_url(endpoints, tenant)?;
                let form = [
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id.as_str()),
                    ("client_secret", secret.as_str()),
                    ("scope", "https://vault.azure.net/.default"),
                ];
                http.post(url).form(&form)
            }
            Credential::Certificate {
                tenant,
                client_id,
                key,
                thumbprint,
            } => {
                let url = token_url(endpoints, tenant)?;
                let assertion = client_assertion(
                    key,
                    thumbprint,
                    client_id,
                    url.as_str(),
                    crate::dates::unix_now(),
                );
                let form = [
                    ("grant_type", "client_credentials"),
                    ("client_id", client_id.as_str()),
                    (
                        "client_assertion_type",
                        "urn:ietf:params:oauth:client-assertion-type:jwt-bearer",
                    ),
                    ("client_assertion", assertion.as_str()),
                    ("scope", "https://vault.azure.net/.default"),
                ];
                http.post(url).form(&form)
            }
        })
    }
}

fn token_url(endpoints: &Endpoints, tenant: &str) -> Result<Url, String> {
    join(&endpoints.authority, &format!("{tenant}/oauth2/v2.0/token"))
}

/// The signed JWT Entra ID takes in place of a client secret: RS256 with the certificate's
/// SHA-1 thumbprint as `x5t`.
fn client_assertion(
    key: &RsaPrivateKey,
    thumbprint: &str,
    client_id: &str,
    audience: &str,
    now: i64,
) -> String {
    let mut jti = [0u8; 16];
    aes_gcm::aead::rand_core::RngCore::fill_bytes(&mut aes_gcm::aead::OsRng, &mut jti);
    let header = json!({"alg": "RS256", "typ": "JWT", "x5t": thumbprint});
    let claims = json!({
        "aud": audience,
        "iss": client_id,
        "sub": client_id,
        "jti": BASE64URL.encode(jti),
        "nbf": now,
        "iat": now,
        "exp": now + ASSERTION_LIFETIME,
    });
    let input = format!(
        "{}.{}",
        BASE64URL.encode(header.to_string()),
        BASE64URL.encode(claims.to_string())
    );
    let signer = SigningKey::<Sha256>::new(key.clone());
    let signature = signer.sign(input.as_bytes()).to_vec();
    format!("{input}.{}", BASE64URL.encode(signature))
}
