//! The platform-independent heart of flexwm.
//!
//! [`World`] owns every piece of window-management state and has exactly two
//! ways in -- [`World::handle_event`] for things a platform observed and
//! [`World::handle_action`] for things a user or agent asked for -- and one way
//! out: [`World::arrange`], a pure projection of where every window should be.
//! It never touches a window.
//!
//! That boundary is what lets one layout drive both a Wayland compositor on
//! Linux, where flexwm owns the pixels, and an accessibility-API adapter on
//! macOS, where it can only ask other apps to move.

#![forbid(unsafe_code)]

mod config;
mod geometry;
mod layout;
mod messages;
mod types;
mod world;

pub use config::Config;
pub use geometry::{Point, Rect, Size};
pub use messages::{Action, Effect, Event, Horizontal, Vertical};
pub use types::{OutputId, SizeHints, WindowId, WindowInfo};
pub use world::{Arrangement, Placement, Workspaces, World};
