//! The popup's buffers: two sealed-memfd `wl_shm` buffers made on first
//! need and dropped with the popup, so a closed popup holds no memory. The
//! bar's own pooling (`daemon::canvas`) in miniature: a buffer is written
//! only while the compositor does not hold it.

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;

use scootbg_mem::{ShmBuffer, ShmError};

use super::PopupId;
use crate::daemon::wayland::{Globals, State};

/// One buffer: its memory, the pool over it, the `wl_buffer`.
#[derive(Debug)]
pub struct Slot {
    shm: ShmBuffer,
    pool: WlShmPool,
    pub buffer: WlBuffer,
    /// Attached and not released since: the compositor may be reading.
    pub held: bool,
    pub dims: (u32, u32),
}

impl Slot {
    fn new(
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: PopupId,
        dims: (u32, u32),
    ) -> Result<Self, ShmError> {
        let mut shm = ShmBuffer::new(dims.0, dims.1)?;
        let geometry = shm.geometry();
        let Some(fd) = shm.fd() else {
            return Err(ShmError::Io(std::io::Error::other(
                "the buffer's memfd is already closed",
            )));
        };
        let pool = globals.shm.create_pool(fd, geometry.len, qh, ());
        shm.close_fd();
        let buffer = pool.create_buffer(
            0,
            geometry.width,
            geometry.height,
            geometry.stride,
            wl_shm::Format::Xrgb8888,
            qh,
            id,
        );
        Ok(Self {
            shm,
            pool,
            buffer,
            held: false,
            dims,
        })
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.shm.pixels_mut()
    }

    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// The two slots.
#[derive(Debug, Default)]
pub struct Pool {
    slots: [Option<Slot>; 2],
}

impl Pool {
    /// A slot to draw `dims` in: a free one of that size, else a new one in
    /// an empty place, else a free one of another size replaced; `None`
    /// while the compositor holds every one (the release wakes the loop,
    /// which draws then).
    pub fn take(
        &mut self,
        globals: &Globals,
        qh: &QueueHandle<State>,
        id: PopupId,
        dims: (u32, u32),
    ) -> Result<Option<&mut Slot>, ShmError> {
        let index = self
            .slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| !s.held && s.dims == dims))
            .or_else(|| self.slots.iter().position(Option::is_none))
            .or_else(|| {
                self.slots
                    .iter()
                    .position(|s| s.as_ref().is_some_and(|s| !s.held))
            });
        let Some(index) = index else {
            return Ok(None);
        };
        let Some(entry) = self.slots.get_mut(index) else {
            return Ok(None);
        };
        if entry.as_ref().is_some_and(|s| s.dims != dims) {
            if let Some(stale) = entry.take() {
                stale.destroy();
            }
        }
        if entry.is_none() {
            *entry = Some(Slot::new(globals, qh, id, dims)?);
        }
        Ok(entry.as_mut())
    }

    /// `wl_buffer.release`: the slot may be written again.
    pub fn released(&mut self, buffer: &WlBuffer) {
        for slot in self.slots.iter_mut().flatten() {
            if &slot.buffer == buffer {
                slot.held = false;
            }
        }
    }

    /// Destroys every buffer. Safe while the compositor holds one: it has
    /// its own mapping of the memfd.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            if let Some(slot) = slot.take() {
                slot.destroy();
            }
        }
    }
}
