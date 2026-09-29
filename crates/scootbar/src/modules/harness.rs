//! The module test harness: drives one module the way the daemon's loop
//! does (its sources polled, each ready one handed to `on_ready`) and reads
//! its view, with no compositor. Every module's tests go through it.

use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};

use super::{Module, OutputView, Sources, Update, View};

pub struct Harness {
    module: Box<dyn Module>,
}

impl Harness {
    pub fn new(module: Box<dyn Module>) -> Self {
        Self { module }
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

    /// How many sources the module asks the loop to poll.
    pub fn source_count(&self) -> usize {
        let placeholder = rustix::fs::CWD;
        let mut fds: Vec<PollFd<'_>> = (0..16)
            .map(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()))
            .collect();
        let mut owners = vec![(0, 0); 16];
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
        let mut fds: Vec<PollFd<'_>> = (0..16)
            .map(|_| PollFd::from_borrowed_fd(placeholder, PollFlags::empty()))
            .collect();
        let mut owners = vec![(0, 0); 16];
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
        drop(fds);
        let mut update = Update::Unchanged;
        for (flags, &(_, source)) in revents.iter().zip(&owners) {
            if !flags.is_empty() && self.module.on_ready(source, *flags) == Update::Changed {
                update = Update::Changed;
            }
        }
        Some(update)
    }
}
