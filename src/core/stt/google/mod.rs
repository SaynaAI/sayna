//! Google Cloud Speech-to-Text provider implementation.
//!
//! This module provides the Google Cloud STT implementation, built on top of the
//! generic Google Cloud infrastructure in `core::providers::google`.

pub mod config;
mod provider;
mod streaming;

pub use config::GoogleSTTConfig;
pub use provider::{GoogleSTT, STTGoogleAuthClient};

#[cfg(test)]
mod tests;
