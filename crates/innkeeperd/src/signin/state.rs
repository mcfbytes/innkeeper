//! The OAuth `state` value carries the whole pending login, sealed under a per-process key, so an
//! unauthenticated `/auth/login` stores nothing (docs/server/innkeeperd.md section 8.6).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64URL;
use base64::Engine;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::rp::Audience;

/// Bytes of the AES-GCM nonce at the head of a sealed state; also the state's replay-set id.
const NONCE_LEN: usize = 12;
/// A state longer than this, as it comes back, is not one of ours.
const MAX_STATE_TEXT: usize = 4096;
/// Callbacks remembered at once. When full the soonest-expiring entry goes, never a new login.
const MAX_USED: usize = 65_536;

/// What a login needs again at its callback. Kept nowhere but inside the sealed `state`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LoginState {
    pub(crate) provider: String,
    pub(crate) audience: Audience,
    /// The PKCE code verifier and, for OIDC, the ID token nonce.
    pub(crate) verifier: String,
    pub(crate) nonce: Option<String>,
    /// Unix milliseconds after which the callback is refused.
    pub(crate) expires_at: i64,
    /// Where the player's page wants the result; `None` for the operator portal.
    pub(crate) page: Option<PageState>,
}

/// The checked page return of a player login, and the nonce the page chose to recognise its result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PageState {
    pub(crate) url: String,
    pub(crate) origin: String,
    pub(crate) redirect: bool,
    pub(crate) attest: Option<(u32, u32)>,
    pub(crate) page_nonce: Option<String>,
}

/// Seals and opens login states, and refuses a state that has already come back once.
pub(crate) struct LoginStates {
    cipher: Aes256Gcm,
    used: Mutex<HashMap<[u8; NONCE_LEN], i64>>,
}

impl std::fmt::Debug for LoginStates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginStates").finish_non_exhaustive()
    }
}

impl LoginStates {
    /// A fresh random key: a restart forgets every login in flight, as the memory map did.
    pub(crate) fn new() -> Self {
        LoginStates {
            cipher: Aes256Gcm::new(&Aes256Gcm::generate_key(OsRng)),
            used: Mutex::new(HashMap::new()),
        }
    }

    /// base64url of nonce and ciphertext; the audience is bound in as associated data.
    pub(crate) fn seal(&self, state: &LoginState) -> Option<String> {
        let plain = Zeroizing::new(serde_json::to_vec(state).ok()?);
        let nonce = Aes256Gcm::generate_nonce(OsRng);
        let aad = associated(state.audience);
        let payload = Payload {
            msg: plain.as_slice(),
            aad: &aad,
        };
        let sealed = self.cipher.encrypt(&nonce, payload).ok()?;
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&sealed);
        Some(BASE64URL.encode(bytes))
    }

    /// The login behind a callback's `state`, once: a forged, expired, foreign-audience or repeated
    /// state is `None`.
    pub(crate) fn open(&self, text: &str, audience: Audience, now: i64) -> Option<LoginState> {
        if text.len() > MAX_STATE_TEXT {
            return None;
        }
        let bytes = BASE64URL.decode(text).ok()?;
        let (head, sealed) = bytes.split_at_checked(NONCE_LEN)?;
        let id: [u8; NONCE_LEN] = head.try_into().ok()?;
        let aad = associated(audience);
        let payload = Payload {
            msg: sealed,
            aad: &aad,
        };
        let plain = Zeroizing::new(self.cipher.decrypt(Nonce::from_slice(head), payload).ok()?);
        let state: LoginState = serde_json::from_slice(&plain).ok()?;
        if state.audience != audience || state.expires_at <= now {
            return None;
        }
        self.first_use(id, state.expires_at, now).then_some(state)
    }

    /// True the first time `id` comes back. Entries leave once their state has expired anyway.
    fn first_use(&self, id: [u8; NONCE_LEN], expires_at: i64, now: i64) -> bool {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        if used.contains_key(&id) {
            return false;
        }
        if used.len() >= MAX_USED {
            used.retain(|_, until| *until > now);
        }
        if used.len() >= MAX_USED {
            let soonest = used
                .iter()
                .min_by_key(|(_, until)| **until)
                .map(|(k, _)| *k);
            if let Some(key) = soonest {
                used.remove(&key);
            }
        }
        used.insert(id, expires_at);
        true
    }
}

fn associated(audience: Audience) -> Vec<u8> {
    let tag: &[u8] = match audience {
        Audience::Player => b"player",
        Audience::Admin => b"admin",
    };
    [b"inn-login-state-v1\0".as_slice(), tag].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(audience: Audience, expires_at: i64) -> LoginState {
        LoginState {
            provider: "dev".into(),
            audience,
            verifier: "v".repeat(43),
            nonce: Some("n".repeat(22)),
            expires_at,
            page: Some(PageState {
                url: "https://play.example.org/inn.html".into(),
                origin: "https://play.example.org".into(),
                redirect: true,
                attest: Some((18, 1)),
                page_nonce: Some("abcDEF123_-abcDEF123_".into()),
            }),
        }
    }

    #[test]
    fn a_state_opens_once_for_its_own_audience_before_it_expires() {
        let states = LoginStates::new();
        let login = state(Audience::Player, 1_000);
        let text = states.seal(&login).unwrap();
        assert!(!text.contains("abcDEF") && !text.contains("play.example"));
        assert_eq!(states.open(&text, Audience::Admin, 10), None);
        assert_eq!(states.open(&text, Audience::Player, 10), Some(login));
        assert_eq!(states.open(&text, Audience::Player, 10), None);
        let late = states.seal(&state(Audience::Player, 1_000)).unwrap();
        assert_eq!(states.open(&late, Audience::Player, 1_000), None);
    }

    #[test]
    fn a_forged_or_tampered_state_or_another_process_key_is_refused() {
        let states = LoginStates::new();
        let text = states.seal(&state(Audience::Admin, 1_000)).unwrap();
        let mut bytes = BASE64URL.decode(&text).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let tampered = BASE64URL.encode(bytes);
        assert_eq!(states.open(&tampered, Audience::Admin, 10), None);
        assert_eq!(LoginStates::new().open(&text, Audience::Admin, 10), None);
        for junk in ["", "unknown", "AAAA", &"A".repeat(MAX_STATE_TEXT + 1)] {
            assert_eq!(states.open(junk, Audience::Admin, 10), None, "{junk}");
        }
    }

    #[test]
    fn a_full_replay_set_drops_the_soonest_entry_and_never_refuses_a_new_login() {
        let states = LoginStates::new();
        for n in 0..MAX_USED {
            let mut id = [0u8; NONCE_LEN];
            id[..8].copy_from_slice(&(n as u64).to_le_bytes());
            assert!(states.first_use(id, 2_000 + n as i64, 10));
        }
        let text = states.seal(&state(Audience::Player, 9_999_999)).unwrap();
        assert!(states.open(&text, Audience::Player, 10).is_some());
        assert_eq!(states.used.lock().unwrap().len(), MAX_USED);
        assert!(states.open(&text, Audience::Player, 10).is_none());
    }
}
