//! The keymap: reading the `wl_keyboard.keymap` fd with a size cap and
//! compiling it, once per keymap change.
//!
//! Two hostile shapes meet here. The fd comes from the compositor over
//! `SCM_RIGHTS`, and its size is whatever the sender wrote: a hostile or
//! buggy sender can send gigabytes, an unterminated dump, or close early.
//! And every client's fd is the *same open file description* (Smithay's
//! `KeymapFile` seeks to 0 once, at creation; see
//! `docs/backlog/core/keymap-fd-shared-offset.md`), so a plain read after
//! another client has read gets zero bytes. The read therefore seeks to 0
//! first -- harmless when already there -- and never trusts the fd's size:
//! bytes are read through a `Take` capped at [`MAX_KEYMAP_BYTES`] plus one,
//! so anything larger fails before it is all in memory, and an empty read
//! is refused rather than compiled into an empty keymap that would decode
//! every key as nothing (libxkbcommon stays silent on that shape; kbvm
//! failed the same way in the spike, which is half of why it lost).
//!
//! `mmap` would dodge the shared offset too (offsets do not apply to
//! mappings), but it cannot cap without a separate `fstat`, and the read
//! path keeps every line of this module in safe code. The server-side fix
//! (rewind per send) removes the need for the seek without removing its
//! harmlessness.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::io::BorrowedFd;

use xkbcommon::xkb::{
    CONTEXT_NO_FLAGS, KEYMAP_COMPILE_NO_FLAGS, KEYMAP_FORMAT_TEXT_V1, Keycode, MOD_INVALID,
    Context, Keymap,
};

/// The largest keymap accepted: a real one is tens of kilobytes (scoot's
/// `us` map is ~36 KB), so 1 MiB is headroom, not a target. Anything
/// larger is refused before it is all in memory.
pub const MAX_KEYMAP_BYTES: usize = 1 << 20;

/// What reading or compiling a keymap can report. All cold paths: these
/// may allocate; per-event paths never build one.
#[derive(Debug)]
pub enum KeymapError {
    /// The `wl_keyboard.keymap` format was not `xkb_v1`. The protocol has
    /// exactly two formats (`no_keymap` and `xkb_v1`) and only the second
    /// carries a keymap.
    UnsupportedFormat(u32),
    /// More than [`MAX_KEYMAP_BYTES`] bytes on the fd.
    TooLarge { found: usize, cap: usize },
    /// Zero bytes: an empty keymap, or another read()-based client moved
    /// the shared offset and nobody rewound (the seek in
    /// [`read_keymap_bytes`] covers our own reads; this fires only when
    /// the fd itself is empty).
    Empty,
    /// The bytes are not UTF-8. The XKB text format is UTF-8 by
    /// construction; anything else is corruption or a wrong fd.
    InvalidUtf8,
    /// libxkbcommon refused the text: truncated, unterminated, or simply
    /// not a keymap. Silent on the library's side, loud here.
    Compile,
    /// The fd could not be read (closed early, `EBADF`, a short transport).
    Io(std::io::Error),
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(format) => {
                write!(f, "unsupported keymap format {format} (want xkb_v1)")
            }
            Self::TooLarge { found, cap } => {
                write!(f, "keymap of {found} bytes exceeds the {cap}-byte cap")
            }
            Self::Empty => write!(f, "keymap fd held zero bytes"),
            Self::InvalidUtf8 => write!(f, "keymap bytes are not valid UTF-8"),
            Self::Compile => write!(f, "libxkbcommon refused to compile the keymap"),
            Self::Io(error) => write!(f, "could not read the keymap fd: {error}"),
        }
    }
}

impl std::error::Error for KeymapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// Read the whole keymap fd into a `String`: seek to 0 first (the shared
/// file description's offset belongs to every client at once), then read
/// at most [`MAX_KEYMAP_BYTES`] plus one byte, so an oversized sender is
/// refused rather than fully buffered.
pub fn read_keymap_bytes(fd: BorrowedFd<'_>) -> Result<String, KeymapError> {
    // Duplicate first: the fd stays the caller's (the Wayland event owns
    // it, and the same number may be re-delivered), while the duplicate
    // shares the offset -- which is the point, since seeking the duplicate
    // to 0 rewinds the shared description for every reader at once.
    let owned = rustix::io::dup(fd)
        .map_err(|error| KeymapError::Io(error.into()))?;
    let mut file = File::from(owned);
    // A failed seek is not fatal (a non-seekable fd still reads from
    // wherever it is); an unseekable empty read reports Empty below.
    let _ = file.seek(SeekFrom::Start(0));
    let mut bytes = Vec::new();
    file.take((MAX_KEYMAP_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(KeymapError::Io)?;
    if bytes.len() > MAX_KEYMAP_BYTES {
        return Err(KeymapError::TooLarge {
            found: bytes.len(),
            cap: MAX_KEYMAP_BYTES,
        });
    }
    if bytes.is_empty() {
        return Err(KeymapError::Empty);
    }
    String::from_utf8(bytes).map_err(|_| KeymapError::InvalidUtf8)
}

/// A compiled keymap with the context that built it and the lock-modifier
/// indices its layouts bind, looked up once here rather than per key
/// event (`mod_name_is_active` heap-allocates its `CString` per call;
/// `mod_index_is_active` does not).
pub struct KeymapSet {
    context: Context,
    keymap: Keymap,
    caps: Option<u32>,
    num: Option<u32>,
}

impl KeymapSet {
    /// Compile keymap text (the [`read_keymap_bytes`] output). Empty text
    /// is refused before it reaches the library: an empty compile would
    /// succeed into a keymap that decodes everything as nothing.
    pub fn compile(text: String) -> Result<Self, KeymapError> {
        if text.is_empty() {
            return Err(KeymapError::Empty);
        }
        let context = Context::new(CONTEXT_NO_FLAGS);
        let keymap = Keymap::new_from_string(
            &context,
            text,
            KEYMAP_FORMAT_TEXT_V1,
            KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or(KeymapError::Compile)?;
        Ok(Self::wrap(context, keymap))
    }

    /// Compile from RMLVO names (rules, model, layout, variant, options),
    /// the same lookup the compositor did for the fd. For tests and for
    /// clients that name a layout instead of taking the server's: names
    /// with an interior NUL are refused rather than reaching the C string
    /// conversion inside the crate (which unwraps).
    pub fn from_names(
        rules: &str,
        model: &str,
        layout: &str,
        variant: &str,
        options: Option<&str>,
    ) -> Result<Self, KeymapError> {
        for name in [rules, model, layout, variant] {
            if name.as_bytes().contains(&0) {
                return Err(KeymapError::Compile);
            }
        }
        if options.is_some_and(|o| o.as_bytes().contains(&0)) {
            return Err(KeymapError::Compile);
        }
        let context = Context::new(CONTEXT_NO_FLAGS);
        let keymap = Keymap::new_from_names(
            &context,
            rules,
            model,
            layout,
            variant,
            options.map(str::to_owned),
            KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or(KeymapError::Compile)?;
        Ok(Self::wrap(context, keymap))
    }

    fn wrap(context: Context, keymap: Keymap) -> Self {
        // Fixed names: no NUL can hide in them, so the crate's internal
        // `CString::new(...).unwrap()` cannot fire. Missing is
        // `MOD_INVALID`, never an error: a layout without the modifier
        // simply reports it off.
        let present = |index: u32| (index != MOD_INVALID).then_some(index);
        let caps = present(keymap.mod_get_index("Lock"));
        let num = present(keymap.mod_get_index("Mod2"));
        Self {
            context,
            keymap,
            caps,
            num,
        }
    }

    pub(crate) fn context(&self) -> &Context {
        &self.context
    }

    pub(crate) fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// The `Lock` (Caps Lock) modifier index, when the layout binds one.
    pub(crate) fn caps_index(&self) -> Option<u32> {
        self.caps
    }

    /// The `Mod2` (Num Lock) modifier index, when the layout binds one.
    pub(crate) fn num_index(&self) -> Option<u32> {
        self.num
    }

    /// Whether this key produces repeats.
    pub fn key_repeats(&self, keycode: Keycode) -> bool {
        self.keymap.key_repeats(keycode)
    }
}
