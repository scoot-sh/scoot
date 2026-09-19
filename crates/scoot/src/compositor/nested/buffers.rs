//! Host-side `wl_shm` buffers: the small double-buffer scoot presents
//! through. Two buffers so the host can still be compositing one while
//! scoot writes the next -- writing into one the host hasn't released yet
//! would tear or show garbage.

use std::error::Error;
use std::os::fd::AsFd;

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer as HostBuffer;
use wayland_client::protocol::wl_shm::{Format, WlShm as HostShm};

use super::super::State;

const COUNT: usize = 2;

pub struct BufferPool {
    slots: [Slot; COUNT],
    frame_len: usize,
    // Kept only so the mapping's backing memory outlives this pool; nothing
    // reads from it after `new()`.
    _mem: MappedMem,
}

struct Slot {
    buffer: HostBuffer,
    /// Byte offset of this slot's frame within the mapping.
    offset: usize,
    free: bool,
}

impl BufferPool {
    /// Builds a pool of [`COUNT`] `width` x `height` `Argb8888` buffers.
    ///
    /// `Err` rather than a truncated pool for a size whose byte count cannot
    /// be named: `wl_shm.create_pool` takes an `i32`, and a plain `as i32`
    /// on a bigger count would hand the host a wrong (possibly negative)
    /// size and be killed by it for a protocol error -- a compositor death
    /// from a size the host itself proposed. Reachable from inside
    /// `nested.rs`'s own `1..=65535`-per-axis guard (65535x8192 needs more
    /// than `i32::MAX` bytes for two slots), so this checks rather than
    /// assumes. The caller decides what a failure means: fatal on the first
    /// configure, "stay at the old size" on a resize (see
    /// `Host::apply_resize`).
    pub fn new(
        shm: &HostShm,
        qh: &QueueHandle<State>,
        width: i32,
        height: i32,
    ) -> Result<Self, Box<dyn Error>> {
        let Layout {
            stride,
            frame_len,
            total_len,
            wire_len,
        } = layout(width, height).ok_or_else(|| {
            format!("a {width}x{height} host buffer pool is larger than wl_shm can describe")
        })?;

        let mem = MappedMem::new(total_len)?;
        let pool = shm.create_pool(mem.fd.as_fd(), wire_len, qh, ());
        let slots = std::array::from_fn(|i| {
            let offset = i * frame_len;
            // Both `as i32`s are in range by the checks above: `offset` and
            // `stride` are each at most `total_len`, which `wire_len` just
            // proved fits.
            let buffer = pool.create_buffer(
                offset as i32,
                width,
                height,
                stride as i32,
                Format::Argb8888,
                qh,
                (),
            );
            Slot {
                buffer,
                offset,
                free: true,
            }
        });
        // The pool only exists to mint buffers; once they're created it has
        // nothing left to do, and the buffers keep the underlying mapping
        // alive on the host side regardless.
        pool.destroy();

        Ok(Self {
            slots,
            frame_len,
            _mem: mem,
        })
    }

    /// Writes `pixels` into whichever slot is free and returns that slot's
    /// buffer to attach, marking it in-flight until `mark_released` is
    /// called for it. `None` if no slot is currently free, or if
    /// `pixels.len()` doesn't match this pool's frame size.
    pub fn write_free(&mut self, pixels: &[u8]) -> Option<&HostBuffer> {
        if pixels.len() != self.frame_len {
            return None;
        }
        let index = free_slot(&self.slots)?;
        let slot = &mut self.slots[index];
        // Safety: `offset..offset + frame_len` is inside the mapping created
        // for exactly this many slots of exactly this size in `new()`, and
        // this slot is marked free -- by construction (see `mark_released`)
        // that means the host has released whatever buffer previously lived
        // here, so nothing else can be reading this range concurrently.
        unsafe {
            self._mem.write(slot.offset, pixels);
        }
        slot.free = false;
        Some(&slot.buffer)
    }

    /// Marks the slot backing `buffer` free again, once the host has sent
    /// `wl_buffer::Event::Release` for it. A no-op if `buffer` doesn't match
    /// any slot (e.g. an event for a buffer from a since-replaced pool after
    /// a resize -- see `nested::Host::apply_size`).
    pub fn mark_released(&mut self, buffer: &HostBuffer) {
        if let Some(slot) = self.slots.iter_mut().find(|slot| &slot.buffer == buffer) {
            slot.free = true;
        }
    }

    /// Destroys this pool's buffers on the host side. Call before dropping a
    /// pool that's being replaced (a resize), so the old objects don't leak
    /// host-side -- generated `wl_buffer` proxies have no destructor of
    /// their own, they only stop being useful once dropped locally.
    pub fn destroy(self) {
        for slot in self.slots {
            slot.buffer.destroy();
        }
    }
}

fn free_slot(slots: &[Slot; COUNT]) -> Option<usize> {
    first_free(slots.iter().map(|slot| slot.free))
}

/// The byte layout of a pool of [`COUNT`] `width` x `height` `Argb8888`
/// frames.
struct Layout {
    /// Bytes per row of one frame.
    stride: usize,
    /// Bytes in one frame, which is also the stride between slots.
    frame_len: usize,
    /// Bytes in the whole pool, for the mapping.
    total_len: usize,
    /// The same count as `wl_shm.create_pool` takes it. Carried separately
    /// rather than cast at the call site so the conversion happens once,
    /// where it is checked.
    wire_len: i32,
}

/// Computes [`Layout`], or `None` for a size whose byte count cannot be
/// described.
///
/// Every step is checked and the total has to fit the `i32`
/// `wl_shm.create_pool` takes. A plain `as i32` on a bigger count would hand
/// the host a wrong (possibly negative) pool size and get this compositor
/// killed for a protocol error -- death by a size the host itself proposed.
/// It is reachable from inside `nested.rs`'s own `1..=65535`-per-axis guard:
/// 65535x8192 is two slots of 2.1 GB. A negative axis is refused the same
/// way, since `width as usize` on one is how a pool comes to ask for sixteen
/// exabytes.
///
/// A free function so the arithmetic is testable without a live host
/// connection, same rationale as [`first_free`].
fn layout(width: i32, height: i32) -> Option<Layout> {
    let stride = usize::try_from(width).ok()?.checked_mul(4)?;
    let frame_len = stride.checked_mul(usize::try_from(height).ok()?)?;
    let total_len = frame_len.checked_mul(COUNT)?;
    let wire_len = i32::try_from(total_len).ok()?;
    Some(Layout {
        stride,
        frame_len,
        total_len,
        wire_len,
    })
}

/// The index of the first `true`. Pulled out of `free_slot` so it's testable
/// against plain bools, without needing a live host connection to build real
/// `Slot`s (which hold a `wayland_client` proxy, not constructible in a unit
/// test).
fn first_free(free: impl Iterator<Item = bool>) -> Option<usize> {
    free.enumerate().find(|&(_, free)| free).map(|(i, _)| i)
}

/// One memfd-backed, mmap'd region sized for `COUNT` frames back to back.
struct MappedMem {
    fd: std::os::fd::OwnedFd,
    ptr: *mut u8,
    len: usize,
}

impl MappedMem {
    fn new(len: usize) -> Result<Self, Box<dyn Error>> {
        let fd = rustix::fs::memfd_create("scoot-nested", rustix::fs::MemfdFlags::CLOEXEC)?;
        rustix::fs::ftruncate(&fd, len as u64)?;
        // Safety: `fd` was just created and truncated to exactly `len` bytes
        // above, and nothing else holds a mapping of it yet -- this is a
        // fresh, exclusively-held memfd.
        let ptr = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE,
                rustix::mm::MapFlags::SHARED,
                &fd,
                0,
            )?
        };
        Ok(Self {
            fd,
            ptr: ptr.cast(),
            len,
        })
    }

    /// Safety: the caller must ensure `offset..offset + data.len()` is
    /// within this mapping and that nothing else (this process or the host,
    /// via an unreleased buffer) is concurrently reading or writing that
    /// range.
    unsafe fn write(&mut self, offset: usize, data: &[u8]) {
        debug_assert!(offset + data.len() <= self.len);
        // Safety: forwarded from the caller, plus the bounds check above.
        unsafe {
            let dst = self.ptr.add(offset);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
        }
    }
}

impl Drop for MappedMem {
    fn drop(&mut self) {
        // Safety: `ptr`/`len` are exactly what `mmap` returned/was given in
        // `new()`, unmodified since, and this is the only unmap of them.
        unsafe {
            let _ = rustix::mm::munmap(self.ptr.cast(), self.len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_free_picks_the_first_true() {
        assert_eq!(first_free([false, false].into_iter()), None);
        assert_eq!(first_free([true, false].into_iter()), Some(0));
        assert_eq!(first_free([false, true].into_iter()), Some(1));
        assert_eq!(first_free([true, true].into_iter()), Some(0));
        assert_eq!(first_free(std::iter::empty()), None);
    }

    #[test]
    fn an_ordinary_size_lays_out_two_slots_back_to_back() {
        let layout = layout(1280, 800).expect("1280x800 fits");
        assert_eq!(layout.stride, 1280 * 4);
        assert_eq!(layout.frame_len, 1280 * 4 * 800);
        assert_eq!(layout.wire_len, 1280 * 4 * 800 * 2);
    }

    #[test]
    fn the_largest_describable_pool_is_accepted() {
        // Two slots totalling eight bytes short of `i32::MAX`, to pin that
        // the bound is "fits an `i32`" and not "fits with room to spare".
        let frame = usize::try_from(i32::MAX).expect("i32::MAX fits a usize") / 2;
        let width = 1;
        let height = i32::try_from(frame / 4).expect("a plausible height");
        let layout = layout(width, height).expect("half of i32::MAX per frame fits");
        assert_eq!(layout.wire_len, height * 8);
    }

    #[test]
    fn a_pool_bigger_than_wl_shm_can_describe_is_refused() {
        // Both inside `nested.rs`'s `1..=65535` per-axis guard, and together
        // past what an `i32` byte count can name: 65535 * 4 * 8192 * 2 is
        // about 4.3 GB. Truncating this to an `i32` is what the check exists
        // to prevent.
        assert!(layout(65535, 8192).is_none());
        assert!(layout(65535, 65535).is_none());
    }

    #[test]
    fn a_negative_axis_is_refused_rather_than_sign_extended() {
        // `-1 as usize` is `usize::MAX`; the `try_from` is what stops a
        // 16-exabyte `ftruncate` request.
        assert!(layout(-1, 800).is_none());
        assert!(layout(1280, -1).is_none());
        assert!(layout(i32::MIN, i32::MIN).is_none());
    }

    #[test]
    fn a_zero_axis_lays_out_an_empty_pool_rather_than_overflowing() {
        // `nested.rs` refuses a zero axis before this is reached, and
        // `--width`/`--height` cannot be zero either; this pins that the
        // arithmetic itself is still total if that ever changes.
        let layout = layout(0, 800).expect("zero width is arithmetically fine");
        assert_eq!(layout.wire_len, 0);
    }
}
