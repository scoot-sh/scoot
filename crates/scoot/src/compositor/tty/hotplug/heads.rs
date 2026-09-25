//! What a fresh probe means for *every* head at once: which ones keep going,
//! which one follows its screen onto a new mode, which ones go dark and are
//! taken away, and which newly connected connectors get a head of their own
//! (milestone 19, phase E2).
//!
//! Pure, like `super::plan`, and for the same reason: everything around it
//! -- probing connectors, building surfaces, the `wl_output` lifecycle --
//! needs a DRM device or a whole `State`, and the decision is where the
//! mistakes that strand a user live. Generic over the connector type so the
//! pins use plain integers.
//!
//! The rules, in the order they are checked:
//!
//! 1. **Nothing driven is still connected.** The single-output rules from
//!    before multi-output, applied to the primary head: if another connector
//!    is connected, the primary moves onto it (the issue #48 fallback); if
//!    none is, the primary holds its last frame until one comes back. Every
//!    other head is removed either way. The primary is never removed: a
//!    session always keeps an output (see `State::remove_output`).
//! 2. **Otherwise** each head whose connector is gone is removed, and each
//!    one still connected keeps going -- following a new mode size if its
//!    screen re-probed to one, and forcing a modeset if it is the primary
//!    coming back from a hold at the same size (the undock/redock case).
//! 3. **Every connected connector no head drives** gets a head (as far as
//!    free CRTCs and `MAX_OUTPUTS` allow -- that is the caller's to enforce,
//!    since only it knows the device). A second monitor plugged into a
//!    laptop is an added output; the panel's head is untouched, which is the
//!    old "stay on the panel the user is looking at" rule's multi-output form.

use super::{Plan, plan};

/// One driven head as the probe saw it: its connector, the size it is
/// driving, and what its own connector says now (`None` when it is no longer
/// `Connected` with a mode).
#[derive(Clone, Copy, Debug)]
pub(super) struct Probed<C> {
    pub(super) connector: C,
    pub(super) size: (i32, i32),
    pub(super) now: Option<(i32, i32)>,
}

/// What to do with one existing head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HeadAction<C> {
    /// Nothing about this head changed.
    Keep,
    /// The same connector re-probed to this size: mode-set onto it.
    NewMode((i32, i32)),
    /// The primary's connector came back at the size it left on after a
    /// hold: force a modeset, since the CRTC spent the gap driving a
    /// connector that had physically gone away.
    Reconnected,
    /// The primary's connector is gone and this one is connected: move the
    /// primary head onto it, at this size.
    MoveTo(C, (i32, i32)),
    /// Nothing is connected at all: the primary holds its last frame.
    Hold,
    /// This head's connector is gone and it is not the last screen: take
    /// the head and its output away.
    Remove,
}

/// The whole plan: one action per head (same order as the input), and the
/// connectors to build new heads for, in the kernel's order.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Replan<C> {
    pub(super) heads: Vec<HeadAction<C>>,
    pub(super) add: Vec<(C, (i32, i32))>,
}

/// Decides what a probe means. `heads` is every driven head in creation
/// order (the primary first); `undriven` is every connected connector no
/// head drives, in the kernel's order, with the size it would be driven at;
/// `holding` is whether the primary is currently holding its last frame
/// (`Tty::nothing_connected`).
pub(super) fn replan<C: Copy + PartialEq>(
    heads: &[Probed<C>],
    undriven: &[(C, (i32, i32))],
    holding: bool,
) -> Replan<C> {
    if heads.is_empty() {
        // Unreachable -- `Tty` never has zero heads -- but an empty plan is
        // the answer that cannot strand anyone: nothing is removed, and no
        // head is built without the primary it would stand beside.
        return Replan {
            heads: Vec::new(),
            add: Vec::new(),
        };
    }
    if heads.iter().all(|head| head.now.is_none()) {
        let mut actions = vec![HeadAction::Remove; heads.len()];
        let add = match undriven.split_first() {
            Some((&(connector, size), rest)) => {
                actions[0] = HeadAction::MoveTo(connector, size);
                rest.to_vec()
            }
            None => {
                actions[0] = HeadAction::Hold;
                Vec::new()
            }
        };
        return Replan {
            heads: actions,
            add,
        };
    }
    let actions = heads
        .iter()
        .enumerate()
        .map(|(index, head)| match head.now {
            None => HeadAction::Remove,
            // Through `super::plan` on the head's own connector: sizes, not
            // modes (see its doc), so a re-ranked mode list at the same size
            // is no change.
            Some(size) => match plan((head.connector, head.size), Some((head.connector, size))) {
                Plan::NewMode => HeadAction::NewMode(size),
                Plan::Unchanged if index == 0 && holding => HeadAction::Reconnected,
                Plan::Unchanged | Plan::NewConnector | Plan::NoConnector => HeadAction::Keep,
            },
        })
        .collect();
    Replan {
        heads: actions,
        add: undriven.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDP: u32 = 52;
    const DP: u32 = 70;
    const HDMI: u32 = 82;
    const PANEL: (i32, i32) = (2560, 1600);
    const FHD: (i32, i32) = (1920, 1080);

    fn alive(connector: u32, size: (i32, i32)) -> Probed<u32> {
        Probed {
            connector,
            size,
            now: Some(size),
        }
    }

    fn gone(connector: u32, size: (i32, i32)) -> Probed<u32> {
        Probed {
            connector,
            size,
            now: None,
        }
    }

    #[test]
    fn plugging_a_second_monitor_adds_an_output_and_leaves_the_panel_alone() {
        // The E2 change and its regression pin in one: the panel's head is
        // `Keep` (the old "stay on the panel" rule), and the monitor gets a
        // head of its own rather than being ignored.
        assert_eq!(
            replan(&[alive(EDP, PANEL)], &[(DP, FHD)], false),
            Replan {
                heads: vec![HeadAction::Keep],
                add: vec![(DP, FHD)],
            }
        );
    }

    #[test]
    fn unplugging_a_secondary_monitor_removes_only_its_output() {
        assert_eq!(
            replan(&[alive(EDP, PANEL), gone(DP, FHD)], &[], false),
            Replan {
                heads: vec![HeadAction::Keep, HeadAction::Remove],
                add: vec![],
            }
        );
    }

    #[test]
    fn unplugging_the_primary_with_a_secondary_lit_removes_the_primary() {
        // The secondary becomes the primary; nothing is moved.
        assert_eq!(
            replan(&[gone(DP, FHD), alive(EDP, PANEL)], &[], false),
            Replan {
                heads: vec![HeadAction::Remove, HeadAction::Keep],
                add: vec![],
            }
        );
    }

    #[test]
    fn the_only_screen_unplugged_with_another_connected_moves_onto_it() {
        // Issue #48's fallback, unchanged for one head: the session follows
        // the display rather than going black.
        assert_eq!(
            replan(&[gone(HDMI, FHD)], &[(EDP, PANEL)], false),
            Replan {
                heads: vec![HeadAction::MoveTo(EDP, PANEL)],
                add: vec![],
            }
        );
    }

    #[test]
    fn every_screen_unplugged_with_one_connected_keeps_the_primary_and_moves_it() {
        // Both driven screens went away at once (a dock unplugged) while a
        // third connector is connected: the primary moves, the other head is
        // removed, and nothing extra is added.
        assert_eq!(
            replan(&[gone(DP, FHD), gone(HDMI, FHD)], &[(EDP, PANEL)], false),
            Replan {
                heads: vec![HeadAction::MoveTo(EDP, PANEL), HeadAction::Remove],
                add: vec![],
            }
        );
    }

    #[test]
    fn a_fallback_with_several_connectors_moves_to_the_first_and_adds_the_rest() {
        assert_eq!(
            replan(&[gone(HDMI, FHD)], &[(EDP, PANEL), (DP, FHD)], false),
            Replan {
                heads: vec![HeadAction::MoveTo(EDP, PANEL)],
                add: vec![(DP, FHD)],
            }
        );
    }

    #[test]
    fn nothing_connected_holds_the_primary_and_never_removes_it() {
        // The last screen is never taken away: the session keeps its output
        // and waits for a display to come back.
        assert_eq!(
            replan(&[gone(EDP, PANEL), gone(DP, FHD)], &[], false),
            Replan {
                heads: vec![HeadAction::Hold, HeadAction::Remove],
                add: vec![],
            }
        );
        assert_eq!(
            replan(&[gone(EDP, PANEL)], &[], true),
            Replan {
                heads: vec![HeadAction::Hold],
                add: vec![],
            }
        );
    }

    #[test]
    fn the_held_primary_coming_back_at_its_size_forces_a_modeset() {
        assert_eq!(
            replan(&[alive(EDP, PANEL)], &[], true),
            Replan {
                heads: vec![HeadAction::Reconnected],
                add: vec![],
            }
        );
    }

    #[test]
    fn a_new_mode_on_any_head_is_followed_on_that_head_only() {
        let resized = Probed {
            connector: DP,
            size: FHD,
            now: Some((1280, 720)),
        };
        assert_eq!(
            replan(&[alive(EDP, PANEL), resized], &[], false),
            Replan {
                heads: vec![HeadAction::Keep, HeadAction::NewMode((1280, 720))],
                add: vec![],
            }
        );
    }

    #[test]
    fn an_uninteresting_uevent_changes_nothing() {
        assert_eq!(
            replan(&[alive(EDP, PANEL), alive(DP, FHD)], &[], false),
            Replan {
                heads: vec![HeadAction::Keep, HeadAction::Keep],
                add: vec![],
            }
        );
    }

    #[test]
    fn holding_only_matters_to_the_primary() {
        // A secondary at an unchanged size is never "reconnected".
        assert_eq!(
            replan(&[alive(EDP, PANEL), alive(DP, FHD)], &[], true),
            Replan {
                heads: vec![HeadAction::Reconnected, HeadAction::Keep],
                add: vec![],
            }
        );
    }

    #[test]
    fn no_heads_is_a_plan_that_does_nothing() {
        assert_eq!(
            replan::<u32>(&[], &[(DP, FHD)], false),
            Replan {
                heads: vec![],
                add: vec![],
            }
        );
    }
}
