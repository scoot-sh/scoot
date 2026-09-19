//! DRM dumb-buffer double-buffering: the two GPU-visible buffers scoot's
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
//! No GBM on this tier at all -- that is the point of it, and why it is the
//! one that needs no GPU stack (see `dumb.rs`'s module doc and the crate's
//! `Cargo.toml` comment on the `gpu-scanout` feature). So presenting a frame
//! here is a plain
//! memcpy from the rendered framebuffer into whichever dumb buffer is
//! free -- row by row, because a dumb buffer's pitch (its driver-chosen
//! stride) is not guaranteed to equal `width * 4`, unlike the wl_shm buffers
//! scoot creates itself in `nested/buffers.rs` and can assume are tightly
//! packed. Getting this wrong is exactly the kind of bug that only shows up
//! on real hardware, never in this VM's happens-to-be-aligned case.
//!
//! Every mapping here goes through drm-rs's own `map_dumb_buffer`, which
//! wraps the mmap in a safe, `Drop`-unmapped `DumbMapping` -- unlike
//! `nested/buffers.rs`'s `MappedMem`, there is no raw pointer or `unsafe`
//! block in this file to write a safety comment for.
//!
//! # Per-slot buffer age (added for cursor rendering)
//!
//! Cursor motion redraws far more often than any other change this project
//! makes (see `cursor.rs`'s module doc), so a naive "copy the whole frame
//! every time" here would turn every mouse-motion event into a multi-MB/s
//! memcpy. [`write_region`](BufferPool::write_region) instead copies only
//! the caller-supplied damage rectangle, and [`next_age`](BufferPool::next_age)
//! tells the caller (`render::draw_frame_with`, via `Tty::next_buffer_age`) how
//! many `render_output` calls out of date whichever slot is about to be
//! written actually is -- not always 1, because the two slots alternate:
//! if slot A was last written 2 renders ago (slot B took the render in
//! between), the pixels currently in A are 2 renders stale, and
//! `OutputDamageTracker` needs to be told exactly that or it hands back
//! only last render's incremental damage, which isn't enough to bring A
//! up to date. See `tty::Tty::next_buffer_age`/`advance_generation`'s docs
//! for the call sequence this depends on.

use std::error::Error;

use smithay::backend::allocator::dumb::{DumbAllocator, DumbBuffer};
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use smithay::backend::drm::DrmDeviceFd;
use smithay::backend::drm::dumb::{DumbFramebuffer, framebuffer_from_dumb_buffer};
use smithay::reexports::drm::buffer::Buffer as DrmBuffer;
use smithay::reexports::drm::control::{Device as ControlDevice, framebuffer};
use smithay::utils::{Physical, Rectangle};

const COUNT: usize = 2;

pub struct BufferPool {
    fd: DrmDeviceFd,
    slots: [Slot; COUNT],
    width: i32,
    height: i32,
    ages: SlotAges,
}

struct Slot {
    buffer: DumbBuffer,
    framebuffer: DumbFramebuffer,
    free: bool,
}

/// Which `render_output` generations each dumb-buffer slot's pixels reflect.
///
/// Split out of [`BufferPool`] so the failure transition -- a write that
/// reached a slot but never reached scanout, whose age must read as zero
/// rather than one -- is unit-testable without a live DRM device, the same
/// rationale as `first_free`/`age` below. `BufferPool` delegates every
/// generation bookkeeping call here; nothing else touches these counters.
struct SlotAges {
    /// Advanced once per [`advance_generation`](BufferPool::advance_generation)
    /// call -- see the module doc's "per-slot buffer age" section.
    generation: u64,
    /// The pool's `generation` at the time each slot's pixels were last
    /// written by [`write_region`](BufferPool::write_region), or `None` if
    /// never written, if [`invalidate`](SlotAges::invalidate) has since run,
    /// or if that write never reached scanout (see
    /// [`write_failed`](SlotAges::write_failed)).
    last_written: [Option<u64>; COUNT],
}

impl SlotAges {
    /// The buffer age to report for `index` -- see [`age`].
    fn age_for(&self, index: usize) -> usize {
        age(self.generation, self.last_written[index])
    }

    /// Advances the generation -- see
    /// [`advance_generation`](BufferPool::advance_generation).
    fn advance(&mut self) {
        self.generation += 1;
    }

    /// Records that `index` was written at the current generation -- see
    /// [`write_region`](BufferPool::write_region).
    fn recorded(&mut self, index: usize) {
        self.last_written[index] = Some(self.generation);
    }

    /// Forces every slot's next pick to report age `0` -- see
    /// [`invalidate_ages`](BufferPool::invalidate_ages).
    fn invalidate(&mut self) {
        self.last_written = [None; COUNT];
    }

    /// Records that `index`'s write never reached scanout (a commit or page
    /// flip the kernel refused after the pixels were already copied into the
    /// slot): the slot keeps whatever bytes the copy left there, but nothing
    /// vouches for what's actually on screen, so its next pick must be a
    /// full redraw rather than trusting the fresh `last_written` the copy
    /// just stored.
    ///
    /// Without this the retry reads as age 1 -- one render out of date, only
    /// this frame's own damage requested -- and a frame with no *new* damage
    /// comes back `None` from the damage tracker (see `render/tests.rs`'s
    /// contract test), so nothing is ever re-presented and scanout stays
    /// stale until unrelated damage arrives.
    fn write_failed(&mut self, index: usize) {
        self.last_written[index] = None;
    }
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
            ages: SlotAges {
                generation: 0,
                last_written: [None; COUNT],
            },
        })
    }

    /// The buffer age to pass `OutputDamageTracker::render_output` for
    /// whichever slot [`write_region`](Self::write_region) would pick next
    /// (the same first-free lookup) -- `0` (forcing a full redraw) if that
    /// slot has no valid history to compute an age from.
    ///
    /// A pure peek: this alone doesn't advance `generation`, so it's safe to
    /// call before deciding whether a render is even worth doing. The
    /// caller must still call [`advance_generation`](Self::advance_generation)
    /// exactly once for every `render_output` call this age was used for --
    /// see that method's doc for why.
    pub fn next_age(&self) -> usize {
        let index = first_free(self.slots.iter().map(|slot| slot.free));
        index.map_or(0, |index| self.ages.age_for(index))
    }

    /// Must be called exactly once after every `render_output` call this
    /// pool's [`next_age`](Self::next_age) was used for, whether or not that
    /// render's damage ends up written anywhere by
    /// [`write_region`](Self::write_region). `OutputDamageTracker`'s own
    /// damage history advances unconditionally on every `render_output`
    /// call (confirmed against the pinned Smithay source: the history push
    /// happens before the "anything actually damaged?" check that might
    /// skip drawing) -- so this generation counter must track it 1:1, or a
    /// later `next_age` computes an age against the wrong baseline and the
    /// tracker hands back damage for the wrong span of history.
    pub fn advance_generation(&mut self) {
        self.ages.advance();
    }

    /// Forces every slot's next pick to report age `0` (a full redraw).
    /// Used after a session reactivation (`Tty::reactivate`): the dumb
    /// buffers' pixel bytes themselves are untouched by a VT switch (they're
    /// just host memory, not GPU state another VT could repurpose), but
    /// what's actually on screen right now is not known to have come from
    /// any generation this pool tracked -- another VT may have driven the
    /// display differently in the meantime. Trusting a slot's pre-pause
    /// `last_written` here would risk presenting only a small incremental
    /// patch onto a screen whose real current content doesn't match what
    /// the tracker assumes, which is exactly the class of bug (a display
    /// left showing stale/wrong pixels with no error anywhere) this
    /// project's standards call out as the worst kind.
    pub fn invalidate_ages(&mut self) {
        self.ages.invalidate();
    }

    /// Writes `pixels` (tightly packed Argb8888/Xrgb8888, exactly
    /// `region.size.w * region.size.h * 4` bytes) into `region` of whichever
    /// slot is free, leaving the rest of that slot's existing contents
    /// alone, and returns that slot's index (to pass back to
    /// [`mark_free`](Self::mark_free) once scanned out) and framebuffer
    /// handle (to hand to `DrmSurface::commit`/`page_flip`). `None` if no
    /// slot is currently free, if `pixels` isn't sized for `region`, or if
    /// `region` doesn't fit within this pool's `width`/`height`.
    pub fn write_region(
        &mut self,
        pixels: &[u8],
        region: Rectangle<i32, Physical>,
    ) -> Option<(usize, framebuffer::Handle)> {
        let row_len = region.size.w as usize * 4;
        if pixels.len() != row_len.checked_mul(region.size.h as usize)? {
            return None;
        }
        if region.loc.x < 0
            || region.loc.y < 0
            || region.loc.x + region.size.w > self.width
            || region.loc.y + region.size.h > self.height
        {
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
            // Logged here, not just left for the caller to infer from a
            // bare `None`: `present()` treats every `None` from this
            // function as "no free slot" for its own log line, but this
            // particular cause (the slot *was* free, the mmap itself
            // failed) is worth distinguishing at the source.
            tracing::warn!("drm: could not map dumb buffer for writing");
            return None;
        };
        // Never assume `mapping.len() == pitch * height`: the kernel rounds
        // a dumb buffer's total allocation up to a page boundary, so the
        // mapping is typically a little *longer* than that even when
        // `pitch == width * 4` -- this bit real hardware in this VM (pitch
        // exactly `width * 4`, mapping still padded to the next 4096) the
        // first time this ran, where nested's wl_shm buffers (self-sized,
        // never padded like this) could never have caught it. Indexing by
        // an explicit byte offset (`y * pitch + x_offset`) rather than
        // `chunks_exact_mut(pitch)` sidesteps the same question while also
        // letting each row land at `region`'s offset instead of row 0.
        let x_offset = region.loc.x as usize * 4;
        let y0 = region.loc.y as usize;
        copy_region_rows(&mut mapping, pitch, x_offset, y0, row_len, pixels);
        slot.free = false;
        self.ages.recorded(index);
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

    /// Releases a slot whose write never reached scanout: a commit or page
    /// flip the kernel refused *after* [`write_region`](Self::write_region)
    /// already copied the pixels in. The slot becomes reusable, but its age
    /// is cleared outright (see [`write_failed`](SlotAges::write_failed)) --
    /// merely freeing it would leave the fresh `last_written` the copy
    /// stored, and the retry would read as age 1 and draw nothing on a quiet
    /// screen. The only caller is `Tty::present`'s commit-failure arm.
    pub fn note_write_failed(&mut self, index: usize) {
        if let Some(slot) = self.slots.get_mut(index) {
            slot.free = true;
            self.ages.write_failed(index);
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
    // Argb8888 image `render/pixman.rs` composites into and `render.rs`
    // reads back (see their module docs and screenshot.rs's comment on the
    // same fact) -- KMS
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

/// The index of the first `true`. Pulled out of `write_region` so it's
/// testable against plain bools, without needing a live DRM device to build
/// real `Slot`s -- same rationale as `nested/buffers.rs`'s `first_free`.
fn first_free(free: impl Iterator<Item = bool>) -> Option<usize> {
    free.enumerate().find(|&(_, free)| free).map(|(i, _)| i)
}

/// The age to report for a slot whose `last_written` generation is given,
/// against the pool's current `generation` -- pulled out of `next_age` so
/// this arithmetic is directly testable, same rationale as `first_free`.
/// `None` (never written) is always age `0` (force a full redraw).
///
/// `last_written` is captured by `write_region` *after*
/// [`advance_generation`](BufferPool::advance_generation) has already run
/// for the render that performed that write (see this module's doc and
/// `render::draw_frame_with`'s call order: `advance_generation` runs before
/// `present`/`write_region`) -- so it equals the total count of completed
/// `render_output` calls through and including that write. The render this
/// age is being computed for hasn't happened yet (this is a peek, taken
/// before that render's own `advance_generation`), so it's one call further
/// on than `generation` currently reads. The number of `render_output`
/// calls whose damage must land in this slot to bring it up to date is
/// therefore `(generation + 1) - last_written`, not `generation -
/// last_written` -- the render about to happen counts too, since its own
/// damage is what's being requested this age for in the first place.
///
/// Getting this wrong by one is silent and easy to miss: every check this
/// project has run so far reads back the pixman intermediate image (which
/// always recomposites correctly regardless of `age`), never the dumb
/// buffer this age actually governs, so an off-by-one here would only ever
/// show up as a stale sliver on real scanout hardware, not in any existing
/// test or IPC screenshot.
fn age(generation: u64, last_written: Option<u64>) -> usize {
    last_written.map_or(0, |last| (generation - last + 1) as usize)
}

/// Copies `pixels` (tightly packed rows, `row_len` bytes each) into `dest`
/// (a full mapped buffer with driver-chosen `pitch` bytes per row) starting
/// at byte column `x_offset` and row `y0`, leaving every byte outside that
/// region untouched. Pulled out of `write_region` so the pitch-aware,
/// offset row-by-row copy math is directly testable against a plain
/// in-memory buffer, without needing a live DRM device to map a real dumb
/// buffer -- same rationale as `first_free`. See this module's doc for why
/// `pitch` can differ from both `x_offset`'s own row width and from
/// `dest`'s total length (kernel page rounding).
fn copy_region_rows(
    dest: &mut [u8],
    pitch: usize,
    x_offset: usize,
    y0: usize,
    row_len: usize,
    pixels: &[u8],
) {
    for (row, src_row) in pixels.chunks_exact(row_len).enumerate() {
        let start = (y0 + row) * pitch + x_offset;
        dest[start..start + row_len].copy_from_slice(src_row);
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
    fn age_of_a_never_written_slot_is_zero() {
        assert_eq!(age(0, None), 0);
        assert_eq!(age(9, None), 0);
    }

    #[test]
    fn age_of_a_slot_written_by_the_immediately_preceding_render_is_one() {
        // Regression for the two-slot alternation this module's doc
        // describes: slot0 is written during render 1 (generation becomes 1
        // *before* the write, per `render::draw_frame_with`'s call order, so
        // `last_written = 1`). Peeking again immediately -- as if render 1
        // were about to be repeated with no other render in between -- must
        // report age 1 (this render's own fresh damage is enough), not 0
        // (which would force a needless full redraw) or 2.
        assert_eq!(age(1, Some(1)), 1);
    }

    #[test]
    fn age_reflects_a_render_that_landed_on_the_other_slot_in_between() {
        // The exact scenario from this module's doc: slot A was last
        // written after render 1 (`last_written = 1`), slot B took render
        // 2, and we're now peeking before render 3 (`generation = 2`, i.e.
        // 2 renders completed so far). Slot A needs both render 2's damage
        // (which it missed) and render 3's own (about to happen) -- age 2,
        // not 1. Getting this wrong by one is exactly the silent
        // stale-pixel bug this project's standards call out as the worst
        // kind: `OutputDamageTracker` would hand back only render 3's own
        // damage, leaving render 2's changes (e.g. the cursor's previous
        // position, which must be erased) never copied into slot A.
        assert_eq!(age(2, Some(1)), 2);
    }

    #[test]
    fn age_grows_by_exactly_one_render_at_a_time() {
        for generation in 0..10u64 {
            assert_eq!(age(generation, Some(0)), (generation + 1) as usize);
        }
    }

    #[test]
    fn slot_ages_track_writes_and_advances_like_the_pool_did() {
        // Parity pin for pulling the counters out of `BufferPool`: one
        // render (advance) plus one write, and the next pick reads as age 1
        // -- the same arithmetic `age()` pins above, now through the struct
        // `next_age`/`advance_generation`/`write_region` delegate to.
        let mut ages = SlotAges {
            generation: 0,
            last_written: [None; COUNT],
        };
        assert_eq!(ages.age_for(0), 0);
        ages.advance();
        ages.recorded(0);
        assert_eq!(ages.age_for(0), 1);
    }

    #[test]
    fn a_write_that_never_reached_scanout_reads_as_age_zero() {
        // Fail-first pin for the present-skip damage loss: the copy stored a
        // fresh `last_written`, but the flip never went out, so the retry
        // must fully redraw (age 0). Merely freeing the slot -- what the
        // commit-failure arm did before this fix -- leaves age 1, and an
        // unchanged frame at age 1 reports no damage at all (see
        // `render/tests.rs`'s contract test), so nothing is ever re-presented.
        // Neuter check: make `write_failed` a no-op and the final assertion
        // reads 1, not 0.
        let mut ages = SlotAges {
            generation: 0,
            last_written: [None; COUNT],
        };
        ages.advance();
        ages.recorded(0);
        assert_eq!(ages.age_for(0), 1);
        ages.write_failed(0);
        assert_eq!(ages.age_for(0), 0);
    }

    #[test]
    fn a_failed_write_leaves_the_other_slots_history_alone() {
        // Precision pin: only the failed slot is unvouched. The innocent
        // slot keeps its age, so a retry that picks it still gets the
        // incremental damage its own history calls for rather than a forced
        // full redraw.
        let mut ages = SlotAges {
            generation: 0,
            last_written: [None; COUNT],
        };
        ages.advance();
        ages.recorded(0);
        ages.advance();
        ages.recorded(1);
        assert_eq!(ages.age_for(0), 2);
        assert_eq!(ages.age_for(1), 1);
        ages.write_failed(1);
        assert_eq!(ages.age_for(1), 0);
        assert_eq!(ages.age_for(0), 2);
    }

    #[test]
    fn copy_region_rows_writes_exact_bytes_and_leaves_the_rest_of_the_buffer_untouched() {
        // A pitch wider than the region's own row width (driver padding)
        // and a `dest` longer than `pitch * height` (kernel page rounding)
        // -- both real hazards this module's doc calls out as having bitten
        // real hardware -- plus a region at a nonzero (x, y) offset, so a
        // bug in either the pitch or the offset arithmetic would land bytes
        // in the wrong place instead of merely failing to compile.
        const PITCH: usize = 24; // wider than any row this test writes
        const DEST_LEN: usize = PITCH * 4 + 37; // padded well past pitch * height
        const SENTINEL: u8 = 0xAA;

        let mut dest = vec![SENTINEL; DEST_LEN];
        // A 2x2 region at byte-column 8 (x_offset), row 1 (y0): distinct,
        // recognizable bytes per pixel so a transposed row/column or a
        // wrong stride shows up as a mismatch, not a coincidental match.
        let x_offset = 8;
        let y0 = 1;
        let row_len = 8; // two BGRA pixels
        #[rustfmt::skip]
        let pixels: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
        ];

        copy_region_rows(&mut dest, PITCH, x_offset, y0, row_len, &pixels);

        // Row 0 of the region lands at byte (y0 + 0) * pitch + x_offset.
        let row0_start = y0 * PITCH + x_offset;
        assert_eq!(&dest[row0_start..row0_start + row_len], &pixels[0..8]);
        // Row 1 of the region lands one full pitch further on, not one
        // `row_len` further on -- the bug this test exists to catch.
        let row1_start = (y0 + 1) * PITCH + x_offset;
        assert_eq!(&dest[row1_start..row1_start + row_len], &pixels[8..16]);

        // Every byte outside the two written spans is still the sentinel:
        // the padding to the left/right of each row, the padding row above
        // and below the region, and the kernel's page-rounding tail past
        // `pitch * height`.
        for (index, &byte) in dest.iter().enumerate() {
            let in_row0 = (row0_start..row0_start + row_len).contains(&index);
            let in_row1 = (row1_start..row1_start + row_len).contains(&index);
            if !in_row0 && !in_row1 {
                assert_eq!(
                    byte, SENTINEL,
                    "byte {index} outside the region was overwritten"
                );
            }
        }
    }

    #[test]
    fn copy_region_rows_at_a_zero_offset_writes_from_the_start_of_each_row() {
        let mut dest = vec![0u8; 16];
        let pixels = [0xFFu8; 8];
        copy_region_rows(&mut dest, 8, 0, 0, 8, &pixels);
        assert_eq!(&dest[0..8], &[0xFF; 8]);
        assert_eq!(&dest[8..16], &[0u8; 8]);
    }
}
