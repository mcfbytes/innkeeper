//! The portal's pages: reads only. A GET never changes anything; the one thing that records is a
//! letter read, which is a POST (`actions.rs`).

use std::fmt::Write as _;

use hyper::StatusCode;
use innkeeper_world::AccountId;

use super::html::{
    self, button, esc, narrator, when, when_opt, window, window_with, Flash, Form, Nav, Tone,
};
use super::store::{self, AccountDetail};
use super::{Admin, Req};
use crate::dates;
use crate::signin::keys;

/// The named bits of `userFlags`, from docs/protocol/messages.md section 3.2 and
/// docs/server/hub-services.md section 3.
pub(crate) const FLAG_BITS: [(u16, &str); 4] = [
    (0x0200, "Mail stamps"),
    (0x0002, "Staff level 1"),
    (0x0080, "Staff level 2"),
    (0x0004, "Staff level 3 (moderator)"),
];

/// What a list shows for a persona the client has not named yet.
const NOT_NAMED: &str = "(not named yet)";

/// The persona, or a note when the account has none.
fn shown(persona: &str) -> &str {
    match persona.trim().is_empty() {
        true => NOT_NAMED,
        false => persona,
    }
}

/// One logged-in line, joined from the world and the switchboard.
#[derive(Clone, Debug)]
pub(crate) struct OnlineRow {
    pub(crate) connection: u64,
    pub(crate) account: u32,
    pub(crate) persona: String,
    pub(crate) land: String,
    pub(crate) since: Option<i64>,
    pub(crate) transport: &'static str,
}

fn tr(cells: &[String]) -> String {
    let mut row = String::from("<tr>");
    for cell in cells {
        row.push_str("<td>");
        row.push_str(cell);
        row.push_str("</td>");
    }
    row.push_str("</tr>");
    row
}

fn table(head: &[&str], rows: &str) -> String {
    let mut out = String::from("<div class=\"tablewrap\"><table><thead><tr>");
    for name in head {
        let _ = write!(out, "<th scope=\"col\">{}</th>", esc(name));
    }
    let _ = write!(out, "</tr></thead><tbody>{rows}</tbody></table></div>");
    out
}

fn link(href: &str, text: &str) -> String {
    format!("<a href=\"{}\">{}</a>", esc(href), esc(text))
}

fn account_link(id: u32) -> String {
    link(&format!("/admin/accounts/{id}"), &id.to_string())
}

fn flag(text: &str, class: &str) -> String {
    format!("<span class=\"flag {class}\">{}</span>", esc(text))
}

/// The flag bits that are set, by name, and the rest as hex.
pub(crate) fn flag_names(flags: u16) -> String {
    let mut out = String::new();
    let mut known = 0u16;
    for (bit, name) in FLAG_BITS {
        known |= bit;
        if flags & bit != 0 {
            out.push_str(&flag(name, ""));
        }
    }
    let other = flags & !known;
    if other != 0 {
        out.push_str(&flag(&format!("other bits {other:#06x}"), "warn"));
    }
    if out.is_empty() {
        out.push_str("<span class=\"muted\">none</span>");
    }
    out
}

impl Admin {
    pub(crate) fn land_names(&self) -> Vec<(u8, String)> {
        let world = &self.settings.world;
        world
            .lands
            .directory(world.host)
            .lands
            .into_iter()
            .map(|entry| (entry.land_type.0, entry.description.replace('_', " ")))
            .collect()
    }

    /// Everyone logged in now, oldest connection first.
    pub(crate) fn online(&self) -> Vec<OnlineRow> {
        let lands = self.land_names();
        let players = {
            let objects = self
                .settings
                .objects
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            objects.online()
        };
        let lines = self.settings.switchboard.connections();
        let mut rows: Vec<OnlineRow> = players
            .into_iter()
            .map(|player| {
                let line = lines
                    .iter()
                    .find(|line| line.connection == player.connection);
                let land = match player.place {
                    Some(place) => lands
                        .iter()
                        .find(|(kind, _)| *kind == place.land_type.0)
                        .map_or_else(
                            || format!("land type {}", place.land_type.0),
                            |(_, name)| name.clone(),
                        ),
                    None => "not in a land yet".to_owned(),
                };
                OnlineRow {
                    connection: player.connection.0,
                    account: player.account.0,
                    persona: player.persona,
                    land,
                    since: line.map(|line| line.since),
                    transport: line.map_or("", |line| line.transport),
                }
            })
            .collect();
        rows.sort_by_key(|row| row.connection);
        rows
    }

    pub(crate) fn online_for(&self, account: u32) -> Vec<OnlineRow> {
        self.online()
            .into_iter()
            .filter(|row| row.account == account)
            .collect()
    }

    pub(crate) fn dashboard(&self, req: &Req) -> Reply {
        self.dashboard_with(req, None)
    }

    pub(crate) fn dashboard_with(&self, req: &Req, flash: Option<&Flash>) -> Reply {
        let now = dates::now_ms();
        let counts = match store::counts(&self.store, now) {
            Ok(counts) => counts,
            Err(error) => return self.failed(req, &error),
        };
        let tile = |href: &str, number: i64, label: &str| {
            format!(
                "<a href=\"{href}\"><b>{number}</b><span>{}</span></a>",
                esc(label)
            )
        };
        let tiles = format!(
            "<div class=\"counts\">{}{}{}{}{}</div>",
            tile("/admin/accounts", counts.accounts, "accounts"),
            tile("/admin/letters", counts.letters, "letters"),
            tile("/admin/posts", counts.posts, "bulletin posts"),
            tile(
                "/admin/complaints",
                counts.complaints_waiting,
                "complaints waiting"
            ),
            tile("/admin/holds", counts.holds, "open legal holds"),
        );
        let waiting = if counts.complaints_waiting == 0 {
            "<p class=\"muted\">The in-tray is empty. The clerk is suspicious.</p>"
        } else {
            "<p>The in-tray has paper in it. Start with the complaints.</p>"
        };
        let online = self.online();
        let who = if online.is_empty() {
            "<p>Nobody is logged in. The switchboard is asleep.</p>".to_owned()
        } else {
            let mut rows = String::new();
            for row in &online {
                let id = row.connection.to_string();
                let hidden = [("connection", id.as_str())];
                let form = Form {
                    action: "/admin/kick",
                    csrf: &req.ctx.csrf,
                    hidden: &hidden,
                    confirm: Some(html::Confirm {
                        question: &format!("Kick {}?", shown(&row.persona)),
                        detail: "This ends their connection now. They can log on again unless the account is suspended.",
                        yes: "Kick",
                    }),
                    inline: true,
                };
                rows.push_str(&tr(&[
                    esc(shown(&row.persona)),
                    account_link(row.account),
                    esc(&row.land),
                    esc(&row.since.map_or_else(|| "unknown".into(), when)),
                    esc(row.transport),
                    html::action_form(&form, "Kick", "danger"),
                ]));
            }
            table(
                &["Persona", "Account", "Land", "Connected since", "Via", ""],
                &rows,
            )
        };
        let status = format!(
            "<ul><li>{}</li><li>{}</li></ul><p class=\"muted\">The first answers for the players' door through the proxy; the second is this office.</p>",
            link("/healthz", "Players' door: /healthz"),
            link("/admin/healthz", "This office: /admin/healthz"),
        );
        let body = format!(
            "<div class=\"grid\">{}{}</div>{}",
            window("The desk today", &format!("{tiles}{waiting}")),
            window("Status", &status),
            window_with(
                &format!("Who is in ({})", online.len()),
                "<a href=\"/admin/\">Refresh</a>",
                &who
            ),
        );
        self.render(req, Nav::Dashboard, "The back office", flash, &body)
    }

    pub(crate) fn accounts(&self, req: &Req) -> Reply {
        let query = req.param("q").to_owned();
        let found = match store::search_accounts(&self.store, &query) {
            Ok(found) => found,
            Err(error) => return self.failed(req, &error),
        };
        let search = format!(
            "<form method=\"get\" action=\"/admin/accounts\" class=\"row\" role=\"search\">\
             <div><label for=\"q\">Account number or persona</label>\
             <input type=\"search\" id=\"q\" name=\"q\" value=\"{}\" autofocus></div>{}</form>",
            esc(&query),
            button("Look up", "primary")
        );
        let results = if found.is_empty() {
            narrator(
                Tone::Plain,
                "No account matches that.",
                Some("The clerk checks under the desk. Nothing but a sandwich."),
            )
        } else {
            let mut rows = String::new();
            for account in &found {
                let mut marks = String::new();
                if account.purged_at.is_some() {
                    marks.push_str(&flag("purged", "bad"));
                } else if account.deleted_at.is_some() {
                    marks.push_str(&flag("deleted", "bad"));
                }
                if account.suspended_at.is_some() {
                    marks.push_str(&flag("suspended", "warn"));
                }
                rows.push_str(&tr(&[
                    account_link(account.id),
                    esc(shown(&account.persona)),
                    flag_names(account.user_flags),
                    marks,
                ]));
            }
            table(&["Number", "Persona", "Flags", "State"], &rows)
        };
        let title = if query.trim().is_empty() {
            "Newest accounts"
        } else {
            "Matches"
        };
        let body = format!(
            "{}{}",
            window("Look up an account", &search),
            window(title, &results)
        );
        self.render(req, Nav::Accounts, "Accounts", None, &body)
    }

    pub(crate) fn account(&self, req: &Req, id: &str) -> Reply {
        self.account_with(req, id, None, "")
    }

    /// The account page, with an optional message and extra HTML (a generated password) up top.
    pub(crate) fn account_with(
        &self,
        req: &Req,
        id: &str,
        flash: Option<&Flash>,
        extra: &str,
    ) -> Reply {
        let Ok(id) = id.parse::<u32>() else {
            return self.not_found(req);
        };
        let detail = match store::account_detail(&self.store, id) {
            Ok(Some(detail)) => detail,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let money = match self.settings.world.store.money(AccountId(id)) {
            Ok(money) => money.map(|money| money.0),
            Err(error) => return self.failed(req, &error),
        };
        let online = self.online_for(id);
        let body = format!(
            "{extra}{}{}{}",
            self.account_facts(&detail, money, &online),
            self.account_links(req, &detail),
            self.account_actions(req, &detail, money, !online.is_empty()),
        );
        let title = format!("Account {id}");
        self.render(req, Nav::Accounts, &title, flash, &body)
    }

    fn account_facts(&self, d: &AccountDetail, money: Option<u32>, online: &[OnlineRow]) -> String {
        let s = &d.summary;
        let mut state = String::new();
        if !online.is_empty() {
            state.push_str(&flag("online now", "ok"));
        }
        if s.suspended_at.is_some() {
            state.push_str(&flag("suspended", "warn"));
        }
        if let Some(at) = s.purged_at {
            state.push_str(&flag("purged", "bad"));
            let _ = write!(
                state,
                " <span class=\"muted\">purged {}: only the number is left, and it is never issued again</span>",
                esc(&when(at))
            );
        } else if let Some(at) = s.deleted_at {
            state.push_str(&flag("deleted", "bad"));
            let purge = dates::add_days(at, self.retention.account_days);
            let _ = write!(
                state,
                " <span class=\"muted\">disabled {}; purged for good {} unless held</span>",
                esc(&when(at)),
                esc(&dates::format_date(purge))
            );
        }
        if state.is_empty() {
            state.push_str(&flag("in good standing", "ok"));
        }
        let rights: Vec<String> = self
            .land_names()
            .into_iter()
            .filter(|(kind, _)| {
                kind.checked_sub(1)
                    .is_some_and(|bit| d.rating & (1u16 << bit) != 0)
            })
            .map(|(_, name)| name)
            .collect();
        let rating = if rights.is_empty() {
            format!(
                "none set ({:#06x}); a Login is then given every land",
                d.rating
            )
        } else {
            format!("{} ({:#06x})", rights.join(", "), d.rating)
        };
        let online_text = online
            .iter()
            .map(|row| {
                format!(
                    "connection {} in {} since {}",
                    row.connection,
                    row.land,
                    row.since.map_or_else(|| "unknown".into(), when)
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let mut facts = format!(
            "<dl class=\"facts\"><dt>Persona</dt><dd>{}</dd><dt>State</dt><dd>{state}</dd>",
            esc(shown(&s.persona))
        );
        if !online.is_empty() {
            let _ = write!(facts, "<dt>Online</dt><dd>{}</dd>", esc(&online_text));
        }
        let _ = write!(
            facts,
            "<dt>Created</dt><dd>{}</dd><dt>Last logon</dt><dd>{}</dd>\
             <dt>User flags</dt><dd>{} <span class=\"muted mono\">{:#06x}</span></dd>\
             <dt>Land rights</dt><dd>{}</dd><dt>CasinoBucks</dt><dd>{}</dd>\
             <dt>Password</dt><dd>{}</dd><dt>Mailbox</dt><dd>{}</dd>\
             <dt>Legal hold</dt><dd>{}</dd><dt>Saved games</dt><dd>{}</dd></dl>",
            esc(&when_opt(d.created_at)),
            esc(&when_opt(d.last_logon)),
            flag_names(s.user_flags),
            s.user_flags,
            esc(&rating),
            money.map_or_else(
                || "no bankroll stored yet (a player starts with 500)".to_owned(),
                |amount| amount.to_string()
            ),
            if d.password_expired {
                "expired; the player must pick a new one"
            } else {
                "current"
            },
            d.mailbox.map_or_else(
                || "none yet".to_owned(),
                |number| link(
                    &format!("/admin/letters/{number}"),
                    &format!("box {number}")
                )
            ),
            d.hold_until.map_or_else(
                || "none".to_owned(),
                |until| format!(
                    "held through {}",
                    esc(&dates::format_date(until - dates::DAY))
                )
            ),
            if d.save_files > 0 {
                link(
                    &format!("/admin/saves/{}", s.id),
                    &format!("{} files", d.save_files),
                )
            } else {
                "none".to_owned()
            },
        );
        window(&format!("{} (account {})", shown(&s.persona), s.id), &facts)
    }

    fn account_links(&self, _req: &Req, d: &AccountDetail) -> String {
        let mut rows = String::new();
        for identity in &d.identities {
            rows.push_str(&tr(&[
                esc(&keys::handle(&identity.key)),
                esc(&identity.provider),
            ]));
        }
        let identities = if d.identities.is_empty() {
            "<p class=\"muted\">No sign-in is linked: this account was made at a Login, not on the web.</p>".to_owned()
        } else {
            table(&["Identity", "Provider"], &rows)
        };
        let now = dates::now_ms();
        let mut rows = String::new();
        for session in &d.sessions {
            let state = if session.expiry > now {
                flag("live", "ok")
            } else {
                flag("ended", "")
            };
            rows.push_str(&tr(&[
                esc(&when(session.created_at)),
                esc(&when(session.expiry)),
                state,
            ]));
        }
        let sessions = if d.sessions.is_empty() {
            "<p class=\"muted\">No web sign-in sessions.</p>".to_owned()
        } else {
            table(&["Signed in", "Expires or ended", ""], &rows)
        };
        format!(
            "<div class=\"grid\">{}{}</div>",
            window("Linked identities", &format!("{identities}<p class=\"muted\">Handle and provider only: the portal never sees who a person is.</p>")),
            window("Web sessions", &sessions)
        )
    }

    fn account_actions(
        &self,
        req: &Req,
        d: &AccountDetail,
        money: Option<u32>,
        online: bool,
    ) -> String {
        let id = d.summary.id;
        let csrf = req.ctx.csrf.as_str();
        let action = |name: &str| format!("/admin/accounts/{id}/{name}");
        let reason = "<div><label>Reason (kept in the audit log, optional)</label><input type=\"text\" name=\"reason\" maxlength=\"200\"></div>";
        if d.summary.purged_at.is_some() {
            return window(
                "Actions",
                "<p class=\"muted\">A purged account cannot be restored.</p>",
            );
        }
        if d.summary.deleted_at.is_some() {
            let purge = d
                .summary
                .deleted_at
                .map_or(0, |at| dates::add_days(at, self.retention.account_days));
            let path = action("restore");
            let form = Form {
                action: &path,
                csrf,
                hidden: &[],
                confirm: None,
                inline: false,
            };
            let restore = format!(
                "{}<p>Restoring brings back the account and what the deletion hid. It works until {}.</p>{reason}{}</form>",
                form.open(),
                esc(&dates::format_date(purge)),
                button("Restore the account", "primary")
            );
            return window(
                "Actions",
                &format!("<div class=\"actions\">{restore}</div>"),
            );
        }
        let mut forms = String::new();
        let suspended = d.summary.suspended_at.is_some();
        if suspended {
            let path = action("unsuspend");
            let form = Form {
                action: &path,
                csrf,
                hidden: &[],
                confirm: None,
                inline: false,
            };
            let _ = write!(
                forms,
                "{}<h3>Unsuspend</h3><p>The player can log on again.</p>{reason}{}</form>",
                form.open(),
                button("Unsuspend", "primary")
            );
        } else {
            let path = action("suspend");
            let form = Form {
                action: &path, csrf, hidden: &[], inline: false,
                confirm: Some(html::Confirm {
                    question: &format!("Suspend {}?", shown(&d.summary.persona)),
                    detail: "Any live connection is kicked now, and a Login is refused until you unsuspend.",
                    yes: "Suspend",
                }),
            };
            let _ = write!(
                forms,
                "{}<h3>Suspend</h3><p>Kicks live lines and refuses a Login.</p>{reason}{}{}</form>",
                form.open(),
                form.confirm_box(),
                button("Suspend", "danger")
            );
        }
        let note = if online {
            "<p class=\"muted\">Online now: this is refused until you kick them or suspend the account.</p>"
        } else {
            ""
        };
        // Stamps and flags.
        let path = action("flags");
        let form = Form {
            action: &path,
            csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let mut boxes = String::new();
        for (bit, name) in FLAG_BITS {
            let checked = if d.summary.user_flags & bit != 0 {
                " checked"
            } else {
                ""
            };
            let _ = write!(boxes, "<label><input type=\"checkbox\" name=\"flag_{bit}\" value=\"1\"{checked}> {}</label>", esc(name));
        }
        for (kind, name) in self.land_names() {
            let Some(bit) = kind.checked_sub(1).filter(|bit| *bit < 16) else {
                continue;
            };
            let checked = if d.rating & (1u16 << bit) != 0 {
                " checked"
            } else {
                ""
            };
            let _ = write!(boxes, "<label><input type=\"checkbox\" name=\"land_{kind}\" value=\"1\"{checked}> Land: {}</label>", esc(&name));
        }
        let _ = write!(
            forms,
            "{}<h3>Stamps, staff level and lands</h3>{boxes}{note}{reason}{}</form>",
            form.open(),
            button("Save flags", "primary")
        );
        // CasinoBucks.
        let path = action("money");
        let form = Form {
            action: &path,
            csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let _ = write!(
            forms,
            "{}<h3>CasinoBucks</h3><label for=\"amount\">New balance</label><input type=\"number\" id=\"amount\" name=\"amount\" min=\"0\" max=\"4294967295\" value=\"{}\" required>{note}{reason}{}</form>",
            form.open(), money.map_or(String::new(), |m| m.to_string()), button("Set balance", "primary")
        );
        // Password.
        let path = action("password");
        let form = Form {
            action: &path,
            csrf,
            hidden: &[],
            inline: false,
            confirm: Some(html::Confirm {
                question: &format!("Reset the password of {}?", shown(&d.summary.persona)),
                detail: "A new password is made and shown once. The old one stops working.",
                yes: "Reset",
            }),
        };
        let _ = write!(forms, "{}<h3>Reset password</h3><p>Makes a new one and shows it once.</p>{note}{reason}{}{}</form>", form.open(), form.confirm_box(), button("Reset password", "danger"));
        // Sessions.
        let path = action("sessions");
        let form = Form {
            action: &path, csrf, hidden: &[], inline: false,
            confirm: Some(html::Confirm {
                question: "End every sign-in for this account?",
                detail: "The web sign-in tokens end now and live lines are kicked. The player signs in again.",
                yes: "Revoke",
            }),
        };
        let _ = write!(forms, "{}<h3>Revoke all sessions</h3><p>Ends web sign-ins and kicks live lines.</p>{reason}{}{}</form>", form.open(), form.confirm_box(), button("Revoke sessions", "danger"));
        // Delete.
        let path = action("delete");
        let preview = crate::subject::preview_delete(&self.store, AccountId(id))
            .ok()
            .flatten();
        let detail = preview.map_or_else(String::new, |p| {
            format!(
                "Hides {} letters in the box, {} letters sent, {} posts and {} saved versions.",
                p.letters_in_box, p.letters_sent, p.posts, p.save_versions
            )
        });
        let question = format!("Delete {}?", shown(&d.summary.persona));
        let yes_detail = format!(
            "{detail} The account is disabled now and erased for good after {} days, unless a legal hold covers it. Until then you can restore it.",
            self.retention.account_days
        );
        let form = Form {
            action: &path,
            csrf,
            hidden: &[],
            inline: false,
            confirm: Some(html::Confirm {
                question: &question,
                detail: &yes_detail,
                yes: "Delete",
            }),
        };
        let _ = write!(
            forms,
            "{}<h3>Delete</h3><p>Disabled now, erased after {} days. {}</p>{reason}{}{}</form>",
            form.open(),
            self.retention.account_days,
            esc(&detail),
            form.confirm_box(),
            button("Delete the account", "danger")
        );
        window("Actions", &format!("<div class=\"actions\">{forms}</div>"))
    }

    pub(crate) fn complaints(&self, req: &Req) -> Reply {
        let all = match store::complaints(&self.store, 300) {
            Ok(all) => all,
            Err(error) => return self.failed(req, &error),
        };
        let show_done = req.param("all") == "1";
        let waiting: Vec<_> = all.iter().filter(|c| c.handled.is_none()).collect();
        let listed: Vec<_> = all
            .iter()
            .filter(|c| show_done || c.handled.is_none())
            .collect();
        let body = if listed.is_empty() {
            narrator(
                Tone::Plain,
                "No complaints are waiting.",
                Some("The in-tray is empty. The clerk is suspicious."),
            )
        } else {
            let mut rows = String::new();
            for c in &listed {
                let state = match &c.handled {
                    Some((at, by)) => format!(
                        "{} {}",
                        flag("handled", "ok"),
                        esc(&format!("{} by {by}", when(*at)))
                    ),
                    None => flag("waiting", "warn"),
                };
                let preview: String = c.text.chars().take(90).collect();
                rows.push_str(&tr(&[
                    link(
                        &format!("/admin/complaints/{}", c.post_id),
                        &c.post_id.to_string(),
                    ),
                    esc(c.kind()),
                    esc(&c.author),
                    format!("<span class=\"wrap\">{}</span>", esc(&preview)),
                    esc(&when(c.created_at)),
                    state,
                ]));
            }
            window(
                &format!("{} waiting", waiting.len()),
                &table(
                    &[
                        "No.",
                        "Kind",
                        "From",
                        "Beginning of the text",
                        "Arrived",
                        "State",
                    ],
                    &rows,
                ),
            )
        };
        let toggle = if show_done {
            link("/admin/complaints", "Show only those waiting")
        } else {
            link("/admin/complaints?all=1", "Show handled ones too")
        };
        let note = "<p class=\"muted\">Complaints and Member Services messages are kept 365 days from arrival. Marking one handled adds a mark beside it; the original is never changed.</p>";
        self.render(
            req,
            Nav::Complaints,
            "Complaints and Member Services",
            None,
            &format!("<p>{toggle}</p>{body}{note}"),
        )
    }

    pub(crate) fn complaint(&self, req: &Req, id: &str) -> Reply {
        self.complaint_with(req, id, None)
    }

    pub(crate) fn complaint_with(&self, req: &Req, id: &str, flash: Option<&Flash>) -> Reply {
        let Ok(post_id) = id.parse::<u32>() else {
            return self.not_found(req);
        };
        let c = match store::complaint(&self.store, post_id) {
            Ok(Some(c)) => c,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        // Both links are the account numbers stored with the complaint, never a name lookup: the
        // writer's, and the one the named player was logged on as when it was filed.
        let author = c.account;
        let offender_name = c.offender();
        let mut facts = format!(
            "<dl class=\"facts\"><dt>Kind</dt><dd>{}</dd><dt>Arrived</dt><dd>{}</dd><dt>From</dt><dd>{} (account {})</dd>",
            esc(c.kind()),
            esc(&when(c.created_at)),
            esc(&c.author),
            account_link(author)
        );
        if offender_name.is_some() || c.about.is_some() {
            let _ = write!(
                facts,
                "<dt>Names</dt><dd>{}{}</dd>",
                esc(offender_name.as_deref().unwrap_or("")),
                c.about.map_or_else(
                    || " (nobody online under that name when it was filed)".to_owned(),
                    |id| format!(" (account {})", account_link(id))
                )
            );
        }
        let mut boxes = String::new();
        for id in [Some(author), c.about].into_iter().flatten() {
            if let Ok(Some(number)) = store::mailbox_of_account(&self.store, id) {
                let _ = write!(
                    boxes,
                    "{} ",
                    link(
                        &format!("/admin/letters/{number}"),
                        &format!("Letters of account {id}")
                    )
                );
            }
        }
        if !boxes.is_empty() {
            let _ = write!(facts, "<dt>Letters</dt><dd>{boxes}<span class=\"muted\">A complaint carries no letter number, so look in the mailboxes of the people it names.</span></dd>");
        }
        match &c.handled {
            Some((at, by)) => {
                let _ = write!(
                    facts,
                    "<dt>Handled</dt><dd>{} by {}</dd>",
                    esc(&when(*at)),
                    esc(by)
                );
            }
            None => facts.push_str("<dt>Handled</dt><dd>not yet</dd>"),
        }
        facts.push_str("</dl>");
        let text = format!("<div class=\"text\">{}</div>", esc(&c.text));
        let path = format!("/admin/complaints/{post_id}/handled");
        let form = Form {
            action: &path,
            csrf: &req.ctx.csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let (value, label) = if c.handled.is_some() {
            ("0", "Mark as waiting again")
        } else {
            ("1", "Mark as handled")
        };
        let mark = format!(
            "{}<input type=\"hidden\" name=\"handled\" value=\"{value}\">{}</form>",
            form.open(),
            button(label, "primary")
        );
        let body = format!(
            "{}{}{}",
            window(&format!("Complaint {post_id}"), &format!("{facts}{text}")),
            window("Desk", &mark),
            "<p>".to_owned() + &link("/admin/complaints", "Back to the list") + "</p>",
        );
        self.render(
            req,
            Nav::Complaints,
            &format!("Complaint {post_id}"),
            flash,
            &body,
        )
    }

    pub(crate) fn boards(&self, req: &Req) -> Reply {
        let boards = match store::boards(&self.store) {
            Ok(boards) => boards,
            Err(error) => return self.failed(req, &error),
        };
        let body = if boards.is_empty() {
            narrator(
                Tone::Plain,
                "No bulletin posts are kept.",
                Some("The corkboard is bare. A single pin glints."),
            )
        } else {
            let mut rows = String::new();
            for board in &boards {
                rows.push_str(&tr(&[
                    link(&format!("/admin/posts/{}", board.board), &board.label()),
                    board.posts.to_string(),
                    esc(&if board.newest > 0 {
                        when(board.newest)
                    } else {
                        "not stored".into()
                    }),
                ]));
            }
            window("Boards", &table(&["Board", "Posts", "Newest"], &rows))
        };
        self.render(req, Nav::Posts, "Bulletin posts", None, &body)
    }

    pub(crate) fn board(&self, req: &Req, board: &str) -> Reply {
        self.board_with(req, board, None)
    }

    pub(crate) fn board_with(&self, req: &Req, board: &str, flash: Option<&Flash>) -> Reply {
        let Ok(board) = board.parse::<u32>() else {
            return self.not_found(req);
        };
        let posts = match store::board_posts(&self.store, board, 200) {
            Ok(posts) => posts,
            Err(error) => return self.failed(req, &error),
        };
        let body = if posts.is_empty() {
            narrator(
                Tone::Plain,
                "This board has no posts.",
                Some("An echo answers, one beat late."),
            )
        } else {
            let mut rows = String::new();
            for post in &posts {
                let id = post.id.to_string();
                let path = format!("/admin/posts/{board}/{id}/remove");
                let form = Form {
                    action: &path,
                    csrf: &req.ctx.csrf,
                    hidden: &[],
                    inline: true,
                    confirm: Some(html::Confirm {
                        question: &format!("Remove post {id}?"),
                        detail: &format!(
                            "It is hidden from readers now and erased for good after {} days, unless a legal hold covers its author.",
                            self.retention.post_days
                        ),
                        yes: "Remove",
                    }),
                };
                rows.push_str(&tr(&[
                    id.clone(),
                    post.author.as_deref().map_or_else(|| "unknown".into(), esc),
                    format!("<div class=\"text\">{}</div>", esc(&post.text)),
                    esc(&if post.created_at > 0 {
                        when(post.created_at)
                    } else {
                        "not stored".into()
                    }),
                    html::action_form(&form, "Remove", "danger"),
                ]));
            }
            window(
                &store::board_label(board),
                &table(&["No.", "Author", "Text", "Posted", ""], &rows),
            )
        };
        let back = format!("<p>{}</p>", link("/admin/posts", "All boards"));
        self.render(
            req,
            Nav::Posts,
            "Board posts",
            flash,
            &format!("{back}{body}"),
        )
    }

    pub(crate) fn mailboxes(&self, req: &Req) -> Reply {
        let boxes = match store::mailboxes(&self.store) {
            Ok(boxes) => boxes,
            Err(error) => return self.failed(req, &error),
        };
        let intro = "<p>This list shows who wrote to whom, when, and how big the letter is. \
            Reading a letter asks for a reason, and the reason is recorded in the audit log with your handle and key.</p>";
        let body = if boxes.is_empty() {
            narrator(
                Tone::Plain,
                "No mailbox exists yet.",
                Some("The mail room is a very tidy nothing."),
            )
        } else {
            let mut rows = String::new();
            for mailbox in &boxes {
                rows.push_str(&tr(&[
                    link(
                        &format!("/admin/letters/{}", mailbox.number),
                        &format!("box {}", mailbox.number),
                    ),
                    account_link(mailbox.account),
                    esc(shown(&mailbox.persona)),
                    mailbox.letters.to_string(),
                ]));
            }
            window(
                "Mailboxes",
                &table(&["Box", "Account", "Persona", "Letters"], &rows),
            )
        };
        self.render(
            req,
            Nav::Letters,
            "Letters",
            None,
            &format!("{intro}{body}"),
        )
    }

    pub(crate) fn mailbox(&self, req: &Req, mailbox: &str) -> Reply {
        let Ok(number) = mailbox.parse::<u32>() else {
            return self.not_found(req);
        };
        let owner = match store::mailbox_owner(&self.store, number) {
            Ok(Some(owner)) => owner,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let letters = match store::letters_in(&self.store, number, 300) {
            Ok(letters) => letters,
            Err(error) => return self.failed(req, &error),
        };
        let body = if letters.is_empty() {
            narrator(
                Tone::Plain,
                "This mailbox holds no letters.",
                Some("The pigeonholes are all pigeon, no hole."),
            )
        } else {
            let mut rows = String::new();
            for letter in &letters {
                rows.push_str(&tr(&[
                    link(
                        &format!("/admin/letters/{number}/{}", letter.id),
                        &letter.id.to_string(),
                    ),
                    esc(&letter.from),
                    esc(&letter.to),
                    esc(letter.received.as_deref().unwrap_or("not stored")),
                    format!("{} bytes", letter.size),
                ]));
            }
            window(
                &format!("Box {number}: {} (account {})", owner.1, owner.0),
                &table(&["Letter", "From", "To", "Received", "Size"], &rows),
            )
        };
        let back = format!(
            "<p>{} &middot; {}</p>",
            link("/admin/letters", "All mailboxes"),
            account_link(owner.0)
        );
        self.render(
            req,
            Nav::Letters,
            &format!("Mailbox {number}"),
            None,
            &format!("{back}{body}"),
        )
    }

    /// The page before a letter is read: its envelope, the warning, and the reason form.
    pub(crate) fn letter_gate(&self, req: &Req, mailbox: &str, id: &str) -> Reply {
        self.letter_gate_with(req, mailbox, id, None, StatusCode::OK)
    }

    pub(crate) fn letter_gate_with(
        &self,
        req: &Req,
        mailbox: &str,
        id: &str,
        flash: Option<&Flash>,
        status: StatusCode,
    ) -> Reply {
        let (Ok(number), Ok(letter)) = (mailbox.parse::<u32>(), id.parse::<u32>()) else {
            return self.not_found(req);
        };
        let meta = match store::letter_meta_of(&self.store, number, letter) {
            Ok(Some(meta)) => meta,
            Ok(None) => return self.not_found(req),
            Err(error) => return self.failed(req, &error),
        };
        let facts = format!(
            "<dl class=\"facts\"><dt>From</dt><dd>{}</dd><dt>To</dt><dd>{}</dd><dt>Received</dt><dd>{}</dd><dt>Size</dt><dd>{} bytes</dd></dl>",
            esc(&meta.from),
            esc(&meta.to),
            esc(meta.received.as_deref().unwrap_or("not stored")),
            meta.size
        );
        let path = format!("/admin/letters/{number}/{letter}/read");
        let form = Form {
            action: &path,
            csrf: &req.ctx.csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let ask = format!(
            "{}<p><b>Before you read it:</b> a letter is private mail. Type why you need to read it. \
             Your reason is saved in the audit log with your handle and identity key, and is kept for 365 days.</p>\
             <label for=\"reason\">Reason (at least 8 characters)</label>\
             <textarea id=\"reason\" name=\"reason\" maxlength=\"300\" required minlength=\"8\"></textarea>\
             <p>{}</p></form>",
            form.open(),
            button("Record the reason and open the letter", "primary")
        );
        let back = format!(
            "<p>{}</p>",
            link(&format!("/admin/letters/{number}"), "Back to the mailbox")
        );
        let body = format!(
            "{back}{}{}",
            window(&format!("Letter {letter}"), &facts),
            window("Reading it", &ask)
        );
        self.render_status(req, status, Nav::Letters, "A private letter", flash, &body)
    }

    pub(crate) fn holds(&self, req: &Req) -> Reply {
        self.holds_with(req, None)
    }

    pub(crate) fn holds_with(&self, req: &Req, flash: Option<&Flash>) -> Reply {
        let holds = match store::holds(&self.store) {
            Ok(holds) => holds,
            Err(error) => return self.failed(req, &error),
        };
        let now = dates::now_ms();
        let list = if holds.is_empty() {
            "<p>No account is under a legal hold. Nothing is being kept past its time.</p>"
                .to_owned()
        } else {
            let mut rows = String::new();
            for hold in &holds {
                let open = hold.until > now;
                let cover = store::coverage(&self.store, hold.account).unwrap_or_default();
                let acct = hold.account.to_string();
                let path = "/admin/holds/release";
                let hidden = [("account", acct.as_str())];
                let form = Form {
                    action: path,
                    csrf: &req.ctx.csrf,
                    hidden: &hidden,
                    inline: true,
                    confirm: Some(html::Confirm {
                        question: &format!("Release the hold on account {acct}?"),
                        detail: "Its rows go back on the normal retention clock, and anything already past its window is erased at the next hourly purge.",
                        yes: "Release",
                    }),
                };
                rows.push_str(&tr(&[
                    account_link(hold.account),
                    esc(hold.persona.as_deref().unwrap_or("(erased)")),
                    esc(&format!(
                        "through {}",
                        dates::format_date(hold.until - dates::DAY)
                    )),
                    if open {
                        flag("open", "warn")
                    } else {
                        flag("over", "")
                    },
                    esc(&format!(
                        "{} sealed records, {} letters, {} saved versions, {} sign-in sessions",
                        cover.sealed_records, cover.letters, cover.save_versions, cover.sessions
                    )),
                    html::action_form(&form, "Release", "danger"),
                ]));
            }
            table(
                &[
                    "Account",
                    "Persona",
                    "Held",
                    "State",
                    "Currently covers",
                    "",
                ],
                &rows,
            )
        };
        let form = Form {
            action: "/admin/holds/place",
            csrf: &req.ctx.csrf,
            hidden: &[],
            confirm: None,
            inline: false,
        };
        let place = format!(
            "{}<div class=\"row\"><div><label for=\"account\">Account number</label><input type=\"number\" id=\"account\" name=\"account\" min=\"1\" required></div>\
             <div><label for=\"until\">Hold through (date)</label><input type=\"date\" id=\"until\" name=\"until\" required></div></div>\
             <div><label for=\"hreason\">Reason (kept in the audit log)</label><input type=\"text\" id=\"hreason\" name=\"reason\" maxlength=\"200\" required></div>\
             <p>{}</p></form>",
            form.open(),
            button("Place the hold", "primary")
        );
        let what = "<p>A legal hold stops the retention clock for one account. While it lasts, the hourly purge keeps that account's \
            sealed connection records, the letters in its box and the letters it sent, its posts and complaints, its saved games and sign-in sessions, \
            and the audit-log entries about it, even past their windows. A hold <b>overrides the purge</b>: nothing it covers is erased until you release it \
            or its date passes. A hold never makes anything visible that a deletion hid, and an extension never shortens an earlier date.</p>";
        let body = format!(
            "{}{}{}",
            window("What a hold does", what),
            window("Holds", &list),
            window("Place or extend a hold", &place)
        );
        self.render(req, Nav::Holds, "Legal holds", flash, &body)
    }

    pub(crate) fn save_owners(&self, req: &Req) -> Reply {
        let owners = match store::save_owners(&self.store) {
            Ok(owners) => owners,
            Err(error) => return self.failed(req, &error),
        };
        let body = if owners.is_empty() {
            narrator(
                Tone::Plain,
                "No saved games are stored.",
                Some("The save-game shelf holds a single dust bunny."),
            )
        } else {
            let mut rows = String::new();
            for owner in &owners {
                rows.push_str(&tr(&[
                    link(
                        &format!("/admin/saves/{}", owner.account),
                        &owner.account.to_string(),
                    ),
                    esc(owner.persona.as_deref().unwrap_or("(erased)")),
                    owner.files.to_string(),
                ]));
            }
            window(
                "Accounts with saved games",
                &table(&["Account", "Persona", "Files"], &rows),
            )
        };
        let note = "<p class=\"muted\">Saved games are shown and downloaded as they are. They cannot be edited here.</p>";
        self.render(
            req,
            Nav::Saves,
            "Saved games",
            None,
            &format!("{note}{body}"),
        )
    }

    pub(crate) fn saves_of(&self, req: &Req, account: &str) -> Reply {
        let Ok(id) = account.parse::<u32>() else {
            return self.not_found(req);
        };
        let files = match self.settings.world.store.saves(AccountId(id)) {
            Ok(files) => files,
            Err(error) => return self.failed(req, &error),
        };
        let body = if files.is_empty() {
            narrator(
                Tone::Plain,
                "This account has no saved files.",
                Some("The shelf is empty. So is the clerk's lunch box."),
            )
        } else {
            let mut sections = String::new();
            for file in &files {
                let versions = match self
                    .settings
                    .world
                    .store
                    .save_versions(AccountId(id), &file.path)
                {
                    Ok(versions) => versions,
                    Err(error) => return self.failed(req, &error),
                };
                let mut rows = String::new();
                for version in &versions {
                    let v = version.version.to_string();
                    let path = format!("/admin/saves/{id}/download");
                    let hidden = [("path", file.path.as_str()), ("version", v.as_str())];
                    let form = Form {
                        action: &path,
                        csrf: &req.ctx.csrf,
                        hidden: &hidden,
                        confirm: None,
                        inline: true,
                    };
                    rows.push_str(&tr(&[
                        format!(
                            "v{}{}",
                            version.version,
                            if version.current { " (current)" } else { "" }
                        ),
                        format!("{} bytes", version.size),
                        esc(&dates::format_datetime(
                            i64::try_from(version.modified / 1000).unwrap_or(0),
                        )),
                        format!(
                            "<span class=\"mono\">{}</span>",
                            esc(&version.hash.chars().take(16).collect::<String>())
                        ),
                        html::action_form(&form, "Download", ""),
                    ]));
                }
                sections.push_str(&window(
                    &file.path,
                    &table(&["Version", "Size", "Modified", "Hash", ""], &rows),
                ));
            }
            sections
        };
        let back = format!(
            "<p>{} &middot; {}</p>",
            link("/admin/saves", "All saved games"),
            account_link(id)
        );
        self.render(
            req,
            Nav::Saves,
            &format!("Saved games of account {id}"),
            None,
            &format!("{back}{body}"),
        )
    }

    pub(crate) fn audit(&self, req: &Req) -> Reply {
        let before = req.param("before").parse::<i64>().ok();
        let entries = match store::recent_audit(&self.store, 100, before) {
            Ok(entries) => entries,
            Err(error) => return self.failed(req, &error),
        };
        let body = if entries.is_empty() {
            narrator(
                Tone::Plain,
                "The audit log has nothing to show.",
                Some("The ledger is spotless. The clerk finds this worrying."),
            )
        } else {
            let mut rows = String::new();
            for entry in &entries {
                rows.push_str(&tr(&[
                    esc(&when(entry.at)),
                    esc(&entry.handle),
                    format!(
                        "<span class=\"mono\">{}</span>",
                        esc(&entry.key_hex.chars().take(16).collect::<String>())
                    ),
                    esc(&entry.action),
                    format!("<span class=\"wrap\">{}</span>", esc(&entry.target)),
                    format!(
                        "<span class=\"wrap\">{}</span>",
                        esc(entry.reason.as_deref().unwrap_or(""))
                    ),
                ]));
            }
            let more = entries.last().map_or_else(String::new, |last| {
                format!(
                    "<p>{}</p>",
                    link(&format!("/admin/audit?before={}", last.id), "Older entries")
                )
            });
            window(
                "Entries, newest first",
                &(table(
                    &["When", "Who", "Key (start)", "Action", "Target", "Reason"],
                    &rows,
                ) + &more),
            )
        };
        let note = format!(
            "<p class=\"muted\">Every change made here, and every letter read, is recorded and kept {} days (longer if a legal hold covers the account). \
             Entries cannot be edited or removed from the portal.</p>",
            self.retention.admin_audit_days
        );
        self.render(req, Nav::Audit, "Audit log", None, &format!("{note}{body}"))
    }

    pub(crate) fn help(&self, req: &Req) -> Reply {
        let text = "<p>This is the back office for whoever runs the server. Everything here goes through the same store the game uses, \
            so the usual rules (retention, holds, soft deletes) still apply.</p>\
            <ul>\
            <li><b>Dashboard</b>: who is connected now, with a Kick button, and the counts of what is waiting.</li>\
            <li><b>Accounts</b>: search, suspend, set flags and CasinoBucks, reset a password, end sessions, delete and restore. \
            Changes to flags, balance and password are refused while the player is online; suspend, revoke and delete kick them first.</li>\
            <li><b>Complaints</b>: the Member Services queue. Marking one handled never changes the original.</li>\
            <li><b>Bulletin posts</b>: removing one hides it, and it is erased after the post window.</li>\
            <li><b>Letters</b>: lists show who wrote to whom and when. Reading one needs a typed reason, which is logged.</li>\
            <li><b>Legal holds</b>, <b>Saved games</b> and the <b>Audit log</b>.</li></ul>\
            <p>Time of day (day, dusk or follow the system) is under Help.</p>";
        self.render(
            req,
            Nav::Help,
            "How this works",
            None,
            &window("The short version", text),
        )
    }
}

use super::Reply;
