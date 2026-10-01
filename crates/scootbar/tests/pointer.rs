//! Pointer input on a headless scoot: clicks, scrolls and hover reaching a
//! real bar through scoot's own input injection (`click`, `pointer`,
//! `scroll`), and the commands the bindings launch.
//!
//! The bar is a clock alone, centered on a 1600-pixel output in the
//! seven-segment test font, so the clock is at the middle and everything
//! else is bare bar. Every binding writes into a file, so a test reads what
//! ran. Skipped without a `scoot` binary (see `common`);
//! `SCOOTBAR_REQUIRE_SCOOT` makes that a failure.
//!
//! What is not here: touch (scoot cannot inject it; the bar never asks for a
//! `wl_touch`, pinned by `pointer::tests::touch_and_the_keyboard_never_get_a_pointer`),
//! and the pure cases (the state machine's every ordering, the scroll flood's
//! arithmetic), which are unit tests.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    PATIENCE, Reaper, Session, Shot, allowed_in_child, inheritable_fds, open_fds, rgb, settled_fds,
};

const BAR: &str = "#102030";
const FG: &str = "#f0f0f0";
const ACCENT: &str = "#f9e2af";
const HEIGHT: u32 = 60;
/// The middle of the clock.
const CLOCK: (u32, u32) = (800, 30);
/// Bare bar, far from the clock.
const BARE: (u32, u32) = (100, 30);

/// The bar's config: the look, the clock, and `bindings` in its table.
fn config(bindings: &str) -> String {
    format!(
        "[bar]\nheight = {HEIGHT}\nfont-size = 50\n\
         [colors]\nbackground = \"{BAR}\"\nforeground = \"{FG}\"\naccent = \"{ACCENT}\"\n\
         [clock]\nformat = \"%H:%M\"\n{bindings}"
    )
}

/// A running bar with `bindings` on its clock, and a directory to write in.
struct Rig {
    session: Session,
    bar: Reaper,
    dir: PathBuf,
    file: PathBuf,
}

impl Rig {
    fn start(tag: &str, bindings: &str) -> Option<Self> {
        Self::start_with(tag, bindings, &[])
    }

    fn start_with(tag: &str, bindings: &str, env: &[(&str, &str)]) -> Option<Self> {
        let session = Session::scoot(tag, 1, "")?;
        let dir = session.runtime_dir().join("out");
        fs::create_dir_all(&dir).unwrap();
        let file = session.runtime_dir().join("bar.toml");
        fs::write(
            &file,
            config(&bindings.replace("DIR", dir.to_str().unwrap())),
        )
        .unwrap();
        let bar = Reaper(session.bar_with_env(
            &["--config", file.to_str().unwrap(), "--center", "clock"],
            env,
        ));
        let mut rig = Self {
            session,
            bar,
            dir,
            file,
        };
        rig.wait_drawn();
        Some(rig)
    }

    /// The bar's first frame is on screen.
    fn wait_drawn(&mut self) {
        self.session
            .wait_for(&mut self.bar.0, "the bar drawn", |session| {
                (session.scoot_screenshot(1).at(0, 0) == rgb(BAR)).then_some(())
            });
    }

    fn pid(&self) -> u32 {
        self.bar.0.id()
    }

    fn ipc(&self, request: &str) {
        let reply = self.session.scoot_ipc(request);
        assert_eq!(reply["type"], "ok", "{request}: {reply}");
    }

    fn click(&self, (x, y): (u32, u32), button: &str) {
        self.ipc(&format!(
            r#"{{"type":"click","x":{x},"y":{y},"button":"{button}"}}"#
        ));
    }

    fn move_to(&self, (x, y): (u32, u32)) {
        self.ipc(&format!(r#"{{"type":"pointer_move","x":{x},"y":{y}}}"#));
    }

    fn button(&self, button: &str, pressed: bool) {
        self.ipc(&format!(
            r#"{{"type":"pointer_button","button":"{button}","pressed":{pressed}}}"#
        ));
    }

    fn scroll(&self, dy: f64) {
        self.ipc(&format!(r#"{{"type":"scroll","dx":0,"dy":{dy}}}"#));
    }

    /// The lines of `name` in the output directory (none if absent).
    fn lines(&self, name: &str) -> Vec<String> {
        fs::read_to_string(self.dir.join(name))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Waits until `name` has at least `n` lines; returns them.
    fn wait_lines(&mut self, name: &str, n: usize) -> Vec<String> {
        let path = self.dir.join(name);
        self.session
            .wait_for(&mut self.bar.0, &format!("{n} lines in {name}"), |_| {
                let lines: Vec<String> = fs::read_to_string(&path)
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect();
                (lines.len() >= n).then_some(lines)
            })
    }

    /// Waits until the bar log says `text`.
    fn wait_log(&mut self, text: &str) {
        self.session
            .wait_for(&mut self.bar.0, &format!("`{text}` in the bar log"), |s| {
                s.bar_stderr().contains(text).then_some(())
            });
    }

    /// Runs `scootbar msg ARGS`; its stdout.
    fn msg(&self, args: &[&str]) -> std::process::Output {
        self.session
            .scootbar()
            .arg("msg")
            .args(args)
            .output()
            .unwrap()
    }
}

/// `echo WORD >> DIR/FILE`, as an exec binding.
fn append(word: &str, file: &str) -> String {
    format!("{{ exec = [\"sh\", \"-c\", \"echo {word} >> DIR/{file}\"] }}")
}

fn has(shot: &Shot, color: [u8; 3], columns: std::ops::Range<u32>) -> bool {
    (0..HEIGHT).any(|y| columns.clone().any(|x| shot.at(x, y) == color))
}

/// Children of `pid`, as `(pid, state, command)`.
fn children(pid: u32) -> Vec<(u32, char, String)> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        let Some(child) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) S ppid ...`: comm may hold spaces and parentheses.
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else {
            continue;
        };
        let mut rest = stat[close + 1..].split_whitespace();
        let (Some(state), Some(parent)) = (rest.next(), rest.next()) else {
            continue;
        };
        if parent.parse() == Ok(pid) {
            found.push((
                child,
                state.chars().next().unwrap(),
                stat[open + 1..close].to_owned(),
            ));
        }
    }
    found
}

fn kill(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
}

#[test]
fn each_button_runs_its_own_binding() {
    let bindings = format!(
        "on-click = {}\non-right-click = {}\non-middle-click = {}\n",
        append("left", "buttons"),
        append("right", "buttons"),
        append("middle", "buttons"),
    );
    let Some(mut rig) = Rig::start("ptr-buttons", &bindings) else {
        return;
    };
    rig.click(CLOCK, "left");
    rig.wait_lines("buttons", 1);
    rig.click(CLOCK, "right");
    rig.wait_lines("buttons", 2);
    rig.click(CLOCK, "middle");
    let lines = rig.wait_lines("buttons", 3);
    assert_eq!(lines, ["left", "right", "middle"]);
}

#[test]
fn a_press_off_the_module_or_a_release_after_leaving_is_no_click() {
    let bindings = format!("on-click = {}\n", append("click", "clicks"));
    let Some(mut rig) = Rig::start("ptr-release", &bindings) else {
        return;
    };
    // Bare bar: nothing there to click.
    rig.click(BARE, "left");
    // Pressed on the clock, released on bare bar: the pointer left it.
    rig.move_to(CLOCK);
    rig.button("left", true);
    rig.move_to(BARE);
    rig.button("left", false);
    // Pressed on bare bar, released on the clock: nothing was armed.
    rig.button("left", true);
    rig.move_to(CLOCK);
    rig.button("left", false);
    // And a real click still works, and is the only one there is.
    rig.click(CLOCK, "left");
    rig.wait_lines("clicks", 1);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        rig.lines("clicks"),
        ["click"],
        "{}",
        rig.session.bar_stderr()
    );
}

#[test]
fn a_scroll_flood_is_a_bounded_number_of_commands() {
    let bindings = format!("on-scroll-down = {}\n", append("down", "down"));
    let Some(rig) = Rig::start("ptr-flood", &bindings) else {
        return;
    };
    rig.move_to(CLOCK);
    let events: u64 = 400;
    let started = Instant::now();
    for _ in 0..events {
        // One wheel notch each: 15 axis units.
        rig.scroll(15.0);
    }
    let flood = started.elapsed();
    // Let what is held back drain.
    std::thread::sleep(Duration::from_millis(500));
    let lines = rig.lines("down");
    eprintln!(
        "{events} scroll events in {flood:?}: {} commands",
        lines.len()
    );
    assert!(!lines.is_empty(), "no scroll ran at all");
    // At most one command per 16 ms frame over the flood (and the settle),
    // however many events there were: the bound is time, not events.
    let frames = flood.as_millis() as u64 / 16 + 2;
    assert!(
        (lines.len() as u64) <= frames,
        "{} commands in {flood:?} for {events} events (at most {frames})",
        lines.len()
    );
    assert!(
        (lines.len() as u64) < events,
        "a command per event: {} for {events}",
        lines.len()
    );
    // Nothing is left a zombie, and every command finished.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !children(rig.pid()).is_empty() {
        assert!(
            Instant::now() < deadline,
            "children left: {:?}",
            children(rig.pid())
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_launched_command_holds_none_of_the_bars_descriptors() {
    let bindings = "on-click = { exec = [\"sh\", \"-c\", \"ls /proc/self/fd > DIR/fds\"] }\n";
    // What this process was handed by whatever started it (a CI runner may
    // leave descriptors open): the bar inherits them, and so does every
    // command it launches. Anything else is the bar's own, and must not.
    let inherited = inheritable_fds("self");
    let Some(mut rig) = Rig::start("ptr-fds", bindings) else {
        return;
    };
    // The bar holds real descriptors by now: a Wayland socket, the control
    // socket and its lock, shm memfds, the clock's timerfd.
    let held = settled_fds(rig.pid());
    assert!(held > 5, "the bar holds only {held} descriptors");
    // Every one of them is close-on-exec but those it was started with:
    // the guard against a descriptor the bar opens without it, whatever
    // the command listing below happens to show.
    let allowed_in_bar: BTreeSet<i32> = inherited.iter().copied().chain(0..=2).collect();
    let leaky: Vec<_> = inheritable_fds(rig.pid())
        .difference(&allowed_in_bar)
        .copied()
        .collect();
    assert!(
        leaky.is_empty(),
        "the bar holds {leaky:?} without close-on-exec, which no command may inherit"
    );
    rig.click(CLOCK, "left");
    let lines = rig.wait_lines("fds", 3);
    let fds: BTreeSet<i32> = lines.iter().filter_map(|l| l.trim().parse().ok()).collect();
    // Stdio, what the bar inherited, and the directory `ls` itself reads.
    let allowed = allowed_in_child(&inherited);
    let extra: Vec<_> = fds.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "inherited: {extra:?} beyond {allowed:?}: {fds:?} (the bar has {held})"
    );
    // The bar holds the child's pidfd until its loop reaps it, a turn after
    // the command printed its last line: the count is back to what it was
    // once that turn has run, never at a fixed moment (a loaded machine
    // makes the turn late, which failed this test about half the runs under
    // six spinning CPUs). A leak is a count that never comes back.
    let deadline = Instant::now() + PATIENCE;
    while open_fds(rig.pid()) != held {
        assert!(
            Instant::now() < deadline,
            "the launch leaked a descriptor into the bar: {} open, {held} before",
            open_fds(rig.pid())
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn finished_commands_are_reaped_and_a_hung_one_cannot_fill_the_table() {
    let bindings = "on-click = { exec = [\"true\"] }\n\
                    on-right-click = { exec = [\"sleep\", \"20\"] }\n";
    let Some(mut rig) = Rig::start("ptr-reap", bindings) else {
        return;
    };
    // Short commands leave no zombie, even with no other event to wake the
    // bar for them.
    for _ in 0..5 {
        rig.click(CLOCK, "left");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !children(rig.pid()).is_empty() {
        assert!(
            Instant::now() < deadline,
            "zombies: {:?}",
            children(rig.pid())
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // Commands that never exit: capped, said, never queued.
    for _ in 0..12 {
        rig.click(CLOCK, "right");
    }
    rig.wait_log("launched commands are still running");
    let sleepers: Vec<u32> = children(rig.pid())
        .into_iter()
        .filter(|(_, _, comm)| comm == "sleep")
        .map(|(pid, ..)| pid)
        .collect();
    let cleanup = || sleepers.iter().for_each(|&pid| kill(pid));
    assert_eq!(sleepers.len(), 8, "{:?}", children(rig.pid()));
    // A slot frees as one exits, and the next launch takes it.
    kill(sleepers[0]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while children(rig.pid()).iter().any(|c| c.0 == sleepers[0]) {
        assert!(
            Instant::now() < deadline,
            "the killed child was never reaped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    rig.click(CLOCK, "right");
    let deadline = Instant::now() + Duration::from_secs(5);
    while children(rig.pid())
        .iter()
        .filter(|c| c.2 == "sleep")
        .count()
        < 8
    {
        assert!(Instant::now() < deadline, "the freed slot was never used");
        std::thread::sleep(Duration::from_millis(20));
    }
    cleanup();
    for (pid, ..) in children(rig.pid()) {
        kill(pid);
    }
}

#[test]
fn a_failing_command_is_said_once_a_second_and_the_bar_carries_on() {
    let bindings = "on-click = { exec = [\"/nonexistent/scootbar-test\"] }\n";
    let Some(mut rig) = Rig::start("ptr-fail", bindings) else {
        return;
    };
    let started = Instant::now();
    for _ in 0..30 {
        rig.click(CLOCK, "left");
    }
    rig.wait_log("scootbar-test");
    let said = rig
        .session
        .bar_stderr()
        .lines()
        .filter(|l| l.contains("scootbar-test"))
        .count();
    // Thirty failures, at most one line a second: bounded by the time they
    // took (plus the first, and one for the count that follows).
    let seconds = started.elapsed().as_secs() + 2;
    assert!(
        said as u64 <= seconds,
        "{said} lines in {seconds} s: {}",
        rig.session.bar_stderr()
    );
    assert!(rig.bar.0.try_wait().unwrap().is_none(), "the bar died");
}

#[test]
fn hover_tints_the_module_under_the_pointer_and_only_while_it_is_there() {
    let bindings = format!("on-click = {}\n", append("click", "clicks"));
    let Some(mut rig) = Rig::start("ptr-hover", &bindings) else {
        return;
    };
    let span = 600..1000;
    assert!(!has(
        &rig.session.scoot_screenshot(1),
        rgb(ACCENT),
        span.clone()
    ));
    rig.move_to(CLOCK);
    rig.session
        .wait_for(&mut rig.bar.0, "the clock tinted", |session| {
            has(&session.scoot_screenshot(1), rgb(ACCENT), span.clone()).then_some(())
        });
    // Moving over bare bar takes it back.
    rig.move_to(BARE);
    rig.session
        .wait_for(&mut rig.bar.0, "the tint gone", |session| {
            (!has(&session.scoot_screenshot(1), rgb(ACCENT), span.clone())).then_some(())
        });
    // Nothing else on the bar was ever the accent.
    assert!(!has(&rig.session.scoot_screenshot(1), rgb(ACCENT), 0..600));
}

#[test]
fn a_clock_with_no_binding_does_not_tint() {
    let Some(mut rig) = Rig::start("ptr-nobind", "") else {
        return;
    };
    rig.move_to(CLOCK);
    std::thread::sleep(Duration::from_millis(300));
    rig.wait_drawn();
    assert!(!has(
        &rig.session.scoot_screenshot(1),
        rgb(ACCENT),
        600..1000
    ));
}

#[test]
fn the_pointer_is_taken_only_while_a_binding_needs_it() {
    let trace = [("WAYLAND_DEBUG", "1")];
    // No binding: the bar never asks for a pointer.
    let Some(mut idle) = Rig::start_with("ptr-idle", "", &trace) else {
        return;
    };
    idle.wait_drawn();
    std::thread::sleep(Duration::from_millis(300));
    let log = idle.session.bar_stderr();
    assert!(log.contains("wl_seat"), "the seat is bound: {log}");
    assert!(
        !log.contains(".get_pointer("),
        "a pointer with nothing to do: {log}"
    );
    drop(idle);

    // A binding: it does, and lets go of it when a reload removes the
    // binding.
    let bindings = format!("on-click = {}\n", append("click", "clicks"));
    let Some(mut rig) = Rig::start_with("ptr-needed", &bindings, &trace) else {
        return;
    };
    rig.wait_log(".get_pointer(");
    assert!(!released(&rig), "released before any reload");
    fs::write(&rig.file, config("")).unwrap();
    let reloaded = rig.msg(&["reload"]);
    assert!(reloaded.status.success(), "{reloaded:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !released(&rig) {
        assert!(Instant::now() < deadline, "the pointer was never released");
        std::thread::sleep(Duration::from_millis(20));
    }
    // A click now runs nothing.
    rig.click(CLOCK, "left");
    std::thread::sleep(Duration::from_millis(300));
    assert!(rig.lines("clicks").is_empty());
}

/// The trace shows a `wl_pointer` released.
fn released(rig: &Rig) -> bool {
    rig.session
        .bar_stderr()
        .lines()
        .any(|l| l.contains("wl_pointer@") && l.contains(".release()"))
}

#[test]
fn a_reload_that_adds_a_binding_takes_the_pointer_and_a_bad_one_changes_nothing() {
    let trace = [("WAYLAND_DEBUG", "1")];
    let Some(mut rig) = Rig::start_with("ptr-reload", "", &trace) else {
        return;
    };
    // A misspelled action: refused by name, the running bar untouched.
    fs::write(&rig.file, config("on-click = \"nxt\"\n")).unwrap();
    let refused = rig.msg(&["reload"]);
    assert!(
        !refused.status.success() || String::from_utf8_lossy(&refused.stdout).contains("error")
    );
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(said.contains("clock.on-click"), "{said}");
    assert!(!rig.session.bar_stderr().contains(".get_pointer("));
    // A good one takes the pointer.
    let dir = rig.dir.to_str().unwrap().to_owned();
    let good = config(&format!(
        "on-click = {{ exec = [\"sh\", \"-c\", \"echo reloaded >> {dir}/clicks\"] }}\n"
    ));
    fs::write(&rig.file, good).unwrap();
    let reloaded = rig.msg(&["reload"]);
    assert!(reloaded.status.success(), "{reloaded:?}");
    rig.wait_log(".get_pointer(");
    rig.wait_drawn();
    rig.click(CLOCK, "left");
    assert_eq!(rig.wait_lines("clicks", 1), ["reloaded"]);
}

#[test]
fn a_reload_leaves_the_pointer_where_it_was() {
    // The bar's surface survives a reload that changes no geometry, so the
    // compositor sends no new `enter`: a click with no movement must still
    // reach the new binding.
    let first = format!("on-click = {}\n", append("before", "clicks"));
    let Some(mut rig) = Rig::start("ptr-reload-keeps", &first) else {
        return;
    };
    rig.move_to(CLOCK);
    rig.click(CLOCK, "left");
    rig.wait_lines("clicks", 1);
    let dir = rig.dir.to_str().unwrap().to_owned();
    fs::write(
        &rig.file,
        config(&format!(
            "on-click = {{ exec = [\"sh\", \"-c\", \"echo after >> {dir}/clicks\"] }}\n"
        )),
    )
    .unwrap();
    let reloaded = rig.msg(&["reload"]);
    assert!(reloaded.status.success(), "{reloaded:?}");
    rig.wait_drawn();
    // No pointer_move: a button at the pointer's place.
    rig.button("left", true);
    rig.button("left", false);
    assert_eq!(rig.wait_lines("clicks", 2), ["before", "after"]);
}
