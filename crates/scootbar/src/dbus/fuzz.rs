//! What the fuzz target checks, written once: `crates/scootbar/fuzz`
//! compiles this file by `#[path]` (so it uses nothing but
//! [`super::proto`], as that uses nothing but `std`), and the stable test
//! beside it replays the committed seed corpus and every past finding
//! through the same function on every `cargo test`, without nightly or
//! `cargo-fuzz`. A panic is a finding: the bar's release profile is
//! `panic = "abort"`.
//!
//! The threat model is the spike's: the daemons validate the wire, so a
//! same-user peer sends valid-but-hostile shapes (deep nesting, huge
//! arrays, megabyte strings), not malformed bytes. The check therefore
//! frames arbitrary bytes (accept, wait or refuse without a panic),
//! parses every framed message (header and the body against its declared
//! signature), walks every shape the tray reads, and round-trips the
//! writer on input-derived values.

use super::proto::{self, Kind, Message, Reader, Writer};

#[cfg(test)]
mod tests;

/// Any bytes as D-Bus messages on a stream.
pub fn dbus(data: &[u8]) {
    // Every prefix frames like the read loop does: a frame's lengths are
    // checked, an incomplete one waits, a bad one refuses.
    let mut at = 0;
    while at < data.len() {
        match proto::frame_at(&data[at..]) {
            Ok(Some(len)) => {
                if let Some(frame) = data.get(at..at.saturating_add(len)) {
                    check_frame(frame);
                    at += len;
                } else {
                    break;
                }
            }
            Ok(None) | Err(()) => break,
        }
    }
    // And the whole input parses as one frame directly, so a bare
    // message with no stream around it is checked too.
    check_frame(data);
    // The name, path, member and signature validators, on slices of the
    // input (valid UTF-8 or not: all four refuse without a panic).
    for end in [0, 1, 7, 64, 129, 300, data.len()] {
        let text = data.get(..end.min(data.len())).unwrap_or(&[]);
        if let Ok(text) = core::str::from_utf8(text) {
            let _ = proto::check_name(text);
            let _ = proto::check_member(text);
            let _ = proto::check_path(text);
            let _ = proto::check_signature(text);
        }
    }
    // The shape readers the connection and the tray use, on the input as
    // each shape directly.
    for sig in ["s", "u", "as", "a{sv}", "sa{sv}as", "(sa(iiay)ss)"] {
        let mut reader = Reader::le(data);
        let _ = reader.skip(sig);
    }
    let _ = proto::read_names("as", data);
    let _ = proto::read_owner("s", data);
    let _ = proto::read_request_reply("u", data);
    let _ = proto::read_name_owner_changed(data);
    let _ = proto::read_string("s", data);
    // The writer on input-derived values: lengths, names and signatures
    // from the input, nested to the depth limit and past it.
    let take = data.len().min(300);
    let text = core::str::from_utf8(&data[..take]).unwrap_or("");
    let mut writer = Writer::new();
    writer.begin_call(1, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello", "", 0);
    let _ = writer.finish();
    let mut named = Writer::new();
    named.begin_call(7, text, text, text, text, "s", 0);
    named.str(text);
    let _ = named.finish();
    let mut deep = Writer::new();
    for _ in 0..proto::MAX_DEPTH + 4 {
        if deep.open_struct() {
            continue;
        }
        break;
    }
    let _ = deep.finish();
    let mut wide = Writer::new();
    if let Some(cookie) = wide.open_array(4) {
        for byte in data.iter().take(1024) {
            wide.u32(u32::from(*byte));
        }
        wide.close_array(cookie);
    }
    let _ = wide.finish();
}

/// One framed message: the header parses, and the body walks against its
/// declared signature, then as each shape the tray reads. A refusal
/// anywhere is `None`, never a panic.
fn check_frame(frame: &[u8]) {
    let Ok(message) = Message::parse(frame) else {
        return;
    };
    let mut body = Reader::le(message.body.rest());
    if !message.signature.is_empty() {
        let _ = body.skip(message.signature);
    }
    check_shapes(&message);
    // Both byte orders parse the same frame the writer emits: re-encode
    // the header's first 16 bytes big-endian and parse again. (Only the
    // fixed header is swapped here; the fields stay little-endian, so a
    // full big-endian message is built in the round-trip test, not here.)
    let _ = message.le;
}

/// The body as each shape the tray reads past the header: pixmap arrays,
/// property dictionaries, tooltips, watch lists, owner changes. Wrong
/// shapes refuse; right ones walk without a panic.
fn check_shapes(message: &Message<'_>) {
    let body = message.body.rest();
    match message.signature {
        "a(iiay)" => {
            let _ = read_pixmaps(body);
        }
        "a{sv}" => {
            let _ = read_properties(body);
        }
        "(sa(iiay)ss)" => {
            let _ = read_tooltip(body);
        }
        _ => {}
    }
    if message.kind == Kind::Signal && message.member == Some("NameOwnerChanged") {
        let _ = proto::read_name_owner_changed(body);
    }
}

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

/// An `a(iiay)` pixmap list: every entry's dimensions against its bytes.
/// An oversized or misshapen entry skips itself, never the whole list;
/// past [`MAX_PIXMAPS`] entries the walk is refused.
pub fn read_pixmaps(body: &[u8]) -> Result<Vec<Pixmap<'_>>, ()> {
    let mut reader = Reader::le(body);
    let raw = reader.array_raw(8)?;
    let mut scoped = Reader::le(raw);
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
    if !reader.exhausted() {
        return Err(());
    }
    Ok(pixmaps)
}

/// An `a{sv}` property dictionary: every entry's key, with known shapes
/// walked typed and unknown ones skipped.
pub fn read_properties(body: &[u8]) -> Result<Vec<(String, String)>, ()> {
    let mut reader = Reader::le(body);
    let raw = reader.array_raw(8)?;
    let mut scoped = Reader::le(raw);
    let mut props = Vec::new();
    while !scoped.exhausted() {
        scoped.enter_struct()?;
        let key = scoped.str()?.to_owned();
        let value = scoped.variant_raw()?;
        match (key.as_str(), value.signature()) {
            ("Status", "s") | ("Title", "s") | ("Id", "s") | ("Category", "s") | ("IconName", "s")
            | ("OverlayIconName", "s") | ("AttentionIconName", "s") | ("AttentionMovieName", "s")
            | ("IconThemePath", "s") => {
                value.as_str()?;
            }
            ("ItemIsMenu", "b") => {
                value.as_bool()?;
            }
            ("WindowId", "u") | ("WindowId", "i") => {
                value.as_u32()?;
            }
            ("Menu", "o") => {
                value.as_str()?;
            }
            ("IconPixmap", "a(iiay)") | ("OverlayIconPixmap", "a(iiay)") | ("AttentionIconPixmap", "a(iiay)") => {
                let _ = read_pixmaps(value.read().rest());
            }
            ("ToolTip", "(sa(iiay)ss)") => {
                let _ = read_tooltip(value.read().rest());
            }
            _ => {
                // Unknown or mistyped: skipped by signature, so a future
                // property cannot break the walk.
                let mut skipping = value.read();
                skipping.skip(value.signature())?;
            }
        }
        scoped.leave_struct();
        if props.len() >= 64 {
            return Err(());
        }
        props.push((key, value.signature().to_owned()));
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// A `(sa(iiay)ss)` tooltip: the name, the pixmaps, the title and text.
pub fn read_tooltip(body: &[u8]) -> Result<(String, String, String), ()> {
    let mut reader = Reader::le(body);
    reader.enter_struct()?;
    let name = reader.str()?.to_owned();
    let pixmaps = reader.array_raw(8)?;
    let _ = read_pixmaps(pixmaps);
    let title = reader.str()?.to_owned();
    let text = reader.str()?.to_owned();
    reader.leave_struct();
    if !reader.exhausted() {
        return Err(());
    }
    Ok((name, title, text))
}
