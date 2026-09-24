//! What a `--nested` frame goes out to the host in: `wl_shm` buffers the
//! read-back is copied into (`buffers.rs`), or -- in a `gpu-scanout` build,
//! with a GLES renderer on the host's own device -- dma-bufs the frame is
//! blitted into on the GPU (`gpu.rs`).
//!
//! One enum, owned by `Host`, so "which buffers are current" is one field
//! that a resize replaces as a whole, and so every host event about a buffer
//! (`release`, `created`, `failed`) reaches the one set it can concern.

use std::error::Error;

use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer as HostBuffer;
use wayland_client::protocol::wl_shm::WlShm as HostShm;

use super::super::State;
use super::buffers::BufferPool;
#[cfg(feature = "gpu-scanout")]
use super::gpu::Swapchain;

pub(super) enum Presenter {
    /// Nothing built: before the first configure (which builds one; a
    /// frame before it is refused anyway, xdg-shell forbidding an attach),
    /// or after the dma-buf path was abandoned and a `wl_shm` pool could not
    /// be built in its place -- the next read-back frame builds one then
    /// (see [`Presenter::shm_pool`]).
    Unbuilt,
    /// Read-back into host `wl_shm` buffers.
    Shm(BufferPool),
    /// GPU copies into host dma-bufs.
    #[cfg(feature = "gpu-scanout")]
    Dmabuf(Swapchain),
}

impl Presenter {
    /// A `wl_shm` presenter at `width` x `height`.
    pub(super) fn shm(
        shm: &HostShm,
        qh: &QueueHandle<State>,
        width: i32,
        height: i32,
    ) -> Result<Self, Box<dyn Error>> {
        Ok(Self::Shm(BufferPool::new(shm, qh, width, height)?))
    }

    /// The `wl_shm` pool a read-back frame is written into, building one at
    /// `size` if this presenter has none yet ([`Presenter::Unbuilt`]).
    /// `None` when it cannot be built -- the frame is then dropped and the
    /// next frame that draws tries again, so a pool that failed once (memory
    /// pressure at the moment the dma-buf path was abandoned) is not given
    /// up on. Nothing re-arms a render for it on its own: a static screen
    /// keeps its last dma-buf frame until something changes. A dma-buf
    /// presenter has no pool: `None`, and never reached, since a frame for
    /// one is never read back (`render::draw_frame_with`).
    pub(super) fn shm_pool(
        &mut self,
        shm: &HostShm,
        qh: &QueueHandle<State>,
        size: (i32, i32),
    ) -> Option<&mut BufferPool> {
        if matches!(self, Self::Unbuilt) {
            match BufferPool::new(shm, qh, size.0, size.1) {
                Ok(pool) => *self = Self::Shm(pool),
                Err(error) => {
                    // debug!: this can repeat per frame until it succeeds,
                    // and the WARN that says why the path changed has
                    // already been logged once (`Host::fall_back`).
                    tracing::debug!(%error, "could not build the host wl_shm pool yet");
                    return None;
                }
            }
        }
        match self {
            Self::Shm(pool) => Some(pool),
            Self::Unbuilt => None,
            #[cfg(feature = "gpu-scanout")]
            Self::Dmabuf(_) => None,
        }
    }

    /// Whether frames go out as dma-bufs.
    pub(super) fn is_dmabuf(&self) -> bool {
        #[cfg(feature = "gpu-scanout")]
        {
            matches!(self, Self::Dmabuf(_))
        }
        #[cfg(not(feature = "gpu-scanout"))]
        {
            false
        }
    }

    /// Marks the buffer backing `buffer` free again after the host's
    /// `release`. A no-op for a buffer this presenter does not own (one from
    /// a set a resize or a fallback has since replaced).
    pub(super) fn mark_released(&mut self, buffer: &HostBuffer) {
        match self {
            Self::Unbuilt => {}
            Self::Shm(pool) => pool.mark_released(buffer),
            #[cfg(feature = "gpu-scanout")]
            Self::Dmabuf(chain) => chain.mark_released(buffer),
        }
    }

    /// Releases every host object this presenter holds. Call instead of
    /// dropping one that is being replaced: generated proxies have no
    /// destructor of their own, so a dropped one leaks on the host.
    pub(super) fn destroy(self) {
        match self {
            Self::Unbuilt => {}
            Self::Shm(pool) => pool.destroy(),
            #[cfg(feature = "gpu-scanout")]
            Self::Dmabuf(chain) => chain.destroy(),
        }
    }
}
