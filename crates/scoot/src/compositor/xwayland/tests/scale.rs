//! X windows at a scaled output (`xwayland/scale.rs`): the X server draws
//! at the output's integer scale -- `ceil([output] scale)` -- in its own
//! pixels, and every coordinate that crosses between X and the layout is
//! converted exactly once. Each test here reads the X server's own answer
//! (where it has a window, where it has the pointer, what it was told in a
//! drag) against the layout's logical one, and the framebuffer's pixels
//! against both, at scale 2 unless it says otherwise -- so a path that
//! skipped the conversion, or did it twice, shows as a factor of two.
//!
//! [`CANVAS`] is physical pixels, so at scale 2 the layout is 200 logical
//! pixels square and the X screen 400 X pixels: X pixels are physical ones.

use std::fs;
use std::sync::Arc;

use scoot_core::{Placement, Rect, WindowId};
use scoot_ipc::{PointerButton, Request};
use smithay::desktop::Window;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    Rectangle as XRectangle, Window as XWindow, WindowClass,
};

use super::drop::{centre, grabbed, settle, start_wayland_drag, visible};
use super::live::{BLUE, BLUE_BGRA, CANVAS, Live, RED, RED_BGRA, live_scaled};
use super::peer::{Ack, CaptureStep, ClipStep, Step};
use super::x11::{Props, XClient, eventually};
use super::xdnd::{Inbox, PROXY_NAME, XDND_VERSION, packed};
use crate::compositor::test_support::pixel;

/// The X server's view of `xid`'s window: root position and size.
fn x_rect(live: &Live, xid: XWindow) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = live.x.root_geometry(xid);
    (x, y, w as i32, h as i32)
}

/// `rect` in X pixels at X scale `scale`.
fn scaled(rect: Rect, scale: i32) -> (i32, i32, i32, i32) {
    (
        rect.x * scale,
        rect.y * scale,
        rect.w * scale,
        rect.h * scale,
    )
}

/// The size of the X screen, as a client reads it off the root window.
fn x_screen(x: &XClient) -> (u16, u16) {
    let geometry = x
        .conn
        .get_geometry(x.root)
        .expect("a geometry request")
        .reply()
        .expect("the root geometry");
    (geometry.width, geometry.height)
}

/// The integer XSETTINGS the X server publishes (`_XSETTINGS_S0`'s owner's
/// `_XSETTINGS_SETTINGS`, parsed per the XSETTINGS spec), by name. Strings
/// and colors are skipped: every setting scoot sets is an integer.
fn xsettings(x: &XClient) -> Vec<(String, i32)> {
    let owner = x
        .conn
        .get_selection_owner(x.atom("_XSETTINGS_S0"))
        .expect("a selection-owner request")
        .reply()
        .expect("the XSETTINGS owner")
        .owner;
    assert_ne!(owner, x11rb::NONE, "nothing owns _XSETTINGS_S0");
    let data = x.read_property(owner, "_XSETTINGS_SETTINGS");
    let mut settings = Vec::new();
    if data.len() < 12 {
        return settings;
    }
    let big = data[0] != 0;
    let word = |at: usize| -> u32 {
        let bytes = [data[at], data[at + 1], data[at + 2], data[at + 3]];
        if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        }
    };
    let half = |at: usize| -> usize {
        let bytes = [data[at], data[at + 1]];
        usize::from(if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        })
    };
    let pad = |len: usize| len.div_ceil(4) * 4;
    let count = word(8);
    let mut at = 12;
    for _ in 0..count {
        let kind = data[at];
        let name_len = half(at + 2);
        let name = String::from_utf8_lossy(&data[at + 4..at + 4 + name_len]).into_owned();
        // Type, pad, name length, name, last-change serial.
        at += 4 + pad(name_len) + 4;
        match kind {
            0 => {
                #[allow(clippy::cast_possible_wrap)]
                settings.push((name, word(at) as i32));
                at += 4;
            }
            1 => at += 4 + pad(word(at) as usize),
            _ => at += 8,
        }
    }
    settings
}

fn setting(settings: &[(String, i32)], name: &str) -> Option<i32> {
    settings
        .iter()
        .find(|(known, _)| known == name)
        .map(|&(_, value)| value)
}

/// The XSETTINGS a toolkit reads the scale from: `(window scaling factor,
/// Xft DPI, unscaled DPI)`, each `None` where unset.
fn toolkit_scale(x: &XClient) -> (Option<i32>, Option<i32>, Option<i32>) {
    let settings = xsettings(x);
    (
        setting(&settings, "Gdk/WindowScalingFactor"),
        setting(&settings, "Xft/DPI"),
        setting(&settings, "Gdk/UnscaledDPI"),
    )
}

/// Maps a managed X window of `RED` and waits until the X server has it at
/// the size scoot placed it at, in X pixels at `scale`: the configure has
/// landed, not just been sent.
fn managed_at(live: &mut Live, scale: i32) -> (XWindow, WindowId, Placement) {
    let xid = live.x.map(&Props::new(RED));
    let id = live.managed(xid);
    let placement = live.placement(id);
    visible("X window", &placement);
    let want = scaled(placement.rect, scale);
    let mut seen = (0, 0, 0, 0);
    eventually(
        &mut live.fixture,
        "the X server holding the window at its scaled placement",
        |_| {
            seen = {
                let (x, y, w, h) = live.x.root_geometry(xid);
                (x, y, w as i32, h as i32)
            };
            seen == want
        },
    );
    (xid, id, placement)
}

/// Fills every other X-pixel column of `window` (`width` x `height` X
/// pixels) with `BLUE`, from column 0: a pattern only a window drawn at
/// the framebuffer's own resolution shows column for column.
fn stripe(x: &XClient, window: XWindow, width: u16, height: u16) {
    let gc = x.conn.generate_id().expect("a GC id");
    x.conn
        .create_gc(gc, window, &CreateGCAux::new().foreground(BLUE))
        .expect("a GC");
    let columns: Vec<XRectangle> = (0..width)
        .step_by(2)
        .map(|column| XRectangle {
            x: column as i16,
            y: 0,
            width: 1,
            height,
        })
        .collect();
    x.conn
        .poly_fill_rectangle(window, gc, &columns)
        .expect("a fill request");
    x.conn.flush().expect("the fill hit the wire");
}

/// A managed X window at scale 2 is laid out in logical pixels like any
/// window, and drawn by the X server at twice that in its own pixels: the
/// X screen is the physical one, the window is configured at its placement
/// times two, and a one-X-pixel pattern reaches the framebuffer -- and the
/// IPC screenshot and the screen capture, which are that framebuffer --
/// one physical pixel per X pixel: native resolution, not a 2x upscale of
/// a logical-sized buffer.
#[test]
fn an_x_window_draws_at_native_resolution_at_scale_2() {
    let Some(mut live) = live_scaled("an_x_window_draws_at_native_resolution_at_scale_2", 2.0)
    else {
        return;
    };
    assert_eq!(
        x_screen(&live.x),
        (CANVAS as u16, CANVAS as u16),
        "the X screen is not the physical output"
    );
    let (xid, id, placement) = managed_at(&mut live, 2);
    let x11 = live
        .fixture
        .state
        .window(id)
        .and_then(Window::x11_surface)
        .expect("an X window")
        .clone();
    let configured = x11.last_configure();
    assert_eq!(
        (
            configured.loc.x,
            configured.loc.y,
            configured.size.w,
            configured.size.h
        ),
        (
            placement.rect.x,
            placement.rect.y,
            placement.rect.w,
            placement.rect.h
        ),
        "Smithay's record of the configure is not the logical placement"
    );

    let (wx, wy, ww, wh) = x_rect(&live, xid);
    stripe(&live.x, xid, ww as u16, wh as u16);
    let row = wy + wh / 2;
    let columns = [wx + ww / 2 - 4, wx + ww / 2 - 2, wx + ww / 2];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        live.drain();
        let seen: Vec<[u8; 4]> = columns
            .iter()
            .flat_map(|&column| [live.pixel_at(column, row), live.pixel_at(column + 1, row)])
            .collect();
        if seen
            .chunks(2)
            .all(|pair| pair[0] == BLUE_BGRA && pair[1] == RED_BGRA)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "one X pixel is not one physical pixel: {seen:?} at row {row}, columns {columns:?}"
        );
    }

    // Both captures are the frame on screen, stripes and all: physical
    // pixels, like every capture (see `capture.rs`).
    let primary = live.fixture.state.outputs.primary_id().expect("an output");
    let shot = live
        .fixture
        .state
        .capture_pixels_for(Some(primary), false)
        .expect("an IPC capture")
        .bgra;
    let framebuffer = live.fixture.pixels_of(primary);
    assert_eq!(
        shot, framebuffer,
        "the screenshot is not the frame on screen"
    );
    let captured = match live.fixture.run(Step::Capture(CaptureStep::Output(0))) {
        Ack::Captured(Ok(pixels)) => pixels,
        other => panic!("expected a capture, got {other:?}"),
    };
    let framebuffer = live.fixture.pixels_of(primary);
    assert_eq!(
        captured, framebuffer,
        "the screen capture is not the frame on screen"
    );
    for (what, pixels) in [("screenshot", &shot), ("screen capture", &captured)] {
        for &column in &columns {
            assert_eq!(
                (
                    pixel(pixels, CANVAS, column, row),
                    pixel(pixels, CANVAS, column + 1, row)
                ),
                (BLUE_BGRA, RED_BGRA),
                "the {what} does not hold the X window at native resolution"
            );
        }
    }
}

/// An override-redirect menu at X `(40, 20)`, 50x40 X pixels, is at
/// logical `(20, 10)`, 25x20 -- drawn over exactly the physical pixels the
/// X client put it on, and hit where it is drawn.
#[test]
fn an_override_redirect_menu_lands_where_it_put_itself_at_scale_2() {
    let Some(mut live) = live_scaled(
        "an_override_redirect_menu_lands_where_it_put_itself_at_scale_2",
        2.0,
    ) else {
        return;
    };
    let mut menu = Props::new(BLUE);
    menu.rect = (40, 20, 50, 40);
    menu.override_redirect = true;
    let menu = live.x.map(&menu);
    eventually(&mut live.fixture, "the menu drawn", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == menu && known.wl_surface().is_some())
    });
    live.drain();
    let rect = {
        let known = live
            .fixture
            .state
            .x11_unmanaged
            .iter()
            .find(|known| known.window_id() == menu)
            .expect("the menu is drawn")
            .clone();
        crate::compositor::xwayland::unmanaged::rect_of(&known)
    };
    assert_eq!(
        (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
        (20, 10, 25, 20),
        "the menu's logical rectangle is not its X one halved"
    );
    for (x, y) in [(40, 20), (89, 59)] {
        assert_eq!(
            live.pixel_at(x, y),
            BLUE_BGRA,
            "the menu is not on physical ({x}, {y})"
        );
    }
    for (x, y) in [(38, 20), (91, 59), (40, 61)] {
        assert_ne!(
            live.pixel_at(x, y),
            BLUE_BGRA,
            "the menu spills onto physical ({x}, {y})"
        );
    }
    let under = live
        .fixture
        .state
        .x11_unmanaged_under((21.0, 11.0).into())
        .map(|(_, at)| at);
    assert_eq!(
        under,
        Some((20.0, 10.0).into()),
        "the menu is not hit where it is drawn"
    );
}

/// An X window's child widget, `(x, y, w, h)` in the parent's X pixels,
/// selecting button presses.
fn widget(x: &XClient, parent: XWindow, rect: (i16, i16, u16, u16), pixel: u32) -> XWindow {
    let window = x.conn.generate_id().expect("an X window id");
    let (wx, wy, ww, wh) = rect;
    x.conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            parent,
            wx,
            wy,
            ww,
            wh,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .background_pixel(pixel)
                .event_mask(EventMask::BUTTON_PRESS),
        )
        .expect("a create request")
        .check()
        .expect("the X server accepted the widget");
    x.conn
        .map_window(window)
        .expect("a map request")
        .check()
        .expect("the X server mapped the widget");
    x.conn.flush().expect("flushed");
    window
}

/// The computer-use path: `scoot msg click` at a logical point over an X
/// window reaches the X widget drawn there, and the X server sees the press
/// at the X-pixel point under it. Two widgets side by side, each clicked at
/// its logical centre.
#[test]
fn an_ipc_click_lands_on_the_x_widget_under_it_at_scale_2() {
    let Some(mut live) = live_scaled(
        "an_ipc_click_lands_on_the_x_widget_under_it_at_scale_2",
        2.0,
    ) else {
        return;
    };
    let (xid, _, placement) = managed_at(&mut live, 2);
    let (_, _, ww, wh) = x_rect(&live, xid);
    let half = (ww / 2) as u16;
    let left = widget(&live.x, xid, (0, 0, half, wh as u16), BLUE);
    let right = widget(&live.x, xid, (half as i16, 0, half, wh as u16), RED);
    live.drain();
    let _ = live.x.drain();

    let rect = placement.rect;
    let quarter = rect.w / 4;
    for (widget, logical_x) in [(left, rect.x + quarter), (right, rect.x + 3 * quarter)] {
        let logical_y = rect.y + rect.h / 2;
        let response = live.fixture.state.handle_request(Request::Click {
            x: f64::from(logical_x),
            y: f64::from(logical_y),
            button: PointerButton::Left,
        });
        assert!(
            !format!("{response:?}").contains("Error"),
            "the click was refused: {response:?}"
        );
        let mut press = None;
        eventually(&mut live.fixture, "the press reaching the X server", |_| {
            for event in live.x.drain() {
                if let XEvent::ButtonPress(event) = event {
                    press = Some(event);
                }
            }
            press.is_some()
        });
        let press = press.expect("just waited for it");
        assert_eq!(
            press.event, widget,
            "the click at logical ({logical_x}, {logical_y}) reached the wrong X widget"
        );
        assert_eq!(
            (i32::from(press.root_x), i32::from(press.root_y)),
            (logical_x * 2, logical_y * 2),
            "the X server has the press somewhere other than under the logical click"
        );
    }
}

/// A dialog that asks for its own position (`USPosition`) in X pixels keeps
/// it, halved into the layout: X clients compute dialog positions in root
/// X pixels.
#[test]
fn a_us_position_dialog_keeps_its_place_at_scale_2() {
    let Some(mut live) = live_scaled("a_us_position_dialog_keeps_its_place_at_scale_2", 2.0) else {
        return;
    };
    let mut dialog = Props::new(BLUE);
    dialog.rect = (100, 60, 60, 40);
    dialog.dialog = true;
    dialog.us_position = true;
    let xid = live.x.map(&dialog);
    let id = live.managed(xid);
    let placement = live.placement(id);
    assert_eq!(
        (placement.rect.x, placement.rect.y),
        (50, 30),
        "the dialog was not placed where it asked, in logical pixels: {placement:?}"
    );
    live.drain();
    assert_eq!(
        x_rect(&live, xid),
        (100, 60, 60, 40),
        "the X server moved the dialog from where it asked"
    );
}

/// A configure request past the X server's size limit, from a window that
/// has not mapped yet (granted, clamped): at scale 2 the clamp is on the X
/// pixels the server gets, not on logical ones twice that. The logical
/// clamp alone sent (and XWayland 24.1 granted) a 65534-wide window --
/// twice the bound, and past `INT16` for anything placed right of it.
#[test]
fn an_oversized_configure_request_stays_in_x_range_at_scale_2() {
    let Some(mut live) = live_scaled(
        "an_oversized_configure_request_stays_in_x_range_at_scale_2",
        2.0,
    ) else {
        return;
    };
    let window = live.x.conn.generate_id().expect("an X window id");
    live.x
        .conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            live.x.root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .expect("a create request")
        .check()
        .expect("the X server accepted the window");
    live.drain();
    live.x
        .conn
        .configure_window(window, &ConfigureWindowAux::new().width(65_535))
        .expect("a configure request");
    live.x.conn.flush().expect("flushed");
    let mut width = 0;
    eventually(&mut live.fixture, "the clamped size landing", |_| {
        width = live.x.root_geometry(window).2;
        width != 100
    });
    assert!(
        width <= i16::MAX as u32,
        "the X server was sent a width past its limit: {width}"
    );
    assert_eq!(width, 32_766, "not the largest even width under the limit");
}

/// Toolkits are told the scale through XSETTINGS -- the way a GNOME
/// session's settings daemon tells them, read live by GTK, Qt and Java:
/// the integer window scale, the DPI fonts render at (Xft/DPI, 1024ths),
/// and GTK's unscaled DPI (so it does not scale fonts twice).
#[test]
fn toolkits_are_told_the_scale_at_scale_2() {
    let Some(live) = live_scaled("toolkits_are_told_the_scale_at_scale_2", 2.0) else {
        return;
    };
    assert_eq!(
        toolkit_scale(&live.x),
        (Some(2), Some(2 * 96 * 1024), Some(96 * 1024))
    );
}

/// A scale-1 session is the session that ran before any of this: the X
/// client scale is 1 and not one XSETTINGS entry is written, so nothing a
/// toolkit reads has changed.
#[test]
fn a_scale_1_session_is_untouched() {
    let Some(mut live) = live_scaled("a_scale_1_session_is_untouched", 1.0) else {
        return;
    };
    assert_eq!(toolkit_scale(&live.x), (None, None, None));
    assert!(
        xsettings(&live.x).is_empty(),
        "a scale-1 session published XSETTINGS: {:?}",
        xsettings(&live.x)
    );
    let (xid, _, placement) = managed_at(&mut live, 1);
    assert_eq!(x_rect(&live, xid), scaled(placement.rect, 1));
    assert_eq!(x_screen(&live.x), (CANVAS as u16, CANVAS as u16));
}

/// A fractional scale draws X at the next integer up and lets the renderer
/// scale that down: at 1.5 the X server is at 2, sharp toolkits size
/// themselves for 2, and what reaches the screen is still the window,
/// where it is placed.
#[test]
fn a_fractional_scale_draws_x_at_the_integer_above() {
    let Some(mut live) = live_scaled("a_fractional_scale_draws_x_at_the_integer_above", 1.5) else {
        return;
    };
    assert_eq!(toolkit_scale(&live.x).0, Some(2));
    let (_, _, placement) = managed_at(&mut live, 2);
    // The window's centre, in physical pixels at 1.5.
    let (cx, cy) = centre(placement.rect);
    #[allow(clippy::cast_possible_truncation)]
    let (px, py) = ((cx * 1.5) as i32, (cy * 1.5) as i32);
    assert_eq!(live.pixel_at(px, py), RED_BGRA);
}

/// `[output] scale` reloaded with X windows open: the X scale follows (1 to
/// 2 and back), every open X window is reconfigured into the new X pixels
/// at its unchanged-in-kind logical placement, toolkits are told the new
/// scale, and the X screen resizes to the physical output. Nothing about
/// the window's logical geometry disagrees at any point, and it still
/// draws where it is placed.
#[test]
fn a_runtime_scale_change_rescales_open_x_windows() {
    let Some(mut live) = live_scaled("a_runtime_scale_change_rescales_open_x_windows", 1.0) else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    live.fixture.state.config_path = Some(path.clone());
    let (xid, id, _) = managed_at(&mut live, 1);

    for scale in [2, 1] {
        fs::write(&path, format!("[output]\nscale = {scale}.0\n")).expect("a config file");
        let response = live.fixture.state.handle_request(Request::Reload);
        assert!(
            format!("{response:?}").contains("output.scale"),
            "the reload did not apply the scale: {response:?}"
        );
        let placement = live.placement(id);
        let want = scaled(placement.rect, scale);
        let mut seen = (0, 0, 0, 0);
        eventually(
            &mut live.fixture,
            "the X window reconfigured into the new X pixels",
            |_| {
                seen = {
                    let (x, y, w, h) = live.x.root_geometry(xid);
                    (x, y, w as i32, h as i32)
                };
                seen == want
            },
        );
        eventually(&mut live.fixture, "the X screen resized", |_| {
            x_screen(&live.x) == (CANVAS as u16, CANVAS as u16)
        });
        let expected = if scale == 1 {
            (Some(1), Some(96 * 1024), Some(96 * 1024))
        } else {
            (Some(2), Some(2 * 96 * 1024), Some(96 * 1024))
        };
        assert_eq!(toolkit_scale(&live.x), expected, "at scale {scale}");
        // Redrawn at the new X size: the surface's logical size is the
        // placement again, and it is on screen where it is placed.
        eventually(&mut live.fixture, "the X window redrawn", |fixture| {
            fixture
                .state
                .window(id)
                .and_then(Window::x11_surface)
                .is_some_and(|x11| x11.bbox().size == x11.last_configure().size)
        });
        let (cx, cy) = centre(placement.rect);
        #[allow(clippy::cast_possible_truncation)]
        let (px, py) = (cx as i32 * scale, cy as i32 * scale);
        let mut pixel = [0; 4];
        for _ in 0..50 {
            pixel = live.pixel_at(px, py);
            if pixel == RED_BGRA {
                break;
            }
            live.drain();
        }
        assert_eq!(pixel, RED_BGRA, "at scale {scale}");
    }
}

/// Wayland to X at scale 2: the window manager tells the X target where the
/// drag is in X root pixels -- the logical pointer times two -- and the drop
/// lands, bytes and all.
#[test]
fn a_wayland_drag_drops_onto_an_x_window_at_scale_2() {
    let Some(mut live) = live_scaled("a_wayland_drag_drops_onto_an_x_window_at_scale_2", 2.0)
    else {
        return;
    };
    let wayland = live.map_peer("wayland");
    assert!(matches!(
        live.fixture.run(Step::Clip(ClipStep::Bind)),
        Ack::Done
    ));
    let target_client = XClient::connect(live.display);
    let target_xid = target_client.map(&Props::new(RED));
    let target = live.managed(target_xid);
    target_client.xdnd_aware(target_xid);
    let (from, to) = (live.placement(wayland), live.placement(target));
    visible("Wayland window", &from);
    visible("drop target", &to);
    let payload = Arc::new(b"dropped at scale 2".to_vec());
    start_wayland_drag(&mut live, from.rect, &payload);

    let (x, y) = centre(to.rect);
    live.fixture.state.pointer_move(x, y);
    settle(&mut live.fixture);
    let utf8 = target_client.atom("UTF8_STRING");
    let copy = target_client.atom("XdndActionCopy");
    let mut inbox = Inbox::new(&target_client);
    let source = inbox
        .message(&mut live.fixture, "XdndEnter")
        .data
        .as_data32()[0];
    let position = inbox.message(&mut live.fixture, "XdndPosition");
    #[allow(clippy::cast_possible_truncation)]
    let at = packed((x * 2.0) as i16, (y * 2.0) as i16);
    assert_eq!(
        position.data.as_data32()[2],
        at,
        "XdndPosition is not the pointer in X root pixels"
    );
    target_client.xdnd_send(source, "XdndStatus", [target_xid, 1, 0, 0, copy]);
    settle(&mut live.fixture);
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    settle(&mut live.fixture);
    inbox.message(&mut live.fixture, "XdndDrop");
    target_client.convert("XdndSelection", utf8, "SCOOT_DROP", target_xid);
    let notify = inbox.selection_notify(&mut live.fixture);
    assert_ne!(notify.property, x11rb::NONE, "the conversion was refused");
    assert_eq!(
        target_client.read_property(target_xid, "SCOOT_DROP"),
        *payload
    );
    target_client.xdnd_send(source, "XdndFinished", [target_xid, 1, copy, 0, 0]);
    settle(&mut live.fixture);
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// X out to Wayland and across X at scale 2: an X drag's source looks for
/// its target under the X pointer, which must be the logical pointer times
/// two -- over another X client's window (an override-redirect one here, a
/// drop-down, so the 200-logical-pixel screen holds all three) it finds
/// that window, and over a Wayland window the window manager's proxy,
/// which covers the whole X screen.
#[test]
fn an_x_drag_finds_its_targets_at_scale_2() {
    let Some(mut live) = live_scaled("an_x_drag_finds_its_targets_at_scale_2", 2.0) else {
        return;
    };
    let wayland = live.map_peer("wayland");
    let (_, source, _) = managed_at(&mut live, 2);
    let (from, on_wayland) = (live.placement(source), live.placement(wayland));
    visible("drag source", &from);
    visible("Wayland window", &on_wayland);
    // Over the source's lower part, in X pixels: logical (from.x + 5,
    // 150), 30 square.
    let other = XClient::connect(live.display);
    let target_xid = other.map(&Props {
        rect: (((from.rect.x + 5) * 2) as i16, 300, 60, 60),
        override_redirect: true,
        ..Props::new(BLUE)
    });
    other.xdnd_aware(target_xid);
    eventually(&mut live.fixture, "the drop-down drawn", |fixture| {
        fixture
            .state
            .x11_unmanaged
            .iter()
            .any(|known| known.window_id() == target_xid && known.wl_surface().is_some())
    });
    live.drain();

    super::dnd::press_at(
        &mut live.fixture.state,
        Rect::new(from.rect.x, 20, from.rect.w, 60),
    );
    live.drain();
    let owner = live.x.take_selection("XdndSelection");
    live.drain();
    assert!(
        super::dnd::taken_over(&live.fixture.state),
        "the X drag did not start"
    );
    let (px, py, proxy) = live.x.pointer();
    assert_eq!(live.x.name_of(proxy), PROXY_NAME);
    let utf8 = live.x.atom("UTF8_STRING");
    let copy = live.x.atom("XdndActionCopy");
    live.x
        .xdnd_send(proxy, "XdndEnter", [owner, XDND_VERSION << 24, utf8, 0, 0]);
    live.x.xdnd_send(
        proxy,
        "XdndPosition",
        [owner, 0, packed(px, py), x11rb::CURRENT_TIME, copy],
    );
    live.drain();

    let over_target = (f64::from(from.rect.x + 20), 165.0);
    for ((x, y), what) in [
        (over_target, "the other X client's window"),
        (centre(on_wayland.rect), "the Wayland window"),
    ] {
        live.fixture.state.pointer_move(x, y);
        live.drain();
        let (px, py, under) = live.x.pointer();
        #[allow(clippy::cast_possible_truncation)]
        let want = ((x * 2.0) as i16, (y * 2.0) as i16);
        assert_eq!(
            (px, py),
            want,
            "over {what}, the X server has the pointer elsewhere"
        );
        if what == "the Wayland window" {
            assert_eq!(live.x.name_of(under), PROXY_NAME, "over {what}");
        } else {
            assert_eq!(under, target_xid, "over {what}: {}", live.x.name_of(under));
        }
    }
    live.fixture
        .state
        .pointer_button(PointerButton::Left, false);
    live.drain();
    assert!(
        !grabbed(&live.fixture.state),
        "the drag outlived the release"
    );
}

/// A reload re-sends every X window's last configure in the new X pixels
/// through the wire clamp again: a window left far off screen (a column
/// scrolled away, clamped to `INT16` for the old scale) must not wrap on
/// the wire at the new one. The window sits on a workspace that is not
/// shown, so `apply()` never re-places it and the re-send is all it gets.
#[test]
fn a_runtime_rescale_keeps_far_x_windows_in_x_range() {
    use scoot_core::{Action, Vertical};
    use smithay::utils::Rectangle;

    let Some(mut live) = live_scaled("a_runtime_rescale_keeps_far_x_windows_in_x_range", 1.0)
    else {
        return;
    };
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("config.toml");
    live.fixture.state.config_path = Some(path.clone());
    let (xid, id, _) = managed_at(&mut live, 1);
    let _ = live
        .fixture
        .state
        .world
        .handle_action(Action::FocusWorkspace(Vertical::Down));
    live.fixture.state.apply();
    assert!(!live.placement(id).visible, "the window is still shown");
    // Where a far-scrolled column's configure leaves it: inside `INT16` at
    // scale 1, twice past it at 2.
    live.fixture
        .state
        .window(id)
        .and_then(Window::x11_surface)
        .expect("an X window")
        .configure(Rectangle::new((20_000, 10).into(), (100, 100).into()))
        .expect("a configure");
    eventually(&mut live.fixture, "the far configure landing", |_| {
        live.x.root_geometry(xid).0 == 20_000
    });

    fs::write(&path, "[output]\nscale = 2.0\n").expect("a config file");
    let response = live.fixture.state.handle_request(Request::Reload);
    assert!(format!("{response:?}").contains("output.scale"));
    let mut x = 0;
    eventually(&mut live.fixture, "the rescaled configure landing", |_| {
        x = live.x.root_geometry(xid).0;
        x != 20_000
    });
    assert_eq!(
        x, 32_766,
        "the far window's position wrapped or left the X range at scale 2"
    );
}
