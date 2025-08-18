use thiserror::Error;

/// Error types that can occur in the FIX engine.
///
/// This enum represents all possible error conditions that can arise
/// during FIX message processing, session management, and system operations.
#[derive(Error, Debug)]
pub enum FixgError {
    /// I/O operation failed
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A communication channel was closed unexpectedly
    #[error("Channel closed unexpectedly")]
    ChannelClosed,

    /// Configuration validation failed
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// FIX protocol violation or parsing error
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Session-level error occurred
    #[error("Session error: {0}")]
    Session(String),
}

pub type Result<T> = std::result::Result<T, FixgError>;
