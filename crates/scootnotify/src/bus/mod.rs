//! `org.freedesktop.Notifications` over scoot-dbus: the method routing,
//! the `Notify` body, hints, and `image-data`.

pub mod hints;
pub mod image;
pub mod notify;
pub mod server;

#[cfg(test)]
mod testutils;
