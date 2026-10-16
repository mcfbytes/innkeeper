//! The identity HMAC key ring, session tokens and generated INN passwords
//! (docs/server/privacy.md section 7).

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};
use base64::Engine;
use hmac::{Hmac, Mac};
use innkeeper_world::EncodedPassword;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

/// An HMAC key shorter than this is refused: it is the whole secret behind the identity column.
pub(crate) const MIN_HMAC_KEY_LEN: usize = 32;
pub(crate) const DEV_KEY_FILE: &str = "dev-identity.key";
const DEV_KEY_VERSION: &str = "dev-identity.key";
const TOKEN_BYTES: usize = 32;
/// The INN password rule is 6 to 10 of A-Z and 0-9; eight keeps every client field happy.
const PASSWORD_LEN: usize = 8;
const PASSWORD_ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
/// `script.095` export 4 XORs the uppercased password with this, byte for byte.
const PASSWORD_MASK: &[u8; 10] = b"M34546788S";

pub(crate) type IdentityKey = [u8; 32];

#[derive(Debug, Error)]
pub(crate) enum KeyError {
    #[error("the identity HMAC key {0} is not base64 of at least 32 bytes")]
    Malformed(String),
    #[error("no identity HMAC key is available")]
    Empty,
    #[error("cannot use {path}: {source}")]
    DevFile { path: String, source: io::Error },
}

/// One version of the identity HMAC secret. The bytes are wiped when it is dropped.
pub(crate) struct HmacKey {
    version: String,
    secret: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for HmacKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HmacKey")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl HmacKey {
    /// Parses a secret value: standard base64, surrounding whitespace ignored.
    pub(crate) fn from_base64(version: &str, text: &str) -> Result<Self, KeyError> {
        let secret = BASE64
            .decode(text.trim())
            .map(Zeroizing::new)
            .map_err(|_| KeyError::Malformed(version.to_owned()))?;
        if secret.len() < MIN_HMAC_KEY_LEN {
            return Err(KeyError::Malformed(version.to_owned()));
        }
        Ok(HmacKey {
            version: version.to_owned(),
            secret,
        })
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    /// `HMAC-SHA256(key, issuer || 0x00 || subject)`.
    pub(crate) fn identity_key(&self, issuer: &str, subject: &str) -> IdentityKey {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.secret)
            .unwrap_or_else(|_| unreachable!("HMAC accepts a key of any length"));
        mac.update(issuer.as_bytes());
        mac.update(&[0]);
        mac.update(subject.as_bytes());
        mac.finalize().into_bytes().into()
    }
}

/// Every enabled version of the identity secret, the current one first.
#[derive(Debug)]
pub(crate) struct KeyRing {
    current: HmacKey,
    older: Vec<HmacKey>,
}

impl KeyRing {
    pub(crate) fn new(keys: Vec<HmacKey>) -> Result<Self, KeyError> {
        let mut keys = keys.into_iter();
        let current = keys.next().ok_or(KeyError::Empty)?;
        Ok(KeyRing {
            current,
            older: keys.collect(),
        })
    }

    pub(crate) fn current(&self) -> &HmacKey {
        &self.current
    }

    /// The key each version gives this subject, newest first: the rotation lookup order.
    pub(crate) fn candidates(&self, issuer: &str, subject: &str) -> Vec<(&str, IdentityKey)> {
        std::iter::once(&self.current)
            .chain(&self.older)
            .map(|key| (key.version(), key.identity_key(issuer, subject)))
            .collect()
    }

    /// The local development key in `data_dir`, created (mode 0600) on first use.
    pub(crate) fn from_dev_file(data_dir: &Path) -> Result<Self, KeyError> {
        let path = data_dir.join(DEV_KEY_FILE);
        let failed = |source| KeyError::DevFile {
            path: path.display().to_string(),
            source,
        };
        if !path.exists() {
            let mut secret = Zeroizing::new(vec![0u8; MIN_HMAC_KEY_LEN]);
            OsRng.fill_bytes(&mut secret);
            write_private(&path, BASE64.encode(&*secret).as_bytes()).map_err(failed)?;
        }
        let text = Zeroizing::new(std::fs::read_to_string(&path).map_err(failed)?);
        KeyRing::new(vec![HmacKey::from_base64(DEV_KEY_VERSION, &text)?])
    }
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;
    file.write_all(b"\n")
}

/// The pseudonymous label the page shows: four hex digits of the identity key.
pub(crate) fn handle(key: &IdentityKey) -> String {
    let [a, b, ..] = *key;
    format!("Caller {a:02X}{b:02X}")
}

/// A fresh opaque session token: 32 random bytes, base64url.
pub(crate) fn new_token() -> Zeroizing<String> {
    let mut bytes = Zeroizing::new([0u8; TOKEN_BYTES]);
    OsRng.fill_bytes(&mut *bytes);
    Zeroizing::new(BASE64URL.encode(*bytes))
}

/// What the `sessions` table keeps instead of the token.
pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// A random INN password of `PASSWORD_LEN` characters from A-Z and 0-9.
pub(crate) fn new_password() -> Zeroizing<String> {
    let mut password = Zeroizing::new(String::with_capacity(PASSWORD_LEN));
    let limit = 252u8; // 7 * 36: rejection keeps every character equally likely
    while password.len() < PASSWORD_LEN {
        let mut byte = [0u8; 1];
        OsRng.fill_bytes(&mut byte);
        let [value] = byte;
        if value < limit {
            let index = usize::from(value % 36);
            if let Some(&character) = PASSWORD_ALPHABET.get(index) {
                password.push(char::from(character));
            }
        }
    }
    password
}

/// The 10-byte form the client stores in `PASS_SET.DTA` and sends in its Login
/// (docs/protocol/messages.md section 4.1). `None` outside the 6-to-10 A-Z/0-9 rule.
pub(crate) fn encode_password(password: &str) -> Option<EncodedPassword> {
    let upper = password.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if !(6..=10).contains(&bytes.len()) || !bytes.iter().all(u8::is_ascii_alphanumeric) {
        return None;
    }
    let mut out = [0u8; 10];
    for ((slot, byte), mask) in out.iter_mut().zip(bytes).zip(PASSWORD_MASK) {
        *slot = byte ^ mask;
    }
    let n = bytes.len();
    for i in 0..n {
        let source = *out.get(i)?;
        *out.get_mut((i + 1) % n)? ^= source;
    }
    Some(EncodedPassword(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> HmacKey {
        HmacKey::from_base64("v", &BASE64.encode([byte; 32])).unwrap()
    }

    #[test]
    fn the_identity_key_is_hmac_sha256_of_issuer_nul_subject() {
        let key = key(7);
        let expected: IdentityKey = {
            let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&[7; 32]).unwrap();
            mac.update(b"https://issuer.example\0subject-1");
            mac.finalize().into_bytes().into()
        };
        assert_eq!(
            key.identity_key("https://issuer.example", "subject-1"),
            expected
        );
        assert_ne!(
            key.identity_key("https://issuer.example", "subject-1"),
            key.identity_key("https://issuer.exampl", "esubject-1"),
            "the NUL separator keeps issuer and subject apart"
        );
    }

    #[test]
    fn another_key_gives_another_identity() {
        let a = key(1).identity_key("i", "s");
        let b = key(2).identity_key("i", "s");
        assert_ne!(a, b);
    }

    #[test]
    fn a_short_or_malformed_secret_is_refused() {
        assert!(HmacKey::from_base64("v", &BASE64.encode([1; 31])).is_err());
        assert!(HmacKey::from_base64("v", "not base64 at all!").is_err());
        assert!(HmacKey::from_base64("v", &format!("{}\n", BASE64.encode([1; 32]))).is_ok());
        assert!(matches!(KeyRing::new(Vec::new()), Err(KeyError::Empty)));
    }

    #[test]
    fn the_ring_tries_the_current_version_first() {
        let ring = KeyRing::new(vec![
            HmacKey::from_base64("new", &BASE64.encode([2; 32])).unwrap(),
            HmacKey::from_base64("old", &BASE64.encode([1; 32])).unwrap(),
        ])
        .unwrap();
        let candidates = ring.candidates("i", "s");
        assert_eq!(candidates[0].0, "new");
        assert_eq!(candidates[1], ("old", key(1).identity_key("i", "s")));
        assert_eq!(ring.current().version(), "new");
    }

    #[test]
    fn the_handle_is_four_hex_digits_of_the_key() {
        let mut identity = [0u8; 32];
        identity[0] = 0x7f;
        identity[1] = 0x3a;
        assert_eq!(handle(&identity), "Caller 7F3A");
    }

    #[test]
    fn tokens_are_random_and_only_their_hash_is_kept() {
        let first = new_token();
        let second = new_token();
        assert_ne!(*first, *second);
        assert_eq!(first.len(), 43);
        assert_eq!(token_hash(&first), token_hash(&first.clone()));
        assert_eq!(
            token_hash(&first).to_vec(),
            Sha256::digest(first.as_bytes()).to_vec()
        );
        assert_ne!(token_hash(&first), token_hash(&second));
    }

    #[test]
    fn the_password_encoder_matches_the_live_capture() {
        let encoded = encode_password("SWORDFISH").unwrap();
        assert_eq!(
            encoded.0,
            [0x1d, 0x7a, 0x01, 0x66, 0x16, 0x66, 0x18, 0x73, 0x03, 0x00]
        );
        assert_eq!(encode_password("swordfish"), Some(encoded));
        assert_eq!(encode_password("ABCDEF").unwrap().0[6..], [0, 0, 0, 0]);
        for bad in ["SHORT", "WAYTOOLONGPW", "HAS SPACE1", "BAD-CHARS1", ""] {
            assert_eq!(encode_password(bad), None, "{bad}");
        }
    }

    #[test]
    fn generated_passwords_follow_the_inn_rule() {
        for _ in 0..50 {
            let password = new_password();
            assert_eq!(password.len(), PASSWORD_LEN);
            assert!(password
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()));
            assert!(encode_password(&password).is_some());
        }
    }

    #[test]
    fn the_dev_key_file_is_created_private_and_reused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("inn-devkey-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = KeyRing::from_dev_file(&dir)
            .unwrap()
            .current()
            .identity_key("i", "s");
        let mode = std::fs::metadata(dir.join(DEV_KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        let again = KeyRing::from_dev_file(&dir)
            .unwrap()
            .current()
            .identity_key("i", "s");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(first, again);
    }
}
