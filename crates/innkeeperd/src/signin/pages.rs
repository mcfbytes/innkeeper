//! How a login ends for the page: a popup that posts the result to its opener, or a redirect
//! with the result in the fragment.

use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::OsRng;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bytes::Bytes;
use hyper::header::{HeaderValue, LOCATION};
use hyper::StatusCode;
use serde_json::{json, Value};

use super::{Mode, PageReturn};
use crate::http::{self, Reply};

fn popup_page(nonce: &str, message: &str, target: &str) -> String {
    format!(
        r#"<!doctype html><meta charset="utf-8"><title>Signing in</title>
<p id="m">One moment.</p>
<script nonce="{nonce}">
var msg = {message}, target = {target};
if (window.opener) {{ window.opener.postMessage(msg, target); window.close(); }}
document.getElementById('m').textContent = window.opener
  ? 'You can close this window.' : 'This window has no game page to talk to. Close it and try again.';
</script>
"#
    )
}

const EXPIRED: &str = r#"<!doctype html><meta charset="utf-8"><title>Sign-in expired</title>
<p>This sign-in has expired or was already used. Close this window and try again.</p>
"#;

/// How a login ends: a session token, a link proof for the session that asked, or an error code.
#[derive(Clone, Copy, Debug)]
pub(super) enum Outcome<'a> {
    Session(&'a str),
    Link(&'a str),
    Error(&'a str),
}

/// The outcome, delivered the way the page asked for it.
pub(super) fn finish(page: &PageReturn, outcome: Outcome<'_>) -> Reply {
    match page.mode {
        Mode::Redirect => {
            let mut fragment = match outcome {
                Outcome::Session(token) => format!("inn-session={token}"),
                Outcome::Link(proof) => format!("inn-link={proof}"),
                Outcome::Error(code) => format!("inn-error={code}"),
            };
            if let Some(nonce) = &page.page_nonce {
                fragment.push_str(&format!("&inn-nonce={nonce}"));
            }
            let mut url = page.url.clone();
            url.set_fragment(Some(&fragment));
            redirect(url.as_str())
        }
        Mode::Popup => {
            let mut message = match outcome {
                Outcome::Session(token) => json!({"type": "inn-auth", "session": token}),
                Outcome::Link(proof) => json!({"type": "inn-auth", "link": proof}),
                Outcome::Error(code) => json!({"type": "inn-auth", "error": code}),
            };
            if let (Some(nonce), Some(fields)) = (&page.page_nonce, message.as_object_mut()) {
                fields.insert("nonce".to_owned(), json!(nonce));
            }
            popup(&message, &page.origin)
        }
    }
}

/// The posted message goes only to the checked page origin, never `*`.
fn popup(message: &Value, origin: &str) -> Reply {
    let mut nonce = [0u8; 16];
    OsRng.fill_bytes(&mut nonce);
    let nonce = BASE64.encode(nonce);
    let target = script_json(&Value::String(origin.to_owned()));
    let body = popup_page(&nonce, &script_json(message), &target);
    let mut reply = http::reply(
        StatusCode::OK,
        Some("text/html; charset=utf-8"),
        Bytes::from(body),
    );
    let policy = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; base-uri 'none'; form-action 'none'; \
         frame-ancestors 'none'"
    );
    if let Ok(value) = HeaderValue::from_str(&policy) {
        reply.headers_mut().insert("content-security-policy", value);
    }
    page_headers(reply)
}

pub(super) fn redirect(location: &str) -> Reply {
    let mut reply = http::empty(StatusCode::FOUND);
    if let Ok(value) = HeaderValue::from_str(location) {
        reply.headers_mut().insert(LOCATION, value);
    }
    page_headers(reply)
}

pub(super) fn expired() -> Reply {
    let reply = http::reply(
        StatusCode::BAD_REQUEST,
        Some("text/html; charset=utf-8"),
        Bytes::from_static(EXPIRED.as_bytes()),
    );
    page_headers(reply)
}

/// The callback URL carries the code and state, so it is never sent on as a referrer.
fn page_headers(mut reply: Reply) -> Reply {
    let headers = reply.headers_mut();
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    reply
}

/// JSON that is safe inside a `<script>` element.
fn script_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use url::Url;

    fn page(mode: Mode) -> PageReturn {
        PageReturn {
            url: Url::parse("https://play.example.org/inn.html?auth=x").unwrap(),
            origin: "https://play.example.org".into(),
            mode,
            attest: None,
            page_nonce: None,
        }
    }

    async fn body(reply: Reply) -> String {
        let bytes = reply.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn a_popup_posts_to_the_page_origin_only() {
        let reply = finish(&page(Mode::Popup), Outcome::Session("TOKEN"));
        let policy = reply.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(policy.contains("script-src 'nonce-"), "{policy}");
        let text = body(reply).await;
        assert!(
            text.contains(r#"var msg = {"session":"TOKEN","type":"inn-auth"}"#),
            "{text}"
        );
        assert!(
            text.contains(r#"target = "https://play.example.org";"#),
            "{text}"
        );
        assert!(!text.contains("'*'"));
        let denied = body(finish(&page(Mode::Popup), Outcome::Error("access_denied"))).await;
        assert!(
            denied.contains(r#"{"error":"access_denied","type":"inn-auth"}"#),
            "{denied}"
        );
    }

    #[test]
    fn a_redirect_carries_the_result_in_the_fragment() {
        let reply = finish(&page(Mode::Redirect), Outcome::Session("TOKEN"));
        assert_eq!(reply.status(), StatusCode::FOUND);
        assert_eq!(
            reply.headers()[LOCATION],
            "https://play.example.org/inn.html?auth=x#inn-session=TOKEN"
        );
        let denied = finish(&page(Mode::Redirect), Outcome::Error("access_denied"));
        assert_eq!(
            denied.headers()[LOCATION],
            "https://play.example.org/inn.html?auth=x#inn-error=access_denied"
        );
        assert_eq!(denied.headers()["referrer-policy"], "no-referrer");
    }

    #[tokio::test]
    async fn the_page_nonce_comes_back_with_the_result_in_both_modes() {
        let mut redirect = page(Mode::Redirect);
        redirect.page_nonce = Some("abcdefghijklmnop".into());
        let reply = finish(&redirect, Outcome::Session("TOKEN"));
        assert_eq!(
            reply.headers()[LOCATION],
            "https://play.example.org/inn.html?auth=x#inn-session=TOKEN&inn-nonce=abcdefghijklmnop"
        );
        let mut popup = page(Mode::Popup);
        popup.page_nonce = Some("abcdefghijklmnop".into());
        let text = body(finish(&popup, Outcome::Error("access_denied"))).await;
        assert!(
            text.contains(
                r#"{"error":"access_denied","nonce":"abcdefghijklmnop","type":"inn-auth"}"#
            ),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_link_proof_comes_back_under_its_own_name() {
        let reply = finish(&page(Mode::Redirect), Outcome::Link("PROOF"));
        assert_eq!(
            reply.headers()[LOCATION],
            "https://play.example.org/inn.html?auth=x#inn-link=PROOF"
        );
        let text = body(finish(&page(Mode::Popup), Outcome::Link("PROOF"))).await;
        assert!(
            text.contains(r#"{"link":"PROOF","type":"inn-auth"}"#),
            "{text}"
        );
    }

    #[test]
    fn script_json_cannot_close_the_script_element() {
        let text = script_json(&json!({"x": "</script><b>&"}));
        assert!(
            !text.contains('<') && !text.contains('>') && !text.contains('&'),
            "{text}"
        );
    }
}
