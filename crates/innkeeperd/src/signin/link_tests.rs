//! Linked sign-in methods over HTTP: the recent sign-in rule, and enrolment, suspension and the
//! daily cap counted per group.

use super::*;
use crate::signin::groups;

impl Fixture {
    /// A live token for `key` whose sign-in happened `age` milliseconds ago.
    fn aged_token(&self, key: &IdentityKey, age: i64) -> String {
        let token = keys::new_token();
        let now = crate::dates::now_ms();
        let hash = keys::token_hash(&token);
        store::create_session(&self.store, key, &hash, now - age, now + crate::dates::HOUR)
            .unwrap();
        token.to_string()
    }

    fn account(&self, key: &IdentityKey) -> AccountId {
        let record = AccountRecord::new(keys::encode_password("SWORDFISH").unwrap(), "");
        match store::create_owned_account(&self.store, key, &record, crate::dates::now_ms()) {
            Ok(Created::Account(id)) => id,
            other => panic!("{other:?}"),
        }
    }
}

#[tokio::test]
async fn changing_methods_needs_a_recent_sign_in_and_the_session_lists_them() {
    let fx = fixture(EnrolmentMode::WebOnly, 3, false).await;
    let (fresh, key) = fx.token("erin");
    let stale = fx.aged_token(&key, crate::signin::RECENT_SIGN_IN + crate::dates::MINUTE);
    let post = reqwest::Method::POST;
    let (status, body) = fx
        .call(post.clone(), "/auth/identities/ticket", &stale, None)
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("reauth_required"))
    );
    let id = json!(groups::method_id(&key));
    let (status, body) = fx
        .call(
            post.clone(),
            "/auth/identities/unlink",
            &stale,
            Some(json!({ "id": id })),
        )
        .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("reauth_required"))
    );
    let (status, body) = fx
        .call(post.clone(), "/auth/identities/ticket", &fresh, None)
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["expiresIn"], 600);
    let (status, body) = fx
        .call(
            post.clone(),
            "/auth/identities/unlink",
            &fresh,
            Some(json!({ "id": "x" })),
        )
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("bad_id")));
    let (status, body) = fx
        .call(
            post,
            "/auth/identities",
            &fresh,
            Some(json!({ "proof": "AAAA" })),
        )
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("bad_proof")));
    let (_, session) = fx
        .call(reqwest::Method::GET, "/auth/session", &stale, None)
        .await;
    assert_eq!(
        session["identities"],
        json!([{
            "id": groups::method_id(&key),
            "provider": "dev",
            "name": "Dev Phone Co.",
            "linkedOn": crate::dates::format_date(crate::dates::now_ms()),
            "current": true,
        }])
    );
}

#[tokio::test]
async fn a_ticket_of_an_ended_session_sends_the_login_back_as_expired() {
    let fx = fixture(EnrolmentMode::WebOnly, 3, false).await;
    let (token, _) = fx.token("fay");
    let (_, body) = fx
        .call(
            reqwest::Method::POST,
            "/auth/identities/ticket",
            &token,
            None,
        )
        .await;
    let ticket = body["ticket"].as_str().unwrap().to_owned();
    fx.call(reqwest::Method::POST, "/auth/logout", &token, None)
        .await;
    let login = format!(
        "/auth/login?provider=dev&mode=redirect&return={PAGE}/inn.html&nonce=abcdefghijklmnop&link={ticket}"
    );
    let answer = fx.http.get(fx.url(&login)).send().await.unwrap();
    assert_eq!(answer.status(), 302);
    let to = answer.headers()["location"].to_str().unwrap();
    assert!(
        to.ends_with("#inn-error=link_expired&inn-nonce=abcdefghijklmnop"),
        "{to}"
    );
}

#[tokio::test]
async fn a_linked_method_follows_its_group_for_invites_suspension_and_the_daily_cap() {
    let dir = std::env::temp_dir().join(format!("inn-link-invites-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let list = dir.join("invites");
    std::fs::write(&list, "").unwrap();
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    let mut sign_in = sign_in_for(&store, EnrolmentMode::Invite, 2, false);
    sign_in.invites.file = Some(crate::admin::allowlist::KeyFile::open(list.clone()).unwrap());
    let fx = serve(store, sign_in, EnrolmentMode::Invite, false).await;
    let (_, invited) = fx.token("gail");
    let (token, linked) = fx.token("gail-too");
    std::fs::write(&list, crate::admin::allowlist::hex(&invited)).unwrap();
    let mine = fx.account(&invited);
    assert_eq!(
        groups::link(&fx.store, &invited, &linked, 1).unwrap(),
        groups::Linked::Joined
    );
    let session = || fx.call(reqwest::Method::GET, "/auth/session", &token, None);
    let create = || {
        fx.call(
            reqwest::Method::POST,
            "/auth/accounts",
            &token,
            Some(json!({})),
        )
    };
    let (_, answer) = session().await;
    assert_eq!(
        answer["canCreate"], true,
        "invited through the group: {answer}"
    );
    assert_eq!(answer["accounts"][0]["id"], mine.0);
    assert_eq!(create().await.0, 201);
    let (status, body) = create().await;
    assert_eq!(
        (status, body["error"].as_str()),
        (429, Some("rate_limited")),
        "two a day per group"
    );
    crate::admin::store::set_suspended(&fx.store, mine.0, true, 1).unwrap();
    std::fs::write(&list, "").unwrap();
    let (_, answer) = session().await;
    assert_eq!(answer["canCreate"], false);
    let _ = std::fs::remove_dir_all(&dir);
}
