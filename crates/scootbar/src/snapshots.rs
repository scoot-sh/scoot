//! Snapshot tests of what the bar draws: whole canvases compared, pixel
//! for pixel, with images checked in under `src/snapshots/`.
//!
//! The pixel tests beside each module (`paint`, `text`, `render`) assert
//! what they mean (a span's color, digits read back); these pin
//! everything else in the picture too (antialiasing, where a fractional
//! scale lands an edge, clipping), so a change to any of it shows up here
//! and is looked at, not only the parts someone thought to assert.
//!
//! The images are ASCII PNM, one image row per line: `P2` (PGM) for the
//! grayscale scenes, `P3` (PPM) for color. Any image viewer opens them,
//! and a change is a readable diff in review.
//!
//! **Updating one** after a change that is meant to alter the pixels:
//!
//! ```sh
//! SCOOTBAR_BLESS=1 cargo test -p scootbar snapshots
//! ```
//!
//! rewrites every snapshot a test compares, then look at the diff (or the
//! images) before committing it. Without it, a missing snapshot fails
//! rather than being written, so CI can never bless one. A failure writes
//! what was drawn beside the temporary directory's
//! `scootbar-snapshots/NAME.actual.pgm` and prints both as text.

use std::fmt::Write as _;
use std::path::PathBuf;

#[cfg(test)]
mod tests;

/// An image as the snapshots store it: one sample per pixel (gray) or
/// three (`r`, `g`, `b`), rows packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub gray: bool,
    pub samples: Vec<u8>,
}

impl Image {
    /// From an `XRGB8888` canvas (`crate::paint::Canvas`'s layout). A gray
    /// image needs every pixel gray: a scene drawn in gray tokens only.
    pub fn from_xrgb(pixels: &[u8], width: u32, height: u32, gray: bool) -> Self {
        let count = width as usize * height as usize;
        let mut samples = Vec::with_capacity(count * if gray { 1 } else { 3 });
        for (i, pixel) in pixels.chunks_exact(4).take(count).enumerate() {
            let [b, g, r] = [pixel[0], pixel[1], pixel[2]];
            if gray {
                assert!(
                    r == g && g == b,
                    "pixel {i} ({r}, {g}, {b}) is not gray: draw this scene in gray tokens"
                );
                samples.push(r);
            } else {
                samples.extend_from_slice(&[r, g, b]);
            }
        }
        assert_eq!(samples.len(), count * if gray { 1 } else { 3 });
        Self {
            width,
            height,
            gray,
            samples,
        }
    }

    fn channels(&self) -> usize {
        if self.gray { 1 } else { 3 }
    }

    fn extension(&self) -> &'static str {
        if self.gray { "pgm" } else { "ppm" }
    }

    /// The brightness of pixel `(x, y)`, 0 to 255.
    fn luma(&self, x: u32, y: u32) -> u8 {
        let i = (y as usize * self.width as usize + x as usize) * self.channels();
        match self.samples.get(i..i + self.channels()) {
            Some([v]) => *v,
            // Rec. 601 weights, in integers.
            Some([r, g, b]) => {
                ((u32::from(*r) * 299 + u32::from(*g) * 587 + u32::from(*b) * 114) / 1000) as u8
            }
            _ => 0,
        }
    }

    /// As ASCII PNM, with `comment` in its header.
    pub fn to_pnm(&self, comment: &str) -> String {
        let magic = if self.gray { "P2" } else { "P3" };
        let mut out = String::new();
        let _ = writeln!(out, "{magic}");
        for line in comment.lines() {
            let _ = writeln!(out, "# {line}");
        }
        let _ = writeln!(out, "{} {}\n255", self.width, self.height);
        let row = self.width as usize * self.channels();
        for samples in self.samples.chunks(row.max(1)) {
            let mut first = true;
            for sample in samples {
                if !first {
                    out.push(' ');
                }
                first = false;
                let _ = write!(out, "{sample}");
            }
            out.push('\n');
        }
        out
    }

    /// Reads ASCII PNM (`P2` or `P3`, a maximum of 255, `#` comments on
    /// lines of their own).
    pub fn from_pnm(text: &str) -> Result<Self, String> {
        let mut tokens = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(str::split_whitespace);
        let gray = match tokens.next() {
            Some("P2") => true,
            Some("P3") => false,
            other => return Err(format!("not ASCII PGM or PPM: {other:?}")),
        };
        let mut number = |what: &str| -> Result<u32, String> {
            let token = tokens.next().ok_or_else(|| format!("no {what}"))?;
            token.parse().map_err(|e| format!("{what} {token:?}: {e}"))
        };
        let width = number("width")?;
        let height = number("height")?;
        if number("maximum")? != 255 {
            return Err("the maximum is not 255".into());
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(if gray { 1 } else { 3 }))
            .filter(|&n| n <= 1 << 24)
            .ok_or("too large")?;
        let mut samples = Vec::with_capacity(count);
        for _ in 0..count {
            let value = number("sample")?;
            samples.push(u8::try_from(value).map_err(|_| format!("sample {value} > 255"))?);
        }
        if tokens.next().is_some() {
            return Err("samples past the image".into());
        }
        Ok(Self {
            width,
            height,
            gray,
            samples,
        })
    }

    /// The image as text, a character a pixel by brightness, at most
    /// `columns` wide.
    fn sketch(&self, columns: u32) -> String {
        const RAMP: &[u8] = b" .:-=+*#%@";
        let mut out = String::new();
        for y in 0..self.height {
            for x in 0..self.width.min(columns) {
                let level = usize::from(self.luma(x, y)) * (RAMP.len() - 1) / 255;
                out.push(char::from(RAMP[level]));
            }
            if self.width > columns {
                out.push('>');
            }
            out.push('\n');
        }
        out
    }
}

/// Where snapshot `name` is kept.
fn path(name: &str, image: &Image) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/snapshots")
        .join(format!("{name}.{}", image.extension()))
}

fn blessing() -> bool {
    std::env::var_os("SCOOTBAR_BLESS").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Compares `image` with snapshot `name` (`what` says what it shows, for
/// its header); with `SCOOTBAR_BLESS` set, writes it instead.
pub fn check(name: &str, what: &str, image: &Image) {
    let path = path(name, image);
    let comment = format!(
        "scootbar snapshot `{name}`: {what}\n\
         Made by the test of that name in src/snapshots/tests.rs; to update it,\n\
         SCOOTBAR_BLESS=1 cargo test -p scootbar snapshots (src/snapshots.rs)."
    );
    if blessing() {
        std::fs::write(&path, image.to_pnm(&comment))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        eprintln!("blessed {}", path.display());
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "no snapshot {} ({e}): make it with SCOOTBAR_BLESS=1 and look at it",
            path.display()
        )
    });
    let expected = Image::from_pnm(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if expected == *image {
        return;
    }
    let actual = std::env::temp_dir()
        .join("scootbar-snapshots")
        .join(format!("{name}.actual.{}", image.extension()));
    let written = std::fs::create_dir_all(actual.parent().unwrap_or(&actual))
        .and_then(|()| std::fs::write(&actual, image.to_pnm(&comment)));
    let mut report = format!("snapshot {name} differs from {}", path.display());
    match written {
        Ok(()) => {
            let _ = write!(report, "\n  drawn: {}", actual.display());
        }
        Err(e) => {
            let _ = write!(report, "\n  (could not write what was drawn: {e})");
        }
    }
    if (expected.width, expected.height, expected.gray) != (image.width, image.height, image.gray) {
        let _ = write!(
            report,
            "\n  expected {}x{} {}, drawn {}x{} {}",
            expected.width,
            expected.height,
            expected.extension(),
            image.width,
            image.height,
            image.extension()
        );
    } else {
        let channels = image.channels();
        let differing: Vec<usize> = (0..image.samples.len() / channels)
            .filter(|&p| {
                expected.samples[p * channels..(p + 1) * channels]
                    != image.samples[p * channels..(p + 1) * channels]
            })
            .collect();
        let worst = expected
            .samples
            .iter()
            .zip(&image.samples)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let first = differing.first().copied().unwrap_or(0);
        let _ = write!(
            report,
            "\n  {} pixel(s) differ, by up to {worst}; the first at ({}, {})",
            differing.len(),
            first % image.width.max(1) as usize,
            first / image.width.max(1) as usize,
        );
    }
    let _ = write!(
        report,
        "\n--- expected\n{}--- drawn\n{}",
        expected.sketch(120),
        image.sketch(120)
    );
    panic!("{report}");
}
