//! How long a deleted or expired row stays before it is hard-purged.
//! The table and the reasons are in docs/server/privacy.md.

/// Days each kind of row is kept after it becomes deletable.
/// A legal hold pauses every clock for that account until the hold date has passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// A letter the player deletes. The 90 days match 18 U.S.C. 2703(f).
    pub mail_days: u32,
    /// A bulletin post deleted by its author or the operator.
    pub post_days: u32,
    /// An account deleted on request. Disabled at once; erasure within a month (GDPR).
    pub account_days: u32,
    /// The operator complaint board, counted from the post's arrival (REPORT Act 2024).
    pub complaint_days: u32,
    /// Sealed connection records, counted from when the connection opened.
    pub audit_days: u32,
    /// Expired session tokens, counted from expiry. The sessions table is not built yet.
    pub session_days: u32,
    /// The operator portal's audit log (who changed what, and every letter read), counted from the
    /// entry. A hold on the account an entry concerns keeps it.
    pub admin_audit_days: u32,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            mail_days: 90,
            post_days: 90,
            account_days: 30,
            complaint_days: 365,
            audit_days: 90,
            session_days: 30,
            admin_audit_days: 365,
        }
    }
}

impl RetentionPolicy {
    /// Seconds in `days`, for a cutoff of `now - seconds`.
    pub fn seconds(days: u32) -> i64 {
        i64::from(days) * 86_400
    }
}
