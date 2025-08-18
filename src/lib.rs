#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]
#![deny(warnings)]
#[cfg(feature = "aeron-ffi")]
pub mod aeron_ffi;
pub mod client;
pub mod config;
pub mod error;
pub mod gateway;
pub mod messages;
pub mod protocol;
pub mod session;
pub mod storage;

pub use client::{FixClient, FixHandler, InboundMessage};
pub use config::{FixClientConfig, GatewayConfig};
pub use gateway::{Gateway, GatewayHandle};
pub use session::{DisconnectReason, Session, SessionConfig};
