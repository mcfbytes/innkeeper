//! The page frame and the small pieces every page is built from. Every string that is not a literal
//! in this crate goes through [`esc`]: personas, posts and letters are written by players.

use std::fmt::Write as _;

use crate::dates;

/// Escapes text for an HTML element body or a quoted attribute value.
pub(crate) fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '`' => out.push_str("&#96;"),
            // Control characters other than tab and newline have no business in a page.
            ch if ch.is_control() && ch != '\n' && ch != '\t' => out.push('\u{fffd}'),
            ch => out.push(ch),
        }
    }
    out
}

/// What every page needs to know about the request that asked for it.
#[derive(Clone, Debug)]
pub(crate) struct Ctx {
    pub(crate) handle: String,
    pub(crate) csrf: String,
    pub(crate) dev_open: bool,
}

/// A narrator box: a plain fact line first, then the clerk's remark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    Good,
    Bad,
}

pub(crate) struct Flash {
    pub(crate) tone: Tone,
    pub(crate) fact: String,
    pub(crate) remark: Option<String>,
}

impl Flash {
    pub(crate) fn good(fact: impl Into<String>, remark: &str) -> Self {
        Flash {
            tone: Tone::Good,
            fact: fact.into(),
            remark: Some(remark.to_owned()),
        }
    }

    pub(crate) fn bad(fact: impl Into<String>, remark: &str) -> Self {
        Flash {
            tone: Tone::Bad,
            fact: fact.into(),
            remark: Some(remark.to_owned()),
        }
    }
}

pub(crate) fn narrator(tone: Tone, fact: &str, remark: Option<&str>) -> String {
    let class = match tone {
        Tone::Plain => "narr",
        Tone::Good => "narr ok",
        Tone::Bad => "narr bad",
    };
    let role = if tone == Tone::Bad { "alert" } else { "status" };
    let remark = remark.map_or_else(String::new, |text| format!("<small>{}</small>", esc(text)));
    format!(
        "<div class=\"{class}\" role=\"{role}\"><span>{}</span>{remark}</div>",
        esc(fact)
    )
}

/// A framed window with a title bar. `body` is already HTML.
pub(crate) fn window(title: &str, body: &str) -> String {
    format!(
        "<section class=\"window\"><div class=\"title\"><h2>{}</h2></div><div class=\"body\">{body}</div></section>",
        esc(title)
    )
}

pub(crate) fn window_with(title: &str, right: &str, body: &str) -> String {
    format!(
        "<section class=\"window\"><div class=\"title\"><h2>{}</h2><span>{right}</span></div><div class=\"body\">{body}</div></section>",
        esc(title)
    )
}

/// A POST form carrying the CSRF token. `confirm` makes it a destructive action: a dialog asks
/// first, and without script a visible "I mean it" box must be ticked (the server insists on it
/// either way).
pub(crate) struct Form<'a> {
    pub(crate) action: &'a str,
    pub(crate) csrf: &'a str,
    pub(crate) hidden: &'a [(&'a str, &'a str)],
    pub(crate) confirm: Option<Confirm<'a>>,
    pub(crate) inline: bool,
}

pub(crate) struct Confirm<'a> {
    pub(crate) question: &'a str,
    pub(crate) detail: &'a str,
    pub(crate) yes: &'a str,
}

impl Form<'_> {
    /// The opening tag, the CSRF field and the hidden fields.
    pub(crate) fn open(&self) -> String {
        let class = if self.inline { " class=\"inline\"" } else { "" };
        let confirm = self.confirm.as_ref().map_or_else(String::new, |c| {
            format!(
                " data-confirm=\"{}\" data-detail=\"{}\" data-yes=\"{}\"",
                esc(c.question),
                esc(c.detail),
                esc(c.yes)
            )
        });
        let mut out = format!(
            "<form method=\"post\" action=\"{}\"{class}{confirm}><input type=\"hidden\" name=\"csrf\" value=\"{}\">",
            esc(self.action),
            esc(self.csrf)
        );
        for (name, value) in self.hidden {
            let _ = write!(
                out,
                "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
                esc(name),
                esc(value)
            );
        }
        out
    }

    /// The tickbox that stands in for the dialog when script is off.
    pub(crate) fn confirm_box(&self) -> String {
        if self.confirm.is_none() {
            return String::new();
        }
        "<label class=\"confirm-box\"><input type=\"checkbox\" name=\"confirm\" value=\"yes\" required> I mean it</label>"
            .to_owned()
    }
}

pub(crate) fn button(label: &str, class: &str) -> String {
    format!(
        "<button type=\"submit\" class=\"abtn {}\">{}</button>",
        esc(class),
        esc(label)
    )
}

/// A one-button form for an action with no inputs.
pub(crate) fn action_form(form: &Form<'_>, label: &str, class: &str) -> String {
    format!(
        "{}{}{}</form>",
        form.open(),
        form.confirm_box(),
        button(label, class)
    )
}

pub(crate) fn when(unix: i64) -> String {
    dates::format_datetime(unix)
}

pub(crate) fn when_opt(unix: Option<i64>) -> String {
    unix.map_or_else(|| "not stored".to_owned(), when)
}

const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' shape-rendering='crispEdges'%3E%3Crect width='16' height='16' fill='%232a2250'/%3E%3Crect x='2' y='9' width='12' height='5' fill='%23a8743c'/%3E%3Crect x='5' y='2' width='6' height='6' fill='%23ffb23c'/%3E%3C/svg%3E";

/// Which menu entry is the current page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Nav {
    Dashboard,
    Accounts,
    SignIns,
    Audit,
    Complaints,
    Posts,
    Letters,
    Saves,
    Holds,
    Help,
    None,
}

fn item(href: &str, label: &str, here: bool) -> String {
    let current = if here { " aria-current=\"page\"" } else { "" };
    format!("<a href=\"{href}\"{current}>{label}</a>")
}

fn menubar(ctx: &Ctx, nav: Nav) -> String {
    let ledger = format!(
        "{}{}{}{}",
        item("/admin/", "Dashboard", nav == Nav::Dashboard),
        item("/admin/accounts", "Accounts", nav == Nav::Accounts),
        item("/admin/signins", "Sign-ins without an account", nav == Nav::SignIns),
        item("/admin/audit", "Audit log", nav == Nav::Audit),
    );
    let desk = format!(
        "{}{}{}",
        item("/admin/complaints", "Complaints", nav == Nav::Complaints),
        item("/admin/posts", "Bulletin posts", nav == Nav::Posts),
        item("/admin/letters", "Letters", nav == Nav::Letters),
    );
    let records = format!(
        "{}{}",
        item("/admin/saves", "Saved games", nav == Nav::Saves),
        item("/admin/holds", "Legal holds", nav == Nav::Holds),
    );
    let help = format!(
        "{}<div class=\"sep\"></div>\
         <button type=\"button\" data-theme-set=\"light\">Time: day</button>\
         <button type=\"button\" data-theme-set=\"dark\">Time: dusk</button>\
         <button type=\"button\" data-theme-set=\"system\">Time: follow system</button>",
        item("/admin/help", "How this works", nav == Nav::Help),
    );
    let menu = |name: &str, links: &str| {
        format!("<details><summary>{name}</summary><div class=\"pop\">{links}</div></details>")
    };
    format!(
        "<nav class=\"menubar\" aria-label=\"Back office\"><span class=\"brand\">Town Hall</span>{}{}{}{}\
         <span class=\"spacer\"></span><span class=\"who\">{}</span>\
         <form method=\"post\" action=\"/admin/logout\"><input type=\"hidden\" name=\"csrf\" value=\"{}\">\
         <button type=\"submit\" class=\"mbtn\">Sign out</button></form></nav>",
        menu("Ledger", &ledger),
        menu("Desk", &desk),
        menu("Records", &records),
        menu("Help", &help),
        esc(&ctx.handle),
        esc(&ctx.csrf),
    )
}

const DEV_BANNER: &str = "<div class=\"dev-open\" role=\"alert\"><b>DEV: open door</b> &mdash; \
    any signed-in identity can enter this portal. Never run it this way on a public host.</div>";

const DIALOG: &str = "<dialog id=\"confirm-dialog\" aria-labelledby=\"confirm-q\"><div class=\"box\">\
    <p class=\"q\" id=\"confirm-q\"></p><p class=\"detail muted\"></p>\
    <div class=\"btns\"><button type=\"button\" class=\"abtn\" data-no>Never mind</button>\
    <button type=\"button\" class=\"abtn danger\" data-yes>Yes, do it</button></div></div></dialog>";

fn head(title: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"color-scheme\" content=\"dark light\"><meta name=\"robots\" content=\"noindex,nofollow\">\
         <title>{} - Town Hall back office</title><link rel=\"icon\" href=\"{FAVICON}\">\
         <link rel=\"preconnect\" href=\"https://fonts.googleapis.com\">\
         <link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\
         <link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=VT323&amp;display=swap\">\
         <script src=\"/admin/static/theme.js\"></script>\
         <link rel=\"stylesheet\" href=\"/admin/static/admin.css\"></head>",
        esc(title)
    )
}

/// A whole signed-in page.
pub(crate) fn page(ctx: &Ctx, nav: Nav, title: &str, flash: Option<&Flash>, body: &str) -> String {
    let banner = if ctx.dev_open { DEV_BANNER } else { "" };
    let flash = flash.map_or_else(String::new, |flash| {
        narrator(flash.tone, &flash.fact, flash.remark.as_deref())
    });
    format!(
        "{}<body>{banner}<a class=\"skip\" href=\"#main\">Skip to the desk</a>{}\
         <main id=\"main\"><div class=\"vignette\"><canvas id=\"vignette\" aria-hidden=\"true\"></canvas>\
         <h1>{}</h1></div>{flash}{body}</main>{DIALOG}\
         <script src=\"/admin/static/admin.js\" defer></script>\
         <script src=\"/admin/static/vignette.js\" defer></script></body></html>",
        head(title),
        menubar(ctx, nav),
        esc(title),
    )
}

/// A page for someone who is not signed in: no menu, no token.
pub(crate) fn bare_page(dev_open: bool, title: &str, body: &str) -> String {
    let banner = if dev_open { DEV_BANNER } else { "" };
    format!(
        "{}<body>{banner}<main id=\"main\"><div class=\"vignette\"><canvas id=\"vignette\" aria-hidden=\"true\"></canvas>\
         <h1>{}</h1></div>{body}</main>\
         <script src=\"/admin/static/admin.js\" defer></script>\
         <script src=\"/admin/static/vignette.js\" defer></script></body></html>",
        head(title),
        esc(title),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_covers_markup_quotes_and_controls() {
        assert_eq!(
            esc("<b>\"x\" & 'y'</b>"),
            "&lt;b&gt;&quot;x&quot; &amp; &#39;y&#39;&lt;/b&gt;"
        );
        assert_eq!(esc("a\u{0}b\u{1b}c\nd"), "a\u{fffd}b\u{fffd}c\nd");
        assert_eq!(esc("`x`"), "&#96;x&#96;");
    }

    #[test]
    fn a_confirming_form_names_its_question_and_needs_the_tickbox_without_script() {
        let form = Form {
            action: "/admin/accounts/1/delete",
            csrf: "tok",
            hidden: &[],
            confirm: Some(Confirm {
                question: "Delete \"x\"?",
                detail: "d",
                yes: "Delete",
            }),
            inline: false,
        };
        let html = action_form(&form, "Delete", "danger");
        assert!(
            html.contains("data-confirm=\"Delete &quot;x&quot;?\""),
            "{html}"
        );
        assert!(
            html.contains("name=\"confirm\" value=\"yes\" required"),
            "{html}"
        );
        assert!(html.contains("name=\"csrf\" value=\"tok\""), "{html}");
    }
}
