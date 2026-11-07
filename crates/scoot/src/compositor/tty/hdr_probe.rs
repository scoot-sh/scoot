//! A read-only report of what each driven connector offers towards HDR,
//! logged once at `--tty` startup, and the reason HDR cannot turn on where
//! it cannot.
//!
//! This is stage 1 of the plan in
//! `docs/backlog/resolved/hdr-color-management-done.md`: knowing what the
//! connector can do comes before any attempt to use it. Nothing here
//! writes a property, commits, or changes a format; scoot's output is
//! 8-bit SDR on every tier whatever this reports.
//!
//! The kernel exposes HDR through four pieces, and all four have to be
//! there for HDR output to be possible at all:
//!
//! - the connector's `HDR_OUTPUT_METADATA` blob property, which carries the
//!   static metadata infoframe;
//! - the connector's `Colorspace` enum property with a BT.2020 value, which
//!   tells the sink how to read the pixels;
//! - the connector's `max bpc` range property reaching 10, so the link can
//!   carry more than 8 bits per channel;
//! - a 10-bit format (`XRGB2101010` family) on the CRTC's primary plane, so
//!   something can be scanned out at that depth.
//!
//! The decision is a pure function over a plain [`Snapshot`] of those
//! four, so every combination is pinnable without DRM hardware (see
//! `hdr_probe/tests.rs`); [`read`] is the thin live half that fills the
//! snapshot from a device. Presence of the controls is what is reported:
//! whether the attached *sink* declares HDR support (the EDID's HDR static
//! metadata block) is not read here.
//!
//! One property walk and one plane query per driven connector, at startup
//! only -- never on a frame, input or IPC path.

use std::fmt;

use smithay::backend::allocator::Fourcc;
use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::Device as DrmControl;
use smithay::reexports::drm::control::connector;
use smithay::reexports::drm::control::crtc;
use smithay::reexports::drm::control::property::ValueType;

#[cfg(test)]
mod tests;

/// What one connector and its primary plane expose, as plain data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    /// The connector has an `HDR_OUTPUT_METADATA` property.
    pub(super) hdr_output_metadata: bool,
    /// The names of the connector's `Colorspace` enum values, or `None`
    /// when it has no such property.
    pub(super) colorspace: Option<Vec<String>>,
    /// The connector's `max bpc` range, `(min, max)`, or `None` when it has
    /// no such property (or one that is not an unsigned range).
    pub(super) max_bpc: Option<(u64, u64)>,
    /// The CRTC's primary plane lists a 10-bit-per-channel format.
    pub(super) ten_bit_plane: bool,
}

/// One thing HDR needs that a connector does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Missing {
    HdrOutputMetadata,
    ColorspaceProperty,
    /// `Colorspace` exists but offers no BT.2020 value.
    ColorspaceBt2020,
    MaxBpcProperty,
    /// `max bpc` exists but tops out below 10; carries the top.
    MaxBpcBelowTen(u64),
    TenBitPlaneFormat,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HdrOutputMetadata => f.write_str("no HDR_OUTPUT_METADATA property"),
            Self::ColorspaceProperty => f.write_str("no Colorspace property"),
            Self::ColorspaceBt2020 => f.write_str("Colorspace offers no BT2020 value"),
            Self::MaxBpcProperty => f.write_str("no max bpc property"),
            Self::MaxBpcBelowTen(top) => write!(f, "max bpc tops out at {top} (needs 10)"),
            Self::TenBitPlaneFormat => {
                f.write_str("primary plane lists no 10-bit format (XRGB2101010 family)")
            }
        }
    }
}

/// The 10-bit-per-channel formats the probe counts: the four RGB
/// permutations the kernel defines at 2-10-10-10.
const TEN_BIT: [Fourcc; 4] = [
    Fourcc::Xrgb2101010,
    Fourcc::Argb2101010,
    Fourcc::Xbgr2101010,
    Fourcc::Abgr2101010,
];

/// Whether `code` is one of the [`TEN_BIT`] formats.
pub(super) fn is_ten_bit(code: Fourcc) -> bool {
    TEN_BIT.contains(&code)
}

/// Whether a `Colorspace` value name is a BT.2020 one (`BT2020_RGB`,
/// `BT2020_YCC`, `BT2020_CYCC`: the kernel's HDR-capable colorimetries).
fn is_bt2020(name: &str) -> bool {
    name.starts_with("BT2020")
}

/// Every piece of HDR the snapshot lacks, in the order the log names them.
/// Empty means the connector exposes everything HDR output needs.
pub(super) fn missing(snapshot: &Snapshot) -> Vec<Missing> {
    let mut missing = Vec::new();
    if !snapshot.hdr_output_metadata {
        missing.push(Missing::HdrOutputMetadata);
    }
    match &snapshot.colorspace {
        None => missing.push(Missing::ColorspaceProperty),
        Some(values) if !values.iter().any(|name| is_bt2020(name)) => {
            missing.push(Missing::ColorspaceBt2020);
        }
        Some(_) => {}
    }
    match snapshot.max_bpc {
        None => missing.push(Missing::MaxBpcProperty),
        Some((_, top)) if top < 10 => missing.push(Missing::MaxBpcBelowTen(top)),
        Some(_) => {}
    }
    if !snapshot.ten_bit_plane {
        missing.push(Missing::TenBitPlaneFormat);
    }
    missing
}

/// Reads `connector`'s properties and `crtc`'s primary plane formats.
/// Every failed read counts as "not there": a probe that cannot see a
/// property reports it missing, which is the safe answer (HDR stays off).
pub(super) fn read(drm: &DrmDevice, connector: connector::Handle, crtc: crtc::Handle) -> Snapshot {
    let mut snapshot = Snapshot::default();
    if let Ok(set) = drm.get_properties(connector) {
        let (handles, _) = set.as_props_and_values();
        for handle in handles {
            let Ok(info) = drm.get_property(*handle) else {
                continue;
            };
            match info.name().to_bytes() {
                b"HDR_OUTPUT_METADATA" => snapshot.hdr_output_metadata = true,
                b"Colorspace" => {
                    if let ValueType::Enum(values) = info.value_type() {
                        let (_, names) = values.values();
                        snapshot.colorspace = Some(
                            names
                                .iter()
                                .map(|value| value.name().to_string_lossy().into_owned())
                                .collect(),
                        );
                    }
                }
                b"max bpc" => {
                    if let ValueType::UnsignedRange(min, max) = info.value_type() {
                        snapshot.max_bpc = Some((min, max));
                    }
                }
                _ => {}
            }
        }
    }
    if let Ok(planes) = drm.planes(&crtc) {
        snapshot.ten_bit_plane = planes
            .primary
            .iter()
            .any(|plane| plane.formats.iter().any(|format| is_ten_bit(format.code)));
    }
    snapshot
}

/// Probes and logs one connector. HDR is off whatever the answer; the line
/// says whether that is for want of a piece or only because it is not
/// implemented yet.
pub(super) fn report(
    drm: &DrmDevice,
    name: &str,
    connector: connector::Handle,
    crtc: crtc::Handle,
) {
    let snapshot = read(drm, connector, crtc);
    log(name, &snapshot);
}

fn log(name: &str, snapshot: &Snapshot) {
    let colorspace = ColorspaceValues(snapshot.colorspace.as_deref());
    let max_bpc = MaxBpc(snapshot.max_bpc);
    let missing = missing(snapshot);
    if missing.is_empty() {
        tracing::info!(
            connector = %name,
            hdr_output_metadata = snapshot.hdr_output_metadata,
            colorspace = %colorspace,
            max_bpc = %max_bpc,
            ten_bit_plane = snapshot.ten_bit_plane,
            "drm: hdr probe: connector exposes everything HDR output needs; \
             HDR stays off (output is 8-bit SDR)"
        );
    } else {
        tracing::info!(
            connector = %name,
            hdr_output_metadata = snapshot.hdr_output_metadata,
            colorspace = %colorspace,
            max_bpc = %max_bpc,
            ten_bit_plane = snapshot.ten_bit_plane,
            missing = %Joined(&missing),
            "drm: hdr probe: HDR cannot turn on for this connector (output is 8-bit SDR)"
        );
    }
}

/// `Default,BT2020_RGB`, or `none` without the property.
struct ColorspaceValues<'a>(Option<&'a [String]>);

impl fmt::Display for ColorspaceValues<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(values) = self.0 else {
            return f.write_str("none");
        };
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                f.write_str(",")?;
            }
            f.write_str(value)?;
        }
        Ok(())
    }
}

/// `8..16`, or `none`.
struct MaxBpc(Option<(u64, u64)>);

impl fmt::Display for MaxBpc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some((min, max)) => write!(f, "{min}..{max}"),
            None => f.write_str("none"),
        }
    }
}

/// The missing pieces, `; `-separated.
struct Joined<'a>(&'a [Missing]);

impl fmt::Display for Joined<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, piece) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{piece}")?;
        }
        Ok(())
    }
}
