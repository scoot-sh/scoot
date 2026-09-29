//! A file mapped read-only for the rest of the process, only where nothing
//! but a root process rewriting the file in place can change it:
//! scootbar's font, when it is a root-owned, unwritable file on a read-only
//! mount (NixOS's `/nix/store`).
//!
//! **Why so narrow.** A mapped file that is truncated in place makes the
//! next touch of a page past the new end a `SIGBUS`, which kills the
//! process; one rewritten in place changes bytes under a `&[u8]` that
//! promised they would not change. A private mapping prevents neither
//! (pages not yet copied read through to the file). Copying a new font over
//! the old one with `cp` does exactly that (it opens the existing file with
//! `O_TRUNC`). Everything that fails [`may_map`] is read into the heap by
//! the caller instead, which no later write can reach.
//!
//! **A read-only mount alone is not enough**: `ST_RDONLY` is a property of
//! the *mount*, not of the file, and the same file is often writable
//! through another path. A review of the first version reproduced it: a
//! font bind-mounted read-only and truncated through its read-write path
//! killed the bar with `SIGBUS`. That is the everyday shape of systemd's
//! `ProtectHome=read-only`, `ProtectSystem=strict` and `ReadOnlyPaths=`,
//! flatpak's `/run/host/fonts`, read-only container roots, and NFS or FUSE
//! mounts. So [`may_map`] also asks the file itself: **owned by root, and
//! with no write bit set for anyone** (`st_mode & 0o222 == 0`). Store files
//! are exactly that (root, `0444`), so NixOS keeps the mapping; a user's
//! file behind a read-only view of their home is not, and is read.
//!
//! **What is left**, stated rather than hidden: any *root* process that opens
//! such a file through a read-write view of the same filesystem and truncates
//! or rewrites it in place. The write bit does not stop root (it holds
//! `CAP_DAC_OVERRIDE`); a review reproduced the `SIGBUS` at the head with a
//! plain `: > file` as root, on a `0444` file, through the read-write side of a
//! bind mount. On NixOS the only root writer is `nix-daemon`,
//! which never rewrites a store file in place: it adds new paths, and
//! deletes (unlinks) or replaces (renames over) whole files, and the
//! mapping keeps the old inode through both. A disk error reading a mapped
//! page is a `SIGBUS` too, as it is for the executable's own pages.
//!
//! The mapping is never unmapped (the font is loaded once, for the
//! process's life), which is what makes the `'static` slice sound.

use std::io;
use std::os::fd::AsFd;
use std::ptr;

use rustix::fs::{FileType, StatVfsMountFlags, fstat, fstatvfs};
use rustix::mm::{MapFlags, ProtFlags, mmap};

#[cfg(test)]
mod tests;

/// What [`may_map`] decides on: the mount's read-only flag, and the file's
/// owner and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Premises {
    /// `statvfs` says `ST_RDONLY` (per mount on Linux).
    pub read_only_mount: bool,
    /// `st_uid`.
    pub owner: u32,
    /// `st_mode`, permission bits included.
    pub mode: u32,
}

/// Whether a file with these premises may be mapped: on a read-only mount,
/// owned by root, and writable by no one (see the module docs for why each
/// is needed).
pub fn may_map(premises: &Premises) -> bool {
    premises.read_only_mount && premises.owner == 0 && premises.mode & 0o222 == 0
}

/// Maps `file` read-only for the rest of the process if it is a non-empty
/// regular file of at most `max_len` bytes that [`may_map`] allows.
/// `Ok(None)` otherwise (the caller reads it instead), and an error only
/// when asking the kernel fails.
pub fn map_if_read_only<F: AsFd>(file: F, max_len: u64) -> io::Result<Option<&'static [u8]>> {
    let file = file.as_fd();
    let stat = fstat(file)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Ok(None);
    }
    let Ok(len) = u64::try_from(stat.st_size) else {
        return Ok(None);
    };
    if len == 0 || len > max_len {
        return Ok(None);
    }
    let Ok(len) = usize::try_from(len) else {
        return Ok(None);
    };
    let premises = Premises {
        read_only_mount: is_read_only_mount(&fstatvfs(file)?.f_flag),
        owner: stat.st_uid,
        mode: stat.st_mode,
    };
    if !may_map(&premises) {
        return Ok(None);
    }
    // SAFETY: a fresh mapping at an address of the kernel's choosing (null
    // hint, no `MAP_FIXED`), so nothing existing is replaced. `len` is the
    // file's size, non-zero. The file is on a read-only mount, owned by
    // root and writable by no one, so neither its size nor its bytes can
    // change under the mapping unless a root process rewrites it in place
    // through a read-write view; root ignores the write bit (see the module
    // docs, which say why that is accepted and where it is not reachable).
    let address = unsafe {
        mmap(
            ptr::null_mut(),
            len,
            ProtFlags::READ,
            MapFlags::PRIVATE,
            file,
            0,
        )
    }?;
    // SAFETY: `address` starts `len` readable bytes (the mapping just
    // made), never written (read-only, private) and never unmapped (this
    // module has no `munmap`: the mapping lives as long as the process),
    // so the slice is valid for `'static`, and nothing else holds a
    // mutable reference to it.
    let bytes = unsafe { std::slice::from_raw_parts(address.cast::<u8>(), len) };
    Ok(Some(bytes))
}

/// Whether `statvfs`'s mount flags say read-only. On Linux they are the
/// mount's own (`ST_RDONLY`), so a read-only bind of a read-write
/// filesystem counts, as NixOS's store is.
pub fn is_read_only_mount(flags: &StatVfsMountFlags) -> bool {
    flags.contains(StatVfsMountFlags::RDONLY)
}
