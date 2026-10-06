//! Tests for the CTM night-light fallback (`super`).
//!
//! The matrix math is pure, so every case here runs without hardware:
//! hand-computed S31.32 vectors (independently derived with integer
//! arithmetic, not copied from the implementation), the exactness
//! theorem for linear ramps, the endpoint-fit contract for curved ones,
//! and the malformed-input refusals. The ioctl half -- property lookup
//! and the atomic commit -- needs DRM master on a real CRTC and is
//! exercised live on the M2 panel (see the PR description), not here.

use super::{Ctm, IDENTITY, from_ramps};

/// A 4-entry linear ramp: `i * 65535 / 3`, the `linear_ramp` shape.
fn linear4() -> Vec<u16> {
    vec![0, 21845, 43690, 65535]
}

#[test]
fn linear_ramps_reproduce_the_identity_matrix_exactly() {
    // The exactness theorem this fallback rests on: at the daemons'
    // default gamma the ramp is a pure linear scale, so the diagonal
    // fit must be bit-exact, not approximate.
    let ramp = linear4();
    assert_eq!(
        from_ramps(&ramp, &ramp, &ramp)
            .expect("a linear ramp converts")
            .matrix,
        IDENTITY.matrix,
    );
}

#[test]
fn identity_is_a_unit_diagonal_in_s31_32() {
    assert_eq!(
        IDENTITY.matrix,
        [1 << 32, 0, 0, 0, 1 << 32, 0, 0, 0, 1 << 32,],
    );
}

#[test]
fn the_blob_layout_matches_drm_color_ctm() {
    // Nine little-endian u64s, row-major: the kernel reads exactly
    // `sizeof(struct drm_color_ctm)` bytes from the blob.
    assert_eq!(std::mem::size_of::<Ctm>(), 72);
    assert_eq!(std::mem::align_of::<Ctm>(), 8);
}

#[test]
fn each_channel_scales_only_itself() {
    let full = vec![65535, 65535, 65535];
    let off = vec![0, 0, 0];
    let matrix = from_ramps(&full, &off, &off)
        .expect("constant ramps convert")
        .matrix;
    assert_eq!(
        matrix,
        [1 << 32, 0, 0, 0, 0, 0, 0, 0, 0],
        "red at full with green and blue off warms nothing else",
    );
}

#[test]
fn endpoint_values_hit_hand_computed_s31_32() {
    // Independently computed as `round(last * 2^32 / 65535)` with integer
    // arithmetic: a regression here means the fixed-point encoding moved.
    let cases: &[(u16, u64)] = &[
        (0, 0x0),
        (1, 0x10001),
        (16877, 0x41ed_41ed),
        (32767, 0x7fff_7fff),
        (32768, 0x8000_8001),
        (46811, 0xb6db_b6dc),
        (65534, 0xfffe_ffff),
        (65535, 0x1_0000_0000),
    ];
    for (last, expected) in cases {
        let ramp = vec![0, *last];
        let matrix = from_ramps(&ramp, &ramp, &ramp)
            .expect("a two-entry ramp converts")
            .matrix;
        assert_eq!(matrix[0], *expected, "red endpoint {last}");
        assert_eq!(matrix[4], *expected, "green endpoint {last}");
        assert_eq!(matrix[8], *expected, "blue endpoint {last}");
        for (index, value) in matrix.iter().enumerate() {
            if index != 0 && index != 4 && index != 8 {
                assert_eq!(*value, 0, "off-diagonal [{index}] stays zero");
            }
        }
    }
}

#[test]
fn a_wlsunset_shaped_ramp_lands_on_its_whitepoint() {
    // `fill_gamma_table` at gamma 1.0 with whitepoint (1.0, 0.714, 0.257):
    // `ramp[i] = 65535 * (i / (size - 1)) * channel`, truncated to u16
    // the way the C cast truncates. The matrix must carry exactly those
    // channel scales -- warming is the whitepoint, nothing else.
    let size: usize = 256;
    let ramp = |channel: f64| {
        (0..size)
            .map(|i| (65535.0 * (i as f64 / (size - 1) as f64) * channel) as u16)
            .collect::<Vec<_>>()
    };
    let (red, green, blue) = (ramp(1.0), ramp(0.714), ramp(0.257));
    let matrix = from_ramps(&red, &green, &blue)
        .expect("a daemon-shaped ramp converts")
        .matrix;
    // Endpoints after C-style truncation: green 46791, blue 16842; the
    // matrix carries `round(endpoint * 2^32 / 65535)` per channel,
    // independently computed (0x1_0000_0000, 0xb6c7_b6c8, 0x41ca_41ca).
    assert_eq!(red[size - 1], 65535);
    assert_eq!(green[size - 1], 46791);
    assert_eq!(blue[size - 1], 16842);
    assert_eq!(matrix[0], 0x1_0000_0000);
    assert_eq!(matrix[4], 0xb6c7_b6c8);
    assert_eq!(matrix[8], 0x41ca_41ca);
}

#[test]
fn a_curved_ramp_still_matches_white_and_black() {
    // A non-default gamma bends the mid-tones (`pow(val * c, 1/g)`),
    // which no 3x3 matrix can follow. The contract is weaker but still
    // useful: black stays black, white lands on the curve's endpoint.
    let size: usize = 256;
    let gamma = 2.2;
    let channel = 0.6;
    let ramp = (0..size)
        .map(|i| {
            let val = i as f64 / (size - 1) as f64;
            (65535.0 * (val * channel).powf(1.0 / gamma)) as u16
        })
        .collect::<Vec<_>>();
    assert_eq!(ramp[0], 0, "black stays black in the ramp itself");
    let matrix = from_ramps(&ramp, &ramp, &ramp)
        .expect("a curved ramp converts")
        .matrix;
    // Curve endpoint 51955 lands as `round(51955 * 2^32 / 65535)`
    // (0xcaf3_caf4, independently computed).
    assert_eq!(ramp[size - 1], 51955);
    assert_eq!(matrix[0], 0xcaf3_caf4, "white follows the curve endpoint");
    assert_eq!(matrix[4], 0xcaf3_caf4);
    assert_eq!(matrix[8], 0xcaf3_caf4);
}

#[test]
fn malformed_ramps_convert_to_nothing() {
    let ramp = linear4();
    assert!(
        from_ramps(&[], &[], &[]).is_none(),
        "empty converts to none"
    );
    assert!(
        from_ramps(&ramp, &[], &ramp).is_none(),
        "a missing channel converts to none"
    );
    assert!(
        from_ramps(&ramp, &ramp[..2], &ramp).is_none(),
        "ragged channels convert to none"
    );
}
