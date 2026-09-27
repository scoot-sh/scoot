//! Frame callbacks for layer surfaces nobody can see.
//!
//! [`withhold_frame`](crate::compositor::layer_shell::occlusion::withhold_frame)
//! lets a covered animated wallpaper stop decoding frames nobody shows,
//! without ever losing a callback: a skipped one stays queued and completes
//! on the first frame that serves the surface again. Each test below pins
//! one hard constraint of
//! `docs/backlog/core/frame-callbacks-for-hidden-surfaces.md`, with the
//! visible-served control inline so "no `done` arrived" can only pass
//! because the callback was withheld, never because `done` is broken.
//!
//! The cover in every test is a real fullscreen window, drawn the way the
//! ticket means: opaque-format (`Xrgb8888`, no opaque region) or
//! alpha-format with a declared opaque region. The wallpaper is a real
//! `background`-layer surface the whole output large.

use super::*;

/// A fullscreen `Xrgb8888` window over a mapped wallpaper: the ticket's
/// motivating case.
fn covered_xrgb(fixture: &mut Fixture) {
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque: true }));
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::DrawXrgb { window: 0 });
}

/// Constraint 1, first-attach unstuck: a callback requested before the
/// surface's first buffer commit is served even under a full opaque cover
/// -- withholding it would stall the frame that unsticks it. Only once the
/// surface has committed a buffer may the cover withhold.
#[test]
fn a_callback_requested_before_the_first_buffer_is_served_under_cover() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::DrawXrgb { window: 0 });
    // Covered, but nothing committed yet: served.
    fixture.done(Step::CreateLayerDeferred(Layer::Wallpaper {
        opaque: false,
    }));
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1],
        "a pre-attach callback is served even under a full cover"
    );
    // The same surface with a buffer committed is a candidate now: the next
    // callback is withheld.
    fixture.done(Step::DrawLayer {
        index: 0,
        color: OTHER_BGRA,
        opaque: false,
    });
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1, 0],
        "once it has a buffer the cover withholds"
    );
}

/// Constraint 2, opaque-format half: an `Xrgb8888` fullscreen window with no
/// opaque region covers the wallpaper, so its callback is withheld -- after
/// the tiled control proves `done` works.
#[test]
fn an_opaque_format_cover_withholds_the_wallpaper() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque: true }));
    // Control: tiled, the wallpaper is visible, the callback completes.
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1],
        "a visible wallpaper is served"
    );
    // The cover goes up without a new request outstanding; the next one is
    // withheld.
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::DrawXrgb { window: 0 });
    let pixels = fixture.render();
    assert_eq!(
        pixel(&pixels, CANVAS / 2, CANVAS / 2),
        WINDOW_BGRA,
        "the window really covers the output"
    );
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1, 0],
        "a fully covered wallpaper gets no callback"
    );
}

/// Constraint 2, opaque-region half: an alpha-format fullscreen window
/// declaring itself opaque covers the wallpaper too.
#[test]
fn an_opaque_region_cover_withholds_the_wallpaper() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque: false }));
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1],
        "a visible wallpaper is served"
    );
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::SetOpaque { window: 0 });
    // Smithay applies an opaque region when the buffer or its view next
    // changes, so the declaration alone covers nothing until a draw.
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1, 0],
        "an opaque-region cover withholds too"
    );
}

/// Partial coverage is served: a tiled window leaves most of the output to
/// the wallpaper, so every callback completes.
#[test]
fn a_tiled_window_leaves_the_wallpaper_served() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::CreateLayer(Layer::Wallpaper { opaque: true }));
    fixture.request_layer_frame(0);
    fixture.render();
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1, 1],
        "a partially covered wallpaper is served every frame"
    );
}

/// Constraint 3, resume: the withheld callback is never lost -- the first
/// frame where any of the surface is visible again completes the very
/// callback the cover held back.
#[test]
fn the_callback_resumes_when_the_cover_lifts() {
    let mut fixture = Fixture::new();
    covered_xrgb(&mut fixture);
    fixture.request_layer_frame(0);
    fixture.render();
    assert_eq!(fixture.layer_frames(), vec![0], "covered: nothing arrives");
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1],
        "the held-back callback completes on the first visible frame"
    );
}

/// Never lost across withhold windows: several covered frames in a row
/// complete nothing, and uncovering completes the one pending callback
/// exactly once.
#[test]
fn a_withheld_callback_survives_repeated_covered_frames() {
    let mut fixture = Fixture::new();
    covered_xrgb(&mut fixture);
    fixture.request_layer_frame(0);
    fixture.render();
    fixture.render();
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![0],
        "no covered frame completes the callback"
    );
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1],
        "uncovering completes the pending callback exactly once"
    );
}

/// The layer-depth rule: a `top` bar hidden under the fullscreen cover is
/// withheld like the wallpaper, while an `overlay` notification above that
/// same cover is still visible and still served.
#[test]
fn a_hidden_bar_is_withheld_but_a_notification_is_served() {
    let mut fixture = Fixture::new();
    fixture.map(WINDOW_BGRA);
    fixture.done(Step::CreateLayer(Layer::Bar));
    fixture.done(Step::CreateLayer(Layer::Notification));
    fixture.configured(Step::SetFullscreen {
        window: 0,
        output: None,
    });
    fixture.done(Step::DrawXrgb { window: 0 });
    fixture.request_layer_frame(0);
    fixture.request_layer_frame(1);
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![0, 1],
        "the hidden bar waits, the notification over the cover is served"
    );
    fixture.configured(Step::UnsetFullscreen { window: 0 });
    fixture.done(Step::Draw {
        window: 0,
        color: WINDOW_BGRA,
    });
    fixture.render();
    assert_eq!(
        fixture.layer_frames(),
        vec![1, 1],
        "the bar resumes with the cover gone"
    );
}
