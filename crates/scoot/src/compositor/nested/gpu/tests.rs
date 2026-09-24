//! Unit tests for the pure halves of `--nested`'s dma-buf presentation:
//! reading the host's feedback as the adversarial input it is, choosing a
//! format from it, and picking a usable host buffer. The GPU half -- a real
//! allocation, copy and import -- is `render/tests/host_copy.rs`, and the
//! protocol half (`create`, `created`, attach) is pinned live on the dev VM
//! (see the PR).

use std::os::fd::OwnedFd;

use smithay::backend::allocator::{Format, Fourcc, Modifier};

use super::feedback::test_access::{dev, index_list, parse, read, resolve_tranches};
use super::feedback::{Choice, HostFeedback, HostTranche, choose};
use super::usable;

const AR24: u32 = Fourcc::Argb8888 as u32;
const XR24: u32 = Fourcc::Xrgb8888 as u32;
const NV12: u32 = Fourcc::Nv12 as u32;
const LINEAR: u64 = 0;
const INVALID: u64 = 0x00ff_ffff_ffff_ffff;
/// Some explicit tiled modifier (the value only has to be neither of the two
/// above).
const TILED: u64 = 0x0100_0000_0000_0001;

const OURS: libc::dev_t = 0xe280;
const THEIRS: libc::dev_t = 0xe281;

fn same(dev: libc::dev_t) -> bool {
    dev == OURS
}

fn tranche(device: libc::dev_t, scanout: bool, formats: &[(u32, u64)]) -> HostTranche {
    HostTranche {
        device,
        scanout,
        formats: formats.to_vec(),
    }
}

fn host(tranches: Vec<HostTranche>) -> HostFeedback {
    HostFeedback {
        main_device: OURS,
        tranches,
    }
}

fn renders_everything(_: Format) -> bool {
    true
}

// --- picking a host buffer -------------------------------------------------

#[test]
fn a_slot_is_usable_once_created_and_while_not_held() {
    assert_eq!(
        usable([(false, false); 3].into_iter()),
        None,
        "none created yet"
    );
    assert_eq!(usable([(true, true); 3].into_iter()), None, "all held");
    assert_eq!(
        usable([(false, false), (true, true), (true, false)].into_iter()),
        Some(2)
    );
    assert_eq!(usable([(true, false), (true, false)].into_iter()), Some(0));
    assert_eq!(usable(std::iter::empty()), None);
}

// --- reading the feedback --------------------------------------------------

fn entry(fourcc: u32, modifier: u64) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&fourcc.to_ne_bytes());
    bytes[4..8].copy_from_slice(&0xdead_beef_u32.to_ne_bytes()); // padding is ignored
    bytes[8..].copy_from_slice(&modifier.to_ne_bytes());
    bytes
}

fn table_fd(bytes: &[u8]) -> OwnedFd {
    let fd = rustix::fs::memfd_create("scoot-test-table", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    let mut written = 0;
    while written < bytes.len() {
        written += rustix::io::write(&fd, &bytes[written..]).expect("a write");
    }
    fd
}

#[test]
fn a_table_parses_entry_by_entry_ignoring_the_padding() {
    let bytes: Vec<u8> = [entry(AR24, LINEAR), entry(XR24, TILED)].concat();
    assert_eq!(parse(&bytes), vec![(AR24, LINEAR), (XR24, TILED)]);
}

#[test]
fn a_table_is_read_at_offset_zero_whatever_the_shared_offset() {
    let bytes: Vec<u8> = [entry(AR24, LINEAR), entry(XR24, INVALID)].concat();
    let fd = table_fd(&bytes);
    // The write left the file offset at the end: a `read` would see
    // nothing. So would a second client sharing the description after a
    // first one `read` it -- which is why the table is `pread`.
    let len = u32::try_from(bytes.len()).expect("small");
    assert_eq!(read(&fd, len), Some(vec![(AR24, LINEAR), (XR24, INVALID)]));
    assert_eq!(
        read(&fd, len),
        Some(vec![(AR24, LINEAR), (XR24, INVALID)]),
        "and again"
    );
}

#[test]
fn a_malformed_table_size_is_refused_not_guessed_at() {
    let bytes: Vec<u8> = [entry(AR24, LINEAR), entry(XR24, LINEAR)].concat();
    let fd = table_fd(&bytes);
    assert_eq!(read(&fd, 17), None, "not a whole number of entries");
    assert_eq!(read(&fd, 48), None, "longer than the file: a short read");
    assert_eq!(
        read(&fd, u32::MAX - 15),
        None,
        "a size past what u16 indices can name is refused before any allocation"
    );
    assert_eq!(read(&fd, 0), Some(Vec::new()), "an empty table is empty");
}

#[test]
fn a_dev_t_must_be_exactly_a_dev_t() {
    let bytes = OURS.to_ne_bytes();
    assert_eq!(dev(&bytes), Some(OURS));
    assert_eq!(dev(&bytes[..4]), None, "short");
    assert_eq!(dev(&[bytes.as_slice(), &[0]].concat()), None, "long");
    assert_eq!(dev(&[]), None, "empty");
}

#[test]
fn indices_are_u16_pairs_and_an_odd_array_is_refused() {
    let bytes = [1u16.to_ne_bytes(), 300u16.to_ne_bytes()].concat();
    assert_eq!(index_list(&bytes), Some(vec![1, 300]));
    assert_eq!(index_list(&bytes[..3]), None);
    assert_eq!(index_list(&[]), Some(Vec::new()));
}

#[test]
fn an_index_past_the_table_discards_the_whole_feedback() {
    let table = [(AR24, LINEAR), (XR24, LINEAR)];
    assert!(
        resolve_tranches(OURS, &[(Some(OURS), vec![0, 1], false)], &table).is_some(),
        "in range"
    );
    assert_eq!(
        resolve_tranches(
            OURS,
            &[(Some(OURS), vec![0], false), (Some(OURS), vec![2], false)],
            &table
        ),
        None,
        "one bad index in any tranche is not trusted for the rest"
    );
    assert_eq!(
        resolve_tranches(OURS, &[(Some(OURS), vec![u16::MAX], false)], &[]),
        None,
        "nothing indexes an empty table"
    );
}

#[test]
fn a_tranche_without_a_target_device_is_dropped() {
    let table = [(AR24, LINEAR)];
    let feedback = resolve_tranches(
        OURS,
        &[(None, vec![0], true), (Some(OURS), vec![0], false)],
        &table,
    )
    .expect("well formed");
    assert_eq!(feedback.tranches.len(), 1);
    assert!(!feedback.tranches[0].scanout);
}

// --- choosing a format -----------------------------------------------------

#[test]
fn argb_at_the_hosts_explicit_modifiers_is_preferred() {
    let feedback = host(vec![tranche(
        OURS,
        false,
        &[
            (XR24, LINEAR),
            (AR24, TILED),
            (AR24, LINEAR),
            (AR24, INVALID),
        ],
    )]);
    assert_eq!(
        choose(&feedback, same, renders_everything),
        Some(Choice {
            fourcc: Fourcc::Argb8888,
            request: vec![Modifier::from(TILED), Modifier::Linear],
            accept: vec![Modifier::from(TILED), Modifier::Linear, Modifier::Invalid],
        }),
        "explicit modifiers only in the request -- implicit is never mixed in -- \
         but an implicit answer is still acceptable where the host lists it"
    );
}

#[test]
fn xrgb_is_taken_when_argb_is_not_common() {
    let feedback = host(vec![tranche(
        OURS,
        false,
        &[(NV12, LINEAR), (XR24, LINEAR)],
    )]);
    let choice = choose(&feedback, same, renders_everything).expect("xrgb");
    assert_eq!(choice.fourcc, Fourcc::Xrgb8888);
    assert_eq!(choice.request, vec![Modifier::Linear]);
}

#[test]
fn only_modifiers_the_renderer_can_render_into_are_asked_for() {
    let feedback = host(vec![tranche(OURS, false, &[(AR24, TILED), (AR24, LINEAR)])]);
    let linear_only = |format: Format| format.modifier == Modifier::Linear;
    let choice = choose(&feedback, same, linear_only).expect("linear is common");
    assert_eq!(choice.request, vec![Modifier::Linear]);
    // Nothing the renderer cannot draw into is acceptable back either, and
    // no implicit answer where the host listed none.
    assert_eq!(choice.accept, vec![Modifier::Linear]);
}

#[test]
fn implicit_is_asked_for_only_when_nothing_explicit_is_common_and_the_host_lists_it() {
    let nothing_explicit = |format: Format| format.modifier == Modifier::Invalid;
    let with_invalid = host(vec![tranche(
        OURS,
        false,
        &[(AR24, TILED), (AR24, INVALID)],
    )]);
    let choice = choose(&with_invalid, same, nothing_explicit).expect("implicit");
    assert_eq!(choice.request, vec![Modifier::Invalid]);
    assert_eq!(choice.accept, vec![Modifier::Invalid]);

    let without_invalid = host(vec![tranche(OURS, false, &[(AR24, TILED)])]);
    assert_eq!(
        choose(&without_invalid, same, nothing_explicit),
        None,
        "an implicit buffer is never handed to a host that did not list one"
    );
}

#[test]
fn scanout_tranches_come_first_then_the_hosts_order() {
    let feedback = host(vec![
        tranche(OURS, false, &[(AR24, LINEAR)]),
        tranche(OURS, true, &[(AR24, TILED)]),
    ]);
    let choice = choose(&feedback, same, renders_everything).expect("argb");
    assert_eq!(
        choice.request,
        vec![Modifier::from(TILED), Modifier::Linear]
    );
}

#[test]
fn tranches_for_another_device_are_ignored() {
    let feedback = host(vec![
        tranche(THEIRS, true, &[(AR24, TILED)]),
        tranche(OURS, false, &[(XR24, LINEAR)]),
    ]);
    let choice = choose(&feedback, same, renders_everything).expect("xrgb on ours");
    assert_eq!(
        choice.fourcc,
        Fourcc::Xrgb8888,
        "argb was only offered for the other device"
    );
}

#[test]
fn a_host_compositing_on_another_device_gets_read_back() {
    let feedback = HostFeedback {
        main_device: THEIRS,
        tranches: vec![tranche(OURS, false, &[(AR24, LINEAR)])],
    };
    assert_eq!(
        choose(&feedback, same, renders_everything),
        None,
        "even a tranche naming our device does not make a cross-device host safe"
    );
}

#[test]
fn nothing_in_common_is_no_choice() {
    assert_eq!(choose(&host(Vec::new()), same, renders_everything), None);
    let yuv_only = host(vec![tranche(OURS, false, &[(NV12, LINEAR)])]);
    assert_eq!(choose(&yuv_only, same, renders_everything), None);
    let unrenderable = host(vec![tranche(OURS, false, &[(AR24, LINEAR)])]);
    assert_eq!(choose(&unrenderable, same, |_| false), None);
}

#[test]
fn a_dev_t_that_is_no_drm_node_matches_nothing() {
    // `/dev/null` is 1:3, not a DRM device: it canonicalises to nothing and
    // so never matches, not even itself.
    let null = libc::makedev(1, 3);
    assert!(!super::feedback::same_drm_device(null, null));
    assert!(!super::feedback::same_drm_device(0, 0));
}

#[test]
fn a_primary_node_and_its_render_node_are_the_same_device() {
    use smithay::backend::drm::{DrmNode, NodeType};
    use std::os::unix::fs::MetadataExt;
    // The dev VM's cage names its main device as the render node while a
    // host is free to name the primary one; either way it is one device.
    let Ok(render) = std::fs::metadata("/dev/dri/renderD128") else {
        eprintln!(
            "a_primary_node_and_its_render_node_are_the_same_device: skipped -- no render node"
        );
        return;
    };
    let render = render.rdev();
    let Some(Ok(primary)) = DrmNode::from_dev_id(render)
        .ok()
        .and_then(|node| node.node_with_type(NodeType::Primary))
    else {
        eprintln!(
            "a_primary_node_and_its_render_node_are_the_same_device: skipped -- no primary node"
        );
        return;
    };
    assert_ne!(primary.dev_id(), render, "two different dev_t values");
    assert!(super::feedback::same_drm_device(primary.dev_id(), render));
    assert!(super::feedback::same_drm_device(render, primary.dev_id()));
    assert!(super::feedback::same_drm_device(render, render));
}

mod live;
