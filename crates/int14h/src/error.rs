use thiserror::Error;

/// Everything the API types and the transport codec can reject.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Int14hError {
    #[error("export index {0} is outside the 17-entry table")]
    UnknownExport(u8),
    #[error("{field} code {value} is not defined")]
    UnknownCode { field: &'static str, value: u8 },
    #[error("{field} of {len} bytes exceeds its limit of {max}")]
    TooLong {
        field: &'static str,
        len: usize,
        max: usize,
    },
    #[error("{field} holds a byte outside printable ASCII")]
    NotAscii { field: &'static str },
    #[error("boolean byte {0} is neither 0 nor 1")]
    InvalidBool(u8),
    #[error("envelope ended early")]
    Truncated,
    #[error("{0} bytes left over after the envelope body")]
    TrailingBytes(usize),
    #[error("envelope kind {0:#04x} is not defined")]
    UnknownKind(u8),
    #[error("HELLO does not start with the protocol magic")]
    BadMagic,
    #[error("envelope of {0} bytes exceeds the transport limit")]
    EnvelopeTooLarge(usize),
}
