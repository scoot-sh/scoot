//! A file mapped read-only for the rest of the process, only where nothing
//! can change it: scootbar's font, when it lies on a read-only mount.
//!
//! **Why only there.** A mapped file that is truncated in place makes the
//! next touch of a page past the new end a `SIGBUS`, which kills the
//! process; one rewritten in place changes bytes under a `&[u8]` that
//! promised they would not change. A private mapping prevents neither
//! (pages not yet copied read through to the file). Copying a new font over
//! the old one with `cp` does exactly that (it opens the existing file with
//! `O_TRUNC`), so a font in the user's own directories is not mapped: the
//! caller reads it into the heap instead. A file on a read-only mount
//! (NixOS's `/nix/store`, an image-based system's `/usr`) cannot be
//! truncated or written through that mount, by anyone, root included
//! (`EROFS`), so there the mapping costs only the pages actually read, in
//! the page cache, shared with every other process mapping the font.
//!
//! **What read-only does not cover**, stated rather than hidden: the same
//! filesystem may be mounted read-write elsewhere. On NixOS that is the
//! store's own read-write mount in `nix-daemon`'s namespace, and the daemon
//! never writes a file in place once it is in the store: it adds new paths,
//! and deletes (unlinks) or replaces (renames over) whole files, and the
//! mapping keeps the old inode through both. Beyond that it takes root
//! writing to the block device, which no mapping or read survives either.
//! A disk error reading a mapped page is a `SIGBUS` too, as it is for the
//! executable's own pages.
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

/// Maps `file` read-only for the rest of the process if it is a non-empty
/// regular file on a read-only mount and at most `max_len` bytes long.
/// `Ok(None)` when it is none of those (the caller reads it instead), and
/// an error only when asking the kernel fails.
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
    if !is_read_only_mount(&fstatvfs(file)?.f_flag) {
        return Ok(None);
    }
    // SAFETY: a fresh mapping at an address of the kernel's choosing (null
    // hint, no `MAP_FIXED`), so nothing existing is replaced. `len` is the
    // file's size, non-zero, and the file is on a read-only mount, so
    // neither its size nor its bytes can change under the mapping through
    // any path this process or its user can take (see the module docs for
    // what "read-only" leaves out).
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
