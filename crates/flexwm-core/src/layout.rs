//! Pure layout arithmetic: spans, gaps and minimums. Nothing here knows about
//! windows, outputs or platforms.

/// Width of a column taking `proportion` of `available`, sized so columns whose
/// proportions sum to 1.0 fill it exactly, gaps included. Never below `min`,
/// and never below 1.
pub fn column_width(available: i32, gap: i32, proportion: f64, min: i32) -> i32 {
    let width = (f64::from(available + gap) * proportion).round() as i32 - gap;
    width.max(min).max(1)
}

/// The start of each span laid end to end with `gap` between them, and the
/// total length of that strip. Saturates rather than overflowing.
pub fn starts(widths: &[i32], gap: i32) -> (Vec<i32>, i32) {
    let mut starts = Vec::with_capacity(widths.len());
    let mut x: i32 = 0;
    for (i, &width) in widths.iter().enumerate() {
        if i > 0 {
            x = x.saturating_add(gap);
        }
        starts.push(x);
        x = x.saturating_add(width);
    }
    (starts, x)
}

/// The scroll offset that brings `[start, start + len)` into a viewport of
/// `view_len` currently at `view`, moving as little as possible and never past
/// either end of a strip of `strip_len`.
pub fn scroll_into_view(view: i32, view_len: i32, start: i32, len: i32, strip_len: i32) -> i32 {
    let wanted = if start < view || len >= view_len {
        start
    } else if start + len > view + view_len {
        start + len - view_len
    } else {
        view
    };
    wanted.clamp(0, (strip_len - view_len).max(0))
}

/// Splits `total` into one span per entry of `mins`, as evenly as possible.
/// Every span gets at least its minimum and at least 1; a minimum is capped so
/// the other spans can keep 1 each. If the capped minimums still exceed
/// `total`, each span gets exactly its minimum and the result overflows.
pub fn distribute(total: i32, mins: &[i32]) -> Vec<i32> {
    let cap = (total - (mins.len() as i32 - 1)).max(1);
    let mins: Vec<i32> = mins.iter().map(|&min| min.clamp(1, cap)).collect();
    let mut fixed = vec![false; mins.len()];
    loop {
        let free: Vec<usize> = (0..mins.len()).filter(|&i| !fixed[i]).collect();
        if free.is_empty() {
            return mins;
        }
        let taken: i32 = mins
            .iter()
            .zip(&fixed)
            .filter(|(_, f)| **f)
            .map(|(m, _)| m)
            .sum();
        let available = (total - taken).max(0);
        let share = available / free.len() as i32;
        let too_small: Vec<usize> = free.iter().copied().filter(|&i| mins[i] > share).collect();
        if too_small.is_empty() {
            let mut spare = available - share * free.len() as i32;
            let mut sizes = mins.clone();
            for i in free {
                sizes[i] = share + i32::from(spare > 0);
                spare -= 1;
            }
            return sizes;
        }
        for i in too_small {
            fixed[i] = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_fill_the_space_exactly() {
        let width = column_width(980, 10, 0.5, 0);
        assert_eq!(width + 10 + width, 980);
    }

    #[test]
    fn column_width_respects_minimum() {
        assert_eq!(column_width(980, 10, 0.5, 600), 600);
    }

    #[test]
    fn distribute_evenly_with_remainder() {
        assert_eq!(distribute(100, &[0, 0, 0]), vec![34, 33, 33]);
    }

    #[test]
    fn distribute_honours_minimums() {
        assert_eq!(distribute(100, &[60, 0]), vec![60, 40]);
        assert_eq!(distribute(100, &[0, 70, 0]), vec![15, 70, 15]);
    }

    #[test]
    fn distribute_overflows_when_minimums_do_not_fit() {
        assert_eq!(distribute(100, &[80, 80]), vec![80, 80]);
    }

    #[test]
    fn distribute_never_hands_out_zero() {
        assert_eq!(distribute(570, &[580, 0]), vec![569, 1]);
        assert_eq!(distribute(2, &[0, 0, 0]), vec![1, 1, 1]);
    }

    #[test]
    fn starts_saturate_instead_of_overflowing() {
        let (_, strip) = starts(&[i32::MAX, i32::MAX], 10);
        assert_eq!(strip, i32::MAX);
    }

    #[test]
    fn scroll_moves_the_least_needed() {
        // Already visible: stay put.
        assert_eq!(scroll_into_view(0, 100, 10, 50, 300), 0);
        // Past the right edge: line up the right edges.
        assert_eq!(scroll_into_view(0, 100, 150, 50, 300), 100);
        // Past the left edge: line up the left edges.
        assert_eq!(scroll_into_view(100, 100, 20, 50, 300), 20);
        // Never scroll beyond the end of the strip.
        assert_eq!(scroll_into_view(250, 100, 0, 50, 80), 0);
    }
}
