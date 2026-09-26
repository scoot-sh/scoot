//! Disconnecting a client with `wl_display.error(no_memory)`: the refusal for
//! "this compositor will not hold more of your resources".
//!
//! Some per-client bounds guard a request whose own interface has no error
//! that fits. A pre-commit hook has no refusal at all (it has no return
//! value), and `zwp_linux_buffer_params_v1.add` has codes only for malformed
//! planes. Those bounds disconnect with `no_memory` instead ("server is out
//! of memory"): it is the code libwayland servers send when a client's
//! demands cannot be met, and it names the reason. The callers are
//! `drm_syncobj/acquire.rs` (outstanding acquire waits),
//! `dmabuf/pending_planes.rs` (an `add` past the plane fds held before
//! `create`, or past the client's fds in the fd ledger, `client_fds.rs`)
//! and `toplevel_cap.rs` (an `xdg_toplevel` past the client's live-toplevel
//! cap).

use smithay::reexports::wayland_server::backend::protocol::{Interface, ProtocolError};
use smithay::reexports::wayland_server::{Client, DisplayHandle};

/// `wl_display.error`'s `no_memory` code ("server is out of memory").
/// wayland-server generates no server-side `wl_display` bindings (the
/// display object is the backend's own), so the value is spelled out from
/// `wayland.xml`.
pub(super) const WL_DISPLAY_NO_MEMORY: u32 = 2;

/// Just enough of `wl_display`'s interface description to name a client's
/// display object (always protocol id 1) to the backend, which matches
/// interfaces by name. wayland-backend keeps its own copy private, and only
/// the name is read.
static WL_DISPLAY: Interface = Interface {
    name: "wl_display",
    version: 1,
    requests: &[],
    events: &[],
    c_ptr: None,
};

/// Disconnects `client` with `wl_display.error(no_memory)` and `message`.
///
/// The error is posted on the client's display object, which is how
/// libwayland reports it. `Client::kill` alone would disconnect without
/// sending any error event (wayland-backend's `kill_client` only marks the
/// client dead), and the client, and anyone reading its log, would see only
/// a bare hang-up. That is still the fallback if the display object cannot
/// be named, though it always exists while the client does.
///
/// Allocates (the message, the `CString`). That is fine only because every
/// caller is on the path that has just refused a client, never on a served
/// request.
pub(super) fn disconnect(dh: &DisplayHandle, client: &Client, message: String) {
    let backend = dh.backend_handle();
    match backend.object_for_protocol_id(client.id(), &WL_DISPLAY, 1) {
        Ok(display) => backend.post_error(
            display,
            WL_DISPLAY_NO_MEMORY,
            std::ffi::CString::new(message).unwrap_or_default(),
        ),
        Err(_) => client.kill(
            dh,
            ProtocolError {
                code: WL_DISPLAY_NO_MEMORY,
                object_id: 1,
                object_interface: WL_DISPLAY.name.to_owned(),
                message,
            },
        ),
    }
}
