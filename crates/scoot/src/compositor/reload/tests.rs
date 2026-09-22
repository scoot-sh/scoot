//! Serving `Request::Reload` against a live `State`: what applies, what
//! refuses, and what a failed reload leaves behind.
//!
//! The unknown-request half of the wire contract (an older server meeting a
//! new `reload` client with an error, not a kill) is pinned in
//! `scoot-ipc/tests/wire.rs` and in `connection`'s decode-failure arm, not
//! here: by the time `handle_request` runs, the request has decoded.
//!
//! The two held-key pins live in `input/tests.rs` next to the
//! `suppressed_keys` mechanism they exercise (they need `resolve_combo`,
//! which is private to `input`); the `--tty` VT-guard pin for a live table
//! is impossible without hardware, so it is pinned purely through
//! `keybindings_for` below instead.

use std::fs;
use std::path::PathBuf;

use scoot_core::{Action, Config, Horizontal};
use scoot_ipc::{Request, Response};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use super::field;
use crate::compositor::config;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless;
use crate::compositor::keybindings::{Bound, Keybindings, Modifiers};
use crate::compositor::state::State;
use crate::compositor::test_support::test_renderer;

const CANVAS: i32 = 200;

/// A live `State` on a headless backend with `config_path` pointed at a
/// temp file holding `contents` -- the shape `input/tests.rs` builds its
/// `State` in, minus the clients (no test here needs one).
struct Fixture {
    state: State,
    path: PathBuf,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn with_config(contents: &str) -> Self {
        let mut event_loop: EventLoop<'static, State> =
            EventLoop::try_new().expect("an event loop");
        let display: Display<State> = Display::new().expect("a wayland display");
        let mut state = State::new(
            &mut event_loop,
            display,
            Config::default(),
            Keybindings::default(),
            Appearance::default(),
            1.0,
            test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");

        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, contents).expect("a config file");
        state.config_path = Some(path.clone());
        // Quiet the render the startup `apply` requested, so `needs_render`
        // below witnesses the reload's own `apply` and nothing else.
        state.needs_render = false;
        Self {
            state,
            path,
            _dir: dir,
        }
    }

    fn rewrite(&self, contents: &str) {
        fs::write(&self.path, contents).expect("a rewritten config file");
    }

    fn reload(&mut self) -> Response {
        self.state.handle_request(Request::Reload)
    }
}

fn applied(response: &Response) -> &[String] {
    match response {
        Response::Reloaded { applied, .. } => applied,
        other => panic!("expected a reload report, got {other:?}"),
    }
}

fn refused(response: &Response) -> &[String] {
    match response {
        Response::Reloaded { refused, .. } => refused,
        other => panic!("expected a reload report, got {other:?}"),
    }
}

fn refused_names(response: &Response) -> Vec<&str> {
    refused(response)
        .iter()
        .map(|entry| entry.split(' ').next().expect("a field name"))
        .collect()
}

#[test]
fn reload_applies_gap_appearance_and_binds_and_lists_them() {
    let mut fixture = Fixture::with_config("");
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty() && refused(&response).is_empty(),
        "an empty file agrees with the defaults it produced: {response:?}"
    );

    fixture.rewrite(
        r##"
        [layout]
        gap = 20

        [appearance]
        background_color = "#ff0000"
        corner_radius = 12

        [binds]
        "super+n" = "focus-column right"
        "##,
    );
    let response = fixture.reload();
    for name in [
        field::GAP,
        field::BACKGROUND,
        field::CORNER_RADIUS,
        field::BINDS,
    ] {
        assert!(
            applied(&response).contains(&name.to_owned()),
            "{name} was not reported applied: {response:?}"
        );
    }
    assert!(
        refused(&response).is_empty(),
        "nothing here should refuse: {response:?}"
    );
    assert_eq!(fixture.state.world.config().gap, 20);
    assert_eq!(
        fixture.state.appearance.background_color,
        Color::parse("#ff0000").expect("a parsable color")
    );
    assert_eq!(fixture.state.appearance.corner_radius, 12);
    assert_eq!(
        fixture.state.keybindings.match_key(
            crate::compositor::input::keysym_named("n").expect("a named key"),
            Modifiers {
                super_: true,
                ..Modifiers::default()
            }
        ),
        Some(Bound::Action(Action::FocusColumn(Horizontal::Right))),
        "the reloaded bind does not fire"
    );
    assert!(
        fixture.state.needs_render,
        "a visible change reloaded without requesting a render"
    );
}

#[test]
fn reload_refuses_startup_only_fields_and_moves_nothing_else() {
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(
        r#"
        [layout]
        gap = 12
        column_widths = [0.25, 0.75]
        default_column_width = 0

        [output]
        scale = 2.0

        [renderer]
        backend = "pixman"

        [tty]
        gpu = "/dev/dri/card9"

        [autostart]
        commands = ["spawn waybar"]

        [binds]
        "super+n" = "close"
        "#,
    );
    let response = fixture.reload();
    // `gap = 12` is the running default, `backend = "pixman"` is the running
    // renderer: agreeing fields stay silent in both lists.
    assert_eq!(
        applied(&response),
        &[field::BINDS.to_owned()],
        "only the new bind should apply: {response:?}"
    );
    for name in [
        field::COLUMN_WIDTHS,
        field::DEFAULT_COLUMN_WIDTH,
        field::SCALE,
        field::GPU,
        field::AUTOSTART,
    ] {
        assert!(
            refused_names(&response).contains(&name),
            "{name} was not refused: {response:?}"
        );
    }
    assert_eq!(fixture.state.world.config().gap, 12);
    assert_eq!(
        fixture.state.world.config().column_widths,
        Config::default().column_widths
    );
    assert_eq!(fixture.state.output_scale, 1.0);
    assert!(
        !fixture.state.needs_render,
        "a refused-only reload must not reconfigure or redraw"
    );
}

#[test]
fn reload_rebuilds_the_cursor_and_reports_each_field() {
    // The Phase 1 pin: all three cursor fields move from the file into the
    // live session, each under its own applied name, with the rendered
    // cursor rebuilt behind them -- and a render requested without a
    // re-arrange (no placement changed, so `apply()` must not run its
    // configure round-trip; `needs_render` witnesses the request).
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(
        r##"
        [appearance]
        cursor_size = 24
        cursor_color = "#ff0000"
        cursor_theme = "Adwaita"
        "##,
    );
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[
            field::CURSOR_SIZE.to_owned(),
            field::CURSOR_COLOR.to_owned(),
            field::CURSOR_THEME.to_owned(),
        ],
        "each changed cursor field reports applied: {response:?}"
    );
    assert!(
        refused(&response).is_empty(),
        "nothing here should refuse: {response:?}"
    );
    assert_eq!(fixture.state.appearance.cursor_size, 24);
    assert_eq!(
        fixture.state.appearance.cursor_color,
        Color::parse("#ff0000").expect("a parsable color")
    );
    assert_eq!(
        fixture.state.appearance.cursor_theme.as_deref(),
        Some("Adwaita")
    );
    assert_eq!(
        fixture.state.cursor.theme().name(),
        "Adwaita",
        "the rebuilt cursor resolves the reloaded theme name"
    );
    assert_eq!(
        fixture.state.cursor.theme().size(),
        24,
        "the rebuilt cursor picks theme images at the reloaded size"
    );
    assert!(
        fixture.state.needs_render,
        "a cursor change reloaded without requesting a render"
    );
}

#[test]
fn a_second_reload_after_a_cursor_change_reports_nothing() {
    // The snapshot rule for the cursor triple (Phase 0): the first reload
    // writes `State::appearance` alongside the rebuild, so the second diffs
    // against what the first applied rather than re-reporting it.
    let mut fixture = Fixture::with_config(
        r##"
        [appearance]
        cursor_size = 24
        cursor_color = "#ff0000"
        cursor_theme = "Adwaita"
        "##,
    );
    let first = fixture.reload();
    assert!(
        applied(&first).contains(&field::CURSOR_SIZE.to_owned()),
        "the first reload should apply the cursor: {first:?}"
    );
    // Quiet the render the first reload requested, so `needs_render` below
    // witnesses the second reload's own behavior and nothing else.
    fixture.state.needs_render = false;
    let second = fixture.reload();
    assert!(
        applied(&second).is_empty() && refused(&second).is_empty(),
        "the second reload changed nothing it was asked to -- and says so: {second:?}"
    );
    assert!(
        !fixture.state.needs_render,
        "an idempotent second reload must not request a render"
    );
}

#[test]
fn reload_with_an_unresolvable_cursor_theme_still_applies() {
    // `Theme::load` never fails: a name that matches nothing installed is
    // an empty theme drawn as the fallback shapes, not an error and not a
    // half-applied session. The config still differs, so it still reports
    // applied -- and the next reload agrees silently.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(
        r#"
        [appearance]
        cursor_theme = "a-theme-that-exists-nowhere"
        "#,
    );
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::CURSOR_THEME.to_owned()],
        "an unresolvable theme still applies: {response:?}"
    );
    assert!(
        !fixture.state.cursor.theme().is_loaded(),
        "nothing installed under that name, so no theme should be loaded"
    );
    assert_eq!(
        fixture.state.appearance.cursor_theme.as_deref(),
        Some("a-theme-that-exists-nowhere")
    );
    let second = fixture.reload();
    assert!(
        applied(&second).is_empty() && refused(&second).is_empty(),
        "the missing theme must not re-report on the next reload: {second:?}"
    );
}

#[test]
fn a_malformed_file_keeps_the_running_config() {
    // Fail-first for the load-bearing failure semantic: a typo mid-session
    // must cost nothing, where at startup the same file would fall back to
    // defaults (see `config.rs`'s module doc for why those differ).
    let mut fixture = Fixture::with_config("[layout]\ngap = 20\n");
    let first = fixture.reload();
    assert!(applied(&first).contains(&field::GAP.to_owned()));
    assert_eq!(fixture.state.world.config().gap, 20);

    fixture.rewrite("this is not valid toml [[[");
    let response = fixture.reload();
    assert!(
        matches!(response, Response::Error { .. }),
        "a malformed reload should be a loud error: {response:?}"
    );
    assert_eq!(
        fixture.state.world.config().gap,
        20,
        "the failed reload moved the running gap"
    );
}

#[test]
fn an_unknown_field_fails_the_whole_reload_not_just_the_table() {
    // Validate-before-apply at the file level: `deny_unknown_fields`
    // rejects the file before any of it is read, so a typo in `[layout]`
    // cannot half-apply an otherwise-valid `[binds]` from the same file.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(
        r#"
        [layout]
        gaps = 5

        [binds]
        "super+n" = "close"
        "#,
    );
    let response = fixture.reload();
    assert!(
        matches!(response, Response::Error { .. }),
        "an unknown field should fail the reload: {response:?}"
    );
    assert_eq!(
        fixture.state.keybindings.match_key(
            crate::compositor::input::keysym_named("n").expect("a named key"),
            Modifiers {
                super_: true,
                ..Modifiers::default()
            }
        ),
        None,
        "the valid binds table applied despite the file failing validation"
    );
}

#[test]
fn a_vanished_path_keeps_the_running_config() {
    let mut fixture = Fixture::with_config("[layout]\ngap = 20\n");
    assert!(applied(&fixture.reload()).contains(&field::GAP.to_owned()));
    fs::remove_file(&fixture.path).expect("the config file removes");
    let response = fixture.reload();
    assert!(
        matches!(response, Response::Error { .. }),
        "a vanished path should be a loud error: {response:?}"
    );
    assert_eq!(
        fixture.state.world.config().gap,
        20,
        "the failed reload moved the running gap"
    );
}

#[test]
fn no_config_path_is_an_error_not_a_guess() {
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("an event loop");
    let display: Display<State> = Display::new().expect("a wayland display");
    let mut state = State::new(
        &mut event_loop,
        display,
        Config::default(),
        Keybindings::default(),
        Appearance::default(),
        1.0,
        test_renderer(),
    )
    .expect("a compositor state");
    // `config_path` stays `None`: nowhere to reload from.
    let response = state.handle_request(Request::Reload);
    assert!(
        matches!(response, Response::Error { .. }),
        "a pathless reload should refuse, not read a default: {response:?}"
    );
}

#[test]
fn a_second_reload_sees_the_first_ones_results() {
    // Two rapid reloads: the second diffs against what the first applied,
    // so it reports two empty lists rather than re-applying.
    let mut fixture = Fixture::with_config("[layout]\ngap = 20\n");
    let first = fixture.reload();
    assert!(applied(&first).contains(&field::GAP.to_owned()));
    let second = fixture.reload();
    assert!(
        applied(&second).is_empty() && refused(&second).is_empty(),
        "the second reload changed nothing it was asked to -- and says so: {second:?}"
    );
}

#[test]
fn keybindings_for_keeps_the_vt_recovery_path_unstrippable() {
    // The TTY-guard pin, without hardware: a file binding over
    // `Ctrl+Alt+F1` loses to the recovery binding when `vt` is set (the
    // `--tty` session), and wins when it is not (headless/nested never had
    // VT binds to keep).
    let mut binds = std::collections::HashMap::new();
    binds.insert("ctrl+alt+f1".to_owned(), "close".to_owned());
    let tty = config::keybindings_for(&binds, true);
    assert_eq!(
        tty.match_key(
            crate::compositor::input::keysym_named("F1").expect("a named key"),
            Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::default()
            }
        ),
        Some(Bound::ChangeVt(1)),
        "a reload must not strip --tty's VT-switch recovery binding"
    );
    let bare = config::keybindings_for(&binds, false);
    assert_eq!(
        bare.match_key(
            crate::compositor::input::keysym_named("F1").expect("a named key"),
            Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::default()
            }
        ),
        Some(Bound::Action(Action::CloseFocused)),
        "off --tty the user's bind stands (there is no VT path to keep)"
    );
}
