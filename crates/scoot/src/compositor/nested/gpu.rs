//! `--nested --renderer gles`: handing each frame to the host as a dma-buf
//! instead of reading it back into `wl_shm`.
//!
//! Without this, a nested GLES session composites on the GPU and then reads
//! every frame back to main memory (`render::read_back`), copies it into a
//! host `wl_shm` buffer (`buffers.rs`), and the host uploads it again to
//! composite it -- so on a GPU box `--renderer gles` buys nothing nested.
//! With it, the frame is composited exactly as before, into the same
//! persistent render target, and then **blitted on the GPU** into one of a
//! few host buffers scoot allocated on the renderer's own device and shared
//! with the host through `zwp_linux_dmabuf_v1`. No CPU copy anywhere.
//!
//! # Build shape
//!
//! Behind the `gpu-scanout` Cargo feature, the one this crate already has
//! for "links libgbm": allocating a buffer the host can import needs GBM (a
//! link-time dependency), or an EGL Wayland-platform window surface
//! (`libwayland-egl`, also link-time unless dlopened, and a second EGL
//! display per session). The default build keeps linking no GPU stack --
//! the `ldd` check in CI is what proves it -- and
//! presents by read-back, as it always has. A sibling feature would name the
//! same link-time cost twice and double the flake's packaging matrix for
//! nothing.
//!
//! # When it is used, and when read-back is
//!
//! Decided once, at startup ([`negotiate`]), and logged once at INFO either
//! way, with the reason, for every GLES session. A pixman session has only
//! ever had the one way and says nothing; a default build's GLES session
//! logs the same read-back line, naming the missing feature as the reason.
//! Read-back is kept whenever any of these holds:
//!
//! - the build has no `gpu-scanout` feature, or the session renders with
//!   pixman (there is no GPU buffer to hand over);
//! - the host has no `zwp_linux_dmabuf_v1` at version 4 or later -- without
//!   its feedback there is no way to know which device it imports on;
//! - the feedback is malformed or never finishes (see [`feedback`]);
//! - the renderer's EGL device names no DRM node, or the host's main device
//!   is a different DRM device from the renderer's (a split-GPU machine
//!   whose host composites on the other GPU) -- **never** worked around by
//!   moving the renderer: the renderer's device is pinned for the session
//!   (`render::gles::GlesDevice`, PR #229), because the dma-buf formats
//!   already advertised to scoot's own clients are that device's;
//! - no `{fourcc, modifier}` is both listed by the host for that device and
//!   renderable by the renderer ([`feedback::choose`]);
//! - GBM cannot allocate one on the device, or the renderer cannot render
//!   into and blit to what it allocated (the probe below).
//!
//! GBM is opened on the renderer's **render node** first and, only if that
//! cannot allocate, the same device's **primary node**. Both are the same
//! device, so nothing about device matching changes; the second rung exists
//! because Mesa's `kms_swrast` (the dev VM's virtio-gpu) allocates through
//! dumb buffers, which a render node refuses (`CREATE_DUMB: Permission
//! denied`) and a primary node allows. On a real GPU the render node
//! answers first.
//!
//! The startup probe allocates one small buffer the way the swapchain will,
//! checks the modifier it came back with is one the host listed (GBM, and
//! Smithay's allocator in front of it, fall back to an implicit layout
//! silently -- see [`feedback::Choice::accept`]), then has the renderer bind
//! it and blit into it. That turns a driver that cannot do either -- no
//! GLES 3 `glBlitFramebuffer`, a layout it imports but cannot render into --
//! into a startup read-back decision rather than a failed first frame.
//!
//! # The swapchain
//!
//! Up to [`SLOTS`] host buffers per size. A chain is created with one, at
//! the size the host configured, and is replaced as a whole on a resize
//! (after `Host::replace_render_target`'s same allocate-first ordering, so a
//! failed allocation leaves the old chain and the old size intact); a
//! buffer is added only when a frame finds every existing one held by the
//! host ([`GpuPresent::grow`]). A drag gives each size a frame or two, so
//! allocating all of them up front paid -- in GBM allocations and host
//! imports -- for buffers most sizes never used. Each is shared through
//! `zwp_linux_buffer_params_v1.create`, **not** `create_immed`: the host
//! refusing an import is then a `failed` event rather than a fatal protocol
//! error on scoot's own host connection, which would end the nested session
//! and every client in it. The requests themselves are built only from what
//! the host listed (the fourcc and the exact modifier, one `add` per plane
//! exactly as GBM exported it, no flags), so the remaining fatal errors --
//! malformed params -- are not reachable from here.
//!
//! A slot is usable once the host has answered `created` and not attached
//! since its last `release`. A frame with no usable slot (all held by the
//! host, or still being created) is not presented but **owed**: the render
//! target already holds it, and the next `release` or `created` copies it
//! over as it stands rather than drawing it again -- which is also what
//! every resize's first frame does, being drawn before the host has created
//! the new chain. At most [`SLOTS`] buffers per size, whatever the host
//! does. Three because a host that samples the buffer on its GPU may hold
//! the previous one until its own frame using the current one completes.
//!
//! A `created` or `failed` answer carries the generation of the chain it
//! was asked for; one for a chain a resize has since replaced has its
//! `wl_buffer` destroyed on arrival and is otherwise ignored.
//!
//! # Falling back mid-session
//!
//! The host refusing a buffer (`failed` for the current chain), the blit
//! failing on the frame path, or a chain that cannot grow when every buffer
//! it has is held (nothing would ever hand the waiting frame over: a dma-buf
//! host keeps the buffer it shows until a newer one replaces it) switches
//! the session to read-back **for good**,
//! with one WARN: a host that refused once is not asked again, and nothing
//! on the frame path retries a device that has just failed. The switch
//! builds a `wl_shm` pool at the current size and asks for a frame; should
//! even that pool fail to build, the next frame that draws -- anything on
//! screen changing -- tries again, so the window is never left stranded on
//! a pool that could now be built.
//! A failed allocation of a *new size's* chain is not a fallback: it is the same
//! "could not follow the host's resize; staying at the previous size" a
//! read-back pool that could not be allocated has always been, and the
//! chain the session already has keeps presenting.
//!
//! # What does not change
//!
//! - **Captures.** `screenshot` and `ext-image-copy-capture-v1` read the
//!   render target (`Backend::capture`), which every frame is still drawn
//!   into; the blit only copies it out.
//! - **Damage.** The whole frame is blitted, and the frame's damage is
//!   passed to `wl_surface.damage_buffer` -- which today is always the full
//!   frame, because `--nested` renders with buffer age 0 (see
//!   `render::draw_frame_with`), exactly as the read-back path reports it.
//! - **Pacing.** `--nested` has never used host frame callbacks: frames are
//!   paced by scoot's own frame timer, and this path keeps that.
//! - **Resize.** A GLES resize still reallocates the render target in place
//!   (PR #232) and still refuses a size over the context's limit before
//!   anything is allocated -- now before the host buffers too.
//! - **Synchronisation.** The blit's sync point is waited on before the
//!   commit, so the host never samples a half-copied buffer -- the same
//!   blocking the read-back's `glReadPixels` already did, without the copy.
//!   Passing the fence to the host instead (`linux-drm-syncobj`) would
//!   remove the wait and is not done here.

pub(in crate::compositor) mod feedback;

use std::error::Error;
use std::os::fd::{AsFd, OwnedFd};

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Fourcc};
use smithay::backend::drm::{DrmNode, NodeType};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_buffer::WlBuffer as HostBuffer;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    Flags, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use self::feedback::{Choice, Collector, HostFeedback};
use super::super::State;
use super::super::render::Backend;

/// How many host buffers the dma-buf path keeps per size. See the module doc.
pub(super) const SLOTS: usize = 3;

/// The side of the startup probe's buffer: small, since it only proves the
/// device, format and renderer work together, and at least one pixel of
/// every row a real frame's blit touches.
const PROBE_SIDE: i32 = 64;

/// Everything the dma-buf path needs once it has been chosen: the host's
/// global, the allocator on the renderer's device, and what to allocate.
pub(super) struct GpuPresent {
    dmabuf: ZwpLinuxDmabufV1,
    allocator: GbmAllocator<OwnedFd>,
    choice: Choice,
    /// The generation the next [`Swapchain`] gets. Only ever incremented, so
    /// a host answer tagged with any other chain's number is recognisably
    /// stale.
    next_generation: u64,
    /// Makes [`GpuPresent::grow`] fail where it would allocate, for the
    /// suites (`Host::fail_growth_for_test`).
    #[cfg(test)]
    fail_growth: bool,
}

/// The user data on each `zwp_linux_buffer_params_v1`: which chain and slot
/// its answer is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compositor) struct ParamsTag {
    pub(in crate::compositor) generation: u64,
    pub(in crate::compositor) slot: usize,
}

/// Decides, once at startup, whether this session presents to the host by
/// dma-buf -- see the module doc for every way the answer is no. Logs the
/// answer at INFO, with the reason when it is no.
///
/// `backend` is the session's primary render target, asked rather than
/// `State::renderer` because what matters is the renderer that was *built*
/// (its device, its render formats, whether it can blit), not what was
/// asked for.
pub(super) fn negotiate(
    conn: &Connection,
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    backend: &mut Backend,
) -> Option<GpuPresent> {
    match try_negotiate(conn, globals, qh, backend) {
        // Nothing to say under pixman, which has only ever had the one way
        // to present -- the same silence a default build keeps there.
        Err(Refusal::NoGpu) => None,
        Ok(gpu) => {
            tracing::info!(
                format = ?gpu.choice.fourcc,
                modifiers = ?gpu.choice.request,
                node = ?gpu.allocator_node(),
                "nested: presenting to the host by dma-buf (no read-back)"
            );
            Some(gpu)
        }
        Err(Refusal::Because(reason)) => {
            tracing::info!(%reason, "nested: presenting to the host by read-back into wl_shm");
            None
        }
    }
}

/// Why [`negotiate`] kept read-back.
enum Refusal {
    /// The session's renderer has no GPU buffer to hand over (pixman).
    NoGpu,
    /// Anything else, said once at INFO.
    Because(Box<dyn Error>),
}

impl<E: Into<Box<dyn Error>>> From<E> for Refusal {
    fn from(error: E) -> Self {
        Self::Because(error.into())
    }
}

fn try_negotiate(
    conn: &Connection,
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    backend: &mut Backend,
) -> Result<GpuPresent, Refusal> {
    let Some(render_formats) = backend.dmabuf_render_formats() else {
        return Err(Refusal::NoGpu);
    };
    let Some(our_device) = backend.render_node() else {
        return Err("the renderer's EGL device names no DRM node".into());
    };
    // Version 4 exactly: feedback needs 4, and 6 retires `main_device` in
    // favour of a sampling-device flag this does not read.
    let dmabuf: ZwpLinuxDmabufV1 = globals
        .bind(qh, 4..=4, ())
        .map_err(|error| format!("the host has no zwp_linux_dmabuf_v1 at version 4: {error}"))?;
    let feedback = match read_feedback(conn, &dmabuf) {
        Ok(feedback) => feedback,
        Err(error) => {
            dmabuf.destroy();
            return Err(error.into());
        }
    };
    let same = |dev| feedback::same_drm_device(dev, our_device);
    let Some(choice) = feedback::choose(&feedback, same, |format| render_formats.contains(&format))
    else {
        dmabuf.destroy();
        return Err(if same(feedback.main_device) {
            "no buffer format is both listed by the host and renderable by scoot".into()
        } else {
            format!(
                "the host composites on another DRM device ({:#x}) than scoot's renderer ({our_device:#x})",
                feedback.main_device
            )
            .into()
        });
    };
    match open_allocator(our_device, &choice, backend) {
        Ok(allocator) => Ok(GpuPresent {
            dmabuf,
            allocator,
            choice,
            next_generation: 0,
            #[cfg(test)]
            fail_growth: false,
        }),
        Err(error) => {
            dmabuf.destroy();
            Err(error.into())
        }
    }
}

/// The host's default feedback, read on a private event queue and a
/// blocking round trip: the same kind of wait the registry bootstrap already
/// did against the same host, once, at startup. Nothing on `State`'s queue
/// is dispatched meanwhile; it is only queued.
fn read_feedback(
    conn: &Connection,
    dmabuf: &ZwpLinuxDmabufV1,
) -> Result<HostFeedback, Box<dyn Error>> {
    let mut queue = conn.new_event_queue::<Collector>();
    let object = dmabuf.get_default_feedback(&queue.handle(), ());
    let mut collector = Collector::default();
    let round_trip = queue.roundtrip(&mut collector);
    object.destroy();
    round_trip.map_err(|error| format!("the host did not answer: {error}"))?;
    Ok(collector.finish()?)
}

/// GBM on the renderer's device, render node first and then the primary
/// node (see the module doc), keeping the first that passes [`probe`].
fn open_allocator(
    render_node: libc::dev_t,
    choice: &Choice,
    backend: &mut Backend,
) -> Result<GbmAllocator<OwnedFd>, Box<dyn Error>> {
    let render = DrmNode::from_dev_id(render_node)
        .map_err(|error| format!("the renderer's DRM node is not usable: {error}"))?;
    let mut nodes = vec![render];
    if let Some(Ok(primary)) = render.node_with_type(NodeType::Primary) {
        nodes.push(primary);
    }
    let mut failures = Vec::with_capacity(nodes.len());
    for node in nodes {
        match open_node(&node).and_then(|mut allocator| {
            probe(&mut allocator, choice, backend)?;
            Ok(allocator)
        }) {
            Ok(allocator) => return Ok(allocator),
            Err(error) => failures.push(format!("{node}: {error}")),
        }
    }
    Err(format!(
        "no node of the renderer's device can serve host buffers ({})",
        failures.join("; ")
    )
    .into())
}

pub(in crate::compositor) fn open_node(
    node: &DrmNode,
) -> Result<GbmAllocator<OwnedFd>, Box<dyn Error>> {
    let path = node
        .dev_path()
        .ok_or_else(|| format!("{node} has no device path"))?;
    // A plain open, not through a session: neither node needs DRM master for
    // allocation, and `--nested` has no seat to go through anyway. Handed to
    // GBM as the bare fd, deliberately not as Smithay's `DrmDeviceFd`: that
    // wrapper tries to become DRM master on construction (`device/fd.rs` at
    // the pinned rev) and logs a WARN when it cannot -- which a nested
    // session inside a desktop never can, and an allocator never needs.
    let fd = rustix::fs::open(
        &path,
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("{}: {error}", path.display()))?;
    let device =
        GbmDevice::new(fd).map_err(|error| format!("GBM on {}: {error}", path.display()))?;
    Ok(GbmAllocator::new(device, GbmBufferFlags::RENDERING))
}

/// One buffer allocated the way the swapchain will, rendered into and
/// blitted to by the session's renderer.
fn probe(
    allocator: &mut GbmAllocator<OwnedFd>,
    choice: &Choice,
    backend: &mut Backend,
) -> Result<(), Box<dyn Error>> {
    let mut dmabuf = allocate(allocator, choice, PROBE_SIDE, PROBE_SIDE)?;
    backend.copy_frame_into(&mut dmabuf)?;
    Ok(())
}

/// One host buffer of `width` x `height`, exported, with the modifier it
/// actually came back with checked against what the host accepts.
pub(in crate::compositor) fn allocate(
    allocator: &mut GbmAllocator<OwnedFd>,
    choice: &Choice,
    width: i32,
    height: i32,
) -> Result<Dmabuf, Box<dyn Error>> {
    let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(format!("{width}x{height} is not a buffer size").into());
    };
    let buffer = allocator
        .create_buffer(w, h, choice.fourcc, &choice.request)
        .map_err(|error| format!("GBM could not allocate {width}x{height}: {error}"))?;
    let modifier = buffer.format().modifier;
    if !choice.accept.contains(&modifier) {
        return Err(format!(
            "GBM allocated {:?} at {modifier:?}, which the host did not list",
            choice.fourcc
        )
        .into());
    }
    buffer
        .export()
        .map_err(|error| format!("could not export the host buffer: {error}").into())
}

impl GpuPresent {
    fn allocator_node(&self) -> Option<DrmNode> {
        DrmNode::from_file(self.allocator.as_fd()).ok()
    }

    /// A new chain at `width` x `height`, holding one buffer: the rest are
    /// added only once a frame finds every existing one in use
    /// ([`GpuPresent::grow`]). A drag gives each size it passes through a
    /// frame or two, so allocating all [`SLOTS`] up front paid for buffers
    /// most sizes never used -- and a host-side import for each.
    pub(super) fn swapchain(
        &mut self,
        qh: &QueueHandle<State>,
        width: i32,
        height: i32,
    ) -> Result<Swapchain, Box<dyn Error>> {
        let dmabuf = allocate(&mut self.allocator, &self.choice, width, height)?;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1);
        let mut chain = Swapchain {
            generation,
            size: (width, height),
            slots: std::array::from_fn(|_| None),
        };
        self.install(&mut chain, qh, 0, dmabuf);
        Ok(chain)
    }

    /// Adds one buffer to `chain`, if [`next_slot`] says a frame is waiting
    /// on one it could have: every existing buffer held by the host, none
    /// still being created, and room below [`SLOTS`]. Answers whether it
    /// did. Bounded by construction -- a chain never holds more than
    /// [`SLOTS`] -- and reached only from a frame that found no free buffer,
    /// never per frame otherwise.
    ///
    /// An allocation that fails is answered `Err`, and the caller falls
    /// back to read-back for good (`Host::present_dmabuf`): this is only
    /// tried when every buffer is held and none is being created, so no
    /// `release` or `created` would ever come to hand the frame over.
    pub(super) fn grow(
        &mut self,
        chain: &mut Swapchain,
        qh: &QueueHandle<State>,
    ) -> Result<bool, Box<dyn Error>> {
        let Some(index) = next_slot(
            chain
                .slots
                .iter()
                .map(|slot| slot.as_ref().map(|slot| (slot.buffer.is_some(), slot.held))),
        ) else {
            return Ok(false);
        };
        #[cfg(test)]
        if self.fail_growth {
            return Err("injected growth failure".into());
        }
        let (width, height) = chain.size;
        let dmabuf = allocate(&mut self.allocator, &self.choice, width, height)?;
        self.install(chain, qh, index, dmabuf);
        Ok(true)
    }

    /// Puts `dmabuf` in `chain`'s slot `index` and asks the host to make a
    /// `wl_buffer` of it; the answer is tagged with the chain's generation
    /// and the slot.
    fn install(
        &mut self,
        chain: &mut Swapchain,
        qh: &QueueHandle<State>,
        index: usize,
        dmabuf: Dmabuf,
    ) {
        let Some(entry) = chain.slots.get_mut(index) else {
            return;
        };
        let params = self.dmabuf.create_params(
            qh,
            ParamsTag {
                generation: chain.generation,
                slot: index,
            },
        );
        let (width, height) = chain.size;
        share(&params, &dmabuf, width, height, self.choice.fourcc);
        *entry = Some(Slot {
            dmabuf,
            buffer: None,
            held: false,
        });
    }

    #[cfg(test)]
    pub(super) fn fail_growth_for_test(&mut self) {
        self.fail_growth = true;
    }

    /// Releases the host's global. Called when the path is abandoned for the
    /// session (see [`Swapchain`]'s fallback).
    pub(super) fn destroy(self) {
        self.dmabuf.destroy();
    }
}

/// Asks the host to make a `wl_buffer` of `dmabuf`: one `add` per plane,
/// exactly as exported, each with the buffer's own modifier; `create` with
/// no flags.
fn share(
    params: &ZwpLinuxBufferParamsV1,
    dmabuf: &Dmabuf,
    width: i32,
    height: i32,
    fourcc: Fourcc,
) {
    let modifier = u64::from(dmabuf.format().modifier);
    // The protocol's split: high word, then low word. The truncating casts
    // are the split itself.
    let (hi, lo) = ((modifier >> 32) as u32, modifier as u32);
    for (index, ((fd, offset), stride)) in dmabuf
        .handles()
        .zip(dmabuf.offsets())
        .zip(dmabuf.strides())
        .enumerate()
    {
        // At most four planes (Smithay's `MAX_PLANES`), so the index fits.
        params.add(fd, index as u32, offset, stride, hi, lo);
    }
    params.create(width, height, fourcc as u32, Flags::empty());
}

/// The host buffers one size's frames go out in. See the module doc.
pub(super) struct Swapchain {
    generation: u64,
    size: (i32, i32),
    /// Filled from the front as frames need them ([`GpuPresent::grow`]).
    slots: [Option<Slot>; SLOTS],
}

struct Slot {
    /// What the renderer blits into. Holds the buffer's fds, which is all
    /// that keeps it alive on scoot's side.
    dmabuf: Dmabuf,
    /// The host's `wl_buffer` for it, once `created` has arrived.
    buffer: Option<HostBuffer>,
    /// Attached and not yet released by the host.
    held: bool,
}

impl Swapchain {
    pub(super) fn size(&self) -> (i32, i32) {
        self.size
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    /// The slot a frame can go out in: created and not held by the host.
    pub(super) fn free_slot(&mut self) -> Option<(&mut Dmabuf, &HostBuffer, &mut bool)> {
        let index = usable(self.slots.iter().map(|slot| {
            slot.as_ref()
                .map_or((false, false), |slot| (slot.buffer.is_some(), slot.held))
        }))?;
        let slot = self.slots.get_mut(index)?.as_mut()?;
        let buffer = slot.buffer.as_ref()?;
        Some((&mut slot.dmabuf, buffer, &mut slot.held))
    }

    /// Records the host's `created` for slot `slot` of this chain. A slot
    /// that already has a buffer (a host answering twice), or that holds
    /// nothing to be created, keeps what it has and the new buffer is
    /// destroyed.
    pub(super) fn created(&mut self, slot: usize, buffer: HostBuffer) {
        match self.slots.get_mut(slot) {
            Some(Some(entry)) if entry.buffer.is_none() => entry.buffer = Some(buffer),
            _ => buffer.destroy(),
        }
    }

    /// Marks the slot backing `buffer` free again. A no-op for a buffer that
    /// is not this chain's (one from a chain a resize replaced).
    pub(super) fn mark_released(&mut self, buffer: &HostBuffer) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|slot| slot.buffer.as_ref() == Some(buffer))
        {
            slot.held = false;
        }
    }

    /// Destroys the host buffers this chain has been given. Answers still
    /// in flight are handled when they arrive: they carry this chain's
    /// generation, which is no longer current, so their buffers are
    /// destroyed then.
    pub(super) fn destroy(self) {
        for slot in self.slots.into_iter().flatten() {
            if let Some(buffer) = slot.buffer {
                buffer.destroy();
            }
        }
    }
}

/// The index of the first slot that is created and not held. A free
/// function over plain values so it is testable without host objects, same
/// rationale as `buffers.rs`'s `first_free`.
fn usable(slots: impl Iterator<Item = (bool, bool)>) -> Option<usize> {
    slots
        .enumerate()
        .find(|&(_, (created, held))| created && !held)
        .map(|(index, _)| index)
}

/// Where a new buffer should go, given each slot as `None` (empty) or
/// `Some((created, held))` -- or `None` when growing would not help: a
/// buffer is already free, one is still being created (its `created` will
/// hand the waiting frame over), or every slot is filled.
fn next_slot(slots: impl Iterator<Item = Option<(bool, bool)>>) -> Option<usize> {
    let mut empty = None;
    for (index, slot) in slots.enumerate() {
        match slot {
            None => {
                empty = empty.or(Some(index));
            }
            Some((true, false)) | Some((false, _)) => return None,
            Some((true, true)) => {}
        }
    }
    empty
}

#[cfg(test)]
mod tests;
