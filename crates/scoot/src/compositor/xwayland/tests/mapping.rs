//! Phase 2: X windows in the layout -- mapping, what they are listed as,
//! floating, rules, override-redirect windows, fullscreen, closing.

use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::ConnectionExt as _;

use super::live::{BLUE, BLUE_BGRA, CANVAS, RED, RED_BGRA, id_of_xid, live};
use super::peer::{Ack, Step};
use super::x11::{Props, eventually};
use crate::compositor::window_rules::{FloatingRules, WindowRuleConfig};

/// A normal X window is a tiled column: placed by the layout (not where it
/// asked), configured X-side to exactly that placement, drawn there, and
/// listed everywhere a window is -- the core, `scoot msg windows` and the
/// wlr foreign-toplevel list -- under its `WM_CLASS` class and title.
#[test]
fn an_x_window_maps_as_a_tiled_column_and_is_listed() {
    let Some(mut live) = live("an_x_window_maps_as_a_tiled_column_and_is_listed") else {
        return;
    };
    assert!(matches!(live.fixture.run(Step::BindTaskbar), Ack::Done));
    let mut props = Props::new(RED);
    props.rect = (300, 250, 64, 48);
    props.class = Some(("xprobe", "XProbe"));
    props.title = Some("probe");
    let xid = live.x.map(&props);
    let id = live.managed(xid);

    let placement = live.placement(id);
    assert!(!placement.floating, "a normal X window floated");
    assert!(placement.visible);
    assert_ne!(
        (placement.rect.x, placement.rect.y),
        (300, 250),
        "the layout, not the window, decides where a tiled X window goes"
    );
    let info = live
        .fixture
        .state
        .world
        .window_info(id)
        .cloned()
        .expect("info");
    assert_eq!(info.app_id, "XProbe");
    assert_eq!(info.title, "probe");

    // X-side: the window is where scoot placed it and the size it asked.
    live.drain();
    let geometry = live
        .x
        .conn
        .get_geometry(xid)
        .expect("a geometry request")
        .reply()
        .expect("the geometry");
    let size = placement
        .requested
        .expect("a tiled window is asked for a size");
    assert_eq!(
        (i32::from(geometry.width), i32::from(geometry.height)),
        (size.w, size.h),
        "the X window was not configured to its placement"
    );
    let origin = live
        .x
        .conn
        .translate_coordinates(xid, live.x.root, 0, 0)
        .expect("a translate request")
        .reply()
        .expect("the translation");
    assert_eq!(
        (i32::from(origin.dst_x), i32::from(origin.dst_y)),
        (placement.rect.x, placement.rect.y),
        "the X window's root position is not its placement (menus would open in the wrong place)"
    );

    // Drawn where it is placed.
    let centre = (
        placement.rect.x + placement.rect.w / 2,
        placement.rect.y + placement.rect.h / 2,
    );
    assert_eq!(live.pixel_at(centre.0, centre.1), RED_BGRA);

    // Listed.
    let snapshot = live
        .fixture
        .state
        .window_snapshots()
        .into_iter()
        .find(|snapshot| snapshot.id == id.0)
        .expect("the X window in `scoot msg windows`");
    assert_eq!(snapshot.app_id, "XProbe");
    assert_eq!(snapshot.title, "probe");
    assert!(!snapshot.floating);
    assert!(
        live.taskbar()
            .contains(&("probe".to_owned(), "XProbe".to_owned())),
        "the taskbar was not told about the X window"
    );
}

/// A title change reaches the core and the taskbar, as a terminal
/// retitling on every prompt needs; unmapping takes the window out of the
/// layout and closes its taskbar entry.
#[test]
fn title_changes_and_unmapping_reach_the_core_and_the_taskbar() {
    let Some(mut live) = live("title_changes_and_unmapping_reach_the_core_and_the_taskbar") else {
        return;
    };
    assert!(matches!(live.fixture.run(Step::BindTaskbar), Ack::Done));
    let mut props = Props::new(RED);
    props.class = Some(("xterm", "XTerm"));
    props.title = Some("first");
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    live.x.retitle(xid, "second");
    eventually(&mut live.fixture, "the new title in the core", |fixture| {
        fixture
            .state
            .world
            .window_info(id)
            .is_some_and(|info| info.title == "second")
    });
    assert!(
        live.taskbar()
            .contains(&("second".to_owned(), "XTerm".to_owned())),
        "the taskbar still has the old title"
    );

    live.x.unmap(xid);
    eventually(
        &mut live.fixture,
        "the X window leaving the layout",
        |fixture| id_of_xid(&fixture.state, xid).is_none(),
    );
    assert!(live.fixture.state.world.window_info(id).is_none());
    assert!(
        live.taskbar().is_empty(),
        "an unmapped X window is still on the taskbar"
    );
}

/// A transient dialog floats, centred on its X parent -- the same centring
/// an xdg dialog gets -- and names that parent in the core.
#[test]
fn a_transient_dialog_floats_centred_on_its_x_parent() {
    let Some(mut live) = live("a_transient_dialog_floats_centred_on_its_x_parent") else {
        return;
    };
    let parent = live.x.map(&Props::new(RED));
    let parent_id = live.managed(parent);
    let mut props = Props::new(BLUE);
    props.rect = (0, 0, 100, 60);
    props.transient_for = Some(parent);
    props.dialog = true;
    let dialog = live.x.map(&props);
    let dialog_id = live.managed(dialog);

    let placement = live.placement(dialog_id);
    assert!(placement.floating, "a transient dialog did not float");
    assert_eq!((placement.rect.w, placement.rect.h), (100, 60));
    assert_eq!(
        live.fixture
            .state
            .world
            .window_info(dialog_id)
            .and_then(|info| info.parent),
        Some(parent_id)
    );
    let parent_rect = live.placement(parent_id).rect;
    let centre = |x: i32, w: i32| x + w / 2;
    assert!(
        (centre(placement.rect.x, placement.rect.w) - centre(parent_rect.x, parent_rect.w)).abs()
            <= 1
            && (centre(placement.rect.y, placement.rect.h) - centre(parent_rect.y, parent_rect.h))
                .abs()
                <= 1,
        "the dialog {:?} is not centred on its parent {:?}",
        placement.rect,
        parent_rect
    );
    // Drawn over its parent.
    let (x, y) = (
        centre(placement.rect.x, placement.rect.w),
        centre(placement.rect.y, placement.rect.h),
    );
    assert_eq!(live.pixel_at(x, y), BLUE_BGRA);
}

/// A dialog-typed window with no parent floats too, and a fixed-size one;
/// a plain one next to them tiles.
#[test]
fn dialog_types_and_fixed_sizes_float_and_plain_windows_tile() {
    let Some(mut live) = live("dialog_types_and_fixed_sizes_float_and_plain_windows_tile") else {
        return;
    };
    let plain = live.x.map(&Props::new(RED));
    let plain = live.managed(plain);
    let mut typed = Props::new(BLUE);
    typed.dialog = true;
    let typed = live.x.map(&typed);
    let typed = live.managed(typed);
    let mut fixed = Props::new(BLUE);
    fixed.rect = (0, 0, 80, 50);
    fixed.min_max = Some(((80, 50), (80, 50)));
    let fixed = live.x.map(&fixed);
    let fixed = live.managed(fixed);
    assert!(!live.placement(plain).floating);
    assert!(
        live.placement(typed).floating,
        "a dialog-typed X window tiled"
    );
    assert!(
        live.placement(fixed).floating,
        "a fixed-size X window tiled"
    );
}

/// A `[[window_rule]]` matches an X window by its `WM_CLASS` class, and its
/// `size` is what the window is configured to.
#[test]
fn a_window_rule_matches_an_x_window_by_class() {
    let Some(mut live) = live("a_window_rule_matches_an_x_window_by_class") else {
        return;
    };
    #[derive(serde::Deserialize)]
    struct File {
        window_rule: Vec<WindowRuleConfig>,
    }
    let file: File = toml::from_str(
        r#"
        [[window_rule]]
        match_app_id = "XRuled"
        float = true
        size = [150, 100]
        "#,
    )
    .expect("a rule");
    let (rules, skipped) = FloatingRules::from_config(None, &file.window_rule);
    assert!(skipped.is_empty(), "{skipped:?}");
    live.fixture.state.floating_rules = rules;

    let mut props = Props::new(RED);
    props.class = Some(("xruled", "XRuled"));
    let xid = live.x.map(&props);
    let id = live.managed(xid);
    let placement = live.placement(id);
    assert!(placement.floating, "the rule did not float the X window");
    assert_eq!((placement.rect.w, placement.rect.h), (150, 100));
    live.drain();
    let geometry = live
        .x
        .conn
        .get_geometry(xid)
        .expect("a geometry request")
        .reply()
        .expect("the geometry");
    assert_eq!((geometry.width, geometry.height), (150, 100));

    // A different class is untouched by the rule.
    let mut other = Props::new(RED);
    other.class = Some(("other", "Other"));
    let other = live.x.map(&other);
    let other = live.managed(other);
    assert!(!live.placement(other).floating);
}

/// A floating X window that asked for a position (`USPosition`) inside the
/// usable area gets it; one asking for a place off every output is centred
/// instead.
#[test]
fn a_floating_window_keeps_its_asked_position_only_when_it_fits() {
    let Some(mut live) = live("a_floating_window_keeps_its_asked_position_only_when_it_fits")
    else {
        return;
    };
    let mut fits = Props::new(BLUE);
    fits.rect = (210, 170, 80, 60);
    fits.dialog = true;
    fits.us_position = true;
    let fits = live.x.map(&fits);
    let fits = live.managed(fits);
    let rect = live.placement(fits).rect;
    assert_eq!(
        (rect.x, rect.y),
        (210, 170),
        "the asked position was not kept"
    );

    let mut off = Props::new(BLUE);
    off.rect = (3000, 3000, 80, 60);
    off.dialog = true;
    off.us_position = true;
    let off = live.x.map(&off);
    let off = live.managed(off);
    let rect = live.placement(off).rect;
    assert_eq!(
        (rect.x + rect.w / 2, rect.y + rect.h / 2),
        (CANVAS / 2, CANVAS / 2),
        "a window asking for a place off screen was not centred"
    );
}

/// An override-redirect window never enters the core (not a column, not
/// listed) but is drawn where it put itself, above the windows, and takes
/// the pointer there.
#[test]
fn an_override_redirect_window_is_drawn_but_never_a_column() {
    let Some(mut live) = live("an_override_redirect_window_is_drawn_but_never_a_column") else {
        return;
    };
    let under = live.x.map(&Props::new(RED));
    live.managed(under);
    let mut props = Props::new(BLUE);
    props.rect = (20, 20, 50, 40);
    props.override_redirect = true;
    let overlay = live.x.map(&props);
    eventually(
        &mut live.fixture,
        "the override-redirect window drawn",
        |fixture| {
            fixture
                .state
                .x11_unmanaged
                .iter()
                .any(|known| known.window_id() == overlay && known.wl_surface().is_some())
        },
    );
    assert!(
        id_of_xid(&live.fixture.state, overlay).is_none(),
        "an override-redirect window entered the core"
    );
    assert_eq!(live.fixture.state.windows.len(), 1);
    assert_eq!(
        live.pixel_at(45, 40),
        BLUE_BGRA,
        "not drawn over the window"
    );
    let surface = live
        .fixture
        .state
        .x11_unmanaged
        .iter()
        .find(|known| known.window_id() == overlay)
        .and_then(|known| known.wl_surface())
        .expect("its surface");
    let under_pointer = live
        .fixture
        .state
        .surface_under((45.0, 40.0).into())
        .map(|(found, _)| found);
    assert_eq!(under_pointer, Some(surface), "the pointer does not find it");
    // A click on it focuses nothing new.
    let focus = live.fixture.state.focus;
    live.fixture.state.pointer_move(45.0, 40.0);
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, true);
    live.fixture
        .state
        .pointer_button(scoot_ipc::PointerButton::Left, false);
    assert_eq!(live.fixture.state.focus, focus);
    // Gone when it unmaps.
    live.x.unmap(overlay);
    eventually(
        &mut live.fixture,
        "the override-redirect window gone",
        |fixture| fixture.state.x11_unmanaged.is_empty(),
    );
}

/// `_NET_WM_STATE_FULLSCREEN` both ways: the client's request makes it
/// fullscreen in the core (covering the output, configured to its size),
/// the property follows, and the reverse puts it back; a window mapping
/// with the state already set maps fullscreen.
#[test]
fn fullscreen_follows_net_wm_state_both_ways() {
    let Some(mut live) = live("fullscreen_follows_net_wm_state_both_ways") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    live.x.request_fullscreen(xid, true);
    eventually(
        &mut live.fixture,
        "the core making it fullscreen",
        |fixture| fixture.state.world.is_fullscreen(id),
    );
    let placement = live.placement(id);
    assert_eq!(
        (placement.rect.w, placement.rect.h),
        (CANVAS, CANVAS),
        "a fullscreen X window does not cover the output"
    );
    live.drain();
    assert!(live.x.is_fullscreen(xid), "the property did not follow");
    let geometry = live
        .x
        .conn
        .get_geometry(xid)
        .expect("a geometry request")
        .reply()
        .expect("the geometry");
    assert_eq!(
        (i32::from(geometry.width), i32::from(geometry.height)),
        (CANVAS, CANVAS)
    );

    live.x.request_fullscreen(xid, false);
    eventually(
        &mut live.fixture,
        "the core leaving fullscreen",
        |fixture| !fixture.state.world.is_fullscreen(id),
    );
    live.drain();
    assert!(!live.x.is_fullscreen(xid));

    let mut preset = Props::new(BLUE);
    preset.fullscreen = true;
    let preset = live.x.map(&preset);
    let preset = live.managed(preset);
    assert!(
        live.fixture.state.world.is_fullscreen(preset),
        "a window mapping with _NET_WM_STATE_FULLSCREEN did not map fullscreen"
    );
}

/// `Effect::Close` (the close bind, IPC `close-focused`) asks an X window
/// to go the ICCCM way: a `WM_DELETE_WINDOW` client message.
#[test]
fn closing_an_x_window_sends_wm_delete_window() {
    let Some(mut live) = live("closing_an_x_window_sends_wm_delete_window") else {
        return;
    };
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    assert_eq!(live.fixture.state.focus, Some(id));
    live.x.drain();
    live.fixture.state.act(scoot_core::Action::CloseFocused);
    live.drain();
    live.x.conn.flush().expect("a flush");
    let asked = live
        .x
        .drain()
        .into_iter()
        .any(|event| matches!(event, XEvent::ClientMessage(message) if message.window == xid));
    assert!(asked, "the X window was never asked to close");
}
