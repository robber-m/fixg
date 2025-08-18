#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]
#![deny(warnings)]
pub mod config;
pub mod error;
pub mod gateway;
pub mod client;
pub mod session;
pub mod messages;
pub mod protocol;
pub mod storage;
#[cfg(feature = "aeron-ffi")]
pub mod aeron_ffi;

pub use client::{FixClient, FixHandler, InboundMessage};
pub use config::{FixClientConfig, GatewayConfig};
pub use gateway::{Gateway, GatewayHandle};
pub use session::{DisconnectReason, Session, SessionConfig};