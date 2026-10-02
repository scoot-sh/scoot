//! The volume module's scripted PulseAudio server and the helpers that
//! drive the module against it, shared by the sink tests (`tests.rs`) and
//! the microphone variant's (`microphone/tests.rs`). Test-only: compiled
//! under `#[cfg(test)]`, never into the bar.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use super::proto::{self, Bounded, Kind};
use super::{Settings, start_with};
use crate::modules::Update;
use crate::modules::harness::Harness;

/// A scripted PulseAudio server: one connection, exact frames, nothing
/// more. Blocking, with a timeout on every read, so a module that stops
/// talking fails the test instead of hanging it.
pub struct Fake {
    dir: PathBuf,
    pub sock: PathBuf,
    listener: Option<UnixListener>,
    /// Killed, not dropped: `serve` owns the directory now, so the
    /// cleanup below must not take it.
    disowned: bool,
}

pub struct Conn {
    stream: UnixStream,
}

impl Fake {
    pub fn bind() -> Self {
        let dir =
            std::env::temp_dir().join(format!("scootbar-volume-{}-{}", std::process::id(), nano()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("native");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        listener.set_nonblocking(false).unwrap();
        Self {
            dir,
            sock,
            listener: Some(listener),
            disowned: false,
        }
    }

    pub fn accept(&self) -> Conn {
        let (stream, _) = self.listener.as_ref().unwrap().accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Conn { stream }
    }

    /// How many connections arrived that nobody accepted: what a module
    /// that re-probes a refusing server would have piled up.
    pub fn unaccepted(&self) -> usize {
        let listener = self.listener.as_ref().unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut count = 0;
        while listener.accept().is_ok() {
            count += 1;
        }
        listener.set_nonblocking(false).unwrap();
        count
    }

    /// Serves `sock` again: the restart test's server coming back on the
    /// same path (the directory is remade: dropping the old server removed
    /// it).
    pub fn serve(sock: &PathBuf) -> Self {
        let dir = sock
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(sock);
        let listener = UnixListener::bind(sock).unwrap();
        listener.set_nonblocking(false).unwrap();
        Self {
            dir,
            sock: sock.clone(),
            listener: Some(listener),
            disowned: false,
        }
    }

    /// Kills the server but keeps the directory: the listener is closed
    /// and the stale socket file removed, as a crash or restart leaves
    /// the runtime directory behind. Returns the path to serve again.
    pub fn kill(mut self) -> PathBuf {
        drop(self.listener.take());
        let _ = std::fs::remove_file(&self.sock);
        self.disowned = true;
        self.sock.clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        if self.disowned {
            return;
        }
        let _ = std::fs::remove_file(&self.sock);
        let _ = std::fs::remove_dir(&self.dir);
    }
}

pub fn nano() -> u64 {
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
    pub fn frame(&mut self) -> (u32, u32, Vec<u8>) {
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

    pub fn expect(&mut self, cmd: u32) -> (u32, Vec<u8>) {
        let (got, tag, payload) = self.frame();
        assert_eq!(got, cmd, "expected command {cmd}, got {got}");
        (tag, payload)
    }

    pub fn reply(&mut self, tag: u32, payload: &[u8]) {
        let mut out = vec![0u8; 20 + 10 + payload.len()];
        let n = proto::encode_into(proto::CMD_REPLY, tag, payload, &mut out).unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    pub fn ack(&mut self, tag: u32) {
        self.reply(tag, &[]);
    }

    /// A `SET_CLIENT_NAME` answer: the server's client index, a u32 the
    /// module ignores (measured: pipewire-pulse answers 184, 185, ... per
    /// connection, never the empty ack the other commands send).
    pub fn ack_name(&mut self, tag: u32) {
        let mut writer = proto::Writer::new();
        writer.put_u32(7).unwrap();
        let payload = writer.done().to_vec();
        self.reply(tag, &payload);
    }

    pub fn error(&mut self, tag: u32, errno: u32) {
        let mut writer = proto::Writer::new();
        writer.put_u32(errno).unwrap();
        let payload = writer.done().to_vec();
        let mut out = vec![0u8; 64];
        let n = proto::encode_into(0, tag, &payload, &mut out).unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    pub fn event(&mut self, change: u32, index: u32) {
        let mut writer = proto::Writer::new();
        writer.put_u32(change).unwrap();
        writer.put_u32(index).unwrap();
        let payload = writer.done().to_vec();
        let mut out = vec![0u8; 64];
        let n = proto::encode_into(
            proto::CMD_SUBSCRIBE_EVENT,
            proto::INVALID_INDEX,
            &payload,
            &mut out,
        )
        .unwrap();
        self.stream.write_all(&out[..n]).unwrap();
    }

    pub fn reply_server_info(&mut self, tag: u32, sink: &str, source: &str) {
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
    pub fn reply_device(
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

    /// Raw bytes, for the malformed-input tests: what no `reply` sends.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    /// Asserts nothing arrives within `ms`: the module sent nothing. A
    /// closed peer reads as `Ok(0)`, which is silence too (the drop tests
    /// assert on it); only a byte is a frame.
    pub fn quiet(&mut self, ms: u64) {
        self.stream
            .set_read_timeout(Some(Duration::from_millis(ms)))
            .unwrap();
        let mut byte = [0u8; 1];
        let read = self.stream.read(&mut byte);
        self.stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        assert!(
            matches!(read, Err(_) | Ok(0)),
            "expected silence, got a frame"
        );
    }
}

pub fn push_str(into: &mut Vec<u8>, text: &str) {
    into.push(b't');
    into.extend_from_slice(text.as_bytes());
    into.push(0);
}

// ---------------------------------------------------------------------------
// Driving the module.
// ---------------------------------------------------------------------------

pub const SINK: &str = "audio_effect.j413-convolver";
pub const SOURCE: &str = "effect_output.j413-mic";
pub const DESC: &str = "Convolver";
pub const INDEX: u32 = 74;
pub const VOL: u32 = 32113;
pub const STEP: u32 = 3277;

pub fn sink_module(sock: PathBuf) -> Harness {
    Harness::new(start_with(&Settings::default(), Kind::Sink, sock))
}

pub fn mic_module(sock: PathBuf) -> Harness {
    Harness::new(start_with(&Settings::default(), Kind::Source, sock))
}

/// One turn of the loop, expecting progress: the module asked, answered or
/// changed, not silence.
pub fn wait(harness: &mut Harness) -> Update {
    harness
        .wait(Duration::from_secs(5))
        .expect("the module went quiet")
}

/// The handshake to a shown level: AUTH, name, server info, subscribe and
/// the default device, at `VOL`, unmuted. `default` is the name the server
/// info gives for this kind, `device` the name the device query answers.
pub fn handshake(conn: &mut Conn, harness: &mut Harness, kind: Kind, default: &str, device: &str) {
    let (cmd, tag, _) = conn.frame();
    assert_eq!(cmd, proto::CMD_AUTH);
    conn.reply(tag, &[b'L', 0, 0, 0, 35]);
    let _ = wait(harness);
    let (tag, _) = conn.expect(proto::CMD_SET_CLIENT_NAME);
    let _ = cmd;
    conn.ack_name(tag);
    let _ = wait(harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, SINK, SOURCE);
    let _ = wait(harness);
    let (tag, payload) = conn.expect(proto::CMD_SUBSCRIBE);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(kind.subscribe_mask()));
    conn.ack(tag);
    let _ = wait(harness);
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
/// `device` is the name the device query answers (the sink's in the sink
/// tests, the source's in the microphone's).
pub fn answer_set(
    conn: &mut Conn,
    harness: &mut Harness,
    kind: Kind,
    device: &str,
    vols: &[u32],
    muted: bool,
) {
    let (ack, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(ack, SINK, SOURCE);
    let _ = wait(harness);
    let (tag, _) = conn.expect(kind.get_info());
    conn.reply_device(tag, INDEX, device, DESC, vols, muted);
    let _ = wait(harness);
}

pub fn view_text(harness: &Harness) -> String {
    harness.view().text().to_owned()
}

/// A fake around an already-bound listener, for the late server: the
/// directory exists but nothing listens yet, so the module starts
/// watching, and `listen` brings the server up after.
pub struct Pending {
    dir: PathBuf,
    sock: PathBuf,
}

pub fn pending() -> Pending {
    let dir = std::env::temp_dir().join(format!("scootbar-volume-late-{}", nano()));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("native");
    Pending { dir, sock }
}

impl Pending {
    pub fn sock(&self) -> &PathBuf {
        &self.sock
    }

    pub fn listen(self) -> Fake {
        let _ = std::fs::remove_file(&self.sock);
        let listener = UnixListener::bind(&self.sock).unwrap();
        listener.set_nonblocking(false).unwrap();
        Fake {
            dir: self.dir,
            sock: self.sock,
            listener: Some(listener),
            disowned: false,
        }
    }
}
