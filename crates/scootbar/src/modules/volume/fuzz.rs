//! What the fuzz target checks, written once: `crates/scootbar/fuzz`
//! compiles this file by `#[path]` (so it uses nothing but
//! [`super::proto`], as that uses nothing but `std`), and the stable test
//! beside it replays the committed seed corpus and every past finding
//! through the same function on every `cargo test`, without nightly or
//! `cargo-fuzz`. A panic is a finding: the bar's release profile is
//! `panic = "abort"`.

use super::proto::{self, Bounded};

#[cfg(test)]
mod tests;

/// Any bytes as PulseAudio-protocol frames: framing accepts, waits or
/// refuses without a panic, and every reply parser accepts or refuses
/// without one. Whatever parses is then shaped the way the module shapes
/// it (bounded names, percent conversion, request encoding), which must
/// not panic either.
pub fn volume(data: &[u8]) {
    // Every prefix frames like the module's read loop does: a frame's
    // payload is checked, an incomplete one waits, a bad length refuses.
    let mut at = 0;
    while at < data.len() {
        match proto::frame_at(&data[at..]) {
            Ok(Some((frame, consumed))) => {
                check_payload(frame.payload);
                at += consumed;
            }
            Ok(None) | Err(()) => break,
        }
    }
    // And the whole input parses as each shape directly, so a bare
    // payload with no framing is checked too.
    check_payload(data);
    check_event(data);
    // The conversions and the request encoding, on input-derived values.
    let mut raw = [0u8; 4];
    let take = data.len().min(4);
    raw[..take].copy_from_slice(&data[..take]);
    let number = u32::from_ne_bytes(raw);
    let _ = proto::to_percent(number);
    let _ = proto::from_percent(number % 200);
    let mut writer = proto::Writer::new();
    let _ = writer.put_u32(number);
    let _ = writer.put_bool(!data.is_empty());
    let _ = writer.put_arbitrary(&data[..data.len().min(300)]);
    let _ = writer.put_proplist("application.name", &data[..data.len().min(100)]);
    let mut vols = [number; 4];
    if !data.is_empty() {
        vols[0] = number;
        vols[1] = number.swap_bytes();
    }
    let _ = writer.put_cvolume(&vols[..1 + (data.len() % 4)]);
    let _ = writer.put_str("scootbar");
    let _ = proto::auth_payload(&[0u8; 256], &mut writer);
    let _ = proto::name_payload(&mut writer);
    let mut out = [0u8; 2048];
    let _ = proto::encode_into(
        proto::CMD_GET_SINK_INFO,
        number,
        &data[..data.len().min(1500)],
        &mut out,
    );
}

fn check_payload(payload: &[u8]) {
    let _ = proto::parse_server_info(payload);
    let _ = proto::parse_device_info(payload);
    walk(payload);
}

fn check_event(payload: &[u8]) {
    let _ = proto::parse_event(payload);
}

/// Every tagged value the payload holds, through the matching getter: a
/// wrong marker or a short read refuses, never panics.
fn walk(payload: &[u8]) {
    let mut reader = proto::Reader::new(payload);
    let mut scratch = Bounded::empty();
    let mut vols = [0; proto::MAX_CHANNELS];
    for _ in 0..64 {
        match reader.peek() {
            None => break,
            Some(b't') | Some(b'N') => {
                let _ = reader.get_str(&mut scratch);
            }
            Some(b'L') => {
                let _ = reader.get_u32();
            }
            Some(b'1') | Some(b'0') => {
                let _ = reader.get_bool();
            }
            Some(b'v') => {
                let _ = reader.get_cvolume(&mut vols);
            }
            Some(b'a') => {
                let _ = reader.skip_spec();
            }
            Some(b'm') => {
                let _ = reader.skip_map();
            }
            // Anything else (a proplist, an arbitrary block, a sample
            // format the module never reads alone) ends the walk: the
            // shape parsers above still take the whole payload.
            Some(_) => break,
        }
    }
}
