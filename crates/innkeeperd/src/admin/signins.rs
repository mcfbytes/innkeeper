//! Sign-ins without an account: the identities whose group owns no account, so the operator can
//! see people who signed in and then hit a wall (not invited, no age and terms answer, ...). Only
//! the identity row's plain columns are shown; the sealed subject is never read.

use std::fmt::Write as _;

use super::allowlist::hex;
use super::html::{esc, narrator, when, window, Nav, Tone};
use super::store;
use super::views::{table, tr};
use super::{Admin, Reply, Req};
use crate::signin::keys;
use crate::signin::CreateBlock;

/// Rows shown, newest first.
const SHOWN: i64 = 200;

fn block_text(block: Option<CreateBlock>) -> &'static str {
    match block {
        None => "Yes: nothing stops it (it has not asked, or gave up)",
        Some(CreateBlock::NotInvited) => "No: not on the invite list",
        Some(CreateBlock::NotAttested) => "No: has not answered the age and terms question",
        Some(CreateBlock::Closed) => "No: enrolment is closed",
        Some(CreateBlock::Suspended) => "No: its group owns a suspended account",
        Some(CreateBlock::DailyLimit) => "No: the accounts-per-day limit",
    }
}

impl Admin {
    /// `/admin/signins`.
    pub(crate) fn lone_signins(&self, req: &Req) -> Reply {
        let rows = match store::lone_identities(&self.store, SHOWN) {
            Ok(rows) => rows,
            Err(error) => return self.failed(req, &error),
        };
        let intro = "<p>Sign-in methods whose owner has no account number. Shown is what the identity row \
            holds in the clear: the provider, the identity key (an HMAC of the provider and the person's \
            subject, from which nobody can tell who they are), dates and the age and terms answer. The \
            subject itself is sealed for the escrow key and never shown here.</p>";
        if rows.is_empty() {
            let body = narrator(
                Tone::Plain,
                "Every signed-in identity has an account.",
                Some("The waiting room is empty. Someone even straightened the chairs."),
            );
            return self.render(req, Nav::SignIns, "Sign-ins without an account", None, &format!("{intro}{body}"));
        }
        let mut lines = String::new();
        for (n, row) in rows.iter().enumerate() {
            let block = match self.sign_in.create_block(&row.key) {
                Ok(block) => block,
                Err(error) => return self.failed(req, &error),
            };
            let key_id = format!("lone-key-{n}");
            let attested = row.attested.map_or_else(
                || "not answered".to_owned(),
                |(age, terms)| format!("{age}+, terms v{terms}"),
            );
            lines.push_str(&tr(&[
                esc(&keys::handle(&row.key)),
                esc(&row.provider),
                esc(&when(row.created_at / 1000)),
                esc(&row.last_session.map_or_else(|| "none kept".to_owned(), |at| when(at / 1000))),
                row.methods.to_string(),
                esc(&attested),
                esc(block_text(block)),
                format!(
                    "<code class=\"mono\" id=\"{key_id}\">{}</code> \
                     <button type=\"button\" class=\"abtn\" data-copy=\"{key_id}\">Copy</button>",
                    esc(&hex(&row.key))
                ),
            ]));
        }
        let mut more = String::new();
        if i64::try_from(rows.len()).unwrap_or(i64::MAX) >= SHOWN {
            let _ = write!(more, "<p class=\"muted\">The newest {SHOWN} are shown.</p>");
        }
        let help = "<p class=\"muted\">To invite one, add its key to the invite list \
            (<span class=\"mono\">echo KEY &gt;&gt; /etc/innkeeperd/invite-list</span>; no restart) or to the \
            vault secret <span class=\"mono\">invite-identities</span> (then restart). A method linked to an \
            invited person needs no invitation of its own; one that was linked by mistake into a group of \
            its own shows here with one method.</p>";
        let body = format!(
            "{intro}{}",
            window(
                &format!("{} sign-in method{}", rows.len(), if rows.len() == 1 { "" } else { "s" }),
                &format!(
                    "{}{more}{help}",
                    table(
                        &[
                            "Caller",
                            "Provider",
                            "First signed in",
                            "Last session",
                            "Methods",
                            "Age and terms",
                            "Can open an account?",
                            "Identity key",
                        ],
                        &lines
                    )
                )
            )
        );
        self.render(req, Nav::SignIns, "Sign-ins without an account", None, &body)
    }
}
