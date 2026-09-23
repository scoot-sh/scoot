//! An input method's candidate window hanging off an application's menu.
//!
//! An input-method popup is placed against whichever surface holds the
//! focused text field, and a menu with a search box is one: the candidate
//! window is then a popup *of the menu*, and it belongs to the input
//! method's connection, not the application's. It is never anyone's parent,
//! so it cannot deepen a chain, and children are counted only for
//! `xdg_popup`s -- which is what lets the application close that menu
//! without being told it left a child behind.
//!
//! One connection plays both halves, as in `input_method/tests.rs`: Smithay
//! only routes a text input to an input method on the focused client, and a
//! second connection would add nothing this test is about.

use smithay::desktop::PopupKind;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface;
use wayland_client::Proxy;
use wayland_client::protocol::wl_keyboard;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3;
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_v2, zwp_input_popup_surface_v2,
};

use super::*;

impl Dispatch<wl_keyboard::WlKeyboard, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { serial, .. } = event {
            client.keyboard_enter = Some(serial);
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for TestClient {
    fn event(
        client: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => client.ime_active = true,
            zwp_input_method_v2::Event::Deactivate => client.ime_active = false,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(TestClient: ignore zwp_text_input_v3::ZwpTextInputV3);
wayland_client::delegate_noop!(
    TestClient: ignore zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2
);

enum ImeStep {
    /// A menu of the window, taking an explicit grab (on the keyboard
    /// `enter` serial) so the keyboard -- and with it the text input --
    /// moves onto it; mapped. Answers the menu's `wl_surface` id.
    MenuWithGrab,
    /// `zwp_text_input_v3.enable` + `commit`: the menu's search box is
    /// ready. Answers whether the input method was activated.
    EnableTextInput,
    /// `zwp_input_method_v2.get_input_popup_surface`: the candidate window.
    /// Answers its `wl_surface` id.
    CandidateWindow,
    /// `xdg_popup.destroy` on the menu, with the candidate window still up.
    CloseMenu,
}

#[derive(Debug)]
enum ImeAck {
    Surface(u32),
    Active(bool),
    Done,
}

fn run_ime_client(
    stream: UnixStream,
    steps: Receiver<ImeStep>,
    acks: Sender<ImeAck>,
) -> Result<(), String> {
    let conn = Connection::from_socket(stream).map_err(|e| e.to_string())?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut client = TestClient::default();
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let globals = Globals {
        compositor: client.compositor.clone().ok_or("no wl_compositor")?,
        shm: client.shm.clone().ok_or("no wl_shm")?,
        wm_base: client.wm_base.clone().ok_or("no xdg_wm_base")?,
        subcompositor: client.subcompositor.clone().ok_or("no wl_subcompositor")?,
        layer_shell: client.layer_shell.clone().ok_or("no zwlr_layer_shell_v1")?,
    };
    let seat = client.seat.clone().ok_or("no wl_seat")?;
    let text_inputs = client
        .text_input_manager
        .clone()
        .ok_or("no zwp_text_input_manager_v3")?;
    let input_methods = client
        .input_method_manager
        .clone()
        .ok_or("no zwp_input_method_manager_v2")?;

    // The input method first: Smithay only enters a text input on focus if
    // an input method already exists (see `input_method/tests.rs`).
    let input_method = input_methods.get_input_method(&seat, &qh, ());
    let _keyboard = seat.get_keyboard(&qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
    let mut made = Made::default();
    map_window(&mut queue, &mut client, &globals, &qh, &mut made)?;
    let text_input = text_inputs.get_text_input(&seat, &qh, ());
    queue.roundtrip(&mut client).map_err(|e| e.to_string())?;

    let mut candidates = Vec::new();
    while let Ok(step) = steps.recv() {
        let ack = match step {
            ImeStep::MenuWithGrab => {
                let serial = wait_for(&mut queue, &mut client, "keyboard focus", |c| {
                    c.keyboard_enter
                })?;
                let index = made.popups.len();
                client.popups.push(PopupRecord::default());
                let surface = globals.compositor.create_surface(&qh, ());
                let xdg = globals
                    .wm_base
                    .get_xdg_surface(&surface, &qh, Role::Popup(index));
                let popup =
                    made.get_popup(&globals, &qh, &xdg, Parent::Window(0), Role::Popup(index))?;
                popup.grab(&seat, serial);
                surface.commit();
                let configure = wait_for(&mut queue, &mut client, "a popup configure", |c| {
                    c.popups.get(index)?.serial
                })?;
                xdg.ack_configure(configure);
                let (buffer, w, h) =
                    solid_buffer(&globals.shm, &qh, POPUP_SIZE, POPUP_SIZE, POPUP_BGRA)?;
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, w, h);
                surface.commit();
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let id = surface.id().protocol_id();
                made.popups.push(Popup {
                    surface,
                    xdg,
                    popup,
                });
                ImeAck::Surface(id)
            }
            ImeStep::EnableTextInput => {
                text_input.enable();
                text_input.commit();
                let active = wait_for(&mut queue, &mut client, "input method activation", |c| {
                    c.ime_active.then_some(())
                });
                ImeAck::Active(active.is_ok())
            }
            ImeStep::CandidateWindow => {
                let surface = globals.compositor.create_surface(&qh, ());
                input_method.get_input_popup_surface(&surface, &qh, ());
                queue.roundtrip(&mut client).map_err(|e| e.to_string())?;
                let id = surface.id().protocol_id();
                candidates.push(surface);
                ImeAck::Surface(id)
            }
            ImeStep::CloseMenu => {
                made.popups.first().ok_or("no menu")?.popup.destroy();
                sync(&conn, &mut queue, &mut client)?;
                ImeAck::Done
            }
        };
        acks.send(ack).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn server_surface(fixture: &Harness<ImeStep, ImeAck>, id: u32) -> ServerSurface {
    fixture
        .client(0)
        .object_from_protocol_id(&fixture.state.display_handle, id)
        .expect("the client's surface")
}

/// The menu is the candidate window's parent -- checked, since without it
/// the rest proves nothing -- and closing the menu under it is not a
/// `not_the_topmost_popup`: the client is not disconnected, and nothing is
/// logged.
#[test]
fn closing_a_menu_under_an_input_methods_candidate_window_is_fine() {
    // The whole run inside the capture, the compositor's creation included
    // (see `capture_logs`).
    let (ack, logs) = test_support::capture_logs(|| {
        let mut fixture: Harness<ImeStep, ImeAck> = Harness::headless(appearance(), CANVAS);
        fixture.spawn(run_ime_client);
        let ImeAck::Surface(menu) = fixture.run(ImeStep::MenuWithGrab) else {
            panic!("expected the menu's surface id");
        };
        let ImeAck::Active(active) = fixture.run(ImeStep::EnableTextInput) else {
            panic!("expected the input method's state");
        };
        assert!(active, "the input method never activated for the menu");
        let ImeAck::Surface(candidate) = fixture.run(ImeStep::CandidateWindow) else {
            panic!("expected the candidate window's surface id");
        };

        let menu = server_surface(&fixture, menu);
        let candidate = server_surface(&fixture, candidate);
        let Some(PopupKind::InputMethod(popup)) = fixture.state.popups.find_popup(&candidate)
        else {
            panic!("the candidate window is not tracked as an input-method popup");
        };
        assert_eq!(
            popup.get_parent().map(|parent| parent.surface.clone()),
            Some(menu),
            "the candidate window is not a popup of the menu"
        );

        fixture.run(ImeStep::CloseMenu)
    });

    assert!(matches!(ack, ImeAck::Done), "{ack:?}");
    assert!(!logs.contains("still has child popups"), "{logs}");
}
