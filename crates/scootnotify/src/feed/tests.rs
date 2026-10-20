//! The feed's contract: the exact payload shape, change-only pushes,
//! the owed watch, and the once-per-run guidance.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bus::hints::{Hints, Urgency};
use crate::bus::notify::Incoming;
use crate::config::Config;
use crate::model::dnd::Dnd;
use crate::model::store::{IdFile, Store};

use super::{Feed, MODULE_ID, State, parse_reply, payload, scan_names, set_line};

fn now() -> Instant {
    Instant::now()
}

fn incoming(sender: &str, app: &str) -> Incoming {
    Incoming {
        app_name: app.to_owned(),
        replaces_id: 0,
        app_icon: String::new(),
        summary: "s".to_owned(),
        body: "b".to_owned(),
        actions: Vec::new(),
        hints: Hints::default(),
        expire_timeout: -1,
        sender: sender.to_owned(),
    }
}

fn urgent(sender: &str, app: &str) -> Incoming {
    let mut in_ = incoming(sender, app);
    in_.hints.urgency = Urgency::Critical;
    in_
}

fn store() -> Store {
    Store::new(Config::default(), IdFile::open(None))
}

fn shown(store: &mut Store, sender: &str, app: &str) -> u32 {
    let start = now();
    let (arrival, _) = store.notify(incoming(sender, app), start, 0);
    let crate::model::store::Arrival::Shown { id, .. } = arrival else {
        panic!("a fresh card shows");
    };
    store.presented(id, start, 0);
    id
}

fn state(dnd: Dnd, unread: usize, critical: bool) -> State {
    State {
        dnd,
        unread,
        critical,
    }
}

fn get<'v>(value: &'v serde_json::Value, key: &str) -> &'v serde_json::Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("payload has `{key}`: {value}"))
}

#[test]
fn idle_payload_is_just_text() {
    let value = payload(state(Dnd::Off, 0, false), "", "");
    assert_eq!(value, serde_json::json!({"text": ""}));
}

#[test]
fn unread_payload_names_normal_and_counts() {
    let value = payload(state(Dnd::Off, 3, false), "", "");
    assert_eq!(get(&value, "text"), &serde_json::json!("3"));
    assert_eq!(get(&value, "class"), &serde_json::json!("normal"));
    assert_eq!(
        get(&value, "tooltip"),
        &serde_json::json!("3 notifications -- click to hold them with do-not-disturb")
    );
    assert!(
        value.get("icon").is_none(),
        "no icon key when empty: {value}"
    );
}

#[test]
fn critical_unread_is_urgent() {
    let value = payload(state(Dnd::Off, 1, true), "", "");
    assert_eq!(get(&value, "class"), &serde_json::json!("urgent"));
    assert_eq!(get(&value, "text"), &serde_json::json!("1"));
}

#[test]
fn dnd_payload_mutes_with_count() {
    let value = payload(state(Dnd::On, 2, false), "", "");
    assert_eq!(get(&value, "text"), &serde_json::json!("DND 2"));
    assert_eq!(get(&value, "class"), &serde_json::json!("muted"));
    assert_eq!(
        get(&value, "tooltip"),
        &serde_json::json!("2 notifications held by do-not-disturb -- click to let them through")
    );
    assert!(
        value.get("icon").is_none(),
        "no icon key when empty: {value}"
    );
}

#[test]
fn dnd_without_unread_is_bare() {
    let value = payload(state(Dnd::On, 0, false), "", "");
    assert_eq!(get(&value, "text"), &serde_json::json!("DND"));
    assert_eq!(get(&value, "class"), &serde_json::json!("muted"));
    assert_eq!(
        get(&value, "tooltip"),
        &serde_json::json!("do-not-disturb is on -- click to let notifications through")
    );
}

#[test]
fn icons_ride_per_update_only_when_set() {
    let unread = payload(state(Dnd::Off, 1, false), "●", "☾");
    assert_eq!(get(&unread, "icon"), &serde_json::json!("●"));
    let dnd = payload(state(Dnd::On, 1, false), "●", "☾");
    assert_eq!(get(&dnd, "icon"), &serde_json::json!("☾"));
    // The idle payload names no icon even when icons are configured:
    // the module's static icon shows, as the script's empty state does.
    let idle = payload(state(Dnd::Off, 0, false), "●", "☾");
    assert!(idle.get("icon").is_none(), "idle names no icon: {idle}");
    // A DND state shows the DND icon, never the unread one.
    assert_ne!(get(&dnd, "icon"), get(&unread, "icon"));
}

#[test]
fn set_line_carries_the_bar_wire_shape() {
    let value = payload(state(Dnd::Off, 2, true), "", "");
    let line = set_line(&value);
    assert!(line.ends_with(b"\n"), "one line, newline included");
    let body = &line[..line.len() - 1];
    let request: serde_json::Value =
        serde_json::from_slice(body).expect("the line is one JSON object");
    assert_eq!(request.get("protocol"), Some(&serde_json::json!(1)));
    assert_eq!(request.get("type"), Some(&serde_json::json!("set")));
    assert_eq!(
        request.get("id"),
        Some(&serde_json::json!(MODULE_ID)),
        "the bar's notifications push module"
    );
    assert_eq!(request.get("value"), Some(&value), "the payload rides raw");
}

#[test]
fn replies_sort_into_sent_not_placed_or_refused() {
    assert!(matches!(
        parse_reply(br#"{"type":"ok"}"#),
        super::Send::Sent
    ));
    assert!(matches!(
        parse_reply(br#"{"type":"error","message":"`notifications` is not placed in this bar (it shows:)"}"#),
        super::Send::NotPlaced
    ));
    assert!(matches!(
        parse_reply(br#"{"type":"error","message":"boom"}"#),
        super::Send::Refused(_)
    ));
    assert!(matches!(parse_reply(b"not json"), super::Send::Refused(_)));
    assert!(matches!(parse_reply(b""), super::Send::Refused(_)));
}

#[test]
fn scan_names_matches_whole_names_only() {
    // One event for our name: seen.
    let mut buf = Vec::new();
    push_event(&mut buf, "scootbar-wayland-0.sock");
    assert!(scan_names(&buf, b"scootbar-wayland-0.sock"));
    // A longer name only shares the prefix: not ours.
    let mut buf = Vec::new();
    push_event(&mut buf, "scootbar-wayland-0.sock2");
    assert!(!scan_names(&buf, b"scootbar-wayland-0.sock"));
    // A shorter name is a different socket: not ours.
    let mut buf = Vec::new();
    push_event(&mut buf, "scootbar-wayland-0.soc");
    assert!(!scan_names(&buf, b"scootbar-wayland-0.sock"));
    // An event pair, ours second: seen.
    let mut buf = Vec::new();
    push_event(&mut buf, "other.sock");
    push_event(&mut buf, "scootbar-wayland-0.sock");
    assert!(scan_names(&buf, b"scootbar-wayland-0.sock"));
    // Nothing buffered: nothing appeared.
    assert!(!scan_names(&[], b"scootbar-wayland-0.sock"));
    // An unframed buffer answers true: a needless dial costs less
    // than a missed bar.
    assert!(scan_names(&[0u8; 4], b"scootbar-wayland-0.sock"));
}

/// One inotify event for `name` (cookie 0, no mask semantics needed).
/// Events pack with no padding, as the kernel emits them.
fn push_event(buf: &mut Vec<u8>, name: &str) {
    let bytes = name.as_bytes();
    let len = bytes.len() + 1;
    buf.extend_from_slice(&0i32.to_ne_bytes()); // wd
    buf.extend_from_slice(&0u32.to_ne_bytes()); // mask
    buf.extend_from_slice(&0u32.to_ne_bytes()); // cookie
    buf.extend_from_slice(&(len as u32).to_ne_bytes());
    buf.extend_from_slice(bytes);
    buf.push(0);
}

/// A fake bar: binds `socket`, answers every `set` with `reply`, and
/// records the values it was sent. Runs until `expect` connections are
/// served or the deadline passes.
struct FakeBar {
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    thread: Option<std::thread::JoinHandle<usize>>,
}

impl FakeBar {
    /// Binds `socket` before returning, then answers every `set` with
    /// `reply`: the first dial cannot lose a bind race.
    fn serve(socket: std::path::PathBuf, reply: &'static str, expect: usize) -> Self {
        let listener = UnixListener::bind(&socket).expect("the fake bar binds");
        Self::serve_on(listener, reply, expect)
    }

    fn serve_on(listener: UnixListener, reply: &'static str, expect: usize) -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&received);
        let thread = std::thread::spawn(move || serve_loop(listener, &recorded, reply, expect));
        Self {
            received,
            thread: Some(thread),
        }
    }
}

/// Serves `expect` connections on the bound `listener`, recording
/// each `set` value and answering `reply`.
fn serve_loop(
    listener: UnixListener,
    recorded: &Arc<Mutex<Vec<serde_json::Value>>>,
    reply: &'static str,
    expect: usize,
) -> usize {
    listener.set_nonblocking(true).expect("nonblocking");
    let start = Instant::now();
    let mut served = 0;
    while served < expect && start.elapsed() < Duration::from_secs(10) {
        match listener.accept() {
            Ok((stream, _)) => {
                served += 1;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok()
                    && let Ok(request) = serde_json::from_str::<serde_json::Value>(&line)
                    && let Some(value) = request.get("value").cloned()
                {
                    recorded.lock().expect("received locks").push(value);
                }
                let _ = (&stream).write_all(reply.as_bytes());
                let _ = (&stream).flush();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
    served
}

impl FakeBar {
    fn received(&self) -> Vec<serde_json::Value> {
        self.received.lock().expect("received locks").clone()
    }

    fn served(mut self) -> usize {
        self.thread
            .take()
            .expect("the thread runs")
            .join()
            .expect("the thread joins")
    }
}

fn scratch_socket(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir =
        tempfile::TempDir::with_prefix(format!("scootnotify-feed-{tag}")).expect("a scratch dir");
    let socket = dir.path().join("scootbar-test.sock");
    (dir, socket)
}

#[test]
fn baseline_sends_nothing_and_change_pushes_once() {
    let (_dir, socket) = scratch_socket("baseline");
    let bar = FakeBar::serve(socket.clone(), "{\"type\":\"ok\"}\n", 1);
    let mut store = store();
    let mut feed = Feed::for_test(socket, "", "");
    feed.baseline(&store);
    // Baslined with no change: nothing to send, no connection.
    assert!(feed.sync(&store).is_none());
    assert!(bar.received().is_empty());
    shown(&mut store, ":1.1", "a");
    // One change: one push, succeeding silently.
    assert!(feed.sync(&store).is_none());
    assert!(!feed.owed());
    assert!(feed.poll_fd().is_none(), "no watch when nothing is owed");
    // Same state again: no second dial.
    assert!(feed.sync(&store).is_none());
    let received = bar.received();
    assert_eq!(received.len(), 1, "exactly one push for one change");
    assert_eq!(received[0], payload(State::of(&store), "", ""));
    assert_eq!(bar.served(), 1);
}

#[test]
fn dismissals_and_dnd_toggles_push_exactly() {
    let (_dir, socket) = scratch_socket("toggles");
    let bar = FakeBar::serve(socket.clone(), "{\"type\":\"ok\"}\n", 3);
    let mut store = store();
    let mut feed = Feed::for_test(socket, "", "");
    feed.baseline(&store);
    let id = shown(&mut store, ":1.1", "a");
    assert!(feed.sync(&store).is_none());
    store.dismiss(id);
    assert!(feed.sync(&store).is_none());
    store.set_dnd(Dnd::On);
    assert!(feed.sync(&store).is_none());
    let received = bar.received();
    assert_eq!(received.len(), 3, "arrival, dismissal, DND toggle");
    assert_eq!(
        received[0],
        serde_json::json!({"text": "1", "class": "normal", "tooltip": "1 notifications -- click to hold them with do-not-disturb"})
    );
    assert_eq!(received[1], serde_json::json!({"text": ""}));
    assert_eq!(
        received[2],
        serde_json::json!({"text": "DND", "class": "muted", "tooltip": "do-not-disturb is on -- click to let notifications through"})
    );
    assert_eq!(bar.served(), 3);
}

#[test]
fn no_bar_is_owed_with_a_watch_and_stays_quiet() {
    let (_dir, socket) = scratch_socket("nobar");
    let mut store = store();
    let mut feed = Feed::for_test(socket, "", "");
    feed.baseline(&store);
    shown(&mut store, ":1.1", "a");
    // Nothing listens: one line, owed, and a watch for the socket.
    let line = feed.sync(&store).expect("a failed push says why");
    assert!(line.contains("cannot reach the bar"), "{line}");
    assert!(feed.owed());
    assert!(feed.poll_fd().is_some(), "the watch exists only while owed");
    // No change: nothing more is said or dialed — the settled change
    // waits for the watch or the next change, never for a turn.
    assert!(feed.sync(&store).is_none());
    assert!(feed.owed(), "still owed, still watching");
}

#[test]
fn not_placed_logs_once_and_counts_as_placed() {
    let (_dir, socket) = scratch_socket("notplaced");
    let bar = FakeBar::serve(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"`notifications` is not placed in this bar (it shows: clock)\"}\n",
        2,
    );
    let mut store = store();
    let mut feed = Feed::for_test(socket, "", "");
    feed.baseline(&store);
    shown(&mut store, ":1.1", "a");
    let line = feed.sync(&store).expect("the guidance goes out once");
    assert!(line.contains("no notifications module"), "{line}");
    assert!(!feed.owed(), "not-placed is placed: no watch, no owed send");
    assert!(feed.poll_fd().is_none());
    // The next change retries, but the guidance stays said.
    shown(&mut store, ":1.1", "b");
    assert!(feed.sync(&store).is_none(), "second refusal stays silent");
    assert_eq!(bar.served(), 2);
}

#[test]
fn other_refusals_settle_until_the_next_change() {
    let (_dir, socket) = scratch_socket("refused");
    let bar = FakeBar::serve(
        socket.clone(),
        "{\"type\":\"error\",\"message\":\"boom\"}\n",
        1,
    );
    let mut store = store();
    let mut feed = Feed::for_test(socket, "", "");
    feed.baseline(&store);
    shown(&mut store, ":1.1", "a");
    let line = feed.sync(&store).expect("a refused push says why");
    assert!(line.contains("boom"), "{line}");
    assert!(
        !feed.owed(),
        "settled: the socket exists, so no appearance will fire"
    );
    assert!(feed.poll_fd().is_none());
    // Same state again: nothing more is said or dialed.
    assert!(feed.sync(&store).is_none());
    assert_eq!(bar.served(), 1);
}

#[test]
fn a_bar_started_later_catches_up_on_the_watch() {
    let (_dir, socket) = scratch_socket("latebar");
    let mut store = store();
    let mut feed = Feed::for_test(socket.clone(), "", "");
    feed.baseline(&store);
    shown(&mut store, ":1.1", "a");
    assert!(feed.sync(&store).is_some(), "nothing listens yet");
    assert!(feed.owed());
    assert!(feed.poll_fd().is_some());
    // The bar starts later: binding creates the socket file the
    // watch waits for. Bound in this thread: `bind` returns listening,
    // so the owed retry cannot lose a bind-before-listen race here
    // (production covers that race with its bounded connect retry).
    let bar = FakeBar::serve(socket, "{\"type\":\"ok\"}\n", 1);
    wait_readable(&mut feed);
    let retried = feed.retry(&store);
    assert!(
        retried.is_none(),
        "the owed send goes out on the watch: {retried:?}"
    );
    assert!(!feed.owed(), "the owed send went out on the watch");
    assert!(feed.poll_fd().is_none(), "the watch retires with the debt");
    assert_eq!(bar.received().len(), 1);
    assert_eq!(bar.served(), 1);
}

/// Spins until the feed's watch reads (the bind's CREATE event), up to
/// 2 s: the event is queued at bind, so this returns at once unless
/// the watch never armed.
fn wait_readable(feed: &mut Feed) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        let Some(watch) = feed.poll_fd() else {
            panic!("the watch is armed while a send is owed");
        };
        let mut fds = [PollFd::new(&watch, PollFlags::IN)];
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        let _ = poll(&mut fds, Some(&timeout));
        if !fds[0].revents().is_empty() {
            return;
        }
    }
    panic!("the watch never fired for the late bar");
}

#[test]
fn state_counts_live_plus_unseen() {
    let mut store = store();
    assert_eq!(State::of(&store), state(Dnd::Off, 0, false));
    let id = shown(&mut store, ":1.1", "a");
    assert_eq!(State::of(&store), state(Dnd::Off, 1, false));
    // Dismissing closes to history seen: the count falls.
    store.dismiss(id);
    assert_eq!(State::of(&store), state(Dnd::Off, 0, false));
    // DND holds the next arrival unseen: the count holds it.
    store.set_dnd(Dnd::On);
    let _ = {
        let (arrival, _) = store.notify(incoming(":1.1", "b"), now(), 0);
        arrival
    };
    assert_eq!(State::of(&store), state(Dnd::On, 1, false));
}

#[test]
fn state_is_critical_for_live_or_unseen() {
    let mut store = store();
    let id = shown(&mut store, ":1.1", "a");
    assert!(!State::of(&store).critical);
    let _ = id;
    let cid = {
        let start = now();
        let (arrival, _) = store.notify(urgent(":1.1", "b"), start, 0);
        let crate::model::store::Arrival::Shown { id, .. } = arrival else {
            panic!("a critical card shows");
        };
        store.presented(id, start, 0);
        id
    };
    assert!(State::of(&store).critical, "a live critical counts");
    store.dismiss(cid);
    assert!(
        !State::of(&store).critical,
        "a seen dismissal stops counting"
    );
    store.set_dnd(Dnd::On);
    let _ = {
        let (arrival, _) = store.notify(urgent(":1.1", "c"), now(), 0);
        arrival
    };
    assert!(State::of(&store).critical, "a held critical counts");
}

#[test]
fn snapshot_and_idle_sync_allocate_nothing() {
    let mut store = store();
    shown(&mut store, ":1.1", "a");
    let mut feed = Feed::for_test("/nonexistent-feed/socket".into(), "", "");
    feed.baseline(&store);
    // Warm up, as the repaint pins do: the measured turn is steady-state.
    let _ = State::of(&store);
    let _ = feed.sync(&store);
    let (_, allocs) = scootbg_mem::count_allocations(|| {
        let _ = State::of(&store);
        let _ = feed.sync(&store);
    });
    assert_eq!(allocs, 0, "an unchanged turn allocates nothing");
}
