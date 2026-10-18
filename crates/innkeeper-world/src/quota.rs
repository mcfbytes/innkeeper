//! How much one account may write in a day, and how full a mailbox or a board may get
//! (docs/server/mail.md section 9, docs/server/hub-services.md section 12).

use tracing::warn;

use crate::assumptions::{LETTERS_PER_DAY_ASSUMED, POSTS_PER_DAY_ASSUMED};
use crate::{AccountId, Store, StoreError};

/// The quotas count what was written in the last 24 hours.
const QUOTA_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// Whether `author` may send one more letter today.
pub(crate) fn may_send_letter(store: &dyn Store, author: AccountId) -> Result<bool, StoreError> {
    let written = store.written_within(author, QUOTA_WINDOW_MS)?;
    let allowed = written.letters < LETTERS_PER_DAY_ASSUMED;
    if !allowed {
        warn!(
            account = author.0,
            sent = written.letters,
            "letter refused: daily quota"
        );
    }
    Ok(allowed)
}

/// Whether `author` may write one more post (a board message, a complaint or a form) today.
pub(crate) fn may_post(store: &dyn Store, author: AccountId) -> Result<bool, StoreError> {
    let written = store.written_within(author, QUOTA_WINDOW_MS)?;
    let allowed = written.posts < POSTS_PER_DAY_ASSUMED;
    if !allowed {
        warn!(
            account = author.0,
            posted = written.posts,
            "post dropped: daily quota"
        );
    }
    Ok(allowed)
}
