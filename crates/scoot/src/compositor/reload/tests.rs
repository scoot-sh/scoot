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

use scoot_core::{Action, Config, Event, Horizontal, WindowId, WindowInfo};
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
fn reload_applies_column_widths_and_refuses_startup_only_fields() {
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
        &[
            field::COLUMN_WIDTHS.to_owned(),
            field::DEFAULT_COLUMN_WIDTH.to_owned(),
            field::BINDS.to_owned(),
        ],
        "the width fields should apply alongside the new bind: {response:?}"
    );
    for name in [field::SCALE, field::GPU, field::AUTOSTART] {
        assert!(
            refused_names(&response).contains(&name),
            "{name} was not refused: {response:?}"
        );
    }
    assert_eq!(fixture.state.world.config().gap, 12);
    assert_eq!(fixture.state.world.config().column_widths, vec![0.25, 0.75]);
    assert_eq!(fixture.state.world.config().default_column_width, 0);
    assert_eq!(fixture.state.output_scale, 1.0);
    assert!(
        fixture.state.needs_render,
        "a width change reloaded without requesting a render"
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
fn reload_to_a_shorter_list_clamps_a_window_on_a_high_preset() {
    // The load-bearing Phase 2 case: a live column holds preset 2 of three,
    // and the reloaded file keeps only one entry. The column must land on
    // the surviving width rather than indexing out of range in `arrange`,
    // and the very next arrangement (read back here, and pushed to windows
    // by the reload's own `apply()`) already uses the new list.
    let mut fixture = Fixture::with_config("");
    // Open a window (it takes the default preset, index 1 of three), then
    // move its column to the top preset, index 2 -- the one a shorter
    // list would leave out of range.
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(1),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    fixture.state.world.handle_action(Action::SetColumnWidth(2));
    let before = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the opened window is placed")
        .rect
        .w;
    // Default list is `[1/3, 1/2, 2/3]`: preset 2 is the widest of the
    // three, so this guards the setup, not just the reload.
    assert!(
        before > 100,
        "the high-preset window should start wide: {before}"
    );

    fixture.rewrite("[layout]\ncolumn_widths = [0.5]\n");
    fixture.state.needs_render = false;
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::COLUMN_WIDTHS.to_owned()],
        "the shorter list should apply: {response:?}"
    );
    assert!(
        refused(&response).is_empty(),
        "nothing here should refuse: {response:?}"
    );
    // `arrange` stays in range, and the clamped column fills half the
    // 200px headless canvas minus its gap share -- narrower than the 2/3
    // it held, so the arrangement actually moved.
    let after = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect
        .w;
    assert!(
        after < before,
        "the clamped column should narrow: {before} -> {after}"
    );
    assert!(
        fixture.state.needs_render,
        "a width change reloaded without requesting a render"
    );

    // The out-of-range rule for `set-column-width` now runs against the
    // NEW list's length: index 2 is past a one-entry list, so it is
    // ignored without disturbing the column; cycling steps mod 1.
    fixture.state.world.handle_action(Action::SetColumnWidth(2));
    fixture.state.world.handle_action(Action::CycleColumnWidth);
    let settled = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect
        .w;
    assert_eq!(
        settled, after,
        "an out-of-range set and a length-1 cycle must not move the column"
    );
}

#[test]
fn reload_to_a_longer_list_keeps_every_preset_and_moves_the_arrangement() {
    // The other direction: growing the list touches no preset, so every
    // column keeps its width choice while the arrangement still recomputes
    // (here through the default column narrowing under the new first entry
    // -- the file replaces the whole list, not one entry).
    let mut fixture = Fixture::with_config("");
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(1),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    let before = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the opened window is placed")
        .rect
        .w;

    fixture.rewrite("[layout]\ncolumn_widths = [0.25, 0.5, 0.75, 1.0]\n");
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::COLUMN_WIDTHS.to_owned()],
        "the longer list should apply: {response:?}"
    );
    // The window opened on the default preset (index 1, the 0.5 entry);
    // under the new list that same index is still 0.5, so its frame holds
    // while the list the session runs grew.
    let after = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect
        .w;
    assert_eq!(
        after, before,
        "an untouched preset must keep its frame across a growing reload"
    );
    assert_eq!(
        fixture.state.world.config().column_widths,
        vec![0.25, 0.5, 0.75, 1.0]
    );
    // ...and the new entries are live for stepping immediately: cycling
    // from preset 1 lands on the 0.75 entry, wider than before.
    fixture.state.world.handle_action(Action::CycleColumnWidth);
    let cycled = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect
        .w;
    assert!(
        cycled > after,
        "cycling should reach the new 0.75 entry: {after} -> {cycled}"
    );
}

#[test]
fn reload_to_an_empty_list_falls_back_to_the_defaults() {
    // `validated()` repairs an emptied list before the clamp runs, so the
    // session keeps the built-in three instead of a list `arrange` cannot
    // index at all.
    let mut fixture = Fixture::with_config("");
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(1),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    fixture.rewrite("[layout]\ncolumn_widths = []\n");
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::COLUMN_WIDTHS.to_owned()],
        "the emptied list should apply as the fallback: {response:?}"
    );
    assert_eq!(
        fixture.state.world.config().column_widths,
        Config::default().column_widths
    );
    assert!(
        fixture.state.world.arrange().get(WindowId(1)).is_some(),
        "the window must still be placed after the fallback"
    );
}

#[test]
fn a_second_widths_reload_reports_nothing() {
    // The snapshot rule for the width fields: the first reload stores
    // exactly what it compared, so the second diffs against what the first
    // applied rather than re-reporting it.
    let mut fixture =
        Fixture::with_config("[layout]\ncolumn_widths = [0.25, 0.75]\ndefault_column_width = 0\n");
    let first = fixture.reload();
    assert_eq!(
        applied(&first),
        &[
            field::COLUMN_WIDTHS.to_owned(),
            field::DEFAULT_COLUMN_WIDTH.to_owned(),
        ],
        "the first reload should apply the widths: {first:?}"
    );
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
fn a_reloaded_default_column_width_serves_new_windows() {
    // `default_column_width` alone: read only at creation, so live columns
    // hold still while the next opened window takes the new default.
    let mut fixture = Fixture::with_config("");
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(1),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    let before = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the opened window is placed")
        .rect
        .w;

    fixture.rewrite("[layout]\ndefault_column_width = 0\n");
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::DEFAULT_COLUMN_WIDTH.to_owned()],
        "the default alone should apply: {response:?}"
    );
    // The live window keeps its preset-1 frame...
    let held = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect
        .w;
    assert_eq!(
        held, before,
        "a default-only reload must not move live columns"
    );
    // ...while a window opened after the reload takes preset 0 (1/3),
    // narrower than the preset-1 (1/2) frame above.
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(2),
        info: WindowInfo::default(),
        output: None,
        focus: true,
    });
    let next = fixture
        .state
        .world
        .arrange()
        .get(WindowId(2))
        .expect("the new window is placed")
        .rect
        .w;
    assert!(
        next < held,
        "the new window should take the narrower default: {held} -> {next}"
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
