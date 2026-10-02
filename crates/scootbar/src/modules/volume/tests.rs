//! The volume module through the harness: the waiting state with no
//! server, and the whole protocol against a fake server on a unix socket
//! in a temp dir. The fake speaks what pipewire-pulse was measured to
//! speak (see the module docs), with scripted replies, so the handshake,
//! every event, every error and a scroll flood are deterministic. Real
//! packets captured from the Asahi machine pin the parser as golden
//! vectors.

use std::path::PathBuf;
use std::time::Duration;

use rustix::event::PollFlags;

use super::fake::*;
use super::proto::{self, Bounded, Kind};
use super::{Settings, start_with};
use crate::action::{Action, ModuleAction, Trigger};
use crate::icon::path::Vector;
use crate::icon::raster::Rasterizer;
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, OutputView, Update};

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
    // Started as the bar starts it, against a path nothing listens on.
    let mut harness = Harness::new(start_with(
        &Settings::default(),
        Kind::Sink,
        dir.join("native"),
    ));
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
    assert_eq!(
        super::socket_path_for(Some(OsStr::from_bytes(b""))),
        fallback
    );
    assert_eq!(
        super::socket_path_for(Some(OsStr::from_bytes(b"unix:"))),
        fallback
    );
}

#[test]
fn against_a_real_server_when_one_is_there() {
    let sock = super::socket_path();
    if !sock.exists() {
        eprintln!("no sound server at {}: skipping", sock.display());
        return;
    }
    // Read-only: the handshake runs itself and the level shows, but no
    // set is ever sent (this may run on the developer's live session).
    let mut harness = sink_module(sock);
    let mut shown = false;
    for _ in 0..20 {
        if !harness.view().is_empty() {
            shown = true;
            break;
        }
        let _ = harness.wait(Duration::from_secs(1));
    }
    assert!(shown, "a server is there but the level never showed");
    let text = view_text(&harness);
    assert!(text.ends_with('%'), "unexpected view {text:?}");
    let value = harness.value_on(None).expect("a shown level has a value");
    assert!(
        value.get("volume").is_some()
            && value.get("muted").is_some()
            && value.get("sink").is_some(),
        "unexpected value {value}"
    );
}

/// A real set round trip on a real server: gated, because it briefly
/// changes the developer's own level (up one step and back). Measures the
/// scroll-to-shown latency the ticket's done-when asks for.
#[test]
fn live_raise_and_lower_round_trip() {
    if std::env::var_os("SCOOTBAR_TEST_LIVE_AUDIO").is_none() {
        eprintln!("SCOOTBAR_TEST_LIVE_AUDIO unset: skipping");
        return;
    }
    let sock = super::socket_path();
    assert!(sock.exists(), "no sound server at {}", sock.display());
    let mut harness = sink_module(sock);
    let mut shown = false;
    for _ in 0..20 {
        if !harness.view().is_empty() {
            shown = true;
            break;
        }
        let _ = harness.wait(Duration::from_secs(1));
    }
    assert!(shown, "a server is there but the level never showed");
    let before = view_text(&harness);
    let output = OutputView { name: None };
    // Up one step, then back: the asserts run after the restore, so a
    // failure never leaves the level moved.
    let start = std::time::Instant::now();
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    let mut raised = false;
    for _ in 0..100 {
        if view_text(&harness) != before {
            raised = true;
            break;
        }
        let _ = harness.wait(Duration::from_millis(50));
    }
    let latency = start.elapsed();
    assert!(raised, "a raise never showed");
    eprintln!("raise showed in {latency:?}");
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("lower", None), 1),
        Ok(Update::Unchanged)
    );
    let mut restored = false;
    for _ in 0..100 {
        if view_text(&harness) == before {
            restored = true;
            break;
        }
        let _ = harness.wait(Duration::from_millis(50));
    }
    assert!(restored, "the level never came back");
    assert_eq!(view_text(&harness), before);
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
    // One source, whatever the state: the socket while up, the directory
    // watch while down. No timer, ever.
    assert_eq!(harness.source_count(), 1);
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
    // One absolute set from the shown level: 32113 + 3277, by name (an
    // index with a name is refused: measured against pipewire-pulse).
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut name = Bounded::empty();
    assert_eq!(
        reader.get_str(&mut name).map(|s| s.map(str::to_owned)),
        Some(Some(SINK.to_owned()))
    );
    let mut vols = [0; proto::MAX_CHANNELS];
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + STEP, VOL + STEP]);
    // A second raise before the answer coalesces behind it: nothing
    // goes out while a set is in flight.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("raise", None), 1),
        Ok(Update::Unchanged)
    );
    conn.quiet(200);
    // Its answer sends the one queued target, the latest absolute level.
    conn.ack(tag);
    let _ = wait(&mut harness);
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let _ = reader.get_str(&mut name);
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + 2 * STEP, VOL + 2 * STEP]);
    conn.ack(tag);
    let _ = wait(&mut harness);
    // That answer re-reads: shown is what the server says.
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[VOL + 2 * STEP, VOL + 2 * STEP],
        false,
    );
    assert_eq!(view_text(&harness), "59%");
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
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[65536, 65536],
        false,
    );
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
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[VOL - 2 * STEP, VOL - 2 * STEP],
        false,
    );
    assert_eq!(view_text(&harness), "39%");
    // Mute: one byte, then the class, the icon slot and the tooltip.
    assert_eq!(
        harness.invoke(&output, &ModuleAction::new("toggle-mute", None), 1),
        Ok(Update::Unchanged)
    );
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_MUTE);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    assert_eq!(reader.get_bool(), Some(true));
    conn.ack(tag);
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[VOL - 2 * STEP, VOL - 2 * STEP],
        true,
    );
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
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[40000, 40000],
        false,
    );
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
    let _ = wait(&mut harness);
    // New default: the device query names it.
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "other-sink", SOURCE);
    let _ = wait(&mut harness);
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
    // Cleared at the event itself, before the re-read answers.
    assert_eq!(wait(&mut harness), Update::Changed);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "", SOURCE);
    // Still no default, so no device is asked for: one quiet turn, then
    // silence.
    let _ = wait(&mut harness);
    assert_eq!(harness.wait(Duration::from_millis(300)), None);
}

#[test]
fn a_device_gone_before_its_read_clears_the_view() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    // The default changes, but its device is gone before the read.
    conn.event(0x17, proto::INVALID_INDEX);
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "other-sink", SOURCE);
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(Kind::Sink.get_info());
    conn.error(tag, 5);
    assert_eq!(wait(&mut harness), Update::Changed);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // Nothing is asked again unprompted.
    conn.quiet(300);
}

#[test]
fn an_error_for_no_device_is_no_change() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    // The tracked device is removed (view cleared at the event), and the
    // re-read's device query errors too: already nothing shown, so no
    // change.
    conn.event(0x20, INDEX);
    assert_eq!(wait(&mut harness), Update::Changed);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, SINK, SOURCE);
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(Kind::Sink.get_info());
    conn.error(tag, 5);
    assert_eq!(wait(&mut harness), Update::Unchanged);
    assert!(harness.view().is_empty());
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
    // A full header with a length no server may send (four bytes alone
    // would only wait for the rest of the header).
    conn.write_raw(&[0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
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
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_SET_CLIENT_NAME);
    conn.ack_name(tag);
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_GET_SERVER_INFO);
    conn.reply_server_info(tag, "", SOURCE);
    let _ = wait(&mut harness);
    let (tag, _) = conn.expect(proto::CMD_SUBSCRIBE);
    conn.ack(tag);
    let _ = wait(&mut harness);
    // Subscribed with still no default, so no device is asked for: the
    // module waits for an event instead of asking again unprompted.
    conn.quiet(300);
    assert!(harness.view().is_empty());
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
    let mut harness = sink_module(pending.sock().clone());
    assert!(harness.view().is_empty());
    // The server appears after the module started watching.
    let fake = pending.listen();
    assert_eq!(
        harness.wait(Duration::from_secs(5)),
        Some(Update::Unchanged)
    );
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
}

/// Turns of the loop the module takes over `ms`: wakeups, which must be
/// zero with nothing going on.
fn wakeups_over(harness: &mut Harness, ms: u64) -> usize {
    let end = std::time::Instant::now() + Duration::from_millis(ms);
    let mut wakes = 0;
    loop {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return wakes;
        }
        if harness.wait(left).is_some() {
            wakes += 1;
        }
    }
}

#[test]
fn a_drop_with_the_socket_still_present_reconnects() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
    // The connection drops but the listener and its path stay, so no
    // CREATE or MOVED_TO ever names the socket.
    drop(conn);
    assert_eq!(wait(&mut harness), Update::Changed);
    assert!(harness.view().is_empty());
    // The drop probed the present socket: a new connection is already
    // waiting, and the handshake runs on it.
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
    assert_eq!(wakeups_over(&mut harness, 500), 0);
}

#[test]
fn a_standing_refusal_is_not_a_connect_loop() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    // Refused by an error to AUTH, as a wrong cookie is.
    let mut conn = fake.accept();
    let (tag, _) = conn.expect(proto::CMD_AUTH);
    conn.error(tag, 2);
    drop(conn);
    let _ = harness.wait(Duration::from_secs(5));
    // And again refused by a plain close: both waits stay quiet.
    assert_eq!(wakeups_over(&mut harness, 500), 0);
    assert_eq!(fake.unaccepted(), 0, "a refusal was probed again");
    assert!(harness.view().is_empty());
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn a_server_that_closes_during_the_handshake_is_not_a_connect_loop() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    drop(fake.accept());
    let _ = harness.wait(Duration::from_secs(5));
    assert_eq!(wakeups_over(&mut harness, 500), 0);
    assert_eq!(fake.unaccepted(), 0, "a refusal was probed again");
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn a_refusal_ends_at_the_next_event_on_the_directory() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    let (tag, _) = conn.expect(proto::CMD_AUTH);
    conn.error(tag, 2);
    drop(conn);
    let _ = harness.wait(Duration::from_secs(5));
    assert_eq!(wakeups_over(&mut harness, 200), 0);
    // The server restarts with a socket that now accepts the cookie.
    let sock = fake.kill();
    let fake = Fake::serve(&sock);
    let _ = harness.wait(Duration::from_secs(5));
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
}

#[test]
fn a_server_restart_clears_then_reshows() {
    let fake = Fake::bind();
    let sock = fake.sock.clone();
    let mut harness = sink_module(sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
    // The server goes away: closing both ends reads as a hang-up, and a
    // dead server's last level is not shown as if live. The directory
    // stays, as a real restart keeps the runtime directory.
    drop(conn);
    let sock = fake.kill();
    assert_eq!(wait(&mut harness), Update::Changed);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // ... and comes back on the same path: the directory watch finds it
    // without polling. The module connects on its next turn, so wait for
    // that before accepting.
    let fake = Fake::serve(&sock);
    let _ = harness.wait(Duration::from_secs(5));
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    assert_eq!(view_text(&harness), "49%");
}

#[test]
fn a_scroll_flood_sends_one_set_then_the_latest() {
    let fake = Fake::bind();
    let mut harness = sink_module(fake.sock.clone());
    let mut conn = fake.accept();
    handshake(&mut conn, &mut harness, Kind::Sink, SINK, SINK);
    // Fifty notches at a touchpad's rate, no answers: one frame goes
    // out at once (the first set, for latency), the rest coalesce into
    // the one queued behind it: an absolute level rather than fifty
    // accumulated steps, clamped at full scale.
    let output = OutputView { name: None };
    for _ in 0..50 {
        assert_eq!(
            harness.invoke(&output, &ModuleAction::new("raise", None), 1),
            Ok(Update::Unchanged)
        );
    }
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    assert_eq!(reader.get_u32(), Some(proto::INVALID_INDEX));
    let mut vols = [0; proto::MAX_CHANNELS];
    let mut name = Bounded::empty();
    let _ = reader.get_str(&mut name);
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [VOL + STEP, VOL + STEP]);
    // Its answer sends the one queued target, the latest absolute level.
    conn.ack(tag);
    let _ = wait(&mut harness);
    let (tag, payload) = conn.expect(proto::CMD_SET_SINK_VOLUME);
    let mut reader = proto::Reader::new(&payload);
    let _ = reader.get_u32();
    let _ = reader.get_str(&mut name);
    assert_eq!(reader.get_cvolume(&mut vols), Some(2));
    assert_eq!(vols[..2], [65536, 65536]);
    // Nothing more is queued behind it.
    conn.quiet(200);
    conn.ack(tag);
    let _ = wait(&mut harness);
    answer_set(
        &mut conn,
        &mut harness,
        Kind::Sink,
        SINK,
        &[65536, 65536],
        false,
    );
    assert_eq!(view_text(&harness), "100%");
}

#[test]
fn click_and_scroll_have_defaults() {
    use crate::density::Scale;
    use crate::modules::{ClickCtx, Input};
    use crate::text::Text;
    use ab_glyph::{FontArc, FontVec};
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
    assert!(proto::parse_server_info(b"tab").is_none());
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
    let text = reader
        .get_str(&mut name)
        .expect("a string")
        .expect("not null");
    assert!(text.len() <= proto::MAX_NAME);
    assert!(text.len() >= proto::MAX_NAME - 4);
    assert!(text.chars().all(|c| c == 'é'));
}

// ---------------------------------------------------------------------------
// The directory watch.
// ---------------------------------------------------------------------------

/// One inotify record as the kernel writes it: a native-endian header
/// (watch, mask, cookie, name length) and the NUL-padded name.
fn inotify_record(name: &str) -> Vec<u8> {
    let mut rec = Vec::new();
    rec.extend_from_slice(&1u32.to_ne_bytes());
    rec.extend_from_slice(&0x100u32.to_ne_bytes());
    rec.extend_from_slice(&0u32.to_ne_bytes());
    rec.extend_from_slice(&(name.len() as u32 + 1).to_ne_bytes());
    rec.extend_from_slice(name.as_bytes());
    rec.push(0);
    rec
}

#[test]
fn scan_names_compares_the_names() {
    // Well-formed records only: the answer is the name comparison, not
    // the conservative malformed-tail path.
    let other = inotify_record("pulse");
    let native = inotify_record("native");
    assert!(!super::scan_names(&other, b"native"));
    assert!(super::scan_names(&native, b"native"));
    assert!(!super::scan_names(&inotify_record("nativer"), b"native"));
    assert!(!super::scan_names(&inotify_record("nativ"), b"native"));
    let both = [other.clone(), native.clone()].concat();
    assert!(super::scan_names(&both, b"native"));
    let both = [native, other.clone()].concat();
    assert!(super::scan_names(&both, b"native"));
    // Nothing read is nothing named.
    assert!(!super::scan_names(&[], b"native"));
    // A name field longer than its text (the kernel pads to a multiple
    // of the header's alignment) ends at the first NUL.
    let mut padded = Vec::new();
    padded.extend_from_slice(&[0u8; 12]);
    padded.extend_from_slice(&16u32.to_ne_bytes());
    padded.extend_from_slice(b"native\0\0\0\0\0\0\0\0\0\0");
    assert!(super::scan_names(&padded, b"native"));
    padded.splice(16..22, *b"pulse\0");
    assert!(!super::scan_names(&padded, b"native"));
}

#[test]
fn scan_names_treats_a_malformed_tail_as_interesting() {
    let other = inotify_record("pulse");
    // Cut mid-header and mid-name.
    assert!(super::scan_names(&other[..10], b"native"));
    assert!(super::scan_names(&other[..other.len() - 1], b"native"));
    // A garbage length far past the bytes read (the old fixture's
    // accident, now on purpose).
    let mut huge = other.clone();
    huge[12..16].copy_from_slice(&u32::MAX.to_ne_bytes());
    assert!(super::scan_names(&huge, b"native"));
    // A well-formed record for another name, then a truncated header.
    let tail = [other.clone(), vec![0u8; 7]].concat();
    assert!(super::scan_names(&tail, b"native"));
    // ... and then a record cut mid-name.
    let cut = [other.clone(), inotify_record("native")[..20].to_vec()].concat();
    assert!(super::scan_names(&cut, b"native"));
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
        [
            index(true, 0),
            index(false, 0),
            index(false, 34),
            index(false, 67)
        ],
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
