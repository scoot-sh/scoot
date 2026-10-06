//! Serving `Request::OutputScale` against a live `State`: what moves, what
//! refuses, and what a `reload` leaves behind.
//!
//! The unknown-request half of the wire contract (an older server meeting a
//! new `output-scale` client with an error, not a kill) is pinned in
//! `scoot-ipc/tests/wire.rs`, not here: by the time `handle_request` runs,
//! the request has decoded. What lives here is the handler's own contract:
//! the scale lands in `msg outputs` (rect and scale), bad input is refused
//! with a reason, a repeat set is a silent no-op, a reset goes back to the
//! config file's scale, a successful reload restores the config file's
//! scale and reports each runtime scale it moved (a failed one keeps the
//! live scale), and a replugged monitor comes back at its runtime scale by
//! name.

use std::fs;

use scoot_core::{Config, OutputId};
use scoot_ipc::{OutputTarget, Request, Response};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;
use crate::compositor::state::State;
use crate::compositor::test_support::test_renderer;

const CANVAS: i32 = 200;

/// A live `State` on a headless backend -- the shape `reload/tests.rs`
/// builds its `State` in, minus the reload path (which the restore test
/// wires up itself).
struct Fixture {
    state: State,
}

impl Fixture {
    fn new() -> Self {
        Self::with_outputs(1)
    }

    fn with_outputs(count: u32) -> Self {
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
        for index in 2..=count {
            headless::add_output(&mut state, &format!("headless-{index}"), CANVAS, CANVAS)
                .expect("another headless output");
        }
        // Quiet the render the startup `apply` requested, so `needs_render`
        // below witnesses the request's own rescale and nothing else.
        state.needs_render = false;
        Self { state }
    }

    /// Points the session's reload path at a temp file holding `contents`,
    /// returning the dir (which must outlive the test).
    fn install_config(&mut self, contents: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("config.toml");
        fs::write(&path, contents).expect("a config file");
        self.state.config_path = Some(path);
        dir
    }

    fn outputs(&mut self) -> Vec<scoot_ipc::OutputSnapshot> {
        match self.state.handle_request(Request::Outputs) {
            Response::Outputs { outputs } => outputs,
            other => panic!("outputs must answer: {other:?}"),
        }
    }

    fn scale_of(&mut self, id: u64) -> f64 {
        self.outputs()
            .iter()
            .find(|output| output.id == id)
            .expect("the output")
            .scale
    }
}

fn ok(response: &Response) {
    assert!(
        matches!(response, Response::Ok { .. }),
        "expected ok, got {response:?}"
    );
}

fn error(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn output_scale_by_id_moves_the_live_output() {
    let mut fixture = Fixture::new();
    let response = fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(2.0),
    });
    ok(&response);
    let outputs = fixture.outputs();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].scale, 2.0);
    assert_eq!(
        (outputs[0].rect.width, outputs[0].rect.height),
        (CANVAS / 2, CANVAS / 2),
        "the logical geometry should halve at scale 2"
    );
    assert!(
        fixture.state.needs_render,
        "a scale change asks for a render"
    );
}

#[test]
fn output_scale_by_name_moves_only_the_named_output() {
    let mut fixture = Fixture::with_outputs(2);
    let response = fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("headless-2".into()),
        scale: Some(2.0),
    });
    ok(&response);
    assert_eq!(fixture.scale_of(1), 1.0, "the unnamed output stays put");
    assert_eq!(fixture.scale_of(2), 2.0);
}

#[test]
fn an_unknown_id_or_name_is_refused() {
    let mut fixture = Fixture::with_outputs(2);
    let message = error(fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(99),
        scale: Some(2.0),
    }));
    assert!(
        message.contains("no such output: 99"),
        "the refusal names the id: {message}"
    );
    let message = error(fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("DP-99".into()),
        scale: Some(2.0),
    }));
    assert!(
        message.contains("no such output: DP-99"),
        "the refusal names the name: {message}"
    );
    assert_eq!(fixture.scale_of(1), 1.0);
    assert_eq!(fixture.scale_of(2), 1.0, "a refused set moves nothing");
}

#[test]
fn an_out_of_range_or_non_finite_scale_is_refused() {
    let mut fixture = Fixture::new();
    for scale in [0.49, 4.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let message = error(fixture.state.handle_request(Request::OutputScale {
            output: OutputTarget::Id(1),
            scale: Some(scale),
        }));
        assert!(
            message.contains("0.5") && message.contains("to 4"),
            "the refusal names the range, for {scale}: {message}"
        );
    }
    assert_eq!(fixture.scale_of(1), 1.0, "a refused set moves nothing");
    // The endpoints themselves apply: the range is inclusive.
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(0.5),
    }));
    assert_eq!(fixture.scale_of(1), 0.5);
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(4.0),
    }));
    assert_eq!(fixture.scale_of(1), 4.0);
}

#[test]
fn a_repeat_set_is_a_silent_noop() {
    let mut fixture = Fixture::new();
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(2.0),
    }));
    assert_eq!(fixture.scale_of(1), 2.0);
    // The first set asked for its render; the repeat must not ask again.
    fixture.state.needs_render = false;
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("headless".into()),
        scale: Some(2.0),
    }));
    assert!(
        !fixture.state.needs_render,
        "a repeat set resends nothing and renders nothing"
    );
    assert_eq!(
        fixture.state.runtime_scales.get("headless"),
        Some(&2.0),
        "the runtime scale is still recorded"
    );
}

#[test]
fn a_reload_restores_the_config_files_scale() {
    let mut fixture = Fixture::new();
    let _dir = fixture.install_config("");
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(2.0),
    }));
    assert_eq!(fixture.scale_of(1), 2.0);
    let response = fixture.state.handle_request(Request::Reload);
    let Response::Reloaded { applied, refused } = &response else {
        panic!("a reload must report: {response:?}");
    };
    assert!(
        refused.is_empty(),
        "restoring the config scale refuses nothing: {response:?}"
    );
    let scales: Vec<&String> = applied
        .iter()
        .filter(|name| name.contains("scale"))
        .collect();
    assert_eq!(
        scales,
        ["outputs.headless.scale"],
        "the dropped runtime scale moved the output, so it reports: {response:?}"
    );
    assert_eq!(
        fixture.scale_of(1),
        1.0,
        "the reload drops the runtime scale for the file's"
    );
    assert!(
        fixture.state.runtime_scales.is_empty(),
        "the runtime map is empty after a reload"
    );
}

/// What a reload's reply says about the runtime scales it drops: one the
/// file disagrees with is a move, reported once by name and sorted --
/// connected or not, since a replug would have come back at it -- and one
/// the file agrees with moved nothing and reports nothing. A changed entry
/// for the same name reports once, not twice.
#[test]
fn a_reload_reports_exactly_the_runtime_scales_it_moved() {
    let mut fixture = Fixture::with_outputs(3);
    let dir = fixture.install_config("[[outputs]]\nname = \"headless-3\"\nscale = 1.5\n");
    assert!(matches!(
        fixture.state.handle_request(Request::Reload),
        Response::Reloaded { .. }
    ));
    assert_eq!(fixture.scale_of(3), 1.5);
    // A live scale equal to the file's (headless-3 at 1.5), one that
    // differs on a connected output (headless-2), one on a monitor that has
    // since been unplugged (DP-1, set while it was here), and one whose file
    // entry itself changes in this reload (headless, to 1.25 below).
    for (name, scale) in [("headless-3", 1.5), ("headless-2", 2.0), ("headless", 3.0)] {
        ok(&fixture.state.handle_request(Request::OutputScale {
            output: OutputTarget::Name(name.into()),
            scale: Some(scale),
        }));
    }
    fixture.state.runtime_scales.insert("DP-1".into(), 2.0);
    fs::write(
        dir.path().join("config.toml"),
        "[[outputs]]\nname = \"headless-3\"\nscale = 1.5\n\n\
         [[outputs]]\nname = \"headless\"\nscale = 1.25\n",
    )
    .expect("the edited config");
    let response = fixture.state.handle_request(Request::Reload);
    let Response::Reloaded { applied, refused } = &response else {
        panic!("a reload must report: {response:?}");
    };
    assert!(refused.is_empty(), "{response:?}");
    assert_eq!(
        applied,
        &[
            "outputs.headless.scale".to_owned(),
            "outputs.DP-1.scale".to_owned(),
            "outputs.headless-2.scale".to_owned(),
        ],
        "the changed entry once, then each moved runtime scale, sorted"
    );
    assert_eq!(fixture.scale_of(1), 1.25);
    assert_eq!(fixture.scale_of(2), 1.0);
    assert_eq!(fixture.scale_of(3), 1.5);
    assert!(fixture.state.runtime_scales.is_empty());
    // And a second reload agrees silently: nothing live is left to drop.
    let response = fixture.state.handle_request(Request::Reload);
    assert_eq!(
        response,
        Response::Reloaded {
            applied: vec![],
            refused: vec![],
        }
    );
}

#[test]
fn a_replugged_monitor_comes_back_at_its_runtime_scale() {
    let mut fixture = Fixture::with_outputs(2);
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("headless-2".into()),
        scale: Some(2.0),
    }));
    assert!(fixture.state.remove_output(OutputId(2)));
    let fresh = headless::add_output(&mut fixture.state, "headless-2", CANVAS, CANVAS)
        .expect("a replugged output");
    assert_ne!(fresh, OutputId(2), "ids are never reused");
    assert_eq!(
        fixture.scale_of(fresh.0),
        2.0,
        "the same name comes back at its runtime scale"
    );
    assert_eq!(fixture.scale_of(1), 1.0);
}

#[test]
fn resolve_ipc_scale_validates_like_the_loader_and_resolves_to_120ths() {
    // Valid values resolve exactly like the loader's `clamp_scale`.
    for (scale, expected_120ths) in [(0.5, 60), (1.33, 160), (1.5, 180), (2.0, 240), (4.0, 480)] {
        assert_eq!(
            State::resolve_ipc_scale(scale),
            Ok(expected_120ths as f64 / 120.0),
            "resolve_ipc_scale({scale}) was wrong"
        );
    }
    // Out of range and non-finite are errors, never silent clamps.
    for scale in [
        0.49,
        4.01,
        f64::MIN,
        f64::MAX,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ] {
        assert!(
            State::resolve_ipc_scale(scale).is_err(),
            "resolve_ipc_scale({scale}) must refuse"
        );
    }
}

#[test]
fn a_reset_goes_back_to_the_config_files_scale() {
    let mut fixture = Fixture::with_outputs(2);
    // The file scales `headless-2` to 1.5: a live 2.0 beats it, and a
    // reset hands it back.
    let _dir = fixture.install_config("[[outputs]]\nname = \"headless-2\"\nscale = 1.5\n");
    let reloaded = fixture.state.handle_request(Request::Reload);
    assert!(
        matches!(reloaded, Response::Reloaded { .. }),
        "the fixture config must load: {reloaded:?}"
    );
    assert_eq!(fixture.scale_of(2), 1.5);
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("headless-2".into()),
        scale: Some(2.0),
    }));
    assert_eq!(fixture.scale_of(2), 2.0);
    fixture.state.needs_render = false;
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(2),
        scale: None,
    }));
    assert_eq!(
        fixture.scale_of(2),
        1.5,
        "a reset lands on the file's scale"
    );
    assert!(fixture.state.needs_render, "a reset that moves renders");
    assert!(fixture.state.runtime_scales.is_empty());
    // A reset with no live scale left moves and renders nothing.
    fixture.state.needs_render = false;
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: None,
    }));
    assert!(!fixture.state.needs_render, "a no-op reset renders nothing");
    assert_eq!(fixture.scale_of(1), 1.0);
    assert_eq!(fixture.scale_of(2), 1.5);
}

#[test]
fn a_reset_of_an_unknown_output_is_refused() {
    let mut fixture = Fixture::new();
    let message = error(fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Name("DP-9".into()),
        scale: None,
    }));
    assert!(message.contains("no such output: DP-9"), "{message}");
}

#[test]
fn a_failed_reload_keeps_the_live_scale() {
    let mut fixture = Fixture::new();
    let _dir = fixture.install_config("[layout\n");
    ok(&fixture.state.handle_request(Request::OutputScale {
        output: OutputTarget::Id(1),
        scale: Some(2.0),
    }));
    let response = fixture.state.handle_request(Request::Reload);
    assert!(
        matches!(response, Response::Error { .. }),
        "a malformed file must refuse the reload: {response:?}"
    );
    assert_eq!(
        fixture.scale_of(1),
        2.0,
        "a refused reload changes nothing, the live scale included"
    );
}
