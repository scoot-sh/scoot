//! Images, from a file to the pixels of one output's buffer, with no
//! Wayland objects and no daemon state, so every step is a unit test. The
//! daemon runs [`render`] on a worker thread (`daemon::worker`), never on
//! the Wayland loop.
//!
//! The pipeline, as decided by measurement in
//! `docs/scootbg/backlog/resolved/dependencies-done.md` (§2, §3b, §6b):
//!
//! 1. [`decode`]: PNG, JPEG or WebP, sniffed from the first bytes, straight
//!    through `png`, `zune-jpeg` and `image-webp` into packed RGB, 3 bytes a
//!    pixel. The size is checked against [`decode::MAX_PIXELS`] from the
//!    header, before anything the size of the image is allocated, and the
//!    EXIF orientation is read ([`exif`]).
//! 2. [`fit`]: where the image goes on the output, per [`Mode`], worked out
//!    in the image's *displayed* orientation and mapped back to how it is
//!    stored ([`orientation`]).
//! 3. [`render`]: crop the stored image in place to what shows, scale it
//!    RGB to RGB ([`scale`], the one call into the scaler) to the stored
//!    orientation's target size, drop the source, and [`pack`] it into the
//!    XRGB8888 buffer in one pass that reads through the rotated index, so
//!    applying the orientation needs no buffer of its own.
//!
//! Images are treated as sRGB and written as 8 bits a channel: color
//! management is out of scope for v1.

#[cfg(test)]
mod bench;
pub mod decode;
pub mod exif;
pub mod fit;
/// The fuzz target's entry point, replayed over the committed corpus by a
/// stable test; the fuzz crate (`crates/scootbg/fuzz`) compiles it itself.
#[cfg(test)]
pub mod fuzz;
pub mod orientation;
pub mod pack;
pub mod render;
pub mod scale;

/// Test images made in code (PNG, WebP) or from the tiny committed JPEG
/// fixtures, shared by the image tests.
#[cfg(test)]
pub mod samples;

/// The stack of the thread that decodes and draws an image
/// (`daemon::worker`): 2 MiB, std's default, named so the fuzz target
/// (`fuzz`) runs its input on a thread of the same size, and an input that
/// needs more stack than the daemon has fails there too, not only here.
pub const DECODE_STACK: usize = 2 << 20;

/// How an image is fitted to an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Cover the output, cropping what overflows, centred (the default).
    #[default]
    Fill,
    /// Show all of it, as large as fits, centred, the rest in the fill
    /// color.
    Fit,
    /// Scale to the output's size, whatever the aspect ratio.
    Stretch,
    /// Unscaled, centred: cropped if larger, the rest in the fill color.
    Center,
    /// Unscaled, repeated from the top-left corner.
    Tile,
}

impl Mode {
    pub const ALL: [Self; 5] = [
        Self::Fill,
        Self::Fit,
        Self::Stretch,
        Self::Center,
        Self::Tile,
    ];

    /// The name on the command line, on the wire and in `query`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Fit => "fit",
            Self::Stretch => "stretch",
            Self::Center => "center",
            Self::Tile => "tile",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.name() == name)
    }
}

/// The resampling filter for modes that scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Sharpest, the default.
    #[default]
    Lanczos3,
    CatmullRom,
    Bilinear,
    /// No smoothing: for pixel art.
    Nearest,
}

impl Filter {
    pub const ALL: [Self; 4] = [
        Self::Lanczos3,
        Self::CatmullRom,
        Self::Bilinear,
        Self::Nearest,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Lanczos3 => "lanczos3",
            Self::CatmullRom => "catmull-rom",
            Self::Bilinear => "bilinear",
            Self::Nearest => "nearest",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|filter| filter.name() == name)
    }
}

/// Serialized by name, straight into the output.
impl serde::Serialize for Mode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl serde::Serialize for Filter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

#[cfg(test)]
mod tests;
