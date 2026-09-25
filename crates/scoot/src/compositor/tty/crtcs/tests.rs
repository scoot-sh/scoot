//! The CRTC matching, over plain integers standing in for CRTC handles.

use super::assign;

#[test]
fn one_connector_takes_its_first_reachable_crtc() {
    // The single-output shape every `--tty` session had before multi-output:
    // the first CRTC the connector can use, exactly the one the old
    // first-that-takes-a-surface loop landed on when CRTC 0 was reachable.
    assert_eq!(assign(&[vec![0, 1, 2]], &[]), vec![Some(0)]);
}

#[test]
fn fixed_wiring_gives_each_connector_its_own_crtc() {
    // Apple's DCP, as `drm_info` reports it on the M2 Air: `eDP-1`'s encoder
    // reaches CRTC 50 only and `DP-1`'s reaches CRTC 68 only.
    assert_eq!(assign(&[vec![50], vec![68]], &[]), vec![Some(50), Some(68)]);
}

#[test]
fn a_flexible_connector_moves_aside_for_a_constrained_one() {
    // The case a greedy pass gets wrong: the first connector could use
    // either CRTC and took 0, which is the only one the second can use.
    assert_eq!(assign(&[vec![0, 1], vec![0]], &[]), vec![Some(1), Some(0)]);
}

#[test]
fn more_connectors_than_crtcs_drives_the_earliest_ones() {
    // Kernel order decides who stays dark, like it always decided which
    // single connector was driven.
    assert_eq!(
        assign(&[vec![0, 1], vec![0, 1], vec![0, 1]], &[]),
        vec![Some(0), Some(1), None]
    );
}

#[test]
fn an_earlier_connector_is_never_unmatched_for_a_later_one() {
    // Re-routing is allowed, dropping is not: connector 0 can only use CRTC
    // 0, so connector 1 (which also wants only 0) stays dark rather than
    // taking it.
    assert_eq!(assign(&[vec![0], vec![0]], &[]), vec![Some(0), None]);
}

#[test]
fn busy_crtcs_are_never_handed_out() {
    // A connector plugged in while others are lit gets what is free, and
    // nothing lit is re-routed to make room.
    assert_eq!(assign(&[vec![0, 1]], &[0]), vec![Some(1)]);
    assert_eq!(assign(&[vec![0]], &[0]), vec![None]);
}

#[test]
fn a_connector_with_no_reachable_crtc_is_left_dark() {
    // An encoder whose mask reads empty (or could not be read at all).
    assert_eq!(assign(&[vec![], vec![0]], &[]), vec![None, Some(0)]);
}

#[test]
fn no_connectors_is_no_assignments() {
    assert_eq!(assign::<u32>(&[], &[]), Vec::<Option<u32>>::new());
}

#[test]
fn a_long_augmenting_chain_rerouting_several_connectors() {
    // Each connector can use its own CRTC or the next one; the last can only
    // use the first connector's preferred CRTC. Everyone has to shift once.
    let possible = vec![vec![0, 1], vec![1, 2], vec![2, 3], vec![0]];
    assert_eq!(
        assign(&possible, &[]),
        vec![Some(1), Some(2), Some(3), Some(0)]
    );
}
