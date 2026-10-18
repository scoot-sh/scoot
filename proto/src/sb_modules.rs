//! Shims the `crate::modules::Class` half of `theme.rs`'s world, the way
//! the fuzz crate does: one real file, so `#[path]` resolves plainly.

#[path = "../../w9-launch/crates/scootbar/src/modules/class.rs"]
pub mod class;

pub use class::Class;
