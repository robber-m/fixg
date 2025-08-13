use thiserror::Error;

#[derive(Debug, Error)]
pub enum FixgError {
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("gateway error: {0}")]
    Gateway(String),

    #[error("client error: {0}")]
    Client(String),

    #[error("channel closed")]
    ChannelClosed,
}

pub type Result<T> = std::result::Result<T, FixgError>;