//! [`driver_tranche`](super::super::driver_tranche), the GLES advertisement
//! rule, against synthetic import sets.
//!
//! Synthetic because the cases that matter most are the ones no machine this
//! project can reach produces: a display with no modifiers extension, a
//! driver that refuses the modifier query for some of its own formats
//! (NVIDIA >= 520, per upstream's comment), a driver that lists tiled
//! layouts and not `LINEAR`. Each is the shape Smithay's
//! `get_dmabuf_formats` would hand over (`egl/display.rs:880-1010` at the
//! pinned rev): explicit modifiers only where the driver answered, and
//! `{fourcc, Invalid}` for every fourcc regardless.

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};

use super::super::driver_tranche;

/// `I915_FORMAT_MOD_X_TILED` and `_Y_TILED`: stand-ins for "a real tiled
/// layout", nothing here depends on which.
const X_TILED: Modifier = Modifier::I915_x_tiled;
const Y_TILED: Modifier = Modifier::I915_y_tiled;

fn format(code: Fourcc, modifier: Modifier) -> Format {
    Format { code, modifier }
}

fn set(formats: &[(Fourcc, Modifier)]) -> FormatSet {
    formats
        .iter()
        .map(|&(code, modifier)| format(code, modifier))
        .collect()
}

#[test]
fn a_mesa_style_set_advertises_every_fourcc_at_its_explicit_modifiers_only() {
    // What the dev VM's llvmpipe reports (raw probe in the PR record): every
    // fourcc at `LINEAR` plus Smithay's unconditional `Invalid`, with the YUV
    // ones external-only at `LINEAR` -- which the texture set cannot show and
    // this rule does not need to, since external-only entries import (as
    // `GL_TEXTURE_EXTERNAL_OES`).
    let importable = set(&[
        (Fourcc::Abgr16161616f, Modifier::Linear),
        (Fourcc::Abgr16161616f, Modifier::Invalid),
        (Fourcc::Argb8888, Modifier::Linear),
        (Fourcc::Argb8888, Modifier::Invalid),
        (Fourcc::Xrgb8888, Modifier::Linear),
        (Fourcc::Xrgb8888, Modifier::Invalid),
        (Fourcc::Nv12, Modifier::Linear),
        (Fourcc::Nv12, Modifier::Invalid),
        (Fourcc::P010, Modifier::Linear),
        (Fourcc::P010, Modifier::Invalid),
    ]);
    assert_eq!(
        driver_tranche(&importable),
        vec![
            format(Fourcc::Xrgb8888, Modifier::Linear),
            format(Fourcc::Argb8888, Modifier::Linear),
            format(Fourcc::Abgr16161616f, Modifier::Linear),
            format(Fourcc::Nv12, Modifier::Linear),
            format(Fourcc::P010, Modifier::Linear),
        ],
        "every fourcc at its explicit modifier, the candidates first and in \
         candidate order, everything else in the driver's order -- and so the \
         old two-entry table is exactly the head of the new one"
    );
}

#[test]
fn implicit_is_never_advertised_where_the_driver_named_explicit_layouts() {
    // The YUV hazard: Smithay puts `{NV12, Invalid}` in the *render* set too,
    // so an implicit NV12 buffer would be bound as `GL_TEXTURE_2D` against the
    // driver's own "external only" answer for every explicit NV12 layout --
    // a wrong picture, not a refusal. Never offering `Invalid` next to an
    // explicit answer closes it for every fourcc, not just the YUV ones.
    let importable = set(&[
        (Fourcc::Nv12, Modifier::Linear),
        (Fourcc::Nv12, Modifier::Invalid),
        (Fourcc::Xrgb8888, X_TILED),
        (Fourcc::Xrgb8888, Modifier::Invalid),
    ]);
    let advertised = driver_tranche(&importable);
    assert!(
        advertised
            .iter()
            .all(|format| format.modifier != Modifier::Invalid),
        "no implicit-modifier entry may be advertised: {advertised:?}"
    );
}

#[test]
fn a_display_without_the_modifiers_extension_keeps_the_old_table() {
    // No `EGL_EXT_image_dma_buf_import_modifiers`: Smithay cannot enumerate
    // formats at all and guesses `[Argb8888, Xrgb8888]`, at `Invalid` only
    // (`egl/display.rs:896`). That display imports a linear buffer (see
    // `imports_linear`), so the table is what it was before this change:
    // both candidates at `LINEAR`, opaque first.
    let importable = set(&[
        (Fourcc::Argb8888, Modifier::Invalid),
        (Fourcc::Xrgb8888, Modifier::Invalid),
    ]);
    assert_eq!(
        driver_tranche(&importable),
        vec![
            format(Fourcc::Xrgb8888, Modifier::Linear),
            format(Fourcc::Argb8888, Modifier::Linear),
        ]
    );
}

#[test]
fn a_driver_that_refuses_the_modifier_query_loses_only_its_unvouched_formats() {
    // NVIDIA-style: the modifier query answered for some fourccs and refused
    // (count 0) for others. A refused candidate still gets the `LINEAR`
    // widening it always had; a refused non-candidate has nothing vouching
    // for any layout at all, and is left out rather than guessed at.
    let importable = set(&[
        (Fourcc::Xrgb8888, Modifier::Invalid),
        (Fourcc::Argb8888, Modifier::Linear),
        (Fourcc::Argb8888, X_TILED),
        (Fourcc::Argb8888, Modifier::Invalid),
        (Fourcc::Nv12, Modifier::Invalid),
        (Fourcc::Abgr8888, Y_TILED),
        (Fourcc::Abgr8888, Modifier::Invalid),
    ]);
    assert_eq!(
        driver_tranche(&importable),
        vec![
            format(Fourcc::Xrgb8888, Modifier::Linear),
            format(Fourcc::Argb8888, Modifier::Linear),
            format(Fourcc::Argb8888, X_TILED),
            format(Fourcc::Abgr8888, Y_TILED),
        ]
    );
}

#[test]
fn a_driver_that_lists_tiled_layouts_but_not_linear_is_not_offered_linear() {
    // The case the old candidate rule got wrong: `Invalid` counted as evidence
    // for `LINEAR` even when the driver had just named its layouts and
    // `LINEAR` was not one. A client allocating that `LINEAR` buffer is
    // imported *with* the modifier attribute (the extension is present), and
    // a driver that never listed `LINEAR` may refuse it -- through
    // `create_immed`, a kill. Its own tiled layouts are what it gets offered.
    let importable = set(&[
        (Fourcc::Xrgb8888, X_TILED),
        (Fourcc::Xrgb8888, Y_TILED),
        (Fourcc::Xrgb8888, Modifier::Invalid),
    ]);
    assert_eq!(
        driver_tranche(&importable),
        vec![
            format(Fourcc::Xrgb8888, X_TILED),
            format(Fourcc::Xrgb8888, Y_TILED),
        ]
    );
}

#[test]
fn a_renderer_that_imports_nothing_is_advertised_nothing() {
    // An EGL display with no dma-buf import extension: Smithay hands over an
    // empty set (`egl/display.rs:884-887`), and `advertise` then creates no
    // global rather than an empty table.
    assert!(driver_tranche(&FormatSet::default()).is_empty());
}

#[test]
fn each_fourcc_is_grouped_with_its_modifiers_in_the_drivers_order() {
    // The driver's set is ordered by insertion, and nothing requires one
    // fourcc's modifiers to be contiguous in it. The table groups them --
    // each fourcc once, its modifiers in the order the driver gave them.
    let importable = set(&[
        (Fourcc::Nv12, Y_TILED),
        (Fourcc::Xrgb8888, Modifier::Linear),
        (Fourcc::Nv12, Modifier::Linear),
        (Fourcc::Xrgb8888, X_TILED),
        (Fourcc::Nv12, Modifier::Invalid),
        (Fourcc::Xrgb8888, Modifier::Invalid),
    ]);
    assert_eq!(
        driver_tranche(&importable),
        vec![
            format(Fourcc::Xrgb8888, Modifier::Linear),
            format(Fourcc::Xrgb8888, X_TILED),
            format(Fourcc::Nv12, Y_TILED),
            format(Fourcc::Nv12, Modifier::Linear),
        ]
    );
}

#[test]
fn every_advertised_pair_is_the_drivers_own_or_the_documented_widening() {
    // The promise with teeth, as a property over every set above: an entry
    // is either literally in the driver's set, or a candidate at `LINEAR`
    // whose fourcc the driver gave no explicit answer for and did list at
    // `Invalid`. Nothing else may reach the wire.
    let cases = [
        set(&[
            (Fourcc::Xrgb8888, Modifier::Linear),
            (Fourcc::Xrgb8888, Modifier::Invalid),
            (Fourcc::Nv12, Modifier::Linear),
            (Fourcc::Nv12, Modifier::Invalid),
        ]),
        set(&[
            (Fourcc::Argb8888, Modifier::Invalid),
            (Fourcc::Xrgb8888, Modifier::Invalid),
            (Fourcc::R8, Modifier::Invalid),
        ]),
        set(&[
            (Fourcc::Xrgb8888, X_TILED),
            (Fourcc::Xrgb8888, Modifier::Invalid),
            (Fourcc::Argb8888, Modifier::Invalid),
        ]),
    ];
    for importable in cases {
        for entry in driver_tranche(&importable) {
            let widened = entry.modifier == Modifier::Linear
                && super::super::DMABUF_CANDIDATES.contains(&entry.code)
                && importable.contains(&format(entry.code, Modifier::Invalid))
                && !importable
                    .iter()
                    .any(|f| f.code == entry.code && f.modifier != Modifier::Invalid);
            assert!(
                importable.contains(&entry) || widened,
                "{entry:?} is neither in the driver's set nor the documented \
                 LINEAR widening of a candidate: {importable:?}"
            );
            assert_ne!(entry.modifier, Modifier::Invalid);
        }
    }
}

/// Prints what building the table costs at a real GPU's size, for the record
/// CLAUDE.md asks for. Startup-only, so there is no hot path to protect; the
/// number is here so "hundreds of pairs" is a measurement, not a shrug.
///
/// ```text
/// cargo test --release -p scoot --bin scoot driver_tranche_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints a timing for a human; asserts nothing"]
fn driver_tranche_cost() {
    use smithay::wayland::dmabuf::DmabufFeedbackBuilder;

    // 24 fourccs x (16 explicit modifiers + Invalid) = 408 entries: larger
    // than any driver this project has measured (llvmpipe: 57 x 2 = 114;
    // a modifier-rich desktop GPU is in the low hundreds of pairs).
    const FOURCCS: [Fourcc; 24] = [
        Fourcc::Xrgb8888,
        Fourcc::Argb8888,
        Fourcc::Abgr8888,
        Fourcc::Xbgr8888,
        Fourcc::Rgba8888,
        Fourcc::Rgbx8888,
        Fourcc::Bgra8888,
        Fourcc::Bgrx8888,
        Fourcc::Argb2101010,
        Fourcc::Xrgb2101010,
        Fourcc::Abgr2101010,
        Fourcc::Xbgr2101010,
        Fourcc::Abgr16161616f,
        Fourcc::Xbgr16161616f,
        Fourcc::Rgb565,
        Fourcc::R8,
        Fourcc::Gr88,
        Fourcc::Nv12,
        Fourcc::Nv21,
        Fourcc::P010,
        Fourcc::Yuv420,
        Fourcc::Yvu420,
        Fourcc::Yuyv,
        Fourcc::Uyvy,
    ];
    let mut formats = Vec::new();
    for code in FOURCCS {
        for modifier in 0u64..16 {
            formats.push(format(code, Modifier::from(modifier)));
        }
        formats.push(format(code, Modifier::Invalid));
    }
    let importable: FormatSet = formats.into_iter().collect();
    const ROUNDS: u32 = 1_000;
    let started = std::time::Instant::now();
    let mut pairs = 0;
    for _ in 0..ROUNDS {
        pairs = std::hint::black_box(driver_tranche(&importable)).len();
    }
    let tranche = started.elapsed() / ROUNDS;
    let table = driver_tranche(&importable);
    let started = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let feedback = DmabufFeedbackBuilder::new(0, table.iter().copied())
            .build()
            .expect("a feedback table");
        std::hint::black_box(feedback);
    }
    let build = started.elapsed() / ROUNDS;
    println!(
        "driver_tranche over {} importable entries -> {pairs} advertised: {tranche:?}; \
         DmabufFeedbackBuilder::build of that table (memfd + seal): {build:?} \
         ({ROUNDS} rounds each)",
        importable.indexset().len()
    );
}
