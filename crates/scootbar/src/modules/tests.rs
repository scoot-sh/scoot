//! The contract, with a second, trivial module written against it here: a
//! counter fed through a pipe. It is what "one file, one registry line"
//! looks like, less the registry line (it is test-only).

mod scan;

use std::fmt::Write as _;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags};

use super::harness::Harness;
use super::{
    Class, Init, MAX_POLL, MAX_SOURCES, MAX_TEXT, Module, OutputView, Placed, REGISTRY, Settings,
    Sources, Spec, StandIn, Update, View, find, start,
};
use crate::layout::Layout;

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

    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
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
    let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
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

/// The contract, for one registry entry, started as the bar starts it and,
/// if it has one, through its [`Spec::stand_in`]: an unavailable one says
/// why and, unavailable on this machine, must have a stand-in, or none of
/// what follows would be checked here; each started one survives every
/// event the loop can hand it on each of its sources (none of them a
/// panic, which would take the bar down) and still fills a view on named
/// and unnamed outputs afterwards, with no control character and nothing
/// cut at [`MAX_TEXT`]. Returns the most sources it saw the module add.
fn check_contract(spec: &Spec) -> usize {
    let settings = Settings::default();
    let mut started = Vec::new();
    match Harness::start(spec, &settings) {
        Ok(harness) => started.push(("init", harness)),
        Err(why) => {
            assert!(!why.trim().is_empty(), "{}: unavailable, silently", spec.id);
            assert!(
                spec.stand_in.is_some(),
                "{}: unavailable on this machine ({why}), and no `stand_in` to hold it to \
                 the contract with",
                spec.id
            );
        }
    }
    if let Some(stand_in) = spec.stand_in {
        started.push(("stand-in", Harness::new(stand_in(&settings))));
    }
    let mut most = 0;
    for (how, mut harness) in started {
        let id = format!("{} ({how})", spec.id);
        let sources = harness.source_count();
        most = most.max(sources);
        assert!(
            sources <= MAX_SOURCES,
            "{id}: {sources} sources, past the loop's {MAX_SOURCES}"
        );
        check_view(&id, &harness, "as started");
        for source in 0..sources {
            for events in FAKE_EVENTS {
                let _ = harness.deliver(source, events);
                check_view(&id, &harness, &format!("{events:?} on source {source}"));
                // What a module polls may change with what it was handed.
                let now = harness.source_count();
                most = most.max(now);
                assert!(now <= MAX_SOURCES, "{id}: {now} sources after {events:?}");
            }
        }
    }
    most
}

/// A view on an unnamed and a named output: no control character, and
/// nothing cut at [`MAX_TEXT`]. The bound is for untrusted text (a window
/// title); a module's own output under the default settings cut there is a
/// bug, shown mid-word. (Its length is no test of that: [`View`] never
/// holds more than the bound, cut or not.)
fn check_view(id: &str, harness: &Harness, after: &str) {
    for name in [None, Some("DP-1")] {
        let view = harness.view_on(name);
        assert!(
            !view.text().chars().any(char::is_control)
                && !view.tooltip().chars().any(char::is_control),
            "{id}: a control character in {:?} / {:?}, {after}",
            view.text(),
            view.tooltip()
        );
        assert!(
            !view.was_cut(),
            "{id}: its view was cut at {MAX_TEXT} bytes, {after}: {:?} / {:?}",
            view.text(),
            view.tooltip()
        );
        // A module whose view carries a tooltip says so: the bar takes a
        // pointer for tooltips from that alone, so a module that writes one
        // and does not would have a tooltip nobody can hover to.
        #[cfg(feature = "popup")]
        assert!(
            view.tooltip().is_empty() || harness.tooltips(),
            "{id}: a tooltip ({:?}) and `Module::tooltips` is false, {after}",
            view.tooltip()
        );
    }
}

/// Every module in this build's registry is held to [`check_contract`]
/// with no new test. Together they poll within the loop's capacity: a
/// layout places each at most once, so that sum is the most a layout can
/// ask of it.
#[test]
fn every_registered_module_honours_the_contract() {
    let total: usize = REGISTRY.iter().map(check_contract).sum();
    assert!(
        total <= MAX_SOURCES,
        "the registry's modules add {total} sources; the loop polls {MAX_SOURCES} \
         (MAX_POLL {MAX_POLL}, less the Wayland connection)"
    );
}

/// An unavailable counter, for the tests of the contract itself: a module
/// whose probe fails on this machine.
fn unavailable(_: &Settings) -> Init {
    Init::Unavailable("no counter here".to_owned())
}

/// The counter as its stand-in: a pipe whose writer is gone, so reading it
/// finds the end of the file.
fn counter_stand_in(_: &Settings) -> Box<dyn Module> {
    let (read, _write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    rustix::fs::fcntl_setfl(&read, rustix::fs::OFlags::NONBLOCK).unwrap();
    Box::new(Counter { read, count: 0 })
}

/// A module that panics on `POLLNVAL`, and one whose view runs past the
/// bound: what the contract exists to catch.
struct Fragile;

impl Module for Fragile {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        sources.add(rustix::fs::CWD, PollFlags::IN);
    }

    fn on_ready(&mut self, _source: usize, events: PollFlags) -> Update {
        assert!(!events.contains(PollFlags::NVAL), "fragile: NVAL");
        Update::Unchanged
    }

    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let _ = view.text_mut().write_str("ok");
    }
}

struct Verbose;

impl Module for Verbose {
    fn sources<'fd>(&'fd self, _sources: &mut Sources<'_, 'fd>) {}

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let _ = write!(view.text_mut(), "{}", "x".repeat(MAX_TEXT + 1));
    }
}

/// Registers every fd it can, one past the loop's capacity.
struct Greedy;

impl Module for Greedy {
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        for _ in 0..=MAX_SOURCES {
            sources.add(rustix::fs::CWD, PollFlags::IN);
        }
    }

    fn on_ready(&mut self, _source: usize, _events: PollFlags) -> Update {
        Update::Unchanged
    }

    fn view(&self, _output: &OutputView<'_>, _view: &mut View) {}
}

fn spec(stand_in: Option<StandIn>) -> Spec {
    Spec {
        id: "test",
        init: unavailable,
        actions: &[],
        stand_in,
    }
}

/// A module unavailable here is still driven through events and view, by
/// its stand-in.
#[test]
fn an_unavailable_module_is_held_to_the_contract_through_its_stand_in() {
    assert_eq!(check_contract(&spec(Some(counter_stand_in))), 1);
}

#[test]
#[should_panic(expected = "fragile: NVAL")]
fn a_stand_in_s_events_are_delivered() {
    check_contract(&spec(Some(|_| Box::new(Fragile))));
}

#[test]
#[should_panic(expected = "no `stand_in` to hold it to the contract with")]
fn an_unavailable_module_without_a_stand_in_fails_the_contract() {
    check_contract(&spec(None));
}

#[test]
#[should_panic(expected = "its view was cut at 256 bytes")]
fn a_view_cut_at_the_bound_fails_the_contract() {
    check_contract(&spec(Some(|_| Box::new(Verbose))));
}

#[test]
#[should_panic(expected = "64 sources, past the loop's 63")]
fn a_module_past_the_loop_s_capacity_fails_the_contract() {
    check_contract(&spec(Some(|_| Box::new(Greedy))));
}

/// "Every module ships tests through the harness", checked: each module in
/// the registry has a `tests.rs` beside it (`modules/<id>/tests.rs`, `-`
/// in an id read as `_`) whose code calls [`Harness`] (`Harness::new(` or
/// `Harness::start(`; a mention in a comment or a string is not a call).
/// The contract test above is no substitute: it knows nothing of what the
/// module is for.
#[test]
fn every_registered_module_has_harness_tests() {
    let modules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/modules");
    for spec in REGISTRY {
        let path = modules.join(spec.id.replace('-', "_")).join("tests.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: no tests at {}: {e}", spec.id, path.display()));
        assert!(
            scan::calls_harness(&source),
            "{}: {} does not drive it through the harness",
            spec.id,
            path.display()
        );
        // A file nothing declares is never compiled: its harness calls
        // would prove nothing.
        let module = modules.join(spec.id.replace('-', "_")).join("mod.rs");
        let declaration = std::fs::read_to_string(&module)
            .unwrap_or_else(|e| panic!("{}: no module file at {}: {e}", spec.id, module.display()));
        assert!(
            scan::declares_tests_module(&declaration),
            "{}: {} has no `#[cfg(test)] mod tests;`, so its tests are never compiled",
            spec.id,
            module.display()
        );
    }
}

#[test]
fn a_harness_call_is_found_in_code_only() {
    let calls = [
        "let h = Harness::new(Box::new(clock));",
        "let h = Harness :: start ( spec, &s );",
        "fn f<'a>(x: &'a str) -> char { let _ = '\"'; Harness::new(m) }",
        "let s = r#\"a \" quote\"#; Harness::new(m);",
        "/* a /* nested */ comment */ Harness::new(m);",
        "let r#type = 1; Harness::new(m);",
        "let c = '\\''; let e = b'\\\\'; Harness::new(m);",
    ];
    for code in calls {
        assert!(scan::calls_harness(code), "{code}");
    }
    let mentions = [
        "",
        "// let h = Harness::new(Box::new(clock));",
        "/// Drives it through `Harness::new(module)`.\nfn f() {}",
        "/* Harness::new(m) /* nested */ Harness::new(m) */",
        "let s = \"Harness::new(m)\";",
        "let s = r#\"Harness::new(m)\"#;",
        "let s = b\"Harness::new(m)\";",
        "let s = \"an \\\" escaped quote, then Harness::new(m)\";",
        "use super::harness::Harness;",
        "let f = Harness::new;",
        "Harness::new",
        "let s = \"unterminated Harness::new(",
        "/* unterminated Harness::new(",
        "let c = 'é'; // Harness::new(m)",
    ];
    for code in mentions {
        assert!(!scan::calls_harness(code), "{code}");
    }
}

#[test]
fn a_tests_module_is_found_in_code_only() {
    for code in [
        "#[cfg(test)]\nmod tests;",
        "#[ cfg ( test ) ] mod tests ;",
        "mod a;\n#[cfg(test)] // the tests\nmod tests;\nmod b;",
    ] {
        assert!(scan::declares_tests_module(code), "{code}");
    }
    for code in [
        "",
        "mod tests;",
        "#[cfg(any())]\nmod tests;",
        "#[cfg(test)]\nmod other;",
        "// #[cfg(test)] mod tests;",
        "/* #[cfg(test)] mod tests; */",
        "let s = \"#[cfg(test)] mod tests;\";",
        "#[cfg(test)] mod tests",
    ] {
        assert!(!scan::declares_tests_module(code), "{code}");
    }
}

/// The scanner on every prefix of a file with every construct it knows,
/// multi-byte characters included: never a panic, whatever is cut off.
#[test]
fn the_scanner_takes_any_prefix() {
    let file = "fn f<'a>() { let _ = ('é', '\\u{1F600}', b'\\'', \"s\\\"é\", r##\"r\"#\"##, \
                br\"x\", c\"y\"); /* é /* ü */ */ // ö\n Harness::new(r#m); }";
    for (end, _) in file.char_indices() {
        let _ = scan::tokens(&file[..end]);
    }
    assert!(scan::calls_harness(file));
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
    let (read, _write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
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
    let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).unwrap();
    rustix::fs::fcntl_setfl(&read, rustix::fs::OFlags::NONBLOCK).unwrap();
    let mut placed = Placed {
        bindings: Default::default(),
        id: "counter",
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
    let placed = start(
        &empty,
        &Settings::default(),
        &mut Vec::new(),
        &mut |id, why| said.push(format!("{id}: {why}")),
    );
    assert!(placed.is_empty());
    #[cfg(feature = "clock")]
    {
        let layout = Layout {
            right: vec!["clock"],
            ..empty.clone()
        };
        let placed = start(
            &layout,
            &Settings::default(),
            &mut Vec::new(),
            &mut |id, why| said.push(format!("{id}: {why}")),
        );
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].id, "clock");
    }
    assert!(said.is_empty(), "{said:?}");
    let _ = Init::Unavailable(String::new());
}
