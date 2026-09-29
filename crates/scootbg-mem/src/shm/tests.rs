use rustix::fs::{SealFlags, fcntl_get_seals, fstat, ftruncate};
use rustix::io::{Errno, pread};

use super::{Geometry, ShmBuffer, ShmError};

#[test]
fn geometry_is_width_times_four_by_height() {
    let g = Geometry::xrgb8888(1920, 1080).unwrap();
    assert_eq!(
        g,
        Geometry {
            width: 1920,
            height: 1080,
            stride: 7680,
            len: 7680 * 1080,
        }
    );
    let one = Geometry::xrgb8888(1, 1).unwrap();
    assert_eq!((one.stride, one.len), (4, 4));
}

#[test]
fn zero_sizes_are_refused() {
    for (w, h) in [(0, 1), (1, 0), (0, 0)] {
        assert!(
            matches!(Geometry::xrgb8888(w, h), Err(ShmError::Empty { .. })),
            "{w}x{h}"
        );
        assert!(matches!(ShmBuffer::new(w, h), Err(ShmError::Empty { .. })));
    }
}

#[test]
fn sizes_past_int32_are_refused() {
    let cases = [
        // The stride alone overflows u32.
        (u32::MAX, 1),
        // The stride overflows i32 but not u32.
        (i32::MAX as u32 / 4 + 1, 1),
        // The total overflows u32.
        (65536, 65536),
        // The total overflows i32 but not u32: 4 * 32768 * 16385.
        (32768, 16385),
        (1, u32::MAX),
    ];
    for (w, h) in cases {
        assert!(
            matches!(Geometry::xrgb8888(w, h), Err(ShmError::TooLarge { .. })),
            "{w}x{h}"
        );
    }
    // The largest that fits: exactly i32::MAX rounded down to a whole row.
    let g = Geometry::xrgb8888(i32::MAX as u32 / 4, 1).unwrap();
    assert_eq!(g.len, g.stride);
    assert!(i32::MAX - g.len < 4);
}

#[test]
fn the_buffer_is_zeroed_writable_and_shared_through_the_fd() {
    let mut buffer = ShmBuffer::new(3, 2).unwrap();
    let g = buffer.geometry();
    assert_eq!((g.stride, g.len), (12, 24));
    let pixels = buffer.pixels_mut();
    assert_eq!(pixels.len(), 24);
    assert!(pixels.iter().all(|&b| b == 0));
    pixels[0] = 0xaa;
    pixels[23] = 0x55;

    // What the compositor sees through the fd is what was written.
    let mut read_back = [0u8; 24];
    assert_eq!(pread(buffer.fd().unwrap(), &mut read_back, 0).unwrap(), 24);
    assert_eq!(read_back[0], 0xaa);
    assert_eq!(read_back[23], 0x55);
    assert_eq!(fstat(buffer.fd().unwrap()).unwrap().st_size, 24);
}

#[test]
fn the_memfd_is_sealed_against_resizing() {
    let buffer = ShmBuffer::new(16, 16).unwrap();
    let seals = fcntl_get_seals(buffer.fd().unwrap()).unwrap();
    assert!(seals.contains(SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL));
    // The kernel refuses both directions, so no one holding the fd can
    // make our next write fault.
    assert_eq!(ftruncate(buffer.fd().unwrap(), 0), Err(Errno::PERM));
    assert_eq!(ftruncate(buffer.fd().unwrap(), 1 << 20), Err(Errno::PERM));
    // And the seal set itself is final.
    assert_eq!(
        rustix::fs::fcntl_add_seals(buffer.fd().unwrap(), SealFlags::WRITE),
        Err(Errno::PERM)
    );
    assert_eq!(fstat(buffer.fd().unwrap()).unwrap().st_size, 16 * 16 * 4);
}

#[test]
fn closing_the_fd_keeps_the_pages_and_frees_the_descriptor() {
    let mut buffer = ShmBuffer::new(2, 2).unwrap();
    buffer.pixels_mut().fill(7);
    // What `wl_shm.create_pool` leaves the compositor: its own copy.
    let compositor = buffer.fd().unwrap().try_clone_to_owned().unwrap();
    let ours = format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&buffer.fd().unwrap())
    );
    assert!(std::fs::symlink_metadata(&ours).is_ok());
    let memfd = fstat(buffer.fd().unwrap()).unwrap();
    buffer.close_fd();
    assert!(buffer.fd().is_none());
    // Compare the file, not the number: tests share this process's fd
    // table, so a neighbour can open something that reuses the number
    // we just freed. That is a different file; only the memfd itself
    // still being open at that path is a failure.
    match std::fs::metadata(&ours) {
        Err(_) => {}
        Ok(now) => {
            use std::os::unix::fs::MetadataExt;
            assert!(
                now.dev() != memfd.st_dev as u64 || now.ino() != memfd.st_ino as u64,
                "the memfd is still open"
            );
        }
    }
    // Our mapping still holds the pages, and writes through it reach
    // the compositor's copy of the file.
    assert!(buffer.pixels_mut().iter().all(|&b| b == 7));
    buffer.pixels_mut()[0] = 9;
    let mut read_back = [0u8; 16];
    assert_eq!(pread(&compositor, &mut read_back, 0).unwrap(), 16);
    assert_eq!(read_back[..2], [9, 7]);
    // Still sealed: the other holder cannot shrink it under the mapping.
    assert_eq!(ftruncate(&compositor, 0), Err(Errno::PERM));
    // Closing twice is harmless.
    buffer.close_fd();
    assert!(buffer.fd().is_none());
}

#[test]
fn buffers_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<ShmBuffer>();

    let mut buffer = ShmBuffer::new(4, 4).unwrap();
    buffer.pixels_mut().fill(1);
    let mut back = std::thread::spawn(move || {
        buffer.pixels_mut()[0] = 2;
        buffer
    })
    .join()
    .unwrap();
    assert_eq!(back.pixels_mut()[..2], [2, 1]);
}
