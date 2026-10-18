//! One-`Writer` marshalling for the bus tests: everything inline, so
//! every value lands aligned the way a real sender's would. A separately
//! marshalled value spliced with `raw` would sit misaligned (its padding
//! was relative to its own start), and the reader would refuse it.
//!
//! Test-only: the corpus generator (`examples/gen_corpus.rs`) carries its
//! own copy, since it builds without `cfg(test)`.

use scoot_dbus::proto::Writer;

/// One hint value, written inline after its `variant(sig)`.
#[derive(Debug, Clone, Copy)]
pub enum Val<'a> {
    Str(&'a str),
    U8(u8),
    U32(u32),
    I32(i32),
    Bool(bool),
    Image {
        width: i32,
        height: i32,
        rowstride: i32,
        alpha: bool,
        bits: i32,
        channels: i32,
        pixels: &'a [u8],
    },
}

/// One marshalled `a{sv}` from `entries`.
pub fn hints_bytes(entries: &[(&str, Val<'_>)]) -> Vec<u8> {
    let mut body = Writer::new();
    write_hints(&mut body, entries);
    body.take_body().unwrap()
}

/// Writes the `a{sv}` into `body` (for embedding in a `Notify`).
pub fn write_hints(body: &mut Writer, entries: &[(&str, Val<'_>)]) {
    let cookie = body.open_array(8).unwrap();
    for (key, value) in entries {
        assert!(body.open_struct());
        body.str(key);
        write_val(body, value);
        body.close_struct();
    }
    body.close_array(cookie);
}

/// Writes one variant value inline: the signature, then the value the
/// reader's alignment expects.
pub fn write_val(body: &mut Writer, value: &Val<'_>) {
    match *value {
        Val::Str(text) => {
            body.variant("s");
            body.str(text);
        }
        Val::U8(number) => {
            body.variant("y");
            body.u8(number);
        }
        Val::U32(number) => {
            body.variant("u");
            body.u32(number);
        }
        Val::I32(number) => {
            body.variant("i");
            body.i32(number);
        }
        Val::Bool(flag) => {
            body.variant("b");
            body.boolean(flag);
        }
        Val::Image {
            width,
            height,
            rowstride,
            alpha,
            bits,
            channels,
            pixels,
        } => {
            body.variant("(iiibiiay)");
            assert!(body.open_struct());
            body.i32(width);
            body.i32(height);
            body.i32(rowstride);
            body.boolean(alpha);
            body.i32(bits);
            body.i32(channels);
            let cookie = body.open_array(1).unwrap();
            body.raw(pixels);
            body.close_array(cookie);
            body.close_struct();
        }
    }
}

/// One marshalled `Notify` body: all eight arguments, hints inline.
#[allow(clippy::too_many_arguments)]
pub fn notify_bytes(
    app_name: &str,
    replaces: u32,
    icon: &str,
    summary: &str,
    body_text: &str,
    actions: &[&str],
    hints: &[(&str, Val<'_>)],
    expire: i32,
) -> Vec<u8> {
    let mut out = Writer::new();
    out.str(app_name);
    out.u32(replaces);
    out.str(icon);
    out.str(summary);
    out.str(body_text);
    let cookie = out.open_array(4).unwrap();
    for action in actions {
        out.str(action);
    }
    out.close_array(cookie);
    write_hints(&mut out, hints);
    out.i32(expire);
    out.take_body().unwrap()
}
