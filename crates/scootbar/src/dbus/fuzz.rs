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

use super::proto::{self, Kind, Message, Reader, Writer, read_pixmaps};

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
        // `variant` positions past the value; borrowing it raw would
        // leave the next entry starting inside this one.
        let sig = scoped.variant(|sig, value| {
            match (key.as_str(), sig) {
                ("Status", "s") | ("Title", "s") | ("Id", "s") | ("Category", "s") | ("IconName", "s")
                | ("OverlayIconName", "s") | ("AttentionIconName", "s") | ("AttentionMovieName", "s")
                | ("IconThemePath", "s") => {
                    value.str()?;
                }
                ("ItemIsMenu", "b") => {
                    value.boolean()?;
                }
                ("WindowId", "u") | ("WindowId", "i") => {
                    value.u32()?;
                }
                ("Menu", "o") => {
                    value.str()?;
                }
                ("IconPixmap", "a(iiay)") | ("OverlayIconPixmap", "a(iiay)") | ("AttentionIconPixmap", "a(iiay)") => {
                    let _ = read_pixmaps(value.rest());
                }
                ("ToolTip", "(sa(iiay)ss)") => {
                    let _ = read_tooltip(value.rest());
                }
                _ => {
                    // Unknown or mistyped: skipped by signature, so a
                    // future property cannot break the walk.
                    value.skip(sig)?;
                }
            }
            Ok(sig.to_owned())
        })?;
        scoped.leave_struct();
        if props.len() >= 64 {
            return Err(());
        }
        props.push((key, sig));
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
    let full = reader.array_full(8)?;
    let _ = read_pixmaps(full);
    let title = reader.str()?.to_owned();
    let text = reader.str()?.to_owned();
    reader.leave_struct();
    if !reader.exhausted() {
        return Err(());
    }
    Ok((name, title, text))
}
