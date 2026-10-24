//! The portal's changes. Each is a POST that passed the origin and CSRF checks, goes through the
//! `Store` trait or the retention functions, and is written to the audit log.

use std::time::Duration;

use bytes::Bytes;
use hyper::header::{HeaderValue, CONTENT_DISPOSITION};
use hyper::StatusCode;
use innkeeper_world::{AccountId, BoardId, ConnectionId, Money, PostId};

use super::html::{self, esc, window, Flash, Nav};
use super::store;
use super::views::FLAG_BITS;
use super::{Admin, Reply, Req};
use crate::dates;
use crate::http;
use crate::retention::{Disable, Restored};
use crate::signin::keys;

/// The shortest reason a letter read is allowed with.
const MIN_LETTER_REASON: usize = 8;
const MAX_REASON: usize = 300;

fn clean_reason(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_REASON)
        .collect();
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn needs_confirmation() -> Flash {
    Flash::bad(
        "That needs a yes first, and none came with the form.",
        "The clerk holds the stamp an inch above the paper and waits.",
    )
}

impl Admin {
    fn log_change(
        &self,
        req: &Req,
        action: &str,
        target: &str,
        reason: Option<&str>,
        account: Option<u32>,
    ) -> Result<(), innkeeper_world::StoreError> {
        store::record(
            &self.store,
            &req.who,
            action,
            target,
            reason,
            account,
            dates::now_ms(),
        )
    }

    /// Ends every live line of the account; returns how many were told.
    fn kick_account(&self, id: u32) -> usize {
        self.online_for(id)
            .iter()
            .filter(|row| self.settings.switchboard.kick(ConnectionId(row.connection)))
            .count()
    }

    pub(crate) async fn kick(&self, req: &Req) -> Reply {
        if !req.confirmed() {
            return self.dashboard_with(req, Some(&needs_confirmation()));
        }
        let Ok(connection) = req.param("connection").parse::<u64>() else {
            return self.not_found(req);
        };
        let row = self
            .online()
            .into_iter()
            .find(|row| row.connection == connection);
        let Some(row) = row else {
            let flash = Flash::bad(
                format!("Connection {connection} is no longer online."),
                "They left before the clerk reached the switch.",
            );
            return self.dashboard_with(req, Some(&flash));
        };
        self.settings.switchboard.kick(ConnectionId(connection));
        let target = format!("connection {connection} (account {})", row.account);
        let reason = clean_reason(req.param("reason"));
        if let Err(error) = self.log_change(
            req,
            "connection.kick",
            &target,
            reason.as_deref(),
            Some(row.account),
        ) {
            return self.failed(req, &error);
        }
        // Give the line a moment to hang up so the list is already right.
        tokio::time::sleep(Duration::from_millis(250)).await;
        let flash = Flash::good(
            format!(
                "Connection {connection} (account {}) was ended.",
                row.account
            ),
            "Click. The line goes quiet.",
        );
        self.dashboard_with(req, Some(&flash))
    }

    /// Sends a notice (command 48) to every logged-in player.
    pub(crate) async fn announce(&self, req: &Req) -> Reply {
        if !req.confirmed() {
            return self.dashboard_with(req, Some(&needs_confirmation()));
        }
        let Some(text) = crate::announce::clean(req.param("text")) else {
            let flash = Flash::bad(
                "There was nothing to announce: the notice was empty once cleaned.",
                "The town crier clears his throat and says nothing.",
            );
            return self.dashboard_with(req, Some(&flash));
        };
        let told = crate::announce::announce(&self.settings, &text);
        if let Err(error) = self.log_change(
            req,
            "notice.send",
            &format!("{told} players"),
            Some(&text),
            None,
        ) {
            return self.failed(req, &error);
        }
        let flash = Flash::good(
            format!(
                "The notice went to {told} logged-in player{}.",
                if told == 1 { "" } else { "s" }
            ),
            "The town crier rings his bell.",
        );
        self.dashboard_with(req, Some(&flash))
    }

    pub(crate) fn account_action(&self, req: &Req, id_text: &str, action: &str) -> Reply {
        let Ok(id) = id_text.parse::<u32>() else {
            return self.not_found(req);
        };
        let detail = match store::account_detail(&self.store, id) {
            Ok(Some(detail)) => detail,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let reason = clean_reason(req.param("reason"));
        let reason = reason.as_deref();
        let outcome = match action {
            "suspend" => self.act_suspend(req, id, true, reason),
            "unsuspend" => self.act_suspend(req, id, false, reason),
            "flags" => self.act_flags(req, &detail, reason),
            "money" => self.act_money(req, id, reason),
            "password" => return self.act_password(req, id, id_text, reason),
            "sessions" => self.act_sessions(req, id, reason),
            "delete" => self.act_delete(req, id, reason),
            "restore" => self.act_restore(req, id, reason),
            _ => return self.not_found(req),
        };
        let (status, flash) = match outcome {
            Ok(done) => (StatusCode::OK, done),
            Err(Refused::Flash(flash)) => (StatusCode::CONFLICT, flash),
            Err(Refused::Store(error)) => return self.failed(req, &error),
        };
        let mut reply = self.account_with(req, id_text, Some(&flash), "");
        if status != StatusCode::OK {
            *reply.status_mut() = status;
        }
        reply
    }

    fn act_suspend(&self, req: &Req, id: u32, suspend: bool, reason: Option<&str>) -> Outcome {
        if suspend && !req.confirmed() {
            return Err(Refused::Flash(needs_confirmation()));
        }
        let now = dates::now_ms();
        let changed = store::set_suspended(&self.store, id, suspend, now)?;
        if !changed {
            return Err(Refused::Flash(Flash::bad(
                format!("Account {id} could not be changed: it is deleted or gone."),
                "The file is in the wrong cabinet.",
            )));
        }
        let kicked = if suspend { self.kick_account(id) } else { 0 };
        let name = if suspend {
            "account.suspend"
        } else {
            "account.unsuspend"
        };
        self.log_change(req, name, &format!("account {id}"), reason, Some(id))?;
        Ok(if suspend {
            Flash::good(
                format!("Account {id} is suspended. {kicked} live line(s) were kicked, and a Login is now refused."),
                "Stamped. Filed. The player will find the door locked and a polite note on it.",
            )
        } else {
            Flash::good(
                format!("Account {id} is no longer suspended. The player can log on again."),
                "The clerk crosses out the word SUSPENDED and initials it.",
            )
        })
    }

    fn refuse_if_online(&self, id: u32, what: &str) -> Result<(), Refused> {
        if self.online_for(id).is_empty() {
            return Ok(());
        }
        Err(Refused::Flash(Flash::bad(
            format!("Account {id} is online now, so its {what} was not changed."),
            "You cannot repaint a ship while it is at sea. Kick them or suspend the account first.",
        )))
    }

    fn act_flags(&self, req: &Req, detail: &store::AccountDetail, reason: Option<&str>) -> Outcome {
        let id = detail.summary.id;
        self.refuse_if_online(id, "flags")?;
        let old_flags = detail.summary.user_flags;
        let mut flags = old_flags;
        for (bit, _) in FLAG_BITS {
            flags &= !bit;
            if req.param(&format!("flag_{bit}")) == "1" {
                flags |= bit;
            }
        }
        let mut rating = detail.rating;
        for (kind, _) in self.land_names() {
            let Some(bit) = kind.checked_sub(1).filter(|bit| *bit < 16) else {
                continue;
            };
            rating &= !(1u16 << bit);
            if req.param(&format!("land_{kind}")) == "1" {
                rating |= 1u16 << bit;
            }
        }
        if !store::set_flags(&self.store, id, flags, rating)? {
            return Err(Refused::Flash(Flash::bad(
                format!("Account {id} could not be changed."),
                "The form went into the wrong tray.",
            )));
        }
        let target = format!(
            "account {id} flags {old_flags:#06x} to {flags:#06x}, rating {:#06x} to {rating:#06x}",
            detail.rating
        );
        self.log_change(req, "account.flags", &target, reason, Some(id))?;
        Ok(Flash::good(
            format!("Account {id}: flags are now {flags:#06x} and land rights {rating:#06x}. The player sees them at the next Login."),
            "A new stamp, still damp.",
        ))
    }

    fn act_money(&self, req: &Req, id: u32, reason: Option<&str>) -> Outcome {
        self.refuse_if_online(id, "CasinoBucks")?;
        let Ok(amount) = req.param("amount").trim().parse::<u32>() else {
            return Err(Refused::Flash(Flash::bad(
                "A balance is a whole number from 0 to 4294967295.",
                "The abacus declines to carry that.",
            )));
        };
        let world_store = &self.settings.world.store;
        let old = world_store.money(AccountId(id))?.map(|money| money.0);
        world_store.set_money(AccountId(id), Money(amount))?;
        let target = format!(
            "account {id} bankroll {} to {amount}",
            old.map_or_else(|| "unset".to_owned(), |old| old.to_string())
        );
        self.log_change(req, "account.money", &target, reason, Some(id))?;
        Ok(Flash::good(
            format!("Account {id} now has {amount} CasinoBucks. The game page reads it when the client next asks."),
            "The cashier counts it twice, then a third time out of spite.",
        ))
    }

    fn act_password(&self, req: &Req, id: u32, id_text: &str, reason: Option<&str>) -> Reply {
        if !req.confirmed() {
            return self.account_with(req, id_text, Some(&needs_confirmation()), "");
        }
        if let Err(Refused::Flash(flash)) = self.refuse_if_online(id, "password") {
            let mut reply = self.account_with(req, id_text, Some(&flash), "");
            *reply.status_mut() = StatusCode::CONFLICT;
            return reply;
        }
        let password = keys::new_password();
        let Some(encoded) = keys::encode_password(&password) else {
            let error = innkeeper_world::StoreError::Backend("no password could be made".into());
            return self.failed(req, &error);
        };
        if let Err(error) = self
            .settings
            .world
            .store
            .update_password(AccountId(id), encoded)
        {
            return self.failed(req, &error);
        }
        if let Err(error) = self.log_change(
            req,
            "account.password",
            &format!("account {id}"),
            reason,
            Some(id),
        ) {
            return self.failed(req, &error);
        }
        let shown = window(
            "New password (shown once)",
            &format!(
                "<p>Give this to the account holder by a safe route. It is not stored in readable form and \
                 this page will not show it again.</p><p class=\"pw\" id=\"newpw\">{}</p> \
                 <button type=\"button\" class=\"abtn\" data-copy=\"newpw\">Copy</button>",
                esc(&password)
            ),
        );
        let flash = Flash::good(
            format!("Account {id} has a new password. The old one no longer works."),
            "Written on a slip, handed over, and the carbon paper burned.",
        );
        self.account_with(req, id_text, Some(&flash), &shown)
    }

    fn act_sessions(&self, req: &Req, id: u32, reason: Option<&str>) -> Outcome {
        if !req.confirmed() {
            return Err(Refused::Flash(needs_confirmation()));
        }
        let ended = store::revoke_web_sessions(&self.store, id, dates::now_ms())?;
        let kicked = self.kick_account(id);
        self.log_change(
            req,
            "account.sessions",
            &format!("account {id}"),
            reason,
            Some(id),
        )?;
        Ok(Flash::good(
            format!(
                "{ended} web sign-in(s) ended and {kicked} live line(s) kicked for account {id}."
            ),
            "Every key on the ring is returned to the hook.",
        ))
    }

    fn act_delete(&self, req: &Req, id: u32, reason: Option<&str>) -> Outcome {
        if !req.confirmed() {
            return Err(Refused::Flash(needs_confirmation()));
        }
        let kicked = self.kick_account(id);
        let days = self.retention.account_days;
        let flash = match crate::subject::disable(&self.store, AccountId(id))? {
            Disable::Fresh(stamp) => {
                self.log_change(
                    req,
                    "account.delete",
                    &format!("account {id}"),
                    reason,
                    Some(id),
                )?;
                Flash::good(
                    format!(
                        "Account {id} is disabled and {kicked} live line(s) were kicked. It is erased for good on {} unless a legal hold covers it; until then it can be restored.",
                        dates::format_date(dates::add_days(stamp, days))
                    ),
                    &format!("Stamped. Filed. Forgotten (for {days} days)."),
                )
            }
            Disable::Already(stamp) => Flash::bad(
                format!(
                    "Account {id} was already disabled on {}.",
                    dates::format_date(stamp)
                ),
                "Even the clerk cannot delete a thing twice.",
            ),
            Disable::Missing => Flash::bad(
                format!("Account {id} could not be found."),
                "The drawer is empty.",
            ),
        };
        if flash.tone == html::Tone::Bad {
            return Err(Refused::Flash(flash));
        }
        Ok(flash)
    }

    fn act_restore(&self, req: &Req, id: u32, reason: Option<&str>) -> Outcome {
        let policy = self.retention;
        let restored = crate::retention::restore_account(&self.store, id, policy, dates::now_ms())?;
        match restored {
            Restored::Ok => {
                self.log_change(
                    req,
                    "account.restore",
                    &format!("account {id}"),
                    reason,
                    Some(id),
                )?;
                Ok(Flash::good(
                    format!("Account {id} is restored, with what its deletion hid."),
                    "Un-stamped. Un-filed. Remembered, after all.",
                ))
            }
            Restored::Missing => Err(Refused::Flash(Flash::bad(
                format!("Account {id} could not be found."),
                "The drawer is empty.",
            ))),
            Restored::NotDeleted => Err(Refused::Flash(Flash::bad(
                format!("Account {id} is not deleted."),
                "There is nothing to take out of the shredder.",
            ))),
            Restored::WindowClosed { deleted_at } => Err(Refused::Flash(Flash::bad(
                format!(
                    "Account {id} was disabled on {} and the {} day window has closed.",
                    dates::format_date(deleted_at),
                    policy.account_days
                ),
                "Some papers do not come back.",
            ))),
        }
    }

    pub(crate) fn complaint_handled(&self, req: &Req, id_text: &str) -> Reply {
        let Ok(post_id) = id_text.parse::<u32>() else {
            return self.not_found(req);
        };
        let complaint = match store::complaint(&self.store, post_id) {
            Ok(Some(complaint)) => complaint,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let handled = req.param("handled") == "1";
        let now = dates::now_ms();
        if let Err(error) = store::set_handled(&self.store, post_id, handled, &req.who.key, now) {
            return self.failed(req, &error);
        }
        let account = Some(complaint.account);
        let action = if handled {
            "complaint.handled"
        } else {
            "complaint.reopened"
        };
        if let Err(error) =
            self.log_change(req, action, &format!("complaint {post_id}"), None, account)
        {
            return self.failed(req, &error);
        }
        let flash = if handled {
            Flash::good(
                format!("Complaint {post_id} is marked handled. The original text is untouched."),
                "Stamped. Filed. Forgotten (for 365 days).",
            )
        } else {
            Flash::good(
                format!("Complaint {post_id} is waiting again."),
                "Back into the in-tray it goes.",
            )
        };
        self.complaint_with(req, id_text, Some(&flash))
    }

    pub(crate) fn post_remove(&self, req: &Req, board_text: &str, id_text: &str) -> Reply {
        let (Ok(board), Ok(post)) = (board_text.parse::<u32>(), id_text.parse::<u32>()) else {
            return self.not_found(req);
        };
        if !req.confirmed() {
            return self.board_with(req, board_text, Some(&needs_confirmation()));
        }
        let found = match store::post_board(&self.store, post) {
            Ok(Some((found_board, view))) if found_board == board => view,
            Ok(_) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let removed = match self
            .settings
            .world
            .store
            .delete_post(BoardId(board), PostId(post))
        {
            Ok(removed) => removed,
            Err(error) => return self.failed(req, &error),
        };
        if !removed {
            return self.not_found(req);
        }
        let account = Some(found.account);
        let target = format!("post {post} on {}", store::board_label(board));
        let reason = clean_reason(req.param("reason"));
        if let Err(error) = self.log_change(req, "post.remove", &target, reason.as_deref(), account)
        {
            return self.failed(req, &error);
        }
        let days = self.retention.post_days;
        let flash = Flash::good(
            format!("Post {post} is hidden from readers and will be erased for good after {days} days unless a legal hold covers its author."),
            &format!("Stamped. Filed. Forgotten (for {days} days)."),
        );
        self.board_with(req, board_text, Some(&flash))
    }

    pub(crate) fn letter_read(&self, req: &Req, mailbox_text: &str, id_text: &str) -> Reply {
        let (Ok(mailbox), Ok(id)) = (mailbox_text.parse::<u32>(), id_text.parse::<u32>()) else {
            return self.not_found(req);
        };
        let reason = clean_reason(req.param("reason"))
            .filter(|reason| reason.chars().count() >= MIN_LETTER_REASON);
        let Some(reason) = reason else {
            let flash = Flash::bad(
                format!("A reason of at least {MIN_LETTER_REASON} characters is needed before a letter is opened."),
                "The clerk will not hand over the envelope without a signature.",
            );
            return self.letter_gate_with(
                req,
                mailbox_text,
                id_text,
                Some(&flash),
                StatusCode::BAD_REQUEST,
            );
        };
        let owner = match store::mailbox_owner(&self.store, mailbox) {
            Ok(owner) => owner,
            Err(error) => return self.failed(req, &error),
        };
        let letter = match store::read_letter(&self.store, mailbox, id) {
            Ok(Some(letter)) => letter,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        // The reason is on record before any word of the letter leaves this function.
        let target = format!("letter {id} in mailbox {mailbox}");
        if let Err(error) = self.log_change(
            req,
            "letter.read",
            &target,
            Some(&reason),
            owner.map(|(account, _)| account),
        ) {
            return self.failed(req, &error);
        }
        let facts = format!(
            "<dl class=\"facts\"><dt>From</dt><dd>{}</dd><dt>To</dt><dd>{}</dd><dt>Received</dt><dd>{}</dd><dt>Subject</dt><dd>{}</dd></dl>\
             <div class=\"text\">{}</div>",
            esc(&letter.meta.from),
            esc(&letter.meta.to),
            esc(letter.meta.received.as_deref().unwrap_or("not stored")),
            esc(&letter.subject),
            esc(&letter.text),
        );
        let flash = Flash::good(
            format!(
                "This read is on record: {} read letter {id}, and wrote: \"{reason}\".",
                req.who.handle
            ),
            "Signed, sealed, and filed under 'curious'.",
        );
        let back = format!("<p><a href=\"/admin/letters/{mailbox}\">Back to the mailbox</a></p>");
        self.render(
            req,
            Nav::Letters,
            "A private letter",
            Some(&flash),
            &format!("{back}{}", window(&format!("Letter {id}"), &facts)),
        )
    }

    pub(crate) fn hold_place(&self, req: &Req) -> Reply {
        let Ok(account) = req.param("account").trim().parse::<u32>() else {
            return self.holds_with(
                req,
                Some(&Flash::bad(
                    "An account number is needed.",
                    "The form has a blank where a number should be.",
                )),
            );
        };
        match store::account_detail(&self.store, account) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return self.holds_with(
                    req,
                    Some(&Flash::bad(
                        format!("There is no account {account}."),
                        "The drawer is empty.",
                    )),
                );
            }
            Err(error) => return self.failed(req, &error),
        }
        let Some(reason) = clean_reason(req.param("reason")) else {
            return self.holds_with(
                req,
                Some(&Flash::bad(
                    "A hold needs a reason for the audit log.",
                    "Even the clerk's stamp wants a signature.",
                )),
            );
        };
        let now = dates::now_ms();
        let until = match dates::hold_until(req.param("until").trim(), now) {
            Ok(until) => until,
            Err(error) => {
                return self.holds_with(
                    req,
                    Some(&Flash::bad(
                        error.to_string(),
                        "The calendar on the wall disagrees.",
                    )),
                );
            }
        };
        let stored = match crate::retention::place_hold(&self.store, account, until) {
            Ok(stored) => stored,
            Err(innkeeper_world::StoreError::UnknownAccount(_)) => {
                return self.holds_with(
                    req,
                    Some(&Flash::bad(
                        format!("Account {account} was never issued, so there is nothing to hold."),
                        "The ledger has no such line.",
                    )),
                );
            }
            Err(error) => return self.failed(req, &error),
        };
        let through = dates::format_date(stored - dates::DAY);
        let target = format!("account {account} through {through}");
        if let Err(error) =
            self.log_change(req, "hold.place", &target, Some(&reason), Some(account))
        {
            return self.failed(req, &error);
        }
        let flash = Flash::good(
            format!("Account {account} is held through {through}. The purge will skip what it covers until then."),
            "A paperweight on the file. A heavy one.",
        );
        self.holds_with(req, Some(&flash))
    }

    pub(crate) fn hold_release(&self, req: &Req) -> Reply {
        if !req.confirmed() {
            return self.holds_with(req, Some(&needs_confirmation()));
        }
        let Ok(account) = req.param("account").trim().parse::<u32>() else {
            return self.not_found(req);
        };
        if let Err(error) = crate::retention::clear_hold(&self.store, account) {
            return self.failed(req, &error);
        }
        let reason = clean_reason(req.param("reason"));
        if let Err(error) = self.log_change(
            req,
            "hold.release",
            &format!("account {account}"),
            reason.as_deref(),
            Some(account),
        ) {
            return self.failed(req, &error);
        }
        let flash = Flash::good(
            format!(
                "The hold on account {account} is released. Its rows are back on the normal clock."
            ),
            "The paperweight is lifted. The papers flutter.",
        );
        self.holds_with(req, Some(&flash))
    }

    pub(crate) fn save_download(&self, req: &Req, account_text: &str) -> Reply {
        let Ok(account) = account_text.parse::<u32>() else {
            return self.not_found(req);
        };
        let path = req.param("path");
        let Ok(version) = req.param("version").parse::<u64>() else {
            return self.not_found(req);
        };
        let found =
            match self
                .settings
                .world
                .store
                .save_read(AccountId(account), path, Some(version))
            {
                Ok(found) => found,
                Err(error) => return self.failed(req, &error),
            };
        let Some((meta, data)) = found else {
            return self.not_found(req);
        };
        let target = format!("account {account} {} v{version}", meta.path);
        if let Err(error) = self.log_change(req, "save.download", &target, None, Some(account)) {
            return self.failed(req, &error);
        }
        let stem: String = meta
            .path
            .rsplit('\\')
            .next()
            .unwrap_or("save")
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let name = format!("account{account}-v{version}-{stem}");
        let mut reply = http::reply(
            StatusCode::OK,
            Some("application/octet-stream"),
            Bytes::from(data),
        );
        if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
            reply.headers_mut().insert(CONTENT_DISPOSITION, value);
        }
        reply
    }
}

type Outcome = Result<Flash, Refused>;

enum Refused {
    Flash(Flash),
    Store(innkeeper_world::StoreError),
}

impl From<innkeeper_world::StoreError> for Refused {
    fn from(error: innkeeper_world::StoreError) -> Self {
        Refused::Store(error)
    }
}
