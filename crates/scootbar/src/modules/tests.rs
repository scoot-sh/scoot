//! The contract, with a second, trivial module written against it here: a
//! counter fed through a pipe. It is what "one file, one registry line"
//! looks like, less the registry line (it is test-only).

use std::fmt::Write as _;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};

use super::harness::Harness;
use super::{
    Class, Init, MAX_TEXT, Module, OutputView, Placed, REGISTRY, Settings, Sources, Update, View,
    find, start,
};
use crate::layout::{Layout, Section};

/// Counts the bytes written to a pipe; shows the count, `warn` past 3.
struct Counter {
    read: OwnedFd,
    count: u64,
}

impl Module for Counter {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        sources.add(self.read.as_fd(), PollFlags::IN);
    }

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        let mut buf = [0u8; 64];
        match rustix::io::read(&self.read, &mut buf) {
            Ok(n) if n > 0 => {
                self.count += n as u64;
                Update::Changed
            }
            _ => Update::Unchanged,
        }
    }

    fn view(&self, output: &OutputView<'_>, view: &mut View) {
        let _ = write!(view.text_mut(), "{}", self.count);
        if let Some(name) = output.name {
            let _ = write!(view.tooltip_mut(), "on {name}");
        }
        view.set_icon(Some('#'));
        view.set_class(if self.count > 3 {
            Class::Warn
        } else {
            Class::Normal
        });
    }
}

fn counter() -> (Harness, OwnedFd) {
    let (read, write) = rustix::pipe::pipe().unwrap();
    rustix::fs::fcntl_setfl(&read, rustix::fs::OFlags::NONBLOCK).unwrap();
    (Harness::new(Box::new(Counter { read, count: 0 })), write)
}

#[test]
fn a_module_is_driven_by_its_sources_and_seen_through_its_view() {
    let (mut harness, write) = counter();
    assert_eq!(harness.source_count(), 1);
    assert_eq!(harness.wait(Duration::from_millis(10)), None);
    assert_eq!(harness.view().text(), "0");
    rustix::io::write(&write, b"ab").unwrap();
    assert_eq!(harness.wait(Duration::from_secs(1)), Some(Update::Changed));
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "2");
    assert_eq!(view.tooltip(), "on DP-1");
    assert_eq!(view.icon(), Some('#'));
    assert_eq!(view.class(), Class::Normal);
    rustix::io::write(&write, b"cd").unwrap();
    assert_eq!(harness.wait(Duration::from_secs(1)), Some(Update::Changed));
    assert_eq!(harness.view().class(), Class::Warn);
    // The writer gone: readable at end of file, and nothing changes.
    drop(write);
    assert_eq!(
        harness.wait(Duration::from_secs(1)),
        Some(Update::Unchanged)
    );
}

/// Fake events: a wake with nothing to read, and the error and hang-up
/// flags, handed straight to the module with no poll.
#[test]
fn fake_events_reach_the_module_without_a_poll() {
    let (mut harness, write) = counter();
    assert_eq!(harness.deliver(0, PollFlags::IN), Update::Unchanged);
    assert_eq!(harness.view().text(), "0");
    rustix::io::write(&write, b"xyz").unwrap();
    // Handed over as `POLLERR`: the counter reads whatever is there.
    assert_eq!(harness.deliver(0, PollFlags::ERR), Update::Changed);
    assert_eq!(harness.view().text(), "3");
    drop(write);
    assert_eq!(
        harness.deliver(0, PollFlags::IN | PollFlags::HUP),
        Update::Unchanged
    );
}

#[test]
#[should_panic(expected = "source 1: the module added 1")]
fn a_fake_event_on_a_source_the_module_never_added_is_a_test_bug() {
    let (mut harness, _write) = counter();
    let _ = harness.deliver(1, PollFlags::IN);
}

/// Every `revents` the loop can hand a module (`poll(2)` sets `ERR`,
/// `HUP` and `NVAL` whatever was asked), on each of its sources.
const FAKE_EVENTS: [PollFlags; 6] = [
    PollFlags::IN,
    PollFlags::PRI,
    PollFlags::ERR,
    PollFlags::HUP,
    PollFlags::NVAL,
    PollFlags::IN.union(PollFlags::HUP).union(PollFlags::ERR),
];

/// The contract, for every module in this build's registry, started as
/// the bar starts it: an unavailable one says why; an available one polls
/// within the loop's capacity, survives every event the loop can hand it
/// (none of them a panic, which would take the bar down), and still
/// fills a view within its bounds afterwards, on named and unnamed
/// outputs. A module added to the registry is held to it with no new test.
#[test]
fn every_registered_module_honours_the_contract() {
    for spec in REGISTRY {
        let mut harness = match Harness::start(spec, &Settings::default()) {
            Ok(harness) => harness,
            Err(why) => {
                assert!(!why.trim().is_empty(), "{}: unavailable, silently", spec.id);
                continue;
            }
        };
        let sources = harness.source_count();
        assert!(sources <= 8, "{}: {sources} sources", spec.id);
        for source in 0..sources {
            for events in FAKE_EVENTS {
                let _ = harness.deliver(source, events);
                for name in [None, Some("DP-1")] {
                    let view = harness.view_on(name);
                    assert!(view.text().len() <= MAX_TEXT, "{}", spec.id);
                    assert!(view.tooltip().len() <= MAX_TEXT, "{}", spec.id);
                    assert!(
                        !view.text().chars().any(char::is_control),
                        "{}: {:?} after {events:?}",
                        spec.id,
                        view.text()
                    );
                }
            }
        }
    }
}

/// "Every module ships tests through the harness", checked: each module in
/// the registry has a `tests.rs` beside it (`modules/<id>/tests.rs`, `-`
/// in an id read as `_`) that uses [`Harness`]. The contract test above is
/// no substitute: it knows nothing of what the module is for.
#[test]
fn every_registered_module_has_harness_tests() {
    let modules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/modules");
    for spec in REGISTRY {
        let path = modules.join(spec.id.replace('-', "_")).join("tests.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: no tests at {}: {e}", spec.id, path.display()));
        assert!(
            source.contains("Harness::"),
            "{}: {} does not drive it through the harness",
            spec.id,
            path.display()
        );
    }
}

#[test]
fn a_view_is_bounded_and_cut_on_a_character() {
    let mut view = View::default();
    assert!(view.is_empty());
    let long = "é".repeat(MAX_TEXT);
    assert!(write!(view.text_mut(), "{long}").is_err());
    assert!(view.text().len() <= MAX_TEXT);
    assert!(view.text().len() >= MAX_TEXT - 1);
    assert!(view.text().chars().all(|c| c == 'é'));
    // Further writes stop at the bound too.
    assert!(write!(view.text_mut(), "x").is_err());
    assert!(write!(view.tooltip_mut(), "{long}").is_err());
    assert!(view.tooltip().len() <= MAX_TEXT);
    view.clear();
    assert!(view.is_empty());
    assert_eq!(view.tooltip(), "");
    view.set_icon(Some('x'));
    assert!(!view.is_empty());
}

#[test]
fn sources_past_the_capacity_are_not_polled() {
    let (read, _write) = rustix::pipe::pipe().unwrap();
    let placeholder = rustix::fs::CWD;
    let mut fds = [
        PollFd::from_borrowed_fd(placeholder, PollFlags::empty()),
        PollFd::from_borrowed_fd(placeholder, PollFlags::empty()),
    ];
    let mut owners = [(9, 9); 2];
    let mut len = 1;
    let mut sources = Sources::new(&mut fds, &mut owners, &mut len, 4);
    assert!(sources.add(read.as_fd(), PollFlags::IN));
    assert!(!sources.add(read.as_fd(), PollFlags::IN));
    assert_eq!(len, 2);
    assert_eq!(owners, [(9, 9), (4, 0)]);
}

#[test]
fn a_change_bumps_the_revision_and_nothing_else_does() {
    let (read, write) = rustix::pipe::pipe().unwrap();
    rustix::fs::fcntl_setfl(&read, rustix::fs::OFlags::NONBLOCK).unwrap();
    let mut placed = Placed {
        section: Section::Left,
        module: Box::new(Counter { read, count: 0 }),
        revision: 0,
    };
    placed.ready(0, PollFlags::IN);
    assert_eq!(placed.revision, 0);
    rustix::io::write(&write, b"a").unwrap();
    placed.ready(0, PollFlags::IN);
    assert_eq!(placed.revision, 1);
}

#[test]
fn the_registry_has_unique_ids_that_resolve() {
    for (i, spec) in REGISTRY.iter().enumerate() {
        assert!(REGISTRY[..i].iter().all(|other| other.id != spec.id));
        assert_eq!(find(spec.id).map(|found| found.id), Some(spec.id));
        assert!(!spec.id.is_empty() && !spec.id.contains(','));
    }
    assert!(find("no-such-module").is_none());
}

#[test]
fn start_places_what_the_layout_lists_in_order() {
    let mut said = Vec::new();
    let empty = Layout {
        left: Vec::new(),
        center: Vec::new(),
        right: Vec::new(),
        ..Layout::default()
    };
    let placed = start(&empty, &Settings::default(), &mut |id, why| {
        said.push(format!("{id}: {why}"))
    });
    assert!(placed.is_empty());
    #[cfg(feature = "clock")]
    {
        let layout = Layout {
            right: vec!["clock"],
            ..empty.clone()
        };
        let placed = start(&layout, &Settings::default(), &mut |id, why| {
            said.push(format!("{id}: {why}"))
        });
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].section, Section::Right);
    }
    assert!(said.is_empty(), "{said:?}");
    let _ = Init::Unavailable(String::new());
}
