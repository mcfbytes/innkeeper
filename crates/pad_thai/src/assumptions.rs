//! PAD behaviour the client does not pin down; each names its section in
//! docs/protocol/link-layer.md.

/// Line break around PAD replies; the client only scans for the words (checklist 3 and 4).
pub(crate) const PAD_LINE_BREAK_ASSUMED: &str = "\r\n";
/// The PAD prompt; the client needs only the `@` (checklist 3).
pub(crate) const PAD_PROMPT_ASSUMED: &str = "\r\n@";
/// A Telenet-style PAD asks for the terminal type after `@D` CR (section 4 notes).
pub(crate) const TERMINAL_PROMPT_ASSUMED: &str = "\r\nTERMINAL=";
/// The dialogue is 7-bit, so a byte with bit 7 set during wake-up is a frame from a client
/// that dialled `DIRECT` and skipped the PAD (checklist 14).
pub(crate) const FRAMING_BIT_ASSUMED: u8 = 0x80;
/// Bare CRs during wake-up after which the PAD prompts even without `D` (checklist 3).
pub(crate) const BARE_RETURNS_BEFORE_PROMPT_ASSUMED: u8 = 2;
