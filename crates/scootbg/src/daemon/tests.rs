//! The request-to-reply mapping. The loop itself runs against a real
//! compositor in `crates/scootbg/tests/`.

use std::borrow::Cow;

use super::respond::{ChangeError, Changes, Ready, Responder, write_ready};
use crate::color::Color;
use crate::control::{Answer, ConnId, Handler};
use crate::image::{Filter, Mode};
use crate::protocol::{ImageRequest, OutputEntry, OutputList, PROTOCOL_VERSION, Request, Show};
use crate::waiters::Outcome;

/// What was asked, owned.
type Choice = Option<Show<'static>>;

fn owned(show: Show<'_>) -> Show<'static> {
    match show {
        Show::Color(color) => Show::Color(color),
        Show::Image(image) => Show::Image(ImageRequest {
            path: Cow::Owned(image.path.into_owned()),
            mode: image.mode,
            fill: image.fill,
            filter: image.filter,
        }),
    }
}

const NO_OUTPUTS: &[OutputEntry<'static>; 0] = &[];

/// Records the changes asked for, or refuses them all with `refuse`.
struct Fake<'a> {
    outputs: &'a dyn OutputList,
    changes: Vec<(u64, Option<String>, Choice)>,
    refuse: Option<ChangeError>,
}

impl<'a> Fake<'a> {
    fn new(outputs: &'a dyn OutputList) -> Self {
        Self {
            outputs,
            changes: Vec::new(),
            refuse: None,
        }
    }
}

impl Changes for Fake<'_> {
    fn outputs(&self) -> &dyn OutputList {
        self.outputs
    }

    fn change(
        &mut self,
        conn: ConnId,
        output: Option<&str>,
        show: Option<Show<'_>>,
    ) -> Result<(), ChangeError> {
        if let Some(error) = self.refuse {
            return Err(error);
        }
        let id = if conn == ConnId::for_test(1) { 1 } else { 0 };
        self.changes
            .push((id, output.map(str::to_owned), show.map(owned)));
        Ok(())
    }
}

fn ask(responder: &mut Responder, line: &str) -> serde_json::Value {
    let mut out = Vec::new();
    let answer = responder.handle(ConnId::for_test(1), line.as_bytes(), &mut out);
    assert_eq!(answer, Answer::Now);
    assert_eq!(out.last(), Some(&b'\n'), "one line, newline-terminated");
    assert_eq!(out.iter().filter(|&&b| b == b'\n').count(), 1);
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn query_answers_an_empty_output_list() {
    let mut fake = Fake::new(NO_OUTPUTS);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "outputs", "outputs": []}));
    assert!(!responder.stop);
}

#[test]
fn version_answers_the_build_and_protocol() {
    let mut fake = Fake::new(NO_OUTPUTS);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(&mut responder, Request::Version.line().trim_end());
    assert_eq!(reply["type"], "version");
    assert_eq!(reply["protocol"], PROTOCOL_VERSION);
    assert_eq!(reply["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn kill_answers_ok_and_asks_the_loop_to_stop() {
    let mut fake = Fake::new(NO_OUTPUTS);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(&mut responder, Request::Kill.line().trim_end());
    assert_eq!(reply, serde_json::json!({"type": "ok"}));
    assert!(responder.stop);
}

#[test]
fn bad_requests_answer_errors_and_change_nothing() {
    let mut fake = Fake::new(NO_OUTPUTS);
    let mut responder = Responder::new(&mut fake);
    for line in [
        "garbage",
        "",
        r#"{"type":"kill"}"#,
        r#"{"protocol":99,"type":"kill"}"#,
        r#"{"protocol":1,"type":"set"}"#,
        r#"{"protocol":1,"type":"set","color":"red"}"#,
        r##"{"protocol":1,"type":"set","color":"#fff"}"##,
        r##"{"protocol":1,"type":"set","color":7}"##,
        r##"{"protocol":1,"type":"set","color":"#c03020","output":7}"##,
        r#"{"protocol":1,"type":"set","image":"relative.png"}"#,
        r#"{"protocol":1,"type":"set","image":"/a.png","mode":"zoom"}"#,
        r##"{"protocol":1,"type":"set","image":"/a.png","color":"#000000"}"##,
        r#"{"protocol":1,"type":"clear","output":["DP-1"]}"#,
        r#"{"protocol":1,"type":"apply-config"}"#,
        r#"{"protocol":1}"#,
    ] {
        let reply = ask(&mut responder, line);
        assert_eq!(reply["type"], "error", "{line:?}");
        assert!(reply["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    assert!(!responder.stop);
    assert!(fake.changes.is_empty());
}

/// `set` and `clear` hand the change over and answer later: nothing is
/// written now, the reply comes once the compositor has it.
#[test]
fn set_and_clear_change_the_wallpaper_and_answer_later() {
    let mut fake = Fake::new(NO_OUTPUTS);
    let mut responder = Responder::new(&mut fake);
    let red = Color::parse("#c03020").unwrap();
    let quoted = "DP-\"1\"";
    let image = Show::Image(ImageRequest {
        path: "/p/a.jpg".into(),
        mode: Mode::Center,
        fill: red,
        filter: Filter::Bilinear,
    });
    for request in [
        Request::Set {
            show: Show::Color(red),
            output: None,
        },
        Request::Set {
            show: Show::Color(red),
            output: Some("DP-2".into()),
        },
        Request::Set {
            show: image.clone(),
            output: None,
        },
        Request::Clear { output: None },
        Request::Clear {
            output: Some(quoted.into()),
        },
    ] {
        let mut out = Vec::new();
        let line = request.line();
        let answer = responder.handle(ConnId::for_test(1), line.trim_end().as_bytes(), &mut out);
        assert_eq!(answer, Answer::Later, "{line}");
        assert!(out.is_empty(), "nothing written yet: {line}");
    }
    let changes: Vec<_> = fake
        .changes
        .iter()
        .map(|(_, o, c)| (o.clone(), c.clone()))
        .collect();
    assert_eq!(
        changes,
        [
            (None, Some(Show::Color(red))),
            (Some("DP-2".to_owned()), Some(Show::Color(red))),
            (None, Some(image)),
            (None, None),
            (Some(quoted.to_owned()), None),
        ]
    );
    assert!(
        fake.changes.iter().all(|(conn, ..)| *conn == 1),
        "the asking connection"
    );
}

#[test]
fn a_refused_change_is_an_error_now_naming_the_output() {
    let mut fake = Fake::new(NO_OUTPUTS);
    fake.refuse = Some(ChangeError::UnknownOutput);
    let mut responder = Responder::new(&mut fake);
    let line = Request::Set {
        show: Show::Color(Color::parse("#c03020").unwrap()),
        output: Some("HDMI-A-9".into()),
    }
    .line();
    let reply = ask(&mut responder, line.trim_end());
    assert_eq!(reply["type"], "error");
    let message = reply["message"].as_str().unwrap();
    assert!(message.contains("\"HDMI-A-9\""), "{message}");
    assert!(message.contains("nothing was changed"), "{message}");
}

#[test]
fn too_many_waiting_images_is_an_error_now() {
    let mut fake = Fake::new(NO_OUTPUTS);
    fake.refuse = Some(ChangeError::Busy);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(
        &mut responder,
        r#"{"protocol":1,"type":"set","image":"/a.png"}"#,
    );
    assert_eq!(reply["type"], "error");
    let message = reply["message"].as_str().unwrap();
    assert!(message.contains("nothing was changed"), "{message}");
}

#[test]
fn the_late_reply_is_ok_or_an_error() {
    let mut out = Vec::new();
    write_ready(&mut out, &Ready::Done(Outcome::Shown));
    assert_eq!(out, b"{\"type\":\"ok\"}\n");
    out.clear();
    write_ready(&mut out, &Ready::Done(Outcome::Failed));
    let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["type"], "error");
    assert!(reply["message"].as_str().unwrap().contains("stderr"));
    out.clear();
    write_ready(
        &mut out,
        &Ready::Refused("cannot show \"/x\": no such file".into()),
    );
    let reply: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["message"], "cannot show \"/x\": no such file");
}

/// The poll set's allocation survives the round trip through `reuse`, so
/// the loop allocates nothing once warm. If std ever stopped collecting in
/// place, this fails rather than the loop quietly allocating per wakeup.
#[test]
fn the_poll_set_keeps_its_allocation() {
    use rustix::event::{PollFd, PollFlags};

    let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut fds: Vec<PollFd<'static>> = Vec::with_capacity(19);
    let ptr = fds.as_ptr();
    for _ in 0..3 {
        let mut round: Vec<PollFd<'_>> = super::reuse(fds);
        for _ in 0..19 {
            round.push(PollFd::new(&a, PollFlags::IN));
        }
        assert_eq!(round.as_ptr(), ptr);
        fds = super::reuse(round);
        assert!(fds.is_empty());
        assert_eq!(fds.as_ptr(), ptr);
        assert!(fds.capacity() >= 19);
    }
}

/// The one panic message the crash hook turns into exit 1: std's own for a
/// failed `print!`/`eprint!`.
#[test]
fn the_crash_hook_recognises_stds_broken_stdio_panic() {
    use super::crash::is_stdio_message;

    // std's `_eprint` cannot be pointed at a test double, so the real
    // panic, an `eprintln!` into a pipe with no reader, is exercised end
    // to end in `tests/daemon.rs` (`WAYLAND_DEBUG` with a broken stderr).
    assert!(is_stdio_message(
        "failed printing to stderr: Broken pipe (os error 32)"
    ));
    assert!(is_stdio_message(
        "failed printing to stdout: Broken pipe (os error 32)"
    ));
    assert!(!is_stdio_message("index out of bounds"));
    assert!(!is_stdio_message(
        "called `Option::unwrap()` on a `None` value"
    ));
}

/// The hook removes the socket only while armed, and at most once; after
/// `disarm` (which the daemon calls before releasing its claim) a panic
/// leaves the path alone, whoever has bound it since.
#[test]
fn the_crash_hook_removes_the_socket_only_while_armed() {
    use super::crash::{Armed, remove_if_armed};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    let dir = std::env::temp_dir().join(format!("sbg-crash-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("s.sock");

    // Armed: removed, and the hold is spent.
    std::fs::write(&socket, b"").unwrap();
    let armed = Armed::for_test(Arc::new(AtomicBool::new(true)));
    remove_if_armed(&armed, &socket);
    assert!(!socket.exists());
    // A new daemon's socket at the same path afterwards survives a second
    // panic of the old one.
    std::fs::write(&socket, b"new").unwrap();
    remove_if_armed(&armed, &socket);
    assert!(socket.exists());

    // Disarmed before any panic: never removed.
    let disarmed = Armed::for_test(Arc::new(AtomicBool::new(true)));
    disarmed.disarm();
    remove_if_armed(&disarmed, &socket);
    assert!(socket.exists());

    std::fs::remove_dir_all(&dir).unwrap();
}

/// Normal operation: the listener is always polled, with no timeout, and
/// the clock is not even read.
#[test]
fn an_armed_listener_polls_forever_and_reads_no_clock() {
    use super::listen::Listening;
    let mut listening = Listening::default();
    for _ in 0..3 {
        let plan = listening.poll_plan(|| panic!("read the clock while armed"));
        assert_eq!(plan, (true, None));
    }
    assert!(!listening.worked(), "nothing to recover from");
}

/// A failed accept rests the listener for `REST`, bounded, then re-arms
/// it; a failure that persists is reported once, and its end once.
#[test]
fn a_failed_accept_rests_the_listener_then_rearms_it() {
    use super::listen::{Listening, REST};
    use std::time::{Duration, Instant};

    let t0 = Instant::now();
    let mut listening = Listening::default();
    assert!(listening.failed(t0), "the first failure is reported");
    assert!(listening.is_resting());
    // Resting: not polled, and the timeout is what is left of the rest.
    assert_eq!(listening.poll_plan(|| t0), (false, Some(REST)));
    let half = t0 + REST / 2;
    assert_eq!(listening.poll_plan(|| half), (false, Some(REST / 2)));
    // Never a zero timeout while resting (that would spin): at the end of
    // the rest it is re-armed instead.
    assert_eq!(listening.poll_plan(|| t0 + REST), (true, None));
    assert!(!listening.is_resting());
    // A clock read late (a long stall) re-arms too.
    assert!(
        !listening.failed(t0 + REST),
        "still failing: not re-reported"
    );
    assert_eq!(
        listening.poll_plan(|| t0 + REST * 10),
        (true, None),
        "re-armed after a stall"
    );
    // Accepting works again: reported once, then quiet.
    assert!(listening.worked());
    assert!(!listening.worked());
    // A new failure later is news again.
    assert!(listening.failed(t0 + Duration::from_secs(60)));
}

/// However many times accepting fails in a row, the loop polls at most once
/// per `REST` with the listener out and once with it in: a bounded rate,
/// never a spin, and never deaf for longer than `REST`.
#[test]
fn a_persistent_accept_failure_is_retried_at_a_bounded_rate() {
    use super::listen::{Listening, REST};
    use std::time::Instant;

    let t0 = Instant::now();
    let mut listening = Listening::default();
    let mut now = t0;
    let mut polls_with_listener = 0;
    let mut reports = 0;
    // Simulate 10 s of a listener whose every accept fails at once.
    while now < t0 + REST * 10 {
        match listening.poll_plan(|| now) {
            (true, None) => {
                polls_with_listener += 1;
                // Readable at once, and the accept fails.
                reports += usize::from(listening.failed(now));
            }
            (false, Some(timeout)) => {
                assert!(!timeout.is_zero());
                now += timeout;
            }
            other => panic!("unexpected plan {other:?}"),
        }
    }
    assert_eq!(reports, 1);
    assert_eq!(polls_with_listener, 10);
}

/// `query` lists the model's outputs as they stand, in order: the first at
/// 1.5 (scoot's `wl_output` says 2; the fraction comes from the surface).
#[test]
fn query_reports_each_output_and_its_surface() {
    use crate::outputs::Outputs;

    let mut outputs = Outputs::<()>::default();
    let a = outputs.add(3, |_| ());
    let b = outputs.add(4, |_| ());
    let c = outputs.add(5, |_| ());
    {
        let first = &mut outputs.get_mut(a).unwrap().output;
        first.stage_name("HEADLESS-1".into());
        first.stage_description("Headless output 1".into());
        first.stage_mode(true, 2560, 1440);
        first.stage_scale(2);
        first.done();
        let _ = first.settled();
        let _ = first.prefer_fractional(180);
        let _ = first.configure(7, 1707, 960);
    }
    {
        let second = &mut outputs.get_mut(b).unwrap().output;
        second.stage_name("HEADLESS-2".into());
        second.done();
        let _ = second.settled();
        let _ = second.closed();
        let _ = second.retry();
        let _ = second.closed();
    }
    let _ = c; // bound, not yet reported: waiting

    let mut fake = Fake::new(&outputs);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(
        reply,
        serde_json::json!({"type": "outputs", "outputs": [
            {
                "name": "HEADLESS-1",
                "description": "Headless output 1",
                "mode": {"width": 2560, "height": 1440},
                "scale": 2,
                "transform": "normal",
                "logical": {"width": 1707, "height": 960},
                "surface": {
                    "state": "configured",
                    "size": {"width": 1707, "height": 960},
                    "scale": 1.5,
                    "pixels": {"width": 2561, "height": 1440},
                },
                "shows": null,
            },
            {
                "name": "HEADLESS-2",
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "gave-up", "size": null, "scale": null, "pixels": null},
                "shows": null,
            },
            {
                "name": null,
                "description": null,
                "mode": null,
                "scale": 1,
                "transform": "normal",
                "logical": null,
                "surface": {"state": "waiting", "size": null, "scale": null, "pixels": null},
                "shows": null,
            },
        ]})
    );
    // Removing one updates the next reply.
    let _ = outputs.remove_global(4);
    let mut fake = Fake::new(&outputs);
    let mut responder = Responder::new(&mut fake);
    let reply = ask(&mut responder, Request::Query.line().trim_end());
    assert_eq!(reply["outputs"].as_array().unwrap().len(), 2);
}

/// A `query` reply into a warm buffer allocates nothing: the entries are
/// borrowed from the model and serialized straight into the connection's
/// reused output buffer.
#[test]
fn a_query_reply_reuses_the_output_buffer() {
    use crate::outputs::Outputs;

    let mut outputs = Outputs::<()>::default();
    for global in 0..4 {
        let id = outputs.add(global, |_| ());
        let output = &mut outputs.get_mut(id).unwrap().output;
        output.stage_name(format!("DP-{global}"));
        output.stage_mode(true, 3840, 2160);
        output.done();
        let _ = output.settled();
        let _ = output.configure(1, 3840, 2160);
    }
    let mut fake = Fake::new(&outputs);
    let mut responder = Responder::new(&mut fake);
    let line = Request::Query.line();
    let mut out = Vec::with_capacity(4096);
    let ptr = out.as_ptr();
    for _ in 0..3 {
        out.clear();
        let _ = responder.handle(ConnId::for_test(1), line.trim_end().as_bytes(), &mut out);
        assert!(out.len() < 4096);
        assert_eq!(out.as_ptr(), ptr, "the buffer was reallocated");
    }
}

/// A queued image request that a newer choice covers is taken out of the
/// queue (never decoded) and its reply waits at its own generation, as a
/// superseded color's does: it resolves once the outputs the newer choice
/// stamped show it, not at once.
#[test]
fn a_superseded_image_request_waits_like_a_color() {
    use std::sync::Arc;

    use super::change::sweep;
    use crate::choices::Choices;
    use crate::image::render::Look;
    use crate::jobs::{Jobs, Trial};
    use crate::waiters::{Outcome, Progress, Waiters, outcome};
    use crate::wallpaper::{Image, Wallpaper};

    let conn = ConnId::for_test(5);
    let mut jobs: Jobs<ConnId> = Jobs::default();
    let image = Arc::new(Image {
        path: "/a.png".into(),
        look: Look {
            mode: Mode::Fill,
            fill: Color { r: 0, g: 0, b: 0 },
            filter: Filter::Lanczos3,
        },
        serial: 1,
    });
    jobs.trial(image, Trial { conn, output: None }, vec![])
        .unwrap();
    // A color for every output, generation 2, recorded; its output (stamp
    // 2) is still drawing.
    let mut choices = Choices::default();
    assert!(choices.set(None, Some(Wallpaper::Color(Color { r: 1, g: 2, b: 3 })), 2));
    let mut waiters: Waiters<ConnId> = Waiters::with_capacity(4);
    sweep(&mut jobs, &choices, &mut waiters);
    assert_eq!(jobs.queued(), 0, "never decoded");
    assert_eq!(waiters.counts(), (1, 0), "waiting, not answered");
    let drawing = [(2, Progress::Waiting)];
    assert_eq!(waiters.resolve(|g| outcome(g, drawing.into_iter())), None);
    let shown = [(2, Progress::Done)];
    let sync = waiters.resolve(|g| outcome(g, shown.into_iter())).unwrap();
    let mut ready = Vec::new();
    waiters.synced(sync, |conn, outcome| ready.push((conn, outcome)));
    assert_eq!(ready, [(conn, Outcome::Shown)]);
}
