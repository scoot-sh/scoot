//! The fd ledger's bookkeeping and decisions, without a compositor: fd
//! numbers are plain integers here and liveness is a set the test controls,
//! except in the tests that pin the real checks (`liveness.rs`) against real
//! fds. What the wire does with it is in `tests/shm.rs` and `tests/dmabuf.rs`
//! here, and in `drm_syncobj/tests.rs` for timelines.

use std::collections::HashSet;
use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};

use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::ClientId;

use super::liveness::{capture, proc_fd_path, still_held, timeline_fd_open};
use super::{
    Check, ClientFds, Kind, LIMITS, Limits, MAX_FDS_PER_CLIENT, PRESSURE_GRACE_FDS, Refusal,
    SWEEP_MARGIN,
};

mod dmabuf;
mod shm;

/// The timeline cap and grace the syncobj suite was written against, with a
/// total bound out of the way.
const TIMELINES: Limits = Limits {
    total: 100_000,
    timelines: 128,
    grace: 32,
};
const CAP: u32 = TIMELINES.timelines;
const GRACE: u32 = TIMELINES.grace;

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

/// Admits and records `count` arrivals of `kind` for `client` on fd numbers
/// starting at `first`, against `limits`, with nothing live but what `live`
/// names and no pressure.
fn arrive_all(
    ledger: &mut ClientFds,
    client: &ClientId,
    kind: Kind,
    limits: Limits,
    first: RawFd,
    count: u32,
    live: &HashSet<RawFd>,
) -> Result<(), Refusal> {
    for fd in first..first + count as RawFd {
        ledger.fd_arrived(fd);
        ledger.admit(
            client,
            kind,
            1,
            limits,
            |fd, _| live.contains(&fd),
            || false,
        )?;
        ledger.record(client, fd, kind, Check::Open, 1);
    }
    Ok(())
}

/// [`arrive_all`] for timelines against [`TIMELINES`].
fn import_all(
    ledger: &mut ClientFds,
    client: &ClientId,
    first: RawFd,
    count: u32,
    live: &HashSet<RawFd>,
) -> Result<(), Refusal> {
    arrive_all(
        ledger,
        client,
        Kind::Timeline,
        TIMELINES,
        first,
        count,
        live,
    )
}

/// Admits one timeline import for `client` against [`TIMELINES`].
fn admit_timeline(
    ledger: &mut ClientFds,
    client: &ClientId,
    still_held: impl FnMut(RawFd) -> bool,
    pressured: impl FnOnce() -> bool,
) -> Result<(), Refusal> {
    let mut still_held = still_held;
    ledger.admit(
        client,
        Kind::Timeline,
        1,
        TIMELINES,
        |fd, _| still_held(fd),
        pressured,
    )
}

// ---------------------------------------------------------------------------
// Bookkeeping
// ---------------------------------------------------------------------------

#[test]
fn records_count_per_client_and_drain_by_sweep() {
    let (_display, a, b) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (100..105).collect();
    import_all(&mut ledger, &a, 100, 10, &live).expect("under the cap");
    import_all(&mut ledger, &b, 200, 3, &live).expect("under the cap");
    assert_eq!(ledger.held_by(&a), 10);
    assert_eq!(ledger.held_by(&b), 3);
    assert_eq!(ledger.records(), 13);
    assert_eq!(ledger.sweep(&a, |fd, _| live.contains(&fd)), (5, 5));
    assert_eq!(ledger.held_by(&a), 5, "a sweep drops a's dead records");
    assert_eq!(ledger.timelines_held_by(&a), 5);
    assert_eq!(ledger.held_by(&b), 3, "and touches nobody else's");
    assert_eq!(ledger.sweep(&b, |_, _| false), (0, 0));
    assert_eq!(ledger.held_by(&b), 0);
    assert_eq!(
        ledger.records(),
        5,
        "an emptied client leaves no records behind"
    );
}

#[test]
fn a_reused_number_forgets_the_old_record_whoever_owned_it() {
    let (_display, a, b) = two_clients();
    let mut ledger = ClientFds::default();
    ledger.record(&a, 7, Kind::Timeline, Check::Syncobj, 1);
    ledger.record(&a, 8, Kind::Pool, Check::Open, 1);
    // Number 7 reaches the compositor again, from anyone, through anything:
    // the fd that held it is gone.
    ledger.fd_arrived(7);
    assert_eq!(ledger.held_by(&a), 1);
    assert_eq!(ledger.timelines_held_by(&a), 0, "the timeline went with it");
    // A new arrival on 8 by another client replaces a's record rather than
    // counting 8 twice.
    ledger.record(&b, 8, Kind::Plane, Check::Open, 1);
    assert_eq!(ledger.held_by(&a), 0);
    assert_eq!(ledger.held_by(&b), 1);
    assert_eq!(ledger.records(), 1);
    // A sweep of the old owner cannot take the new owner's record.
    assert_eq!(ledger.sweep(&a, |_, _| false), (0, 0));
    assert_eq!(ledger.held_by(&b), 1);
}

#[test]
fn fd_arrived_on_an_empty_ledger_is_a_no_op() {
    let mut ledger = ClientFds::default();
    ledger.fd_arrived(3);
    assert_eq!(ledger.records(), 0);
}

#[test]
fn the_ledger_is_bounded_by_fd_numbers_not_by_clients() {
    let display: Display<()> = Display::new().expect("a display");
    let mut handle = display.handle();
    struct Data;
    impl smithay::reexports::wayland_server::backend::ClientData for Data {}
    let mut ledger = ClientFds::default();
    // A thousand short-lived clients, each handing over the same few
    // numbers the kernel would hand out again, and never sweeping.
    for _ in 0..1000 {
        let (server, _client) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        let id = handle
            .insert_client(server, std::sync::Arc::new(Data))
            .expect("a client")
            .id();
        for (fd, kind) in [
            (20, Kind::Pool),
            (21, Kind::Plane),
            (22, Kind::Plane),
            (23, Kind::Timeline),
        ] {
            ledger.fd_arrived(fd);
            ledger.record(&id, fd, kind, Check::Open, 1);
        }
    }
    drop(display);
    assert_eq!(
        ledger.records(),
        4,
        "one record per number, whoever held it last"
    );
}

// ---------------------------------------------------------------------------
// The total bound: every kind together
// ---------------------------------------------------------------------------

/// The ticket's shape at the ledger level: pools, planes and timelines all
/// stay open (committed on surfaces after their objects died), so the total
/// refuses at the bound, with a fresh count, whichever kind arrives next.
#[test]
fn every_kind_counts_toward_one_total() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..10_000).collect();
    let third = MAX_FDS_PER_CLIENT / 3;
    arrive_all(&mut ledger, &a, Kind::Pool, LIMITS, 0, third, &live).expect("under");
    arrive_all(&mut ledger, &a, Kind::Plane, LIMITS, 1000, third, &live).expect("under");
    let rest = MAX_FDS_PER_CLIENT - 2 * third;
    // Timelines stop at their own cap first if the rest is past it, so pools
    // make up whatever the timeline cap leaves.
    let timelines = rest.min(LIMITS.timelines);
    arrive_all(
        &mut ledger,
        &a,
        Kind::Timeline,
        LIMITS,
        2000,
        timelines,
        &live,
    )
    .expect("under");
    arrive_all(
        &mut ledger,
        &a,
        Kind::Pool,
        LIMITS,
        3000,
        rest - timelines,
        &live,
    )
    .expect("under");
    assert_eq!(ledger.held_by(&a), MAX_FDS_PER_CLIENT);
    for kind in [Kind::Pool, Kind::Plane, Kind::Timeline] {
        assert_eq!(
            ledger.admit(&a, kind, 1, LIMITS, |fd, _| live.contains(&fd), || false),
            Err(Refusal::Total {
                held: MAX_FDS_PER_CLIENT
            }),
            "{kind:?} past the total"
        );
    }
}

/// Released fds are swept away at the bound, however many there were, so a
/// client that churns (a video player reallocating on every resolution
/// change, a toolkit re-creating pools on resize) never reaches it.
#[test]
fn churned_fds_never_reach_the_total() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..40).collect();
    arrive_all(&mut ledger, &a, Kind::Pool, LIMITS, 0, 40, &live).expect("a session's worth");
    arrive_all(
        &mut ledger,
        &a,
        Kind::Plane,
        LIMITS,
        10_000,
        10 * MAX_FDS_PER_CLIENT,
        &live,
    )
    .expect("dead records are swept");
    assert!(ledger.held_by(&a) <= MAX_FDS_PER_CLIENT);
    assert_eq!(ledger.sweep(&a, |fd, _| live.contains(&fd)).0, 40);
}

/// The timeline cap binds timelines only: a client at 128 timelines may
/// still have scoot keep pools and planes, up to the total.
#[test]
fn the_timeline_cap_does_not_bind_other_kinds() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..10_000).collect();
    arrive_all(
        &mut ledger,
        &a,
        Kind::Timeline,
        LIMITS,
        0,
        LIMITS.timelines,
        &live,
    )
    .expect("the timeline cap itself is admitted");
    arrive_all(&mut ledger, &a, Kind::Pool, LIMITS, 1000, 64, &live).expect("pools still pass");
    assert_eq!(
        ledger.admit(
            &a,
            Kind::Timeline,
            1,
            LIMITS,
            |fd, _| live.contains(&fd),
            || false
        ),
        Err(Refusal::Timelines {
            held: LIMITS.timelines
        })
    );
    assert_eq!(ledger.held_by(&a), LIMITS.timelines + 64);
}

/// A plane is admitted at its full weight, renderer copies included, and an
/// admitted arrival never takes the weight past the bound, whatever it
/// weighs: the rule the review of PR #239 found import-time charging broke.
#[test]
fn a_weighted_arrival_never_takes_the_weight_past_the_bound() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let weights = [1u8, 2, 3, 5, 2, 9];
    let mut refused = None;
    for (n, fd) in (0..10_000).enumerate() {
        let weight = weights[n % weights.len()];
        let kind = if weight == 1 { Kind::Pool } else { Kind::Plane };
        match ledger.admit(&a, kind, weight, LIMITS, |_, _| true, || false) {
            Ok(()) => ledger.record(&a, fd, kind, Check::Open, weight),
            Err(refusal) => {
                refused = Some(refusal);
                break;
            }
        }
        assert!(
            ledger.held_by(&a) <= MAX_FDS_PER_CLIENT,
            "weight {} after admitting {weight}",
            ledger.held_by(&a)
        );
    }
    assert!(
        matches!(refused, Some(Refusal::Total { .. })),
        "{refused:?}"
    );

    // The boundary exactly: 510 held, a two-fd plane fits, the next does not.
    let mut ledger = ClientFds::default();
    arrive_all(
        &mut ledger,
        &a,
        Kind::Pool,
        LIMITS,
        0,
        510,
        &(0..600).collect(),
    )
    .expect("510");
    assert_eq!(
        ledger.admit(&a, Kind::Plane, 2, LIMITS, |_, _| true, || false),
        Ok(())
    );
    ledger.record(&a, 1000, Kind::Plane, Check::Open, 2);
    assert_eq!(ledger.held_by(&a), MAX_FDS_PER_CLIENT);
    assert_eq!(
        ledger.admit(&a, Kind::Plane, 2, LIMITS, |_, _| true, || false),
        Err(Refusal::Total {
            held: MAX_FDS_PER_CLIENT
        })
    );
}

/// The pressure grace weighs an arrival the same way: a client at the grace
/// may add one more fd's worth, and a heavier arrival from there is checked.
#[test]
fn the_pressure_grace_weighs_the_arrival() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..1000).collect();
    arrive_all(
        &mut ledger,
        &a,
        Kind::Pool,
        LIMITS,
        0,
        PRESSURE_GRACE_FDS - 1,
        &live,
    )
    .expect("under");
    // 127 held: a two-fd plane takes it to 129, one past the grace, like a
    // one-fd arrival at 128 would: admitted.
    assert_eq!(
        ledger.admit(
            &a,
            Kind::Plane,
            2,
            LIMITS,
            |fd, _| live.contains(&fd),
            || true
        ),
        Ok(())
    );
    // (Recorded at weight 1, to stand exactly at the grace for the next.)
    ledger.record(&a, 500, Kind::Plane, Check::Open, 1);
    // 128 held: the same plane would take it to 130, and is refused.
    assert_eq!(
        ledger.admit(
            &a,
            Kind::Plane,
            2,
            LIMITS,
            |fd, _| live.contains(&fd),
            || true
        ),
        Err(Refusal::Pressure {
            held: PRESSURE_GRACE_FDS
        })
    );
}

/// Settling a plane's copies after import only ever lowers its weight.
#[test]
fn settling_copies_only_lowers_a_weight() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    ledger.record(&a, 7, Kind::Plane, Check::Open, 3);
    ledger.settle_copies(7, 4);
    assert_eq!(ledger.held_by(&a), 3, "never raised");
    ledger.settle_copies(7, 1);
    assert_eq!(ledger.held_by(&a), 2);
    ledger.settle_copies(7, 0);
    assert_eq!(ledger.held_by(&a), 1);
    ledger.settle_copies(99, 0);
    assert_eq!(ledger.records(), 1);
}

/// A client whose records keep emptying and refilling -- one pool at a
/// time, its number coming back each time -- reuses its record list rather
/// than allocating one per arrival.
#[test]
fn an_emptied_entry_leaves_its_list_for_reuse() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    ledger.record(&a, 30, Kind::Pool, Check::Open, 1);
    ledger.fd_arrived(30);
    assert_eq!(ledger.records(), 0);
    assert_eq!(ledger.spare.len(), 1, "the emptied list was kept");
    let capacity = ledger.spare[0].capacity();
    ledger.record(&a, 30, Kind::Pool, Check::Open, 1);
    assert!(ledger.spare.is_empty(), "and taken back");
    assert_eq!(ledger.per_client[&a].records.capacity(), capacity);
}

// ---------------------------------------------------------------------------
// The timeline cap (ported from `drm_syncobj/retained.rs`)
// ---------------------------------------------------------------------------

/// Every timeline stays held (its points outlive the object), so the cap
/// refuses exactly at the cap, with a fresh count.
#[test]
fn a_client_really_holding_the_timeline_cap_is_refused_at_it() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (1000..2000).collect();
    import_all(&mut ledger, &a, 1000, CAP, &live).expect("the cap itself is admitted");
    assert_eq!(ledger.timelines_held_by(&a), CAP);
    let refusal = admit_timeline(&mut ledger, &a, |fd| live.contains(&fd), || false);
    assert_eq!(refusal, Err(Refusal::Timelines { held: CAP }));
}

/// Swapchain churn: imports whose timelines are gone by the time the cap is
/// reached are swept away, not counted, however many there were.
#[test]
fn churned_timelines_never_reach_the_cap() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    // 16 live (one Vulkan window), then 10 x the cap of imports that die.
    let live: HashSet<RawFd> = (0..16).collect();
    import_all(&mut ledger, &a, 0, 16, &live).expect("a window's worth");
    import_all(&mut ledger, &a, 10_000, 10 * CAP, &live).expect("dead records are swept");
    assert!(
        ledger.timelines_held_by(&a) <= CAP,
        "the records never exceed the cap"
    );
    assert_eq!(ledger.sweep(&a, |fd, _| live.contains(&fd)), (16, 16));
}

/// The amortization the margin exists for: a client really holding one
/// under the cap is refused, rather than admitted with a full sweep on every
/// import it churns.
#[test]
fn a_client_within_the_margin_of_the_cap_is_refused_not_swept_per_import() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..CAP as RawFd - 1).collect();
    import_all(&mut ledger, &a, 0, CAP - 1, &live).expect("under the cap");
    ledger.record(&a, 5000, Kind::Timeline, Check::Syncobj, 1); // one churned import, dead
    let mut checks = 0;
    let refusal = admit_timeline(
        &mut ledger,
        &a,
        |fd| {
            checks += 1;
            live.contains(&fd)
        },
        || false,
    );
    assert_eq!(refusal, Err(Refusal::Timelines { held: CAP - 1 }));
    assert_eq!(checks, CAP, "one sweep, of the client's own records");
}

#[test]
fn a_sweep_that_frees_the_margin_admits_and_buys_that_many_imports() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live_count = CAP - SWEEP_MARGIN;
    let live: HashSet<RawFd> = (0..live_count as RawFd).collect();
    import_all(&mut ledger, &a, 0, live_count, &live).expect("under the cap");
    import_all(&mut ledger, &a, 5000, SWEEP_MARGIN, &live).expect("to the cap");
    let mut sweeps = 0;
    for fd in 6000..6000 + SWEEP_MARGIN as RawFd {
        let mut swept = false;
        admit_timeline(
            &mut ledger,
            &a,
            |fd| {
                swept = true;
                live.contains(&fd) || fd >= 6000
            },
            || false,
        )
        .expect("the dead margin is reclaimed");
        sweeps += u32::from(swept);
        ledger.record(&a, fd, Kind::Timeline, Check::Syncobj, 1);
    }
    assert_eq!(
        sweeps, 1,
        "one sweep for the whole margin's worth of imports"
    );
}

// ---------------------------------------------------------------------------
// The pressure grace
// ---------------------------------------------------------------------------

#[test]
fn nothing_is_observed_under_the_grace() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    for (n, fd) in (0..GRACE as RawFd).enumerate() {
        let kind = [Kind::Pool, Kind::Plane, Kind::Timeline][n % 3];
        ledger
            .admit(
                &a,
                kind,
                1,
                TIMELINES,
                |_, _| panic!("no sweep under the grace"),
                || panic!("no table observation under the grace"),
            )
            .expect("under the grace");
        ledger.record(&a, fd, kind, Check::Open, 1);
    }
}

/// The production grace, both boundaries: at it passes under pressure, one
/// past it (on a fresh count) is refused, and past it with a calm table
/// passes. The `>`/`&&` shape `dispatch::pressure_refusal_for` pins for the
/// acquire waits.
#[test]
fn the_production_grace_refuses_one_past_it_only_under_pressure() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..10_000).collect();
    arrive_all(
        &mut ledger,
        &a,
        Kind::Pool,
        LIMITS,
        0,
        PRESSURE_GRACE_FDS,
        &live,
    )
    .expect("calm");
    assert_eq!(
        ledger.admit(
            &a,
            Kind::Plane,
            1,
            LIMITS,
            |fd, _| live.contains(&fd),
            || true
        ),
        Ok(()),
        "at the grace passes under pressure"
    );
    ledger.record(&a, 9000, Kind::Plane, Check::Open, 1);
    let mut calm = ClientFds::default();
    arrive_all(
        &mut calm,
        &a,
        Kind::Pool,
        LIMITS,
        0,
        PRESSURE_GRACE_FDS + 1,
        &live,
    )
    .expect("past the grace with a calm table passes");
    assert_eq!(
        ledger.admit(
            &a,
            Kind::Pool,
            1,
            LIMITS,
            |fd, _| live.contains(&fd) || fd == 9000,
            || true
        ),
        Err(Refusal::Pressure {
            held: PRESSURE_GRACE_FDS + 1
        }),
        "one past the grace under pressure is refused"
    );
}

#[test]
fn under_pressure_a_client_past_the_grace_is_refused_on_a_fresh_count() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..=GRACE as RawFd).collect();
    import_all(&mut ledger, &a, 0, GRACE + 1, &live).expect("no pressure yet");
    let refusal = admit_timeline(&mut ledger, &a, |fd| live.contains(&fd), || true);
    assert_eq!(refusal, Err(Refusal::Pressure { held: GRACE + 1 }));
}

#[test]
fn under_pressure_dead_records_do_not_count_against_the_grace() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    // Ten live, then enough dead to put the raw count far past the grace.
    let live: HashSet<RawFd> = (0..10).collect();
    import_all(&mut ledger, &a, 0, 10, &live).expect("under the grace");
    for fd in 100..100 + 3 * GRACE as RawFd {
        ledger.record(&a, fd, Kind::Pool, Check::Open, 1);
    }
    assert!(ledger.held_by(&a) > GRACE);
    admit_timeline(&mut ledger, &a, |fd| live.contains(&fd), || true)
        .expect("the fresh count is 10, under the grace");
    assert_eq!(ledger.held_by(&a), 10);
}

#[test]
fn under_pressure_sweeps_are_amortized_by_the_margin() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    // Exactly the grace live, plus one dead record: the first sweep under
    // pressure admits, and buys the margin's worth of unswept arrivals.
    let live: HashSet<RawFd> = (0..10_000).collect();
    import_all(&mut ledger, &a, 0, GRACE, &live).expect("no pressure yet");
    ledger.record(&a, 20_000, Kind::Plane, Check::Open, 1);
    let (mut sweeps, mut observations) = (0, 0);
    let mut outcome = Ok(());
    for fd in GRACE as RawFd..GRACE as RawFd + 2 * SWEEP_MARGIN as RawFd {
        let mut swept = false;
        outcome = admit_timeline(
            &mut ledger,
            &a,
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
        ledger.record(&a, fd, Kind::Timeline, Check::Syncobj, 1);
    }
    assert_eq!(
        outcome,
        Err(Refusal::Pressure {
            held: GRACE + SWEEP_MARGIN
        }),
        "the slack past a sweep that admitted is the margin, and not a record more"
    );
    assert_eq!(
        sweeps, 2,
        "the sweep that admitted, and the one that refused"
    );
    assert_eq!(
        observations, 2,
        "the table is observed only when a sweep is due"
    );
}

/// A legitimate churner, calm table: its records run far past the grace
/// with dead ones while it holds 16 live, and the table is observed at most
/// once per margin's worth of arrivals rather than on every one.
#[test]
fn a_calm_table_is_observed_once_per_margin_not_per_arrival() {
    const ARRIVALS: u32 = 200;
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let live: HashSet<RawFd> = (0..16).collect();
    import_all(&mut ledger, &a, 0, 16, &live).expect("a window's worth");
    let mut observations = 0;
    for fd in 1000..1000 + ARRIVALS as RawFd {
        admit_timeline(
            &mut ledger,
            &a,
            |fd| live.contains(&fd),
            || {
                observations += 1;
                false
            },
        )
        .expect("calm, and under the cap once swept");
        ledger.record(&a, fd, Kind::Timeline, Check::Syncobj, 1);
    }
    assert!(
        observations <= ARRIVALS / SWEEP_MARGIN + 1,
        "{observations} observations for {ARRIVALS} arrivals"
    );
    assert!(
        observations > 0,
        "past the grace the table is still watched"
    );
}

// ---------------------------------------------------------------------------
// The real checks, against real fds
// ---------------------------------------------------------------------------

fn memfd(name: &str) -> OwnedFd {
    let fd = rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC).expect("a memfd");
    rustix::fs::ftruncate(&fd, 4096).expect("a page");
    fd
}

/// A pool or plane record is the file, not the number: it reads live while
/// the same file is on the number, and dead once the number is closed or
/// reused by any other file -- another memfd, an eventfd, a socket -- which
/// is what keeps scoot's own fds from being counted as a client's.
#[test]
fn an_identity_check_follows_the_file_not_the_number() {
    for kind in [Kind::Pool, Kind::Plane] {
        let first = memfd("client-fds-identity");
        let number = first.as_raw_fd();
        let check = capture(first.as_fd(), kind);
        assert!(matches!(check, Check::Same { .. }), "{check:?}");
        assert!(still_held(number, check), "the same file is held");
        drop(first);
        assert!(!still_held(number, check), "a closed number is not held");
        for reuse in [
            memfd("client-fds-identity-reuse"),
            rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC).expect("an eventfd"),
            OwnedFd::from(
                std::os::unix::net::UnixStream::pair()
                    .expect("a socket pair")
                    .0,
            ),
        ] {
            // The kernel hands out the lowest free number, and nothing else
            // in this test runs between the close and this open; if another
            // test's thread took it first the assertion is simply vacuous.
            if reuse.as_raw_fd() == number {
                assert!(
                    !still_held(number, check),
                    "a number reused by another file is not held"
                );
            }
            drop(reuse);
        }
    }
    assert!(!still_held(-1, Check::Open));
    assert!(!still_held(-1, Check::Same { dev: 0, ino: 0 }));
}

/// Two dma-buf planes named by one client fd arrive as two fds here: two
/// numbers, one file. Each record holds while its own number is open.
#[test]
fn two_numbers_on_one_file_are_two_records() {
    let (_display, a, _) = two_clients();
    let mut ledger = ClientFds::default();
    let file = memfd("client-fds-two-planes");
    let dup = file.try_clone().expect("a second fd on the same file");
    for fd in [&file, &dup] {
        ledger.fd_arrived(fd.as_raw_fd());
        ledger.record_arrival(&a, fd.as_fd(), Kind::Plane, 1);
    }
    assert_eq!(ledger.sweep(&a, still_held).0, 2);
    drop(dup);
    assert_eq!(ledger.sweep(&a, still_held).0, 1, "one number closed");
    drop(file);
    assert_eq!(ledger.sweep(&a, still_held).0, 0);
}

#[test]
fn a_timeline_arrival_captures_the_syncobj_check_without_a_syscall_of_its_own() {
    let eventfd =
        rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC).expect("an eventfd");
    assert_eq!(capture(eventfd.as_fd(), Kind::Timeline), Check::Syncobj);
    assert!(
        !still_held(eventfd.as_raw_fd(), Check::Syncobj),
        "an eventfd is not a timeline"
    );
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
    let eventfd =
        rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC).expect("an eventfd");
    assert!(
        !timeline_fd_open(eventfd.as_raw_fd()),
        "an eventfd is not a timeline"
    );
    let memfd = rustix::fs::memfd_create("retained-test", rustix::fs::MemfdFlags::CLOEXEC)
        .expect("a memfd");
    assert!(
        !timeline_fd_open(memfd.as_raw_fd()),
        "a memfd is not a timeline"
    );
    let closed = memfd.as_raw_fd();
    drop(memfd);
    assert!(!timeline_fd_open(closed), "a closed number is not held");
    assert!(!timeline_fd_open(-1));

    let Some(syncobj) = syncobj_fd() else {
        eprintln!(
            "timeline_fd_open_tells_a_syncobj_from_everything_else: no render node syncobj here; the syncobj half is skipped"
        );
        return;
    };
    assert!(
        timeline_fd_open(syncobj.as_raw_fd()),
        "a syncobj fd is a timeline"
    );
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

// ---------------------------------------------------------------------------
// Costs, for a human
// ---------------------------------------------------------------------------

/// Prints what one sweep costs per record, against real fds: memfds (the
/// identity check, every record live), real syncobjs where this machine can
/// make them (the link check, every record live), and eventfds (open, but of
/// the wrong kind). Asserts nothing about time; run by hand:
///
/// ```text
/// cargo test --release -p scoot --bin scoot client_fds::tests::sweep_cost -- --ignored --nocapture
/// ```
#[test]
#[ignore = "prints per-record sweep timings for a human; asserts nothing"]
fn sweep_cost() {
    const RECORDS: usize = 128;
    const ROUNDS: u32 = 2_000;
    let (_display, a, _) = two_clients();
    let memfds: Vec<OwnedFd> = (0..RECORDS).map(|_| memfd("client-fds-cost")).collect();
    let syncobjs: Vec<OwnedFd> = (0..RECORDS).map_while(|_| syncobj_fd()).collect();
    let eventfds: Vec<OwnedFd> = (0..RECORDS)
        .map(|_| {
            rustix::event::eventfd(0, rustix::event::EventfdFlags::CLOEXEC).expect("an eventfd")
        })
        .collect();
    for (label, fds, kind) in [
        ("memfd pool (identity, live)", &memfds, Kind::Pool),
        ("syncobj (link, live)", &syncobjs, Kind::Timeline),
        ("eventfd (link, wrong kind)", &eventfds, Kind::Timeline),
    ] {
        if fds.len() < RECORDS {
            println!("sweep cost, {label}: skipped, only {} fds", fds.len());
            continue;
        }
        let mut swept = std::time::Duration::ZERO;
        for _ in 0..ROUNDS {
            let mut ledger = ClientFds::default();
            for fd in fds {
                ledger.record_arrival(&a, fd.as_fd(), kind, 1);
            }
            let started = std::time::Instant::now();
            std::hint::black_box(ledger.sweep(&a, still_held));
            swept += started.elapsed();
        }
        let per_record = swept / (ROUNDS * RECORDS as u32);
        println!(
            "sweep cost, {label}: {per_record:?} per record, so {:?} for a {RECORDS}-record sweep",
            per_record * RECORDS as u32
        );
    }
}

/// Prints what one arrival costs the ledger on the admitted path, the one
/// every `create_pool`, `add` and `import_timeline` pays: forget the number,
/// decide against the bounds, and record (an `fstat` for a pool or plane).
/// Measured in the steady state of a client that holds 40 fds and churns one
/// more, whose number comes back every time. Asserts nothing about time; run
/// by hand as for [`sweep_cost`].
#[test]
#[ignore = "prints per-arrival timings for a human; asserts nothing"]
fn arrival_cost() {
    const ROUNDS: u32 = 200_000;
    let (_display, a, _) = two_clients();
    let held: Vec<OwnedFd> = (0..40).map(|_| memfd("client-fds-held")).collect();
    let churned = memfd("client-fds-churned");
    for (label, kind) in [
        ("pool/plane", Kind::Pool),
        ("timeline (no fstat)", Kind::Timeline),
    ] {
        let mut ledger = ClientFds::default();
        for fd in &held {
            ledger.record_arrival(&a, fd.as_fd(), Kind::Pool, 1);
        }
        let started = std::time::Instant::now();
        for _ in 0..ROUNDS {
            ledger
                .admit_arrival(&a, churned.as_raw_fd(), kind, 1)
                .expect("under every bound");
            ledger.record_arrival(&a, churned.as_fd(), kind, 1);
        }
        let per = started.elapsed() / ROUNDS;
        println!("arrival cost, {label}: {per:?} per arrival (admit and record)");
        std::hint::black_box(&ledger);
    }
}
