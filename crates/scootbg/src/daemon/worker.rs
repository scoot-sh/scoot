//! The thread that decodes and scales, so a slow or huge image never
//! stalls the Wayland loop (frame handling, the other outputs, the control
//! socket).
//!
//! **One job at a time, one thread per job.** A thread is started for a
//! job and ends with it, so an idle daemon has one thread, and each job's
//! heap is returned as it finishes (the large-block allocator,
//! dependencies-done.md §6b). A thread costs tens of microseconds to
//! start, against hundreds of milliseconds of decoding.
//!
//! **Waking the loop without `unsafe`.** The thread sends its result down a
//! channel, then writes to an `eventfd` the loop polls (`rustix`, raw
//! syscalls). The loop reads the eventfd (resetting it) and takes the
//! result. With no job running, nothing can make the eventfd readable, so
//! it costs an idle daemon no wakeups.
//!
//! **A panic cannot hang the loop.** The release profile aborts on panic
//! (which is why every size is checked before the scaler, `image::scale`);
//! in a build that unwinds, the thread's guard still sends a result as it
//! unwinds, so the job is not waited for forever.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use rustix::event::{EventfdFlags, eventfd};
use rustix::io::Errno;
use scootbg_mem::ShmBuffer;

use crate::image::DECODE_STACK;
use crate::image::decode::{DecodeError, decode_file};
use crate::image::render::render_each;
use crate::jobs::Target;
use crate::wallpaper::Image;

#[cfg(test)]
mod tests;

/// What became of one buffer size (pixels), for every target of that size:
/// its buffer, shared by all of them, or why not (for stderr).
pub type Rendered = ((u32, u32), Result<ShmBuffer, String>);

/// A finished job: every target drawn (or not), or the image unusable.
pub type Done = Result<Vec<Rendered>, JobError>;

#[derive(Debug)]
pub enum JobError {
    Decode(DecodeError),
    /// A download failed (`crate::fetch`): said once, naming the URL.
    Fetch(crate::fetch::FetchError),
    /// The thread ended without a result (a panic, in a build that
    /// unwinds).
    Lost,
    /// No thread could be started.
    Spawn(io::Error),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(error) => write!(f, "{error}"),
            Self::Fetch(error) => write!(f, "{error}"),
            Self::Lost => write!(f, "the decoding thread failed"),
            Self::Spawn(error) => write!(f, "cannot start a decoding thread: {error}"),
        }
    }
}

impl JobError {
    /// Whether the job's download failed (said once per job, not once per
    /// output: `daemon::images`).
    pub fn is_fetch(&self) -> bool {
        matches!(self, Self::Fetch(_))
    }
}

/// A thread starter: std's in the daemon, a failing one in tests.
pub type Spawn = fn(Box<dyn FnOnce() + Send>) -> io::Result<()>;

/// Starts `job` on a named thread, which ends with it. Its stack is
/// [`DECODE_STACK`], set rather than left to std (whose default
/// `RUST_MIN_STACK` could change), so the fuzz target runs on the same.
fn spawn_thread(job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    std::thread::Builder::new()
        .name("scootbg-decode".into())
        .stack_size(DECODE_STACK)
        .spawn(job)
        .map(|handle| {
            // A test waits on its own thread, not on every thread of that
            // name in a process its neighbours share.
            #[cfg(test)]
            tests::LAST_THREAD.with(|last| *last.borrow_mut() = Some(handle));
            #[cfg(not(test))]
            drop(handle);
        })
}

/// A result, and the ticket of the job it belongs to.
type Message = (u64, Done);

pub struct Worker {
    /// Readable once a result is waiting.
    wake: Arc<OwnedFd>,
    results: Sender<Message>,
    inbox: Receiver<Message>,
    spawn: Spawn,
    /// The last job's ticket; each start takes a new one, never reused.
    ticket: u64,
    /// The ticket of the job whose result is awaited, while one is.
    awaited: Option<u64>,
}

impl Worker {
    pub fn new() -> io::Result<Self> {
        Self::with_spawn(spawn_thread)
    }

    pub fn with_spawn(spawn: Spawn) -> io::Result<Self> {
        let wake = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        let (results, inbox) = channel();
        Ok(Self {
            wake: Arc::new(wake),
            results,
            inbox,
            spawn,
            ticket: 0,
            awaited: None,
        })
    }

    #[cfg(test)]
    pub fn set_spawn(&mut self, spawn: Spawn) {
        self.spawn = spawn;
    }

    /// The fd to poll for readability.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    /// Starts a thread decoding `image` and drawing it for `targets`.
    ///
    /// A thread that cannot be started sends nothing: its error is this
    /// return value, the one place it is reported. (The guard that
    /// reports a thread that died is made inside the thread, so a spawn
    /// that never ran cannot leave a result behind.)
    ///
    /// Under parallel load thread creation can fail with EAGAIN (out of
    /// threads, not out of memory): retried a few times with a short
    /// sleep, so a momentary spike does not fail the `set`. Anything else,
    /// or EAGAIN that persists, is returned for the reply.
    pub fn start(&mut self, image: Arc<Image>, targets: Vec<Target>) -> Result<(), JobError> {
        // 2^64 jobs cannot happen; wrapping keeps it panic-free.
        self.ticket = self.ticket.wrapping_add(1);
        let ticket = self.ticket;
        // EAGAIN consumes the boxed job (it moved into the failed spawn),
        // so each attempt rebuilds it from clones: the image is shared
        // either way, and the targets are small.
        for attempt in 0..5 {
            let results = self.results.clone();
            let wake = Arc::clone(&self.wake);
            let image = Arc::clone(&image);
            let targets = targets.clone();
            let job = Box::new(move || {
                let mut guard = Guard {
                    ticket,
                    results,
                    wake,
                    sent: false,
                };
                let done = work(&image, &targets);
                // The source and every buffer not handed over are gone by
                // here; what is sent is what the outputs keep.
                guard.send(done);
            });
            match (self.spawn)(job) {
                Ok(()) => {
                    self.awaited = Some(ticket);
                    return Ok(());
                }
                Err(error)
                    if error.raw_os_error() == Some(11) // EAGAIN on Linux
                        && attempt < 4 =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    self.awaited = None;
                    return Err(JobError::Spawn(error));
                }
            }
        }
        unreachable!("the loop returns on every path");
    }

    /// The awaited job's result, if it has come. Resets the eventfd.
    /// Anything else in the channel (which should never be there) is
    /// discarded rather than taken for the awaited job's result.
    pub fn take(&mut self) -> Option<Done> {
        let mut counter = [0; 8];
        match rustix::io::read(&*self.wake, &mut counter) {
            Ok(_) | Err(Errno::AGAIN) | Err(Errno::INTR) => {}
            // An eventfd read cannot fail otherwise; the channel below is
            // the truth either way.
            Err(_) => {}
        }
        // Empty (nothing finished) or disconnected (cannot be: this holds
        // a sender) alike: nothing to land.
        while let Ok((ticket, done)) = self.inbox.try_recv() {
            if Some(ticket) == self.awaited {
                self.awaited = None;
                return Some(done);
            }
        }
        None
    }

    #[cfg(test)]
    pub fn inject(&self, ticket: u64, done: Done) {
        let _ = self.results.send((ticket, done));
        let _ = rustix::io::write(&*self.wake, &1_u64.to_ne_bytes());
    }
}

/// Sends the job's result exactly once: when asked, or when the thread
/// unwinds without having sent it.
struct Guard {
    ticket: u64,
    results: Sender<Message>,
    wake: Arc<OwnedFd>,
    sent: bool,
}

impl Guard {
    fn send(&mut self, done: Done) {
        self.sent = true;
        // The receiver lives as long as the daemon's loop; if it is gone,
        // so is anyone to tell.
        let _ = self.results.send((self.ticket, done));
        // Adds 1 to the eventfd's counter: the loop wakes. A failure
        // (the counter at its maximum, which a pending read resets long
        // before) leaves the loop to find the result on its next wakeup.
        let _ = rustix::io::write(&*self.wake, &1_u64.to_ne_bytes());
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if !self.sent {
            self.send(Err(JobError::Lost));
        }
    }
}

/// Decodes the image once and draws it once per distinct size among the
/// targets: outputs of one size share that buffer (`daemon::images`
/// offers it to each). A download is fetched first, on this thread (off
/// the loop); the decoded source is handed to the last size's draw by
/// value, so it is dropped before that buffer is allocated.
pub fn work(image: &Image, targets: &[Target]) -> Done {
    let path;
    let file = match &image.fetch {
        None => Path::new(&image.path),
        Some(fetch) => {
            path = crate::fetch::ensure_cached(fetch).map_err(JobError::Fetch)?;
            path.as_path()
        }
    };
    let decoded = decode_file(file, image.look.fill).map_err(JobError::Decode)?;
    let mut sizes: Vec<(u32, u32)> = Vec::with_capacity(targets.len());
    for target in targets {
        if !sizes.contains(&target.dims) {
            sizes.push(target.dims);
        }
    }
    let mut out = Vec::with_capacity(sizes.len());
    render_each(decoded, image.look, &sizes, |dims, drawn| {
        out.push((dims, drawn.map_err(|error| error.to_string())));
    });
    Ok(out)
}
