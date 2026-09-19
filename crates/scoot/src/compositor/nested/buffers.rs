//! Host-side `wl_shm` buffers: the small double-buffer flexwm presents
//! through. Two buffers so the host can still be compositing one while
//! flexwm writes the next -- writing into one the host hasn't released yet
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
    pub fn new(
        shm: &HostShm,
        qh: &QueueHandle<State>,
        width: i32,
        height: i32,
    ) -> Result<Self, Box<dyn Error>> {
        let stride = width as usize * 4;
        let frame_len = stride * height as usize;
        let total_len = frame_len * COUNT;

        let mem = MappedMem::new(total_len)?;
        let pool = shm.create_pool(mem.fd.as_fd(), total_len as i32, qh, ());
        let slots = std::array::from_fn(|i| {
            let offset = i * frame_len;
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
        let fd = rustix::fs::memfd_create("flexwm-nested", rustix::fs::MemfdFlags::CLOEXEC)?;
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
}
