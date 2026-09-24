//! The retained-timeline ledger's bookkeeping and decisions, without a
//! compositor: fd numbers are plain integers here and liveness is a set the
//! test controls, except in the tests that pin [`timeline_fd_open`] against
//! real fds. What the wire does with it is in `drm_syncobj/tests.rs`.

use std::collections::HashSet;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::Display;

use super::{Refusal, RetainedTimelines, SWEEP_MARGIN, proc_fd_path, timeline_fd_open};

const CAP: u32 = 128;
const GRACE: u32 = 32;

/// Two distinct live `ClientId`s. `ClientId` has no public constructor, so
/// these are real clients on a throwaway display, which is only a map key
/// here.
fn two_clients() -> (Display<()>, ClientId, ClientId) {
    struct Data;
    impl smithay::reexports::wayland_server::backend::ClientData for Data {}
    let display: Display<()> = Display::new().expect("a display");
    let mut handle = display.handle();
    let mut client = || {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        // The client end is dropped at once; the id stays valid as a key.
        handle
            .insert_client(server, std::sync::Arc::new(Data))
            .expect("a client")
            .id()
    };
    let a = client();
    let b = client();
    (display, a, b)
}

/// Imports `count` timelines for `client` on fd numbers starting at `first`,
/// each admitted with nothing live but what `live` names and no pressure.
fn import_all(
    ledger: &mut RetainedTimelines,
    client: &ClientId,
    first: RawFd,
    count: u32,
    live: &HashSet<RawFd>,
) -> Result<(), Refusal> {
    for fd in first..first + count as RawFd {
        ledger.fd_arrived(fd);
        ledger.admit(client, CAP, GRACE, |fd| live.contains(&fd), || false)?;
        ledger.record(client, fd);
    }
    Ok(())
}

#[test]
fn records_count_per_client_and_drain_by_sweep() {
    let (_display, a, b) = two_clients();
    let mut ledger = RetainedTimelines::default();
    let live: HashSet<RawFd> = (100..105).collect();
    import_all(&mut ledger, &a, 100, 10, &live).expect("under the cap");
    import_all(&mut ledger, &b, 200, 3, &live).expect("under the cap");
    assert_eq!(ledger.held_by(&a), 10);
    assert_eq!(ledger.held_by(&b), 3);
    assert_eq!(ledger.records(), 13);
    assert_eq!(ledger.sweep(&a, |fd| live.contains(&fd)), 5);
    assert_eq!(ledger.held_by(&a), 5, "a sweep drops a's dead records");
    assert_eq!(ledger.held_by(&b), 3, "and touches nobody else's");
    assert_eq!(ledger.sweep(&b, |_| false), 0);
    assert_eq!(ledger.held_by(&b), 0);
    assert_eq!(ledger.records(), 5, "an emptied client leaves no records behind");
}

#[test]
fn a_reused_number_forgets_the_old_record_whoever_owned_it() {
    let (_display, a, b) = two_clients();
    let mut ledger = RetainedTimelines::default();
    ledger.record(&a, 7);
    ledger.record(&a, 8);
    // Number 7 reaches the compositor again, from anyone, through anything:
    // the timeline that held it is gone.
    ledger.fd_arrived(7);
    assert_eq!(ledger.held_by(&a), 1);
    // A new import on 8 by another client replaces a's record rather than
    // counting 8 twice.
    ledger.record(&b, 8);
    assert_eq!(ledger.held_by(&a), 0);
    assert_eq!(ledger.held_by(&b), 1);
    assert_eq!(ledger.records(), 1);
    // A sweep of the old owner cannot take the new owner's record.
    assert_eq!(ledger.sweep(&a, |_| false), 0);
    assert_eq!(ledger.held_by(&b), 1);
}

#[test]
fn fd_arrived_on_an_empty_ledger_is_a_no_op() {
    let mut ledger = RetainedTimelines::default();
    ledger.fd_arrived(3);
    assert_eq!(ledger.records(), 0);
}

/// The ticket's shape: every timeline stays held (its points outlive the
/// object), so the cap refuses exactly at the cap, with a fresh count.
#[test]
fn a_client_really_holding_the_cap_is_refused_at_it() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    let live: HashSet<RawFd> = (1000..2000).collect();
    import_all(&mut ledger, &a, 1000, CAP, &live).expect("the cap itself is admitted");
    assert_eq!(ledger.held_by(&a), CAP);
    let refusal = ledger.admit(&a, CAP, GRACE, |fd| live.contains(&fd), || false);
    assert_eq!(refusal, Err(Refusal::Cap { held: CAP }));
}

/// Swapchain churn: imports whose timelines are gone by the time the cap is
/// reached are swept away, not counted, however many there were.
#[test]
fn churned_timelines_never_reach_the_cap() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    // 16 live (one Vulkan window), then 10 x the cap of imports that die.
    let live: HashSet<RawFd> = (0..16).collect();
    import_all(&mut ledger, &a, 0, 16, &live).expect("a window's worth");
    import_all(&mut ledger, &a, 10_000, 10 * CAP, &live).expect("dead records are swept");
    assert!(ledger.held_by(&a) <= CAP, "the records never exceed the cap");
    assert_eq!(ledger.sweep(&a, |fd| live.contains(&fd)), 16);
}

/// The amortization the margin exists for: a client really holding one
/// under the cap is refused, rather than admitted with a full sweep on every
/// import it churns.
#[test]
fn a_client_within_the_margin_of_the_cap_is_refused_not_swept_per_import() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    let live: HashSet<RawFd> = (0..CAP as RawFd - 1).collect();
    import_all(&mut ledger, &a, 0, CAP - 1, &live).expect("under the cap");
    ledger.record(&a, 5000); // one churned import, dead
    let mut checks = 0;
    let refusal = ledger.admit(
        &a,
        CAP,
        GRACE,
        |fd| {
            checks += 1;
            live.contains(&fd)
        },
        || false,
    );
    assert_eq!(refusal, Err(Refusal::Cap { held: CAP - 1 }));
    assert_eq!(checks, CAP, "one sweep, of the client's own records");
}

#[test]
fn a_sweep_that_frees_the_margin_admits_and_buys_that_many_imports() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    let live_count = CAP - SWEEP_MARGIN;
    let live: HashSet<RawFd> = (0..live_count as RawFd).collect();
    import_all(&mut ledger, &a, 0, live_count, &live).expect("under the cap");
    import_all(&mut ledger, &a, 5000, SWEEP_MARGIN, &live).expect("to the cap");
    let mut sweeps = 0;
    for fd in 6000..6000 + SWEEP_MARGIN as RawFd {
        let mut swept = false;
        ledger
            .admit(
                &a,
                CAP,
                GRACE,
                |fd| {
                    swept = true;
                    live.contains(&fd) || fd >= 6000
                },
                || false,
            )
            .expect("the dead margin is reclaimed");
        sweeps += u32::from(swept);
        ledger.record(&a, fd);
    }
    assert_eq!(sweeps, 1, "one sweep for the whole margin's worth of imports");
}

#[test]
fn nothing_is_observed_under_the_grace() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    for fd in 0..GRACE as RawFd {
        ledger
            .admit(
                &a,
                CAP,
                GRACE,
                |_| panic!("no sweep under the grace"),
                || panic!("no table observation under the grace"),
            )
            .expect("under the grace");
        ledger.record(&a, fd);
    }
}

#[test]
fn under_pressure_a_client_past_the_grace_is_refused_on_a_fresh_count() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    let live: HashSet<RawFd> = (0..=GRACE as RawFd).collect();
    import_all(&mut ledger, &a, 0, GRACE + 1, &live).expect("no pressure yet");
    let refusal = ledger.admit(&a, CAP, GRACE, |fd| live.contains(&fd), || true);
    assert_eq!(refusal, Err(Refusal::Pressure { held: GRACE + 1 }));
}

#[test]
fn under_pressure_dead_records_do_not_count_against_the_grace() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    // Ten live, then enough dead to put the raw count far past the grace.
    let live: HashSet<RawFd> = (0..10).collect();
    import_all(&mut ledger, &a, 0, 10, &live).expect("under the grace");
    for fd in 100..100 + 3 * GRACE as RawFd {
        ledger.record(&a, fd);
    }
    assert!(ledger.held_by(&a) > GRACE);
    ledger
        .admit(&a, CAP, GRACE, |fd| live.contains(&fd), || true)
        .expect("the fresh count is 10, under the grace");
    assert_eq!(ledger.held_by(&a), 10);
}

#[test]
fn under_pressure_sweeps_are_amortized_by_the_margin() {
    let (_display, a, _) = two_clients();
    let mut ledger = RetainedTimelines::default();
    // Exactly the grace live, plus one dead record: the first sweep under
    // pressure admits, and buys the margin's worth of unswept imports.
    let live: HashSet<RawFd> = (0..10_000).collect();
    import_all(&mut ledger, &a, 0, GRACE, &live).expect("no pressure yet");
    ledger.record(&a, 20_000);
    let (mut sweeps, mut observations) = (0, 0);
    let mut outcome = Ok(());
    for fd in GRACE as RawFd..GRACE as RawFd + 2 * SWEEP_MARGIN as RawFd {
        let mut swept = false;
        outcome = ledger.admit(
            &a,
            CAP,
            GRACE,
            |fd| {
                swept = true;
                live.contains(&fd)
            },
            || {
                observations += 1;
                true
            },
        );
        sweeps += u32::from(swept);
        if outcome.is_err() {
            break;
        }
        ledger.record(&a, fd);
    }
    assert_eq!(
        outcome,
        Err(Refusal::Pressure {
            held: GRACE + SWEEP_MARGIN
        }),
        "the slack past a sweep that admitted is the margin, and not a record more"
    );
    assert_eq!(sweeps, 2, "the sweep that admitted, and the one that refused");
    assert_eq!(observations, 2, "the table is observed only when a sweep is due");
}

#[test]
fn the_ledger_is_bounded_by_fd_numbers_not_by_clients() {
    let display: Display<()> = Display::new().expect("a display");
    let mut handle = display.handle();
    struct Data;
    impl smithay::reexports::wayland_server::backend::ClientData for Data {}
    let mut ledger = RetainedTimelines::default();
    // A thousand short-lived clients, each importing on the same few
    // numbers the kernel would hand out again, and never sweeping.
    for _ in 0..1000 {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        let id = handle
            .insert_client(server, std::sync::Arc::new(Data))
            .expect("a client")
            .id();
        for fd in 20..24 {
            ledger.fd_arrived(fd);
            ledger.record(&id, fd);
        }
    }
    drop(display);
    assert_eq!(ledger.records(), 4, "one record per number, whoever held it last");
}

#[test]
fn the_proc_path_is_formatted_in_place() {
    let mut buf = [0u8; 32];
    for (fd, expected) in [
        (0, "/proc/self/fd/0"),
        (7, "/proc/self/fd/7"),
        (1023, "/proc/self/fd/1023"),
        (i32::MAX, "/proc/self/fd/2147483647"),
    ] {
        let path = proc_fd_path(fd, &mut buf).expect("fits");
        assert_eq!(path.to_str().expect("ascii"), expected);
    }
    assert!(proc_fd_path(-1, &mut buf).is_none());
}

/// The real check against real fds: a closed number and an fd of any other
/// kind read as not held, and a real syncobj does, where this machine can
/// make one.
#[test]
fn timeline_fd_open_tells_a_syncobj_from_everything_else() {
    let eventfd = rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC)
        .expect("an eventfd");
    assert!(!timeline_fd_open(eventfd.as_raw_fd()), "an eventfd is not a timeline");
    let memfd = rustix::fs::memfd_create("retained-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    assert!(!timeline_fd_open(memfd.as_raw_fd()), "a memfd is not a timeline");
    let closed = memfd.as_raw_fd();
    drop(memfd);
    assert!(!timeline_fd_open(closed), "a closed number is not held");
    assert!(!timeline_fd_open(-1));

    let Some(syncobj) = syncobj_fd() else {
        eprintln!("timeline_fd_open_tells_a_syncobj_from_everything_else: no render node syncobj here; the syncobj half is skipped");
        return;
    };
    assert!(timeline_fd_open(syncobj.as_raw_fd()), "a syncobj fd is a timeline");
}

/// A timeline syncobj exported as an fd from this machine's render node, or
/// `None` where there is none (the same skip `drm_syncobj/tests.rs` makes).
fn syncobj_fd() -> Option<OwnedFd> {
    use smithay::reexports::drm::control::Device as ControlDevice;
    struct Node(OwnedFd);
    impl std::os::fd::AsFd for Node {
        fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    impl smithay::reexports::drm::Device for Node {}
    impl ControlDevice for Node {}
    let node = rustix::fs::open(
        "/dev/dri/renderD128",
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .ok()?;
    let node = Node(node);
    let handle = node.create_syncobj(false).ok()?;
    let fd = node.syncobj_to_fd(handle, false).ok();
    let _ = node.destroy_syncobj(handle);
    fd
}

/// Prints what one sweep costs per record, against real fds: real syncobjs
/// where this machine can make them (every record live, the full check), and
/// eventfds (every record open but of the wrong kind). Asserts nothing about
/// time; run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot sweep_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints per-record sweep timings for a human; asserts nothing"]
fn sweep_cost() {
    const RECORDS: usize = 128;
    const ROUNDS: u32 = 2_000;
    let (_display, a, _) = two_clients();
    let syncobjs: Vec<OwnedFd> = (0..RECORDS).map_while(|_| syncobj_fd()).collect();
    let eventfds: Vec<OwnedFd> = (0..RECORDS)
        .map(|_| {
            rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC).expect("an eventfd")
        })
        .collect();
    for (label, fds) in [("syncobj (live)", &syncobjs), ("eventfd (wrong kind)", &eventfds)] {
        if fds.len() < RECORDS {
            println!("sweep cost, {label}: skipped, only {} fds", fds.len());
            continue;
        }
        let mut swept = std::time::Duration::ZERO;
        for _ in 0..ROUNDS {
            let mut ledger = RetainedTimelines::default();
            for fd in fds {
                ledger.record(&a, fd.as_raw_fd());
            }
            let started = std::time::Instant::now();
            std::hint::black_box(ledger.sweep(&a, timeline_fd_open));
            swept += started.elapsed();
        }
        let per_record = swept / (ROUNDS * RECORDS as u32);
        println!(
            "sweep cost, {label}: {per_record:?} per record, so {:?} for a {RECORDS}-record sweep",
            per_record * RECORDS as u32
        );
    }
}
