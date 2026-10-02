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
//! - [`MAX_MESSAGE`]: a message past 1 MiB is refused (the spec allows
//!   128 MiB; the tray's largest legitimate payload, pixmaps at requested
//!   device pixels, is tens of KiB).
//! - [`MAX_DEPTH`]: containers nested past 32 deep are refused.
//! - [`MAX_NAME`]: a bus, interface, member or error name past 128 bytes
//!   is refused (the volume module's rule for names).
//! - [`MAX_SIGNATURE`]: a signature past the spec's 255 bytes is refused.
//!
//! Strings in a body carry no cap here: they are borrowed, never copied,
//! and the consumer bounds them at use (titles are cut like view text).
//! The writer mirrors the caps: anything past them overflows, and
//! [`Writer::finish`] returns `None`.
//!
//! ## Endianness
//!
//! The client sends little-endian only and accepts either byte order on
//! the wire (both daemons speak little-endian in practice).

/// The most bytes one message holds, header and body: the starting point
/// the spike set (tray pixmaps at requested device pixels are tens of
/// KiB; menu layouts are the unbounded one).
pub const MAX_MESSAGE: usize = 1024 * 1024;

/// The deepest containers nest, in a signature or on the wire.
pub const MAX_DEPTH: usize = 32;

/// The longest bus, interface, member or error name taken, in bytes.
pub const MAX_NAME: usize = 128;

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
    /// Do not start a service to answer (calls that must not wake one).
    pub const NO_AUTO_START: u8 = 0x2;
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
    pub const IN_QUEUE: u32 = 2;
    pub const EXISTS: u32 = 3;
    pub const ALREADY_OWNER: u32 = 4;
}

/// How many bytes frame `buf`'s first message: `Ok(Some(n))` when the
/// whole message is there, `Ok(None)` when it is incomplete, `Err(())`
/// when it is refused (bad magic, version, kind, or a length past
/// [`MAX_MESSAGE`]).
pub fn frame_at(buf: &[u8]) -> Result<Option<usize>, ()> {
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
    if body_len > MAX_MESSAGE || fields_len > MAX_MESSAGE {
        return Err(());
    }
    let mut total = 16usize.saturating_add(fields_len);
    total = total.saturating_add((8usize.saturating_sub(total % 8)) % 8);
    total = total.saturating_add(body_len);
    if total > MAX_MESSAGE {
        return Err(());
    }
    if buf.len() < total {
        return Ok(None);
    }
    Ok(Some(total))
}

fn get_u32(at: &[u8], le: bool) -> u32 {
    let bytes = [at[0], at[1], at[2], at[3]];
    if le {
        u32::from_le_bytes(bytes)
    } else {
        u32::from_be_bytes(bytes)
    }
}

/// A parsed message, borrowing the frame. The header's strings are
/// validated (names bounded by [`MAX_NAME`], paths and signatures by
/// their own rules); the body is walked lazily through [`Reader`], so a
/// hostile body is refused where it is touched, not here.
#[derive(Debug)]
pub struct Message<'a> {
    /// Little-endian when true, big-endian when false.
    pub le: bool,
    pub kind: Kind,
    pub flags: u8,
    pub serial: u32,
    /// The sender's unique name, when the daemon set one.
    pub sender: Option<&'a str>,
    /// The destination: a unique or well-known name, or absent
    /// (broadcast signals).
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
        let le = match *frame.first().ok_or(())? {
            b'l' => true,
            b'B' => false,
            _ => return Err(()),
        };
        if frame.len() < 16 || frame[3] != 1 {
            return Err(());
        }
        let kind = Kind::from_byte(frame[1]).ok_or(())?;
        let flags = frame[2];
        let body_len = get_u32(&frame[4..8], le) as usize;
        let serial = get_u32(&frame[8..12], le);
        if serial == 0 {
            return Err(());
        }
        let fields_len = get_u32(&frame[12..16], le) as usize;
        let mut fields = Reader {
            buf: frame.get(16..16usize.saturating_add(fields_len)).ok_or(())?,
            pos: 0,
            le,
            depth: 0,
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
            let value = fields.variant_raw()?;
            fields.leave();
            match code {
                1 => path = Some(check_path(value.as_str()?)?),
                2 => interface = Some(check_name(value.as_str()?)?),
                3 => member = Some(check_member(value.as_str()?)?),
                4 => error = Some(check_name(value.as_str()?)?),
                5 => reply_serial = Some(value.as_u32()?),
                6 => destination = Some(check_name(value.as_str()?)?),
                7 => sender = Some(check_name(value.as_str()?)?),
                8 => signature = check_signature(value.as_signature()?)?,
                // File descriptors are never negotiated by this client:
                // one on the wire is a peer speaking out of turn.
                9 => return Err(()),
                _ => return Err(()),
            }
        }
        let mut at = 16usize.saturating_add(fields_len);
        at = at.saturating_add((8usize.saturating_sub(at % 8)) % 8);
        let body = frame.get(at..at.saturating_add(body_len)).ok_or(())?;
        Ok(Self {
            le,
            kind,
            flags,
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
            },
        })
    }
}

/// A cursor over message bytes: every read is bounds-checked, aligned as
/// the type needs, and refused past the end. Containers count depth
/// against [`MAX_DEPTH`].
#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    le: bool,
    depth: u8,
}

impl<'a> Reader<'a> {
    /// A reader over `buf` in little-endian order: what the writer emits,
    /// and what the tests feed the fuzz check.
    pub fn le(buf: &'a [u8]) -> Self {
        Self {
            buf,
            pos: 0,
            le: true,
            depth: 0,
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

    /// Where the cursor sits, for a nested body borrowed whole.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The unread bytes, for handing a nested body over whole.
    pub fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos.min(self.buf.len())..]
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ()> {
        let bytes = self.buf.get(self.pos..self.pos.saturating_add(n)).ok_or(())?;
        self.pos = self.pos.saturating_add(n);
        Ok(bytes)
    }

    fn align(&mut self, n: usize) -> Result<(), ()> {
        let pad = (n.saturating_sub(self.pos % n)) % n;
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

    /// Any unsigned integer width the protocol has (1, 2, 4 or 8 bytes).
    pub fn uint(&mut self, n: usize) -> Result<u64, ()> {
        match n {
            1 | 2 | 4 | 8 => self.int(n),
            _ => Err(()),
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
        Ok(self.int(8)? as u64 as i64)
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
    /// refused past what is left.
    pub fn array_raw(&mut self, element_align: usize) -> Result<&'a [u8], ()> {
        self.align(4)?;
        let len = self.u32_raw()? as usize;
        if len > self.remaining() {
            return Err(());
        }
        if element_align > 1 {
            let pad = (element_align.saturating_sub(self.pos % element_align)) % element_align;
            if pad > self.remaining() {
                return Err(());
            }
            self.pos += pad;
            if len > self.remaining() {
                return Err(());
            }
        }
        self.take(len)
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
        let mut scoped = Reader {
            buf: value.raw,
            pos: 0,
            le: self.le,
            depth: self.depth,
        };
        let out = read(value.sig, &mut scoped)?;
        self.pos = self.pos.saturating_add(scoped.pos);
        Ok(out)
    }

    /// Skips one complete value of signature `sig`, refusing a malformed
    /// one. What the tray walks past unknown properties with, and what
    /// the fuzz check walks whole bodies with.
    pub fn skip(&mut self, sig: &str) -> Result<(), ()> {
        let bytes = sig.as_bytes();
        check_signature(sig)?;
        let (at, _) = skip_value(self, bytes, 0, self.depth as usize)?;
        let _ = at;
        Ok(())
    }
}

/// Skips one complete value of the type at `bytes[at]`, returning the
/// signature offset past it. Dict entries and structs need their field
/// signatures from the enclosing signature, which is why the walk is over
/// signature bytes rather than wire bytes alone.
fn skip_value<'a>(reader: &mut Reader<'a>, bytes: &[u8], at: usize, depth: usize) -> Result<(usize, ()), ()> {
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
        b'q' => {
            reader.u16()?;
            Ok((at + 1, ()))
        }
        b'i' | b'u' => {
            reader.u32()?;
            Ok((at + 1, ()))
        }
        b'x' | b't' => {
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
            let raw = reader.array_raw(align)?;
            let mut scoped = Reader {
                buf: raw,
                pos: 0,
                le: reader.le,
                depth: reader.depth,
            };
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
        b'b' | b'i' | b'u' | b's' | b'o' | b'a' => Ok(4),
        b'x' | b't' | b'd' | b'(' | b'{' => Ok(8),
        _ => Err(()),
    }
}

/// A variant's value, borrowed: its signature and its raw bytes.
#[derive(Debug, Clone, Copy)]
pub struct Variant<'a> {
    le: bool,
    depth: u8,
    sig: &'a str,
    raw: &'a [u8],
}

impl<'a> Variant<'a> {
    pub fn signature(&self) -> &'a str {
        self.sig
    }

    /// The value's bytes from here on, to walk with a reader at the
    /// caller's depth.
    pub fn read(&self) -> Reader<'a> {
        Reader {
            buf: self.raw,
            pos: 0,
            le: self.le,
            depth: self.depth,
        }
    }

    /// The value as a string (`s`, `o` or `g`).
    pub fn as_str(&self) -> Result<&'a str, ()> {
        match self.sig {
            "s" | "o" => self.read().string(),
            "g" => {
                let mut reader = self.read();
                let len = reader.u8()? as usize;
                let bytes = reader.take(len)?;
                if reader.take(1)? != [0] {
                    return Err(());
                }
                core::str::from_utf8(bytes).map_err(|_| ())
            }
            _ => Err(()),
        }
    }

    /// The value as a signature (`g`).
    pub fn as_signature(&self) -> Result<&'a str, ()> {
        if self.sig != "g" {
            return Err(());
        }
        self.as_str()
    }

    /// The value as a 32-bit integer (`i`, `u` or `b`).
    pub fn as_u32(&self) -> Result<u32, ()> {
        match self.sig {
            "i" | "u" | "b" => Ok(self.read().int(4)? as u32),
            _ => Err(()),
        }
    }

    /// The value as a boolean (`b`).
    pub fn as_bool(&self) -> Result<bool, ()> {
        if self.sig != "b" {
            return Err(());
        }
        self.read().boolean()
    }
}

/// Whether `name` is a usable bus, interface, member or error name:
/// 1 to [`MAX_NAME`] bytes, a unique name (`:1.42`) or dotted
/// well-known one of at least two valid elements. Refuses anything else,
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
    if path.len() > MAX_MESSAGE {
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

/// Parses one complete type at `at`, returning the end and whether it is
/// a basic type (a dict key must be one).
fn complete(bytes: &[u8], at: usize, depth: usize) -> Result<(usize, bool), ()> {
    if depth > MAX_DEPTH {
        return Err(());
    }
    let byte = *bytes.get(at).ok_or(())?;
    match byte {
        b'y' | b'b' | b'n' | b'q' | b'i' | b'u' | b'x' | b't' | b'd' | b'h' | b's' | b'o' | b'g' => {
            Ok((at + 1, true))
        }
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
                let next;
                (at, _) = complete(bytes, at, depth + 1)?;
                next = at;
                match bytes.get(next) {
                    Some(b')') => return Ok((next + 1, false)),
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
/// is memory, not time).
pub const MAX_LIST_NAMES: usize = 4096;

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
        check_name(name)?;
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
pub fn read_name_owner_changed(body: &[u8]) -> Result<(String, Option<String>, Option<String>), ()> {
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

/// A message writer: little-endian, capped at [`MAX_MESSAGE`]. Anything
/// past a cap sets the overflow, and [`Writer::finish`] returns `None` —
/// a message that does not fit is never half-sent.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
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

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether anything overflowed so far.
    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    fn reserve(&mut self, n: usize) {
        if self.overflow {
            return;
        }
        if self.buf.len().saturating_add(n) > MAX_MESSAGE {
            self.overflow = true;
            return;
        }
        self.buf.reserve(n);
    }

    fn pad(&mut self, n: usize) {
        if n <= 1 || self.overflow {
            return;
        }
        let pad = (n.saturating_sub(self.buf.len() % n)) % n;
        self.reserve(pad);
        if !self.overflow {
            self.buf.extend(core::iter::repeat(0).take(pad));
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

    pub fn u8(&mut self, value: u8) {
        self.push(&[value]);
    }

    pub fn u16(&mut self, value: u16) {
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
        self.buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
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

    /// Opens an array: 4-aligned, then aligned for `element_align`, with
    /// the length word left for [`Writer::close_array`]. Returns the
    /// cookie it takes.
    pub fn open_array(&mut self, element_align: usize) -> Option<usize> {
        if self.depth >= MAX_DEPTH {
            self.overflow = true;
            return None;
        }
        self.pad(4);
        if element_align > 1 {
            self.pad(element_align);
        }
        self.reserve(4);
        if self.overflow {
            return None;
        }
        self.depth += 1;
        let cookie = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        Some(cookie)
    }

    /// Closes the array `open_array` opened: the length word takes the
    /// bytes since.
    pub fn close_array(&mut self, cookie: usize) {
        self.depth = self.depth.saturating_sub(1);
        if self.overflow {
            return;
        }
        let len = self.buf.len().saturating_sub(cookie + 4) as u32;
        if let Some(slot) = self.buf.get_mut(cookie..cookie + 4) {
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
    /// signature (empty while there is no body).
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
        // The fields array: written whole, then its length patched, so no
        // second pass over the header is needed.
        let array_at = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
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
        let fields_len = self.buf.len().saturating_sub(array_at + 4) as u32;
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
        let Some(body_at) = self.body_at else {
            return None;
        };
        let len = self.buf.len().saturating_sub(body_at) as u32;
        self.buf[4..8].copy_from_slice(&len.to_le_bytes());
        self.body_at = None;
        Some(core::mem::take(&mut self.buf))
    }
}
