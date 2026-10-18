//! Writes the fuzz seed corpora (`fuzz/corpus/`): real marshalled
//! `Notify` bodies, markup texts and `image-data` structs, every value
//! written inline in one `Writer` (a separately marshalled value spliced
//! with `raw` would sit misaligned, and the reader would refuse the
//! seed). Run once by hand after changing what the seeds cover; the
//! files are committed, and both CI's fuzz job and the corpus-replay
//! unit tests read them.
//!
//! ```sh
//! cargo run -p scootnotify --example gen_corpus
//! ```
//!
//! Dev tooling only: never part of the shipped binary.

use std::path::PathBuf;

use scoot_dbus::proto::Writer;

fn out_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus")
}

fn write(kind: &str, name: &str, bytes: &[u8]) {
    let dir = out_dir().join(kind);
    std::fs::create_dir_all(&dir).expect("the corpus dir writes");
    std::fs::write(dir.join(name), bytes).expect("a seed writes");
    println!("{kind}/{name}: {} bytes", bytes.len());
}

/// One hint entry, value written inline after its signature.
fn entry_str(out: &mut Writer, key: &str, value: &str) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("s");
    out.str(value);
    out.close_struct();
}

/// One hint entry with a `y` value.
fn entry_u8(out: &mut Writer, key: &str, value: u8) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("y");
    out.u8(value);
    out.close_struct();
}

/// One hint entry with a `u` value.
fn entry_u32(out: &mut Writer, key: &str, value: u32) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("u");
    out.u32(value);
    out.close_struct();
}

/// One hint entry with an `i` value.
fn entry_i32(out: &mut Writer, key: &str, value: i32) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("i");
    out.i32(value);
    out.close_struct();
}

/// One hint entry with a `b` value.
fn entry_bool(out: &mut Writer, key: &str, value: bool) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("b");
    out.boolean(value);
    out.close_struct();
}

/// One `image-data` entry, struct written inline.
#[allow(clippy::too_many_arguments)]
fn entry_image(
    out: &mut Writer,
    key: &str,
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
    bits: i32,
    channels: i32,
    pixels: &[u8],
) {
    assert!(out.open_struct());
    out.str(key);
    out.variant("(iiibiiay)");
    assert!(out.open_struct());
    out.i32(width);
    out.i32(height);
    out.i32(rowstride);
    out.boolean(alpha);
    out.i32(bits);
    out.i32(channels);
    let cookie = out.open_array(1).unwrap();
    out.raw(pixels);
    out.close_array(cookie);
    out.close_struct();
    out.close_struct();
}

/// One `image-data` struct value's bytes (a fuzz seed on its own: read
/// from offset zero, exactly as the check reads it).
#[allow(clippy::too_many_arguments)]
fn image_val(
    width: i32,
    height: i32,
    rowstride: i32,
    alpha: bool,
    bits: i32,
    channels: i32,
    pixels: &[u8],
) -> Vec<u8> {
    let mut body = Writer::new();
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
    body.take_body().unwrap()
}

/// One `Notify` body: all eight arguments, one `Writer`, aligned as a
/// real sender's. The hints come from `hints`, which writes entries
/// with the `entry_*` helpers above.
#[allow(clippy::too_many_arguments)]
fn notify(
    app: &str,
    replaces: u32,
    icon: &str,
    summary: &str,
    text: &str,
    actions: &[&str],
    expire: i32,
    hints: impl FnOnce(&mut Writer),
) -> Vec<u8> {
    let mut out = Writer::new();
    out.str(app);
    out.u32(replaces);
    out.str(icon);
    out.str(summary);
    out.str(text);
    let cookie = out.open_array(4).unwrap();
    for action in actions {
        out.str(action);
    }
    out.close_array(cookie);
    let scope = out.open_array(8).unwrap();
    hints(&mut out);
    out.close_array(scope);
    out.i32(expire);
    out.take_body().unwrap()
}

fn main() {
    // --- notify: one seed per shape the server sorts ---
    write(
        "notify",
        "plain",
        &notify("Mail", 0, "", "Hi", "a body", &[], -1, |_| {}),
    );
    write(
        "notify",
        "markup-actions",
        &notify(
            "Chat",
            0,
            "chat-app",
            "A <b>nick</b>",
            "hey <i>you</i> &amp; co",
            &["default", "Open", "reply", "Reply"],
            5000,
            |out| {
                entry_u8(out, "urgency", 1);
                entry_str(out, "category", "im.received");
                entry_i32(out, "value", 42);
                entry_bool(out, "transient", false);
                entry_bool(out, "resident", true);
                entry_str(out, "desktop-entry", "chat");
                entry_str(out, "sound-file", "/s.wav");
                entry_str(out, "x-vendor", "v");
            },
        ),
    );
    let pixels = vec![0x7fu8; 16 * 16 * 4];
    write(
        "notify",
        "image-data",
        &notify("Photos", 0, "", "Exported", "done", &[], 0, |out| {
            entry_image(out, "image-data", 16, 16, 64, true, 8, 4, &pixels);
        }),
    );
    write(
        "notify",
        "replace-critical",
        &notify("Volume", 7, "", "Volume", "80%", &[], -1, |_| {}),
    );
    // Hostile-adjacent but well-formed: long fields (cut, never
    // refused), many actions (16 read), many hints, odd hint types.
    let long = "w".repeat(9000);
    let actions: Vec<&str> = (0..20).map(|_| "a").collect();
    write(
        "notify",
        "hostile",
        &notify(&long, u32::MAX, &long, &long, &long, &actions, i32::MAX, |out| {
            for index in 0..70 {
                entry_str(out, &format!("x-k{index}"), "v");
            }
            entry_str(out, "urgency", "high");
            entry_u32(out, "value", u32::MAX);
        }),
    );

    // --- markup: one seed per parser row ---
    for (name, text) in [
        ("plain", "a plain body, nothing to parse"),
        ("tags", "a <b>bold</b>, <i>italic</i>, <u>underlined</u> word"),
        (
            "link-img",
            "a <a href=\"http://example.com/x?y=1\">link</a> and <img src=\"i\" alt=\"pic\">",
        ),
        (
            "entities",
            "&amp; &lt; &gt; &quot; &apos; &#65; &#x42; &#xD800; &#; &bogus;",
        ),
        ("stripped", "a<blink>fast</blink> and<br/>broken <b>nest <i>both</b> tail"),
        ("malformed", "ok <b>bold <i>never closed <3 and < kmh"),
        ("overflow", &"<b>x</b>".repeat(100)),
        ("controls", "bell\x07 kept\ttab\nnewline"),
    ] {
        write("markup", name, text.as_bytes());
    }

    // --- image-data: honest shapes the validator sorts ---
    write("image-data", "rgba-4x4", &image_val(4, 4, 16, true, 8, 4, &[3u8; 64]));
    write("image-data", "rgb-2x1", &image_val(2, 1, 8, false, 8, 3, &[9u8; 6]));
    write("image-data", "padded", &image_val(1, 1, 8, false, 8, 3, &[0u8; 8]));
    write(
        "image-data",
        "big-512",
        &image_val(512, 256, 2048, true, 8, 4, &vec![1u8; 512 * 256 * 4]),
    );
    // Odd shapes: the validator refuses them, the fuzzer mutates them.
    write("image-data", "zero-width", &image_val(0, 4, 16, true, 8, 4, &[1u8; 64]));
    write("image-data", "short-pixels", &image_val(4, 4, 16, true, 8, 4, &[1u8; 15]));
    println!("done");
}
