use thiserror::Error;

/// Everything the link layer can reject.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LinkError {
    #[error("sequence number {0} is outside 0..=7")]
    InvalidSeq(u8),
    #[error("control byte {0:#04x} has no known frame type")]
    UnknownControl(u8),
    #[error("escaped payload of {escaped} bytes exceeds the frame limit")]
    PayloadTooLarge { escaped: usize },
    #[error("message of {len} bytes exceeds the 16-bit length prefix")]
    MessageTooLarge { len: usize },
}
