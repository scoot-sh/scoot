use std::os::fd::AsFd;

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
    assert_eq!(pread(buffer.fd(), &mut read_back, 0).unwrap(), 24);
    assert_eq!(read_back[0], 0xaa);
    assert_eq!(read_back[23], 0x55);
    assert_eq!(fstat(buffer.fd()).unwrap().st_size, 24);
}

#[test]
fn the_memfd_is_sealed_against_resizing() {
    let buffer = ShmBuffer::new(16, 16).unwrap();
    let seals = fcntl_get_seals(buffer.fd()).unwrap();
    assert!(seals.contains(SealFlags::SHRINK | SealFlags::GROW | SealFlags::SEAL));
    // The kernel refuses both directions, so no one holding the fd can
    // make our next write fault.
    assert_eq!(ftruncate(buffer.fd(), 0), Err(Errno::PERM));
    assert_eq!(ftruncate(buffer.fd(), 1 << 20), Err(Errno::PERM));
    // And the seal set itself is final.
    assert_eq!(
        rustix::fs::fcntl_add_seals(buffer.fd(), SealFlags::WRITE),
        Err(Errno::PERM)
    );
    assert_eq!(fstat(buffer.fd()).unwrap().st_size, 16 * 16 * 4);
}

#[test]
fn attach_then_release_keeps_the_pixels() {
    let mut buffer = ShmBuffer::new(2, 2).unwrap();
    buffer.pixels_mut().fill(7);
    let attached = buffer.attach();
    // While attached only the fd and geometry are reachable.
    assert_eq!(attached.geometry().len, 16);
    let _fd = attached.fd().as_fd();
    let mut buffer = attached.released();
    assert!(buffer.pixels_mut().iter().all(|&b| b == 7));
}

#[test]
fn buffers_can_move_between_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<ShmBuffer>();
    assert_send::<super::Attached>();

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
