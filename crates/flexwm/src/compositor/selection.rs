//! Clipboard and primary-selection globals: three Smithay protocol states.
//!
//! - `zwlr_data_control_manager_v1` (version 2): clipboard managers
//!   (`cliphist`, `clipman`).
//! - `ext_data_control_manager_v1` (version 1): the successor protocol, for
//!   managers that speak the new generation. Exposed alongside the old one,
//!   the way current compositors do, rather than picking a generation for
//!   the client.
//! - `zwp_primary_selection_device_manager_v1` (version 1): middle-click
//!   paste.
//!
//! All three are Smithay's own server implementations (see
//! `src/wayland/selection/` at the pinned rev), constructed in `State::new`
//! and routed through `handlers.rs`. Both data-control states are built with
//! `Some(&primary_selection_state)` so a clipboard manager can also touch the
//! primary selection, matching toolkit expectations; all three use an
//! allow-everyone filter (`|_| true`), because flexwm has no
//! security-context support and an allow-list would be theatre -- the same
//! rationale as the session-lock global, documented in `README.md`'s trust
//! note.
//!
//! Nothing here is per-frame: selection traffic is per client action, so
//! there is nothing to benchmark (see `tests.rs` for the round-trips this
//! covers instead).

#[cfg(test)]
mod tests;
