//! The probe's arithmetic, and its fd walk against real fds. What it learns
//! from a real renderer is pinned over the wire in `client_fds/tests/dmabuf.rs`.

use std::os::fd::AsFd;

use super::{MAX_COPIES_PER_PLANE, fds_naming, identity, per_plane};

#[test]
fn copies_are_spread_over_planes_and_backends_rounding_up() {
    // llvmpipe, one output: three planes on one file, three copies.
    assert_eq!(per_plane(3, 3, 1), 1);
    // A driver that keeps nothing.
    assert_eq!(per_plane(0, 3, 1), 0);
    // Two outputs, each renderer keeping one copy of each plane.
    assert_eq!(per_plane(6, 3, 2), 1);
    // A partial count rounds up: an over-count costs headroom, an
    // under-count hides fds.
    assert_eq!(per_plane(1, 3, 1), 1);
    // Past the cap is believed to be something else, and capped.
    assert_eq!(per_plane(100, 1, 1), MAX_COPIES_PER_PLANE);
    // Degenerate inputs never divide by zero.
    assert_eq!(per_plane(2, 0, 0), 2);
}

#[test]
fn every_fd_naming_a_file_is_counted_and_no_other() {
    let memfd = rustix::fs::memfd_create("renderer-copies-probe", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let (dev, ino) = identity(memfd.as_fd()).expect("an identity");
    assert_eq!(fds_naming(dev, ino), Some(1));
    let copies: Vec<_> = (0..3).map(|_| memfd.try_clone().expect("a copy")).collect();
    assert_eq!(fds_naming(dev, ino), Some(4));
    drop(copies);
    let other = rustix::fs::memfd_create("renderer-copies-other", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("another memfd");
    assert_eq!(fds_naming(dev, ino), Some(1), "another file is not counted");
    drop(other);
}
