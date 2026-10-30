//! The dmenu contract against a headless scoot: every repo call
//! site's flags, the masked password prompt, `--no-run-if-empty`,
//! Escape, `--index`, the stdin bounds, and run mode.
//!
//! Each test starts its own headless scoot (so no test reads the
//! machine's desktop files) and drives the real `scootlaunch` binary
//! with piped stdin. Skipped without a `scoot` binary beside the test
//! binary; `SCOOTLAUNCH_REQUIRE_SCOOT` makes that a failure, so CI
//! cannot go green by skipping. A dmenu pick prints and launches
//! nothing (exit 0); a dismiss exits 1; a usage error exits 2.

#[path = "harness.rs"]
mod harness;

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use scootui::test_support::{Session, Shot};

use harness::{Panel, launch_bin, rgb, session_with_apps, session_with_stubs, wait_marker};

/// Starts a dmenu panel on `session` with `stdin_bytes` already
/// written and closed (small inputs fit the pipe buffer, so the write
/// never blocks), `args` after `--font`, and `env` overrides.
fn start_dmenu(
    session: Session,
    data_home: PathBuf,
    stdin_bytes: &[u8],
    args: &[&str],
    env: &[(&str, &str)],
) -> Panel {
    let log = session.scratch.0.join("panel.log");
    let file = fs::File::create(&log).unwrap();
    let empty_dirs = session.scratch.0.join("empty-dirs");
    let mut command = Command::new(launch_bin());
    command
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("WAYLAND_DEBUG")
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .arg("--font")
        .arg(session.font())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(file);
    session.scratch.sandbox(&mut command);
    command
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_DATA_DIRS", &empty_dirs);
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin_bytes).unwrap();
    // Closed: the panel reads to EOF before its first frame.
    drop(child.stdin.take());
    Panel {
        session,
        child,
        log,
        data_home,
    }
}

fn dmenu_session(tag: &str) -> Option<(Session, PathBuf)> {
    session_with_apps(tag, &[])
}

/// Each repo call site's flags, same values: the WiFi and Bluetooth
/// pickers (`--prompt --no-run-if-empty --only-match`), the clipboard
/// picker, and the screencast chooser. One pick each: filter, Enter,
/// the line on stdout, exit 0, nothing launched.
#[test]
fn each_call_sites_flags_pick_a_line() {
    for (tag, prompt) in [
        ("wifi", "wifi: "),
        ("bluetooth", "bluetooth: "),
        ("clipboard", "clipboard: "),
        ("share", "Share: "),
    ] {
        let Some((session, data_home)) = dmenu_session(&format!("call-site-{tag}")) else {
            return;
        };
        let mut panel = start_dmenu(
            session,
            data_home,
            b"alpha\nbeta\ngamma\n",
            &[
                "--dmenu",
                "--prompt",
                prompt,
                "--no-run-if-empty",
                "--only-match",
            ],
            &[],
        );
        panel.wait_shown(rgb("#f9e2af"));
        std::thread::sleep(Duration::from_millis(300));
        panel.type_text("bet");
        std::thread::sleep(Duration::from_millis(300));
        panel.key("Return");
        let status = panel.wait_exit();
        let out = panel.stdout();
        assert!(
            status.success(),
            "{tag}: exit 0 on a pick: {}",
            panel.stderr()
        );
        assert_eq!(out, "beta\n", "{tag}: the pick prints");
        panel.clean();
    }
}

/// The `=` form every call site actually passes (`--prompt='wifi: '`
/// is one word): attached values parse the same.
#[test]
fn attached_flag_values_pick_a_line() {
    let Some((session, data_home)) = dmenu_session("attached-values") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"alpha\nbeta\ngamma\n",
        &[
            "--dmenu",
            "--prompt=wifi: ",
            "--no-run-if-empty",
            "--only-match",
        ],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("gam");
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0: {}", panel.stderr());
    assert_eq!(panel.stdout(), "gamma\n");
    // Dmenu instances neither listen nor signal: no socket file.
    assert!(
        !panel
            .session
            .scratch
            .0
            .join(format!(
                "scootlaunch-{}.sock",
                panel.session.wayland_display
            ))
            .exists(),
        "no toggle socket for dmenu"
    );
    panel.clean();
}

/// Without `--only-match`, Enter with no match prints the typed text
/// (fuzzel's dmenu meaning); with it, Enter does nothing and Escape
/// dismisses with exit 1 and empty stdout.
#[test]
fn only_match_refuses_a_custom_entry() {
    let Some((session, data_home)) = dmenu_session("only-match") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"alpha\nbeta\n",
        &["--dmenu", "--prompt=x: ", "--only-match"],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("zzz-no-such-row");
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Return");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "no match with --only-match: still open"
    );
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "dismiss exits 1");
    assert_eq!(panel.stdout(), "", "no pick prints");
    panel.clean();
}

/// The execute-input exception: Ctrl+Enter prints the typed text even
/// with `--only-match`.
#[test]
fn ctrl_enter_prints_despite_only_match() {
    let Some((session, data_home)) = dmenu_session("ctrl-enter") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"alpha\nbeta\n",
        &["--dmenu", "--only-match"],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("custom words");
    std::thread::sleep(Duration::from_millis(300));
    panel.key("ctrl+Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0: {}", panel.stderr());
    assert_eq!(panel.stdout(), "custom words\n");
    panel.clean();
}

/// `--index` prints the 0-based stdin position, so duplicates print
/// their own occurrence.
#[test]
fn index_prints_the_position() {
    let Some((session, data_home)) = dmenu_session("index") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"same\nsame\nother\n",
        &["--dmenu", "--index"],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.key("Down");
    std::thread::sleep(Duration::from_millis(200));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0: {}", panel.stderr());
    assert_eq!(panel.stdout(), "1\n", "the second occurrence");
    panel.clean();
}

/// `--no-run-if-empty`: empty stdin exits 1 without opening. Without
/// the flag the empty panel still opens, and Escape dismisses it.
#[test]
fn no_run_if_empty_skips_the_panel() {
    let Some((session, data_home)) = dmenu_session("empty-flag") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"",
        &["--dmenu", "--no-run-if-empty"],
        &[],
    );
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "empty stdin exits 1");
    assert_eq!(panel.stdout(), "", "nothing prints");
    panel.clean();

    let Some((session, data_home)) = dmenu_session("empty-open") else {
        return;
    };
    let mut panel = start_dmenu(session, data_home, b"", &["--dmenu"], &[]);
    panel.wait_shown(rgb("#f9e2af"));
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1), "Escape dismisses the empty panel");
    panel.clean();
}

/// Hostile stdin end to end: NUL, invalid UTF-8 and a missing trailing
/// newline neither panic nor wedge, and the pick round-trips (lossy
/// for the invalid bytes, NUL intact).
#[test]
fn hostile_stdin_round_trips() {
    let Some((session, data_home)) = dmenu_session("hostile") else {
        return;
    };
    let input = b"a\0b\nok \xff\xfe\nplain".to_vec();
    // No trailing newline on the last line, on purpose.
    assert!(!input.ends_with(b"\n"));
    let mut panel = start_dmenu(session, data_home, &input, &["--dmenu"], &[]);
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // Empty query shows stdin order: the first row is `a\0b`.
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0: {}", panel.stderr());
    let out = panel.stdout_bytes();
    assert_eq!(out, b"a\0b\n", "NUL rides through");
    panel.clean();
}

/// A 65,536-line stdin stays bounded: what arrived past the line cap
/// is not shown (a query for it matches nothing), while the last kept
/// line still picks.
#[test]
fn sixty_five_thousand_lines_stay_bounded() {
    let mut input = Vec::new();
    for index in 0..70_000 {
        input.extend_from_slice(format!("row-{index:05}\n").as_bytes());
    }
    let Some((session, data_home)) = dmenu_session("bounded") else {
        return;
    };
    // The writer thread feeds the pipe while the panel reads: neither
    // side wedges, and the panel's 5 s bound covers a slow writer.
    let mut panel = spawn_with_writer(session, data_home, input, &["--dmenu", "--only-match"]);
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(500));
    // Past the cap: no match, Enter does nothing, the panel stays open.
    // (`zzzabsent` holds a `z` no line has, so no fuzzy surprise.)
    panel.type_text("zzzabsent");
    std::thread::sleep(Duration::from_millis(500));
    panel.key("Return");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        panel.child.try_wait().unwrap().is_none(),
        "a dropped line matches nothing and prints nothing"
    );
    // The last kept line still picks.
    for _ in 0..10 {
        panel.key("BackSpace");
    }
    panel.type_text("row-65535");
    std::thread::sleep(Duration::from_millis(500));
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "exit 0: {}", panel.stderr());
    assert_eq!(panel.stdout(), "row-65535\n");
    panel.clean();
}

/// Spawns a dmenu panel while a writer thread feeds `input` and closes:
/// the pipe never wedges either side.
fn spawn_with_writer(session: Session, data_home: PathBuf, input: Vec<u8>, args: &[&str]) -> Panel {
    let log = session.scratch.0.join("panel.log");
    let file = fs::File::create(&log).unwrap();
    let empty_dirs = session.scratch.0.join("empty-dirs");
    let mut command = Command::new(launch_bin());
    command
        .env("XDG_RUNTIME_DIR", &session.scratch.0)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("WAYLAND_DEBUG")
        .env("WAYLAND_DISPLAY", &session.wayland_display)
        .arg("--font")
        .arg(session.font())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(file);
    session.scratch.sandbox(&mut command);
    command
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_DATA_DIRS", &empty_dirs);
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    // Closed on drop below: the write end closes when the thread's
    // `stdin` drops, but the panel's 5 s bound covers a slow writer
    // either way.
    Panel {
        session,
        child,
        log,
        data_home,
    }
}

/// Run mode launches the picked executable by basename (exit 0, empty
/// stdout, frecency keyed `run:<basename>`), like an apps pick.
#[test]
fn run_mode_launches_by_basename() {
    let Some((session, data_home, markers)) = session_with_stubs("run", &["mytool"]) else {
        return;
    };
    let bins = session.scratch.0.join("bins");
    let mut panel = start_dmenu(
        session,
        data_home,
        b"",
        &["--run"],
        &[("PATH", bins.to_str().unwrap())],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(500));
    panel.type_text("mytool");
    std::thread::sleep(Duration::from_millis(500));
    panel.key("Return");
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(
        status.success(),
        "exit 0 on a run launch: {}",
        panel.stderr()
    );
    assert_eq!(out, "", "a launch prints nothing");
    let line = wait_marker(&markers[0]);
    assert!(line.contains("mytool.sh"), "{line}");
    // Frecency keyed `run:<basename>`, as the history file's docs say.
    let history = panel
        .session
        .scratch
        .state_home()
        .join("scootlaunch")
        .join("history");
    let text = fs::read_to_string(&history).unwrap_or_default();
    assert!(
        text.contains("run:mytool.sh"),
        "the run is recorded: {text}"
    );
    panel.clean();
}

/// A click on a row picks it at once (exit 0, the line on stdout):
/// the mouse path prints like the keyboard one does.
#[test]
fn a_click_on_a_row_prints_it() {
    let Some((session, data_home)) = dmenu_session("row-click") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"alpha\nbeta\ngamma\n",
        &["--dmenu"],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    // The second row's middle, from the border box: entry at inner 10,
    // rows 21 px from list_y0 = 35.
    let (x0, y0, x1, _) = panel.find_panel(rgb("#f9e2af"));
    panel.click((x0 + x1) / 2, y0 + 35 + 21 + 10);
    let status = panel.wait_exit();
    let out = panel.stdout();
    assert!(
        status.success(),
        "exit 0 on a mouse pick: {}",
        panel.stderr()
    );
    assert_eq!(out, "beta\n", "the row prints");
    panel.clean();
}

/// The WiFi password prompt draws masked: the same-length secrets draw
/// identical pixels (no letterforms), and the secret never reaches the
/// log.
#[test]
fn password_prompt_draws_masked() {
    let Some((session, data_home)) = dmenu_session("password") else {
        return;
    };
    let mut panel = start_dmenu(
        session,
        data_home,
        b"",
        &["--dmenu", "--password", "--prompt-only=key for net: "],
        &[],
    );
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    panel.type_text("iiii");
    std::thread::sleep(Duration::from_millis(300));
    let first = panel.shot();
    // Same length, different widths as letterforms: identical as masks.
    for _ in 0..4 {
        panel.key("BackSpace");
    }
    panel.type_text("wwww");
    std::thread::sleep(Duration::from_millis(300));
    let second = panel.shot();
    assert_same_frame(&first, &second, "the masked entry draws no letterforms");
    // And the mask drew ink at all (not an empty box).
    panel.key("Escape");
    let status = panel.wait_exit();
    assert_eq!(status.code(), Some(1));
    assert!(
        !panel.stderr().contains("iiii") && !panel.stderr().contains("wwww"),
        "the secret never reaches the log: {}",
        panel.stderr()
    );
    panel.clean();
}

/// Two screenshots of identical content are pixel-identical: the
/// compositor renders the same scene the same way, so any difference
/// is the panel drawing differently.
fn assert_same_frame(first: &Shot, second: &Shot, what: &str) {
    assert_eq!(
        (first.width, first.height),
        (second.width, second.height),
        "{what}: frame sizes differ"
    );
    for y in 0..first.height {
        for x in 0..first.width {
            assert_eq!(
                first.at(x, y),
                second.at(x, y),
                "{what}: pixel ({x},{y}) differs"
            );
        }
    }
}

trait StdoutBytes {
    fn stdout_bytes(&mut self) -> Vec<u8>;
}

impl StdoutBytes for Panel {
    fn stdout_bytes(&mut self) -> Vec<u8> {
        use std::io::Read as _;
        let mut out = Vec::new();
        self.child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        out
    }
}

/// The look's shadow and motion are the apps and run panel's: a dmenu
/// panel under the same look draws with its fixed painter, prints at once
/// and exits at once (a script waiting on its pick never waits on a fade).
#[test]
fn a_look_with_a_shadow_and_motion_leaves_dmenu_alone() {
    let Some((session, data_home)) = dmenu_session("dmenu-effects") else {
        return;
    };
    let look_dir = session.scratch.state_home().join("scoot").join("look");
    fs::create_dir_all(&look_dir).unwrap();
    fs::write(
        look_dir.join("launch.toml"),
        "shadow = \"deep\"\nfill = \"gradient\"\nmotion = \"spring\"\nmotion-duration-ms = 900\n",
    )
    .unwrap();
    let mut panel = start_dmenu(
        session,
        data_home,
        b"alpha\nbeta\ngamma\n",
        &["--dmenu"],
        &[],
    );
    // The border is there at full strength at once: no entrance.
    panel.wait_shown(rgb("#f9e2af"));
    std::thread::sleep(Duration::from_millis(300));
    let sent = std::time::Instant::now();
    panel.key("Return");
    let status = panel.wait_exit();
    assert!(status.success(), "{}", panel.stderr());
    assert_eq!(panel.stdout(), "alpha\n");
    assert!(
        sent.elapsed() < Duration::from_millis(700),
        "the pick did not wait for a fade: {:?}",
        sent.elapsed()
    );
    panel.clean();
}
