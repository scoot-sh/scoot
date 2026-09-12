//! DRM dumb-buffer double-buffering: the two GPU-visible buffers flexwm's
//! rendered frame gets copied into for real scanout.
//!
//! Structured like `nested/buffers.rs`'s pool -- two slots, write into
//! whichever is free, mark free again once the write is confirmed safe to
//! reuse -- but confirmation here is a `DrmEvent::VBlank` (see
//! `tty::Tty::on_vblank`), not a `wl_buffer::Release`, and the buffers
//! themselves are DRM dumb buffers, not wl_shm. That's close enough in
//! shape to reuse the free-tracking idea, not close enough (different
//! confirmation source, different pitch handling, no host-side protocol
//! object) to justify forcing a shared abstraction between the two -- see
//! this project's "no premature abstraction" standard.
//!
//! No GBM in this rev of Smithay for dumb buffers (see the crate's
//! `Cargo.toml` comment on `backend_gbm`), so presenting a frame is a plain
//! memcpy from the pixman-rendered framebuffer into whichever dumb buffer is
//! free -- row by row, because a dumb buffer's pitch (its driver-chosen
//! stride) is not guaranteed to equal `width * 4`, unlike the wl_shm buffers
//! flexwm creates itself in `nested/buffers.rs` and can assume are tightly
//! packed. Getting this wrong is exactly the kind of bug that only shows up
//! on real hardware, never in this VM's happens-to-be-aligned case.
//!
//! Every mapping here goes through drm-rs's own `map_dumb_buffer`, which
//! wraps the mmap in a safe, `Drop`-unmapped `DumbMapping` -- unlike
//! `nested/buffers.rs`'s `MappedMem`, there is no raw pointer or `unsafe`
//! block in this file to write a safety comment for.

use std::error::Error;

use smithay::backend::allocator::dumb::{DumbAllocator, DumbBuffer};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::dumb::{DumbFramebuffer, framebuffer_from_dumb_buffer};
use smithay::reexports::drm::buffer::Buffer as DrmBuffer;
use smithay::reexports::drm::control::{Device as ControlDevice, framebuffer};

const COUNT: usize = 2;

pub struct BufferPool {
    fd: DrmDeviceFd,
    slots: [Slot; COUNT],
    width: i32,
    height: i32,
}

struct Slot {
    buffer: DumbBuffer,
    framebuffer: DumbFramebuffer,
    free: bool,
}

impl BufferPool {
    pub fn new(fd: &DrmDeviceFd, width: i32, height: i32) -> Result<Self, Box<dyn Error>> {
        let mut allocator = DumbAllocator::new(fd.clone());
        let slots = [
            make_slot(fd, &mut allocator, width, height)?,
            make_slot(fd, &mut allocator, width, height)?,
        ];
        Ok(Self {
            fd: fd.clone(),
            slots,
            width,
            height,
        })
    }

    /// Writes `pixels` (tightly packed Argb8888/Xrgb8888, `width * height *
    /// 4` bytes) into whichever slot is free, and returns that slot's index
    /// (to pass back to [`mark_free`](Self::mark_free) once scanned out)
    /// and framebuffer handle (to hand to `DrmSurface::commit`/`page_flip`).
    /// `None` if no slot is currently free -- the previous flip hasn't been
    /// confirmed by a `VBlank` yet -- or if `pixels` isn't sized for this
    /// pool's `width`/`height`.
    pub fn write_free(&mut self, pixels: &[u8]) -> Option<(usize, framebuffer::Handle)> {
        let row_len = self.width as usize * 4;
        if pixels.len() != row_len.checked_mul(self.height as usize)? {
            return None;
        }
        let index = first_free(self.slots.iter().map(|slot| slot.free))?;
        let slot = &mut self.slots[index];
        // `handle()` returns drm-rs's own `Copy` handle type, not the
        // `Send`-ineligible mapping itself -- this local copy is what
        // `map_dumb_buffer` wants (it needs `&mut` only to tie the
        // mapping's lifetime to it, not because mapping mutates the
        // handle).
        let mut handle = *slot.buffer.handle();
        let pitch = handle.pitch() as usize;
        let Ok(mut mapping) = self.fd.map_dumb_buffer(&mut handle) else {
            return None;
        };
        // Never assume `mapping.len() == pitch * height`: the kernel rounds
        // a dumb buffer's total allocation up to a page boundary, so the
        // mapping is typically a little *longer* than that even when
        // `pitch == row_len` -- this bit real hardware in this VM (pitch
        // exactly `width * 4`, mapping still padded to the next 4096) the
        // first time this ran, where nested's wl_shm buffers (self-sized,
        // never padded like this) could never have caught it. Copying row
        // by row via `chunks_exact_mut` sidesteps the whole question: it
        // takes exactly `height` chunks of `pitch` bytes from the front of
        // the mapping and ignores whatever, if anything, follows them.
        for (src_row, dst_row) in pixels
            .chunks_exact(row_len)
            .zip(mapping.chunks_exact_mut(pitch))
        {
            dst_row[..row_len].copy_from_slice(src_row);
        }
        slot.free = false;
        Some((index, *slot.framebuffer.as_ref()))
    }

    /// Marks a slot free again once its previous contents are confirmed off
    /// screen (a `VBlank` for the flip that replaced it). A no-op if
    /// `index` is out of range, which cannot happen through normal use but
    /// avoids a panic if `tty.rs` and this pool's slot count ever drift.
    pub fn mark_free(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.free = true;
        }
    }

    /// Marks every slot free. Used after a session reactivation: the
    /// surface's own scanout state is gone (see `Tty::reactivate`), so
    /// whatever was "showing" before is no longer meaningfully in flight.
    pub fn mark_all_free(&mut self) {
        for slot in &mut self.slots {
            slot.free = true;
        }
    }
}

fn make_slot(
    fd: &DrmDeviceFd,
    allocator: &mut DumbAllocator,
    width: i32,
    height: i32,
) -> Result<Slot, Box<dyn Error>> {
    // Xrgb8888: the same little-endian BGRx byte layout as the pixman
    // Argb8888 image `headless.rs` renders into and reads back (see its
    // module doc and screenshot.rs's comment on the same fact) -- KMS
    // doesn't care whether the unused top byte means "ignored" or "alpha",
    // only that the channel order underneath matches, and it does.
    let buffer = allocator.create_buffer(
        width as u32,
        height as u32,
        Fourcc::Xrgb8888,
        &[Modifier::Linear],
    )?;
    let framebuffer = framebuffer_from_dumb_buffer(fd, &buffer, false)?;
    Ok(Slot {
        buffer,
        framebuffer,
        free: true,
    })
}

/// The index of the first `true`. Pulled out of `write_free` so it's
/// testable against plain bools, without needing a live DRM device to build
/// real `Slot`s -- same rationale as `nested/buffers.rs`'s `first_free`.
fn first_free(free: impl Iterator<Item = bool>) -> Option<usize> {
    free.enumerate().find(|&(_, free)| free).map(|(i, _)| i)
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
