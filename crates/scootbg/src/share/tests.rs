use std::cell::Cell;
use std::rc::Rc;

use super::{Shared, Slot};

/// Pages that count their own drops, standing in for the memfd mapping
/// (whose drop unmaps it and destroys its pool).
struct Pages<'a> {
    pixel: u8,
    dropped: &'a Cell<u32>,
}

impl Drop for Pages<'_> {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
    }
}

fn pages(dropped: &Cell<u32>) -> Rc<Shared<Pages<'_>>> {
    Shared::new(Pages { pixel: 0, dropped })
}

/// One output, as before sharing: writable until attached, and again once
/// released.
#[test]
fn one_output_writes_only_while_its_buffer_is_released() {
    let dropped = Cell::new(0);
    let mut slot = Slot::new("a", pages(&dropped));
    assert!(slot.is_free() && slot.is_writable());
    slot.memory_mut().unwrap().pixel = 1;
    slot.attached();
    assert!(!slot.is_free() && !slot.is_writable());
    assert!(slot.memory_mut().is_none());
    slot.released();
    assert!(slot.is_free() && slot.is_writable());
    slot.memory_mut().unwrap().pixel = 2;
    assert_eq!(slot.memory().pixel, 2);
    assert_eq!(slot.retire(), "a");
    assert_eq!(dropped.get(), 1, "the last share frees the pages");
}

/// Two outputs of one size showing one image: neither may write while the
/// other shows the pages, whatever each one's own buffer is doing.
#[test]
fn shared_pages_are_never_writable_while_another_output_shows_them() {
    let dropped = Cell::new(0);
    let shared = pages(&dropped);
    let mut a = Slot::new("a", Rc::clone(&shared));
    let mut b = Slot::new("b", shared);
    a.attached();
    b.attached();
    for (a_released, b_released) in [(false, false), (true, false), (false, true), (true, true)] {
        if a_released {
            a.released();
        } else {
            a.attached();
        }
        if b_released {
            b.released();
        } else {
            b.attached();
        }
        assert_eq!(a.is_free(), a_released);
        assert_eq!(b.is_free(), b_released);
        assert!(!a.is_writable() && a.memory_mut().is_none());
        assert!(!b.is_writable() && b.memory_mut().is_none());
    }
    assert_eq!(dropped.get(), 0);
}

/// "A buffer is free only when every surface it's attached to has released
/// it, or has moved on": B moves on (released, then dropped), and A may
/// write once its own buffer is released too.
#[test]
fn the_pages_become_writable_once_every_other_output_released_and_moved_on() {
    let dropped = Cell::new(0);
    let shared = pages(&dropped);
    let mut a = Slot::new("a", Rc::clone(&shared));
    let mut b = Slot::new("b", shared);
    a.attached();
    b.attached();
    // B's compositor releases it after B showed something else.
    b.released();
    assert!(!a.is_writable(), "B still has them");
    assert_eq!(b.retire(), "b");
    assert!(!a.is_writable(), "A's own buffer is still held");
    assert_eq!(dropped.get(), 0, "A still shows them");
    a.released();
    assert!(a.is_writable());
    a.memory_mut().unwrap().pixel = 9;
    let _ = a.retire();
    assert_eq!(dropped.get(), 1);
}

/// An output unplugged (or cleared) while the compositor held its buffer:
/// the compositor may still be reading those pages, so the output still
/// showing them never writes them, even once they are its alone.
#[test]
fn pages_dropped_under_a_held_buffer_are_frozen_for_good() {
    let dropped = Cell::new(0);
    let shared = pages(&dropped);
    let mut a = Slot::new("a", Rc::clone(&shared));
    let mut b = Slot::new("b", shared);
    a.attached();
    b.attached();
    let _ = b.retire();
    assert!(a.shared().is_frozen());
    a.released();
    assert!(a.is_free(), "A's own buffer may still be dropped");
    assert!(!a.is_writable());
    assert!(a.memory_mut().is_none());
    // Attached again (shown as it is) and released: still frozen.
    a.attached();
    a.released();
    assert!(a.memory_mut().is_none());
    let _ = a.retire();
    assert_eq!(dropped.get(), 1, "freed with the last share all the same");
}

/// A slot dropped after its release freezes nothing.
#[test]
fn a_released_slot_dropped_leaves_the_pages_writable() {
    let dropped = Cell::new(0);
    let shared = pages(&dropped);
    let mut a = Slot::new("a", Rc::clone(&shared));
    let mut b = Slot::new("b", shared);
    b.attached();
    b.released();
    let _ = b.retire();
    assert!(!a.shared().is_frozen());
    assert!(a.is_writable());
    // Never attached at all, dropped: nothing frozen either.
    let c = Slot::new("c", Rc::clone(a.shared()));
    assert!(!a.is_writable());
    let _ = c.retire();
    assert!(a.memory_mut().is_some());
}

/// An image rendered for an output and waiting to go on screen there holds
/// the pages as surely as a buffer does.
#[test]
fn a_rendered_image_waiting_blocks_writes() {
    let dropped = Cell::new(0);
    let mut a = Slot::new("a", pages(&dropped));
    let waiting = Rc::clone(a.shared());
    assert!(a.is_free() && !a.is_writable());
    assert!(a.memory_mut().is_none());
    drop(waiting);
    assert!(a.memory_mut().is_some());
    let _ = a.retire();
    assert_eq!(dropped.get(), 1);
}

/// Attached twice before a release (the same buffer committed again for a
/// new buffer scale): one release frees it, as Smithay sends only one.
#[test]
fn attaching_a_held_buffer_again_needs_one_release() {
    let dropped = Cell::new(0);
    let mut a = Slot::new("a", pages(&dropped));
    a.attached();
    a.attached();
    a.released();
    assert!(a.is_writable());
    // A release with nothing held (a stray, or one for a buffer attached
    // once and released) changes nothing.
    a.released();
    assert!(a.is_writable());
}

/// The pages outlive every slot until the last share goes, in any order.
#[test]
fn the_pages_go_with_the_last_share_in_any_order() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let dropped = Cell::new(0);
        let shared = pages(&dropped);
        let mut slots: Vec<Option<Slot<usize, Pages<'_>>>> = (0..3)
            .map(|i| Some(Slot::new(i, Rc::clone(&shared))))
            .collect();
        drop(shared);
        for (n, &i) in order.iter().enumerate() {
            let slot = slots[i].take().unwrap();
            assert_eq!(slot.retire(), i);
            let want = u32::from(n == 2);
            assert_eq!(dropped.get(), want, "{order:?} after {n}");
        }
    }
}
