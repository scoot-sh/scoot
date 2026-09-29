//! The module test harness: drives one module the way the daemon's loop
//! does and reads its view, with no compositor. Two ways in:
//!
//! - **Real events** ([`Harness::wait`]): the module's sources polled, each
//!   ready one handed to `on_ready`, as the loop does.
//! - **Fake events** ([`Harness::deliver`]): any events on any source the
//!   module added, handed over without polling, for what the kernel sends
//!   rarely or at a bad time (a wake with nothing to read, `POLLERR`,
//!   `POLLHUP`).
//!
//! Either way the test asserts on the returned [`Update`] and the [`View`].
//! Every module's tests go through it, and a module in the registry
//! without them fails `modules::tests::every_registered_module_has_harness_tests`.

use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use super::{Init, MAX_POLL, Module, OutputView, Settings, Sources, Spec, Update, View};

pub struct Harness {
    module: Box<dyn Module>,
}

impl Harness {
    pub fn new(module: Box<dyn Module>) -> Self {
        Self { module }
    }

    /// The registry's module `spec`, started with `settings` as the bar
    /// starts it; `Err` with its reason when it is unavailable.
    pub fn start(spec: &Spec, settings: &Settings) -> Result<Self, String> {
        match (spec.init)(settings) {
            Init::Available(module) => Ok(Self::new(module)),
            Init::Unavailable(why) => Err(why),
        }
    }

    /// A fake event: `events` on the module's source `source`, handed to
    /// `on_ready` as the loop hands a ready one, without polling. The loop
    /// only hands over sources the module added this turn, so `source`
    /// must be below [`Harness::source_count`].
    pub fn deliver(&mut self, source: usize, events: PollFlags) -> Update {
        let added = self.source_count();
        assert!(source < added, "source {source}: the module added {added}");
        self.module.on_ready(source, events)
    }

    /// The module's view for an unnamed output.
    pub fn view(&self) -> View {
        self.view_on(None)
    }

    pub fn view_on(&self, name: Option<&str>) -> View {
        let mut view = View::default();
        self.module.view(&OutputView { name }, &mut view);
        view
    }

    /// How many sources the module asks the loop to poll, counted up to
    /// [`MAX_POLL`]: one past what the loop has for all the modules
    /// together, so a module over that shows.
    pub fn source_count(&self) -> usize {
        let placeholder = rustix::fs::CWD;
        let mut fds: [PollFd<'_>; MAX_POLL] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
        let mut owners = [(0, 0); MAX_POLL];
        let mut len = 0;
        let mut sources = Sources::new(&mut fds, &mut owners, &mut len, 0);
        self.module.sources(&mut sources);
        len
    }

    /// Waits up to `timeout` for a source to be ready and hands every
    /// ready one to the module. `None` when nothing was ready in time;
    /// otherwise `Changed` if any hand-over changed the view.
    pub fn wait(&mut self, timeout: Duration) -> Option<Update> {
        let placeholder = rustix::fs::CWD;
        let mut fds: [PollFd<'_>; MAX_POLL] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
        let mut owners = [(0, 0); MAX_POLL];
        let mut len = 0;
        let mut sources = Sources::new(&mut fds, &mut owners, &mut len, 0);
        self.module.sources(&mut sources);
        let timeout = Timespec {
            tv_sec: timeout.as_secs() as i64,
            tv_nsec: i64::from(timeout.subsec_nanos()),
        };
        let ready = poll(&mut fds[..len], Some(&timeout)).expect("poll");
        if ready == 0 {
            return None;
        }
        let revents: Vec<PollFlags> = fds[..len].iter().map(PollFd::revents).collect();
        let mut update = Update::Unchanged;
        for (flags, &(_, source)) in revents.iter().zip(&owners) {
            if !flags.is_empty() && self.module.on_ready(source, *flags) == Update::Changed {
                update = Update::Changed;
            }
        }
        Some(update)
    }
}
