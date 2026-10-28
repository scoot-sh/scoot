//! Pins for the CPU swap's binding tail (see the module doc for the strand
//! these guard). The fd and wiring half needs a real seat and DRM master;
//! these pin the decisions the wiring feeds.

use super::*;

/// Empty bindings after an empty restore, one fresh head: the head is
/// routed to a new output, never dropped. The old tail dropped it.
#[test]
fn a_fresh_head_no_binding_claims_is_added_not_dropped() {
    assert_eq!(unbound_heads(&[false]), vec![0]);
}

#[test]
fn only_the_heads_without_an_output_are_added() {
    assert_eq!(unbound_heads(&[true, false, true, false]), vec![1, 3]);
    assert!(unbound_heads(&[true, true]).is_empty());
    assert!(unbound_heads(&[]).is_empty());
}

/// Zero-output edge: a proven swap that built no head at all is dark and
/// holds the primary, so `live` is false and the next probe retries.
#[test]
fn a_proven_swap_with_no_heads_is_dark_and_holds() {
    let (live, active, hold) = verdict(SwapLive::Live, &[]);
    assert!(matches!(live, SwapLive::Dark { .. }));
    assert!(!active, "no head presents, so the device must not be live");
    assert!(
        hold,
        "a headless swap holds the primary like the empty restore"
    );
}

/// One-output edge, the strand itself: a head exists but never reached an
/// output (output creation refused). Not live; no hold, the re-probe retries.
#[test]
fn a_proven_swap_whose_only_head_found_no_output_is_dark() {
    let (live, active, hold) = verdict(SwapLive::Live, &[false]);
    assert!(matches!(live, SwapLive::Dark { .. }));
    assert!(!active);
    assert!(!hold, "a head that merely missed its output claims no hold");
}

/// One-output edge, the healthy case: the head presents, the swap is live.
#[test]
fn a_proven_swap_with_a_presenting_head_stays_live() {
    let (live, active, hold) = verdict(SwapLive::Live, &[true]);
    assert!(matches!(live, SwapLive::Live));
    assert!(active);
    assert!(!hold);
}

/// One presenting head among unpresenting ones is enough to be live.
#[test]
fn any_presenting_head_keeps_the_swap_live() {
    let (live, active, _) = verdict(SwapLive::Live, &[false, true]);
    assert!(matches!(live, SwapLive::Live));
    assert!(active);
}

/// A failed proof stays dark whatever is bound.
#[test]
fn a_failed_proof_stays_dark_even_with_presenting_heads() {
    let (live, active, _) = verdict(
        SwapLive::Dark {
            proof: "refused".to_owned(),
        },
        &[true],
    );
    assert!(matches!(live, SwapLive::Dark { .. }));
    assert!(!active);
}
