//! Which CRTC drives which connector, when `--tty` drives more than one.
//!
//! A connector reaches the screen through an encoder, and each encoder can
//! only be routed to the CRTCs its `possible_crtcs` mask names. On a PC GPU
//! that mask is usually "any of them"; on an SoC display controller it is
//! often exactly one -- Apple's DCP wires `eDP-1`'s encoder to CRTC 0 and
//! `DP-1`'s to CRTC 1 and nothing else (measured with `drm_info` on the M2
//! Air, 2026-09-25). A first-come search that hands the first connector the
//! first CRTC that takes a surface would work there only by luck of order:
//! Smithay's `create_surface` refuses a CRTC only once its primary plane is
//! already claimed, not because the encoder cannot reach it.
//!
//! So the choice is made here, up front, from the masks alone: a maximum
//! bipartite matching (connectors on one side, CRTCs on the other), built by
//! augmenting paths in connector order. Two properties fall out of that
//! order and are what the callers rely on:
//!
//! - **An earlier connector never loses its CRTC to a later one.** An
//!   augmenting path may *re-route* an already-matched connector onto another
//!   CRTC it can use, but it never unmatches one. So when there are more
//!   connectors than CRTCs, the ones the kernel lists first are the ones
//!   driven -- the same "kernel order" the single-output search always used.
//! - **As many connectors as possible get one.** A greedy pass would give
//!   connector A (reachable from CRTCs 0 and 1) CRTC 0 and then find nothing
//!   left for connector B (reachable from CRTC 0 only); the matching moves A
//!   to CRTC 1 and drives both.
//!
//! Pure and generic over the handle type, so the whole decision is pinnable
//! with plain integers: the live half (reading encoders, building surfaces)
//! needs a DRM device, and `drm` 0.14.1's `connector::Info` cannot be built
//! outside that crate (its fields are `pub(crate)` with no constructor), so a
//! fake device could not stand in for it anyway.

#[cfg(test)]
mod tests;

/// For each connector in `possible` (in the order given), the CRTC it should
/// drive, or `None` when no CRTC is left that it can be routed to.
///
/// `possible[i]` is connector `i`'s reachable CRTCs, in the order it should
/// prefer them (the device's own CRTC order, as `ResourceHandles::filter_crtcs`
/// returns them). `busy` are CRTCs already driving something this call must
/// not disturb -- a connector plugged in while others are lit gets only what
/// is free, and the lit ones are never re-routed to make room.
///
/// Allocation is two small vectors per call, on paths that run at startup
/// and when a cable moves -- never per frame.
pub(super) fn assign<T: Copy + Eq>(possible: &[Vec<T>], busy: &[T]) -> Vec<Option<T>> {
    let mut owner: Vec<(T, usize)> = Vec::new();
    for connector in 0..possible.len() {
        let mut visited: Vec<T> = Vec::new();
        augment(connector, possible, busy, &mut owner, &mut visited);
    }
    (0..possible.len())
        .map(|connector| {
            owner
                .iter()
                .find(|(_, owned_by)| *owned_by == connector)
                .map(|(crtc, _)| *crtc)
        })
        .collect()
}

/// Tries to give `connector` a CRTC: a free one it can reach if there is one
/// (in its preference order), and only otherwise one freed by re-routing an
/// already-matched connector along an augmenting path. Free-first is what
/// keeps a lit arrangement stable: nothing is moved while a connector can be
/// served without moving it. Depth is bounded by the number of CRTCs (each is
/// visited at most once per top-level call), which is a handful on any real
/// device.
fn augment<T: Copy + Eq>(
    connector: usize,
    possible: &[Vec<T>],
    busy: &[T],
    owner: &mut Vec<(T, usize)>,
    visited: &mut Vec<T>,
) -> bool {
    let Some(reachable) = possible.get(connector) else {
        return false;
    };
    let free = reachable.iter().copied().find(|crtc| {
        !busy.contains(crtc)
            && !visited.contains(crtc)
            && !owner.iter().any(|(owned, _)| owned == crtc)
    });
    if let Some(crtc) = free {
        owner.push((crtc, connector));
        return true;
    }
    for &crtc in reachable {
        if busy.contains(&crtc) || visited.contains(&crtc) {
            continue;
        }
        visited.push(crtc);
        let Some(slot) = owner.iter().position(|(owned, _)| *owned == crtc) else {
            // Unreachable: the free pass above would have taken it.
            continue;
        };
        let holder = owner[slot].1;
        if augment(holder, possible, busy, owner, visited) {
            // `holder` moved to another CRTC (pushed as a new entry by the
            // recursive call), so this one is free for us. The slot index is
            // still valid: entries are only ever pushed, never removed,
            // during one top-level call.
            owner[slot].1 = connector;
            return true;
        }
    }
    false
}
