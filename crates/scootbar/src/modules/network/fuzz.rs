//! What the fuzz target checks, written once: `crates/scootbar/fuzz`
//! compiles this file by `#[path]` (so it uses nothing but
//! [`super::netlink`], as that uses nothing but `std`), and the stable test
//! beside it replays the committed seed corpus and every past finding
//! through the same function on every `cargo test`, without nightly or
//! `cargo-fuzz`. A panic is a finding: the bar's release profile is
//! `panic = "abort"`.

use super::netlink;

#[cfg(test)]
mod tests;

/// Any bytes as netlink datagrams: framing accepts or stops without a
/// panic, and every message and attribute parser accepts or refuses
/// without one. Whatever parses is then shaped the way the module shapes
/// it (sanitized names, bars, request encoding), which must not panic
/// either.
pub fn network(data: &[u8]) {
    // Every prefix parses like the module's drain does: complete messages
    // apply, a truncated one ends the walk.
    for end in 0..=data.len() {
        let prefix = &data[..end];
        for msg in netlink::messages(prefix) {
            check_message(msg.kind, msg.body);
        }
        for attr in netlink::attrs(prefix) {
            let _ = (attr.kind, attr.payload.len());
        }
    }
    // The request builders, on input-derived values.
    let mut out = Vec::new();
    let seq = u32::from_ne_bytes(
        data.first()
            .copied()
            .map(|b| [b, 0, 0, 0])
            .unwrap_or([0; 4]),
    );
    netlink::link_dump(&mut out, seq);
    netlink::addr_dump(&mut out, seq, data.first().copied().unwrap_or(0));
    netlink::route_dump(&mut out, seq, data.first().copied().unwrap_or(0));
    netlink::family_request(&mut out, seq, &data[..data.len().min(16)]);
    netlink::interface_request(&mut out, 30, seq, seq);
    netlink::scan_request(&mut out, 30, seq, seq);
    netlink::station_request(&mut out, 30, seq, seq);
    let thresholds = [data.first().copied().map(|b| b as i8).unwrap_or(-70)];
    netlink::cqm_request(&mut out, 30, seq, seq, &thresholds);
    // And the shaping, on an input-derived signal and SSID.
    let _ = netlink::bars_for(data.first().copied().map(|b| b as i8).unwrap_or(0));
    let _ = netlink::sanitize(data);
}

fn check_message(kind: u16, body: &[u8]) {
    match kind {
        netlink::RTM_NEWLINK | netlink::RTM_DELLINK => {
            let mut link = netlink::Link::default();
            netlink::parse_link(body, &mut link);
            let _ = (link.name_str(), link.kind_str());
            let _ = link.is_up() || link.is_loopback() || link.is_tunnel() || link.is_empty();
        }
        netlink::RTM_NEWADDR | netlink::RTM_DELADDR => {
            let _ = netlink::parse_addr(body);
        }
        netlink::RTM_NEWROUTE | netlink::RTM_DELROUTE => {
            let _ = netlink::parse_route(body, true);
        }
        netlink::NLMSG_ERROR => {
            let _ = netlink::error_of(body);
        }
        _ => {
            if let Some((command, fields)) = netlink::genl_of(body) {
                match command {
                    netlink::NL80211_CMD_GET_SCAN => {
                        let mut out = [netlink::Bss::default(); 4];
                        let n = netlink::fold_scan(body, &mut out);
                        for bss in &out[..n] {
                            let _ = bss.ssid_str();
                        }
                    }
                    netlink::NL80211_CMD_GET_STATION => {
                        let _ = netlink::parse_station(body);
                    }
                    _ => {
                        let _ = netlink::parse_family(body);
                        let _ = netlink::parse_interface(fields);
                        let mut bss = netlink::Bss::default();
                        netlink::parse_bss(fields, &mut bss);
                        let _ = netlink::find_u32(fields, netlink::NL80211_ATTR_IFINDEX);
                    }
                }
            }
        }
    }
}
