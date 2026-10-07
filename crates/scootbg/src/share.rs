//! One image's pixels on several outputs: the pages are shared, the
//! `wl_buffer`s are not. Pure (generic over the buffer handle and what the
//! pages hold), so every ordering of attaches, releases and drops is a unit
//! test; `daemon::canvas` uses it with a `WlBuffer` and the shm memory.
//!
//! **Why not one `wl_buffer` on every surface.** `wl_surface.attach` says
//! that once a buffer has been committed to more than one surface, "the
//! delivery of wl_buffer.release events becomes undefined", and suggests
//! "creating multiple wl_buffer objects from the same backing storage"
//! instead. Compositors do differ: Smithay keeps the one buffer wrapper
//! when the same buffer is attached again, wlroots counts locks per
//! buffer. So each output gets its own `wl_buffer`, all made from one
//! `wl_shm_pool` over one memfd: the memory (tens of MB at 4K, on both
//! sides) exists once, and each buffer's releases are exactly as defined
//! as they were with a buffer per output.
//!
//! **When the pixels may be written** ([`Slot::memory_mut`], the one way
//! to them):
//!
//! - this slot's own buffer is released (or was never attached), as for
//!   any shm buffer;
//! - no other slot, on any output, shares the pages: each is either still
//!   held, or free and showing them; either way a write would change what
//!   it shows. A slot another output drops only after its own release (a
//!   free slot, or one released and no longer wanted) leaves nothing
//!   behind;
//! - and no slot was ever dropped while its buffer was still held: that
//!   happens only when its surface is destroyed (`clear`, an output
//!   unplugged, a surface given up on), and the compositor may still be
//!   reading the pages for a frame it had started. Such pages are
//!   [frozen](Shared::is_frozen) for good: shown again as they are, never
//!   written.
//!
//! Anything that shares the pages without a buffer (an image rendered for
//! an output, waiting to be shown there) holds an [`Rc`] too, and so blocks
//! writes the same way.
//!
//! Single-threaded, like the loop that owns it: `Rc` and `Cell`, no locks.

use std::cell::Cell;
use std::rc::Rc;

#[cfg(test)]
mod tests;

/// Pages that one or more slots show.
#[derive(Debug)]
pub struct Shared<M> {
    memory: M,
    /// A slot was dropped while the compositor held its buffer: never
    /// written again (see the module docs).
    frozen: Cell<bool>,
}

impl<M> Shared<M> {
    pub fn new(memory: M) -> Rc<Self> {
        Rc::new(Self {
            memory,
            frozen: Cell::new(false),
        })
    }

    pub fn memory(&self) -> &M {
        &self.memory
    }

    /// The memory to write: `None` while frozen. (Sole ownership is the
    /// caller's: through `Rc::get_mut`, which a share held anywhere else
    /// fails. Transition snapshots use this: never in a slot, so never
    /// frozen, but the check stays.)
    pub fn memory_mut(&mut self) -> Option<&mut M> {
        if self.frozen.get() {
            return None;
        }
        Some(&mut self.memory)
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen.get()
    }
}

/// One output's buffer over [`Shared`] pages.
#[derive(Debug)]
pub struct Slot<B, M> {
    buffer: B,
    shared: Rc<Shared<M>>,
    /// Attached and not released since: the compositor may be reading.
    held: bool,
}

impl<B, M> Slot<B, M> {
    /// A buffer over `shared`, not attached yet.
    pub fn new(buffer: B, shared: Rc<Shared<M>>) -> Self {
        Self {
            buffer,
            shared,
            held: false,
        }
    }

    pub fn buffer(&self) -> &B {
        &self.buffer
    }

    pub fn shared(&self) -> &Rc<Shared<M>> {
        &self.shared
    }

    pub fn memory(&self) -> &M {
        &self.shared.memory
    }

    /// `wl_surface.attach` with this buffer (a commit follows): held until
    /// its release. Attaching a held buffer again keeps it held; no count,
    /// because a compositor may send one release for both (Smithay keeps
    /// the one wrapper for the same buffer attached twice, and a count
    /// would then never come back to zero).
    pub fn attached(&mut self) {
        self.held = true;
    }

    /// `wl_buffer.release` for this buffer.
    pub fn released(&mut self) {
        self.held = false;
    }

    /// Released (or never attached): the compositor is done with this
    /// buffer, so it may be dropped.
    pub fn is_free(&self) -> bool {
        !self.held
    }

    /// The pixels may be written: see the module docs.
    pub fn is_writable(&self) -> bool {
        !self.held && !self.shared.is_frozen() && Rc::strong_count(&self.shared) == 1
    }

    /// The pixels, to write, when [`Slot::is_writable`].
    pub fn memory_mut(&mut self) -> Option<&mut M> {
        if self.held || self.shared.is_frozen() {
            return None;
        }
        // `None` while any other `Rc` (another slot, a rendered image
        // waiting) shares them. No `Weak` is ever made.
        Rc::get_mut(&mut self.shared).map(|shared| &mut shared.memory)
    }

    /// Takes the slot apart, handing back its buffer for the caller to
    /// destroy. Its share of the pages goes: they are freed with the last
    /// one. Dropped while held (its surface destroyed under it), the pages
    /// are frozen for whoever still shows them.
    pub fn retire(self) -> B {
        if self.held {
            self.shared.frozen.set(true);
        }
        self.buffer
    }
}
