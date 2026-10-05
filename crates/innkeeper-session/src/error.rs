use thiserror::Error;

/// Everything a session can refuse to do.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    #[error("no PAD call is up, so there is no link to send on")]
    NoHostCall,
}
