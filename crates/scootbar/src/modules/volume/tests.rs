//! The volume module through the harness: the waiting state with no
//! server, and the whole protocol against a fake server on a unix socket
//! in a temp dir. The fake speaks what pipewire-pulse was measured to
//! speak (see the module docs), with scripted replies, so the handshake,
//! every event, every error and a scroll flood are deterministic. Real
//! packets captured from the Asahi machine pin the parser as golden
//! vectors.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use rustix::event::PollFlags;

use super::proto::{self, Bounded, Kind};
use super::{Settings, start_with};
use crate::action::{Action, ModuleAction, Trigger};
use crate::icon::path::Vector;
use crate::icon::raster::Rasterizer;
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, OutputView, Update};

// ---------------------------------------------------------------------------
// The fake server.
// ---------------------------------------------------------------------------

/// A scripted PulseAudio server: one connection, exact frames, nothing
/// more. Blocking, with a timeout on every read, so a module that stops
/// talking fails the test instead of hanging it.
struct Fake {
    dir: PathBuf,
    sock: PathBuf,
    listener: UnixListener,
}

struct Conn {
    stream: UnixStream,
}

impl Fake {
    fn bind() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "scootbar-volume-{}-{}",
            std::process::id(),
            nano()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("native");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(false).unwrap();
        Self { dir, sock, listener }
    }

    fn accept(&self) -> Conn {
        let (stream, _) = self.listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Conn { stream }
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sock);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

fn nano() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_nanos() as u64)
        .unwrap_or(n)
        .wrapping_add(n)
}

impl Conn {
    /// The next request: its command, tag and payload.
    fn frame(&mut self) -> (u32, u32, Vec<u8>) {
        let mut header = [0u8; 20];
        self.stream.read_exact(&mut header).unwrap();
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        assert!((10..=proto::MAX_FRAME).contains(&len), "bad length {len}");
        let mut body = vec![0u8; len];
        self.stream.read_exact(&mut body).unwrap();
        let mut full = header.to_vec();
        full.extend_from_slice(&body);
        let (frame, consumed) = proto::frame_at(&full).unwrap().expect("no frame");
        assert_eq!(consumed, 20 + len);
        (frame.cmd, frame.tag, frame.payload.to_vec())
    }

    fn expect(&mut self, cmd: u32) -> (u32, Vec<u8>) {
        let (got, tag, payload) = self.frame();
        assert_eq!(got, cmd, "expected command {cmd}, got {got}");
        (tag, payload)
    }

    fn reply(&mut self, tag: u32, payload: &[u8]) {
        let mut out = vec![0u8; 20 + 10 + payload.len()];
        let n = proto::encode_into(proto::CMD_REPLY, tag, payload, &mut out).unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    fn ack(&mut self, tag: u32) {
        self.reply(tag, &[]);
    }

    fn error(&mut self, tag: u32, errno: u32) {
        let mut writer = proto::Writer::new();
        writer.put_u32(errno).unwrap();
        let payload = writer.done().to_vec();
        let mut out = vec![0u8; 64];
        let n = proto::encode_into(0, tag, &payload, &mut out).unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    fn event(&mut self, change: u32, index: u32) {
        let mut writer = proto::Writer::new();
        writer.put_u32(change).unwrap();
        writer.put_u32(index).unwrap();
        let payload = writer.done().to_vec();
        let mut out = vec![0u8; 64];
        let n = proto::encode_into(proto::CMD_SUBSCRIBE_EVENT, proto::INVALID_INDEX, &payload, &mut out)
            .unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    fn reply_server_info(&mut self, tag: u32, sink: &str, source: &str) {
        let mut payload = Vec::new();
        for text in ["PipeAudio", "17.0", "steve", "nixos"] {
            payload.push(b't');
            payload.extend_from_slice(text.as_bytes());
            payload.push(0);
        }
        payload.push(b'a');
        payload.extend_from_slice(&[5, 2, 0, 0, 0xbb, 0x80]);
        push_str(&mut payload, sink);
        push_str(&mut payload, source);
        payload.push(b'L');
        payload.extend_from_slice(&711939437u32.to_be_bytes());
        payload.push(b'm');
        payload.extend_from_slice(&[2, 1, 2]);
        self.reply(tag, &payload);
    }

    #[allow(clippy::too_many_arguments)]
    fn reply_device(
        &mut self,
        tag: u32,
        index: u32,
        name: &str,
        desc: &str,
        vols: &[u32],
        muted: bool,
    ) {
        let mut payload = Vec::new();
        payload.push(b'L');
        payload.extend_from_slice(&index.to_be_bytes());
        push_str(&mut payload, name);
        push_str(&mut payload, desc);
        payload.push(b'a');
        payload.extend_from_slice(&[7, 2, 0, 0, 0xbb, 0x80]);
        payload.push(b'm');
        payload.extend_from_slice(&[2, 1, 2]);
        payload.push(b'L');
        payload.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
        payload.push(b'v');
        payload.push(vols.len() as u8);
        for volume in vols {
            payload.extend_from_slice(&volume.to_be_bytes());
        }
        payload.push(if muted { b'1' } else { b'0' });
        self.reply(tag, &payload);
    }

    /// Asserts nothing arrives within `ms`: the module sent nothing.
    fn quiet(&mut self, ms: u64) {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(ms)))
            .unwrap();
        let mut byte = [0u8; 1];
        let read = self.stream.read(&mut byte);
        self.stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert!(read.is_err(), "expected silence, got a frame");
    }
}

fn push_str(into: &mut Vec<u8>, text: &str) {
    into.push(b't');
    into.extend_from_slice(text.as_bytes());
    into.push(0);
}

// ---------------------------------------------------------------------------
// Driving the module.
// ---------------------------------------------------------------------------

const SINK: &str = "audio_effect.j413-convolver";
const SOURCE: &str = "effect_output.j413-mic";
const DESC: &str = "Convolver";
const INDEX: u32 = 74;
const VOL: u32 = 32113;
const STEP: u32 = 3277;

fn sink_module(sock: PathBuf) -> Harness {
    Harness::new(start_with(&Settings::default(), Kind::Sink, sock))
}

fn mic_module(sock: PathBuf) -> Harness {
    Harness::new(start_with(&Settings::default(), Kind::Source, sock))
}

/// One turn of the loop, expecting progress: the module asked, answered or
/// changed, not silence.
fn wait(harness: &mut Harness) -> Update {
    harness
        .wait(Duration::from_secs(5))
        .expect("the module went quiet")
}

/// The handshake to a shown level: AUTH, name, server info, subscribe and
/// the default device, at `VOL`, unmuted. `default` is the name the server
/// info gives for this kind, `device` the name the device query answers.
fn handshake(conn: &mut Conn, harness: &mut Harness, kind: Kind, default: &str, device: &str) {
    let (cmd, tag, _) = conn.frame();
    assert_eq!(cmd, proto::CMD_AUTH);
    conn.reply(tag, &[b'L', 0, 0, 0, 35]);
    wait(harness);
    let (tag, _) = conn.expect(proto::CMD_SET_CLIENT_NAME);
    let _ = cmd;
    conn.ack(tag);
    wait(harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, SINK, SOURCE);
    wait(harness);
    let (tag, payload) = conn.expect(proto::CMD_SUBSCRIBE);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(kind.subscribe_mask()));
    conn.ack(tag);
    wait(harness);
    let (tag, payload) = conn.expect(kind.get_info());
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut name = Bounded::empty();
    assert_eq!(
        reader.get_str(&mut name).map(|s| s.map(str::to_owned)),
        Some(Some(default.to_owned()))
    );
    conn.reply_device(tag, INDEX, device, DESC, &[VOL, VOL], false);
    assert_eq!(wait(harness), Update::Changed);
}

/// A set answer and its re-read, ending at the device's new values.
fn answer_set(conn: &mut Conn, harness: &mut Harness, vols: &[u32], muted: bool) {
    let (ack, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(ack, SINK, SOURCE);
    wait(harness);
    let (tag, _) = conn.expect(Kind::Sink.get_info());
    conn.reply_device(tag, INDEX, SINK, DESC, vols, muted);
    wait(harness);
}

fn view_text(harness: &Harness) -> String {
    harness.view().text().to_owned()
}

// ---------------------------------------------------------------------------
// The waiting state.
// ---------------------------------------------------------------------------

const FAKE_EVENTS: [PollFlags; 6] = [
    PollFlags::IN,
    PollFlags::PRI,
    PollFlags::ERR,
    PollFlags::HUP,
    PollFlags::NVAL,
    PollFlags::IN.union(PollFlags::HUP).union(PollFlags::ERR),
];

#[test]
fn with_no_server_it_waits_with_nothing_shown() {
    let dir = std::env::temp_dir().join(format!("scootbar-volume-wait-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = sink_module(dir.join("native"));
    // One source (the directory watch), an empty view and no value.
    assert_eq!(harness.source_count(), 1);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // Every event the loop can hand it, without a panic and still empty.
    for events in FAKE_EVENTS {
        let _ = harness.deliver(0, events);
        assert!(harness.view().is_empty());
    }
    // Nothing to act on: invoking names the missing server.
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Err(InvokeError::Refused("no sound server is running"))
    );
    let _ = std::fs::remove_dir(&dir);
}

#[test]
fn socket_path_forms() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(
        super::socket_path_for(Some(OsStr::from_bytes(b"unix:/tmp/x"))),
        PathBuf::from("/tmp/x")
    );
    assert_eq!(
        super::socket_path_for(Some(OsStr::from_bytes(b"/tmp/y"))),
        PathBuf::from("/tmp/y")
    );
    let fallback = super::runtime_dir().join("pulse").join("native");
    assert_eq!(super::socket_path_for(None), fallback);
    assert_eq!(super::socket_path_for(Some(OsStr::from_bytes(b""))), fallback);
    assert_eq!(
        super::socket_path_for(Some(OsStr::from_bytes(b"unix:"))),
        fallback
    );
}

// ---------------------------------------------------------------------------
// The handshake and the shown state.
// ---------------------------------------------------------------------------

#[test]
fn the_handshake_shows_the_level() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    let view = harness.view();
    assert_eq!(view.text(), "49%");
    assert_eq!(view.class(), Class::Normal);
    assert!(view.art().is_some());
    assert_eq!(view.tooltip(), "Convolver: 49%");
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"volume": 49, "muted": false, "sink": SINK}))
    );
}

#[test]
fn the_microphone_follows_the_default_source() {
    let fake = Fake::bind();
    let mut harness = mic_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Source, SOURCE, SOURCE);
    assert_eq!(view_text(&harness), "49%");
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"volume": 49, "muted": false, "source": SOURCE}))
    );
}

// ---------------------------------------------------------------------------
// Sets.
// ---------------------------------------------------------------------------

#[test]
fn raise_sends_an_absolute_set_and_shows_what_the_server_says() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    // One absolute set from the shown level: 32113 + 3277.
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(INDEX));
    let mut name = Bounded::empty();
    assert_eq!(
        reader.get_str(&mut name).map(|s| s.map(str::to_owned)),
        Some(Some(SINK.to_owned()))
    );
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + STEP, VOL + STEP]);
    // A second raise before the answer coalesces: no second frame.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    conn.quiet(200);
    conn.ack(tag);
    wait(&mut harness);
    // The answer re-reads, and the server clamped the set: shown is 34000.
    answer_set(&mut conn, &mut harness, &[34000, 34000], false);
    assert_eq!(view_text(&harness), "52%");
}

#[test]
fn at_max_no_set_is_sent() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    // To full scale first.
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 30),
        Ok(Update::Unchanged)
    );
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [65536, 65536]);
    conn.ack(tag);
    wait(&mut harness);
    answer_set(&mut conn, &mut harness, &[65536, 65536], false);
    assert_eq!(view_text(&harness), "100%");
    // Already there: nothing is sent.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    conn.quiet(200);
}

#[test]
fn lower_and_toggle_mute() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("lower", None), 2),
        Ok(Update::Unchanged)
    );
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL - 2 * STEP, VOL - 2 * STEP]);
    conn.ack(tag);
    wait(&mut harness);
    answer_set(&mut conn, &mut harness, &[VOL - 2 * STEP, VOL - 2 * STEP], false);
    assert_eq!(view_text(&harness), "39%");
    // Mute: one byte, then the class, the icon slot and the tooltip.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("toggle-mute", None), 1),
        Ok(Update::Unchanged)
    );
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_MUTE);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(INDEX));
    assert_eq!(reader.get_bool(), Some(true));
    conn.ack(tag);
    wait(&mut harness);
    answer_set(&mut conn, &mut harness, &[VOL - 2 * STEP, VOL - 2 * STEP], true);
    let view = harness.view();
    assert_eq!(view.class(), Class::Muted);
    assert!(view.art().is_some());
    assert!(view.tooltip().ends_with(" (muted)"));
    assert_eq!(
        harness.value_on(None),
        Some(serde_json::json!({"volume": 39, "muted": true, "sink": SINK}))
    );
}

// ---------------------------------------------------------------------------
// Events.
// ---------------------------------------------------------------------------

#[test]
fn a_sink_change_re_reads_the_tracked_device() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    conn.event(0x10, INDEX);
    wait(&mut harness);
    answer_set(&mut conn, &mut harness, &[40000, 40000], false);
    assert_eq!(view_text(&harness), "61%");
}

#[test]
fn an_untracked_sink_change_is_ignored() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    conn.event(0x10, 99);
    let _ = harness.wait(Duration::from_millis(300));
    conn.quiet(200);
    assert_eq!(view_text(&harness), "49%");
}

#[test]
fn a_server_change_rereads_the_defaults() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    conn.event(0x17, proto::INVALID_INDEX);
    wait(&mut harness);
    // New default: the device query names it.
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "other-sink", SOURCE);
    wait(&mut harness);
    let (tag, payload) = conn.expect(Kind::Sink.get_info());
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let mut name = Bounded::empty();
    assert_eq!(
        reader.get_str(&mut name).map(|s| s.map(str::to_owned)),
        Some(Some("other-sink".to_owned()))
    );
    conn.reply_device(tag, 80, "other-sink", "Other", &[10000, 10000], false);
    assert_eq!(wait(&mut harness), Update::Changed);
    assert_eq!(view_text(&harness), "15%");
}

#[test]
fn a_removed_device_clears_the_view() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    conn.event(0x20, INDEX);
    wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "", SOURCE);
    assert_eq!(wait(&mut harness), Update::Changed);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
}

// ---------------------------------------------------------------------------
// Failure.
// ---------------------------------------------------------------------------

#[test]
fn malformed_bytes_drop_the_connection() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    // A length no server may send.
    conn.stream.write_all(&[0, 0, 0x10, 0]).unwrap();
    let _ = harness.wait(Duration::from_secs(5));
    assert!(harness.view().is_empty());
    // Waiting again: nothing is sent unprompted.
    conn.quiet(200);
}

#[test]
fn a_missing_default_refuses_sets() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    // Handshake with no default sink at all.
    let (cmd, tag, _) = conn.frame();
    assert_eq!(cmd, proto::CMD_AUTH);
    conn.reply(tag, &[b'L', 0, 0, 0, 35]);
    wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_SET_CLIENT_NAME);
    conn.ack(tag);
    wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "", SOURCE);
    wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_SUBSCRIBE);
    conn.ack(tag);
    wait(&mut harness);
    // The answer re-reads: still no default, so no device is asked for.
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "", SOURCE);
    wait(&mut harness);
    conn.quiet(300);
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Err(InvokeError::Refused("no default sink to change"))
    );
}

// ---------------------------------------------------------------------------
// Appearance and input.
// ---------------------------------------------------------------------------

#[test]
fn a_late_server_is_found_on_its_directory() {
    let pending = pending();
    let mut harness = sink_module(pending.sock.clone());
    assert!(harness.view().is_empty());
    // The server appears after the module started watching.
    let fake = pending.listen();
    assert_eq!(harness.wait(Duration::from_secs(5)), Some(Update::Unchanged));
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
}

// A fake around an already-bound listener, for the late server.
struct Pending {
    dir: PathBuf,
    sock: PathBuf,
}

fn pending() -> Pending {
    let dir = std::env::temp_dir().join(format!("scootbar-volume-late-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("native");
    Pending { dir, sock }
}

impl Pending {
    fn listen(self) -> Fake {
        let _ = std::fs::remove_file(&self.sock);
        let listener = UnixListener::bind(&self.sock).unwrap();
        Fake {
            dir: self.dir,
            sock: self.sock,
            listener,
        }
    }
}

#[test]
fn click_and_scroll_have_defaults() {
    use ab_glyph::{FontArc, FontVec};
    use crate::density::Scale;
    use crate::modules::{ClickCtx, Input};
    use crate::text::Text;
    let harness = sink_module(PathBuf::from("/nonexistent"));
    let view = harness.view();
    let font = FontArc::new(FontVec::try_from_vec(crate::testfont::build()).unwrap());
    let text = Text::new(font);
    let output = OutputView { name: None };
    let ctx = ClickCtx {
        output,
        x: 10,
        view: &view,
        text: &text,
        em: 50.0,
        padding: 10,
        span_width: 400,
        height: 60,
        scale: Scale::Integer(1),
    };
    let input = |trigger| harness.input(&Input { trigger, at: &ctx });
    assert_eq!(
        input(Trigger::Click),
        Some(Action::Module(ModuleAction::new("toggle-mute", None)))
    );
    assert_eq!(
        input(Trigger::ScrollUp),
        Some(Action::Module(ModuleAction::new("raise", None)))
    );
    assert_eq!(
        input(Trigger::ScrollDown),
        Some(Action::Module(ModuleAction::new("lower", None)))
    );
    assert_eq!(input(Trigger::RightClick), None);
    assert_eq!(input(Trigger::MiddleClick), None);
}

#[test]
fn invoke_names_and_arities() {
    let mut harness = sink_module(PathBuf::from("/nonexistent"));
    let output = OutputView { name: None };
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("frobnicate", None), 1),
        Err(InvokeError::Unknown)
    );
    assert_eq!(
        harness.invoke(
            &output,
            &ModuleAction {
                name: "raise".into(),
                arg: Some(5),
            },
            1
        ),
        Err(InvokeError::NoArg)
    );
}

// ---------------------------------------------------------------------------
// The parser, pinned to real captured packets.
// ---------------------------------------------------------------------------

fn unhex(hex: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks(2) {
        let text = std::str::from_utf8(pair).unwrap();
        out.push(u8::from_str_radix(text, 16).unwrap());
    }
    out
}

/// A whole frame around `body`: the descriptor, the command and the tag.
fn framed(cmd: u32, tag: u32, body: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(10 + body.len());
    payload.push(b'L');
    payload.extend_from_slice(&cmd.to_be_bytes());
    payload.push(b'L');
    payload.extend_from_slice(&tag.to_be_bytes());
    payload.extend_from_slice(body);
    let mut frame = vec![0u8; 20 + payload.len()];
    frame[0..4].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    frame[4..8].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
    frame[20..].copy_from_slice(&payload);
    frame
}

#[test]
fn percent_round_trips() {
    assert_eq!(proto::to_percent(0), 0);
    assert_eq!(proto::to_percent(VOL), 49);
    assert_eq!(proto::to_percent(65536), 100);
    assert_eq!(proto::to_percent(98304), 150);
    assert_eq!(proto::from_percent(0), 0);
    assert_eq!(proto::from_percent(49), VOL);
    assert_eq!(proto::from_percent(100), 65536);
    assert_eq!(proto::from_percent(150), 98304);
    assert_eq!(proto::from_percent(u32::MAX), u32::MAX);
}

#[test]
fn frames_round_trip_and_incomplete_ones_wait() {
    let mut writer = proto::Writer::new();
    writer.put_u32(0x83).unwrap();
    let payload = writer.done().to_vec();
    let mut out = [0u8; 64];
    let n = proto::encode_into(proto::CMD_SUBSCRIBE, 7, &payload, &mut out).unwrap();
    assert_eq!(proto::frame_at(&out[..10]), Ok(None));
    assert_eq!(proto::frame_at(&out[..n - 1]), Ok(None));
    let (frame, consumed) = proto::frame_at(&out[..n]).unwrap().expect("a frame");
    assert_eq!(consumed, n);
    assert_eq!((frame.cmd, frame.tag), (proto::CMD_SUBSCRIBE, 7));
    let mut reader = proto::Reader::new(frame.payload);
    assert_eq!(reader.get_u32(), Some(0x83));
}

#[test]
fn bad_lengths_and_markers_are_refused() {
    // Shorter than a command and a tag.
    let mut short = framed(2, 0, &[]);
    short[3] = 9;
    assert_eq!(proto::frame_at(&short), Err(()));
    // Past the cap.
    let mut big = framed(2, 0, &[]);
    big[0..4].copy_from_slice(&(proto::MAX_FRAME as u32 + 1).to_be_bytes());
    assert_eq!(proto::frame_at(&big), Err(()));
    // A command and tag that are not tagged values.
    let mut bad = framed(2, 0, &[0x77, 0, 0, 0, 1]);
    bad[20] = 0x77;
    assert_eq!(proto::frame_at(&bad), Err(()));
}

/// The AUTH answer, as pipewire-pulse sent it.
#[test]
fn golden_auth_reply() {
    let frame = framed(2, 0, &unhex("4c00000023"));
    let (parsed, _) = proto::frame_at(&frame).unwrap().expect("a frame");
    let mut reader = proto::Reader::new(parsed.payload);
    assert_eq!(reader.get_u32(), Some(35));
}

/// The server info, as pipewire-pulse sent it: the defaults are the
/// fifth and sixth strings.
#[test]
fn golden_server_info() {
    let payload = unhex(
        "7450756c7365417564696f20286f6e20506970655769726520312e362e382900\
         7431352e302e300074737465766500746e69786f7300\
         6105020000bb80\
         74617564696f5f6566666563742e6a3431332d636f6e766f6c76657200\
         746566666563745f6f75747075742e6a3431332d6d696300\
         4c2a6f556d6d020102",
    );
    let defaults = proto::parse_server_info(&payload).expect("server info");
    assert_eq!(defaults.sink.as_str(), SINK);
    assert_eq!(defaults.source.as_str(), SOURCE);
}

/// One sink, as pipewire-pulse sent it: index 74 at 32113, unmuted.
#[test]
fn golden_sink_info() {
    let payload = unhex(
        "4c0000004a\
         74617564696f5f6566666563742e6a3431332d636f6e766f6c76657200\
         744d6163426f6f6b20416972204a34313320537065616b65727300\
         6105020000bb80\
         6d020102\
         4cffffffff\
         760200007d7100007d71\
         30",
    );
    let device = proto::parse_device_info(&payload).expect("sink info");
    assert_eq!(device.index, INDEX);
    assert_eq!(device.name.as_str(), SINK);
    assert_eq!(device.description.as_str(), "MacBook Air J413 Speakers");
    assert_eq!(device.level(), VOL);
    assert!(!device.muted);
}

/// A sink change for index 74, as pipewire-pulse sent it.
#[test]
fn golden_subscribe_event() {
    let frame = framed(66, proto::INVALID_INDEX, &unhex("4c000000104c0000004a"));
    let (parsed, _) = proto::frame_at(&frame).unwrap().expect("a frame");
    assert_eq!((parsed.cmd, parsed.tag), (66, proto::INVALID_INDEX));
    let event = proto::parse_event(parsed.payload).expect("an event");
    assert_eq!(
        event,
        proto::Event {
            facility: 0,
            change: proto::Change::Changed,
            index: INDEX,
        }
    );
}

/// A refused request: command 0, errno 5 (`PA_ERR_NOENTITY`).
#[test]
fn golden_error() {
    let frame = framed(0, 2, &unhex("4c00000005"));
    let (parsed, _) = proto::frame_at(&frame).unwrap().expect("a frame");
    assert_eq!((parsed.cmd, parsed.tag), (0, 2));
    let mut reader = proto::Reader::new(parsed.payload);
    assert_eq!(reader.get_u32(), Some(5));
}

#[test]
fn cut_replies_are_malformed() {
    let full = unhex(
        "4c0000004a\
         74617564696f5f6566666563742e6a3431332d636f6e766f6c76657200\
         744d6163426f6f6b20416972204a34313320537065616b65727300\
         6105020000bb80\
         6d020102\
         4cffffffff\
         760200007d7100007d71\
         30",
    );
    // Every truncation but the whole one fails (the parser stops at mute,
    // so cuts past it still parse: only cut before the end).
    for end in [0, 1, 5, 10, 40, 80, full.len() - 20, full.len() - 1] {
        assert!(
            proto::parse_device_info(&full[..end]).is_none(),
            "cut at {end} parsed"
        );
    }
    assert!(proto::parse_device_info(&full).is_some());
    // A volume with no channels, or more than the most, is malformed.
    let mut writer = proto::Writer::new();
    writer.put_u32(1).unwrap();
    writer.put_str("s").unwrap();
    writer.put_str("d").unwrap();
    // spec + map + owner, hand-rolled around the bad volume.
    let mut payload = writer.done().to_vec();
    payload.extend_from_slice(&[b'a', 5, 2, 0, 0, 0xbb, 0x80, b'm', 2, 1, 2]);
    payload.push(b'L');
    payload.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
    let mut empty = payload.clone();
    empty.extend_from_slice(&[b'v', 0]);
    assert!(proto::parse_device_info(&empty).is_none());
    let mut many = payload.clone();
    many.extend_from_slice(&[b'v', 33, 0]);
    assert!(proto::parse_device_info(&many).is_none());
    // A string without its terminator is malformed.
    assert!(proto::parse_server_info(&[b't', b'a', b'b']).is_none());
}

#[test]
fn names_are_bounded_at_a_character() {
    let mut long = "é".repeat(200);
    long.push_str(" tail");
    let mut writer = proto::Writer::new();
    writer.put_u32(1).unwrap();
    assert!(writer.put_str(&long).is_none(), "a long name is refused");
    // Through the reader: cut, never split.
    let mut payload = vec![b't'];
    payload.extend_from_slice(long.as_bytes());
    payload.push(0);
    let mut reader = proto::Reader::new(&payload);
    let mut name = Bounded::empty();
    let text = reader.get_str(&mut name).expect("a string").expect("not null");
    assert!(text.len() <= proto::MAX_NAME);
    assert!(text.len() >= proto::MAX_NAME - 4);
    assert!(text.chars().all(|c| c == 'é'));
}

// ---------------------------------------------------------------------------
// The directory watch.
// ---------------------------------------------------------------------------

#[test]
fn scan_names_finds_native() {
    // One record for another name, one for native.
    let mut buf = Vec::new();
    for name in ["pulse", "native"] {
        buf.extend_from_slice(&1u32.to_ne_bytes());
        buf.extend_from_slice(&0x100u32.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes());
        buf.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.push(0);
    }
    // Big-endian length: not a record, but the scan is conservative.
    assert!(super::scan_names(&buf, b"native"));
    // Cut mid-record: malformed, so conservative too.
    assert!(super::scan_names(&buf[..30], b"native"));
    assert!(!super::scan_names(&[], b"native"));
    assert!(super::scan_names(&[0u8; 10], b"native"));
}

// ---------------------------------------------------------------------------
// The built-in icons.
// ---------------------------------------------------------------------------

#[test]
fn the_level_icons_parse_and_grow_with_the_level() {
    use super::icons::{PATHS, index};
    assert_eq!(PATHS[index(true, 100)], PATHS[0]);
    assert_eq!(PATHS[index(false, 0)], PATHS[1]);
    assert_eq!(PATHS[index(false, 33)], PATHS[1]);
    assert_eq!(PATHS[index(false, 34)], PATHS[2]);
    assert_eq!(PATHS[index(false, 66)], PATHS[2]);
    assert_eq!(PATHS[index(false, 67)], PATHS[3]);
    assert_eq!(PATHS[index(false, 150)], PATHS[3]);
    assert_eq!(
        [index(true, 0), index(false, 0), index(false, 34), index(false, 67)],
        [0, 1, 2, 3]
    );
    let mut segs = Vec::new();
    for path in PATHS {
        let vector =
            Vector::parse(path, crate::icon::path::ViewBox::default()).expect("a level icon");
        segs.push(vector.segs().len());
        // Ink: a 24-pixel fill with real coverage somewhere.
        let mut raster = Rasterizer::default();
        let mut out = vec![0u8; 24 * 24];
        raster.fill(&vector, 24, &mut out);
        let ink: usize = out.iter().map(|c| *c as usize).sum();
        assert!(ink > 24 * 10, "no ink in {path}");
        // The speaker alone reaches no further right than x=14; the
        // waves and the cross do.
        let right: usize = out
            .chunks(24)
            .flat_map(|row| row[15..].iter())
            .map(|c| *c as usize)
            .sum();
        if path == PATHS[1] {
            assert_eq!(right, 0, "the speaker reaches past x=15");
        } else {
            assert!(right > 24, "no ink right of x=15 in {path}");
        }
    }
    assert!(segs[1] < segs[2], "a wave adds segments: {segs:?}");
    assert!(segs[2] < segs[3], "a second wave adds segments: {segs:?}");
    assert!(segs[0] > segs[1], "a cross adds segments: {segs:?}");
}
