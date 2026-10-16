//! The vault client against a mock IMDS, Entra ID token endpoint and Key Vault, all on loopback.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};
use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::pkcs8::EncodePrivateKey;
use rsa::signature::Verifier;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use serde_json::json;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use super::*;

const CERT_DER: &[u8] = b"not a real certificate, only its thumbprint matters";

fn test_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| RsaPrivateKey::new(&mut aes_gcm::aead::OsRng, 2048).unwrap())
}

struct Version {
    id: &'static str,
    value: &'static str,
    enabled: bool,
    created: i64,
}

struct Mock {
    base: String,
    /// Bearer tokens the vault accepted, in order.
    seen: Mutex<Vec<String>>,
}

fn hmac_versions() -> Vec<Version> {
    vec![
        Version {
            id: "aaa111",
            value: "b2xkLW9sZC1vbGQtb2xkLW9sZC1vbGQtb2xkLW9sZC0=",
            enabled: true,
            created: 100,
        },
        Version {
            id: "ccc333",
            value: "Y3VycmVudC1jdXJyZW50LWN1cnJlbnQtY3VycmVudC0=",
            enabled: true,
            created: 300,
        },
        Version {
            id: "bbb222",
            value: "b2Zmb2Zmb2Zmb2Zmb2Zmb2Zmb2Zmb2Zmb2Zmb2Zmb2Y=",
            enabled: false,
            created: 200,
        },
        Version {
            id: "ddd000",
            value: "bWlkZGxlLW1pZGRsZS1taWRkbGUtbWlkZGxlLW1pZGQ=",
            enabled: true,
            created: 150,
        },
    ]
}

fn reply(status: StatusCode, body: Value) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body.to_string())));
    *response.status_mut() = status;
    response
}

async fn handle(mock: Arc<Mock>, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let query = request.uri().query().unwrap_or("").to_owned();
    if path == "/metadata/identity/oauth2/token" {
        let metadata = request
            .headers()
            .get("metadata")
            .map(|v| v.as_bytes().to_vec());
        let ok = metadata.as_deref() == Some(b"true")
            && query.contains("api-version=2018-02-01")
            && query.contains("resource=https%3A%2F%2Fvault.azure.net");
        return match ok {
            true => reply(
                StatusCode::OK,
                json!({"access_token": "mi-token", "expires_in": "3599"}),
            ),
            false => reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"})),
        };
    }
    if path == "/tenant-1/oauth2/v2.0/token" {
        let body = request.into_body().collect().await.unwrap().to_bytes();
        let form: HashMap<String, String> =
            url::form_urlencoded::parse(&body).into_owned().collect();
        return sp_token(&mock, &form);
    }
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let Some(token) = auth.strip_prefix("Bearer ") else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error": {"code": "Unauthorized"}}),
        );
    };
    mock.seen.lock().unwrap().push(token.to_owned());
    if !query.contains("api-version=7.4") {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({"error": {"code": "BadParameter"}}),
        );
    }
    vault(&mock, &path, &query)
}

fn sp_token(mock: &Mock, form: &HashMap<String, String>) -> Response<Full<Bytes>> {
    let get = |name: &str| form.get(name).map(String::as_str);
    if get("grant_type") != Some("client_credentials")
        || get("scope") != Some("https://vault.azure.net/.default")
    {
        return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid_request"}));
    }
    if get("client_secret") == Some("sp-secret") {
        return reply(StatusCode::OK, json!({"access_token": "sp-token"}));
    }
    let Some(assertion) = get("client_assertion") else {
        return reply(StatusCode::UNAUTHORIZED, json!({"error": "invalid_client"}));
    };
    let parts: Vec<&str> = assertion.split('.').collect();
    let header: Value = serde_json::from_slice(&BASE64URL.decode(parts[0]).unwrap()).unwrap();
    let claims: Value = serde_json::from_slice(&BASE64URL.decode(parts[1]).unwrap()).unwrap();
    let signature = Signature::try_from(BASE64URL.decode(parts[2]).unwrap().as_slice()).unwrap();
    let verifier = VerifyingKey::<Sha256>::new(RsaPublicKey::from(test_key()));
    let signed = verifier.verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature);
    let audience = format!("{}/tenant-1/oauth2/v2.0/token", mock.base);
    let thumbprint = BASE64URL.encode(Sha1::digest(CERT_DER));
    let ok = signed.is_ok()
        && header["x5t"] == thumbprint.as_str()
        && claims["aud"] == audience.as_str()
        && claims["iss"] == "app-1"
        && claims["sub"] == "app-1";
    match ok {
        true => reply(StatusCode::OK, json!({"access_token": "cert-token"})),
        false => reply(StatusCode::UNAUTHORIZED, json!({"error": "invalid_client"})),
    }
}

fn secret_body(mock: &Mock, name: &str, version: &Version) -> Value {
    json!({
        "value": version.value,
        "id": format!("{}/secrets/{name}/{}", mock.base, version.id),
        "attributes": {"enabled": version.enabled, "created": version.created},
    })
}

fn vault(mock: &Mock, path: &str, query: &str) -> Response<Full<Bytes>> {
    let not_found = || {
        reply(
            StatusCode::NOT_FOUND,
            json!({"error": {"code": "SecretNotFound"}}),
        )
    };
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let simple = |value: &'static str| Version {
        id: "v1",
        value,
        enabled: true,
        created: 1,
    };
    match segments.as_slice() {
        ["secrets", "identity-hmac-key", "versions"] => {
            let all = hmac_versions();
            let (page, next) = match query.contains("page=2") {
                false => (
                    &all[..2],
                    Some(format!(
                        "{}/secrets/identity-hmac-key/versions?api-version=7.4&page=2",
                        mock.base
                    )),
                ),
                true => (&all[2..], None),
            };
            let value: Vec<Value> = page
                .iter()
                .map(|v| json!({"id": format!("{}/secrets/identity-hmac-key/{}", mock.base, v.id), "attributes": {"enabled": v.enabled, "created": v.created}}))
                .collect();
            reply(StatusCode::OK, json!({"value": value, "nextLink": next}))
        }
        ["secrets", "identity-hmac-key"] => {
            let versions = hmac_versions();
            let current = versions.iter().find(|v| v.id == "ccc333").unwrap();
            reply(
                StatusCode::OK,
                secret_body(mock, "identity-hmac-key", current),
            )
        }
        ["secrets", "identity-hmac-key", id] => {
            match hmac_versions().iter().find(|v| v.id == *id) {
                Some(version) if version.enabled => reply(
                    StatusCode::OK,
                    secret_body(mock, "identity-hmac-key", version),
                ),
                Some(_) => reply(
                    StatusCode::FORBIDDEN,
                    json!({"error": {"code": "Forbidden"}}),
                ),
                None => not_found(),
            }
        }
        ["secrets", "oidc-dev-client-secret"] => reply(
            StatusCode::OK,
            secret_body(mock, "oidc-dev-client-secret", &simple("s3cret")),
        ),
        ["secrets", "audit-public-key"] => reply(
            StatusCode::OK,
            secret_body(
                mock,
                "audit-public-key",
                &simple("-----BEGIN PUBLIC KEY-----\n..."),
            ),
        ),
        ["secrets", "admin-identities"] => reply(
            StatusCode::OK,
            secret_body(mock, "admin-identities", &simple("# staff\nab12, cd34\n")),
        ),
        ["secrets", "audit-key-id"] => reply(
            StatusCode::OK,
            secret_body(
                mock,
                "audit-key-id",
                &simple("https://escrow.vault.azure.net/keys/audit/v9\n"),
            ),
        ),
        _ => not_found(),
    }
}

async fn start_mock() -> (Arc<Mock>, Endpoints) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let base = format!("http://{address}");
    let mock = Arc::new(Mock {
        base: base.clone(),
        seen: Mutex::new(Vec::new()),
    });
    let shared = Arc::clone(&mock);
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let mock = Arc::clone(&shared);
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let mock = Arc::clone(&mock);
                    async move { Ok::<_, Infallible>(handle(mock, request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    let endpoints = Endpoints::new(
        "https://inn-server.vault.azure.net",
        Some(&base),
        Some(&base),
        Some(&base),
    )
    .unwrap();
    (mock, endpoints)
}

#[tokio::test]
async fn a_managed_identity_reads_every_startup_secret() {
    let (mock, endpoints) = start_mock().await;
    let client = VaultClient::new(endpoints, Credential::Managed { client_id: None }).unwrap();
    let names = vec!["oidc-dev-client-secret".to_owned()];
    let secrets = client.startup(&names, false).await.unwrap();
    let versions: Vec<&str> = secrets.hmac_keys.iter().map(|(v, _)| v.as_str()).collect();
    assert_eq!(
        versions,
        ["ccc333", "ddd000", "aaa111"],
        "current first, disabled skipped"
    );
    assert_eq!(
        secrets.client_secrets["oidc-dev-client-secret"].as_str(),
        "s3cret"
    );
    assert!(secrets
        .audit_public_key
        .starts_with("-----BEGIN PUBLIC KEY-----"));
    assert_eq!(
        secrets.audit_key_id,
        "https://escrow.vault.azure.net/keys/audit/v9"
    );
    assert!(mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .all(|token| token == "mi-token"));
}

#[tokio::test]
async fn a_missing_secret_is_an_error_that_names_it_and_not_its_value() {
    let (_mock, endpoints) = start_mock().await;
    let client = VaultClient::new(endpoints, Credential::Managed { client_id: None }).unwrap();
    let error = client
        .startup(&["oidc-absent".to_owned()], false)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("oidc-absent") && error.contains("SecretNotFound"),
        "{error}"
    );
    let refused = client
        .startup(&["../escape".to_owned()], false)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("secret name"), "{refused}");
}

#[tokio::test]
async fn a_service_principal_uses_its_secret_or_a_signed_certificate_assertion() {
    let (mock, endpoints) = start_mock().await;
    let secret = Credential::Secret {
        tenant: "tenant-1".into(),
        client_id: "app-1".into(),
        secret: Zeroizing::new("sp-secret".into()),
    };
    let client = VaultClient::new(endpoints.clone(), secret).unwrap();
    client.startup(&[], false).await.unwrap();
    assert_eq!(
        mock.seen.lock().unwrap().last().map(String::as_str),
        Some("sp-token")
    );

    let key_pem = test_key().to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    let pem = format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n{}",
        BASE64.encode(CERT_DER),
        key_pem.as_str()
    );
    let certificate =
        Credential::from_certificate_pem("tenant-1".into(), "app-1".into(), &pem).unwrap();
    let client = VaultClient::new(endpoints, certificate).unwrap();
    client.startup(&[], false).await.unwrap();
    assert_eq!(
        mock.seen.lock().unwrap().last().map(String::as_str),
        Some("cert-token")
    );
}

#[test]
fn only_a_bare_https_vault_url_is_accepted() {
    assert!(check_vault_url("https://inn-server.vault.azure.net").is_ok());
    assert!(check_vault_url("https://inn-server.vault.azure.net/").is_ok());
    for bad in [
        "http://inn-server.vault.azure.net",
        "https://vault.azure.net",
        "https://inn.vault.azure.net.evil.test",
        "https://inn.vault.azure.net/secrets/x",
        "https://inn.vault.azure.net:8443",
    ] {
        assert!(check_vault_url(bad).is_err(), "{bad}");
    }
    assert_eq!(
        last_segment("https://x.vault.azure.net/secrets/n/abc?x=1"),
        Some("abc")
    );
}

#[tokio::test]
async fn the_admin_allowlist_is_read_only_when_the_portal_is_on_and_may_be_absent() {
    let (_mock, endpoints) = start_mock().await;
    let client = VaultClient::new(endpoints, Credential::Managed { client_id: None }).unwrap();
    let off = client.startup(&[], false).await.unwrap();
    assert!(off.admin_identities.is_none());
    let token = client.token().await.unwrap();
    let absent = client.optional_secret(&token, "admin-none").await.unwrap();
    assert!(absent.is_none(), "a 404 is no list, not an error");
    let on = client.startup(&[], true).await.unwrap();
    assert_eq!(
        on.admin_identities.as_deref().map(String::as_str),
        Some("# staff\nab12, cd34\n")
    );
}
