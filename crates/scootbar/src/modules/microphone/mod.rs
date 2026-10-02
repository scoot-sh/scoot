//! The microphone module: the default source's level and mute, sharing
//! the volume module's code (same protocol, same handshake, same bounds;
//! only the device kind differs). Its registry id, settings and actions
//! are the volume one's, re-exported so the two cannot drift apart: a new
//! action or option added to one is added to both.

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = super::volume::MIC_ID;

pub use super::volume::{ACTIONS, Settings, init_microphone as init};

#[cfg(test)]
mod tests;
