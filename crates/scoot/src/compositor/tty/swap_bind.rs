//! The pure half of the CPU-tier seat-reconnect swap's binding tail: which
//! fresh heads still need an output, and whether the swapped-in device may
//! report `live: true` once binding is done.
//!
//! The CPU `swap` binds by connector without dropping the fresh heads no
//! binding claims (`rebind`'s `retain_attached` would, leaving a proven swap
//! `active` with nothing presenting). Unclaimed heads take the
//! `Change::Added` shape, and a swap whose heads present nothing reports
//! `live: false` (and holds the primary when it has no heads at all). The GPU
//! tier's `swap_scanout` follows the same two rules; this module is their
//! headlessly-testable form for the CPU swap.

use super::reconnect::{SwapLive, swap_downgrade};

/// Indices of the fresh heads that no binding claimed. `bound[i]` is whether
/// head `i` has an output after the bindings were applied. Each of these is
/// routed through the hotplug `Added` shape (a new output, attached by
/// connector) rather than dropped.
pub(super) fn unbound_heads(bound: &[bool]) -> Vec<usize> {
    bound
        .iter()
        .enumerate()
        .filter_map(|(index, bound)| (!bound).then_some(index))
        .collect()
}

/// What the swapped-in device reports once every output creation has been
/// tried: the possibly-downgraded [`SwapLive`], the `active` to write onto
/// the fresh `Tty`, and its `nothing_connected` hold. `bound[i]` is whether
/// head `i` presents an output *now* (after the `Added` changes applied).
/// A proven swap with no presenting head is dark: `active` must clear or
/// IPC reports `live: true` on a blank screen.
pub(super) fn verdict(live: SwapLive, bound: &[bool]) -> (SwapLive, bool, bool) {
    swap_downgrade(live, false, !bound.contains(&true), bound.is_empty())
}

#[cfg(test)]
mod tests;
