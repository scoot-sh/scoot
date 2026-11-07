//! The HDR-readiness decision over plain snapshots: no DRM device needed.

use super::*;

fn ready() -> Snapshot {
    Snapshot {
        hdr_output_metadata: true,
        colorspace: Some(vec!["Default".into(), "BT2020_RGB".into()]),
        max_bpc: Some((6, 12)),
        ten_bit_plane: true,
    }
}

#[test]
fn a_connector_with_all_four_pieces_misses_nothing() {
    assert_eq!(missing(&ready()), vec![]);
}

#[test]
fn an_empty_snapshot_names_every_piece() {
    // Apple's DCP and a virtual GPU look like this: nothing attached.
    assert_eq!(
        missing(&Snapshot::default()),
        vec![
            Missing::HdrOutputMetadata,
            Missing::ColorspaceProperty,
            Missing::MaxBpcProperty,
            Missing::TenBitPlaneFormat,
        ]
    );
}

#[test]
fn each_piece_is_named_alone() {
    let mut s = ready();
    s.hdr_output_metadata = false;
    assert_eq!(missing(&s), vec![Missing::HdrOutputMetadata]);

    let mut s = ready();
    s.colorspace = None;
    assert_eq!(missing(&s), vec![Missing::ColorspaceProperty]);

    let mut s = ready();
    s.max_bpc = None;
    assert_eq!(missing(&s), vec![Missing::MaxBpcProperty]);

    let mut s = ready();
    s.ten_bit_plane = false;
    assert_eq!(missing(&s), vec![Missing::TenBitPlaneFormat]);
}

#[test]
fn a_colorspace_without_bt2020_is_not_enough() {
    let mut s = ready();
    s.colorspace = Some(vec![
        "Default".into(),
        "opRGB".into(),
        "DCI-P3_RGB_D65".into(),
    ]);
    assert_eq!(missing(&s), vec![Missing::ColorspaceBt2020]);
    s.colorspace = Some(Vec::new());
    assert_eq!(missing(&s), vec![Missing::ColorspaceBt2020]);
}

#[test]
fn any_bt2020_colorimetry_counts() {
    for name in ["BT2020_RGB", "BT2020_YCC", "BT2020_CYCC"] {
        let mut s = ready();
        s.colorspace = Some(vec![name.into()]);
        assert_eq!(missing(&s), vec![], "{name}");
    }
}

#[test]
fn max_bpc_must_reach_ten() {
    let mut s = ready();
    s.max_bpc = Some((8, 8));
    assert_eq!(missing(&s), vec![Missing::MaxBpcBelowTen(8)]);
    s.max_bpc = Some((8, 9));
    assert_eq!(missing(&s), vec![Missing::MaxBpcBelowTen(9)]);
    s.max_bpc = Some((8, 10));
    assert_eq!(missing(&s), vec![]);
    s.max_bpc = Some((0, 0));
    assert_eq!(missing(&s), vec![Missing::MaxBpcBelowTen(0)]);
}

#[test]
fn the_ten_bit_set_is_the_2101010_family_only() {
    for code in TEN_BIT {
        assert!(is_ten_bit(code), "{code:?}");
    }
    for code in [
        Fourcc::Xrgb8888,
        Fourcc::Argb8888,
        Fourcc::Rgb565,
        Fourcc::Xbgr16161616f,
    ] {
        assert!(!is_ten_bit(code), "{code:?}");
    }
}

#[test]
fn the_missing_wording_names_the_piece() {
    assert_eq!(
        Missing::HdrOutputMetadata.to_string(),
        "no HDR_OUTPUT_METADATA property"
    );
    assert_eq!(
        Missing::MaxBpcBelowTen(8).to_string(),
        "max bpc tops out at 8 (needs 10)"
    );
    assert_eq!(
        Joined(&[Missing::ColorspaceProperty, Missing::TenBitPlaneFormat]).to_string(),
        "no Colorspace property; primary plane lists no 10-bit format (XRGB2101010 family)"
    );
}

#[test]
fn the_log_fragments_render_absent_and_present_values() {
    assert_eq!(ColorspaceValues(None).to_string(), "none");
    let values = vec!["Default".to_owned(), "BT2020_RGB".to_owned()];
    assert_eq!(
        ColorspaceValues(Some(&values)).to_string(),
        "Default,BT2020_RGB"
    );
    assert_eq!(MaxBpc(None).to_string(), "none");
    assert_eq!(MaxBpc(Some((8, 16))).to_string(), "8..16");
}
