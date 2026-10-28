//! The pure half of the CPU-tier seat-reconnect swap's binding tail: which
//! fresh heads still need an output, and whether the swapped-in device may
//! report `live: true` once binding is done.
//!
//! The CPU `swap` used to bind with `rebind`, whose `retain_attached` drops
//! every fresh head no binding claims. After an empty restore (the old
//! device drove nothing, so the next attempt's binding snapshot is empty)
//! that dropped *every* head, yet the fresh `Tty` had already been built
//! `active: true` on the strength of the proof: no head presenting, the one
//! stale output kept because the last output is never removed, and `live:
//! true` on a screen showing nothing until the next seat win healed it. The
//! GPU tier's `swap_scanout` never had the hole (it routes unbound heads
//! through `Change::Added` and downgrades a swap that presents nothing);
//! this module is the shared, headlessly-testable form of those two rules
//! for the CPU swap.

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
