//! The memory behind a `wl_shm` buffer: a sealed memfd, mapped shared.
//!
//! The buffer is `XRGB8888`: 4 bytes per pixel, `stride = width * 4`, and
//! `len = stride * height`, each from a checked multiplication and bounded
//! to `i32::MAX`, because `wl_shm.create_pool` and `create_buffer` take
//! `int32` and the size comes from the compositor.
//!
//! **Never write after attach.** A compositor may read the pages at any
//! time between `wl_surface.attach` + `commit` and `wl_buffer.release`, so
//! writing then tears what it shows. The type state keeps scootbg's own
//! code paths to that rule: [`ShmBuffer::attach`] consumes the writable
//! handle, and the [`Attached`] it returns has no way to the pixels until
//! [`Attached::released`] gives a [`ShmBuffer`] back. That is a discipline
//! for callers, not a guarantee: safe code could still clone the fd and
//! write through it, and nothing checks that `released` is only called on
//! a real `wl_buffer.release`. Tearing is all a violation costs; it is not
//! what memory safety rests on (see `pixels_mut`).
//!
//! **Sealed.** The memfd is created with `MFD_ALLOW_SEALING` and sealed
//! `F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_SEAL` once sized. The compositor
//! holds this fd; if anything could shrink the file, our next write would
//! fault with SIGBUS and kill the daemon. With the seals the kernel refuses
//! the truncation instead.
//!
//! scootbg only writes these pages and never reads them back, the same
//! practice as SCTK and Smithay's clients: another process maps them too.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::ptr::{self, NonNull};

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, ftruncate, memfd_create};
use rustix::mm::{MapFlags, ProtFlags, mmap, munmap};

#[cfg(test)]
mod tests;

/// Bytes per `XRGB8888` pixel.
pub const BYTES_PER_PIXEL: u32 = 4;

/// Why a buffer could not be made.
#[derive(Debug)]
pub enum ShmError {
    /// A zero width or height: there is nothing to map, and `wl_shm`
    /// rejects an empty buffer.
    Empty { width: u32, height: u32 },
    /// The stride or the total size does not fit `wl_shm`'s `int32`.
    TooLarge { width: u32, height: u32 },
    /// The memfd, its seals or the mapping failed.
    Io(io::Error),
}

impl fmt::Display for ShmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { width, height } => {
                write!(f, "a {width}x{height} buffer is empty")
            }
            Self::TooLarge { width, height } => {
                write!(f, "a {width}x{height} buffer is larger than wl_shm allows")
            }
            Self::Io(error) => write!(f, "shared memory: {error}"),
        }
    }
}

impl std::error::Error for ShmError {}

impl From<rustix::io::Errno> for ShmError {
    fn from(errno: rustix::io::Errno) -> Self {
        Self::Io(errno.into())
    }
}

/// A buffer's dimensions, validated for `wl_shm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: i32,
    pub height: i32,
    /// Bytes per row: `width * 4`.
    pub stride: i32,
    /// Bytes in the whole buffer: `stride * height`, never zero.
    pub len: i32,
}

impl Geometry {
    /// Validates an `XRGB8888` buffer of `width` x `height` pixels: both
    /// non-zero, and the stride and total size within `i32::MAX`.
    pub fn xrgb8888(width: u32, height: u32) -> Result<Self, ShmError> {
        if width == 0 || height == 0 {
            return Err(ShmError::Empty { width, height });
        }
        let too_large = || ShmError::TooLarge { width, height };
        let stride = width.checked_mul(BYTES_PER_PIXEL).ok_or_else(too_large)?;
        let len = stride.checked_mul(height).ok_or_else(too_large)?;
        // `len >= stride >= width` and `len >= height` (both factors are
        // non-zero), so one bound on `len` bounds all four.
        let len = i32::try_from(len).map_err(|_| too_large())?;
        Ok(Self {
            // Each is at most `len`, which fits.
            width: width as i32,
            height: height as i32,
            stride: stride as i32,
            len,
        })
    }

    fn len_usize(self) -> usize {
        // Positive `i32` into `usize`: lossless on every target Linux runs.
        self.len as usize
    }
}

/// One shared mapping, unmapped on drop. Private: it is reached only
/// through the two type states below.
struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: `Mapping` owns its pages exclusively within this process (no
// other `Mapping` or reference points at them), so moving it to another
// thread moves the only handle. A decode thread fills a buffer and hands it
// over. It is deliberately not `Sync`: shared `&Mapping`s on two threads
// would let two writers race through `ShmBuffer::pixels_mut`.
unsafe impl Send for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `(ptr, len)` is exactly what `mmap` returned in
        // `ShmBuffer::new`, and dropping the only handle means nothing
        // borrows the pages any more. A failure leaves the mapping in place
        // (a leak, not unsoundness) and cannot be reported from `drop`.
        let _ = unsafe { munmap(self.ptr.as_ptr().cast::<c_void>(), self.len) };
    }
}

/// A buffer scootbg may write: not attached, or released by the
/// compositor.
pub struct ShmBuffer {
    fd: OwnedFd,
    map: Mapping,
    geometry: Geometry,
}

impl fmt::Debug for ShmBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShmBuffer")
            .field("fd", &self.fd)
            .field("geometry", &self.geometry)
            .finish_non_exhaustive()
    }
}

impl ShmBuffer {
    /// A zero-filled `XRGB8888` buffer of `width` x `height` pixels, in a
    /// sealed memfd.
    pub fn new(width: u32, height: u32) -> Result<Self, ShmError> {
        let geometry = Geometry::xrgb8888(width, height)?;
        let len = geometry.len_usize();

        let fd = memfd_create(
            c"scootbg-wallpaper",
            MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
        )?;
        ftruncate(&fd, len as u64)?;
        fcntl_add_seals(&fd, SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL)?;

        // SAFETY: a null hint without `MAP_FIXED` cannot replace any
        // existing mapping, and `len` is non-zero. It is exactly the
        // file's size, which the seals keep from shrinking, so no access
        // through the mapping can fault for lack of a backing page because
        // someone truncated the file. (A first touch can still fail for
        // lack of memory, which the kernel handles like any allocation
        // failure: that is not a memory-safety question.)
        let mapped = unsafe {
            mmap(
                ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                &fd,
                0,
            )
        }?;
        // A successful `mmap` never returns null (the kernel does not map
        // page zero for a non-fixed request); treat it as a failure anyway
        // rather than assume.
        let Some(ptr) = NonNull::new(mapped.cast::<u8>()) else {
            return Err(ShmError::Io(io::Error::other("mmap returned null")));
        };
        Ok(Self {
            fd,
            map: Mapping { ptr, len },
            geometry,
        })
    }

    /// The pixels, `stride * height` bytes, row by row.
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        // SAFETY: `ptr` is valid for reads and writes of `len` bytes for as
        // long as `self.map` lives (it is unmapped only in its `Drop`); the
        // pages are initialised (a memfd reads as zeros until written); and
        // the slice borrows `self` mutably, so it is the only reference
        // into the mapping from this process.
        //
        // Another process maps the same pages: the compositor, from
        // `wl_shm.create_pool` (which takes `fd()` before any attach) until
        // it drops the pool, whatever the buffer's state. So `&mut [u8]`'s
        // "nothing else changes this memory" holds only because the
        // compositor does not write a client's shm buffers. That is
        // `wl_shm`'s contract and every compositor's behaviour, not
        // something the kernel enforces (Smithay maps pools read-write),
        // and it is the same trust every shm client places in its
        // compositor, SCTK's and Smithay's included. scootbg also never
        // reads these pages back, so nothing it decides depends on their
        // contents even if a compositor broke that contract. Its reads
        // while scootbg writes only affect what it displays.
        unsafe { std::slice::from_raw_parts_mut(self.map.ptr.as_ptr(), self.map.len) }
    }

    /// The validated size, stride and length, for `create_pool` and
    /// `create_buffer`.
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// The memfd, to pass to `wl_shm.create_pool`.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// Hands the buffer to the compositor: call this where the buffer is
    /// attached and committed. The pixels are unreachable until
    /// [`Attached::released`].
    pub fn attach(self) -> Attached {
        Attached { inner: self }
    }
}

/// A buffer the compositor may be reading. It has no way to reach the
/// pixels; `wl_buffer.release` turns it back into a [`ShmBuffer`].
#[derive(Debug)]
pub struct Attached {
    inner: ShmBuffer,
}

impl Attached {
    /// The compositor sent `wl_buffer.release`: the buffer is ours to write
    /// again.
    pub fn released(self) -> ShmBuffer {
        self.inner
    }

    pub fn geometry(&self) -> Geometry {
        self.inner.geometry
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inner.fd()
    }
}
