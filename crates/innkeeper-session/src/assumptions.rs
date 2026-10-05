//! Session behaviour the original client does not pin down; each names its document and section.

use std::time::Duration;

/// How long the host holds its frames across a program switch if the child stays silent: longer
/// than a child takes to load and send its first message (docs/protocol/int14h-api.md 9.3).
pub(crate) const PROGRAM_SWITCH_QUIET_MAX_ASSUMED: Duration = Duration::from_secs(10);
