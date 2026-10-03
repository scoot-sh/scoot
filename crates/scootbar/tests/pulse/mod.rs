//! A tiny live PulseAudio-protocol server for the popup's end-to-end test:
//! one default sink whose level and mute the test can read and move, which
//! answers the module's whole conversation (the same frames the module's
//! own `fake.rs` scripts one at a time) and pushes a subscription event when
//! the test changes the level, as a real server does when another client
//! does. The bar finds it through `PULSE_SERVER`.
//!
//! The wire code is the module's own `proto.rs`, compiled into this test by
//! path: the encoder under test is the one the server answers with.

#![allow(dead_code)]

#[path = "../../src/modules/volume/proto.rs"]
pub mod proto;

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SINK: &str = "test_sink";
const DESC: &str = "Test Speakers";
const INDEX: u32 = 3;

/// What the server holds and what it was asked.
#[derive(Debug)]
pub struct Shared {
    /// The level, raw (`proto::NORM` is full scale), both channels.
    pub volume: u32,
    pub muted: bool,
    /// Every `SET_SINK_VOLUME` it was sent, raw, in order.
    pub sets: Vec<u32>,
    pub mutes: Vec<bool>,
    /// An event owed to the client: the level changed under it.
    notify: bool,
}

pub struct Server {
    pub dir: PathBuf,
    pub shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    /// Listens on `dir/native`; `PULSE_SERVER` for the bar is
    /// [`Server::address`].
    pub fn start(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        let socket = dir.join("native");
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let shared = Arc::new(Mutex::new(Shared {
            volume: proto::from_percent(49),
            muted: false,
            sets: Vec::new(),
            mutes: Vec::new(),
            notify: false,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (shared.clone(), stop.clone());
            std::thread::spawn(move || serve(listener, shared, stop))
        };
        Self {
            dir: dir.to_owned(),
            shared,
            stop,
            thread: Some(thread),
        }
    }

    pub fn address(&self) -> String {
        format!("unix:{}", self.dir.join("native").display())
    }

    /// Another client changed the level: store it and tell the module.
    pub fn set_externally(&self, percent: u32) {
        let mut shared = self.shared.lock().unwrap();
        shared.volume = proto::from_percent(percent);
        shared.notify = true;
    }

    pub fn volume_percent(&self) -> u32 {
        proto::to_percent(self.shared.lock().unwrap().volume)
    }

    pub fn sets_percent(&self) -> Vec<u32> {
        self.shared
            .lock()
            .unwrap()
            .sets
            .iter()
            .map(|raw| proto::to_percent(*raw))
            .collect()
    }

    pub fn mutes(&self) -> Vec<bool> {
        self.shared.lock().unwrap().mutes.clone()
    }

    /// Goes away as a crashed server does: the listener and the
    /// connections close, the socket file stays gone.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(self.dir.join("native"));
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(listener: UnixListener, shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => connection(stream, &shared, &stop),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn connection(mut stream: UnixStream, shared: &Mutex<Shared>, stop: &AtomicBool) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    while !stop.load(Ordering::SeqCst) {
        // An event owed, before the next read.
        let owed = std::mem::take(&mut shared.lock().unwrap().notify);
        if owed {
            let mut writer = proto::Writer::new();
            writer.put_u32(0x10).unwrap();
            writer.put_u32(INDEX).unwrap();
            send(
                &mut stream,
                proto::CMD_SUBSCRIBE_EVENT,
                proto::INVALID_INDEX,
                writer.done(),
            );
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return,
        }
        while let Ok(Some((frame, used))) = proto::frame_at(&buffer) {
            let (cmd, tag, payload) = (frame.cmd, frame.tag, frame.payload.to_vec());
            buffer.drain(..used);
            answer(&mut stream, shared, cmd, tag, &payload);
        }
    }
}

fn send(stream: &mut UnixStream, cmd: u32, tag: u32, payload: &[u8]) {
    let mut out = vec![0u8; 30 + payload.len()];
    let n = proto::encode_into(cmd, tag, payload, &mut out).unwrap();
    let _ = stream.write_all(&out[..n]);
}

fn put_str(into: &mut Vec<u8>, text: &str) {
    into.push(b't');
    into.extend_from_slice(text.as_bytes());
    into.push(0);
}

fn answer(stream: &mut UnixStream, shared: &Mutex<Shared>, cmd: u32, tag: u32, payload: &[u8]) {
    match cmd {
        proto::CMD_AUTH => send(stream, proto::CMD_REPLY, tag, &[b'L', 0, 0, 0, 35]),
        proto::CMD_SET_CLIENT_NAME => {
            let mut writer = proto::Writer::new();
            writer.put_u32(7).unwrap();
            send(stream, proto::CMD_REPLY, tag, writer.done());
        }
        proto::CMD_GET_SERVER_INFO => {
            let mut out = Vec::new();
            for text in ["PipeAudio", "17.0", "tester", "host"] {
                put_str(&mut out, text);
            }
            out.push(b'a');
            out.extend_from_slice(&[5, 2, 0, 0, 0xbb, 0x80]);
            put_str(&mut out, SINK);
            put_str(&mut out, "test_source");
            out.push(b'L');
            out.extend_from_slice(&1u32.to_be_bytes());
            out.push(b'm');
            out.extend_from_slice(&[2, 1, 2]);
            send(stream, proto::CMD_REPLY, tag, &out);
        }
        proto::CMD_SUBSCRIBE => send(stream, proto::CMD_REPLY, tag, &[]),
        proto::CMD_GET_SINK_INFO => {
            let (volume, muted) = {
                let shared = shared.lock().unwrap();
                (shared.volume, shared.muted)
            };
            let mut out = vec![b'L'];
            out.extend_from_slice(&INDEX.to_be_bytes());
            put_str(&mut out, SINK);
            put_str(&mut out, DESC);
            out.push(b'a');
            out.extend_from_slice(&[7, 2, 0, 0, 0xbb, 0x80]);
            out.push(b'm');
            out.extend_from_slice(&[2, 1, 2]);
            out.push(b'L');
            out.extend_from_slice(&0xffff_ffffu32.to_be_bytes());
            out.push(b'v');
            out.push(2);
            for _ in 0..2 {
                out.extend_from_slice(&volume.to_be_bytes());
            }
            out.push(if muted { b'1' } else { b'0' });
            send(stream, proto::CMD_REPLY, tag, &out);
        }
        proto::CMD_SET_SINK_VOLUME => {
            let mut reader = proto::Reader::new(payload);
            let _ = reader.get_u32();
            let mut name = proto::Bounded::empty();
            let _ = reader.get_str(&mut name);
            let mut volumes = [0; proto::MAX_CHANNELS];
            if reader.get_cvolume(&mut volumes) == Some(2) {
                let mut shared = shared.lock().unwrap();
                shared.volume = volumes[0];
                shared.sets.push(volumes[0]);
            }
            send(stream, proto::CMD_REPLY, tag, &[]);
        }
        proto::CMD_SET_SINK_MUTE => {
            let mut reader = proto::Reader::new(payload);
            let _ = reader.get_u32();
            let mut name = proto::Bounded::empty();
            let _ = reader.get_str(&mut name);
            if let Some(muted) = reader.get_bool() {
                let mut shared = shared.lock().unwrap();
                shared.muted = muted;
                shared.mutes.push(muted);
            }
            send(stream, proto::CMD_REPLY, tag, &[]);
        }
        _ => send(stream, proto::CMD_REPLY, tag, &[]),
    }
}
