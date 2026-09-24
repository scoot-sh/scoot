//! The host's `zwp_linux_dmabuf_feedback_v1`, read once at startup, and the
//! one decision made from it: which `{fourcc, modifier}` scoot allocates its
//! host buffers in, if any.
//!
//! Everything here is **input from another process** and is parsed as such:
//! a table size that is not a whole number of entries, an index past the end
//! of the table, a `dev_t` array of the wrong length, a feedback that never
//! finishes -- each is "no usable feedback", which means read-back, never a
//! panic and never a guess. The table is read with `pread` at offset 0 rather
//! than `read`: a compositor may hand every client the same open file
//! description, and advancing its shared offset would starve the next
//! reader (the protocol's own advice is a private read-only mapping, which
//! `pread` matches without the `unsafe`).
//!
//! Startup-only: the collector allocates freely, once per session.

use std::os::fd::OwnedFd;

use smithay::backend::allocator::{Format, Fourcc, Modifier};
use smithay::backend::drm::{DrmNode, NodeType};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_feedback_v1::{
    self, TrancheFlags, ZwpLinuxDmabufFeedbackV1,
};

/// The most table entries a feedback can name: tranche indices are `u16`.
/// A table longer than this could only carry entries nothing can point at,
/// so the read is capped here rather than trusting a host's `size`.
const MAX_ENTRIES: usize = 1 << 16;

/// One `{fourcc, modifier}` pair of the format table, as the wire has it.
const ENTRY_LEN: usize = 16;

/// The host's default feedback, as far as it got, collected on a private
/// event queue (see `gpu::negotiate`).
#[derive(Default)]
pub(in crate::compositor) struct Collector {
    table: Option<(OwnedFd, u32)>,
    main_device: Option<libc::dev_t>,
    tranches: Vec<RawTranche>,
    pending: RawTranche,
    /// Set by `done`: the feedback is complete.
    done: bool,
    /// Set by any event this could not make sense of. Sticky: a feedback
    /// with one malformed part is not trusted for any of it.
    malformed: bool,
}

#[derive(Default)]
struct RawTranche {
    device: Option<libc::dev_t>,
    indices: Vec<u16>,
    scanout: bool,
}

/// The host's feedback, resolved against its format table.
#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::compositor) struct HostFeedback {
    pub(in crate::compositor) main_device: libc::dev_t,
    pub(in crate::compositor) tranches: Vec<HostTranche>,
}

/// One preference tranche, most preferred first in [`HostFeedback`].
#[derive(Debug, Default, PartialEq, Eq)]
pub(in crate::compositor) struct HostTranche {
    pub(in crate::compositor) device: libc::dev_t,
    pub(in crate::compositor) scanout: bool,
    /// `(fourcc, modifier)`, as raw wire values.
    pub(in crate::compositor) formats: Vec<(u32, u64)>,
}

impl Collector {
    /// The finished feedback, or why there is none to use.
    pub(in crate::compositor) fn finish(self) -> Result<HostFeedback, &'static str> {
        if self.malformed {
            return Err("the host's dma-buf feedback was malformed");
        }
        if !self.done {
            return Err("the host did not finish sending its dma-buf feedback");
        }
        let Some(main_device) = self.main_device else {
            return Err("the host's dma-buf feedback named no main device");
        };
        let Some((fd, size)) = self.table else {
            return Err("the host's dma-buf feedback carried no format table");
        };
        let table = read_table(&fd, size).ok_or("the host's dma-buf format table is unreadable")?;
        resolve(main_device, &self.tranches, &table)
            .ok_or("the host's dma-buf feedback indexes past its format table")
    }
}

/// The tranches with their indices looked up in `table`, or `None` if any
/// index points past it -- a host that sends one is not trusted for the rest.
/// A tranche with no target device is dropped (the protocol requires one, so
/// nothing can be concluded about where its formats may be allocated).
fn resolve(
    main_device: libc::dev_t,
    tranches: &[RawTranche],
    table: &[(u32, u64)],
) -> Option<HostFeedback> {
    let mut resolved = Vec::with_capacity(tranches.len());
    for tranche in tranches {
        let formats = tranche
            .indices
            .iter()
            .map(|&index| table.get(usize::from(index)).copied())
            .collect::<Option<Vec<_>>>()?;
        if let Some(device) = tranche.device {
            resolved.push(HostTranche {
                device,
                scanout: tranche.scanout,
                formats,
            });
        }
    }
    Some(HostFeedback {
        main_device,
        tranches: resolved,
    })
}

/// The table's `(fourcc, modifier)` entries, or `None` for a size that is
/// not a whole number of entries, is past [`MAX_ENTRIES`], or that could not
/// be read in full.
fn read_table(fd: &OwnedFd, size: u32) -> Option<Vec<(u32, u64)>> {
    let len = usize::try_from(size).ok()?;
    if !len.is_multiple_of(ENTRY_LEN) || len / ENTRY_LEN > MAX_ENTRIES {
        return None;
    }
    let mut bytes = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        // `pread` at an explicit offset: see the module doc for why not `read`.
        let offset = u64::try_from(filled).ok()?;
        match rustix::io::pread(fd, &mut bytes[filled..], offset) {
            Ok(0) | Err(_) => return None,
            Ok(n) => filled += n,
        }
    }
    Some(parse_table(&bytes))
}

/// Splits a whole table into entries. `bytes.len()` is a multiple of
/// [`ENTRY_LEN`] (checked by the caller); a trailing partial entry, were
/// there one, is ignored rather than read past.
fn parse_table(bytes: &[u8]) -> Vec<(u32, u64)> {
    bytes
        .chunks_exact(ENTRY_LEN)
        .map(|entry| {
            let (fourcc, rest) = entry.split_at(4);
            let modifier = &rest[4..];
            (
                u32::from_ne_bytes(fourcc.try_into().unwrap_or_default()),
                u64::from_ne_bytes(modifier.try_into().unwrap_or_default()),
            )
        })
        .collect()
}

/// A `dev_t` out of a protocol array, or `None` for an array of any other
/// length (a fatal `invalid_dev_t_size` on the server side of the same
/// protocol; here, just unusable).
fn dev_t(bytes: &[u8]) -> Option<libc::dev_t> {
    Some(libc::dev_t::from_ne_bytes(bytes.try_into().ok()?))
}

/// `u16` indices out of a protocol array, or `None` for an odd length.
fn indices(bytes: &[u8]) -> Option<Vec<u16>> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(2)
            .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
            .collect(),
    )
}

impl Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for Collector {
    fn event(
        collector: &mut Self,
        _: &ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwp_linux_dmabuf_feedback_v1::Event;
        // Only the first complete feedback is used; anything after `done`
        // (a host re-sending on a device change) is ignored -- this queue is
        // not dispatched again once `negotiate` has its answer.
        if collector.done {
            return;
        }
        match event {
            Event::FormatTable { fd, size } => collector.table = Some((fd, size)),
            Event::MainDevice { device } => match dev_t(&device) {
                Some(device) => collector.main_device = Some(device),
                None => collector.malformed = true,
            },
            Event::TrancheTargetDevice { device } => match dev_t(&device) {
                Some(device) => collector.pending.device = Some(device),
                None => collector.malformed = true,
            },
            Event::TrancheFormats { indices: bytes } => match indices(&bytes) {
                Some(mut more) => collector.pending.indices.append(&mut more),
                None => collector.malformed = true,
            },
            Event::TrancheFlags { flags } => {
                collector.pending.scanout = match flags {
                    WEnum::Value(flags) => flags.contains(TrancheFlags::Scanout),
                    // Unknown bits from a newer host: only `scanout` is
                    // read, and it is bit 0 in every version.
                    WEnum::Unknown(bits) => bits & 1 != 0,
                };
            }
            Event::TrancheDone => {
                let tranche = std::mem::take(&mut collector.pending);
                collector.tranches.push(tranche);
            }
            Event::Done => collector.done = true,
            _ => {}
        }
    }
}

/// What scoot allocates its host buffers as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::compositor) struct Choice {
    pub(in crate::compositor) fourcc: Fourcc,
    /// What GBM is asked for: the explicit modifiers both sides take, most
    /// preferred tranche first -- or `[Invalid]` alone for an implicit
    /// allocation, when no explicit modifier is common to both.
    pub(in crate::compositor) request: Vec<Modifier>,
    /// What an allocated buffer may come back as and still be offered to the
    /// host: `request`, plus `Invalid` where the host lists an implicit
    /// layout. GBM -- and Smithay's allocator in front of it, which retries
    /// without modifiers when a list naming `Linear` fails -- can silently
    /// hand back an implicit buffer, so the answer is checked against this,
    /// not assumed to be one of `request`.
    pub(in crate::compositor) accept: Vec<Modifier>,
}

/// The fourccs scoot's host buffers may be, most preferred first.
/// `Argb8888` first because it is what the read-back path presents
/// (`buffers.rs`), so a session looks the same to the host either way --
/// including a translucent background, which an `Xrgb8888` buffer would
/// make opaque. The render target is `Argb8888` too, and a blit between
/// the two 8-bit layouts needs no conversion.
const FOURCCS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

/// Picks the format and modifiers to allocate host buffers in, or `None`
/// when nothing both sides take exists on this device.
///
/// - Only tranches whose target device *is* the renderer's device count
///   (`same_device`), and the host's main device must be that device too:
///   the host has to import these buffers for compositing on its main
///   device, and scoot renders into them on its own. Anything else is a
///   cross-device import this path does not attempt.
/// - Scanout-flagged tranches come first, then the host's own order, which
///   is its preference.
/// - An explicit modifier must be one the renderer can render into
///   (`renders`). `Invalid` -- an implicit layout -- is only asked for when
///   no explicit modifier is common to both, and only if the host lists it:
///   Smithay adds `{fourcc, Invalid}` to every GLES render set
///   unconditionally (PR #229), so the renderer's side of that is not
///   evidence, and implicit is never mixed with explicit. The caller's
///   render-and-blit probe is what proves an implicit buffer really works.
pub(in crate::compositor) fn choose(
    feedback: &HostFeedback,
    same_device: impl Fn(libc::dev_t) -> bool,
    renders: impl Fn(Format) -> bool,
) -> Option<Choice> {
    if !same_device(feedback.main_device) {
        return None;
    }
    let ours = || {
        let scanout = feedback.tranches.iter().filter(|tranche| tranche.scanout);
        let rest = feedback.tranches.iter().filter(|tranche| !tranche.scanout);
        scanout
            .chain(rest)
            .filter(|tranche| same_device(tranche.device))
    };
    FOURCCS.into_iter().find_map(|fourcc| {
        let mut implicit = false;
        let mut explicit: Vec<Modifier> = Vec::new();
        for tranche in ours() {
            for &(code, modifier) in &tranche.formats {
                if code != fourcc as u32 {
                    continue;
                }
                let modifier = Modifier::from(modifier);
                if modifier == Modifier::Invalid {
                    implicit = true;
                } else if !explicit.contains(&modifier)
                    && renders(Format {
                        code: fourcc,
                        modifier,
                    })
                {
                    explicit.push(modifier);
                }
            }
        }
        if explicit.is_empty() && !implicit {
            return None;
        }
        let mut accept = explicit.clone();
        if implicit {
            accept.push(Modifier::Invalid);
        }
        let request = if explicit.is_empty() {
            vec![Modifier::Invalid]
        } else {
            explicit
        };
        Some(Choice {
            fourcc,
            request,
            accept,
        })
    })
}

/// Whether `a` and `b` name the same DRM device, whichever node type each
/// is -- the protocol says in as many words that two `dev_t`s cannot be
/// compared directly (a host may name its primary node where scoot names the
/// render node, which the dev VM's cage does). Each is taken to its device's
/// render node where it has one; a `dev_t` that is not a DRM node at all
/// matches nothing.
pub(in crate::compositor) fn same_drm_device(a: libc::dev_t, b: libc::dev_t) -> bool {
    match (canonical(a), canonical(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn canonical(dev: libc::dev_t) -> Option<libc::dev_t> {
    let node = DrmNode::from_dev_id(dev).ok()?;
    match node.node_with_type(NodeType::Render) {
        Some(Ok(render)) => Some(render.dev_id()),
        // A device with no render node (a display-only controller) is
        // compared as the node it was named by.
        _ => Some(node.dev_id()),
    }
}

#[cfg(test)]
pub(in crate::compositor) mod test_access {
    //! Constructors for the collector's private halves, for `gpu/tests.rs`.
    use super::*;

    pub(in crate::compositor) fn parse(bytes: &[u8]) -> Vec<(u32, u64)> {
        parse_table(bytes)
    }

    pub(in crate::compositor) fn read(fd: &OwnedFd, size: u32) -> Option<Vec<(u32, u64)>> {
        read_table(fd, size)
    }

    pub(in crate::compositor) fn dev(bytes: &[u8]) -> Option<libc::dev_t> {
        dev_t(bytes)
    }

    pub(in crate::compositor) fn index_list(bytes: &[u8]) -> Option<Vec<u16>> {
        indices(bytes)
    }

    /// Resolves hand-written tranches `(device, indices, scanout)` against
    /// `table`, the way [`Collector::finish`] does.
    pub(in crate::compositor) fn resolve_tranches(
        main_device: libc::dev_t,
        tranches: &[(Option<libc::dev_t>, Vec<u16>, bool)],
        table: &[(u32, u64)],
    ) -> Option<HostFeedback> {
        let raw: Vec<RawTranche> = tranches
            .iter()
            .map(|(device, indices, scanout)| RawTranche {
                device: *device,
                indices: indices.clone(),
                scanout: *scanout,
            })
            .collect();
        resolve(main_device, &raw, table)
    }
}
