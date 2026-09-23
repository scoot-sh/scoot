//! Subsurface nesting depth, driven through a real `wayland-client`
//! connection: how deep a tree may nest, and every way a client could build
//! one deeper than that -- bottom-up, by re-attaching, through a destroyed
//! parent -- without any single link being deep.
//!
//! The client is `popup_parent`'s (see its `subsurfaces` module), because
//! one test needs popups and subsurfaces in the same tree. As there, every
//! test asserts on what the client was told, on real framebuffer pixels, or
//! on a second client still being served after the first did its worst.
//!
//! Nothing here names a server-side symbol from `subsurface_depth.rs` (the
//! cap is spelled out as [`CAP`]), so this file compiles against the code
//! before it, which is how its tests were watched failing first -- by
//! crashing the test process, mostly: a stack overflow aborts it.

use crate::compositor::popup_parent::tests::{
    CANVAS, Fixture, MARKED_BGRA, Node, Op, STEP, SUB_SIZE, SubOp, pixel,
};

mod bench;
mod bypass;
mod depth;
mod popup;

/// The most levels of subsurface below a tree's root --
/// `subsurface_depth::MAX_SUBSURFACE_DEPTH`, spelled out so this file does
/// not depend on it (see the module doc).
const CAP: usize = 64;

/// A desynchronized chain of `len` drawn subsurfaces under `parent`, the
/// deepest in [`MARKED_BGRA`].
fn chain(parent: Node, len: usize) -> Op {
    Op::Sub(SubOp::Chain {
        parent,
        len,
        sync: false,
        draw: true,
    })
}

/// Runs `ops` in one flush and draws a frame whatever came of it -- a tree
/// too deep for the stack took the compositor down there before the cap --
/// then asserts that the client was cut off, and hands back its error.
fn attack(fixture: &mut Fixture, ops: Vec<Op>) -> String {
    fixture
        .attacked(ops)
        .expect_err("the client survived a tree past the cap")
}

/// Asserts that `error` is the depth refusal: `bad_parent`, on the
/// `wl_subcompositor`.
fn assert_too_deep(error: &str) {
    assert!(error.contains("wl_subcompositor@"), "{error}");
    assert!(error.contains("bad_parent"), "{error}");
}

/// Asserts that the surface `levels` steps in from `(x, y)` -- the deepest
/// of a drawn chain whose root's parent sits at `(x, y)` -- is on top, in
/// [`MARKED_BGRA`].
fn assert_drawn_at(pixels: &[u8], (x, y): (i32, i32), levels: usize) {
    let levels = i32::try_from(levels).expect("a small depth");
    let corner = (x + STEP * levels, y + STEP * levels);
    let centre = (corner.0 + SUB_SIZE / 2, corner.1 + SUB_SIZE / 2);
    assert!(
        centre.0 < CANVAS && centre.1 < CANVAS,
        "the deepest surface is off screen"
    );
    assert_eq!(pixel(pixels, centre), MARKED_BGRA);
}

/// Window `index`'s top-left corner.
fn window_corner(fixture: &Fixture, index: usize) -> (i32, i32) {
    let rect = fixture.rect_of(index);
    (rect.x, rect.y)
}
