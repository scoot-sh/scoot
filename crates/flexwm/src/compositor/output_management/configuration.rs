//! The write half of `wlr-output-management-unstable-v1`, which flexwm refuses.
//!
//! `zwlr_output_manager_v1.create_configuration` is the protocol's only door
//! into reconfiguration, so this file is the whole of it: a client may build a
//! configuration object and describe whatever it likes on it, and `apply` and
//! `test` both answer `failed`.
//!
//! ## Why refuse rather than not offer, and why refuse rather than no-op
//!
//! The configuration requests cannot be left out: they are part of version 1
//! of the interface, so advertising the manager at all advertises them. What
//! *is* a choice is the answer.
//!
//! - **Not a silent success.** flexwm has exactly one output whose mode,
//!   position, scale and transform are fixed for the process (see the parent
//!   module). A configuration that reported `succeeded` and changed nothing
//!   would give a shell a Display page whose buttons appear to work -- the
//!   worst of the three outcomes, and the reason this item shipped read-only
//!   rather than stubbed.
//! - **Not `cancelled`.** That means "your information is stale, build a new
//!   configuration with a newer serial and try again", which is an invitation
//!   to a retry loop that can never succeed.
//! - **`failed`** is the protocol's own word for "the compositor rejects the
//!   changes". `wlr-randr` prints it and exits; a shell shows an error. That
//!   is exactly what is true.
//!
//! The one thing a refusal still has to get right is the object's own state
//! machine, because the protocol makes it a client-visible error: after
//! `apply` or `test`, any request but the destructor is `already_used`.
//!
//! ## What is deliberately not validated
//!
//! `already_configured_head`, `unconfigured_head`, `already_set`,
//! `invalid_mode`, `invalid_custom_mode`, `invalid_transform`,
//! `invalid_scale`, `invalid_adaptive_sync_state`: none of these are checked.
//! They exist so a compositor that is going to *act* on a configuration can
//! reject an incoherent one before it does; this one rejects every
//! configuration whether coherent or not, so enforcing them would add a state
//! machine whose only effect is to disconnect a client that was going to be
//! refused anyway. A client gets `failed` instead of a protocol error, which
//! is the gentler of the two and just as final.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_head_v1::{
    self, ZwlrOutputConfigurationHeadV1,
};
use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_configuration_v1::{
    self, ZwlrOutputConfigurationV1,
};
use smithay::reexports::wayland_server::{Client, DataInit, DisplayHandle, Resource};
use smithay::wayland::Dispatch2;

use crate::compositor::State;

/// User data on a `zwlr_output_configuration_v1`.
#[derive(Default)]
pub(super) struct ConfigurationData {
    /// Whether `apply` or `test` has already been answered on this object.
    ///
    /// Per-object rather than a list on [`State`], which is what keeps this
    /// bounded with nothing to clean up: the flag lives and dies with the
    /// client's own object, so a client that opens configurations and never
    /// destroys them grows its own object table and nothing of the
    /// compositor's.
    ///
    /// Atomic because wayland user data must be `Send + Sync`. Every access is
    /// on the event-loop thread, so `Relaxed` is the right ordering -- there
    /// is no other thread to synchronise with.
    used: AtomicBool,
}

impl ConfigurationData {
    /// Posts `already_used` and returns `true` when this configuration has
    /// already been applied or tested.
    ///
    /// The protocol is explicit: after `apply` or `test`, "sending a request
    /// that isn't the destructor is a protocol error".
    fn refuse_if_used(&self, configuration: &ZwlrOutputConfigurationV1) -> bool {
        if !self.used.load(Ordering::Relaxed) {
            return false;
        }
        configuration.post_error(
            zwlr_output_configuration_v1::Error::AlreadyUsed,
            "this output configuration has already been applied or tested",
        );
        true
    }

    /// Answers an `apply` or a `test`: always `failed`, once.
    fn refuse(&self, configuration: &ZwlrOutputConfigurationV1) {
        if self.refuse_if_used(configuration) {
            return;
        }
        self.used.store(true, Ordering::Relaxed);
        configuration.failed();
    }
}

impl Dispatch2<ZwlrOutputConfigurationV1, State> for ConfigurationData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        configuration: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head: _ } => {
                // Initialized *before* the used check, and unconditionally: a
                // `New` left uninitialized is a panic path in wayland-backend
                // (see `dispatch.rs`'s module doc), and the argument that
                // makes not initializing it safe depends on the client being
                // killed synchronously -- which is true here, but costs
                // nothing to avoid relying on. An object on a client about to
                // be disconnected is inert.
                data_init.init(id, ConfigurationHeadData);
                self.refuse_if_used(configuration);
            }
            zwlr_output_configuration_v1::Request::DisableHead { head: _ } => {
                self.refuse_if_used(configuration);
            }
            zwlr_output_configuration_v1::Request::Apply
            | zwlr_output_configuration_v1::Request::Test => self.refuse(configuration),
            // The destructor. The client's configuration-head objects go with
            // it, per the protocol; nothing here tracks them.
            zwlr_output_configuration_v1::Request::Destroy => {}
            // Same reasoning as the manager's catch-all in the parent module:
            // unreachable at every version this compositor advertises, and not
            // worth a panic if a future one makes it reachable.
            _ => {}
        }
    }
}

/// User data on a `zwlr_output_configuration_head_v1`.
///
/// Empty: every request on it is discarded (see below), so there is nothing
/// per-object to remember.
pub(super) struct ConfigurationHeadData;

impl Dispatch2<ZwlrOutputConfigurationHeadV1, State> for ConfigurationHeadData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _head: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        // Every request here describes a property of a configuration that the
        // parent object will refuse wholesale, so all of them are discarded.
        // Recording them would build state whose only reader is a `succeeded`
        // path that does not exist. Logged at `debug` so "my settings page did
        // nothing" has a trail.
        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { .. }
            | zwlr_output_configuration_head_v1::Request::SetCustomMode { .. }
            | zwlr_output_configuration_head_v1::Request::SetPosition { .. }
            | zwlr_output_configuration_head_v1::Request::SetTransform { .. }
            | zwlr_output_configuration_head_v1::Request::SetScale { .. }
            | zwlr_output_configuration_head_v1::Request::SetAdaptiveSync { .. } => {
                tracing::debug!(
                    "ignoring an output-configuration property: flexwm refuses every \
                     output configuration"
                );
            }
            _ => {}
        }
    }
}
