//! The network module's scripted kernel: a socketpair stands in for each
//! netlink socket. The test writes canned datagrams into one end and
//! drains the module's dump and query requests from it; the module cannot
//! tell it from the kernel, framing being identical. Test-only: compiled
//! under `#[cfg(test)]`, never into the bar.

use std::os::fd::OwnedFd;

use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, recv, send, socketpair,
};

use super::netlink;
use super::{Settings, start_with};
use crate::modules::harness::Harness;

/// The sequence canned replies carry: nonzero, like the kernel's own
/// (notices use zero, and the module tells them apart by it).
const REPLY_SEQ: u32 = 9;

/// The test ends of both pairs: what the module sends is drained from
/// here, so its outbox never blocks it.
pub struct Fake {
    rt: OwnedFd,
    genl: OwnedFd,
}

/// One unconnected pair: `stand_in`'s way in when even the sockets fail.
pub fn pair() -> (OwnedFd, OwnedFd) {
    let (a, b) = socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::CLOEXEC,
        None,
    )
    .expect("a socketpair");
    (a, b)
}

/// The fake kernel's nl80211 family id: arbitrary, like the real one's.
pub const FAMILY: u16 = 30;

impl Fake {
    pub fn start(settings: &Settings) -> (Harness, Self) {
        let (rt_mod, rt) = pair();
        let (genl_mod, genl) = pair();
        let harness = Harness::new(start_with(settings, rt_mod, genl_mod, FAMILY));
        (harness, Self { rt, genl })
    }

    /// A canned rtnetlink datagram for the module to read.
    pub fn rt(&self, bytes: &[u8]) {
        send(&self.rt, bytes, SendFlags::empty()).expect("the module reads");
    }

    /// A canned generic-netlink datagram for the module to read.
    pub fn genl(&self, bytes: &[u8]) {
        send(&self.genl, bytes, SendFlags::empty()).expect("the module reads");
    }

    /// What the module sent since the last drain: its dumps and queries.
    pub fn sent(&self) -> (Vec<u8>, Vec<u8>) {
        (drain(&self.rt), drain(&self.genl))
    }
}

fn drain(fd: &OwnedFd) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    rustix::fs::fcntl_setfl(fd, rustix::fs::OFlags::NONBLOCK).ok();
    loop {
        match recv(fd, &mut buf, RecvFlags::empty()) {
            Ok((_, 0)) | Err(_) => break,
            Ok((_, n)) => out.extend_from_slice(&buf[..n]),
        }
    }
    rustix::fs::fcntl_setfl(fd, rustix::fs::OFlags::empty()).ok();
    out
}

/// One `NEWLINK`/`DELLINK`: `struct ifinfomsg`, native-endian throughout
/// (family, pad, then the type at `body[2..4]` — see `netlink/unit`).
pub fn link(
    kind: u16,
    index: u32,
    flags: u32,
    oper: u8,
    name: &str,
    link_kind: Option<&str>,
) -> Vec<u8> {
    let mut body = vec![0u8; 16];
    body[2..4].copy_from_slice(&1u16.to_ne_bytes());
    body[4..8].copy_from_slice(&index.to_ne_bytes());
    body[8..12].copy_from_slice(&flags.to_ne_bytes());
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    body.extend_from_slice(&attr(3, &payload));
    body.extend_from_slice(&attr(16, &[oper]));
    if let Some(kind_name) = link_kind {
        let mut inner = attr(1, kind_name.as_bytes());
        let mut outer = vec![0u8; 4];
        outer.append(&mut inner);
        let len = outer.len() as u16;
        outer[0..2].copy_from_slice(&len.to_ne_bytes());
        outer[2..4].copy_from_slice(&18u16.to_ne_bytes());
        body.extend_from_slice(&outer);
    }
    framed(kind, 0, &body)
}

/// One `NEWADDR`/`DELADDR` for `index` (`2` is v4, `10` v6).
pub fn addr(kind: u16, index: u32, family: u8) -> Vec<u8> {
    let mut body = vec![family, 24, 0, 0];
    body.extend_from_slice(&index.to_ne_bytes());
    framed(kind, 0, &body)
}

/// One `NEWROUTE`/`DELROUTE`: the default route (`family`) via `oif`.
pub fn route(kind: u16, family: u8, oif: u32) -> Vec<u8> {
    let mut body = vec![family, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    body.extend_from_slice(&attr(4, &oif.to_ne_bytes()));
    framed(kind, 0, &body)
}

/// One `DEL_INTERFACE` notice for a wireless interface going away.
pub fn del_interface(index: u32) -> Vec<u8> {
    let mut body = vec![netlink::NL80211_CMD_DEL_INTERFACE, 1, 0, 0];
    body.extend_from_slice(&attr(3, &index.to_ne_bytes()));
    framed(FAMILY, 0, &body)
}

/// One `NEW_INTERFACE` notice for a wireless interface: the name travels
/// NUL-terminated, as the kernel sends it, and the SSID rides along
/// while associated. A station, like the existing tests' interfaces.
pub fn interface(index: u32, name: &str, ssid: Option<&[u8]>) -> Vec<u8> {
    interface_with_type(index, name, ssid, netlink::NL80211_IFTYPE_STATION)
}

/// One `NEW_INTERFACE` notice with an explicit interface type (a station,
/// an access point, ...): what tells the module which radio's scan to
/// keep. The kernel always sends the type; the value is `nl80211_iftype`.
pub fn interface_with_type(index: u32, name: &str, ssid: Option<&[u8]>, iftype: u32) -> Vec<u8> {
    let mut body = vec![netlink::NL80211_CMD_NEW_INTERFACE, 1, 0, 0];
    body.extend_from_slice(&attr(3, &index.to_ne_bytes()));
    let mut terminated = name.as_bytes().to_vec();
    terminated.push(0);
    body.extend_from_slice(&attr(4, &terminated));
    body.extend_from_slice(&attr(5, &iftype.to_ne_bytes()));
    if let Some(ssid) = ssid {
        body.extend_from_slice(&attr(52, ssid));
    }
    framed(FAMILY, 0, &body)
}

/// One `GET_SCAN` reply page: `networks` are SSID, signal dBm and whether
/// associated. The entries travel as `NEW_SCAN_RESULTS`, like the
/// kernel's own pages.
pub fn scan(networks: &[(&[u8], i32, bool)]) -> Vec<u8> {
    let mut body = vec![netlink::NL80211_CMD_NEW_SCAN_RESULTS, 1, 0, 0];
    for (ssid, mbm, associated) in networks {
        let mut nest = Vec::new();
        let mut ie = vec![0u8, ssid.len() as u8];
        ie.extend_from_slice(ssid);
        nest.extend_from_slice(&attr(6, &ie));
        nest.extend_from_slice(&attr(7, &mbm.to_ne_bytes()));
        nest.extend_from_slice(&attr(9, &(*associated as u32).to_ne_bytes()));
        let mut outer = vec![0u8; 4];
        outer.append(&mut nest);
        let len = outer.len() as u16;
        outer[0..2].copy_from_slice(&len.to_ne_bytes());
        outer[2..4].copy_from_slice(&47u16.to_ne_bytes());
        body.extend_from_slice(&outer);
    }
    framed(FAMILY, REPLY_SEQ, &body)
}

/// One station dump entry with the peer at `signal` dBm. The entries
/// travel as `NEW_STATION`, like the kernel's own.
pub fn station(signal: i8) -> Vec<u8> {
    let mut info = vec![0u8; 4];
    info[0..2].copy_from_slice(&5u16.to_ne_bytes());
    info[2..4].copy_from_slice(&7u16.to_ne_bytes());
    info.push(signal as u8);
    info.extend_from_slice(&[0u8; 3]);
    let mut nest = vec![0u8; 4];
    nest.append(&mut info);
    let len = nest.len() as u16;
    nest[0..2].copy_from_slice(&len.to_ne_bytes());
    nest[2..4].copy_from_slice(&21u16.to_ne_bytes());
    let mut body = vec![netlink::NL80211_CMD_NEW_STATION, 1, 0, 0];
    body.append(&mut nest);
    framed(FAMILY, REPLY_SEQ, &body)
}

/// One `NLMSG_ERROR` carrying `errno` (negative, as the kernel sends).
pub fn error(errno: i32) -> Vec<u8> {
    framed(netlink::NLMSG_ERROR, 0, &errno.to_ne_bytes())
}

/// One `NLMSG_DONE` answering the dump request with `seq`: every dump
/// ends in one, releasing the next queued dump. The module matches the
/// sequence, as with the kernel's own terminators.
pub fn done_seq(seq: u32) -> Vec<u8> {
    framed(netlink::NLMSG_DONE, seq, &[])
}

/// One `ASSOCIATE`/`ROAM` notice for `index`: the module re-reads the
/// interface, the station and the scan.
pub fn roam(index: u32) -> Vec<u8> {
    let mut body = vec![netlink::NL80211_CMD_ROAM, 1, 0, 0];
    body.extend_from_slice(&attr(3, &index.to_ne_bytes()));
    framed(FAMILY, 0, &body)
}

/// One `DEAUTHENTICATE` notice for `index`: the SSID and signal clear.
pub fn deauth(index: u32) -> Vec<u8> {
    let mut body = vec![netlink::NL80211_CMD_DEAUTHENTICATE, 1, 0, 0];
    body.extend_from_slice(&attr(3, &index.to_ne_bytes()));
    framed(FAMILY, 0, &body)
}

/// One `NEW_SCAN_RESULTS` notice: the module re-dumps the scan.
pub fn scan_done() -> Vec<u8> {
    let body = vec![netlink::NL80211_CMD_NEW_SCAN_RESULTS, 1, 0, 0];
    framed(FAMILY, 0, &body)
}

fn attr(kind: u16, payload: &[u8]) -> Vec<u8> {
    let mut attr = Vec::new();
    attr.extend_from_slice(&((payload.len() + 4) as u16).to_ne_bytes());
    attr.extend_from_slice(&kind.to_ne_bytes());
    attr.extend_from_slice(payload);
    while attr.len() % 4 != 0 {
        attr.push(0);
    }
    attr
}

fn framed(kind: u16, seq: u32, body: &[u8]) -> Vec<u8> {
    let mut datagram = vec![0u8; 16];
    datagram[4..6].copy_from_slice(&kind.to_ne_bytes());
    datagram[8..12].copy_from_slice(&seq.to_ne_bytes());
    datagram.extend_from_slice(body);
    let len = datagram.len() as u32;
    datagram[0..4].copy_from_slice(&len.to_ne_bytes());
    datagram
}
