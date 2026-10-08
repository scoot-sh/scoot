//! The D-Bus wire protocol: framing, marshalling and parsing.
//!
//! The shared bus client speaks the protocol itself (no `zbus`, no
//! libdbus), in the shape the spike decided
//! (`docs/scootbar/spikes/dbus-client.md`): one multiplexed connection,
//! a poll-loop fd, a pending-call table, central `NameOwnerChanged`
//! tracking, and a marshaller for exactly the types the consumers need —
//! every basic type, plus `ARRAY`, `STRUCT`, `VARIANT` and `DICT_ENTRY`.
//!
//! This file is `std` only, on purpose: the fuzz crate compiles it by
//! `#[path]` (`crates/scootbar/fuzz`), as the volume module's `proto.rs`
//! is, so nothing here may use anything but `std`.
//!
//! ## Bounds
//!
//! A same-user peer cannot smuggle malformed bytes through either daemon;
//! what it can send is absurd-but-valid shapes (deep nesting, huge
//! arrays, megabyte pixmaps). The bounds below answer those, and every
//! refusal is a dropped message, never a panic:
//!
//! - [`MAX_MESSAGE`]: a message past 1 MiB is not processed (the spec
//!   allows 128 MiB, [`MAX_WIRE`]; SNI has no way to ask an item for a
//!   size, so a pixmap past 512 by 512 can legitimately arrive). It is
//!   *skipped*, not fatal: [`frame_header`] frames it, the connection
//!   discards its bytes as they arrive and, for a reply, tells the
//!   consumer which call lost its answer. Only a header that is not a
//!   message at all, or past the spec's own maximum, ends the connection.
//! - [`MAX_DEPTH`]: containers nested past 32 deep are refused.
//! - [`MAX_NAME`]: a bus, interface, member or error name past the spec's
//!   255 bytes is refused; a path past [`MAX_PATH`] is.
//! - [`MAX_SIGNATURE`]: a signature past the spec's 255 bytes is refused.
//!
//! Strings in a body carry no cap here: they are borrowed, never copied,
//! and the consumer bounds them at use (titles are cut like view text).
//! The writer mirrors the caps: anything past them overflows, and
//! [`Writer::finish`] returns `None`.
//!
//! ## Endianness
//!
//! The client sends little-endian only and parses little-endian only:
//! every body reader below (`read_names`, `read_pixmaps`, the tray's
//! property walk) takes bytes with no byte order beside them. A
//! big-endian message is framed (so it is skipped whole, never a lost
//! connection) and then dropped by [`Message::parse`]: a big-endian peer
//! on the bus loses its own messages, nothing else.

/// The most bytes one message holds that this client processes, header
/// and body: a 512 by 512 pixmap is just over it, a 256 by 256 one
/// (the most the tray keeps) a quarter. Past it a message is skipped
/// whole ([`frame_header`]), never a lost connection.
pub const MAX_MESSAGE: usize = 1024 * 1024;

/// The most bytes the spec allows one message (128 MiB): a header
/// claiming more is not a message, and ends the connection.
pub const MAX_WIRE: usize = 128 * 1024 * 1024;

/// The deepest containers nest, in a signature or on the wire.
pub const MAX_DEPTH: usize = 32;

/// The longest bus, interface, member or error name taken, in bytes (the
/// spec's own maximum).
pub const MAX_NAME: usize = 255;

/// The longest object path taken, in bytes: the spec sets none, an item's
/// is a few dozen.
pub const MAX_PATH: usize = 1024;

/// The longest signature taken, in bytes (the spec's own maximum).
pub const MAX_SIGNATURE: usize = 255;

/// A message's kind, the header's second byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    MethodCall,
    MethodReturn,
    Error,
    Signal,
}

impl Kind {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::MethodCall),
            2 => Some(Self::MethodReturn),
            3 => Some(Self::Error),
            4 => Some(Self::Signal),
            _ => None,
        }
    }
}

/// Header flags the client sends or honors.
pub mod flag {
    /// No reply is wanted (activation calls set it).
    pub const NO_REPLY_EXPECTED: u8 = 0x1;
}

/// `RequestName` flags (`org.freedesktop.DBus.RequestName`).
pub mod request {
    /// Let a later owner take the name (a replacing watcher demotes us to
    /// a host, which we watch for).
    pub const ALLOW_REPLACEMENT: u32 = 0x1;
    /// Refuse the queue when the name is owned and cannot be taken: the
    /// reply says `Exists` at once instead of waiting.
    pub const DO_NOT_QUEUE: u32 = 0x4;
}

/// `RequestName` replies (`org.freedesktop.DBus.RequestName`).
pub mod request_reply {
    pub const PRIMARY_OWNER: u32 = 1;
    pub const ALREADY_OWNER: u32 = 4;
}

/// The total length of the message `buf` starts, from its 16-byte header
/// alone: `Ok(None)` while the header is incomplete, `Err(())` when it is
/// not a message (bad magic, version or kind, or a length past
/// [`MAX_WIRE`]). The length may be past [`MAX_MESSAGE`]: the caller
/// skips such a message rather than buffer it.
pub fn frame_header(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 16 {
        return Ok(None);
    }
    match buf[0] {
        b'l' | b'B' => {}
        _ => return Err(()),
    }
    if buf[3] != 1 {
        return Err(());
    }
    Kind::from_byte(buf[1]).ok_or(())?;
    let le = buf[0] == b'l';
    let body_len = get_u32(&buf[4..8], le) as usize;
    let fields_len = get_u32(&buf[12..16], le) as usize;
    // The spec's own bounds: the fields array at most 64 MiB, the whole
    // message 128 MiB.
    if fields_len > MAX_WIRE / 2 || body_len > MAX_WIRE {
        return Err(());
    }
    let mut total = 16usize.saturating_add(fields_len);
    total = total.saturating_add((8usize.saturating_sub(total % 8)) % 8);
    total = total.saturating_add(body_len);
    if total > MAX_WIRE {
        return Err(());
    }
    Ok(Some(total))
}

/// How many bytes frame `buf`'s first message: `Ok(Some(n))` when the
/// whole message is there, `Ok(None)` when it is incomplete, `Err(())`
/// when it is refused (a bad header, or a length past [`MAX_MESSAGE`]:
/// what a caller that cannot skip a message, such as the blocking
/// set-up, treats as the end).
pub fn frame_at(buf: &[u8]) -> Result<Option<usize>, ()> {
    match frame_header(buf)? {
        None => Ok(None),
        Some(total) if total > MAX_MESSAGE => Err(()),
        Some(total) if buf.len() < total => Ok(None),
        Some(total) => Ok(Some(total)),
    }
}

fn get_u32(at: &[u8], le: bool) -> u32 {
    let bytes = [at[0], at[1], at[2], at[3]];
    if le {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    }
}

/// A parsed header field, borrowing the frame.
#[derive(Debug)]
enum Header<'a> {
    Path(&'a str),
    Interface(&'a str),
    Member(&'a str),
    Error(&'a str),
    ReplySerial(u32),
    Destination(&'a str),
    Sender(&'a str),
    Signature(&'a str),
    Ignored,
}

/// The variant's value as a string (`s` or `o`).
fn str_of<'a>(sig: &str, value: &mut Reader<'a>) -> Result<&'a str, ()> {
    match sig {
        "s" | "o" => value.str(),
        _ => Err(()),
    }
}

/// The variant's value as a 32-bit integer (`i`, `u` or `b`).
fn uint_of(sig: &str, value: &mut Reader<'_>) -> Result<u32, ()> {
    match sig {
        "i" | "u" | "b" => value.u32(),
        _ => Err(()),
    }
}

/// The variant's value as a signature (`g`): syntax-checked, but not
/// validated as a complete type — the header's body signature is empty
/// while there is no body, and the caller decides what that means.
fn sig_of<'a>(sig: &str, value: &mut Reader<'a>) -> Result<&'a str, ()> {
    match sig {
        "g" => {
            let len = value.u8()? as usize;
            let bytes = value.take(len)?;
            if value.take(1)? != [0] {
                return Err(());
            }
            core::str::from_utf8(bytes).map_err(|_| ())
        }
        _ => Err(()),
    }
}

/// A parsed message, borrowing the frame. The header's strings are
/// validated (names bounded by [`MAX_NAME`], paths and signatures by
/// their own rules); the body is walked lazily through [`Reader`], so a
/// hostile body is refused where it is touched, not here.
#[derive(Debug)]
pub struct Message<'a> {
    pub kind: Kind,
    pub serial: u32,
    /// The sender's unique name, when the daemon set one.
    pub sender: Option<&'a str>,
    /// The destination: a unique or well-known name, or absent
    /// (broadcast signals). Read by future consumers routing on it;
    /// the tray answers what is its object regardless.
    #[allow(dead_code)]
    pub destination: Option<&'a str>,
    pub path: Option<&'a str>,
    pub interface: Option<&'a str>,
    pub member: Option<&'a str>,
    /// `ERROR` only.
    pub error: Option<&'a str>,
    /// `METHOD_RETURN` and `ERROR` only.
    pub reply_serial: Option<u32>,
    /// The body's signature (empty while there is no body).
    pub signature: &'a str,
    /// The body, to walk with [`Reader`].
    pub body: Reader<'a>,
}

impl<'a> Message<'a> {
    /// Parses one framed message (`frame_at` said it is whole). `Err(())`
    /// refuses it: a bad header, a name past its bound, or a body shorter
    /// than the frame claims.
    pub fn parse(frame: &'a [u8]) -> Result<Self, ()> {
        Self::parse_with(frame, true)
    }

    /// Parses the header of a message whose body is not (and will not be)
    /// buffered: `prefix` is at least the 16 fixed bytes and the fields
    /// array, and the body reads as empty. For a message past
    /// [`MAX_MESSAGE`], to learn what it answers before it is skipped.
    pub fn parse_header(prefix: &'a [u8]) -> Result<Self, ()> {
        Self::parse_with(prefix, false)
    }

    fn parse_with(frame: &'a [u8], with_body: bool) -> Result<Self, ()> {
        // Little-endian only: see the module docs.
        let le = match *frame.first().ok_or(())? {
            b'l' => true,
            _ => return Err(()),
        };
        if frame.len() < 16 || frame[3] != 1 {
            return Err(());
        }
        let kind = Kind::from_byte(frame[1]).ok_or(())?;
        let body_len = get_u32(&frame[4..8], le) as usize;
        let serial = get_u32(&frame[8..12], le);
        if serial == 0 {
            return Err(());
        }
        let fields_len = get_u32(&frame[12..16], le) as usize;
        let mut fields = Reader {
            buf: frame
                .get(16..16usize.saturating_add(fields_len))
                .ok_or(())?,
            pos: 0,
            le,
            depth: 0,
            base: 16,
        };
        let mut sender = None;
        let mut destination = None;
        let mut path = None;
        let mut interface = None;
        let mut member = None;
        let mut error = None;
        let mut reply_serial = None;
        let mut signature = "";
        while !fields.exhausted() {
            fields.align(8)?;
            fields.enter()?;
            let code = fields.u8()?;
            // `variant` positions past the value when the closure
            // returns; a loop that only borrows the value (through
            // `variant_raw`) would re-read the next element from inside
            // this one — right by alignment luck for a `u32`, wrong for
            // a string.
            let parsed = fields.variant(|sig, value| {
                Ok(match code {
                    1 => Header::Path(check_path(str_of(sig, value)?)?),
                    2 => Header::Interface(check_interface(str_of(sig, value)?)?),
                    3 => Header::Member(check_member(str_of(sig, value)?)?),
                    4 => Header::Error(check_interface(str_of(sig, value)?)?),
                    5 => Header::ReplySerial(uint_of(sig, value)?),
                    6 => Header::Destination(check_name(str_of(sig, value)?)?),
                    7 => Header::Sender(check_name(str_of(sig, value)?)?),
                    8 => Header::Signature({
                        // The body's signature: a sequence of complete
                        // types, empty while there is no body.
                        let text = sig_of(sig, value)?;
                        check_body_signature(text)?
                    }),
                    // A code this client does not know (the spec reserves
                    // the rest for later, and 9 counts file descriptors,
                    // never negotiated here): walked past by its
                    // signature, as a reader of a future spec must.
                    _ => {
                        value.skip(sig)?;
                        Header::Ignored
                    }
                })
            })?;
            fields.leave();
            match parsed {
                Header::Path(text) => path = Some(text),
                Header::Interface(text) => interface = Some(text),
                Header::Member(text) => member = Some(text),
                Header::Error(text) => error = Some(text),
                Header::ReplySerial(serial) => reply_serial = Some(serial),
                Header::Destination(text) => destination = Some(text),
                Header::Sender(text) => sender = Some(text),
                Header::Signature(text) => signature = text,
                Header::Ignored => {}
            }
        }
        let mut at = 16usize.saturating_add(fields_len);
        at = at.saturating_add((8usize.saturating_sub(at % 8)) % 8);
        let body = if with_body {
            frame.get(at..at.saturating_add(body_len)).ok_or(())?
        } else {
            &[]
        };
        Ok(Self {
            kind,
            serial,
            sender,
            destination,
            path,
            interface,
            member,
            error,
            reply_serial,
            signature,
            body: Reader {
                buf: body,
                pos: 0,
                le,
                depth: 0,
                base: at,
            },
        })
    }
}

/// A cursor over message bytes: every read is bounds-checked, aligned as
/// the type needs, and refused past the end. Containers count depth
/// against [`MAX_DEPTH`].
///
/// Alignment is absolute: `base` is the message offset `buf[0]` sits at,
/// so padding lands where the sender put it even when this reader covers
/// a sub-slice (an array's elements, a variant's value). A reader over a
/// fresh message or body starts at a multiple of 8, for which relative
/// and absolute padding agree.
#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    le: bool,
    depth: u8,
    base: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `buf` in little-endian order, at message offset 0:
    /// what the writer emits, whole messages, and what the tests feed
    /// the fuzz check.
    pub fn le(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            le: true,
            depth: 0,
            base: 0,
        }
    }

    /// Whether every byte is read.
    pub fn exhausted(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Bytes left, for the drain bound.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// The unread bytes, for handing a nested body over whole.
    pub fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ()> {
        let bytes = self
            .buf
            .get(self.pos..self.pos.saturating_add(n))
            .ok_or(())?;
        self.pos = self.pos.saturating_add(n);
        Ok(bytes)
    }

    fn align(&mut self, n: usize) -> Result<(), ()> {
        let at = self.base.saturating_add(self.pos);
        let pad = (n.saturating_sub(at % n)) % n;
        if self.pos.saturating_add(pad) > self.buf.len() {
            return Err(());
        }
        self.pos += pad;
        Ok(())
    }

    fn enter(&mut self) -> Result<(), ()> {
        if self.depth as usize >= MAX_DEPTH {
            return Err(());
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn int(&mut self, n: usize) -> Result<u64, ()> {
        self.align(n)?;
        let bytes = self.take(n)?;
        let mut full = [0u8; 8];
        if self.le {
            full[..n].copy_from_slice(bytes);
            Ok(u64::from_le_bytes(full))
        } else {
            full[8 - n..].copy_from_slice(bytes);
            Ok(u64::from_be_bytes(full))
        }
    }

    pub fn u8(&mut self) -> Result<u8, ()> {
        Ok(self.int(1)? as u8)
    }

    pub fn u16(&mut self) -> Result<u16, ()> {
        Ok(self.int(2)? as u16)
    }

    pub fn u32(&mut self) -> Result<u32, ()> {
        Ok(self.int(4)? as u32)
    }

    pub fn u64(&mut self) -> Result<u64, ()> {
        self.int(8)
    }

    pub fn i16(&mut self) -> Result<i16, ()> {
        Ok(self.int(2)? as u16 as i16)
    }

    pub fn i32(&mut self) -> Result<i32, ()> {
        Ok(self.int(4)? as u32 as i32)
    }

    pub fn i64(&mut self) -> Result<i64, ()> {
        Ok(self.int(8)? as i64)
    }

    pub fn f64(&mut self) -> Result<f64, ()> {
        Ok(f64::from_bits(self.int(8)?))
    }

    /// A boolean: 0 is false, 1 is true, anything else is refused (a
    /// stricter reader than the daemon, which tolerates).
    pub fn boolean(&mut self) -> Result<bool, ()> {
        match self.int(4)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(()),
        }
    }

    fn string(&mut self) -> Result<&'a str, ()> {
        self.align(4)?;
        let len = self.u32_raw()? as usize;
        let bytes = self.take(len)?;
        if self.take(1)? != [0] {
            return Err(());
        }
        core::str::from_utf8(bytes).map_err(|_| ())
    }

    fn u32_raw(&mut self) -> Result<u32, ()> {
        let bytes = self.take(4)?;
        let word = [bytes[0], bytes[1], bytes[2], bytes[3]];
        Ok(if self.le {
            u32::from_le_bytes(word)
        } else {
            u32::from_be_bytes(word)
        })
    }

    /// A string (`s`) or object path (`o`): borrowed, NUL-checked, valid
    /// UTF-8. Unbounded here; the consumer bounds it at use.
    pub fn str(&mut self) -> Result<&'a str, ()> {
        self.string()
    }

    /// A signature (`g`): one byte of length, borrowed, well-formed.
    pub fn signature(&mut self) -> Result<&'a str, ()> {
        let len = self.u8()? as usize;
        let bytes = self.take(len)?;
        if self.take(1)? != [0] {
            return Err(());
        }
        let sig = core::str::from_utf8(bytes).map_err(|_| ())?;
        check_signature(sig)?;
        Ok(sig)
    }

    /// An array's raw elements (`a...`): the length-delimited bytes, to
    /// walk with a sub-reader at the element's alignment. The length is
    /// refused past what is left after the element padding (the padding
    /// is on the wire but not in the length). Padding is absolute (this
    /// reader's `base` plus its position), so it is right inside a
    /// variant's value too; a fresh [`Reader::le`] over a sub-slice
    /// restarts at offset zero, which is right only for a slice that
    /// started 8-aligned in the message (array elements of 8-aligned
    /// types do).
    pub fn array_raw(&mut self, element_align: usize) -> Result<&'a [u8], ()> {
        self.align(4)?;
        let len = self.u32_raw()? as usize;
        if element_align > 1 {
            // Absolute, as `align` is: inside a variant's value this
            // reader's `base` is not a multiple of 8, and the sender
            // padded to the message, not to the variant.
            let at = self.base.saturating_add(self.pos);
            let pad = (element_align.saturating_sub(at % element_align)) % element_align;
            if pad > self.remaining() {
                return Err(());
            }
            self.pos += pad;
        }
        if len > self.remaining() {
            return Err(());
        }
        self.take(len)
    }

    /// An array's elements as a sub-reader: [`Reader::array_raw`], then
    /// positioned at the elements with their message offset, so its own
    /// alignment lands absolutely. The way arrays are walked everywhere.
    pub fn elements(&mut self, element_align: usize) -> Result<Reader<'a>, ()> {
        let raw = self.array_raw(element_align)?;
        Ok(Reader {
            buf: raw,
            pos: 0,
            le: self.le,
            depth: self.depth,
            // Where the elements start in the message: past what this
            // reader consumed for them.
            base: self.base + self.pos.saturating_sub(raw.len()),
        })
    }

    /// Enters a struct or dict entry: 8-aligned, depth-counted. Pair with
    /// [`Reader::leave_struct`].
    pub fn enter_struct(&mut self) -> Result<(), ()> {
        self.align(8)?;
        self.enter()
    }

    pub fn leave_struct(&mut self) {
        self.leave();
    }

    /// A variant's raw value (`v`): its signature and the value's bytes
    /// from here to the end of this reader's view. Variants are last in
    /// their struct (or alone in a body), so the rest belongs to the
    /// value; [`Reader::variant`] positions past it.
    pub fn variant_raw(&mut self) -> Result<Variant<'a>, ()> {
        let sig = self.signature()?;
        Ok(Variant {
            le: self.le,
            depth: self.depth,
            sig,
            raw: &self.buf[self.pos.min(self.buf.len())..],
            base: self.base.saturating_add(self.pos.min(self.buf.len())),
        })
    }

    /// Reads the variant's value with `read`, then positions past it. The
    /// value's length comes from what `read` consumes, so `read` must
    /// consume exactly the value (typed getters and [`Reader::skip`]
    /// both do).
    pub fn variant<T>(
        &mut self,
        read: impl FnOnce(&str, &mut Reader<'a>) -> Result<T, ()>,
    ) -> Result<T, ()> {
        let value = self.variant_raw()?;
        let mut scoped = value.read();
        let out = read(value.sig, &mut scoped)?;
        self.pos = self.pos.saturating_add(scoped.pos);
        Ok(out)
    }

    /// Skips one complete value of signature `sig`, refusing a malformed
    /// one. What the tray walks past unknown properties with, and what
    /// the fuzz check walks whole bodies with.
    pub fn skip(&mut self, sig: &str) -> Result<(), ()> {
        let bytes = sig.as_bytes();
        check_body_signature(sig)?;
        let (at, _) = skip_value(self, bytes, 0, self.depth as usize)?;
        let _ = at;
        Ok(())
    }
}

/// Skips one complete value of the type at `bytes[at]`, returning the
/// signature offset past it. Dict entries and structs need their field
/// signatures from the enclosing signature, which is why the walk is over
/// signature bytes rather than wire bytes alone.
fn skip_value<'a>(
    reader: &mut Reader<'a>,
    bytes: &[u8],
    at: usize,
    depth: usize,
) -> Result<(usize, ()), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    match *bytes.get(at).ok_or(())? {
        b'y' => {
            reader.u8()?;
            Ok((at + 1, ()))
        }
        b'b' => {
            reader.boolean()?;
            Ok((at + 1, ()))
        }
        b'n' => {
            reader.i16()?;
            Ok((at + 1, ()))
        }
        #[allow(clippy::match_same_arms)]
        b'q' => {
            reader.u16()?;
            Ok((at + 1, ()))
        }
        b'i' => {
            reader.i32()?;
            Ok((at + 1, ()))
        }
        b'u' => {
            reader.u32()?;
            Ok((at + 1, ()))
        }
        b'x' => {
            reader.i64()?;
            Ok((at + 1, ()))
        }
        b't' => {
            reader.u64()?;
            Ok((at + 1, ()))
        }
        b'd' => {
            reader.f64()?;
            Ok((at + 1, ()))
        }
        b'h' => {
            reader.u32()?;
            Ok((at + 1, ()))
        }
        b's' | b'o' => {
            reader.string()?;
            Ok((at + 1, ()))
        }
        b'g' => {
            reader.signature()?;
            Ok((at + 1, ()))
        }
        b'v' => {
            reader.variant(|sig, scoped| {
                let inner = sig.as_bytes();
                check_signature(sig)?;
                let (end, _) = skip_value(scoped, inner, 0, depth + 1)?;
                if end != inner.len() {
                    return Err(());
                }
                Ok(())
            })?;
            Ok((at + 1, ()))
        }
        b'a' => {
            let (end, _) = complete(bytes, at + 1, depth + 1)?;
            let align = element_alignment(bytes, at + 1)?;
            let mut scoped = reader.elements(align)?;
            // Elements run to the array's end: each is one complete
            // value of the element signature.
            while !scoped.exhausted() {
                skip_value(&mut scoped, bytes, at + 1, depth + 1)?;
            }
            Ok((end, ()))
        }
        b'(' | b'{' => {
            let closing = if bytes[at] == b'(' { b')' } else { b'}' };
            reader.enter_struct()?;
            let mut at = at + 1;
            loop {
                if *bytes.get(at).ok_or(())? == closing {
                    reader.leave_struct();
                    return Ok((at + 1, ()));
                }
                let (next, _) = skip_value(reader, bytes, at, depth + 1)?;
                at = next;
            }
        }
        _ => Err(()),
    }
}

/// The alignment one complete type at `bytes[at]` needs.
fn element_alignment(bytes: &[u8], at: usize) -> Result<usize, ()> {
    match *bytes.get(at).ok_or(())? {
        b'y' | b'v' | b'g' => Ok(1),
        b'n' | b'q' => Ok(2),
        b'b' | b'i' | b'u' | b's' | b'o' | b'a' | b'h' => Ok(4),
        b'x' | b't' | b'd' | b'(' | b'{' => Ok(8),
        _ => Err(()),
    }
}

/// A variant's value, borrowed: its signature, its raw bytes, and the
/// message offset the bytes sit at.
#[derive(Debug, Clone, Copy)]
pub struct Variant<'a> {
    le: bool,
    depth: u8,
    sig: &'a str,
    raw: &'a [u8],
    base: usize,
}

impl<'a> Variant<'a> {
    pub fn signature(&self) -> &'a str {
        self.sig
    }

    /// The value's bytes from here on, to walk with a reader at the
    /// caller's depth and offset.
    pub fn read(&self) -> Reader<'a> {
        Reader {
            buf: self.raw,
            pos: 0,
            le: self.le,
            depth: self.depth,
            base: self.base,
        }
    }
}

/// Whether `name` is a usable bus, interface, member or error name:
/// 1 to [`MAX_NAME`] bytes, a unique name (`:1.42`) or dotted
/// well-known one of at least two valid elements. Bus-name elements may
/// hold dashes (`org.kde.StatusNotifierItem-100-1`); interface and
/// member names never do (see [`check_member`]). Refuses anything else,
/// including the empty string.
pub fn check_name(name: &str) -> Result<&str, ()> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(());
    }
    if let Some(rest) = name.strip_prefix(':') {
        if rest.is_empty()
            || !rest
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_')
        {
            return Err(());
        }
        return Ok(name);
    }
    let mut elements = 0;
    for element in name.split('.') {
        elements += 1;
        let mut chars = element.bytes();
        match chars.next() {
            // A `-` may begin an element of a bus name (the spec allows
            // it, and a daemon takes `org.example.-x`); a digit may not.
            Some(first) if first.is_ascii_alphabetic() || first == b'_' || first == b'-' => {}
            _ => return Err(()),
        }
        if !chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-') {
            return Err(());
        }
    }
    if elements < 2 {
        return Err(());
    }
    Ok(name)
}

/// Whether `name` is a usable interface or error name: dotted, at least
/// two elements, each of `[A-Za-z0-9_]` and not starting with a digit
/// (no dashes, unlike a bus name), within [`MAX_NAME`].
pub fn check_interface(name: &str) -> Result<&str, ()> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(());
    }
    let mut elements = 0;
    for element in name.split('.') {
        elements += 1;
        let mut chars = element.bytes();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
            _ => return Err(()),
        }
        if !chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
            return Err(());
        }
    }
    if elements < 2 {
        return Err(());
    }
    Ok(name)
}

/// A member or signal name: like [`check_name`] but with no dots.
pub fn check_member(name: &str) -> Result<&str, ()> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(());
    }
    let mut chars = name.bytes();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return Err(()),
    }
    if !chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err(());
    }
    Ok(name)
}

/// Whether `path` is a usable object path: starting with `/`, no empty
/// elements, no trailing slash but the root, of the path character set.
pub fn check_path(path: &str) -> Result<&str, ()> {
    if !path.starts_with('/') {
        return Err(());
    }
    if path.len() > MAX_PATH {
        return Err(());
    }
    if path == "/" {
        return Ok(path);
    }
    for element in path.split('/').skip(1) {
        if element.is_empty() {
            return Err(());
        }
        if !element
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(());
        }
    }
    Ok(path)
}

/// Whether `sig` is a well-formed single complete type within
/// [`MAX_SIGNATURE`] bytes and [`MAX_DEPTH`] nesting: dict entries only
/// as `{key value}` with a basic-type key, structs nonempty, arrays with
/// an element.
pub fn check_signature(sig: &str) -> Result<&str, ()> {
    if sig.is_empty() || sig.len() > MAX_SIGNATURE {
        return Err(());
    }
    let bytes = sig.as_bytes();
    let (at, _) = complete(bytes, 0, 0)?;
    if at != bytes.len() {
        return Err(());
    }
    Ok(sig)
}

/// Whether `sig` is a well-formed message-body signature: a sequence of
/// complete types (empty while there is no body), each within
/// [`MAX_SIGNATURE`] bytes total and [`MAX_DEPTH`] nesting. A body's
/// signature concatenates its values' (`su` for a string and a word),
/// where a variant's holds exactly one.
pub fn check_body_signature(sig: &str) -> Result<&str, ()> {
    if sig.len() > MAX_SIGNATURE {
        return Err(());
    }
    let bytes = sig.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let (next, _) = complete(bytes, at, 0)?;
        if next == at {
            return Err(());
        }
        at = next;
    }
    Ok(sig)
}

/// Parses one complete type at `at`, returning the end and whether it is
/// a basic type (a dict key must be one).
fn complete(bytes: &[u8], at: usize, depth: usize) -> Result<(usize, bool), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    let byte = *bytes.get(at).ok_or(())?;
    match byte {
        b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b'h' | b's' | b'o'
        | b'g' => Ok((at + 1, true)),
        b'v' => Ok((at + 1, false)),
        b'a' => {
            let (end, _) = complete(bytes, at + 1, depth + 1)?;
            Ok((end, false))
        }
        b'(' => {
            let mut at = at + 1;
            if bytes.get(at) == Some(&b')') {
                return Err(());
            }
            loop {
                at = complete(bytes, at, depth + 1)?.0;
                match bytes.get(at) {
                    Some(b')') => return Ok((at + 1, false)),
                    Some(_) => {}
                    None => return Err(()),
                }
            }
        }
        b'{' => {
            let (at, key) = complete(bytes, at + 1, depth + 1)?;
            if !key {
                return Err(());
            }
            let (at, _) = complete(bytes, at, depth + 1)?;
            if bytes.get(at) != Some(&b'}') {
                return Err(());
            }
            Ok((at + 1, false))
        }
        _ => Err(()),
    }
}

/// The most `ListNames` entries read: a session bus holds dozens; past
/// this the rest are left out (each is at most 128 bytes, so the bound
/// is memory, not time). The cut is made before a consumer filters the
/// names (the media module's MPRIS prefix), so a bus holding more than this
/// many names can hide a later player from the start-up listing; its
/// `NameOwnerChanged` is still heard.
pub const MAX_LIST_NAMES: usize = 4096;

/// One pixmap entry, borrowed: dimensions and the `ARGB32` bytes in
/// network order (see the spec's icon-pixmap page).
#[derive(Debug, Clone, Copy)]
pub struct Pixmap<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
}

/// The longest pixmap side taken, in pixels: icons at requested device
/// pixels are tens of pixels; past this an entry is skipped, never
/// scaled (a 512-pixel side is a megabyte of `ARGB32`).
pub const MAX_PIXMAP_SIDE: u32 = 256;

/// The most pixmap entries walked in one property: more is not an icon
/// set but a flood.
pub const MAX_PIXMAPS: usize = 64;

/// An `a(iiay)` pixmap list's elements (what [`Reader::array_raw`]
/// returns, never the length word itself: a fresh reader over the whole
/// array would align relative to the word, not the message). Every
/// entry's dimensions against its bytes; an oversized or misshapen entry
/// skips itself, never the whole list; past [`MAX_PIXMAPS`] entries the
/// walk is refused.
pub fn read_pixmaps(elements: &[u8]) -> Result<Vec<Pixmap<'_>>, ()> {
    let mut scoped = Reader::le(elements);
    let mut pixmaps = Vec::new();
    while !scoped.exhausted() {
        scoped.enter_struct()?;
        let width = scoped.u32()?;
        let height = scoped.u32()?;
        let pixels = scoped.array_raw(1)?;
        scoped.leave_struct();
        let well_shaped = width >= 1
            && height >= 1
            && width <= MAX_PIXMAP_SIDE
            && height <= MAX_PIXMAP_SIDE
            && pixels.len() == width as usize * height as usize * 4;
        if well_shaped {
            if pixmaps.len() >= MAX_PIXMAPS {
                return Err(());
            }
            pixmaps.push(Pixmap {
                width,
                height,
                pixels,
            });
        }
    }
    if !scoped.exhausted() {
        return Err(());
    }
    Ok(pixmaps)
}

/// Reads the `as` reply of `ListNames` into names. `Err(())` refuses the
/// shape; the caller drops the reply, never the connection.
pub fn read_names(signature: &str, body: &[u8]) -> Result<Vec<String>, ()> {
    if signature != "as" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let raw = reader.array_raw(4)?;
    let mut scoped = Reader::le(raw);
    let mut names = Vec::new();
    while !scoped.exhausted() {
        let name = scoped.str()?;
        // One name this client does not take (a daemon accepts what is
        // valid to it, and a peer can own such a name) is skipped, not
        // the whole list: start-up enumeration must not be blinded by it.
        if check_name(name).is_err() {
            continue;
        }
        if names.len() >= MAX_LIST_NAMES {
            break;
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

/// Reads a `GetNameOwner` reply into the unique name.
pub fn read_owner(signature: &str, body: &[u8]) -> Result<String, ()> {
    if signature != "s" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let owner = reader.str()?;
    check_name(owner)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(owner.to_owned())
}

/// Reads a `RequestName` reply word.
pub fn read_request_reply(signature: &str, body: &[u8]) -> Result<u32, ()> {
    if signature != "u" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let word = reader.u32().map_err(|_| ())?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(word)
}

/// Reads a `NameOwnerChanged(name, old, new)` signal body. Empty owners
/// are `None` (appeared or vanished).
pub fn read_name_owner_changed(
    body: &[u8],
) -> Result<(String, Option<String>, Option<String>), ()> {
    let mut reader = Reader::le(body);
    let name = reader.str()?;
    let old = reader.str()?;
    let new = reader.str()?;
    if !reader.exhausted() {
        return Err(());
    }
    check_name(name)?;
    let owned = |text: &str| {
        if text.is_empty() {
            Ok(None)
        } else {
            check_name(text)?;
            Ok(Some(text.to_owned()))
        }
    };
    Ok((name.to_owned(), owned(old)?, owned(new)?))
}

/// The most properties walked in one `GetAll` answer: a real item has a
/// dozen; past this the answer is refused (it is bytes either way, but a
/// dictionary of a hundred thousand one-byte keys is not an item).
pub const MAX_PROPERTIES: usize = 128;

/// What the bar takes from an item's `GetAll` answer, borrowing the body.
/// Each field is `None` when the item did not send it (or sent it with a
/// type the spec does not give it, which is skipped like any unknown
/// property).
#[derive(Debug, Default)]
pub struct ItemProps<'a> {
    pub status: Option<&'a str>,
    pub title: Option<&'a str>,
    pub menu: Option<&'a str>,
    pub item_is_menu: Option<bool>,
    /// The tooltip's title and text (its icon name and pixmaps are
    /// walked for shape and dropped: no tooltip icon is drawn yet).
    pub tooltip: Option<(&'a str, &'a str)>,
    pub pixmaps: Option<Vec<Pixmap<'a>>>,
    /// The themed icon names: drawn through the icon-theme lookup (see
    /// the tray's `theme` module), when no pixmap was sent. `IconThemePath`
    /// is one extra base directory searched first.
    pub icon_name: Option<&'a str>,
    pub icon_theme_path: Option<&'a str>,
    /// The attention icon's name: drawn instead of the main one while the
    /// status is `NeedsAttention`, for items without a pixmap (an item
    /// that sent a pixmap keeps it while alarmed). The overlay icon's name is walked for
    /// shape and dropped (compositing it would be a second scaled draw
    /// per frame for a badge real items rarely send).
    pub attention_icon_name: Option<&'a str>,
    pub overlay_icon_name: Option<&'a str>,
}

/// Reads an item's `GetAll` body (`a{sv}`): the properties the bar uses
/// typed, every other one skipped by its signature, so a property a
/// future spec adds cannot break the walk. `Err(())` refuses the whole
/// answer (a misshapen entry, a pixmap list past its bound, trailing
/// bytes): the caller keeps the item's last state. This is the one walk,
/// shared by the tray and the fuzz target.
pub fn read_item_props(body: &[u8]) -> Result<ItemProps<'_>, ()> {
    let mut reader = Reader::le(body);
    let mut entries = reader.elements(8)?;
    let mut props = ItemProps::default();
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("Status", "s") => props.status = Some(entries.str()?),
            ("Title", "s") => props.title = Some(entries.str()?),
            ("Menu", "o") => props.menu = Some(entries.str()?),
            ("ItemIsMenu", "b") => props.item_is_menu = Some(entries.boolean()?),
            ("ToolTip", "(sa(iiay)ss)") => {
                entries.enter_struct()?;
                entries.str()?;
                read_pixmaps(entries.array_raw(8)?)?;
                let title = entries.str()?;
                let text = entries.str()?;
                entries.leave_struct();
                props.tooltip = Some((title, text));
            }
            ("IconPixmap", "a(iiay)") => {
                props.pixmaps = Some(read_pixmaps(entries.array_raw(8)?)?);
            }
            ("IconName", "s") => props.icon_name = Some(entries.str()?),
            ("IconThemePath", "s") => props.icon_theme_path = Some(entries.str()?),
            ("AttentionIconName", "s") => props.attention_icon_name = Some(entries.str()?),
            ("OverlayIconName", "s") => props.overlay_icon_name = Some(entries.str()?),
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// Reads a single `s` body (a register signal's service argument, a
/// `NewStatus` argument).
pub fn read_string(signature: &str, body: &[u8]) -> Result<String, ()> {
    if signature != "s" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let text = reader.str()?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(text.to_owned())
}

/// How deep a `GetLayout` answer nests/addressing goes: the request asks
/// for this, and an answer nesting past it is refused (a hostile item may
/// answer deeper than asked).
pub const MAX_MENU_DEPTH: usize = 8;

/// How many items one `GetLayout` answer holds, nesting included: a real
/// menu has dozens; past this the answer is refused, never half-read.
pub const MAX_MENU_ITEMS: usize = 64;

/// What kind of toggle a menu item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuToggle {
    None,
    Check,
    Radio,
}

/// One DBusMenu item, borrowed: what the bar draws of a `GetLayout`
/// answer. `enabled` and `visible` default to true (an item that omits
/// them is shown); `toggle_state` is 0 or 1 (`-1`, indeterminate, reads
/// as off); `submenu` says the item asked for submenu display
/// (`children-display`), whether or not children arrived (a lazy item
/// sends none until it is asked).
#[derive(Debug, Clone)]
pub struct MenuItem<'a> {
    pub id: i32,
    pub label: Option<&'a str>,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    pub toggle: MenuToggle,
    pub toggle_state: i32,
    pub submenu: bool,
    pub children: Vec<MenuItem<'a>>,
}

/// Reads a `GetLayout` reply (`u(ia{sv}av)`): the revision and the
/// root's children. The out-args ride bare on the wire, not wrapped in
/// a struct (a real Qt reply arrives as `u(ia{sv}av)`); typed like
/// [`read_item_props`]: a known property with
/// a wrong type is skipped (the default holds), every other one is
/// skipped by its signature, and anything misshapen refuses the whole
/// answer — the caller keeps the menu's last state. This is the one
/// walk, shared by the tray and the fuzz target.
pub fn read_menu_layout<'a>(
    signature: &str,
    body: &'a [u8],
) -> Result<(u32, Vec<MenuItem<'a>>), ()> {
    if signature != "u(ia{sv}av)" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let revision = reader.u32()?;
    let mut roots = Vec::new();
    parse_menu_node(&mut reader, 0, &mut 0, &mut roots)?;
    if !reader.exhausted() {
        return Err(());
    }
    // The root's own row is the menu, not an item: its children are.
    let root = roots.pop().ok_or(())?;
    if !roots.is_empty() {
        return Err(());
    }
    Ok((revision, root.children))
}

/// Parses one `(ia{sv}av)` node, pushing it onto `out`. `depth` is the
/// node's own (the root is 0); `seen` counts every node against
/// [`MAX_MENU_ITEMS`], the root included.
fn parse_menu_node<'a>(
    reader: &mut Reader<'a>,
    depth: usize,
    seen: &mut usize,
    out: &mut Vec<MenuItem<'a>>,
) -> Result<(), ()> {
    if depth > MAX_MENU_DEPTH {
        return Err(());
    }
    *seen = seen.saturating_add(1);
    if *seen > MAX_MENU_ITEMS {
        return Err(());
    }
    reader.enter_struct()?;
    let id = reader.i32()?;
    let mut entries = reader.elements(8)?;
    let mut item = MenuItem {
        id,
        label: None,
        enabled: true,
        visible: true,
        separator: false,
        toggle: MenuToggle::None,
        toggle_state: 0,
        submenu: false,
        children: Vec::new(),
    };
    while !entries.exhausted() {
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("label", "s") => item.label = Some(entries.str()?),
            ("enabled", "b") => item.enabled = entries.boolean()?,
            ("visible", "b") => item.visible = entries.boolean()?,
            ("type", "s") => item.separator = entries.str()? == "separator",
            ("toggle-type", "s") => {
                item.toggle = match entries.str()? {
                    "checkmark" => MenuToggle::Check,
                    "radio" => MenuToggle::Radio,
                    _ => MenuToggle::None,
                };
            }
            ("toggle-state", "i") => item.toggle_state = entries.i32()?,
            ("children-display", "s") => item.submenu = entries.str()? == "submenu",
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    // The children: an array of variants, each one more node. Read
    // through the array's own reader (message offsets, not slice ones),
    // so a struct value aligns as the sender padded it.
    let mut kids = reader.elements(1)?;
    while !kids.exhausted() {
        let child = kids.variant(|sig, reader| {
            if sig != "(ia{sv}av)" {
                return Err(());
            }
            let mut one = Vec::new();
            parse_menu_node(reader, depth + 1, seen, &mut one)?;
            if one.len() != 1 {
                return Err(());
            }
            one.pop().ok_or(())
        })?;
        item.children.push(child);
    }
    reader.leave_struct();
    out.push(item);
    Ok(())
}

/// Reads a `LayoutUpdated` signal body: the revision, with or without the
/// parent both implementations send. Anything else is not an update.
pub fn read_layout_updated(signature: &str, body: &[u8]) -> Result<u32, ()> {
    if signature != "u" && signature != "ui" {
        return Err(());
    }
    let mut reader = Reader::le(body);
    let revision = reader.u32()?;
    if signature == "ui" {
        let _ = reader.i32()?;
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(revision)
}

/// A message writer: little-endian, capped at [`MAX_MESSAGE`]. Anything
/// past a cap sets the overflow, and [`Writer::finish`] returns `None` —
/// a message that does not fit is never half-sent.
#[derive(Debug)]
pub struct Writer {
    buf: Vec<u8>,
    cap: usize,
    overflow: bool,
    depth: usize,
    body_at: Option<usize>,
}

/// A header field: the code and the value. The writer knows the
/// variant's signature from the value.
pub enum Field<'a> {
    Path(&'a str),
    Interface(&'a str),
    Member(&'a str),
    Error(&'a str),
    ReplySerial(u32),
    Destination(&'a str),
    Signature(&'a str),
}

/// Where an open array's length word and first element are, for
/// [`Writer::close_array`] to patch the one from the other.
#[derive(Debug, Clone, Copy)]
pub struct ArrayCookie {
    len_at: usize,
    data_at: usize,
}

impl Default for Writer {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            cap: MAX_MESSAGE,
            overflow: false,
            depth: 0,
            body_at: None,
        }
    }
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// A writer capped at `cap` bytes instead of [`MAX_MESSAGE`]: the
    /// tests' way to make a valid message this client would skip.
    #[cfg(test)]
    pub fn with_cap(cap: usize) -> Self {
        Self {
            cap,
            ..Self::default()
        }
    }

    /// Whether anything overflowed so far.
    #[allow(dead_code)]
    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    fn reserve(&mut self, n: usize) {
        if self.overflow {
            return;
        }
        if self.buf.len().saturating_add(n) > self.cap {
            self.overflow = true;
            return;
        }
        self.buf.reserve(n);
    }

    /// Aligns the cursor past the padding for `n`. Crate-visible: the
    /// tests' fake bus builds headers by hand.
    pub(crate) fn pad(&mut self, n: usize) {
        if n <= 1 || self.overflow {
            return;
        }
        let pad = (n.saturating_sub(self.buf.len() % n)) % n;
        self.reserve(pad);
        if !self.overflow {
            self.buf.extend(core::iter::repeat_n(0, pad));
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.overflow {
            return;
        }
        self.reserve(bytes.len());
        if !self.overflow {
            self.buf.extend_from_slice(bytes);
        }
    }

    /// The full basic-type surface the spike prescribes: used by tests
    /// and future consumers, exercised here by the fuzz check.
    #[allow(dead_code)]
    pub fn u8(&mut self, value: u8) {
        self.push(&[value]);
    }

    #[allow(dead_code)]
    pub fn u16(&mut self, value: u16) {
        self.pad(2);
        self.push(&value.to_le_bytes());
    }

    #[allow(dead_code)]
    pub fn i16(&mut self, value: i16) {
        self.pad(2);
        self.push(&value.to_le_bytes());
    }

    pub fn u32(&mut self, value: u32) {
        self.pad(4);
        self.push(&value.to_le_bytes());
    }

    pub fn i32(&mut self, value: i32) {
        self.pad(4);
        self.push(&value.to_le_bytes());
    }

    #[allow(dead_code)]
    pub fn u64(&mut self, value: u64) {
        self.pad(8);
        self.push(&value.to_le_bytes());
    }

    pub fn boolean(&mut self, value: bool) {
        self.u32(u32::from(value));
    }

    /// Appends `bytes` verbatim (already-marshalled values, the tests'
    /// fake bus answers from these). Counted against the cap like
    /// everything else.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.push(bytes);
    }

    /// Takes the bytes written so far, for a body built without a header
    /// (a call's body, a reply's `as` list). `None` on any overflow.
    pub fn take_body(&mut self) -> Option<Vec<u8>> {
        if self.overflow {
            return None;
        }
        self.depth = 0;
        Some(core::mem::take(&mut self.buf))
    }

    /// A string (`s`) or object path (`o`): the length, the bytes, the
    /// NUL. Overlong single strings overflow rather than truncate (the
    /// caller bounds what it shows).
    pub fn str(&mut self, value: &str) {
        let bytes = value.as_bytes();
        self.pad(4);
        self.reserve(4usize.saturating_add(bytes.len()).saturating_add(1));
        if self.overflow {
            return;
        }
        self.buf
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(bytes);
        self.buf.push(0);
    }

    /// A signature (`g`): one byte of length, the bytes, the NUL.
    pub fn signature(&mut self, sig: &str) {
        let bytes = sig.as_bytes();
        if bytes.len() > MAX_SIGNATURE {
            self.overflow = true;
            return;
        }
        self.reserve(bytes.len() + 2);
        if self.overflow {
            return;
        }
        self.buf.push(bytes.len() as u8);
        self.buf.extend_from_slice(bytes);
        self.buf.push(0);
    }

    /// Opens an array: the 4-aligned length word, then aligned for
    /// `element_align`, with the elements after. Returns the cookie
    /// [`Writer::close_array`] takes. The length counts the elements
    /// only — never the padding between the length word and the first
    /// element, which is on the wire but not in the length.
    pub fn open_array(&mut self, element_align: usize) -> Option<ArrayCookie> {
        if self.depth >= MAX_DEPTH {
            self.overflow = true;
            return None;
        }
        self.pad(4);
        if self.overflow {
            return None;
        }
        let len_at = self.buf.len();
        self.reserve(4);
        if self.overflow {
            return None;
        }
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        if element_align > 1 {
            self.pad(element_align);
        }
        if self.overflow {
            return None;
        }
        self.depth += 1;
        Some(ArrayCookie {
            len_at,
            data_at: self.buf.len(),
        })
    }

    /// Closes the array `open_array` opened: the length word takes the
    /// elements' bytes since.
    pub fn close_array(&mut self, cookie: ArrayCookie) {
        self.depth = self.depth.saturating_sub(1);
        if self.overflow {
            return;
        }
        let ArrayCookie { len_at, data_at } = cookie;
        let len = self.buf.len().saturating_sub(data_at) as u32;
        if let Some(slot) = self.buf.get_mut(len_at..len_at + 4) {
            slot.copy_from_slice(&len.to_le_bytes());
        } else {
            self.overflow = true;
        }
    }

    /// Opens a struct or dict entry: 8-aligned, no length word. Pair with
    /// [`Writer::close_struct`].
    pub fn open_struct(&mut self) -> bool {
        if self.depth >= MAX_DEPTH {
            self.overflow = true;
            return false;
        }
        self.pad(8);
        if self.overflow {
            return false;
        }
        self.depth += 1;
        true
    }

    pub fn close_struct(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// A variant: the signature, then the value the caller writes next.
    pub fn variant(&mut self, sig: &str) {
        if check_signature(sig).is_err() {
            self.overflow = true;
            return;
        }
        self.signature(sig);
    }

    /// Starts a method call header; the caller writes the body next and
    /// ends with [`Writer::finish`]. `body_sig` is the body's whole
    /// signature (empty while there is no body). Eight arguments: a call
    /// names its serial, destination, object, interface, member, body
    /// and reply wish, like the header it becomes.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_call(
        &mut self,
        serial: u32,
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
        flags: u8,
    ) {
        self.begin(
            1,
            serial,
            flags,
            &[
                Field::Path(path),
                Field::Interface(interface),
                Field::Member(member),
                Field::Destination(destination),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts a method return for `reply_to`.
    pub fn begin_return(&mut self, serial: u32, reply_to: u32, body_sig: &str) {
        self.begin(
            2,
            serial,
            0,
            &[Field::ReplySerial(reply_to), Field::Signature(body_sig)],
        );
    }

    /// Starts a method return with an explicit destination (probe).
    pub fn begin_return_to(&mut self, serial: u32, dest: &str, reply_to: u32, body_sig: &str) {
        self.begin(
            2,
            serial,
            0,
            &[
                Field::ReplySerial(reply_to),
                Field::Destination(dest),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts an error reply for `reply_to` with `error`'s name and an
    /// explicit destination.
    pub fn begin_error_to(
        &mut self,
        serial: u32,
        dest: &str,
        reply_to: u32,
        error: &str,
        body_sig: &str,
    ) {
        self.begin(
            3,
            serial,
            0,
            &[
                Field::Error(error),
                Field::ReplySerial(reply_to),
                Field::Destination(dest),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts an error reply for `reply_to` with `error`'s name.
    pub fn begin_error(&mut self, serial: u32, reply_to: u32, error: &str, body_sig: &str) {
        self.begin(
            3,
            serial,
            0,
            &[
                Field::Error(error),
                Field::ReplySerial(reply_to),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts a signal addressed to `dest`: what a peer sends to forge
    /// one at a single listener (tests).
    #[cfg(test)]
    pub fn begin_signal_to(
        &mut self,
        serial: u32,
        dest: &str,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
    ) {
        self.begin(
            4,
            serial,
            0,
            &[
                Field::Path(path),
                Field::Interface(interface),
                Field::Member(member),
                Field::Destination(dest),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts a broadcast signal.
    pub fn begin_signal(
        &mut self,
        serial: u32,
        path: &str,
        interface: &str,
        member: &str,
        body_sig: &str,
    ) {
        self.begin(
            4,
            serial,
            0,
            &[
                Field::Path(path),
                Field::Interface(interface),
                Field::Member(member),
                Field::Signature(body_sig),
            ],
        );
    }

    /// Starts a message header of `kind` with `fields`; the caller writes
    /// the body next and ends with [`Writer::finish`].
    fn begin(&mut self, kind: u8, serial: u32, flags: u8, fields: &[Field<'_>]) {
        debug_assert!(self.body_at.is_none());
        self.reserve(16);
        if self.overflow {
            return;
        }
        self.buf.extend_from_slice(&[b'l', kind, flags, 1]);
        // The body length: patched at finish.
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.buf.extend_from_slice(&serial.to_le_bytes());
        // The fields array: the length word, then 8-aligned elements,
        // written whole with the length patched after, so no second
        // pass over the header is needed. The length counts the
        // elements only, never the padding before the first one.
        let array_at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.pad(8);
        if self.overflow {
            return;
        }
        let data_at = self.buf.len();
        for field in fields {
            // Each element is a `{BYTE, VARIANT}`: 8-aligned, then the
            // code byte and the variant's signature and value.
            self.pad(8);
            if self.overflow {
                return;
            }
            let code = match field {
                Field::Path(_) => 1,
                Field::Interface(_) => 2,
                Field::Member(_) => 3,
                Field::Error(_) => 4,
                Field::ReplySerial(_) => 5,
                Field::Destination(_) => 6,
                Field::Signature(_) => 8,
            };
            self.push(&[code]);
            match field {
                Field::Path(text) => {
                    self.signature("o");
                    self.str(text);
                }
                Field::Interface(text)
                | Field::Member(text)
                | Field::Error(text)
                | Field::Destination(text) => {
                    self.signature("s");
                    self.str(text);
                }
                Field::ReplySerial(value) => {
                    self.signature("u");
                    self.u32(*value);
                }
                Field::Signature(text) => {
                    self.signature("g");
                    self.signature(text);
                }
            }
            if self.overflow {
                return;
            }
        }
        let fields_len = self.buf.len().saturating_sub(data_at) as u32;
        if let Some(slot) = self.buf.get_mut(array_at..array_at + 4) {
            slot.copy_from_slice(&fields_len.to_le_bytes());
        } else {
            self.overflow = true;
            return;
        }
        // The body starts 8-aligned past the fields.
        self.pad(8);
        if self.overflow {
            return;
        }
        self.body_at = Some(self.buf.len());
    }

    /// Ends the message `begin` started: patches the body length and
    /// hands the bytes over, or `None` on any overflow.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        if self.overflow {
            return None;
        }
        let body_at = self.body_at?;
        let len = self.buf.len().saturating_sub(body_at) as u32;
        self.buf[4..8].copy_from_slice(&len.to_le_bytes());
        self.body_at = None;
        Some(core::mem::take(&mut self.buf))
    }
}
