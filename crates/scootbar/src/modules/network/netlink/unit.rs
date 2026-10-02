//! The wire, byte by byte. `struct ifinfomsg` is native-endian throughout
//! (family, pad, then `__u16 ifi_type`), so every multi-byte fixture uses
//! `to_ne_bytes` — a `to_be_bytes` fixture would decode to nonsense on
//! little-endian and pass through the malformed path instead of the parse
//! (the volume module's `scan_names` test did exactly that; see
//! `docs/scootbar/backlog/volume-scan-names-test.md`). The live
//! `against_a_real_rtnetlink` pins the type against the real kernel's `lo`.

use super::*;

fn attr_of(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut attr = Vec::new();
    let len = (payload.len() + 4) as u16;
    attr.extend_from_slice(&len.to_ne_bytes());
    attr.extend_from_slice(&kind.to_ne_bytes());
    attr.extend_from_slice(payload);
    while attr.len() % 4 != 0 {
        attr.push(0);
    }
    attr
}

fn named(name: &[u8]) -> Vec<u8> {
    let mut payload = name.to_vec();
    payload.push(0);
    attr_of(IFLA_IFNAME, &payload)
}

fn msg(kind: u16, seq: u32, body: &[u8]) -> Vec<u8> {
    let mut datagram = Vec::new();
    let start = request(&mut datagram, kind, seq, false);
    // `request` sets ACK; a canned event carries no flags.
    datagram[start + 6..start + 8].copy_from_slice(&0u16.to_ne_bytes());
    datagram.extend_from_slice(body);
    finish(&mut datagram, start);
    datagram
}

fn link_body(index: u32, flags: u32, oper: u8, name: &[u8], kind: Option<&[u8]>) -> Vec<u8> {
    let mut body = vec![0u8; 16];
    body[2..4].copy_from_slice(&1u16.to_ne_bytes());
    body[4..8].copy_from_slice(&index.to_ne_bytes());
    body[8..12].copy_from_slice(&flags.to_ne_bytes());
    body.extend_from_slice(&named(name));
    body.extend_from_slice(&attr_of(IFLA_OPERSTATE, &[oper]));
    if let Some(kind) = kind {
        let mut info = attr_of(IFLA_INFO_KIND, kind);
        let mut linkinfo = Vec::new();
        let start = linkinfo.len();
        linkinfo.extend_from_slice(&[0u8; 4]);
        linkinfo.append(&mut info);
        let len = linkinfo.len() as u16;
        linkinfo[start..start + 2].copy_from_slice(&len.to_ne_bytes());
        linkinfo[start + 2..start + 4]
            .copy_from_slice(&(IFLA_LINKINFO | NLA_F_NESTED).to_ne_bytes());
        // The outer kind carries the NESTED bit; the inner must not.
        body.extend_from_slice(&linkinfo);
    }
    body
}

#[test]
fn attributes_stop_at_truncation() {
    assert_eq!(attrs(&[]).count(), 0);
    assert_eq!(attrs(&[0u8; 3]).count(), 0);
    // A length past the end ends the walk, silently.
    let mut two = attr_of(3, b"eth0\0");
    two.extend_from_slice(&attr_of(16, &[6]));
    two.truncate(two.len() - 5);
    let kinds: Vec<u16> = attrs(&two).map(|attr| attr.kind).collect();
    assert_eq!(kinds, [3]);
    // The NESTED bit is masked off the kind.
    let nested = attr_of(NLA_F_NESTED | 18, b"x");
    let kinds: Vec<u16> = attrs(&nested).map(|attr| attr.kind).collect();
    assert_eq!(kinds, [18]);
}

#[test]
fn messages_stop_at_truncation() {
    let mut datagram = msg(16, 7, b"abcdefgh");
    datagram.extend_from_slice(&msg(17, 7, b"xy"));
    let kinds: Vec<u16> = messages(&datagram).map(|msg| msg.kind).collect();
    assert_eq!(kinds, [16, 17]);
    datagram.truncate(datagram.len() - 1);
    let kinds: Vec<u16> = messages(&datagram).map(|msg| msg.kind).collect();
    assert_eq!(kinds, [16]);
    assert_eq!(messages(&[0u8; 15]).count(), 0);
}

#[test]
fn a_link_parses_name_flags_oper_and_kind() {
    let body = link_body(3, IFF_UP, IF_OPER_UP, b"wlan0", Some(b"nl80211"));
    let mut link = Link::default();
    parse_link(&body, &mut link);
    assert_eq!(link.index, 3);
    assert_eq!(link.name_str(), "wlan0");
    assert_eq!(link.arp, 1, "the type sits at body[2..4], native-endian");
    assert!(link.is_up());
    assert!(!link.is_loopback());
    assert!(!link.is_tunnel());
    assert_eq!(link.kind_str(), "nl80211");
    // Truncated: nothing is read, and the link reads unusable.
    let mut short = Link::default();
    parse_link(&body[..10], &mut short);
    assert!(short.is_empty());
}

#[test]
fn a_tunnel_reads_as_vpn() {
    // A TUN device: headerless, no linkinfo.
    let mut body = vec![0u8; 16];
    body[2..4].copy_from_slice(&ARPHRD_NONE.to_ne_bytes());
    body[4..8].copy_from_slice(&9u32.to_ne_bytes());
    body[8..12].copy_from_slice(&IFF_UP.to_ne_bytes());
    body.extend_from_slice(&named(b"tun0"));
    let mut link = Link::default();
    parse_link(&body, &mut link);
    assert!(link.is_tunnel());
    assert!(link.is_up());
    // WireGuard says so by kind.
    let body = link_body(10, IFF_UP, IF_OPER_UP, b"wg0", Some(b"wireguard"));
    let mut link = Link::default();
    parse_link(&body, &mut link);
    assert!(link.is_tunnel());
    // SIT (IPv6-in-IPv4) is one by type.
    let mut body = vec![0u8; 16];
    body[2..4].copy_from_slice(&ARPHRD_SIT.to_ne_bytes());
    body[4..8].copy_from_slice(&12u32.to_ne_bytes());
    body[8..12].copy_from_slice(&IFF_UP.to_ne_bytes());
    body.extend_from_slice(&named(b"sit0"));
    let mut link = Link::default();
    parse_link(&body, &mut link);
    assert!(link.is_tunnel());
    // A name past 16 bytes is cut, never grown.
    let body = link_body(11, IFF_UP, IF_OPER_UP, b"averylonginterfacename", None);
    let mut link = Link::default();
    parse_link(&body, &mut link);
    assert_eq!(link.name_len, 16);
}

#[test]
fn an_address_parses_index_and_family() {
    let mut body = vec![2u8, 64, 0, 0];
    body.extend_from_slice(&5u32.to_ne_bytes());
    let event = parse_addr(&body).expect("an address");
    assert_eq!((event.index, event.family), (5, 2));
    assert!(parse_addr(&body[..7]).is_none());
}

#[test]
fn only_a_default_route_with_an_oif_counts() {
    let mut body = vec![2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    body.extend_from_slice(&attr_of(RTA_OIF, &3u32.to_ne_bytes()));
    let event = parse_route(&body, true).expect("a default route");
    assert_eq!((event.oif, event.family), (3, 2));
    // A /24 is not the default route.
    let mut lan = body.clone();
    lan[1] = 24;
    assert!(parse_route(&lan, true).is_none());
    // No output interface: unusable.
    assert!(parse_route(&body[..12], true).is_none());
    assert!(parse_route(&body[..11], true).is_none());
}

#[test]
fn dump_requests_carry_kind_and_family() {
    let mut out = Vec::new();
    link_dump(&mut out, 11);
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].kind, RTM_GETLINK);
    assert_eq!(msgs[0].seq, 11);
    assert_eq!(msgs[0].body.len(), 16);
    let mut out = Vec::new();
    route_dump(&mut out, 12, 10);
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs[0].kind, RTM_GETROUTE);
    assert_eq!(msgs[0].body[0], 10);
    let mut out = Vec::new();
    addr_dump(&mut out, 13, 10);
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs[0].kind, RTM_GETADDR);
    assert_eq!(msgs[0].body[0], 10);
    assert_eq!(msgs[0].body.len(), 8);
}

#[test]
fn a_family_reply_carries_its_groups() {
    let mut out = Vec::new();
    family_request(&mut out, 99, b"nl80211");
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs.len(), 1);
    let (command, _) = genl_of(msgs[0].body).expect("generic");
    assert_eq!(command, CTRL_CMD_GETFAMILY);
    // A canned reply: id 30, `scan` 31, `mlme` 32, and one unknown group.
    let mut reply = attr_of(CTRL_ATTR_FAMILY_ID, &30u16.to_ne_bytes());
    let mut groups = Vec::new();
    for (name, id) in [("scan", 31u32), ("mlme", 32u32), ("vendor", 33u32)] {
        // Names travel NUL-terminated; the match is without.
        let mut terminated = name.as_bytes().to_vec();
        terminated.push(0);
        let mut entry = attr_of(CTRL_ATTR_MCAST_GRP_NAME, &terminated);
        entry.extend_from_slice(&attr_of(CTRL_ATTR_MCAST_GRP_ID, &id.to_ne_bytes()));
        let mut nest = vec![0u8; 4];
        nest.append(&mut entry);
        let len = nest.len() as u16;
        nest[0..2].copy_from_slice(&len.to_ne_bytes());
        groups.append(&mut nest);
    }
    let mut nest = vec![0u8; 4];
    nest.append(&mut groups);
    let len = nest.len() as u16;
    nest[0..2].copy_from_slice(&len.to_ne_bytes());
    nest[2..4].copy_from_slice(&(CTRL_ATTR_MCAST_GROUPS | NLA_F_NESTED).to_ne_bytes());
    reply.append(&mut nest);
    let mut body = vec![CTRL_CMD_GETFAMILY, 1, 0, 0];
    body.append(&mut reply);
    let family = parse_family(&body);
    assert_eq!(family.id, 30);
    assert_eq!((family.scan, family.mlme), (31, 32));
    assert_eq!(parse_family(&[]), Family::default());
}

#[test]
fn an_interface_parses_index_and_name() {
    let mut out = Vec::new();
    interface_request(&mut out, 30, 5, 0);
    let msgs: Vec<Message> = messages(&out).collect();
    let (command, _) = genl_of(msgs[0].body).expect("generic");
    assert_eq!(command, NL80211_CMD_GET_INTERFACE);
    // A canned notice for wlan0, NUL-terminated with its SSID, as the
    // kernel sends it while associated.
    let mut body = vec![NL80211_CMD_NEW_INTERFACE, 1, 0, 0];
    body.extend_from_slice(&attr_of(NL80211_ATTR_IFINDEX, &3u32.to_ne_bytes()));
    body.extend_from_slice(&attr_of(NL80211_ATTR_IFNAME, b"wlan0\0"));
    body.extend_from_slice(&attr_of(NL80211_ATTR_SSID, b"Wimbly"));
    let wifi = parse_interface(&body);
    assert_eq!(wifi.index, 3);
    assert_eq!(&wifi.name[..wifi.name_len as usize], b"wlan0");
    assert_eq!(&wifi.ssid[..wifi.ssid_len as usize], b"Wimbly");
    assert_eq!(parse_interface(&[]), Wireless::default());
}

#[test]
fn a_scan_folds_the_associated_bss_first() {
    fn bss(ssid: Option<&[u8]>, signal_mbm: i32, status: Option<u32>) -> Vec<u8> {
        let mut nest = Vec::new();
        if let Some(ssid) = ssid {
            let mut ie = vec![0u8, ssid.len() as u8];
            ie.extend_from_slice(ssid);
            nest.extend_from_slice(&attr_of(NL80211_BSS_INFORMATION_ELEMENTS, &ie));
        }
        nest.extend_from_slice(&attr_of(NL80211_BSS_SIGNAL_MBM, &signal_mbm.to_ne_bytes()));
        if let Some(status) = status {
            nest.extend_from_slice(&attr_of(NL80211_BSS_STATUS, &status.to_ne_bytes()));
        }
        let mut outer = vec![0u8; 4];
        outer.append(&mut nest);
        let len = outer.len() as u16;
        outer[0..2].copy_from_slice(&len.to_ne_bytes());
        outer[2..4].copy_from_slice(&(NL80211_ATTR_BSS | NLA_F_NESTED).to_ne_bytes());
        outer
    }
    let mut body = vec![NL80211_CMD_GET_SCAN, 1, 0, 0];
    body.extend_from_slice(&bss(Some(b"Other"), -7000, None));
    body.extend_from_slice(&bss(Some(b"Wimbly"), -5400, Some(NL80211_BSS_ASSOCIATED)));
    body.extend_from_slice(&bss(None, -8000, None));
    let mut out = [Bss::default(), Bss::default()];
    // Two slots: the third BSS is dropped, never grown into.
    let count = fold_scan(&body, &mut out);
    assert_eq!(count, 2);
    assert_eq!(out[0].ssid_str(), "Wimbly");
    assert!(out[0].associated);
    assert_eq!(out[0].signal, Some(-54));
    assert_eq!(out[1].ssid_str(), "Other");
    // A hidden SSID is present but unnamed.
    let mut body = vec![NL80211_CMD_GET_SCAN, 1, 0, 0];
    body.extend_from_slice(&bss(Some(b""), -8000, None));
    let mut out = [Bss::default()];
    assert_eq!(fold_scan(&body, &mut out), 1);
    assert_eq!(out[0].ssid_len, 0);
    // A truncated IE does not read past the end.
    let mut body = vec![NL80211_CMD_GET_SCAN, 1, 0, 0];
    body.extend_from_slice(&bss(
        Some(b"Waytoolongforthisbufferisnotreally"),
        -8000,
        None,
    ));
    let mut out = [Bss::default()];
    assert_eq!(fold_scan(&body, &mut out), 1);
    assert!(out[0].ssid_len <= 32);
}

#[test]
fn a_station_reply_carries_the_signal() {
    let mut out = Vec::new();
    station_request(&mut out, 30, 6, 3);
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].flags & NLM_F_DUMP, NLM_F_DUMP);
    let (command, _) = genl_of(msgs[0].body).expect("generic");
    assert_eq!(command, NL80211_CMD_GET_STATION);
    let mut info = vec![0u8; 4];
    info[0..2].copy_from_slice(&5u16.to_ne_bytes());
    info[2..4].copy_from_slice(&NL80211_STA_INFO_SIGNAL.to_ne_bytes());
    info.push(-60i8 as u8);
    info.extend_from_slice(&[0u8; 3]);
    let mut nest = vec![0u8; 4];
    nest.append(&mut info);
    let len = nest.len() as u16;
    nest[0..2].copy_from_slice(&len.to_ne_bytes());
    nest[2..4].copy_from_slice(&(NL80211_ATTR_STA_INFO | NLA_F_NESTED).to_ne_bytes());
    let mut body = vec![NL80211_CMD_GET_STATION, 1, 0, 0];
    body.append(&mut nest);
    assert_eq!(parse_station(&body), Some(-60));
    assert_eq!(parse_station(&[NL80211_CMD_GET_STATION, 1, 0, 0]), None);
}

#[test]
fn the_cqm_probe_is_shaped_like_the_kernel_reads() {
    let mut out = Vec::new();
    cqm_request(&mut out, 30, 7, 3, &[-70, -80]);
    let msgs: Vec<Message> = messages(&out).collect();
    assert_eq!(msgs.len(), 1);
    let (command, fields) = genl_of(msgs[0].body).expect("generic");
    assert_eq!(command, NL80211_CMD_SET_CQM);
    let attrs: Vec<Attr> = attrs(fields).collect();
    assert!(attrs.iter().any(|attr| attr.kind == NL80211_ATTR_IFINDEX));
    let nested = attrs
        .iter()
        .find(|attr| attr.kind == NL80211_ATTR_CQM)
        .expect("the CQM nest");
    let inner: Vec<u16> = super::attrs(nested.payload).map(|a| a.kind).collect();
    assert_eq!(inner, [NL80211_CQM_RSSI_THOLD, NL80211_CQM_RSSI_THOLD, 2]);
}

#[test]
fn bars_follow_the_measured_thresholds() {
    assert_eq!(bars_for(-40), 4);
    assert_eq!(bars_for(-55), 4);
    assert_eq!(bars_for(-56), 3);
    assert_eq!(bars_for(-67), 3);
    assert_eq!(bars_for(-68), 2);
    assert_eq!(bars_for(-77), 2);
    assert_eq!(bars_for(-78), 1);
    assert_eq!(bars_for(-100), 1);
}

#[test]
fn an_error_ack_reads_its_errno() {
    assert_eq!(error_of(&[]), None);
    assert_eq!(error_of(&0i32.to_ne_bytes()), None);
    assert_eq!(error_of(&(-2i32).to_ne_bytes()), Some(-2));
}
