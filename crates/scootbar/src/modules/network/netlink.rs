//! Netlink framing and the rtnetlink/nl80211 messages the network module
//! speaks: built here, parsed here, with no I/O. Every constant is the
//! kernel ABI (`linux-headers-7.1` on the dev VM, `include/linux/nl80211.h`,
//! `rtnetlink.h`, `if_link.h`, `if_arp.h`, `genetlink.h`, `netlink.h`);
//! nothing is guessed from userspace tools.
//!
//! Two sockets speak this: `NETLINK_ROUTE` for links, addresses and the
//! default route, and `NETLINK_GENERIC` for nl80211 (the WiFi interface
//! list, the current BSS and the signal). Requests carry a sequence number
//! the replies echo; multicast is joined at `bind` time, so a generic
//! group whose id does not fit the 32-bit mask is refused at start-up
//! rather than silently missed (rustix offers no `NETLINK_ADD_MEMBERSHIP`).
//!
//! Every constant is the kernel ABI it is, checked by compiling a probe
//! against the headers (`linux/nl80211.h`, `rtnetlink.h`, `if_link.h`,
//! `if_arp.h`, `genetlink.h`, `netlink.h`): counting an enum by eye got
//! the nl80211 commands wrong by two, and the compiler caught it.

/// A netlink message header: length, type, flags, sequence, port.
pub const HEADER_LEN: usize = 16;

/// The most datagrams handled per ready turn: a flapping link is a bounded
/// resync, not an unbounded storm.
pub const MAX_DATAGRAMS: usize = 32;

/// The read buffer: the largest dump seen (a full scan on a crowded
/// street) fits with room; past it the socket resyncs.
pub const READ_LEN: usize = 65536;

// Message types.
pub const NLMSG_DONE: u16 = 3;
pub const NLMSG_ERROR: u16 = 2;

// Request flags.
const NLM_F_REQUEST: u16 = 0x01;
const NLM_F_ACK: u16 = 0x04;
/// The dump flag, for matching a reply's terminator to its request (and
/// for the tests, to tell dump requests from singles).
pub const NLM_F_DUMP: u16 = 0x300;

// rtnetlink types.
pub const RTM_NEWLINK: u16 = 16;
pub const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
pub const RTM_NEWADDR: u16 = 20;
pub const RTM_DELADDR: u16 = 21;
const RTM_GETADDR: u16 = 22;
pub const RTM_NEWROUTE: u16 = 24;
pub const RTM_DELROUTE: u16 = 25;
const RTM_GETROUTE: u16 = 26;

// rtnetlink multicast groups, for the `bind` mask. All below 32.
pub const RTMGRP_LINK: u32 = 1;
pub const RTMGRP_IPV4_IFADDR: u32 = 0x10;
pub const RTMGRP_IPV4_ROUTE: u32 = 0x40;
pub const RTMGRP_IPV6_IFADDR: u32 = 0x100;
pub const RTMGRP_IPV6_ROUTE: u32 = 0x400;

// Interface attributes.
const IFLA_IFNAME: u16 = 3;
const IFLA_OPERSTATE: u16 = 16;
const IFLA_LINKINFO: u16 = 18;
const IFLA_INFO_KIND: u16 = 1;

// Interface flags and hardware types (`ARPHRD_*`, native-endian on the
// wire; `ARPHRD_LOOPBACK` is public for the live test that pins it).
const IFF_UP: u32 = 0x1;
const IFF_LOOPBACK: u32 = 0x8;
const ARPHRD_PPP: u16 = 512;
pub const ARPHRD_LOOPBACK: u16 = 772;
const ARPHRD_NONE: u16 = 0xfffe;

// Operational states.
const IF_OPER_DOWN: u8 = 2;
/// `IF_OPER_UP`, for the fixtures: production only tests for down.
#[cfg(test)]
pub const IF_OPER_UP: u8 = 6;

// Address and route attributes.
const RTA_OIF: u16 = 4;

// Generic netlink.
pub const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const CTRL_ATTR_MCAST_GROUPS: u16 = 7;
const CTRL_ATTR_MCAST_GRP_NAME: u16 = 1;
const CTRL_ATTR_MCAST_GRP_ID: u16 = 2;

// nl80211 commands.
pub const NL80211_CMD_GET_INTERFACE: u8 = 5;
pub const NL80211_CMD_NEW_INTERFACE: u8 = 7;
pub const NL80211_CMD_DEL_INTERFACE: u8 = 8;
pub const NL80211_CMD_GET_STATION: u8 = 17;
/// A station dump's entries arrive as this, not as `GET_STATION`.
pub const NL80211_CMD_NEW_STATION: u8 = 19;
pub const NL80211_CMD_GET_SCAN: u8 = 32;
/// A scan dump's entries arrive as this, not as `GET_SCAN`. A bare one
/// (sequence zero) is the kernel's completion notice instead.
pub const NL80211_CMD_NEW_SCAN_RESULTS: u8 = 34;
pub const NL80211_CMD_ASSOCIATE: u8 = 38;
pub const NL80211_CMD_DEAUTHENTICATE: u8 = 39;
pub const NL80211_CMD_DISASSOCIATE: u8 = 40;
pub const NL80211_CMD_ROAM: u8 = 47;
pub const NL80211_CMD_SET_CQM: u8 = 63;

// nl80211 attributes.
pub const NL80211_ATTR_IFINDEX: u16 = 3;
pub use NL80211_ATTR_IFINDEX as ATTR_IFINDEX;
const NL80211_ATTR_IFNAME: u16 = 4;
const NL80211_ATTR_STA_INFO: u16 = 21;
const NL80211_ATTR_BSS: u16 = 47;
const NL80211_ATTR_SSID: u16 = 52;
const NL80211_ATTR_CQM: u16 = 94;
const NL80211_STA_INFO_SIGNAL: u16 = 7;
const NL80211_BSS_INFORMATION_ELEMENTS: u16 = 6;
const NL80211_BSS_SIGNAL_MBM: u16 = 7;
const NL80211_BSS_STATUS: u16 = 9;
const NL80211_BSS_ASSOCIATED: u32 = 1;
const NL80211_CQM_RSSI_THOLD: u16 = 1;
const NL80211_CQM_RSSI_HYST: u16 = 2;

// Attribute framing: 4-byte aligned, the high bit marks nesting.
const NLA_F_NESTED: u16 = 0x8000;
const NLA_TYPE_MASK: u16 = 0x3fff;

/// A parsed netlink attribute: its type and raw payload.
pub struct Attr<'a> {
    pub kind: u16,
    pub payload: &'a [u8],
}

/// Attributes in `bytes`: stops at the first truncated one, which the
/// callers treat as the end (a partial datagram is re-read whole next
/// turn, never half-parsed).
pub fn attrs(mut bytes: &[u8]) -> impl Iterator<Item = Attr<'_>> {
    std::iter::from_fn(move || {
        if bytes.len() < 4 {
            return None;
        }
        let len = u16::from_ne_bytes([bytes[0], bytes[1]]) as usize;
        let kind = u16::from_ne_bytes([bytes[2], bytes[3]]) & NLA_TYPE_MASK;
        if len < 4 || len > bytes.len() {
            bytes = &[];
            return None;
        }
        let payload = &bytes[4..len];
        let aligned = len.next_multiple_of(4);
        bytes = &bytes[aligned.min(bytes.len())..];
        Some(Attr { kind, payload })
    })
}

/// A parsed netlink message: its header and body.
pub struct Message<'a> {
    pub kind: u16,
    /// The flags (the dump bit tells a request's terminator from noise);
    /// read by the tests, which match requests to replies.
    #[allow(dead_code)]
    pub flags: u16,
    pub seq: u32,
    pub body: &'a [u8],
}

/// Messages in one datagram: like [`attrs`], stops at truncation.
pub fn messages(mut bytes: &[u8]) -> impl Iterator<Item = Message<'_>> {
    std::iter::from_fn(move || {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let len = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let kind = u16::from_ne_bytes([bytes[4], bytes[5]]);
        let flags = u16::from_ne_bytes([bytes[6], bytes[7]]);
        let seq = u32::from_ne_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if len < HEADER_LEN || len > bytes.len() {
            bytes = &[];
            return None;
        }
        let body = &bytes[HEADER_LEN..len];
        let aligned = len.next_multiple_of(4);
        bytes = &bytes[aligned.min(bytes.len())..];
        Some(Message {
            kind,
            flags,
            seq,
            body,
        })
    })
}

/// Writes one request header into `out`, then the caller's body. Dumps
/// carry no `ACK`: like iproute2, the `DONE` terminator is the answer.
pub fn request(out: &mut Vec<u8>, kind: u16, seq: u32, dump: bool) -> usize {
    let start = out.len();
    out.extend_from_slice(&[0u8; HEADER_LEN]);
    let mut flags = NLM_F_REQUEST;
    if dump {
        flags |= NLM_F_DUMP;
    } else {
        flags |= NLM_F_ACK;
    }
    out[start + 4..start + 6].copy_from_slice(&kind.to_ne_bytes());
    out[start + 6..start + 8].copy_from_slice(&flags.to_ne_bytes());
    out[start + 8..start + 12].copy_from_slice(&seq.to_ne_bytes());
    start
}

/// Finishes the request started at `start`: stamps its length.
pub fn finish(out: &mut [u8], start: usize) {
    let len = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&len.to_ne_bytes());
}

/// The u32 value of attribute `kind` in `bytes`, or 0.
pub fn find_u32(bytes: &[u8], kind: u16) -> u32 {
    for attr in attrs(bytes) {
        if attr.kind == kind && attr.payload.len() >= 4 {
            return u32::from_ne_bytes([
                attr.payload[0],
                attr.payload[1],
                attr.payload[2],
                attr.payload[3],
            ]);
        }
    }
    0
}

/// The `NLMSG_ERROR` payload's errno, or `None` for the success ack (a
/// zero errno with no further bytes).
pub fn error_of(body: &[u8]) -> Option<i32> {
    if body.len() < 4 {
        return None;
    }
    let errno = i32::from_ne_bytes([body[0], body[1], body[2], body[3]]);
    if errno == 0 { None } else { Some(errno) }
}

/// A link: what `RTM_NEWLINK` carries and what the module keeps per
/// interface. `wireless` and the linkinfo `kind` come from nl80211 and the
/// `IFLA_LINKINFO` nest, not guessed from the name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Link {
    pub index: u32,
    pub name: [u8; 16],
    pub name_len: u8,
    pub arp: u16,
    pub flags: u32,
    pub oper: u8,
    pub kind: [u8; 16],
    pub kind_len: u8,
}

impl Link {
    pub fn name_str(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    pub fn kind_str(&self) -> &str {
        std::str::from_utf8(&self.kind[..self.kind_len as usize]).unwrap_or("")
    }

    pub fn is_loopback(&self) -> bool {
        self.flags & IFF_LOOPBACK != 0 || self.arp == ARPHRD_LOOPBACK
    }

    pub fn is_up(&self) -> bool {
        self.flags & IFF_UP != 0 && self.oper != IF_OPER_DOWN
    }

    /// A tunnel of some kind: WireGuard says so, anything headerless
    /// (`tun`, `tap`, `sit`, Tailscale's) is one by construction, and PPP
    /// is one by type. What the module shows as VPN.
    pub fn is_tunnel(&self) -> bool {
        self.kind_str() == "wireguard" || self.arp == ARPHRD_NONE || self.arp == ARPHRD_PPP
    }

    /// Whether the kernel refused to parse this far: an empty name never
    /// happens on a real link.
    pub fn is_empty(&self) -> bool {
        self.name_len == 0
    }
}

/// Parses `RTM_NEWLINK`/`RTM_DELLINK`'s body into `link`. Unknown
/// attributes are skipped; a truncated body leaves what parsed (the name
/// may be empty, which the module reads as unusable).
pub fn parse_link(body: &[u8], link: &mut Link) {
    if body.len() < 16 {
        return;
    }
    // `struct ifinfomsg` is native-endian throughout (`__u16 ifi_type`):
    // the index and flags are too. Verified against the wire (lo's type
    // arrives `04 03`, i.e. 772 little-endian).
    link.arp = u16::from_ne_bytes([body[2], body[3]]);
    link.index = u32::from_ne_bytes([body[4], body[5], body[6], body[7]]);
    link.flags = u32::from_ne_bytes([body[8], body[9], body[10], body[11]]);
    // `ifi_change` follows the flags; the attributes start at 16 bytes in.
    for attr in attrs(&body[16..]) {
        match attr.kind {
            IFLA_IFNAME => put_bytes(&mut link.name, &mut link.name_len, attr.payload),
            IFLA_OPERSTATE => {
                if let Some(state) = attr.payload.first() {
                    link.oper = *state;
                }
            }
            IFLA_LINKINFO => {
                for inner in attrs(attr.payload) {
                    if inner.kind == IFLA_INFO_KIND {
                        put_bytes(&mut link.kind, &mut link.kind_len, inner.payload);
                    }
                }
            }
            _ => {}
        }
    }
}

fn put_bytes(slot: &mut [u8; 16], len: &mut u8, bytes: &[u8]) {
    // A `IFLA_IFNAME` payload is NUL-terminated; the kind is not.
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let take = end.min(slot.len());
    slot[..take].copy_from_slice(&bytes[..take]);
    *len = take as u8;
}

/// An address event: the interface index and family of an
/// `RTM_NEWADDR`/`RTM_DELADDR`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddrEvent {
    pub index: u32,
    pub family: u8,
}

/// Parses `RTM_NEWADDR`/`RTM_DELADDR`'s body. `None` on truncation.
pub fn parse_addr(body: &[u8]) -> Option<AddrEvent> {
    if body.len() < 8 {
        return None;
    }
    let index = u32::from_ne_bytes([body[4], body[5], body[6], body[7]]);
    Some(AddrEvent {
        index,
        family: body[0],
    })
}

/// A route event: whether the default route (empty destination) gained or
/// lost an output interface, and which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteEvent {
    pub present: bool,
    pub oif: u32,
    pub family: u8,
}

/// Parses `RTM_NEWROUTE`/`RTM_DELROUTE`'s body. `None` for a non-default
/// route or a truncated body.
pub fn parse_route(body: &[u8], present: bool) -> Option<RouteEvent> {
    if body.len() < 12 || body[1] != 0 {
        return None;
    }
    let family = body[0];
    let mut oif = 0;
    for attr in attrs(&body[12..]) {
        if attr.kind == RTA_OIF && attr.payload.len() >= 4 {
            oif = u32::from_ne_bytes([
                attr.payload[0],
                attr.payload[1],
                attr.payload[2],
                attr.payload[3],
            ]);
        }
    }
    if oif == 0 {
        return None;
    }
    Some(RouteEvent {
        present,
        oif,
        family,
    })
}

/// Builds a `GETLINK`/`GETADDR`/`GETROUTE` dump request. The family byte
/// (`AF_UNSPEC` for links, `AF_INET`/`AF_INET6` for addresses and routes)
/// starts the body.
pub fn dump_request(out: &mut Vec<u8>, kind: u16, seq: u32, family: u8, extra: &[u8]) {
    let start = request(out, kind, seq, true);
    out.push(family);
    out.extend_from_slice(extra);
    finish(out, start);
}

/// Builds the `GETLINK` body after the family byte: the full `ifinfomsg`
/// (family plus 15 zero bytes), the way iproute2 sends it. A short body
/// answers on lenient kernels but is not a well-formed request.
pub fn link_dump(out: &mut Vec<u8>, seq: u32) {
    dump_request(out, RTM_GETLINK, seq, 0, &[0u8; 15]);
}

/// Builds the `GETADDR`/`GETROUTE` bodies after the family byte.
pub fn addr_dump(out: &mut Vec<u8>, seq: u32, family: u8) {
    dump_request(out, RTM_GETADDR, seq, family, &[0u8; 7]);
}

pub fn route_dump(out: &mut Vec<u8>, seq: u32, family: u8) {
    dump_request(out, RTM_GETROUTE, seq, family, &[0u8; 11]);
}

/// Generic netlink header: command, version, padding.
pub const GENL_LEN: usize = 4;

/// Starts a generic netlink request for `family`/`command`.
pub fn genl_request(out: &mut Vec<u8>, family: u16, command: u8, seq: u32, dump: bool) -> usize {
    let start = request(out, family, seq, dump);
    out.push(command);
    out.push(1);
    out.extend_from_slice(&[0u8; 2]);
    start
}

/// The generic command of a received message's body, and its attributes.
pub fn genl_of(body: &[u8]) -> Option<(u8, &[u8])> {
    if body.len() < GENL_LEN {
        return None;
    }
    Some((body[0], &body[GENL_LEN..]))
}

/// Builds `CTRL_CMD_GETFAMILY` for `name`.
pub fn family_request(out: &mut Vec<u8>, seq: u32, name: &[u8]) {
    let start = genl_request(out, GENL_ID_CTRL, CTRL_CMD_GETFAMILY, seq, false);
    attr_bytes(out, CTRL_ATTR_FAMILY_NAME, name);
    finish(out, start);
}

/// The nl80211 family id and its multicast group ids in a `GETFAMILY`
/// reply. Stops at the reply's end; an id of 0 is "not found".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Family {
    pub id: u16,
    pub scan: u32,
    pub mlme: u32,
}

pub fn parse_family(body: &[u8]) -> Family {
    let mut family = Family::default();
    let Some((_, fields)) = genl_of(body) else {
        return family;
    };
    for attr in attrs(fields) {
        match attr.kind {
            CTRL_ATTR_FAMILY_ID => {
                if attr.payload.len() >= 2 {
                    family.id = u16::from_ne_bytes([attr.payload[0], attr.payload[1]]);
                }
            }
            CTRL_ATTR_MCAST_GROUPS => {
                for group in attrs(attr.payload) {
                    // Each entry is itself nested: walk its name and id.
                    // Names arrive NUL-terminated; the match is without.
                    let mut name = &[][..];
                    let mut id = 0;
                    for inner in attrs(group.payload) {
                        match inner.kind {
                            CTRL_ATTR_MCAST_GRP_NAME => {
                                name = strip_nul(inner.payload);
                            }
                            CTRL_ATTR_MCAST_GRP_ID if inner.payload.len() >= 4 => {
                                id = u32::from_ne_bytes([
                                    inner.payload[0],
                                    inner.payload[1],
                                    inner.payload[2],
                                    inner.payload[3],
                                ]);
                            }
                            _ => {}
                        }
                    }
                    match name {
                        b"scan" => family.scan = id,
                        b"mlme" => family.mlme = id,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    family
}

/// Appends one byte-string attribute, NUL-terminated with padding.
fn attr_bytes(out: &mut Vec<u8>, kind: u16, bytes: &[u8]) {
    let len = bytes.len() + 1;
    out.extend_from_slice(&(len as u16 + 4).to_ne_bytes());
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(bytes);
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Appends one u32 attribute.
pub fn attr_u32(out: &mut Vec<u8>, kind: u16, value: u32) {
    out.extend_from_slice(&8u16.to_ne_bytes());
    out.extend_from_slice(&kind.to_ne_bytes());
    out.extend_from_slice(&value.to_ne_bytes());
}

/// Appends one u8 attribute.
fn attr_u8(out: &mut Vec<u8>, kind: u16, value: u8) {
    out.extend_from_slice(&5u16.to_ne_bytes());
    out.extend_from_slice(&kind.to_ne_bytes());
    out.push(value);
    out.extend_from_slice(&[0u8; 3]);
}

/// Builds `NL80211_CMD_GET_INTERFACE` (a dump, or for one `ifindex`).
pub fn interface_request(out: &mut Vec<u8>, family: u16, seq: u32, ifindex: u32) {
    let start = genl_request(out, family, NL80211_CMD_GET_INTERFACE, seq, ifindex == 0);
    if ifindex != 0 {
        attr_u32(out, NL80211_ATTR_IFINDEX, ifindex);
    }
    finish(out, start);
}

/// A wireless interface: its index and name from a `NEW_INTERFACE` notice.
/// Takes the full message body, like every other `parse_*` here (the
/// generic header is stripped inside, not by the caller).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wireless {
    pub index: u32,
    pub name: [u8; 16],
    pub name_len: u8,
    pub ssid: [u8; 32],
    pub ssid_len: u8,
}

pub fn parse_interface(body: &[u8]) -> Wireless {
    let mut wifi = Wireless::default();
    let Some((_, fields)) = genl_of(body) else {
        return wifi;
    };
    for attr in attrs(fields) {
        match attr.kind {
            NL80211_ATTR_IFINDEX => {
                if attr.payload.len() >= 4 {
                    wifi.index = u32::from_ne_bytes([
                        attr.payload[0],
                        attr.payload[1],
                        attr.payload[2],
                        attr.payload[3],
                    ]);
                }
            }
            NL80211_ATTR_IFNAME => {
                let name = strip_nul(attr.payload);
                let take = name.len().min(wifi.name.len());
                wifi.name[..take].copy_from_slice(&name[..take]);
                wifi.name_len = take as u8;
            }
            NL80211_ATTR_SSID => {
                // Present on interface info while associated (seen on a
                // live dump); absent on a fresh interface.
                let take = attr.payload.len().min(wifi.ssid.len());
                wifi.ssid[..take].copy_from_slice(&attr.payload[..take]);
                wifi.ssid_len = take as u8;
            }
            _ => {}
        }
    }
    wifi
}

/// The payload without one trailing NUL: netlink strings travel
/// terminated, and the terminator is not part of the value.
fn strip_nul(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\0").unwrap_or(bytes)
}

/// Builds `NL80211_CMD_GET_SCAN` (a dump of the cached results) for
/// `ifindex`.
pub fn scan_request(out: &mut Vec<u8>, family: u16, seq: u32, ifindex: u32) {
    let start = genl_request(out, family, NL80211_CMD_GET_SCAN, seq, true);
    attr_u32(out, NL80211_ATTR_IFINDEX, ifindex);
    finish(out, start);
}

/// A BSS from the scan: its SSID (from the IEs), signal in dBm and
/// whether this interface is associated to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bss {
    pub ssid: [u8; 32],
    pub ssid_len: u8,
    /// The signal in dBm, where the nest carried `SIGNAL_MBM`.
    pub signal: Option<i8>,
    pub associated: bool,
}

impl Bss {
    #[allow(dead_code)]
    pub fn ssid_str(&self) -> &str {
        std::str::from_utf8(&self.ssid[..self.ssid_len as usize]).unwrap_or("")
    }
}

/// Parses one `NL80211_ATTR_BSS` nest. A BSS without IEs, or with a hidden
/// SSID (empty tag 0), keeps `ssid_len` 0: present but unnamed.
pub fn parse_bss(payload: &[u8], bss: &mut Bss) {
    for attr in attrs(payload) {
        match attr.kind {
            NL80211_BSS_INFORMATION_ELEMENTS => parse_ies(attr.payload, bss),
            NL80211_BSS_SIGNAL_MBM => {
                if attr.payload.len() >= 4 {
                    let mbm = i32::from_ne_bytes([
                        attr.payload[0],
                        attr.payload[1],
                        attr.payload[2],
                        attr.payload[3],
                    ]);
                    bss.signal = Some((mbm / 100).clamp(-128, 127) as i8);
                }
            }
            NL80211_BSS_STATUS if attr.payload.len() >= 4 => {
                let status = u32::from_ne_bytes([
                    attr.payload[0],
                    attr.payload[1],
                    attr.payload[2],
                    attr.payload[3],
                ]);
                bss.associated = status == NL80211_BSS_ASSOCIATED;
            }
            _ => {}
        }
    }
}

/// Pulls the SSID (IE tag 0) out of the information elements. Anything
/// past 32 bytes is cut; a second tag 0 does not overwrite the first.
fn parse_ies(payload: &[u8], bss: &mut Bss) {
    let mut ies = payload;
    while ies.len() >= 2 {
        let tag = ies[0];
        let len = ies[1] as usize;
        if ies.len() < 2 + len {
            break;
        }
        if tag == 0 && bss.ssid_len == 0 {
            let take = len.min(bss.ssid.len());
            bss.ssid[..take].copy_from_slice(&ies[2..2 + take]);
            bss.ssid_len = take as u8;
        }
        ies = &ies[2 + len..];
    }
}

/// Folds one scan dump's messages into `out`: the associated BSS first,
/// then the rest in arrival order, at most `out.len()`. Returns how many
/// entries are valid.
pub fn fold_scan(body: &[u8], out: &mut [Bss]) -> usize {
    let mut count = 0;
    let Some((_, fields)) = genl_of(body) else {
        return 0;
    };
    for attr in attrs(fields) {
        if attr.kind != NL80211_ATTR_BSS || count >= out.len() {
            continue;
        }
        let mut bss = Bss::default();
        parse_bss(attr.payload, &mut bss);
        out[count] = bss;
        count += 1;
    }
    // The associated network is what is shown: move it to the front.
    if let Some(at) = out[..count].iter().position(|bss| bss.associated) {
        out[..=at].rotate_right(1);
    }
    count
}

/// Builds `NL80211_CMD_GET_STATION` for the peer on `ifindex`: a dump,
/// not a single query — a client interface has exactly one peer, and the
/// single form is refused without privileges where the dump answers.
pub fn station_request(out: &mut Vec<u8>, family: u16, seq: u32, ifindex: u32) {
    let start = genl_request(out, family, NL80211_CMD_GET_STATION, seq, true);
    attr_u32(out, NL80211_ATTR_IFINDEX, ifindex);
    finish(out, start);
}

/// The peer's signal in dBm from a `GET_STATION` reply, or `None` when it
/// carries none (disassociated between the request and the reply).
pub fn parse_station(body: &[u8]) -> Option<i8> {
    let (_, fields) = genl_of(body)?;
    for attr in attrs(fields) {
        if attr.kind == NL80211_ATTR_STA_INFO {
            for info in attrs(attr.payload) {
                if info.kind == NL80211_STA_INFO_SIGNAL && !info.payload.is_empty() {
                    return Some(info.payload[0] as i8);
                }
            }
        }
    }
    None
}

/// Builds `NL80211_CMD_SET_CQM` with RSSI thresholds: the measurement
/// probe for whether the driver reports signal changes as events. Not
/// sent by the module (see the ticket's signal decision); the live test
/// sends it once and records the errno.
#[allow(dead_code)]
pub fn cqm_request(out: &mut Vec<u8>, family: u16, seq: u32, ifindex: u32, thresholds: &[i8]) {
    let start = genl_request(out, family, NL80211_CMD_SET_CQM, seq, false);
    attr_u32(out, NL80211_ATTR_IFINDEX, ifindex);
    let nested = out.len();
    out.extend_from_slice(&[0u8; 4]);
    for threshold in thresholds {
        attr_u8(out, NL80211_CQM_RSSI_THOLD, *threshold as u8);
    }
    attr_u8(out, NL80211_CQM_RSSI_HYST, 4);
    let len = (out.len() - nested) as u16;
    out[nested..nested + 2].copy_from_slice(&len.to_ne_bytes());
    out[nested + 2..nested + 4].copy_from_slice(&(NL80211_ATTR_CQM | NLA_F_NESTED).to_ne_bytes());
    finish(out, start);
}

/// An untrusted string made view-safe: lossy UTF-8, no control
/// characters (newlines included, so a menu line is one network).
pub fn sanitize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|c| if c.is_control() { '�' } else { c })
        .collect()
}

/// Signal bars from dBm, 1 to 4. Measured against the Asahi box: −54 dBm
/// at arm's length from the AP is 4, the far bedroom at −72 is 2, and a
/// network that barely associates (−80 and below) is 1.
pub fn bars_for(signal: i8) -> u8 {
    if signal >= -55 {
        4
    } else if signal >= -67 {
        3
    } else if signal >= -77 {
        2
    } else {
        1
    }
}

/// The bar glyphs for `bars`: `▂▄▆█` filled left to right.
pub const BAR_GLYPHS: [char; 4] = ['▂', '▄', '▆', '█'];

#[cfg(test)]
mod unit;
