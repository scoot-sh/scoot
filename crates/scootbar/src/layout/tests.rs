use super::{Layout, Section, arrange};
use crate::paint::Span;

use Section::{Center as C, Left as L, Right as R};

fn spans(sections: &[Section], widths: &[u32], spacing: u32, bar: u32) -> Vec<(u32, u32)> {
    let mut out = vec![Span::default(); sections.len()];
    arrange(sections, widths, spacing, bar, &mut out);
    out.iter().map(|s| (s.x, s.width)).collect()
}

#[test]
fn sections_pack_left_center_and_right() {
    assert_eq!(
        spans(&[L, L, C, R, R], &[10, 20, 30, 40, 50], 5, 1000),
        [(0, 10), (15, 20), (485, 30), (905, 40), (950, 50)]
    );
    // One module in the center: exactly centered (rounded down).
    assert_eq!(spans(&[C], &[101], 0, 1000), [(449, 101)]);
    assert_eq!(
        spans(&[C, C], &[100, 100], 10, 1000),
        [(395, 100), (505, 100)]
    );
}

#[test]
fn an_empty_module_takes_no_space_and_no_spacing() {
    assert_eq!(
        spans(&[L, L, L], &[10, 0, 10], 5, 100),
        [(0, 10), (15, 0), (15, 10)]
    );
    assert_eq!(spans(&[C, C], &[0, 50], 10, 100), [(25, 0), (25, 50)]);
    assert_eq!(spans(&[], &[], 5, 100), []);
}

#[test]
fn the_center_moves_aside_for_a_long_left_side() {
    // The left takes 600 of 1000: the center (200) would start at 400 and
    // overlap it, so it starts at 600 instead.
    assert_eq!(
        spans(&[L, C, R], &[600, 200, 100], 0, 1000),
        [(0, 600), (600, 200), (900, 100)]
    );
    // Too little room: the center is cut at the right side.
    assert_eq!(
        spans(&[L, C, R], &[600, 400, 100], 0, 1000),
        [(0, 600), (600, 300), (900, 100)]
    );
}

#[test]
fn nothing_overlaps_and_nothing_passes_the_edge() {
    // The left alone wider than the bar: cut at the edge, and everything
    // else gets nothing.
    assert_eq!(
        spans(&[L, C, R], &[1500, 200, 100], 0, 1000),
        [(0, 1000), (1000, 0), (1000, 0)]
    );
    // Absurd widths and spacing saturate rather than overflow.
    let out = spans(&[L, L, C, R, R], &[u32::MAX; 5], u32::MAX, u32::MAX);
    let mut end = 0;
    for (x, width) in out {
        assert!(x >= end || width == 0, "overlap at {x}");
        end = end.max(x + width);
    }
    // A bar of width 1, or 0.
    assert_eq!(spans(&[C], &[50], 0, 1), [(0, 1)]);
    assert_eq!(
        spans(&[L, C, R], &[5, 5, 5], 0, 0),
        [(0, 0), (0, 0), (0, 0)]
    );
}

/// Every layout of a few modules at any bar width keeps the rules: in the
/// bar, in order, no overlap.
#[test]
fn every_small_layout_keeps_the_rules() {
    let sections = [L, L, C, C, R, R];
    for bar in [0u32, 1, 7, 50, 99, 100, 101, 1000] {
        for mask in 0u32..(1 << 6) {
            let widths: Vec<u32> = (0..6)
                .map(|i| if mask >> i & 1 == 1 { 7 + i * 13 } else { 0 })
                .collect();
            let out = spans(&sections, &widths, 3, bar);
            let mut end = 0;
            for (&(x, width), &wanted) in out.iter().zip(&widths) {
                assert!(x + width <= bar, "past the bar: {bar} {widths:?} {out:?}");
                assert!(width <= wanted);
                if width > 0 {
                    assert!(x >= end, "overlap: {bar} {widths:?} {out:?}");
                    end = x + width;
                }
            }
        }
    }
}

#[test]
fn placed_lists_left_to_right() {
    let layout = Layout {
        left: vec!["a", "b"],
        center: vec!["c"],
        right: vec!["d"],
        padding: 0,
        spacing: 0,
    };
    let placed: Vec<_> = layout.placed().collect();
    assert_eq!(placed, [(L, "a"), (L, "b"), (C, "c"), (R, "d")]);
    assert!(!layout.is_empty());
    assert_eq!(Section::Left.flag(), "--left");
}
