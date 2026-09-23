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
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use scoot_core::{Action, Config, Event, Horizontal, OutputId, WindowId, WindowInfo};
use scoot_ipc::{Request, Response};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_v1,
};

use super::{ScaleReload, autostart_delta, field, scale_reload};
use crate::cli::RendererKind;
use crate::compositor::config;
use crate::compositor::decorations::{Appearance, Color};
use crate::compositor::headless;
use crate::compositor::keybindings::{Bound, Keybindings, Modifiers};
use crate::compositor::state::State;
use crate::compositor::test_support::{
    Harness, assert_marker_never_appears, locker, marker_path, test_renderer, touch_entry,
    wait_for, wait_for_marker,
};

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
        state.scene_dirty = false;
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
fn reload_applies_column_widths_scale_and_refuses_restart_fields() {
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
        commands = ["quit"]

        [binds]
        "super+n" = "close"
        "#,
    );
    let response = fixture.reload();
    // `gap = 12` is the running default, `backend = "pixman"` is the running
    // renderer: agreeing fields stay silent in both lists. `scale = 2.0`
    // applies live now (Phase 3), alongside the widths and the new bind.
    // `quit` is the non-spawn autostart refusal (Phase 4 runs only new
    // spawns, and never a quit); the gpu refusal names restart (Phase 5-6).
    assert_eq!(
        applied(&response),
        &[
            field::COLUMN_WIDTHS.to_owned(),
            field::DEFAULT_COLUMN_WIDTH.to_owned(),
            field::SCALE.to_owned(),
            field::BINDS.to_owned(),
        ],
        "the width and scale fields should apply alongside the new bind: {response:?}"
    );
    for name in [field::GPU, field::AUTOSTART] {
        assert!(
            refused_names(&response).contains(&name),
            "{name} was not refused: {response:?}"
        );
    }
    let gpu = refused(&response)
        .iter()
        .find(|entry| entry.starts_with(field::GPU))
        .expect("the gpu refusal");
    assert!(
        gpu.contains("takes effect on restart"),
        "the gpu refusal should name restart: {gpu}"
    );
    let autostart = refused(&response)
        .iter()
        .find(|entry| entry.starts_with(field::AUTOSTART))
        .expect("the autostart refusal");
    assert!(
        autostart.contains("Quit"),
        "the non-spawn autostart entry should be refused by name: {autostart}"
    );
    assert_eq!(fixture.state.world.config().gap, 12);
    assert_eq!(fixture.state.world.config().column_widths, vec![0.25, 0.75]);
    assert_eq!(fixture.state.world.config().default_column_width, 0);
    assert_eq!(fixture.state.output_scale, 2.0);
    assert_eq!(fixture.state.integer_scale, 2);
    assert!(
        fixture.state.needs_render,
        "a width change reloaded without requesting a render"
    );
}

#[test]
fn reload_refuses_an_xwayland_flip_with_restart_named() {
    // The X server starts once at startup (or never): flipping the knob in
    // the file cannot start or stop one, so the reload refuses -- while an
    // agreeing file stays silent in both lists. The fixture never asked
    // (its snapshot is off), so `enabled = true` is the flip and the bare
    // file is the agreement.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[xwayland]\nenabled = true\n");
    let response = fixture.reload();
    let xwayland = refused(&response)
        .iter()
        .find(|entry| entry.starts_with(field::XWAYLAND))
        .expect("the xwayland refusal");
    assert!(
        xwayland.contains("takes effect on restart"),
        "the xwayland refusal should name restart: {xwayland}"
    );

    fixture.rewrite("");
    let response = fixture.reload();
    assert!(
        !refused_names(&response).contains(&field::XWAYLAND),
        "an agreeing xwayland knob should stay silent: {response:?}"
    );
}

#[test]
fn reload_applies_output_scale_and_halves_the_logical_geometry() {
    // The Phase 3 pin: `scale = 2.0` moves from the file into the live
    // session under its own applied name, the precomputed integer follows,
    // the 200px headless canvas becomes a 100x100 logical desktop, and the
    // core lays out against the halved area -- with a render requested.
    let mut fixture = Fixture::with_config("");
    // A mapped window, so the arrangement the reload recomputes is a real
    // one, not an empty session's.
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

    fixture.rewrite("[output]\nscale = 2.0\n");
    fixture.state.needs_render = false;
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::SCALE.to_owned()],
        "the new scale should apply: {response:?}"
    );
    assert!(
        refused(&response).is_empty(),
        "nothing here should refuse: {response:?}"
    );
    assert_eq!(fixture.state.output_scale, 2.0);
    assert_eq!(fixture.state.integer_scale, 2);
    let output = fixture
        .state
        .outputs
        .primary()
        .expect("the headless output")
        .clone();
    let geometry = fixture
        .state
        .space
        .output_geometry(&output)
        .expect("the mapped output's geometry");
    assert_eq!(
        (geometry.size.w, geometry.size.h),
        (CANVAS / 2, CANVAS / 2),
        "the logical desktop should halve at scale 2"
    );
    let usable = fixture.state.world.usable_areas();
    assert_eq!(usable.len(), 1, "one output, one usable area");
    assert_eq!(
        (usable[0].w, usable[0].h),
        (CANVAS / 2, CANVAS / 2),
        "the core should lay out against the halved area"
    );
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
        "the mapped window should relayout into the smaller desktop: {before} -> {after}"
    );
    assert!(
        fixture.state.needs_render,
        "a scale change reloaded without requesting a render"
    );
}

#[test]
fn a_second_scale_reload_reports_nothing() {
    // The snapshot rule for the scale pair: the first reload stores exactly
    // what it compared (`output_scale` plus its integer), so the second
    // diffs against what the first applied rather than re-reporting it.
    let mut fixture = Fixture::with_config("[output]\nscale = 2.0\n");
    let first = fixture.reload();
    assert_eq!(
        applied(&first),
        &[field::SCALE.to_owned()],
        "the first reload should apply the scale: {first:?}"
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
fn an_out_of_range_scale_applies_as_its_clamped_self() {
    // Invalid values are clamped per the existing parse rules
    // (`into_scale`), never refused: the file asked for 100, the session
    // runs the 4.0 it clamps to, and says applied.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[output]\nscale = 100.0\n");
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::SCALE.to_owned()],
        "the clamped scale should apply: {response:?}"
    );
    assert!(
        refused(&response).is_empty(),
        "clamping is not a refusal: {response:?}"
    );
    assert_eq!(
        fixture.state.output_scale,
        crate::compositor::output_scale::MAX_SCALE
    );
    assert_eq!(fixture.state.integer_scale, 4);
}

#[test]
fn a_non_finite_scale_falls_back_to_one_silently() {
    // TOML can spell `nan`; it resolves to 1.0 like its absence would, so a
    // session already at 1.0 agrees with it in both lists.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[output]\nscale = nan\n");
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty() && refused(&response).is_empty(),
        "a non-finite scale agreeing with the live 1.0 says nothing: {response:?}"
    );
    assert_eq!(fixture.state.output_scale, 1.0);
}

#[test]
fn scale_reloads_round_trip_through_fractional_and_back() {
    // Fractional to integer to fractional, back to back with no settle in
    // between: every step reports applied against the live value the
    // previous step stored, and the integer companion tracks the `ceil`.
    let mut fixture = Fixture::with_config("");
    for (text, scale, integer) in [
        ("[output]\nscale = 1.5\n", 1.5, 2),
        ("[output]\nscale = 2.0\n", 2.0, 2),
        ("[output]\nscale = 1.5\n", 1.5, 2),
        ("[output]\nscale = 1.0\n", 1.0, 1),
    ] {
        fixture.rewrite(text);
        let response = fixture.reload();
        assert_eq!(
            applied(&response),
            &[field::SCALE.to_owned()],
            "each rescale should apply: {response:?}"
        );
        assert_eq!(fixture.state.output_scale, scale);
        assert_eq!(fixture.state.integer_scale, integer);
    }
    fixture.state.needs_render = false;
    let settled = fixture.reload();
    assert!(
        applied(&settled).is_empty() && refused(&settled).is_empty(),
        "the round trip should settle silently: {settled:?}"
    );
    assert!(!fixture.state.needs_render);
}

#[test]
fn rescale_recompacts_outputs_like_a_fresh_session() {
    // Review follow-up pin: two 200px outputs at scale 1 sit at x=0 and
    // x=200. A reload to 2.0 must recompact them to x=0 and x=100 -- what a
    // session started at 2.0 builds -- not preserve x=200 behind a gap, and
    // the trip back to 1.0 must not overlap. A window on the second output
    // follows it both ways.
    let mut fixture = Fixture::with_config("");
    let second = headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a second headless output");
    fixture.state.world.handle_event(Event::WindowOpened {
        id: WindowId(1),
        info: WindowInfo::default(),
        output: Some(second),
        focus: true,
    });
    assert_eq!(origin_of(&fixture.state, OutputId(1)), (0, 0));
    assert_eq!(origin_of(&fixture.state, second), (CANVAS, 0));
    let placed = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the opened window is placed")
        .rect;
    assert!(
        placed.x >= CANVAS,
        "the window should open on the second output: {placed:?}"
    );

    fixture.rewrite("[output]\nscale = 2.0\n");
    let response = fixture.reload();
    assert_eq!(
        applied(&response),
        &[field::SCALE.to_owned()],
        "the rescale should apply: {response:?}"
    );
    assert_eq!(origin_of(&fixture.state, OutputId(1)), (0, 0));
    assert_eq!(
        origin_of(&fixture.state, second),
        (CANVAS / 2, 0),
        "the second output should recompact against the first, like a fresh session at 2.0"
    );
    let moved = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect;
    assert!(
        (CANVAS / 2..CANVAS).contains(&moved.x),
        "the window should follow the second output into its new area: {moved:?}"
    );

    fixture.rewrite("[output]\nscale = 1.0\n");
    let back = fixture.reload();
    assert_eq!(
        applied(&back),
        &[field::SCALE.to_owned()],
        "the trip back should apply too: {back:?}"
    );
    assert_eq!(
        origin_of(&fixture.state, second),
        (CANVAS, 0),
        "the trip back must not leave the outputs overlapping"
    );
    let home = fixture
        .state
        .world
        .arrange()
        .get(WindowId(1))
        .expect("the window is still placed")
        .rect;
    assert!(
        home.x >= CANVAS,
        "the window should follow the second output home: {home:?}"
    );
}

/// Where the `Space` puts `id`'s output -- the position a recompact moves.
fn origin_of(state: &State, id: OutputId) -> (i32, i32) {
    let output = state.outputs.get(id).expect("a known output");
    let geometry = state
        .space
        .output_geometry(output)
        .expect("a mapped output's geometry");
    (geometry.loc.x, geometry.loc.y)
}

#[test]
fn scale_reload_decides_apply_agree_and_nested_refusal() {
    // The pure decision pin: a change applies, an agreement stays silent,
    // and under `--nested` any change refuses (the host owns the scale).
    // Pure so the nested refusal pins without a host connection, which no
    // test harness can fake.
    assert_eq!(scale_reload(2.0, 1.0, false), ScaleReload::Apply);
    assert_eq!(scale_reload(1.5, 1.0, false), ScaleReload::Apply);
    assert_eq!(scale_reload(1.0, 1.0, false), ScaleReload::Agree);
    assert_eq!(scale_reload(2.0, 2.0, false), ScaleReload::Agree);
    assert_eq!(scale_reload(2.0, 1.0, true), ScaleReload::RefuseNested);
    assert_eq!(scale_reload(1.0, 1.0, true), ScaleReload::Agree);
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
    // A cursor change like any other (`State::cursor_changed`): news for
    // every capture that asked for the pointer, and no scene change. This
    // backend's frames do not draw the cursor, so nothing is re-rendered.
    assert_ne!(fixture.state.cursor_serial, 0, "the cursor serial moved");
    assert!(
        !fixture.state.scene_dirty,
        "a cursor reload is not a scene change"
    );
    assert!(
        !fixture.state.needs_render,
        "headless frames never draw the cursor: nothing to redraw"
    );
}

#[test]
fn a_cursor_reload_redraws_where_frames_draw_the_cursor() {
    // The `--tty` shape (the frame seam): the rebuilt cursor has to reach
    // the screen, through the cursor's own redraw.
    let mut fixture = Fixture::with_config("");
    fixture.state.frame_cursor_for_test = Some(true);
    fixture.rewrite(
        r##"
        [appearance]
        cursor_color = "#ff0000"
        "##,
    );
    let response = fixture.reload();
    assert_eq!(applied(&response), &[field::CURSOR_COLOR.to_owned()]);
    assert!(fixture.state.needs_render, "the rebuilt cursor is redrawn");
    assert!(
        !fixture.state.scene_dirty,
        "as a cursor change, not a scene one"
    );
    assert_ne!(fixture.state.cursor_serial, 0);
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

// -- Phase 4: autostart spawn-delta ------------------------------------------

fn spawn_action(command: &[&str]) -> Action {
    Action::Spawn(command.iter().map(|word| (*word).to_owned()).collect())
}

#[test]
fn new_spawn_entries_run_once_and_a_second_reload_is_silent() {
    // The Phase 4 pin: a reload runs exactly the entries the session has
    // not seen (here: one `touch`), reports them under the field name, and
    // advances the snapshot -- so the identical next reload runs nothing
    // and says nothing.
    let marker = marker_path("delta");
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"{}\"]\n",
        touch_entry(&marker)
    ));
    let response = fixture.reload();
    assert!(
        applied(&response).contains(&field::AUTOSTART.to_owned()),
        "the new spawn entry should apply: {response:?}"
    );
    assert!(
        refused(&response).is_empty(),
        "nothing here should refuse: {response:?}"
    );
    wait_for_marker(&marker);
    assert_eq!(
        fixture.state.startup_autostart,
        vec![spawn_action(&["touch", &marker.to_string_lossy()])],
        "the reload must advance the snapshot past what it ran"
    );

    let second = fixture.reload();
    assert!(
        applied(&second).is_empty() && refused(&second).is_empty(),
        "the second reload changed nothing it was asked to -- and says so: {second:?}"
    );
    assert_marker_never_appears(&marker);
}

#[test]
fn a_reloaded_quit_is_refused_and_applies_nothing() {
    // The policy's sharp edge, pinned before it lands: `quit` parses as a
    // legal autostart entry (startup runs it through `act`), but a reload
    // must never hand it to `act` -- that path ends the session. It is
    // refused by name, applies nothing, and is decided once: the identical
    // next reload is silent.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[autostart]\ncommands = [\"quit\"]\n");
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty(),
        "a reloaded quit must apply nothing: {response:?}"
    );
    assert!(
        refused(&response)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART) && entry.contains("Quit")),
        "the quit entry should be refused by name: {response:?}"
    );
    let second = fixture.reload();
    assert!(
        applied(&second).is_empty() && refused(&second).is_empty(),
        "the refused quit is decided, not re-refused: {second:?}"
    );

    // ...and the session is still alive to serve: a later spawn entry
    // applies on the same state a quit reload just passed through.
    let marker = marker_path("after-quit");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"quit\", \"{}\"]\n",
        touch_entry(&marker)
    ));
    let third = fixture.reload();
    assert!(
        applied(&third).contains(&field::AUTOSTART.to_owned()),
        "the session should still serve after a reloaded quit: {third:?}"
    );
    wait_for_marker(&marker);
}

#[test]
fn device_and_renderer_refusals_name_restart() {
    // Phases 5-6: the two fields that never go live keep refusing, but the
    // refusal now names the remedy (a restart) instead of the opaque
    // "startup-only".
    //
    // The renderer named is whichever one this session is *not* running
    // (see `test_renderer`): naming the running one is no change at all, so
    // under `SCOOT_TEST_RENDERER=gles` a hardcoded `gles` would be refused
    // by nothing.
    let mut fixture = Fixture::with_config("");
    let other = match test_renderer() {
        RendererKind::Pixman => RendererKind::Gles,
        RendererKind::Gles => RendererKind::Pixman,
    };
    fixture.rewrite(&format!(
        "[renderer]\nbackend = \"{other}\"\n\n[tty]\ngpu = \"/dev/dri/card9\"\n"
    ));
    let response = fixture.reload();
    assert!(applied(&response).is_empty());
    for name in [field::GPU, field::BACKEND] {
        let entry = refused(&response)
            .iter()
            .find(|entry| entry.starts_with(name))
            .unwrap_or_else(|| panic!("{name} was not refused: {response:?}"))
            .to_owned();
        assert!(
            entry.contains("takes effect on restart"),
            "{name} should name restart as the remedy: {entry}"
        );
    }
}

#[test]
fn autostart_delta_runs_only_unseen_spawns() {
    // The decision underneath the reply: multiset difference against the
    // snapshot, split by variant. Seen entries (in any order) stay silent;
    // unseen spawns run; unseen anything-else refuses.
    let seen = vec![spawn_action(&["waybar"]), Action::CloseFocused];
    let delta = autostart_delta(
        &[
            spawn_action(&["waybar"]),
            Action::CloseFocused,
            spawn_action(&["mako"]),
            Action::Quit,
        ],
        &seen,
    );
    assert_eq!(delta.run, vec![spawn_action(&["mako"])]);
    assert_eq!(delta.refuse, vec![Action::Quit]);
}

#[test]
fn autostart_delta_treats_an_edited_entry_as_new() {
    // Diffing is by value, not by position: editing `mako` into `dunst`
    // retires the old entry and runs the new one. Documented, not inferred
    // -- there is no identity subtler than the action itself to key on.
    let seen = vec![spawn_action(&["mako"])];
    let delta = autostart_delta(&[spawn_action(&["dunst"])], &seen);
    assert_eq!(delta.run, vec![spawn_action(&["dunst"])]);
    assert!(delta.refuse.is_empty());
}

#[test]
fn autostart_delta_runs_a_removed_then_readded_entry_again() {
    // Removal advances the snapshot past the entry (the removal reload
    // itself runs nothing); re-adding it is new again, so it runs again.
    // Explicitly the semantics: there is no "ever ran" memory beyond the
    // last unlocked snapshot.
    let full = vec![spawn_action(&["waybar"]), spawn_action(&["mako"])];

    let removal = autostart_delta(&[spawn_action(&["waybar"])], &full);
    assert!(removal.run.is_empty() && removal.refuse.is_empty());

    let readded = autostart_delta(&full, &[spawn_action(&["waybar"])]);
    assert_eq!(readded.run, vec![spawn_action(&["mako"])]);
}

#[test]
fn autostart_delta_runs_duplicate_entries_per_occurrence() {
    // Two identical entries are two runs at startup, so the delta counts
    // occurrences, not membership: one seen `touch` plus two fresh ones is
    // one run, not zero and not two.
    let seen = vec![spawn_action(&["a"])];
    let delta = autostart_delta(&[spawn_action(&["a"]), spawn_action(&["a"])], &seen);
    assert_eq!(delta.run, vec![spawn_action(&["a"])]);
}

#[test]
fn reload_under_lock_skips_new_spawns_without_advancing_the_snapshot() {
    // The lock half of the policy, against a real lock: new spawn entries
    // neither run (a spawned program at lock time could disclose or
    // interfere) nor are dropped -- the snapshot stays, so the first
    // unlocked reload runs them. Deferred, not denied.
    let mut harness: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "").expect("a config file");
    harness.state.config_path = Some(path.clone());

    let client = harness.spawn(locker);
    harness.wait_for_ack(client);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !harness.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the lock request never landed; the reload below would pass unlocked"
        );
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
    }

    let marker = marker_path("locked");
    fs::write(
        &path,
        format!("[autostart]\ncommands = [\"{}\"]\n", touch_entry(&marker)),
    )
    .expect("a rewritten config file");
    let response = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&response).is_empty(),
        "a locked reload must apply nothing: {response:?}"
    );
    assert!(
        refused(&response)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART) && entry.contains("locked")),
        "the skipped spawn should be refused as locked, not silent: {response:?}"
    );
    harness.settle();
    assert_marker_never_appears(&marker);
    assert!(
        harness.state.startup_autostart.is_empty(),
        "a locked reload must not advance the snapshot -- the entry stays pending"
    );
    assert!(
        harness.state.session_lock.is_locked(),
        "the reload under test must not disturb the lock"
    );
}

// -- Follow-ups: failed-spawn honesty, lock wording, removal cancellation ---

/// A program path no machine provides, so `Command::spawn` fails
/// deterministically (ENOENT) wherever this runs.
const MISSING_PROGRAM: &str = "/nonexistent-scoot-reload-probe";

#[test]
fn a_failed_spawn_is_refused_and_stays_pending_for_the_next_reload() {
    // Finding 1, fail-first: the old code reported `applied` and marked the
    // entry seen, so the failure lived only in the log and no later reload
    // ever retried it. Now the reply refuses the entry by name, the
    // snapshot does not advance past it, and the identical next reload
    // retries (and refuses again) rather than going silent.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"spawn {MISSING_PROGRAM}\"]\n"
    ));
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty(),
        "nothing started, so nothing may report applied: {response:?}"
    );
    assert!(
        refused(&response)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART)
                && entry.contains("Spawn")
                && entry.contains("retried")),
        "the failed entry should be refused by name, still pending: {response:?}"
    );
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "a failed spawn must not advance the snapshot -- it stays pending"
    );

    let second = fixture.reload();
    assert!(
        applied(&second).is_empty(),
        "the retry also started nothing: {second:?}"
    );
    assert!(
        refused(&second)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART) && entry.contains("Spawn")),
        "the still-failing entry is retried, not silently dropped: {second:?}"
    );
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "two failures advance nothing"
    );
}

#[test]
fn a_failed_spawn_runs_once_its_program_appears() {
    // The other half of finding 1: pending really means pending. The entry
    // fails while its program is missing, then runs exactly once after an
    // executable appears at that path -- decided on success, silent after.
    let program = marker_path("late-prog");
    let marker = marker_path("late-marker");
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"spawn {}\"]\n",
        program.display()
    ));
    let first = fixture.reload();
    assert!(
        applied(&first).is_empty()
            && refused(&first)
                .iter()
                .any(|entry| entry.starts_with(field::AUTOSTART)),
        "missing program: refused, not applied: {first:?}"
    );

    fs::write(&program, format!("#!/bin/sh\ntouch {}\n", marker.display()))
        .expect("a probe program");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755))
            .expect("an executable probe program");
    }

    let second = fixture.reload();
    assert!(
        applied(&second).contains(&field::AUTOSTART.to_owned()),
        "with the program present the pending entry should run: {second:?}"
    );
    wait_for_marker(&marker);
    let program_text = program.to_string_lossy().into_owned();
    assert_eq!(
        fixture.state.startup_autostart,
        vec![spawn_action(&[&program_text])],
        "success decides the entry"
    );

    let third = fixture.reload();
    assert!(
        applied(&third).is_empty() && refused(&third).is_empty(),
        "the entry ran once and stays decided: {third:?}"
    );
    let _ = fs::remove_file(&program);
}

#[test]
fn a_spawn_failing_mid_list_does_not_stop_later_entries() {
    // Bug bash: one bad entry must cost only itself. Both neighbours run,
    // the reply mixes applied (for what ran) with a refusal naming the
    // failure, the snapshot advances past exactly the two that ran, and the
    // next reload retries only the failure.
    let first = marker_path("mid-first");
    let last = marker_path("mid-last");
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"{}\", \"spawn {MISSING_PROGRAM}\", \"{}\"]\n",
        touch_entry(&first),
        touch_entry(&last)
    ));
    let response = fixture.reload();
    wait_for_marker(&first);
    wait_for_marker(&last);
    assert!(
        applied(&response).contains(&field::AUTOSTART.to_owned()),
        "the entries that ran should apply: {response:?}"
    );
    assert_eq!(
        refused(&response)
            .iter()
            .filter(|entry| entry.starts_with(field::AUTOSTART))
            .count(),
        1,
        "exactly the failing entry refuses: {response:?}"
    );
    assert_eq!(
        fixture.state.startup_autostart,
        vec![
            spawn_action(&["touch", &first.to_string_lossy()]),
            spawn_action(&["touch", &last.to_string_lossy()]),
        ],
        "the snapshot advances past what ran, not what failed"
    );

    let second = fixture.reload();
    assert!(
        applied(&second).is_empty(),
        "the retry runs nothing new: {second:?}"
    );
    assert_eq!(
        refused(&second)
            .iter()
            .filter(|entry| entry.starts_with(field::AUTOSTART))
            .count(),
        1,
        "only the failure retries: {second:?}"
    );
}

#[test]
fn duplicate_failing_entries_are_each_refused_and_each_retried() {
    // Bug bash: per-occurrence counting cuts both ways. Two identical bad
    // entries are two attempts (matching the duplicate-runs-twice rule the
    // delta pins above), two refusals, and two pending retries -- not one
    // of each, and not a silent second occurrence.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!(
        "[autostart]\ncommands = [\"spawn {MISSING_PROGRAM}\", \"spawn {MISSING_PROGRAM}\"]\n"
    ));
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty(),
        "nothing started: {response:?}"
    );
    assert_eq!(
        refused(&response)
            .iter()
            .filter(|entry| entry.starts_with(field::AUTOSTART))
            .count(),
        2,
        "each failing occurrence refuses: {response:?}"
    );
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "neither occurrence advances the snapshot"
    );

    let second = fixture.reload();
    assert_eq!(
        refused(&second)
            .iter()
            .filter(|entry| entry.starts_with(field::AUTOSTART))
            .count(),
        2,
        "both occurrences retry: {second:?}"
    );
}

#[test]
fn entries_without_a_command_never_reach_the_delta() {
    // Bug bash: `""` has no tokens and bare `spawn` names no command, so
    // both are load-time skips (fail-open, like every malformed entry) --
    // the file loads with no autostart at all and the reload is silent in
    // both lists. No spawn is attempted, so there is nothing to refuse.
    let mut fixture = Fixture::with_config("");
    fixture.rewrite("[autostart]\ncommands = [\"\", \"spawn\"]\n");
    let response = fixture.reload();
    assert!(
        applied(&response).is_empty() && refused(&response).is_empty(),
        "command-less entries are skipped at load, never delta entries: {response:?}"
    );
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "nothing parsed, nothing to decide"
    );
}

/// A session-lock client that unlocks on demand. The shared [`locker`]
/// parks holding the lock until the harness ends it (an abandoned lock
/// stays locked by design); these sequences need a real
/// `unlock_and_destroy` mid-test, so they bring their own script on the
/// same `Harness<(), ()>` shape: lock, ack, then park until the test sends
/// a step, at which point unlock, ack, and exit. A dropped step channel
/// (the harness ending the test still locked) is a clean exit too.
struct UnlockingLocker {
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    locked: bool,
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_registry::WlRegistry, ()>
    for UnlockingLocker
{
    fn event(
        client: &mut Self,
        registry: &wayland_client::protocol::wl_registry::WlRegistry,
        event: wayland_client::protocol::wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wayland_client::protocol::wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface.as_str() == "ext_session_lock_manager_v1"
        {
            client.manager = Some(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl wayland_client::Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()>
    for UnlockingLocker
{
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: ext_session_lock_manager_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

impl wayland_client::Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for UnlockingLocker {
    fn event(
        client: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            client.locked = true;
        }
    }
}

fn unlocking_locker(
    stream: UnixStream,
    steps: Receiver<()>,
    acks: Sender<()>,
) -> Result<(), String> {
    use wayland_client::Connection;
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut client = UnlockingLocker {
        manager: None,
        locked: false,
    };
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let manager = client
        .manager
        .take()
        .ok_or("no ext_session_lock_manager_v1")?;
    let lock = manager.lock(&qh, ());
    queue.flush().map_err(|e| e.to_string())?;
    acks.send(()).map_err(|e| e.to_string())?;
    // Parked holding the lock until the test sends the unlock step -- or
    // the harness ends the test, which drops the step channel.
    if steps.recv().is_err() {
        return Ok(());
    }
    // Smithay only routes `unlock_and_destroy` once `locked` has been
    // sent, and headless confirms on a drawn frame -- so wait for the
    // confirmation event rather than racing it. The test side requested a
    // render before sending the step; these round trips are what let the
    // compositor draw it.
    wait_for(
        &mut queue,
        &mut client,
        "the session lock confirmation",
        |seen| seen.locked.then_some(()),
    )?;
    lock.unlock_and_destroy();
    queue.flush().map_err(|e| e.to_string())?;
    acks.send(()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Locks the session with a real lock client (the [`unlocking_locker`]
/// script, which the test later unlocks), dispatching until the lock
/// lands -- the same wait the parked-`locker` tests inline.
fn lock_with(harness: &mut Harness<(), ()>) -> usize {
    let client = harness.spawn(unlocking_locker);
    harness.wait_for_ack(client);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !harness.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the lock request never landed; the reload below would pass unlocked"
        );
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
    }
    client
}

/// Sends the unlock step and dispatches until the session is unlocked.
///
/// Requests a render first: these sequences map no lock surface, and the
/// lock only confirms (sending `locked`, which Smithay requires before it
/// routes `unlock_and_destroy`) on a drawn frame.
fn unlock(harness: &mut Harness<(), ()>, client: usize) {
    harness.state.request_render();
    harness.send_step(client, ());
    harness.wait_for_ack(client);
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state.session_lock.is_locked() {
        assert!(
            Instant::now() < deadline,
            "the unlock never landed; the reload below would pass locked"
        );
        harness
            .event_loop
            .dispatch(Some(Duration::from_millis(5)), &mut harness.state)
            .expect("a compositor dispatch");
    }
}

fn lock_harness() -> (Harness<(), ()>, PathBuf, tempfile::TempDir) {
    let mut harness: Harness<(), ()> = Harness::headless(Appearance::default(), CANVAS);
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    fs::write(&path, "").expect("a config file");
    harness.state.config_path = Some(path.clone());
    (harness, path, dir)
}

#[test]
fn a_quit_only_locked_reload_defers_without_promising_a_run() {
    // Finding 2, fail-first: the old skip message promised "new entries run
    // on the first unlocked reload", which is strictly false for a quit --
    // on unlock it lands in `refuse`, never runs. The reworded message
    // promises only that pending entries are *decided*. Pinned end to end:
    // quit-only delta under lock, unlock, then the quit refuses by name
    // while the session keeps serving.
    let (mut harness, path, _dir) = lock_harness();
    let client = lock_with(&mut harness);

    fs::write(&path, "[autostart]\ncommands = [\"quit\"]\n").expect("a rewritten config file");
    let response = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&response).is_empty(),
        "a locked reload must apply nothing: {response:?}"
    );
    let entry = refused(&response)
        .iter()
        .find(|entry| entry.starts_with(field::AUTOSTART))
        .unwrap_or_else(|| panic!("the skipped delta should be refused as locked: {response:?}"))
        .to_owned();
    assert!(
        entry.contains("locked") && entry.contains("decided"),
        "the skip should name the lock and promise only a decision: {entry}"
    );
    assert!(
        !entry.contains("run on the first"),
        "a quit never runs, so the message must not promise a run: {entry}"
    );
    assert!(
        harness.state.startup_autostart.is_empty(),
        "a locked reload must not advance the snapshot -- the quit stays pending"
    );

    unlock(&mut harness, client);
    let unlocked = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&unlocked).is_empty(),
        "a reloaded quit must apply nothing: {unlocked:?}"
    );
    assert!(
        refused(&unlocked)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART) && entry.contains("Quit")),
        "on unlock the quit is decided as a refusal by name: {unlocked:?}"
    );

    // ...and the session is still alive to serve: a later spawn entry
    // applies on the same state the quit sequence just passed through.
    let marker = marker_path("quit-locked-alive");
    fs::write(
        &path,
        format!(
            "[autostart]\ncommands = [\"quit\", \"{}\"]\n",
            touch_entry(&marker)
        ),
    )
    .expect("a rewritten config file");
    let third = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&third).contains(&field::AUTOSTART.to_owned()),
        "the session should still serve after the quit sequence: {third:?}"
    );
    wait_for_marker(&marker);
}

#[test]
fn a_removed_while_locked_entry_never_runs() {
    // Finding 3: the three-step cancellation the lock-skip exists to
    // protect. Add-while-locked defers (snapshot frozen), remove-while-
    // locked recomputes the delta to nothing (silent, not a skip -- there
    // is nothing actionable left), and after unlock the entry still never
    // runs: nothing was ever queued.
    let (mut harness, path, _dir) = lock_harness();
    let client = lock_with(&mut harness);

    let marker = marker_path("removed-locked");
    fs::write(
        &path,
        format!("[autostart]\ncommands = [\"{}\"]\n", touch_entry(&marker)),
    )
    .expect("a rewritten config file");
    let added = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&added).is_empty(),
        "a locked reload must apply nothing: {added:?}"
    );
    assert!(
        refused(&added)
            .iter()
            .any(|entry| entry.starts_with(field::AUTOSTART) && entry.contains("locked")),
        "the skipped spawn should be refused as locked, not silent: {added:?}"
    );
    assert!(
        harness.state.startup_autostart.is_empty(),
        "a locked reload must not advance the snapshot -- the entry stays pending"
    );

    fs::write(&path, "[autostart]\ncommands = []\n").expect("a rewritten config file");
    let removed = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&removed).is_empty() && refused(&removed).is_empty(),
        "with the entry gone there is nothing actionable to skip: {removed:?}"
    );
    assert!(
        harness.state.startup_autostart.is_empty(),
        "the removal must not mark anything seen"
    );

    unlock(&mut harness, client);
    let after = harness.state.handle_request(Request::Reload);
    assert!(
        applied(&after).is_empty() && refused(&after).is_empty(),
        "after unlock the cancelled entry stays silent: {after:?}"
    );
    harness.settle();
    assert_marker_never_appears(&marker);
    assert!(
        harness.state.startup_autostart.is_empty(),
        "a cancelled entry never enters the snapshot"
    );
    assert!(
        !harness.state.session_lock.is_locked(),
        "the sequence must end unlocked"
    );
}

#[test]
fn a_removed_then_readded_entry_runs_again() {
    // The snapshot tracks the live file, not history: add runs, removal is
    // silent and shrinks the snapshot, and re-adding the identical entry
    // runs it again. Regression pin for a grow-only snapshot (extend
    // without shrink), which made the re-add silent forever: removal
    // memory must never outlive the file.
    let marker = marker_path("readded");
    let entry = touch_entry(&marker);
    let mut fixture = Fixture::with_config("");
    fixture.rewrite(&format!("[autostart]\ncommands = [\"{entry}\"]\n"));
    let added = fixture.reload();
    assert!(
        applied(&added).contains(&field::AUTOSTART.to_owned()),
        "the new entry should apply: {added:?}"
    );
    wait_for_marker(&marker);

    fixture.rewrite("[autostart]\ncommands = []\n");
    let removed = fixture.reload();
    assert!(
        applied(&removed).is_empty() && refused(&removed).is_empty(),
        "removal runs nothing and refuses nothing: {removed:?}"
    );
    assert!(
        fixture.state.startup_autostart.is_empty(),
        "removal must shrink the snapshot -- nothing is remembered past the file"
    );

    fixture.rewrite(&format!("[autostart]\ncommands = [\"{entry}\"]\n"));
    let readded = fixture.reload();
    assert!(
        applied(&readded).contains(&field::AUTOSTART.to_owned()),
        "the re-added entry is new again and must run: {readded:?}"
    );
    wait_for_marker(&marker);
}
