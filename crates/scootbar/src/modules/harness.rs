//! The module test harness: drives one module the way the daemon's loop
//! does and reads its view, with no compositor. Two ways in:
//!
//! - **Real events** ([`Harness::wait`]): the module's sources polled, each
//!   ready one handed to `on_ready`, as the loop does.
//! - **Fake events** ([`Harness::deliver`]): any events on any source the
//!   module added, handed over without polling, for what the kernel sends
//!   rarely or at a bad time (a wake with nothing to read, `POLLERR`,
//!   `POLLHUP`).
//! - **Dispatched Wayland state** ([`Harness::dispatch`]) and **pointer
//!   input** ([`Harness::input`], [`Harness::invoke`]): what the loop hands
//!   a module after its central dispatch and pointer routing, for the
//!   modules that keep Wayland objects (workspaces) rather than fd sources.
//!
//! Either way the test asserts on the returned [`Update`] and the [`View`].
//! Every module's tests go through it, and a module in the registry
//! without them fails `modules::tests::every_registered_module_has_harness_tests`.

use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

#[cfg(feature = "exec")]
use super::Placed;
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

    /// A turn of the loop after its Wayland dispatch: what arrived on the
    /// connection's fd, reported through [`Module::on_dispatch`].
    /// Only the workspaces module's tests call it.
    #[allow(dead_code)]
    pub fn dispatch(&mut self) -> Update {
        self.module.on_dispatch()
    }

    /// A pointer input in the module's span, as the loop routes one: the
    /// action it means by default.
    #[allow(dead_code)]
    pub fn input(&self, input: &super::Input<'_>) -> Option<crate::action::Action> {
        self.module.on_input(input)
    }

    /// The device pixels the module's span grows past its measured text.
    #[allow(dead_code)]
    pub fn span_extra(&self, measure: &super::Measure<'_>) -> u32 {
        self.module.span_extra(measure)
    }

    /// A module action, as a binding, a default or an agent runs it.
    #[allow(dead_code)]
    pub fn invoke(
        &mut self,
        output: &OutputView<'_>,
        action: &crate::action::ModuleAction,
        steps: u32,
    ) -> Result<Update, super::InvokeError> {
        self.module.invoke(output, action, steps)
    }

    /// An action the module asks the bar to carry out, taken once.
    #[allow(dead_code)]
    pub fn take_action(&mut self) -> Option<crate::action::Action> {
        self.module.take_action()
    }

    /// A `scootbar msg set` value, as the daemon hands it over.
    #[allow(dead_code)]
    pub fn set(&mut self, value: &serde_json::Value) -> Result<Update, super::SetError> {
        self.module.on_set(value)
    }

    /// The module's view for an unnamed output.
    /// Whether the module says it may show a tooltip (`Module::tooltips`).
    #[cfg(feature = "popup")]
    pub fn tooltips(&self) -> bool {
        self.module.tooltips()
    }

    pub fn view(&self) -> View {
        self.view_on(None)
    }

    pub fn view_on(&self, name: Option<&str>) -> View {
        let mut view = View::default();
        self.module.view(&OutputView { name }, &mut view);
        view
    }

    /// The module's popup content for an unnamed output: whether it has one
    /// now.
    #[cfg(all(feature = "popup", any(feature = "volume", feature = "microphone")))]
    pub fn popup(&self, content: &mut crate::popup::Content) -> bool {
        self.module.popup(&OutputView { name: None }, content)
    }

    /// The module's `value` for the output `name`, as `query` reports it.
    #[allow(dead_code)]
    pub fn value_on(&self, name: Option<&str>) -> Option<serde_json::Value> {
        self.module.value(&OutputView { name })
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

/// Turns of the loop for `placed`, until `done` holds, or the test fails:
/// what [`Harness::wait`] is for one module, for a module that lives in a
/// [`Placed`] (a reload hands the old bar's over), which the harness
/// cannot take apart. Goes through [`Placed::ready`], so the revision
/// moves as the loop moves it. Only the `exec` reload tests drive one.
#[cfg(feature = "exec")]
pub fn drive_placed(placed: &mut Placed, what: &str, mut done: impl FnMut(&Placed) -> bool) {
    let start = std::time::Instant::now();
    let deadline = Duration::from_secs(10);
    while !done(placed) {
        assert!(start.elapsed() < deadline, "never {what}: {}", placed.id);
        let placeholder = rustix::fs::CWD;
        let mut fds: [PollFd<'_>; MAX_POLL] =
            std::array::from_fn(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()));
        let mut owners = [(0, 0); MAX_POLL];
        let mut len = 0;
        let mut sources = Sources::new(&mut fds, &mut owners, &mut len, 0);
        placed.module.sources(&mut sources);
        let timeout = Timespec {
            tv_sec: 0,
            tv_nsec: 50_000_000,
        };
        let ready = poll(&mut fds[..len], Some(&timeout)).expect("poll");
        if ready == 0 {
            continue;
        }
        let revents: Vec<PollFlags> = fds[..len].iter().map(PollFd::revents).collect();
        for (flags, &(_, source)) in revents.iter().zip(&owners) {
            if !flags.is_empty() {
                placed.ready(source, *flags);
            }
        }
    }
}
