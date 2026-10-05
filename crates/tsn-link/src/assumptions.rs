//! Link-layer facts the original client does not confirm; each names its section in
//! docs/protocol/link-layer.md.

/// A CR while no frame is open means the client left framing to talk to the PAD (checklist 12).
pub(crate) const PAD_ESCAPE_BYTE_ASSUMED: u8 = b'\r';
