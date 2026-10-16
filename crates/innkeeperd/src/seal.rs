//! Envelope encryption for sealed connection records (docs/server/privacy.md).
//! The server only wraps a data key; it never decrypts. Opening is an offline tool.

use std::io;
use std::path::Path;

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};

type AesNonce = Nonce<<Aes256Gcm as AeadCore>::NonceSize>;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde_json::{json, Value};
use sha2::Sha256;
use thiserror::Error;

const VERSION: u64 = 1;
/// OAEP-SHA-256 of a 32-byte key needs a modulus of at least this many bits.
pub(crate) const MIN_KEY_BITS: usize = 2048;
/// The escrow vault key is this size or larger (docs/server/privacy.md).
pub(crate) const RECOMMENDED_KEY_BITS: usize = 3072;
/// Local development keys. Production keys are generated inside the escrow vault.
pub(crate) const DEV_KEY_BITS: usize = 3072;
const DATA_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// The operator public key records are sealed to, and the identifier stored with them.
#[derive(Clone, Debug)]
pub(crate) struct Recipient {
    pub key: RsaPublicKey,
    pub kid: String,
}

#[derive(Debug, Error)]
pub(crate) enum SealError {
    #[error("the audit public key could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("the audit public key is not an SPKI PEM RSA key: {0}")]
    PublicKey(String),
    #[error("the audit private key is not a PEM RSA key: {0}")]
    PrivateKey(String),
    #[error("the audit public key is {bits} bits; it must be at least {MIN_KEY_BITS}")]
    KeyTooSmall { bits: usize },
    #[error("a sealed record is not a version 1 envelope")]
    Envelope,
    #[error("the data key could not be wrapped")]
    Wrap,
    #[error("the data key could not be unwrapped")]
    Unwrap,
    #[error("a data key must be {DATA_KEY_LEN} bytes")]
    DataKey,
    #[error("the sealed record failed authentication")]
    Authentication,
}

/// What an export line needs out of a sealed blob.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvelopeView {
    pub kid: String,
    pub wrapped_key: Vec<u8>,
}

pub(crate) fn load_public_key(path: &Path) -> Result<RsaPublicKey, SealError> {
    let pem = std::fs::read_to_string(path)?;
    let key = RsaPublicKey::from_public_key_pem(&pem)
        .map_err(|error| SealError::PublicKey(error.to_string()))?;
    let bits = key.size().saturating_mul(8);
    if bits < MIN_KEY_BITS {
        return Err(SealError::KeyTooSmall { bits });
    }
    Ok(key)
}

pub(crate) fn load_private_key(path: &Path) -> Result<RsaPrivateKey, SealError> {
    let pem = std::fs::read_to_string(path)?;
    RsaPrivateKey::from_pkcs8_pem(&pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(&pem))
        .map_err(|error| SealError::PrivateKey(error.to_string()))
}

pub(crate) fn key_bits(key: &RsaPublicKey) -> usize {
    key.size().saturating_mul(8)
}

/// Seal `plaintext` to `recipient`. The result is the JSON envelope bytes.
pub(crate) fn seal(recipient: &Recipient, plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
    let data_key = Aes256Gcm::generate_key(OsRng);
    let nonce = Aes256Gcm::generate_nonce(OsRng);
    let ciphertext = Aes256Gcm::new(&data_key)
        .encrypt(&nonce, plaintext)
        .map_err(|_| SealError::Authentication)?;
    let wrapped = wrap_key(&recipient.key, data_key.as_slice())?;
    let envelope = json!({
        "v": VERSION,
        "kid": recipient.kid,
        "wrapped_key": BASE64.encode(wrapped),
        "nonce": BASE64.encode(nonce),
        "ciphertext": BASE64.encode(ciphertext),
    });
    serde_json::to_vec(&envelope).map_err(|_| SealError::Envelope)
}

pub(crate) fn open_with_private_key(
    sealed: &[u8],
    private_key: &RsaPrivateKey,
) -> Result<Vec<u8>, SealError> {
    let data_key = unwrap_key(private_key, &view(sealed)?.wrapped_key)?;
    open_with_data_key(sealed, &data_key)
}

pub(crate) fn open_with_data_key(sealed: &[u8], data_key: &[u8]) -> Result<Vec<u8>, SealError> {
    let value = envelope(sealed)?;
    let nonce_bytes = field_bytes(&value, "nonce")?;
    let ciphertext = field_bytes(&value, "ciphertext")?;
    let nonce = nonce_of(&nonce_bytes)?;
    let key = data_key_of(data_key)?;
    Aes256Gcm::new(&key)
        .decrypt(&nonce, ciphertext.as_slice())
        .map_err(|_| SealError::Authentication)
}

pub(crate) fn view(sealed: &[u8]) -> Result<EnvelopeView, SealError> {
    let value = envelope(sealed)?;
    let kid = value
        .get("kid")
        .and_then(Value::as_str)
        .ok_or(SealError::Envelope)?;
    Ok(EnvelopeView {
        kid: kid.to_owned(),
        wrapped_key: field_bytes(&value, "wrapped_key")?,
    })
}

/// A local RSA pair for development. The private key is written mode 0600.
pub(crate) fn write_dev_keypair(dir: &Path, bits: usize) -> Result<(), SealError> {
    std::fs::create_dir_all(dir)?;
    let private_path = dir.join("audit-private.pem");
    let public_path = dir.join("audit-public.pem");
    if private_path.exists() || public_path.exists() {
        return Err(SealError::Io(io::Error::other(
            "refusing to overwrite an audit key; choose an empty --out-dir",
        )));
    }
    let mut rng = OsRng;
    let private =
        RsaPrivateKey::new(&mut rng, bits).map_err(|_| SealError::KeyTooSmall { bits })?;
    let public = RsaPublicKey::from(&private);
    let private_pem = private
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|error| SealError::PrivateKey(error.to_string()))?;
    write_private(&private_path, private_pem.as_bytes())?;
    let public_pem = public
        .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|error| SealError::PublicKey(error.to_string()))?;
    std::fs::write(public_path, public_pem)?;
    Ok(())
}

fn write_private(path: &Path, pem: &[u8]) -> Result<(), io::Error> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    std::io::Write::write_all(&mut file, pem)
}

fn wrap_key(key: &RsaPublicKey, data_key: &[u8]) -> Result<Vec<u8>, SealError> {
    let mut rng = OsRng;
    key.encrypt(&mut rng, Oaep::new::<Sha256>(), data_key)
        .map_err(|_| SealError::Wrap)
}

pub(crate) fn unwrap_key(key: &RsaPrivateKey, wrapped: &[u8]) -> Result<Vec<u8>, SealError> {
    key.decrypt(Oaep::new::<Sha256>(), wrapped)
        .map_err(|_| SealError::Unwrap)
}

fn envelope(sealed: &[u8]) -> Result<Value, SealError> {
    let value: Value = serde_json::from_slice(sealed).map_err(|_| SealError::Envelope)?;
    match value.get("v").and_then(Value::as_u64) {
        Some(VERSION) => Ok(value),
        _ => Err(SealError::Envelope),
    }
}

fn field_bytes(value: &Value, name: &str) -> Result<Vec<u8>, SealError> {
    let text = value
        .get(name)
        .and_then(Value::as_str)
        .ok_or(SealError::Envelope)?;
    decode_b64(text)
}

pub(crate) fn decode_b64(text: &str) -> Result<Vec<u8>, SealError> {
    let text = text.trim();
    BASE64
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(text))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text))
        .map_err(|_| SealError::Envelope)
}

fn data_key_of(bytes: &[u8]) -> Result<Key<Aes256Gcm>, SealError> {
    if bytes.len() != DATA_KEY_LEN {
        return Err(SealError::DataKey);
    }
    let mut key = Key::<Aes256Gcm>::default();
    key.copy_from_slice(bytes);
    Ok(key)
}

fn nonce_of(bytes: &[u8]) -> Result<AesNonce, SealError> {
    if bytes.len() != NONCE_LEN {
        return Err(SealError::Envelope);
    }
    let mut nonce = AesNonce::default();
    nonce.copy_from_slice(bytes);
    Ok(nonce)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;

    use super::*;

    fn key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut OsRng, MIN_KEY_BITS).unwrap())
    }

    fn recipient(kid: &str) -> Recipient {
        Recipient {
            key: RsaPublicKey::from(key()),
            kid: kid.to_owned(),
        }
    }

    #[test]
    fn sealing_round_trips_with_the_private_key_and_the_data_key_and_rejects_tampering() {
        let recipient = recipient("dev-kid");
        let plaintext = br#"{"peer_ip":"203.0.113.5","peer_port":443}"#;
        let sealed = seal(&recipient, plaintext).unwrap();
        assert_eq!(
            open_with_private_key(&sealed, key()).unwrap(),
            plaintext.to_vec()
        );
        let wrapped = view(&sealed).unwrap();
        assert_eq!(wrapped.kid, "dev-kid");
        let data_key = unwrap_key(key(), &wrapped.wrapped_key).unwrap();
        assert_eq!(open_with_data_key(&sealed, &data_key).unwrap(), plaintext);

        let mut value: Value = serde_json::from_slice(&sealed).unwrap();
        let mut ciphertext = decode_b64(value["ciphertext"].as_str().unwrap()).unwrap();
        ciphertext[0] ^= 0xff;
        value["ciphertext"] = Value::String(BASE64.encode(&ciphertext));
        let tampered = serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            open_with_private_key(&tampered, key()),
            Err(SealError::Authentication)
        ));
        assert!(matches!(
            open_with_data_key(&tampered, &data_key),
            Err(SealError::Authentication)
        ));
    }

    #[test]
    fn a_key_shorter_than_2048_bits_is_refused() {
        let private = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
        let public = RsaPublicKey::from(&private);
        let pem = public
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let dir = std::env::temp_dir().join(format!(
            "innkeeperd-seal-short-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("small.pem"), pem).unwrap();
        let error = load_public_key(&dir.join("small.pem")).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(error, SealError::KeyTooSmall { bits: 1024 }));
    }

    #[test]
    fn a_dev_pair_is_mode_0600() {
        let dir = std::env::temp_dir().join(format!(
            "innkeeperd-seal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        write_dev_keypair(&dir, MIN_KEY_BITS).unwrap();
        let mode = std::fs::metadata(dir.join("audit-private.pem"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let loaded = load_public_key(&dir.join("audit-public.pem")).unwrap();
        assert!(key_bits(&loaded) >= MIN_KEY_BITS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
