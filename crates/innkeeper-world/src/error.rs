use thiserror::Error;

/// Everything the message parsers can reject.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MessageError {
    #[error("command {0} has no parser")]
    UnsupportedCommand(u8),
    #[error("command {command} sub-command {sub} has no parser")]
    UnsupportedSub { command: u8, sub: u8 },
    #[error("{field} value {value} is not defined")]
    UnknownValue { field: &'static str, value: u16 },
    #[error("message ended inside {field}")]
    Truncated { field: &'static str },
    #[error("{0} bytes left over after the message")]
    TrailingBytes(usize),
    #[error("{field} holds a byte outside ASCII")]
    NotAscii { field: &'static str },
}
