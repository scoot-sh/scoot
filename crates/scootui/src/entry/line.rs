//! One line of text: a byte cap the caller sets, a UTF-8 aware caret,
//! word moves, BackSpace/Delete, Ctrl+W/U/A, paste (non-password only),
//! and a password mode that is masked, writes into a buffer the caller
//! supplies and never resizes, and refuses copy, paste and IME.

/// The most bytes an [`Entry`] ever holds: the launcher passes 512, a
/// password 1 KiB. A larger cap is clamped here, so a mistaken caller
/// cannot reserve megabytes on the content path.
pub const MAX_ENTRY_BYTES: usize = 8192;

/// What the entry draws per character in password mode: the font's
/// `.notdef` (a solid block in the test font), one per character, so the
/// mask keeps the length but never the text.
pub const MASK_CHAR: char = '*';

/// The X11 keysyms [`Entry::apply_key`] and the list understand, so
/// consumers match on one documented table rather than scattering
/// keysym constants through two apps.
pub const KEY_BACKSPACE: u32 = 0xff08;
pub const KEY_TAB: u32 = 0xff09;
pub const KEY_LEFT: u32 = 0xff51;
pub const KEY_UP: u32 = 0xff52;
pub const KEY_RIGHT: u32 = 0xff53;
pub const KEY_DOWN: u32 = 0xff54;
pub const KEY_PAGE_UP: u32 = 0xff55;
pub const KEY_PAGE_DOWN: u32 = 0xff56;
pub const KEY_END: u32 = 0xff57;
pub const KEY_HOME: u32 = 0xff50;
pub const KEY_DELETE: u32 = 0xffff;

/// What an insertion did. Insertions never split a character: text past
/// the cap (or past the caller's slice, for a password) is cut at the
/// last boundary that fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// All of it landed.
    Inserted,
    /// A prefix landed, cut at a character boundary.
    Partial,
    /// Nothing landed: the entry is full (or the text was empty).
    Full,
}

/// One line of editable text, UTF-8 throughout.
///
/// The buffer is sized once (at construction and at the first insert
/// that needs it) and never grows after: every editing method only
/// moves bytes inside it, so a warmed-up entry allocates nothing per
/// keystroke. Selection is select-all only (plus delete), as the design
/// says: there is no anchor, no drag, no clipboard but paste.
#[derive(Debug, Clone)]
pub struct Entry {
    text: Vec<u8>,
    cap: usize,
    caret: usize,
    selected_all: bool,
}

impl Entry {
    /// Empty, holding at most `cap` bytes (clamped to
    /// [`MAX_ENTRY_BYTES`]). The allocation happens here, once: later
    /// edits reuse it.
    pub fn new(cap: usize) -> Self {
        let cap = cap.min(MAX_ENTRY_BYTES);
        Self {
            text: Vec::with_capacity(cap),
            cap,
            caret: 0,
            selected_all: false,
        }
    }

    /// The text, always valid UTF-8: only `&str` ever enters, and cuts
    /// keep boundaries.
    pub fn text(&self) -> &str {
        str::from_utf8(&self.text).unwrap_or("")
    }

    /// Bytes held.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The byte cap.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// The caret as a byte index into [`Entry::text`], always a
    /// character boundary.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Whether select-all (Ctrl+A) is in force: the next insertion or
    /// deletion replaces the whole line.
    pub fn selected_all(&self) -> bool {
        self.selected_all
    }

    /// Insert `s` at the caret, replacing the selection first. Never
    /// splits a character; reports how much landed.
    pub fn insert_text(&mut self, s: &str) -> InsertOutcome {
        if s.is_empty() {
            return InsertOutcome::Full;
        }
        if self.selected_all {
            self.text.clear();
            self.caret = 0;
            self.selected_all = false;
        }
        let room = self.cap.saturating_sub(self.text.len());
        let mut take = room.min(s.len());
        while take > 0 && !s.is_char_boundary(take) {
            take -= 1;
        }
        if take == 0 {
            return InsertOutcome::Full;
        }
        let bytes = &s.as_bytes()[..take];
        // Within capacity after warm-up (the first insert sizes the
        // buffer to the cap), so this moves bytes, never allocates.
        let len = self.text.len();
        if self.text.capacity() < len + take {
            self.text.reserve(len + take - self.text.capacity());
        }
        self.text.resize(len + take, 0);
        self.text.copy_within(self.caret..len, self.caret + take);
        self.text[self.caret..self.caret + take].copy_from_slice(bytes);
        self.caret += take;
        if take < s.len() {
            InsertOutcome::Partial
        } else {
            InsertOutcome::Inserted
        }
    }

    /// Paste `s` (the launcher only: there is no paste method on
    /// [`PasswordEntry` at all, so a password paste cannot compile).
    pub fn paste(&mut self, s: &str) -> InsertOutcome {
        self.insert_text(s)
    }

    /// The selection for copy: the whole line when select-all is in
    /// force, else nothing. There is no finer selection to copy.
    pub fn copy_text(&self) -> Option<&str> {
        self.selected_all.then(|| self.text())
    }

    /// The launcher offers IME through `text-input-v3` later; the shape
    /// is already here so the follow-up wires a protocol, not a widget.
    pub fn accepts_ime(&self) -> bool {
        true
    }

    /// Feed one decoded key: navigation and control keys act, other text
    /// inserts. `ctrl` is the app's own modifier tracking (Ctrl held);
    /// `text` is what the seat decoded for the key (possibly empty for
    /// a navigation key). Returns whether the key did anything, so the
    /// app knows whether to keep it (Enter, Escape and Tab travel on to
    /// the list, the focus chain and the panel).
    pub fn apply_key(&mut self, keysym: u32, ctrl: bool, text: &str) -> bool {
        if ctrl {
            match keysym {
                KEY_LEFT => {
                    self.move_word_left();
                    return true;
                }
                KEY_RIGHT => {
                    self.move_word_right();
                    return true;
                }
                _ => {}
            }
            // Ctrl+letter arrives as the letter's keysym (the app says
            // Ctrl is held) or as a control character; both spellings
            // act, so neither app has to normalize first.
            let lower = ascii_lower(keysym);
            if lower == u32::from(b'w') || keysym == 0x17 {
                self.delete_word_back();
                return true;
            }
            if lower == u32::from(b'u') || keysym == 0x15 {
                self.delete_to_start();
                return true;
            }
            if lower == u32::from(b'a') || keysym == 0x01 {
                self.select_all();
                return true;
            }
            return false;
        }
        match keysym {
            KEY_BACKSPACE => {
                self.backspace();
                true
            }
            KEY_DELETE => {
                self.delete();
                true
            }
            KEY_LEFT => {
                self.move_left();
                true
            }
            KEY_RIGHT => {
                self.move_right();
                true
            }
            KEY_HOME => {
                self.move_home();
                true
            }
            KEY_END => {
                self.move_end();
                true
            }
            _ => {
                if text.is_empty() {
                    false
                } else {
                    // Control characters never land as text (they are
                    // Ctrl+letter without the flag, or a stray): typed
                    // text is printable.
                    let printable: &str = text.trim_matches(|c: char| c.is_control());
                    if printable.is_empty() {
                        false
                    } else {
                        self.insert_text(printable);
                        true
                    }
                }
            }
        }
    }

    /// One character left, clearing the selection.
    pub fn move_left(&mut self) {
        self.selected_all = false;
        self.caret = prev_boundary(self.text(), self.caret);
    }

    /// One character right, clearing the selection.
    pub fn move_right(&mut self) {
        self.selected_all = false;
        self.caret = next_boundary(self.text(), self.caret);
    }

    /// To the line start, clearing the selection.
    pub fn move_home(&mut self) {
        self.selected_all = false;
        self.caret = 0;
    }

    /// To the line end, clearing the selection.
    pub fn move_end(&mut self) {
        self.selected_all = false;
        self.caret = self.text.len();
    }

    /// One word left (words are `alphanumeric`/`_` runs): skip
    /// non-word characters, then the word itself.
    pub fn move_word_left(&mut self) {
        self.selected_all = false;
        self.caret = word_left(self.text(), self.caret);
    }

    /// One word right: skip the word under the caret, then non-word
    /// characters.
    pub fn move_word_right(&mut self) {
        self.selected_all = false;
        self.caret = word_right(self.text(), self.caret);
    }

    /// Delete the character before the caret (or the selection).
    pub fn backspace(&mut self) {
        if self.clear_selected() {
            return;
        }
        let prev = prev_boundary(self.text(), self.caret);
        if prev < self.caret {
            self.text.drain(prev..self.caret);
            self.caret = prev;
        }
    }

    /// Delete the character under the caret (or the selection).
    pub fn delete(&mut self) {
        if self.clear_selected() {
            return;
        }
        let next = next_boundary(self.text(), self.caret);
        if next > self.caret {
            self.text.drain(self.caret..next);
        }
    }

    /// Ctrl+W: kill back over the word and the whitespace before it
    /// (readline's `unix-word-rubout` shape, with the whitespace going
    /// too), or the selection.
    pub fn delete_word_back(&mut self) {
        if self.clear_selected() {
            return;
        }
        let start = rubout_left(self.text(), self.caret);
        if start < self.caret {
            self.text.drain(start..self.caret);
            self.caret = start;
        }
    }

    /// Ctrl+U: kill back to the line start (or the selection).
    pub fn delete_to_start(&mut self) {
        if self.clear_selected() {
            return;
        }
        if self.caret > 0 {
            self.text.drain(..self.caret);
            self.caret = 0;
        }
    }

    /// Ctrl+A: the whole line is the selection.
    pub fn select_all(&mut self) {
        self.selected_all = !self.text.is_empty();
    }

    /// Empties the line, keeping the buffer: the next fill allocates
    /// nothing.
    pub fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
        self.selected_all = false;
    }

    /// Drop the selection, deleting the whole line when it was in
    /// force. True when there was a selection.
    fn clear_selected(&mut self) -> bool {
        if self.selected_all {
            self.text.clear();
            self.caret = 0;
            self.selected_all = false;
            true
        } else {
            false
        }
    }
}

/// The password entry: the same caret and edit keys over a buffer the
/// caller supplies, which is never resized (it is a slice: growing it
/// cannot compile), masked at draw time, with no copy, no paste and no
/// IME.
///
/// The buffer the greeter passes lives in a locked, zeroized page (see
/// the greeter design); the launcher's `--password` prompt passes a
/// `zeroize`-on-drop buffer. Either way this widget leaves no copy of
/// the secret in freed heap: it owns no heap at all.
///
/// [`PasswordEntry::clear`] and `Drop` zero the whole slice, so a test
/// reading the caller's buffer after clear finds zeros.
#[derive(Debug)]
pub struct PasswordEntry<'a> {
    buf: &'a mut [u8],
    len: usize,
    caret: usize,
    selected_all: bool,
}

impl<'a> PasswordEntry<'a> {
    /// Empty, writing into `buf` (at most `buf.len()` bytes) and never
    /// beyond it. A zero-length slice is a valid, always-full entry.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self {
            buf,
            len: 0,
            caret: 0,
            selected_all: false,
        }
    }

    /// The secret. The caller builds the auth frame from these bytes
    /// and clears right after; nothing here copies them elsewhere.
    pub fn secret_bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    /// The whole caller slice, used and unused: the seam a test reads
    /// to prove zeroization (after [`PasswordEntry::clear`], after
    /// deletes, after drop of a scoped entry). It is the caller's own
    /// memory, borrowed back for the look.
    pub fn backing_bytes(&self) -> &[u8] {
        self.buf
    }

    /// The secret as text (valid UTF-8 by the same construction rule as
    /// [`Entry`]). For drawing nothing (the paint masks) and for
    /// measuring caret motion only: never log or display this.
    fn secret(&self) -> &str {
        str::from_utf8(self.secret_bytes()).unwrap_or("")
    }

    /// Characters (not bytes) held: what the mask draws.
    pub fn char_len(&self) -> usize {
        self.secret().chars().count()
    }

    /// Bytes held.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The cap: the slice's length, fixed for life.
    pub fn cap(&self) -> usize {
        self.buf.len()
    }

    /// The caret as a byte index, always a character boundary.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Characters before the caret: what the paint walks to place the
    /// caret bar over masked text.
    pub(crate) fn caret_chars(&self) -> usize {
        self.secret()
            .get(..self.caret.min(self.len))
            .unwrap_or("")
            .chars()
            .count()
    }

    pub fn selected_all(&self) -> bool {
        self.selected_all
    }

    /// Insert `s` at the caret, replacing the selection first, cut at
    /// the last boundary that fits the slice. Nothing past the slice
    /// ever exists, so nothing is left in freed heap by growing.
    pub fn insert_text(&mut self, s: &str) -> InsertOutcome {
        if s.is_empty() {
            return InsertOutcome::Full;
        }
        if self.selected_all {
            self.zero_used();
            self.selected_all = false;
        }
        let room = self.buf.len().saturating_sub(self.len);
        let mut take = room.min(s.len());
        while take > 0 && !s.is_char_boundary(take) {
            take -= 1;
        }
        if take == 0 {
            return InsertOutcome::Full;
        }
        let bytes = &s.as_bytes()[..take];
        // Shift the tail right inside the slice, then copy in: no heap
        // anywhere, only the caller's bytes moving inside themselves.
        // (`caret` is at most `len`, and `len + take` fits the slice by
        // the `room` cut above, so the window exists.)
        let tail = self.len.saturating_sub(self.caret);
        let rest = self.buf.split_at_mut(self.caret).1;
        if let Some(slot) = rest.get_mut(..tail + take) {
            slot.copy_within(..tail, take);
            slot[..take].copy_from_slice(bytes);
        } else {
            return InsertOutcome::Full;
        }
        self.len += take;
        self.caret += take;
        if take < s.len() {
            InsertOutcome::Partial
        } else {
            InsertOutcome::Inserted
        }
    }

    /// Refused, always: a password is never copied anywhere.
    pub fn copy_text(&self) -> Option<&str> {
        None
    }

    /// Refused, always: an IME candidate popup would show the secret.
    pub fn accepts_ime(&self) -> bool {
        false
    }

    /// The same key table as [`Entry::apply_key`], minus text landing
    /// control characters; printable text inserts.
    pub fn apply_key(&mut self, keysym: u32, ctrl: bool, text: &str) -> bool {
        if ctrl {
            match keysym {
                KEY_LEFT => {
                    self.move_word_left();
                    return true;
                }
                KEY_RIGHT => {
                    self.move_word_right();
                    return true;
                }
                _ => {}
            }
            let lower = ascii_lower(keysym);
            if lower == u32::from(b'w') || keysym == 0x17 {
                self.delete_word_back();
                return true;
            }
            if lower == u32::from(b'u') || keysym == 0x15 {
                self.delete_to_start();
                return true;
            }
            if lower == u32::from(b'a') || keysym == 0x01 {
                self.select_all();
                return true;
            }
            return false;
        }
        match keysym {
            KEY_BACKSPACE => {
                self.backspace();
                true
            }
            KEY_DELETE => {
                self.delete();
                true
            }
            KEY_LEFT => {
                self.move_left();
                true
            }
            KEY_RIGHT => {
                self.move_right();
                true
            }
            KEY_HOME => {
                self.move_home();
                true
            }
            KEY_END => {
                self.move_end();
                true
            }
            _ => {
                if text.is_empty() {
                    false
                } else {
                    let printable: &str = text.trim_matches(|c: char| c.is_control());
                    if printable.is_empty() {
                        false
                    } else {
                        self.insert_text(printable);
                        true
                    }
                }
            }
        }
    }

    pub fn move_left(&mut self) {
        self.selected_all = false;
        self.caret = prev_boundary(self.secret(), self.caret);
    }

    pub fn move_right(&mut self) {
        self.selected_all = false;
        self.caret = next_boundary(self.secret(), self.caret);
    }

    pub fn move_home(&mut self) {
        self.selected_all = false;
        self.caret = 0;
    }

    pub fn move_end(&mut self) {
        self.selected_all = false;
        self.caret = self.len;
    }

    pub fn move_word_left(&mut self) {
        self.selected_all = false;
        self.caret = word_left(self.secret(), self.caret);
    }

    pub fn move_word_right(&mut self) {
        self.selected_all = false;
        self.caret = word_right(self.secret(), self.caret);
    }

    pub fn backspace(&mut self) {
        if self.clear_selected() {
            return;
        }
        let prev = prev_boundary(self.secret(), self.caret);
        if prev < self.caret {
            self.remove_range(prev..self.caret);
            self.caret = prev;
        }
    }

    pub fn delete(&mut self) {
        if self.clear_selected() {
            return;
        }
        let next = next_boundary(self.secret(), self.caret);
        if next > self.caret {
            self.remove_range(self.caret..next);
        }
    }

    pub fn delete_word_back(&mut self) {
        if self.clear_selected() {
            return;
        }
        let start = rubout_left(self.secret(), self.caret);
        if start < self.caret {
            self.remove_range(start..self.caret);
            self.caret = start;
        }
    }

    pub fn delete_to_start(&mut self) {
        if self.clear_selected() {
            return;
        }
        if self.caret > 0 {
            self.remove_range(0..self.caret);
            self.caret = 0;
        }
    }

    pub fn select_all(&mut self) {
        self.selected_all = self.len > 0;
    }

    /// Zero the whole slice and empty the entry: a test reading the
    /// caller's buffer after this finds zeros.
    pub fn clear(&mut self) {
        self.buf.fill(0);
        self.len = 0;
        self.caret = 0;
        self.selected_all = false;
    }

    fn clear_selected(&mut self) -> bool {
        if self.selected_all {
            self.zero_used();
            self.selected_all = false;
            true
        } else {
            false
        }
    }

    /// Remove `range` (a char-aligned span inside the secret), shifting
    /// the tail left and zeroing the freed tail: no trace of the
    /// deleted bytes stays past `len`.
    fn remove_range(&mut self, range: core::ops::Range<usize>) {
        let end = range.end.min(self.len);
        let start = range.start.min(end);
        let tail = self.len.saturating_sub(end);
        self.buf.copy_within(end..end + tail, start);
        self.len = start + tail;
        if let Some(freed) = self.buf.get_mut(self.len..end + tail) {
            freed.fill(0);
        }
    }

    /// Zero the used prefix (selection replaced, whole line killed):
    /// the bytes stop existing here, not at the next clear.
    fn zero_used(&mut self) {
        if let Some(used) = self.buf.get_mut(..self.len) {
            used.fill(0);
        }
        self.len = 0;
        self.caret = 0;
    }
}

impl Drop for PasswordEntry<'_> {
    /// Zero the whole slice: dropping without clearing (panic unwinding
    /// past the login screen, an early return) leaves nothing either.
    fn drop(&mut self) {
        self.buf.fill(0);
    }
}

/// ASCII case-fold for keysym letters: Ctrl+W and Ctrl+w are the same
/// key. Non-ASCII keysyms pass through unchanged.
fn ascii_lower(keysym: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&keysym) {
        keysym + (u32::from(b'a') - u32::from(b'A'))
    } else {
        keysym
    }
}

/// The largest character boundary at or before `i`.
fn prev_boundary(s: &str, i: usize) -> usize {
    let mut prev = 0;
    for (index, _) in s.char_indices() {
        if index >= i {
            break;
        }
        prev = index;
    }
    // `i` past the end still steps to the last boundary, never past it.
    if i > s.len() { s.len() } else { prev }
}

/// The smallest character boundary at or after `i`.
fn next_boundary(s: &str, i: usize) -> usize {
    for (index, _) in s.char_indices() {
        if index > i {
            return index;
        }
    }
    s.len()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// One word left: over non-word characters, then over the word.
fn word_left(s: &str, caret: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = caret.min(bytes.len());
    while i > 0 {
        let prev = prev_boundary(s, i);
        if prev >= i {
            break;
        }
        let c = s.get(prev..i).and_then(|g| g.chars().next());
        if c.is_some_and(is_word_char) {
            break;
        }
        i = prev;
    }
    while i > 0 {
        let prev = prev_boundary(s, i);
        if prev >= i {
            break;
        }
        let c = s.get(prev..i).and_then(|g| g.chars().next());
        if !c.is_some_and(is_word_char) {
            break;
        }
        i = prev;
    }
    i
}

/// One word right: over the word under the caret, then over non-word
/// characters.
fn word_right(s: &str, caret: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = caret.min(bytes.len());
    while i < bytes.len() {
        let next = next_boundary(s, i);
        if next <= i {
            break;
        }
        let c = s.get(i..next).and_then(|g| g.chars().next());
        if !c.is_some_and(is_word_char) {
            break;
        }
        i = next;
    }
    while i < bytes.len() {
        let next = next_boundary(s, i);
        if next <= i {
            break;
        }
        let c = s.get(i..next).and_then(|g| g.chars().next());
        if c.is_some_and(is_word_char) {
            break;
        }
        i = next;
    }
    i
}

/// Ctrl+W: back over non-whitespace, then over whitespace (readline's
/// `unix-word-rubout`: whitespace-delimited, so `foo/bar` kills whole).
fn rubout_left(s: &str, caret: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = caret.min(bytes.len());
    while i > 0 {
        let prev = prev_boundary(s, i);
        if prev >= i {
            break;
        }
        let c = s.get(prev..i).and_then(|g| g.chars().next());
        if c.is_some_and(|c| c.is_whitespace()) {
            break;
        }
        i = prev;
    }
    while i > 0 {
        let prev = prev_boundary(s, i);
        if prev >= i {
            break;
        }
        let c = s.get(prev..i).and_then(|g| g.chars().next());
        if !c.is_some_and(|c| c.is_whitespace()) {
            break;
        }
        i = prev;
    }
    i
}
