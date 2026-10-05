//! Switching one specific output's workspace over IPC.
//!
//! The transport half of the output-targeted action: `focus-workspace-index
//! N --output ID` names the output whose list the index counts within, and
//! the switch moves keyboard focus there -- the same move `ext-workspace-v1`
//! drives for a bar click on another monitor, through the same core action.
//! An unknown output id or a stale index answers `Ok` and moves nothing,
//! like the out-of-range plain index; only the lock refuses with an `Error`
//! (pinned with the other cross-output actions in `session_lock`'s input
//! tests).
//!
//! Like every other live-`State` test module here, these need a writable
//! `$XDG_RUNTIME_DIR`: [`State::new`] binds a real listening socket.

use scoot_core::{Config, Event as CoreEvent, OutputId, WindowId, WindowInfo};
use scoot_ipc::{Request, Response};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::compositor::State;
use crate::compositor::decorations::Appearance;
use crate::compositor::headless;
use crate::compositor::keybindings::Keybindings;

/// The canvas the headless backend renders into. Nothing here reads a
/// pixel; the backend exists so the `State` is a whole session.
const CANVAS: i32 = 200;

/// A live compositor with two side-by-side headless outputs and no client:
/// windows are filed straight into the core, and every change below still
/// reaches the layout through [`State::apply`].
struct Fixture {
    _event_loop: EventLoop<'static, State>,
    state: State,
}

impl Fixture {
    fn two_outputs() -> Self {
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
            crate::compositor::test_support::test_renderer(),
        )
        .expect("a compositor state with a wayland socket");
        headless::init(&mut state, CANVAS, CANVAS).expect("a headless backend");
        headless::add_output(&mut state, "headless-2", CANVAS, CANVAS).expect("a second output");
        Self {
            _event_loop: event_loop,
            state,
        }
    }

    /// Opens window `id` on `output` and runs the `apply()` a real mapping
    /// ends in.
    fn open(&mut self, id: u64, output: OutputId, focus: bool) {
        self.state.world.handle_event(CoreEvent::WindowOpened {
            id: WindowId(id),
            info: WindowInfo::default(),
            output: Some(output),
            focus,
        });
        self.state.apply();
    }

    /// Sends one targeted switch the way `scoot msg` does.
    fn switch(&mut self, output: u64, index: usize) -> Response {
        self.state.handle_request(Request::Action(
            scoot_ipc::Action::FocusOutputWorkspaceIndex { output, index },
        ))
    }

    fn workspaces(&self, output: u64) -> (usize, usize) {
        let workspaces = self
            .state
            .world
            .workspaces(OutputId(output))
            .expect("the output exists");
        (workspaces.count, workspaces.active)
    }
}

#[test]
fn a_targeted_switch_over_ipc_moves_that_output_and_focus() {
    let mut fixture = Fixture::two_outputs();
    fixture.open(1, OutputId(1), true);
    fixture.open(2, OutputId(2), false);
    assert_eq!(fixture.workspaces(1), (2, 0));
    assert_eq!(fixture.workspaces(2), (2, 0));
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(1)),
        "the setup should leave focus on the first output"
    );

    fixture.state.needs_render = false;
    let response = fixture.switch(2, 1);
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the targeted switch was not served, got {response:?}"
    );
    assert_eq!(
        fixture.workspaces(2),
        (2, 1),
        "output 2 switches to its second workspace"
    );
    assert_eq!(
        fixture.workspaces(1),
        (2, 0),
        "output 1's workspaces are unchanged"
    );
    assert_eq!(
        fixture.state.world.focused_output(),
        Some(OutputId(2)),
        "the switch moves focus to the named output"
    );
    assert_eq!(
        fixture.state.focus, None,
        "output 2's second workspace holds no window"
    );
    assert!(
        fixture.state.needs_render,
        "a real cross-output switch must run the full apply"
    );

    // ...and back onto window 2, which is also where shell focus lands.
    let response = fixture.switch(2, 0);
    assert!(
        matches!(response, Response::Ok { locked: false }),
        "the switch back was not served, got {response:?}"
    );
    assert_eq!(fixture.workspaces(2), (2, 0));
    assert_eq!(
        fixture.state.world.focused_window(),
        Some(WindowId(2)),
        "the switch back focuses window 2 in the core"
    );
    assert_eq!(
        fixture.state.focus,
        Some(WindowId(2)),
        "the switch back focuses window 2 in the shell"
    );
}

#[test]
fn unknown_output_and_stale_index_answer_ok_and_move_nothing() {
    // The two refusals: answered `Ok` -- the core ignores them, and there
    // is nothing to refuse loudly the way a locked action is -- while the
    // session comes back as it was left, focus included.
    let mut fixture = Fixture::two_outputs();
    fixture.open(1, OutputId(1), true);
    fixture.open(2, OutputId(2), false);

    for (output, index) in [(99, 0), (2, 7)] {
        let response = fixture.switch(output, index);
        assert!(
            matches!(response, Response::Ok { locked: false }),
            "targeted refusal ({output}, {index}) was not served, got {response:?}"
        );
        assert_eq!(fixture.workspaces(1), (2, 0));
        assert_eq!(fixture.workspaces(2), (2, 0));
        assert_eq!(
            fixture.state.world.focused_output(),
            Some(OutputId(1)),
            "targeted refusal ({output}, {index}) moved focus"
        );
        assert_eq!(
            fixture.state.world.focused_window(),
            Some(WindowId(1)),
            "targeted refusal ({output}, {index}) moved window focus"
        );
    }
}
