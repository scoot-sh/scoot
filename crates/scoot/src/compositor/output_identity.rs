//! Which monitor an output is, across unplug and replug.
//!
//! A replugged monitor comes back under a fresh [`OutputId`](scoot_core::OutputId)
//! (ids are never reused), so anything that must follow the *monitor* rather
//! than the id -- restoring its workspaces, keeping the default output binds
//! reaching it -- keys on this instead: the connector name, refined by the
//! EDID where the connector has one.
//!
//! - The name (`DP-1`, `eDP-1`) is always there: it is the `wl_output` name
//!   every backend sets, so headless and nested outputs (and every test)
//!   identify by name alone.
//! - The EDID summary (make, product, serial, manufacture week/year) tells a
//!   *different* monitor on the same connector apart from the one that left:
//!   swapping the monitor on `DP-1` must not inherit the old one's
//!   workspaces. It is `None` where there is no EDID to read -- a panel with
//!   no serial, a KVM hiding the blob -- and then the name alone decides,
//!   exactly as it does on the connector-less backends.

/// A monitor's EDID summary: the fields that identify the physical panel, in
/// the byte order the EDID stores them (the product code and serial are
/// little-endian on the wire, the manufacturer big-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdidIdentity {
    /// Raw bytes 8-9: three 5-bit manufacturer characters.
    pub manufacturer: u16,
    /// Little-endian bytes 10-11: the manufacturer's product code.
    pub product: u16,
    /// Little-endian bytes 12-15: the unit's serial number.
    pub serial: u32,
    /// Byte 16 (week of manufacture, `0`/`255` mean "not given") and byte 17
    /// (years since 1990).
    pub week: u8,
    pub year: u8,
}

/// Which monitor an output is: the connector name plus the EDID summary
/// where one could be read. Equality is strict on both halves -- a record
/// filed with an EDID does not match a connector that now answers none (a
/// KVM stepped in between), and a name-only record does not match a
/// different connector. Either mismatch leaves the windows where they are,
/// which is the safe direction: a missed restore is one move away, while a
/// wrong one undoes the user's arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OutputIdentity {
    /// The connector name (`DP-1`), or the `wl_output` name where there is
    /// no connector (`headless`, `headless-2`, ...).
    pub name: String,
    /// The EDID summary, where the connector had an EDID blob to read.
    pub edid: Option<EdidIdentity>,
}

impl OutputIdentity {
    /// An identity with no EDID: panels without one, KVMs hiding it, and
    /// every connector-less backend.
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            edid: None,
        }
    }
}

/// Reads the identity summary off a raw EDID base block: `None` for anything
/// that is not one (too short, wrong magic). Only the first 18 bytes are
/// touched, so a blob with extension blocks parses the same as without.
pub fn parse_edid(bytes: &[u8]) -> Option<EdidIdentity> {
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if bytes.len() < 18 || bytes[0..8] != HEADER {
        return None;
    }
    Some(EdidIdentity {
        manufacturer: u16::from_be_bytes([bytes[8], bytes[9]]),
        product: u16::from_le_bytes([bytes[10], bytes[11]]),
        serial: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        week: bytes[16],
        year: bytes[17],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic 128-byte base block: header, manufacturer `ABC`
    /// (`0x0441`), product `0x1234`, serial `0x56789ABC`, week 12 of 2021
    /// (year byte 31), zero descriptor payload.
    fn block() -> Vec<u8> {
        let mut edid = vec![0u8; 128];
        edid[0..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
        edid[8..10].copy_from_slice(&[0x04, 0x41]);
        edid[10..12].copy_from_slice(&[0x34, 0x12]);
        edid[12..16].copy_from_slice(&[0xBC, 0x9A, 0x78, 0x56]);
        edid[16] = 12;
        edid[17] = 31;
        edid[127] = 0x4E; // Checksum-looking filler; not verified.
        edid
    }

    #[test]
    fn a_base_block_parses_to_its_summary() {
        assert_eq!(
            parse_edid(&block()),
            Some(EdidIdentity {
                manufacturer: 0x0441,
                product: 0x1234,
                serial: 0x5678_9ABC,
                week: 12,
                year: 31,
            })
        );
    }

    #[test]
    fn a_short_blob_or_a_wrong_magic_is_no_edid() {
        assert_eq!(parse_edid(&[]), None);
        assert_eq!(parse_edid(&block()[..17]), None);
        let mut bad = block();
        bad[0] = 0x01;
        assert_eq!(parse_edid(&bad), None);
    }

    #[test]
    fn identity_equality_needs_both_halves() {
        let full = OutputIdentity {
            name: "DP-1".to_owned(),
            edid: parse_edid(&block()),
        };
        // A different monitor on the same connector: same name, other serial.
        let mut other_blob = block();
        other_blob[12] ^= 0xFF;
        let swapped = OutputIdentity {
            name: "DP-1".to_owned(),
            edid: parse_edid(&other_blob),
        };
        assert_ne!(full, swapped);
        // The same monitor answering no EDID (a KVM hid it): no match.
        assert_ne!(full, OutputIdentity::named("DP-1"));
        // And a different connector never matches, EDID or not.
        assert_ne!(
            full,
            OutputIdentity {
                name: "DP-2".to_owned(),
                edid: full.edid,
            }
        );
        assert_eq!(full.clone(), full);
    }
}
