use std::sync::Arc;

use super::{Bitmap, Cache};
use crate::icon::path::{Vector, ViewBox};
use crate::icon::{Art, MAX_ARENA, MAX_ENTRIES, MAX_SIDE};

fn square() -> Art {
    Art::Vector(Arc::new(
        Vector::parse("M0 0H24V24H0z", ViewBox::default()).unwrap(),
    ))
}

#[test]
fn a_second_ask_at_the_same_size_is_a_hit_and_allocates_nothing() {
    let art = square();
    let mut cache = Cache::default();
    let Some(Bitmap::Mask(first)) = cache.get(&art, 20) else {
        panic!("no bitmap")
    };
    assert_eq!(first.len(), 400);
    assert!(first.iter().all(|&c| c >= 254));
    assert_eq!(cache.cached(), (1, 400));
    let (bitmap, allocations) = scootbg_mem::count_allocations(|| {
        cache.get(&art, 20).map(|b| matches!(b, Bitmap::Mask(_)))
    });
    assert_eq!(bitmap, Some(true));
    assert_eq!(allocations, 0, "a hit allocated");
    assert_eq!(cache.cached(), (1, 400));
}

#[test]
fn each_size_and_each_icon_is_its_own_entry() {
    let (a, b) = (square(), square());
    let mut cache = Cache::default();
    for (art, side) in [(&a, 16), (&a, 24), (&b, 16)] {
        assert!(cache.get(art, side).is_some());
    }
    assert_eq!(cache.cached(), (3, 256 + 576 + 256));
}

#[test]
fn a_size_of_zero_or_past_the_bound_draws_nothing() {
    let art = square();
    let mut cache = Cache::default();
    assert!(cache.get(&art, 0).is_none());
    assert!(cache.get(&art, MAX_SIDE + 1).is_none());
    assert!(cache.get(&art, u32::MAX).is_none());
    assert_eq!(cache.cached(), (0, 0));
    assert!(cache.get(&art, MAX_SIDE).is_some());
}

#[test]
fn the_cache_is_bounded_under_a_stream_of_sizes() {
    // Every size from 1 to 512 twice over, and a few hundred icons at one
    // size: neither bound is ever passed.
    let mut cache = Cache::default();
    let art = square();
    for round in 0..2 {
        for side in 1..=MAX_SIDE {
            let side = if round == 0 {
                side
            } else {
                MAX_SIDE + 1 - side
            };
            assert!(cache.get(&art, side).is_some());
            let (entries, bytes) = cache.cached();
            assert!(entries <= MAX_ENTRIES, "{entries} entries");
            assert!(bytes <= MAX_ARENA, "{bytes} bytes");
        }
    }
    for _ in 0..300 {
        assert!(cache.get(&square(), 24).is_some());
        let (entries, bytes) = cache.cached();
        assert!(entries <= MAX_ENTRIES && bytes <= MAX_ARENA);
    }
}

#[test]
fn a_dropped_cache_refills_with_the_same_picture() {
    let art = square();
    let mut cache = Cache::default();
    let first = match cache.get(&art, 10) {
        Some(Bitmap::Mask(pixels)) => pixels.to_vec(),
        _ => panic!("no bitmap"),
    };
    // Pass the entry bound with other sizes, so size 10 was dropped.
    for side in 11..11 + MAX_ENTRIES as u32 + 1 {
        assert!(cache.get(&art, side).is_some());
    }
    match cache.get(&art, 10) {
        Some(Bitmap::Mask(pixels)) => assert_eq!(pixels, first.as_slice()),
        _ => panic!("no bitmap"),
    }
}
