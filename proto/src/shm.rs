//! One wl_shm buffer over a sealed memfd (Linux-only, like everything
//! Wayland here). The only `unsafe` in this binary is the `mmap` below and
//! the slice built on it, both confined here.

use std::ffi::CStr;
use std::os::fd::OwnedFd;

#[cfg(target_os = "linux")]
use rustix::fs::{MemfdFlags, ftruncate, memfd_create};
#[cfg(target_os = "linux")]
use rustix::mm::{MapFlags, ProtFlags, mmap};

pub struct Shm {
    pub memfd: OwnedFd,
    ptr: *mut u8,
    len: usize,
}

impl Shm {
    #[cfg(target_os = "linux")]
    pub fn new(w: u32, h: u32) -> Option<Self> {
        let len = w as usize * h as usize * 4;
        let name = CStr::from_bytes_with_nul(b"w9-launch\0").ok()?;
        let memfd = memfd_create(name, MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING).ok()?;
        ftruncate(&memfd, len as u64).ok()?;
        // SAFETY: length-checked above; SHARED mapping of our own memfd.
        let map = unsafe {
            mmap(
                std::ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED,
                &memfd,
                0,
            )
        }
        .ok()?;
        Some(Self { memfd, ptr: map as *mut u8, len })
    }

    /// Non-Linux stub so `cargo check` passes on the Mac; never run (the
    /// prototype only runs on the dev VM).
    #[cfg(not(target_os = "linux"))]
    pub fn new(_w: u32, _h: u32) -> Option<Self> {
        None
    }

    pub fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: the mapping outlives `self` by construction.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}
