use thiserror::Error;

/// Everything the PAD dialogue can reject.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PadError {
    #[error("{0:?} is not a PAD host address")]
    InvalidHostAddress(String),
}
