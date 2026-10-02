//! The scanout tranche's rule, pinned against the plane shapes it has to
//! read: the dev VM's virtio-gpu (no `IN_FORMATS`: every fourcc at `Invalid`
//! only), a modifier-capable plane (`IN_FORMATS` naming `LINEAR` and tiled
//! layouts), and the corners in between. Pure -- no DRM device, no client --
//! because the tranche is a function of two lists; what a client receives
//! over the wire, and when, is pinned in `fullscreen/tests/scanout_feedback.rs`.

use smithay::backend::allocator::format::FormatSet;
use smithay::backend::allocator::{Format, Fourcc, Modifier};

use super::{FormatsKey, ScanoutFeedback, overlay_tranche, scanout_tranche, single_plane};

const TILED: Modifier = Modifier::I915_x_tiled;
const OTHER_TILED: Modifier = Modifier::I915_y_tiled;

fn f(code: Fourcc, modifier: Modifier) -> Format {
    Format { code, modifier }
}

fn plane(formats: &[Format]) -> FormatSet {
    formats.iter().copied().collect()
}

/// What Smithay reads off a plane with no `IN_FORMATS`: each fourcc at
/// `Invalid` and nothing else. The dev VM's virtio-gpu primary, give or
/// take the fourcc list.
fn implicit_plane(codes: &[Fourcc]) -> FormatSet {
    codes
        .iter()
        .map(|code| f(*code, Modifier::Invalid))
        .collect()
}

/// The dev VM's GLES table, in shape: the two candidates first at `LINEAR`,
/// then a single-plane fourcc the plane does not list, a multi-plane one,
/// and a 16-bit packed one.
fn llvmpipe_table() -> Vec<Format> {
    vec![
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Argb8888, Modifier::Linear),
        f(Fourcc::Abgr8888, Modifier::Linear),
        f(Fourcc::Nv12, Modifier::Linear),
        f(Fourcc::Rgb565, Modifier::Linear),
    ]
}

#[test]
fn an_implicit_only_plane_offers_linear_for_the_single_plane_formats_it_lists() {
    // The virtio shape, and the path PR #228 measured going direct: a
    // single-plane LINEAR buffer is GBM-imported without modifiers, so its
    // framebuffer is `{XR24, Invalid}`, which this plane lists.
    let plane = implicit_plane(&[Fourcc::Xrgb8888, Fourcc::Argb8888]);
    assert_eq!(
        scanout_tranche(&llvmpipe_table(), &plane, &[]),
        vec![
            f(Fourcc::Xrgb8888, Modifier::Linear),
            f(Fourcc::Argb8888, Modifier::Linear),
        ],
        "ABGR is not listed (its opaque twin XBGR is not either), RGB565 is \
         not listed, and NV12 is multi-plane: it takes the modifier import, \
         and an implicit-only plane lists no explicit modifier at all"
    );
}

#[test]
fn an_alpha_format_is_matched_as_its_opaque_twin() {
    // The primary exports with the opaque fallback, so an `AR24` buffer
    // becomes an `XR24` framebuffer: admitted where only `XR24` is listed,
    // and refused where only `AR24` is -- exact-fourcc matching would get
    // both backwards.
    let table = [f(Fourcc::Argb8888, Modifier::Linear)];
    assert_eq!(
        scanout_tranche(&table, &implicit_plane(&[Fourcc::Xrgb8888]), &[]),
        table.to_vec()
    );
    assert!(scanout_tranche(&table, &implicit_plane(&[Fourcc::Argb8888]), &[]).is_empty());
    // Explicit modifiers follow the same mapping.
    let tiled = [f(Fourcc::Argb8888, TILED)];
    assert_eq!(
        scanout_tranche(
            &tiled,
            &plane(&[
                f(Fourcc::Xrgb8888, Modifier::Invalid),
                f(Fourcc::Xrgb8888, TILED)
            ]),
            &[]
        ),
        tiled.to_vec()
    );
}

#[test]
fn a_modifier_plane_offers_exactly_the_explicit_layouts_it_lists() {
    let plane = plane(&[
        f(Fourcc::Xrgb8888, Modifier::Invalid),
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
    ]);
    let table = [
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Xrgb8888, OTHER_TILED),
        f(Fourcc::Argb8888, TILED),
        f(Fourcc::Argb8888, OTHER_TILED),
    ];
    assert_eq!(
        scanout_tranche(&table, &plane, &[]),
        vec![
            f(Fourcc::Xrgb8888, Modifier::Linear),
            f(Fourcc::Xrgb8888, TILED),
            f(Fourcc::Argb8888, TILED),
        ],
        "only the layouts the plane names, in the advertised order"
    );
}

#[test]
fn linear_is_not_offered_where_the_plane_named_its_layouts_without_it() {
    // The plane has `IN_FORMATS` for XR24 and `LINEAR` is not in it. A
    // no-modifier framebuffer of a linear buffer would pass Smithay's list
    // check on the `Invalid` entry every plane carries, and then fail the
    // atomic test: the plane has said what it takes.
    let plane = plane(&[
        f(Fourcc::Xrgb8888, Modifier::Invalid),
        f(Fourcc::Xrgb8888, TILED),
    ]);
    let table = [
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
    ];
    assert_eq!(
        scanout_tranche(&table, &plane, &[]),
        vec![f(Fourcc::Xrgb8888, TILED)]
    );
}

#[test]
fn a_multi_plane_linear_format_needs_an_explicit_linear_entry() {
    // NV12 at LINEAR takes the modifier import (more than one plane), so its
    // framebuffer carries `LINEAR` and the plane must list exactly that.
    let table = [f(Fourcc::Nv12, Modifier::Linear)];
    assert!(scanout_tranche(&table, &implicit_plane(&[Fourcc::Nv12]), &[]).is_empty());
    let explicit = plane(&[
        f(Fourcc::Nv12, Modifier::Invalid),
        f(Fourcc::Nv12, Modifier::Linear),
    ]);
    assert_eq!(scanout_tranche(&table, &explicit, &[]), table.to_vec());
}

#[test]
fn implicit_is_never_offered() {
    // The default table never carries `Invalid`; if it ever did, the plane's
    // own `Invalid` entry must not smuggle it into the scanout tranche.
    // Smithay refuses to scan out an implicit client buffer regardless.
    let table = [f(Fourcc::Xrgb8888, Modifier::Invalid)];
    let plane = implicit_plane(&[Fourcc::Xrgb8888]);
    assert!(scanout_tranche(&table, &plane, &[]).is_empty());
}

#[test]
fn a_modifier_the_exporter_refused_is_dropped_and_linear_never_is() {
    // A modifier this device's GBM lost is one the layout exporter will
    // refuse, so offering it could only ever composite. `LINEAR` is never
    // refused by that exporter (it passes whatever GBM reports), so a `lost`
    // list naming it -- which the exporter never produces -- changes nothing.
    let plane = plane(&[
        f(Fourcc::Xrgb8888, Modifier::Invalid),
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Xrgb8888, OTHER_TILED),
    ]);
    let table = [
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Xrgb8888, OTHER_TILED),
    ];
    assert_eq!(
        scanout_tranche(&table, &plane, &[TILED]),
        vec![
            f(Fourcc::Xrgb8888, Modifier::Linear),
            f(Fourcc::Xrgb8888, OTHER_TILED),
        ]
    );
    assert_eq!(
        scanout_tranche(&table, &plane, &[Modifier::Linear]),
        table.to_vec()
    );
}

#[test]
fn a_plane_that_takes_nothing_advertised_gives_an_empty_tranche() {
    assert!(scanout_tranche(&llvmpipe_table(), &FormatSet::default(), &[]).is_empty());
    assert!(scanout_tranche(&llvmpipe_table(), &implicit_plane(&[Fourcc::Yuyv]), &[]).is_empty());
    assert!(scanout_tranche(&[], &implicit_plane(&[Fourcc::Xrgb8888]), &[]).is_empty());
}

#[test]
fn the_tranche_is_an_ordered_subset_of_the_advertised_table_for_every_shape() {
    // The property the promise with teeth rests on: nothing reaches the
    // scanout tranche that the default table does not already offer (and so
    // import), and the default's preference order is kept. Swept over every
    // plane built from a small universe of entries.
    let universe = [
        f(Fourcc::Xrgb8888, Modifier::Invalid),
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Argb8888, Modifier::Invalid),
        f(Fourcc::Argb8888, OTHER_TILED),
        f(Fourcc::Nv12, Modifier::Invalid),
        f(Fourcc::Nv12, Modifier::Linear),
    ];
    let table = [
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Argb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Argb8888, TILED),
        f(Fourcc::Argb8888, OTHER_TILED),
        f(Fourcc::Nv12, Modifier::Linear),
    ];
    for mask in 0u32..(1 << universe.len()) {
        let plane: FormatSet = universe
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, format)| *format)
            .collect();
        for lost in [&[][..], &[TILED][..]] {
            let tranche = scanout_tranche(&table, &plane, lost);
            let mut cursor = table.iter();
            for entry in &tranche {
                assert_ne!(entry.modifier, Modifier::Invalid, "mask {mask:#b}");
                assert!(
                    cursor.any(|advertised| advertised == entry),
                    "mask {mask:#b}: {entry:?} is not an in-order entry of the table"
                );
            }
        }
    }
}

#[test]
fn a_fresh_tracker_needs_a_build_and_a_built_one_only_for_a_new_key() {
    // The frame path's per-frame question: rebuild only when the plane set
    // or the lost-modifier record moved.
    let mut feedback = ScanoutFeedback::default();
    let key = FormatsKey { planes: 0, lost: 0 };
    assert!(feedback.needs_build(key));
    feedback.install(key, None, None);
    assert!(!feedback.needs_build(key), "a cached `None` is cached too");
    assert!(feedback.needs_build(FormatsKey { planes: 1, lost: 0 }));
    assert!(feedback.needs_build(FormatsKey { planes: 0, lost: 1 }));
}

#[test]
fn single_plane_means_packed_and_known() {
    // Smithay's table carries two planar formats among the packed ones; the
    // `LINEAR`-on-an-implicit-plane offer must not reach either, because a
    // multi-plane buffer takes the modifier import and its framebuffer
    // carries `LINEAR`, which such a plane does not list.
    for planar in [Fourcc::Nv12, Fourcc::Yuv420] {
        assert!(!single_plane(planar), "{planar:?}");
    }
    for packed in [
        Fourcc::Xrgb8888,
        Fourcc::Argb8888,
        Fourcc::Rgb565,
        Fourcc::Rgb888,
        Fourcc::Xrgb2101010,
        Fourcc::Argb16161616f,
    ] {
        assert!(single_plane(packed), "{packed:?}");
    }
    // Not in Smithay's table: conservatively not offered.
    for unknown in [Fourcc::Yuyv, Fourcc::P010, Fourcc::Nv21] {
        assert!(!single_plane(unknown), "{unknown:?}");
    }
}

/// Prints what building the scanout tranche costs at a real GPU's size --
/// startup and CRTC switches only, never a frame -- so "a few dozen fourccs
/// by a few modifiers" is a measurement. Run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot --features gpu-scanout scanout_tranche_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints a timing for a human; asserts nothing"]
fn scanout_tranche_cost() {
    const ROUNDS: u32 = 1_000;
    // 24 fourccs x 17 modifiers advertised (408 pairs, the size
    // `driver_tranche_cost` measures), against a plane listing 12 of those
    // fourccs at 8 modifiers plus `Invalid`.
    let fourccs = [
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
    let advertised: Vec<Format> = fourccs
        .iter()
        .flat_map(|code| (0u64..17).map(|m| f(*code, Modifier::from(m))))
        .collect();
    let plane: FormatSet = fourccs[..12]
        .iter()
        .flat_map(|code| {
            (0u64..8)
                .map(|m| f(*code, Modifier::from(m)))
                .chain(std::iter::once(f(*code, Modifier::Invalid)))
        })
        .collect();
    let started = std::time::Instant::now();
    let mut kept = 0;
    for _ in 0..ROUNDS {
        kept = std::hint::black_box(scanout_tranche(&advertised, &plane, &[])).len();
    }
    let each = started.elapsed() / ROUNDS;
    println!(
        "scanout tranche: {} advertised x {} plane entries -> {kept} pairs in {each:?} ({ROUNDS} rounds)",
        advertised.len(),
        plane.iter().count()
    );
}

/// `apple,dcp`'s overlay plane, in shape: `LINEAR` only, alpha formats and
/// YUV, no opaque `X` fourcc (`IN_FORMATS` names exactly eight fourccs, of
/// which Smithay's `Fourcc` names four -- `NV16`, `NV24`, `P010` and `P210`
/// have no variant, so Smithay's plane list, and this tranche, never hold
/// them).
fn dcp_overlay() -> FormatSet {
    [
        Fourcc::Argb2101010,
        Fourcc::Argb8888,
        Fourcc::Abgr8888,
        Fourcc::Nv12,
    ]
    .into_iter()
    .map(|code| f(code, Modifier::Linear))
    .collect()
}

#[test]
fn the_overlay_tranche_holds_linear_alpha_formats_and_no_opaque_twin() {
    // No opaque fallback on the overlay path: an `AR24` buffer is added as
    // `AR24`, so it is admitted where `AR24` is listed -- and an `XR24`
    // buffer is refused where only `XR24` is, the exact inverse of the
    // primary arm's twin mapping.
    let table = vec![
        f(Fourcc::Argb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, Modifier::Linear),
    ];
    assert_eq!(
        overlay_tranche(&table, &[&dcp_overlay()], &[]),
        vec![f(Fourcc::Argb8888, Modifier::Linear)],
        "XR24 is not listed on the overlay, and nothing maps it to AR24"
    );
    assert_eq!(
        overlay_tranche(&table, &[&implicit_plane(&[Fourcc::Xrgb8888])], &[]),
        vec![f(Fourcc::Xrgb8888, Modifier::Linear)],
        "no twin mapping: AR24 is refused where only XR24 is listed, while \
         XR24 itself still takes the implicit-plane LINEAR fallback"
    );
    // The primary arm, same inputs: the twin mapping admits AR24 where XR24
    // is listed, and refuses it where only AR24 is.
    assert_eq!(
        scanout_tranche(&table, &implicit_plane(&[Fourcc::Xrgb8888]), &[]),
        table,
    );
}

#[test]
fn the_overlay_tranche_unions_the_crtc_overlays_and_drops_lost_layouts() {
    // Two overlays listing different halves: a client may allocate either,
    // and Smithay assigns whichever fits.
    let low = plane(&[f(Fourcc::Argb8888, Modifier::Linear)]);
    let high = plane(&[f(Fourcc::Nv12, Modifier::Linear)]);
    let table = vec![
        f(Fourcc::Argb8888, Modifier::Linear),
        f(Fourcc::Xrgb8888, TILED),
        f(Fourcc::Nv12, Modifier::Linear),
    ];
    assert_eq!(
        overlay_tranche(&table, &[&low, &high], &[]),
        vec![
            f(Fourcc::Argb8888, Modifier::Linear),
            f(Fourcc::Nv12, Modifier::Linear),
        ],
        "tiled XR24 is in neither overlay's list"
    );
    // A tiled layout both the client and an overlay agree on is offered --
    // until the exporter sees GBM lose it, when it is dropped.
    let tiled = plane(&[f(Fourcc::Xrgb8888, TILED)]);
    let table = vec![f(Fourcc::Xrgb8888, TILED)];
    assert_eq!(overlay_tranche(&table, &[&tiled], &[]), table);
    assert!(
        overlay_tranche(&table, &[&tiled], &[TILED]).is_empty(),
        "a lost modifier steers nothing"
    );
}

#[test]
fn implicit_is_never_offered_on_the_overlay_either() {
    // `Modifier::Invalid` is not a layout a client can allocate: refused on
    // both arms, whatever the plane lists.
    let table = vec![f(Fourcc::Argb8888, Modifier::Invalid)];
    assert!(overlay_tranche(&table, &[&dcp_overlay()], &[]).is_empty());
}

#[test]
fn the_overlay_tranche_is_an_ordered_subset_of_the_advertised_table() {
    // The promise with teeth, overlay arm: every steered pair is already one
    // the default offers every client, so a client that allocates from it
    // and then composites imports like any other.
    let advertised = vec![
        f(Fourcc::Argb8888, Modifier::Linear),
        f(Fourcc::Argb8888, TILED),
        f(Fourcc::Xrgb8888, Modifier::Linear),
        f(Fourcc::Nv12, Modifier::Linear),
    ];
    let tranche = overlay_tranche(&advertised, &[&dcp_overlay()], &[]);
    assert_eq!(
        tranche,
        vec![
            f(Fourcc::Argb8888, Modifier::Linear),
            f(Fourcc::Nv12, Modifier::Linear),
        ]
    );
    let positions: Vec<usize> = tranche
        .iter()
        .map(|kept| advertised.iter().position(|pair| pair == kept).unwrap())
        .collect();
    let mut ordered = positions.clone();
    ordered.sort();
    assert_eq!(positions, ordered, "the advertised order, filtered");
}
