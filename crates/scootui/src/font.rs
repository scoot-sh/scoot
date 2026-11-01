//! Finding and loading the bar's font: a file path, no fontconfig.
//!
//! `--font PATH` names it; without the flag, the first usable file in a
//! short fixed list of well-known places ([`WELL_KNOWN`]) is used, and with
//! none the bar refuses to start, saying how to give it one. A font is
//! needed only when a module is placed: a bar with no modules draws no text.
//!
//! An app with a look can also name a family ([`Spec::family`]) and the
//! file the look tool resolved it to ([`Spec::file`], from fontconfig or the
//! look's own font files, found once at `scoot look apply`). Between the
//! flag and the well-known list the file is tried first and reads nothing
//! else; with no file (a hand-edited config, a look applied by an older
//! tool) [`family`] looks for the family's files in the usual font
//! directories instead. A bold face beside the regular one (the same for a
//! `--font` file or a well-known one) is loaded when the app draws bold.
//! A family or file that cannot be used is a note and the next choice,
//! never a refusal. All of it happens when the text is built, at start or
//! on a reload, never when a frame is drawn.
//!
//! ## Mapped or read: the decision
//!
//! M0 chose to map the font (`mmap`): with `ab_glyph`'s lazy parser, a
//! mapping costs only the pages actually read, in the page cache, shared
//! with every other process using the font, where reading costs the whole
//! file in the bar's heap (DejaVu Sans: 742 KiB). But **a mapped file
//! truncated in place kills the bar with `SIGBUS`**, and `cp new.ttf
//! ~/.local/share/fonts/font.ttf` over the one in use does exactly that
//! (it opens the old file with `O_TRUNC`).
//!
//! So the font is mapped **only where nothing but a root process rewriting the
//! file in place can change it** (root ignores the write bit): a root-owned file with no write bit,
//! on a read-only mount (`scootbg_mem::file::map_if_read_only`). That is
//! NixOS's `/nix/store` (root, `0444`, mounted read-only), where Stylix and
//! the NixOS modules take fonts from. A read-only mount alone is not
//! enough: it is a property of the mount, and the same file is often
//! writable elsewhere (a read-only bind mount, systemd's
//! `ProtectHome=read-only`, flatpak's `/run/host/fonts`), which a review
//! showed ends in `SIGBUS`. **Everywhere else it is read into the heap**,
//! at most [`MAX_FONT`] bytes, and no later write to the file can reach the
//! bar, at the price of the file's size in memory. The measured cost of
//! each is in `docs/scootbar/backlog/resolved/dependencies-done.md` §8.
//!
//! Opened `O_NONBLOCK` and checked to be a regular file first, so a FIFO or
//! a device cannot hang or balloon start-up.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use ab_glyph::{FontArc, FontRef, FontVec};
use rustix::fs::{FileType, Mode, OFlags, fstat, open};

use crate::text::{MAX_FALLBACKS, Text};

pub mod family;

#[cfg(test)]
mod tests;

/// The largest font file taken, in bytes. The largest common fonts, CJK
/// collections, are 20 to 30 MB; past this is not a font meant for a bar.
pub const MAX_FONT: u64 = 64 * 1024 * 1024;

/// Where a font is looked for without `--font`: a regular sans face in the
/// place each distribution's package puts it, first found wins.
pub const WELL_KNOWN: &[&str] = &[
    // Debian, Ubuntu (fonts-dejavu-core).
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    // Fedora (dejavu-sans-fonts).
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
    // Arch (ttf-dejavu).
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    // openSUSE, Alpine.
    "/usr/share/fonts/truetype/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    // NixOS with `fonts.fontDir.enable` and DejaVu installed.
    "/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf",
    // Noto Sans: Debian and Ubuntu (fonts-noto-core), Arch (noto-fonts).
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
];

/// How the font's bytes are held (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// Mapped: a root-owned, unwritable file on a read-only mount.
    Mapped,
    /// Read into the heap.
    Read,
}

/// A loaded font.
#[derive(Clone)]
pub struct Font {
    pub face: FontArc,
    pub path: PathBuf,
    pub held: Held,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Font")
            .field("path", &self.path)
            .field("held", &self.held)
            .finish_non_exhaustive()
    }
}

/// Why no font could be used.
#[derive(Debug)]
pub enum Error {
    /// `--font` was not given and no well-known file is usable; `tried`
    /// says what was wrong with each that exists. `named` says why the
    /// look's own file or family, if it had one, was not used either.
    NoneFound {
        named: Vec<String>,
        tried: Vec<(PathBuf, FileError)>,
    },
    /// The `--font` file cannot be used.
    Given { path: PathBuf, error: FileError },
    /// A `bar.fallback-fonts` file cannot be used.
    Fallback { path: PathBuf, error: FileError },
}

/// Why one file cannot be used.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    NotRegular,
    Empty,
    TooLarge(u64),
    /// ttf-parser cannot read it as a TrueType or OpenType font.
    NotAFont,
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::NotRegular => write!(f, "not a regular file"),
            Self::Empty => write!(f, "empty"),
            Self::TooLarge(size) => {
                write!(f, "{size} bytes, larger than the {MAX_FONT} a font may be")
            }
            Self::NotAFont => write!(f, "not a TrueType or OpenType font"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Given { path, error } => {
                write!(f, "cannot use the font {}: {error}", path.display())
            }
            Self::Fallback { path, error } => {
                write!(
                    f,
                    "cannot use the fallback font {}: {error}",
                    path.display()
                )
            }
            Self::NoneFound { named, tried } => {
                write!(f, "no font: ")?;
                if !named.is_empty() {
                    write!(f, "{}, and ", named.join("; "))?;
                }
                write!(
                    f,
                    "none of the usual font files is usable; give one with \
                     `--font PATH` (a .ttf or .otf file, such as DejaVuSans.ttf)"
                )?;
                for (path, error) in tried {
                    write!(f, "; {}: {error}", path.display())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Error {}

/// The bar's text: the font `given` names (or the first well-known one)
/// with `fallbacks` behind it, each a file that must load: a fallback that
/// cannot is a loud refusal, like the primary, not a silent gap in the
/// glyphs. At most [`MAX_FALLBACKS`] are taken (the config refuses more).
pub fn text(given: Option<&Path>, fallbacks: &[PathBuf]) -> Result<Text, Error> {
    text_of(find(given)?, fallbacks)
}

/// `primary` with `fallbacks` behind it.
fn text_of(primary: Font, fallbacks: &[PathBuf]) -> Result<Text, Error> {
    let fallbacks = fallbacks
        .iter()
        .take(MAX_FALLBACKS)
        .map(|path| {
            load(path)
                .map(|font| font.face)
                .map_err(|error| Error::Fallback {
                    path: path.clone(),
                    error,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Text::with_fallbacks(primary.face, fallbacks))
}

/// What an app asks of [`text_with`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Spec<'a> {
    /// The font file the user named (`--font`, a config's `font`): it wins.
    pub given: Option<&'a Path>,
    /// The font file the look tool resolved the family to (the fragment's
    /// `font-file`), tried before any directory is read. Ignored when
    /// `given` is set.
    pub file: Option<&'a Path>,
    /// The look's family, looked for in the font directories when no file
    /// is given or the file cannot be used. Empty: none.
    pub family: &'a str,
    /// Fallback files, as for [`text`].
    pub fallbacks: &'a [PathBuf],
    /// Whether the app draws bold, and so wants the family's bold face.
    pub bold: bool,
}

/// [`text`] for an app with a look: the font `spec.given` names, else the
/// look's resolved `spec.file`, else its `spec.family` found by name, else
/// the first well-known one, with the bold face beside it when
/// `spec.bold`. The note says why a named file or family was not used (not
/// installed, or its file cannot be read), for the caller to log; the
/// text is the next choice then. A bold file that cannot be used is
/// dropped without a note (the app strikes regular glyphs twice, as it did
/// before there was a bold face).
pub fn text_with(spec: &Spec<'_>) -> Result<(Text, Option<String>), Error> {
    build(spec, family::font_dirs)
}

/// [`text_with`] looking for the family in the directories `dirs` lists,
/// asked only if the walk is needed.
fn build(
    spec: &Spec<'_>,
    dirs: impl FnOnce() -> Vec<PathBuf>,
) -> Result<(Text, Option<String>), Error> {
    let looked_up = spec.given.is_none();
    let mut notes: Vec<String> = Vec::new();
    let mut primary = None;
    let mut bold_path = None;
    if looked_up && let Some(file) = spec.file {
        match load(file) {
            Ok(font) => {
                bold_path = spec
                    .bold
                    .then(|| family::bold_sibling(&font.path))
                    .flatten();
                primary = Some(font);
            }
            Err(error) => notes.push(format!(
                "the look's font file {}: {error}{}",
                file.display(),
                if spec.family.is_empty() {
                    String::new()
                } else {
                    format!("; looking for `{}` instead", spec.family)
                }
            )),
        }
    }
    if primary.is_none() && looked_up && !spec.family.is_empty() {
        match family::find_family(spec.family, &dirs(), spec.bold) {
            Some(found) => match load(&found.regular) {
                Ok(font) => {
                    primary = Some(font);
                    bold_path = found.bold;
                }
                Err(error) => notes.push(format!(
                    "the look's font `{}`: {}: {error}",
                    spec.family,
                    found.regular.display()
                )),
            },
            None => notes.push(format!(
                "the look's font `{}` has no .ttf or .otf file in the font directories",
                spec.family
            )),
        }
    }
    // The note ends with the fallback only when the default font was the
    // outcome: a file that failed but whose family was found says no more.
    let default_used = primary.is_none();
    let primary = match primary {
        Some(primary) => primary,
        None => {
            let font = find(spec.given).map_err(|error| match error {
                Error::NoneFound { tried, .. } => Error::NoneFound {
                    named: std::mem::take(&mut notes),
                    tried,
                },
                other => other,
            })?;
            bold_path = spec
                .bold
                .then(|| family::bold_sibling(&font.path))
                .flatten();
            font
        }
    };
    let mut text = text_of(primary, spec.fallbacks)?;
    if let Some(bold) = bold_path.as_deref().and_then(|path| load(path).ok()) {
        text = text.with_bold(bold.face);
    }
    let note = (!notes.is_empty()).then(|| {
        let tail = if default_used {
            "; using the default font"
        } else {
            ""
        };
        format!("{}{tail}", notes.join("; "))
    });
    Ok((text, note))
}

/// The font `--font` names, or the first usable well-known one.
pub fn find(given: Option<&Path>) -> Result<Font, Error> {
    if let Some(path) = given {
        return load(path).map_err(|error| Error::Given {
            path: path.to_owned(),
            error,
        });
    }
    let mut tried = Vec::new();
    for candidate in WELL_KNOWN {
        let path = Path::new(candidate);
        match load(path) {
            Ok(font) => return Ok(font),
            // Absent is the common case, and says nothing worth listing.
            Err(FileError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tried.push((path.to_owned(), error)),
        }
    }
    Err(Error::NoneFound {
        named: Vec::new(),
        tried,
    })
}

/// Loads the font at `path`, mapped or read (see the module docs).
pub fn load(path: &Path) -> Result<Font, FileError> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY,
        Mode::empty(),
    )
    .map_err(|errno| FileError::Io(errno.into()))?;
    let stat = fstat(&fd).map_err(|errno| FileError::Io(errno.into()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(FileError::NotRegular);
    }
    let size = u64::try_from(stat.st_size).unwrap_or(0);
    if size == 0 {
        return Err(FileError::Empty);
    }
    if size > MAX_FONT {
        return Err(FileError::TooLarge(size));
    }
    let path = path.to_owned();
    if let Some(bytes) =
        scootbg_mem::file::map_if_read_only(&fd, MAX_FONT).map_err(FileError::Io)?
    {
        let face = FontRef::try_from_slice(bytes).map_err(|_| FileError::NotAFont)?;
        return Ok(Font {
            face: FontArc::new(face),
            path,
            held: Held::Mapped,
        });
    }
    let bytes = read_capped(fd, size)?;
    let face = FontVec::try_from_vec(bytes).map_err(|_| FileError::NotAFont)?;
    Ok(Font {
        face: FontArc::new(face),
        path,
        held: Held::Read,
    })
}

/// The file's bytes, at most [`MAX_FONT`]: a file that grew past it since
/// `fstat` is refused rather than read without end.
fn read_capped(fd: std::os::fd::OwnedFd, size: u64) -> Result<Vec<u8>, FileError> {
    // At most `MAX_FONT`, which fits a `usize` on every Linux target.
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    std::fs::File::from(fd)
        .take(MAX_FONT + 1)
        .read_to_end(&mut bytes)
        .map_err(FileError::Io)?;
    let read = bytes.len() as u64;
    if read > MAX_FONT {
        return Err(FileError::TooLarge(read));
    }
    if read == 0 {
        return Err(FileError::Empty);
    }
    Ok(bytes)
}
