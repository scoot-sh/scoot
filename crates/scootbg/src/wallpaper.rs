//! What an output is to show: a color, or an image as a `set` asked for
//! it.
//!
//! An image is shared (`Arc`) between the choices, each output's model and
//! the worker thread decoding it, so passing it around allocates nothing.
//! Two images are the same wallpaper only if they come from the same
//! request (`serial`): `set` of the same path again is a new request, and
//! reads the file again (it may have changed). Outputs of one size share
//! one image's pixels only within one request (`daemon::canvas::Pixels`
//! are found by `serial`).
//!
//! A remote image (`fetch`) decodes from its cache file (`path`), fetched
//! on demand by the worker thread (`crate::fetch`).

use std::sync::Arc;

use crate::color::Color;
use crate::fetch::Fetch;
use crate::image::render::Look;

#[cfg(test)]
mod tests;

/// An image as a `set` asked for it.
#[derive(Debug)]
pub struct Image {
    /// Absolute (the CLI resolves it, the daemon refuses anything else):
    /// the daemon's working directory is not the client's. A remote
    /// image's cache file (`crate::fetch::cached_path`), best effort when
    /// the cache directory is not UTF-8 (lossy): the worker decodes what
    /// [`crate::fetch::ensure`] returns, never this string, so this stays
    /// only a name for messages, the state file and the choice comparison.
    pub path: String,
    pub look: Look,
    /// The request's generation (`crate::waiters`): unique to it.
    pub serial: u64,
    /// A download, when the image is a URL: the worker fetches it into
    /// the cache before decoding. `None` for a file.
    pub fetch: Option<Fetch>,
}

#[derive(Debug, Clone)]
pub enum Wallpaper {
    Color(Color),
    Image(Arc<Image>),
}

impl PartialEq for Wallpaper {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Color(a), Self::Color(b)) => a == b,
            (Self::Image(a), Self::Image(b)) => a.serial == b.serial,
            _ => false,
        }
    }
}

impl Eq for Wallpaper {}

impl Wallpaper {
    /// The image, if it is one.
    pub fn image(&self) -> Option<&Arc<Image>> {
        match self {
            Self::Image(image) => Some(image),
            Self::Color(_) => None,
        }
    }
}
