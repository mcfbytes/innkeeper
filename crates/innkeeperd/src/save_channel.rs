//! The save channel of a WebSocket line: JSON control messages in text frames that keep a signed-on
//! account's game save files in the store. Sans-IO: a text goes in, reply texts come out. The
//! protocol and its rules are in docs/server/saves.md.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use innkeeper_world::saves::{
    canonical_path, hash_hex, PathRefusal, SaveMeta, MAX_ACCOUNT_BYTES, MAX_ACCOUNT_FILES,
    MAX_FILE_BYTES,
};
use innkeeper_world::{AccountId, Store};
use serde_json::{json, Map, Value};
use tracing::{debug, warn};

type Reply = Map<String, Value>;

/// One WebSocket's save channel; it serves nobody until the line session has logged on.
#[derive(Debug)]
pub(crate) struct SaveChannel {
    store: Arc<dyn Store>,
    account: Option<AccountId>,
}

/// Why a request got an `error` reply; `code` is what the page switches on.
struct Refusal {
    code: &'static str,
    detail: String,
}

fn refuse(code: &'static str, detail: impl Into<String>) -> Refusal {
    Refusal {
        code,
        detail: detail.into(),
    }
}

impl SaveChannel {
    pub(crate) fn new(store: Arc<dyn Store>) -> Self {
        SaveChannel {
            store,
            account: None,
        }
    }

    /// The line session logged on as `account` (or ended, with `None`). A logon is announced with
    /// the manifest, which the page reconciles against its own files.
    pub(crate) fn set_account(&mut self, account: Option<AccountId>) -> Vec<String> {
        self.account = account;
        let Some(account) = account else {
            return Vec::new();
        };
        let reply = self
            .manifest(account)
            .unwrap_or_else(|refusal| error_reply(&refusal));
        vec![self.reply_to(&Map::new(), reply)]
    }

    /// Answers one text frame; `now` is the host clock in milliseconds since the Unix epoch.
    pub(crate) fn handle(&mut self, text: &str, now: u64) -> Vec<String> {
        let Ok(Value::Object(request)) = serde_json::from_str(text) else {
            let reply = error_reply(&refuse("bad_request", "not a JSON object"));
            return vec![self.reply_to(&Map::new(), reply)];
        };
        let request = &request;
        let outcome = match self.account {
            None => Err(refuse("not_logged_on", "log on through the line first")),
            Some(account) => self.dispatch(account, request, now),
        };
        let reply = outcome.unwrap_or_else(|refusal| error_reply(&refusal));
        vec![self.reply_to(request, reply)]
    }

    /// The reply text, tagged with the request's `id` when it had one.
    fn reply_to(&self, request: &Map<String, Value>, mut reply: Reply) -> String {
        if let Some(id) = request.get("id") {
            reply.insert("id".to_owned(), id.clone());
        }
        Value::Object(reply).to_string()
    }

    fn dispatch(
        &self,
        account: AccountId,
        request: &Map<String, Value>,
        now: u64,
    ) -> Result<Reply, Refusal> {
        let op = request.get("op").and_then(Value::as_str).unwrap_or("");
        debug!(account = account.0, op, "save channel request");
        match op {
            "manifest" => self.manifest(account),
            "get" => self.get(account, request),
            "versions" => self.versions(account, request),
            "put" => self.put(account, request, now),
            _ => Err(refuse("unknown_op", format!("no such op: {op:?}"))),
        }
    }

    fn manifest(&self, account: AccountId) -> Result<Reply, Refusal> {
        let files = self.store.saves(account).map_err(store_refusal)?;
        Ok(object(json!({
            "op": "manifest",
            "account": account.0,
            "files": files.iter().map(meta_json).collect::<Vec<_>>(),
        })))
    }

    fn get(&self, account: AccountId, request: &Map<String, Value>) -> Result<Reply, Refusal> {
        let path = path_of(request)?;
        let version = optional_number(request, "version")?;
        let found = self
            .store
            .save_read(account, &path, version)
            .map_err(store_refusal)?;
        let (meta, data) = found.ok_or_else(|| refuse("no_such_file", path.clone()))?;
        let mut reply = meta_json(&meta);
        reply.insert("op".to_owned(), json!("file"));
        reply.insert("data".to_owned(), json!(BASE64.encode(data)));
        Ok(reply)
    }

    fn versions(&self, account: AccountId, request: &Map<String, Value>) -> Result<Reply, Refusal> {
        let path = path_of(request)?;
        let kept = self
            .store
            .save_versions(account, &path)
            .map_err(store_refusal)?;
        Ok(object(json!({
            "op": "versions",
            "path": path,
            "versions": kept.iter().map(meta_json).collect::<Vec<_>>(),
        })))
    }

    /// Stores an upload under the rules of saves.md section 4.
    fn put(
        &self,
        account: AccountId,
        request: &Map<String, Value>,
        now: u64,
    ) -> Result<Reply, Refusal> {
        let path = path_of(request)?;
        let base = optional_number(request, "base")?.unwrap_or(0);
        let modified = optional_number(request, "mtime")?.unwrap_or(now).min(now);
        let encoded = request
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| refuse("bad_request", "put needs a data string"))?;
        if encoded.len() > MAX_FILE_BYTES.div_ceil(3) * 4 {
            return Err(too_large(&path));
        }
        let data = BASE64
            .decode(encoded)
            .map_err(|_| refuse("bad_data", "data is not base64"))?;
        if data.len() > MAX_FILE_BYTES {
            return Err(too_large(&path));
        }
        let held = self.store.saves(account).map_err(store_refusal)?;
        let current = held.iter().find(|meta| meta.path == path);
        let others: usize = held
            .iter()
            .filter(|meta| meta.path != path)
            .map(|m| m.size)
            .sum();
        if others + data.len() > MAX_ACCOUNT_BYTES
            || (current.is_none() && held.len() >= MAX_ACCOUNT_FILES)
        {
            return Err(refuse("quota", "this account's save space is full"));
        }

        let hash = hash_hex(&data);
        let write = |as_current: bool| {
            self.store
                .save_write(account, &path, &data, modified, as_current)
                .map_err(store_refusal)
        };
        let Some(current) = current else {
            return Ok(stored(&write(true)?, false, false));
        };
        if current.hash == hash {
            return Ok(stored(current, false, true));
        }
        if current.version == base {
            return Ok(stored(&write(true)?, false, false));
        }
        // Both sides changed since the page's last sync: the later modification wins.
        if modified > current.modified {
            return Ok(stored(&write(true)?, true, false));
        }
        let kept = write(false)?;
        let mut reply = meta_json(current);
        reply.insert("op".to_owned(), json!("superseded"));
        reply.insert("kept_version".to_owned(), json!(kept.version));
        Ok(reply)
    }
}

fn too_large(path: &str) -> Refusal {
    refuse(
        "too_large",
        format!("{path} may not exceed {MAX_FILE_BYTES} bytes"),
    )
}

/// The `stored` reply: `replaced` when the upload won over a newer-looking server copy, `unchanged`
/// when the server already held exactly this content.
fn stored(meta: &SaveMeta, replaced: bool, unchanged: bool) -> Reply {
    let mut reply = meta_json(meta);
    reply.insert("op".to_owned(), json!("stored"));
    reply.insert("replaced".to_owned(), json!(replaced));
    reply.insert("unchanged".to_owned(), json!(unchanged));
    reply
}

fn meta_json(meta: &SaveMeta) -> Reply {
    object(json!({
        "path": meta.path,
        "version": meta.version,
        "hash": meta.hash,
        "size": meta.size,
        "mtime": meta.modified,
        "current": meta.current,
    }))
}

fn object(value: Value) -> Reply {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn path_of(request: &Map<String, Value>) -> Result<String, Refusal> {
    let raw = request
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| refuse("bad_request", "the request needs a path string"))?;
    canonical_path(raw).map_err(|why| match why {
        PathRefusal::Malformed => refuse("bad_path", "not a drive-relative DOS path"),
        PathRefusal::NotASaveFile => refuse("not_allowed", "not a save file"),
        PathRefusal::PasswordFile => refuse("not_allowed", "password files are never stored"),
    })
}

fn optional_number(request: &Map<String, Value>, key: &str) -> Result<Option<u64>, Refusal> {
    match request.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            refuse(
                "bad_request",
                format!("{key} must be a non-negative integer"),
            )
        }),
    }
}

fn store_refusal(error: innkeeper_world::StoreError) -> Refusal {
    warn!(%error, "save store failed");
    refuse("server_error", "the store failed")
}

fn error_reply(refusal: &Refusal) -> Reply {
    object(json!({"op": "error", "code": refusal.code, "detail": refusal.detail}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)] // assertions may abort
mod tests {
    use innkeeper_world::MemoryStore;

    use super::*;

    const ALICE: AccountId = AccountId(100_001);
    const BOB: AccountId = AccountId(100_002);
    const CHARS: &str = "INN\\YSERBIUS\\MYCHARS.DAT";

    fn channel_on(store: &Arc<dyn Store>, account: AccountId) -> SaveChannel {
        let mut channel = SaveChannel::new(Arc::clone(store));
        channel.set_account(Some(account));
        channel
    }

    fn new_store() -> Arc<dyn Store> {
        Arc::new(MemoryStore::new())
    }

    fn ask(channel: &mut SaveChannel, request: &Value, now: u64) -> Value {
        let replies = channel.handle(&request.to_string(), now);
        assert_eq!(replies.len(), 1);
        serde_json::from_str(&replies[0]).unwrap()
    }

    fn put(path: &str, base: u64, mtime: u64, data: &[u8]) -> Value {
        json!({"op": "put", "path": path, "base": base, "mtime": mtime, "data": BASE64.encode(data)})
    }

    fn code(reply: &Value) -> &str {
        assert_eq!(reply["op"], "error", "{reply}");
        reply["code"].as_str().unwrap()
    }

    fn content(channel: &mut SaveChannel, path: &str) -> (u64, Vec<u8>) {
        let reply = ask(channel, &json!({"op": "get", "path": path}), 0);
        let data = BASE64.decode(reply["data"].as_str().unwrap()).unwrap();
        (reply["version"].as_u64().unwrap(), data)
    }

    #[test]
    fn nothing_is_served_before_logon() {
        let mut channel = SaveChannel::new(new_store());
        for request in [
            json!({"op": "manifest"}),
            json!({"op": "get", "path": CHARS}),
            put(CHARS, 0, 1, b"x"),
            json!("not an object"),
        ] {
            let reply = ask(&mut channel, &request, 0);
            let wanted = if request.is_object() {
                "not_logged_on"
            } else {
                "bad_request"
            };
            assert_eq!(code(&reply), wanted);
        }
        assert_eq!(channel.store.saves(ALICE).unwrap(), []);
    }

    #[test]
    fn the_logon_announces_the_manifest_and_a_hang_up_closes_the_channel() {
        let store = new_store();
        store.save_write(ALICE, CHARS, b"abc", 77, true).unwrap();
        let mut channel = SaveChannel::new(Arc::clone(&store));
        let sent = channel.set_account(Some(ALICE));
        let manifest: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(manifest["op"], "manifest");
        assert_eq!(manifest["account"], 100_001);
        let file = &manifest["files"][0];
        assert_eq!(file["path"], CHARS);
        assert_eq!(
            (
                file["version"].as_u64(),
                file["size"].as_u64(),
                file["mtime"].as_u64()
            ),
            (Some(1), Some(3), Some(77))
        );
        assert_eq!(file["hash"], hash_hex(b"abc"));

        assert!(channel.set_account(None).is_empty());
        assert_eq!(
            code(&ask(&mut channel, &json!({"op": "manifest"}), 0)),
            "not_logged_on"
        );
    }

    #[test]
    fn upload_then_download_and_the_id_is_echoed() {
        let store = new_store();
        let mut channel = channel_on(&store, ALICE);
        let mut request = put("inn\\yserbius\\MyChars.dat", 0, 500, b"first");
        request["id"] = json!("u1");
        let reply = ask(&mut channel, &request, 1_000);
        assert_eq!(
            (reply["op"].as_str(), reply["id"].as_str()),
            (Some("stored"), Some("u1"))
        );
        assert_eq!(
            (reply["version"].as_u64(), reply["replaced"].as_bool()),
            (Some(1), Some(false))
        );
        assert_eq!(content(&mut channel, CHARS), (1, b"first".to_vec()));

        let reply = ask(&mut channel, &put(CHARS, 1, 600, b"second"), 1_000);
        assert_eq!(reply["version"], 2);
        assert_eq!(content(&mut channel, CHARS), (2, b"second".to_vec()));
        let versions = ask(&mut channel, &json!({"op": "versions", "path": CHARS}), 0);
        assert_eq!(versions["versions"].as_array().unwrap().len(), 2);
        let old = ask(
            &mut channel,
            &json!({"op": "get", "path": CHARS, "version": 1}),
            0,
        );
        assert_eq!(
            (old["current"].as_bool(), old["version"].as_u64()),
            (Some(false), Some(1))
        );
        let missing = ask(
            &mut channel,
            &json!({"op": "get", "path": CHARS, "version": 9}),
            0,
        );
        assert_eq!(code(&missing), "no_such_file");
    }

    #[test]
    fn accounts_do_not_see_each_others_files() {
        let store = new_store();
        let mut alice = channel_on(&store, ALICE);
        let mut bob = channel_on(&store, BOB);
        ask(&mut alice, &put(CHARS, 0, 1, b"alice"), 5);
        assert_eq!(
            code(&ask(&mut bob, &json!({"op": "get", "path": CHARS}), 0)),
            "no_such_file"
        );
        ask(&mut bob, &put(CHARS, 0, 1, b"bob"), 5);
        assert_eq!(content(&mut alice, CHARS).1, b"alice");
    }

    #[test]
    fn only_the_listed_paths_are_stored() {
        let mut channel = channel_on(&new_store(), ALICE);
        for (path, wanted) in [
            ("INN\\PASS_SET.DTA", "not_allowed"),
            ("inn/pass_set.dta", "not_allowed"),
            ("INN\\TSNEXEC.EXE", "not_allowed"),
            ("INN\\YSERBIUS\\MYCHARS.NEW", "not_allowed"),
            ("INN\\..\\LSCI.CFG", "bad_path"),
            ("", "bad_path"),
        ] {
            assert_eq!(
                code(&ask(&mut channel, &put(path, 0, 1, b"x"), 5)),
                wanted,
                "{path}"
            );
            let get = ask(&mut channel, &json!({"op": "get", "path": path}), 5);
            assert_eq!(code(&get), wanted, "{path}");
        }
        for path in [
            "INN\\PLAYER.DIR",
            "INN\\GUYBRUSH.DTA",
            "INN\\TWINION\\TWINCHAR.DAT",
            "INN\\TWINION\\AUTOMAPS.DAT",
            "INN\\YSERBIUS\\AUTOMAPS.DAT",
        ] {
            assert_eq!(
                ask(&mut channel, &put(path, 0, 1, b"x"), 5)["op"],
                "stored",
                "{path}"
            );
        }
        assert_eq!(channel.store.saves(ALICE).unwrap().len(), 5);
    }

    #[test]
    fn size_and_quota_limits_refuse_without_storing() {
        let store = new_store();
        let mut channel = channel_on(&store, ALICE);
        let at_limit = vec![7; MAX_FILE_BYTES];
        assert_eq!(
            ask(&mut channel, &put(CHARS, 0, 1, &at_limit), 5)["op"],
            "stored"
        );
        let over = vec![7; MAX_FILE_BYTES + 1];
        assert_eq!(
            code(&ask(&mut channel, &put(CHARS, 1, 1, &over), 5)),
            "too_large"
        );
        let huge = json!({"op": "put", "path": CHARS, "data": "A".repeat(MAX_FILE_BYTES * 2)});
        assert_eq!(code(&ask(&mut channel, &huge, 5)), "too_large");
        let garbage = json!({"op": "put", "path": CHARS, "data": "***"});
        assert_eq!(code(&ask(&mut channel, &garbage, 5)), "bad_data");
        assert_eq!(
            code(&ask(&mut channel, &json!({"op": "put", "path": CHARS}), 5)),
            "bad_request"
        );
        assert_eq!(store.save_versions(ALICE, CHARS).unwrap().len(), 1);

        // Eight 1 MiB files fill the account; a ninth path is refused, a replacement is not.
        let others = [
            "INN\\YSERBIUS\\AUTOMAPS.DAT",
            "INN\\TWINION\\TWINCHAR.DAT",
            "INN\\TWINION\\AUTOMAPS.DAT",
            "INN\\PLAYER.DIR",
            "INN\\A.DTA",
            "INN\\B.DTA",
            "INN\\C.DTA",
        ];
        for path in others {
            assert_eq!(
                ask(&mut channel, &put(path, 0, 1, &at_limit), 5)["op"],
                "stored",
                "{path}"
            );
        }
        assert_eq!(
            code(&ask(&mut channel, &put("INN\\D.DTA", 0, 1, b"x"), 5)),
            "quota"
        );
        assert_eq!(
            ask(&mut channel, &put(CHARS, 1, 9, b"small"), 10)["op"],
            "stored"
        );
        assert_eq!(
            ask(&mut channel, &put("INN\\D.DTA", 0, 1, b"x"), 5)["op"],
            "stored"
        );
    }

    #[test]
    fn the_file_count_is_capped() {
        let mut channel = channel_on(&new_store(), ALICE);
        for n in 0..MAX_ACCOUNT_FILES {
            let path = format!("INN\\F{n}.DTA");
            assert_eq!(
                ask(&mut channel, &put(&path, 0, 1, b"x"), 5)["op"],
                "stored",
                "{path}"
            );
        }
        assert_eq!(
            code(&ask(&mut channel, &put("INN\\LAST.DTA", 0, 1, b"x"), 5)),
            "quota"
        );
        assert_eq!(
            ask(&mut channel, &put("INN\\F0.DTA", 1, 1, b"y"), 5)["op"],
            "stored"
        );
    }

    #[test]
    fn an_upload_on_the_current_version_or_of_the_same_bytes_is_not_a_conflict() {
        let mut channel = channel_on(&new_store(), ALICE);
        ask(&mut channel, &put(CHARS, 0, 100, b"one"), 1_000);
        let same = ask(&mut channel, &put(CHARS, 0, 50, b"one"), 1_000);
        assert_eq!(
            (
                same["op"].as_str(),
                same["version"].as_u64(),
                same["unchanged"].as_bool()
            ),
            (Some("stored"), Some(1), Some(true))
        );
        let next = ask(&mut channel, &put(CHARS, 1, 50, b"two"), 1_000);
        assert_eq!(
            (next["version"].as_u64(), next["replaced"].as_bool()),
            (Some(2), Some(false))
        );
    }

    #[test]
    fn when_both_changed_the_later_modification_wins_and_the_other_is_kept() {
        let store = new_store();
        let mut channel = channel_on(&store, ALICE);
        ask(&mut channel, &put(CHARS, 0, 100, b"base"), 1_000);
        ask(&mut channel, &put(CHARS, 1, 200, b"server edit"), 1_000);

        // A page that last synced version 1 and edited at 300 wins over the server's edit at 200.
        let newer = ask(&mut channel, &put(CHARS, 1, 300, b"page edit"), 1_000);
        assert_eq!(
            (
                newer["op"].as_str(),
                newer["version"].as_u64(),
                newer["replaced"].as_bool()
            ),
            (Some("stored"), Some(3), Some(true))
        );
        assert_eq!(content(&mut channel, CHARS), (3, b"page edit".to_vec()));
        let previous = store.save_read(ALICE, CHARS, Some(2)).unwrap().unwrap();
        assert_eq!(
            previous.1, b"server edit",
            "the loser stays as a previous version"
        );

        // A page that edited at 250 loses: the server copy stays current, the upload is kept beside it.
        let older = ask(&mut channel, &put(CHARS, 2, 250, b"stale page edit"), 1_000);
        assert_eq!(
            (
                older["op"].as_str(),
                older["version"].as_u64(),
                older["kept_version"].as_u64()
            ),
            (Some("superseded"), Some(3), Some(4))
        );
        assert_eq!(older["hash"], hash_hex(b"page edit"));
        assert_eq!(content(&mut channel, CHARS), (3, b"page edit".to_vec()));
        let kept = store.save_read(ALICE, CHARS, Some(4)).unwrap().unwrap();
        assert_eq!(
            (kept.0.current, kept.1.as_slice()),
            (false, &b"stale page edit"[..])
        );
    }

    #[test]
    fn three_versions_are_kept_per_file() {
        let store = new_store();
        let mut channel = channel_on(&store, ALICE);
        for n in 0..5u8 {
            let reply = ask(
                &mut channel,
                &put(CHARS, u64::from(n), 10 + u64::from(n), &[n]),
                1_000,
            );
            assert_eq!(reply["version"], u64::from(n) + 1);
        }
        let versions = ask(&mut channel, &json!({"op": "versions", "path": CHARS}), 0);
        let numbers: Vec<_> = versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["version"].as_u64().unwrap())
            .collect();
        assert_eq!(numbers, [5, 4, 3]);
    }

    #[test]
    fn a_modification_time_from_the_future_is_clamped_to_the_host_clock() {
        let store = new_store();
        let mut channel = channel_on(&store, ALICE);
        let reply = ask(&mut channel, &put(CHARS, 0, u64::MAX, b"x"), 4_000);
        assert_eq!(reply["mtime"], 4_000);
        let no_time = json!({"op": "put", "path": "INN\\PLAYER.DIR", "data": BASE64.encode(b"y")});
        assert_eq!(ask(&mut channel, &no_time, 4_100)["mtime"], 4_100);
    }
}
