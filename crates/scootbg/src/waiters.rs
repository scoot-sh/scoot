//! Replies that wait for the compositor: a `set` or `clear` is answered
//! only once every output it changed shows the change *and* a
//! `wl_display.sync` sent after those commits has come back, so the
//! compositor has processed them and a screenshot taken straight after the
//! reply shows them.
//!
//! The loop never blocks on this. Each request bumps a *generation* and
//! stamps it on every output it targets (`Output::want`). A waiter is
//! resolved once no output stamped with its generation or later is still
//! [`Progress::Waiting`]; that needs no list of outputs per request, and
//! outputs that appear later carry no stamp and do not hold it up. An
//! output removed meanwhile is simply gone from the list, so the reply
//! covers the outputs that remained.
//!
//! Each loop turn, the waiters resolved in that turn move *in flight*
//! behind **one** sync, numbered; its callback moves every in-flight
//! waiter up to that number to the ready list (syncs come back in order),
//! and the loop hands those replies to their connections.
//!
//! ## Bounds, by construction
//!
//! A connection is not read while it waits, so it has at most one request
//! waiting or in flight. Waiters whose connection is gone (hung up,
//! evicted) are forgotten at the start of every loop turn
//! ([`Waiters::forget_gone`]); their changes still happen, only the reply
//! is dropped. So at any time the lists hold at most one entry per live
//! connection plus one per connection evicted during the current turn:
//! twice the connection limit, the capacity they are made with, and they
//! never grow past it. No request is ever refused for being one too many.

#[cfg(test)]
mod tests;

/// Where one output stands with respect to what it should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// It shows it (or nothing can be shown there: gave up).
    Done,
    /// Not yet: its surface is not configured, or a buffer is awaited.
    Waiting,
    /// Drawing failed (the daemon said why on stderr).
    Failed,
}

/// How a request ended, for its reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Shown,
    /// At least one targeted output could not be drawn.
    Failed,
}

/// The outcome for generation `generation` given every current output's
/// `(stamp, progress)`, or `None` while any stamped output still waits.
pub fn outcome(generation: u64, outputs: impl Iterator<Item = (u64, Progress)>) -> Option<Outcome> {
    let mut failed = false;
    for (stamp, progress) in outputs {
        if stamp < generation {
            continue;
        }
        match progress {
            Progress::Done => {}
            Progress::Waiting => return None,
            Progress::Failed => failed = true,
        }
    }
    Some(if failed {
        Outcome::Failed
    } else {
        Outcome::Shown
    })
}

/// The waiting requests, and the counters; `C` identifies the connection
/// to answer.
#[derive(Debug)]
pub struct Waiters<C> {
    /// `(connection, generation)`, not yet shown.
    waiting: Vec<(C, u64)>,
    /// `(connection, outcome, sync)`: shown, waiting for sync number
    /// `sync` to come back.
    in_flight: Vec<(C, Outcome, u64)>,
    generation: u64,
    syncs: u64,
}

impl<C: Copy> Waiters<C> {
    /// Room for `capacity` waiters in each list, which the bound above
    /// keeps them within.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            waiting: Vec::with_capacity(capacity),
            in_flight: Vec::with_capacity(capacity),
            generation: 0,
            syncs: 0,
        }
    }

    /// A new generation for a request about to change outputs. Never 0,
    /// which is every output's stamp before any request touches it.
    pub fn next_generation(&mut self) -> u64 {
        // 2^64 requests cannot happen; saturating keeps it panic-free and
        // monotonic anyway.
        self.generation = self.generation.saturating_add(1);
        self.generation
    }

    /// `conn` waits for `generation`.
    pub fn push(&mut self, conn: C, generation: u64) {
        self.waiting.push((conn, generation));
    }

    /// Drops every waiter whose connection is gone (`alive` says which are
    /// not). Call once per loop turn, before [`resolve`](Self::resolve).
    pub fn forget_gone(&mut self, alive: impl Fn(C) -> bool) {
        self.waiting.retain(|(conn, _)| alive(*conn));
        self.in_flight.retain(|(conn, ..)| alive(*conn));
    }

    /// Moves every waiter whose generation is done, as `outcome_of` says,
    /// in flight. Returns the number of the sync to send for them, if any
    /// moved. Allocates nothing.
    pub fn resolve(&mut self, mut outcome_of: impl FnMut(u64) -> Option<Outcome>) -> Option<u64> {
        let sync = self.syncs.wrapping_add(1);
        let in_flight = &mut self.in_flight;
        let mut moved = false;
        self.waiting
            .retain(|&(conn, generation)| match outcome_of(generation) {
                Some(outcome) => {
                    in_flight.push((conn, outcome, sync));
                    moved = true;
                    false
                }
                None => true,
            });
        if moved {
            self.syncs = sync;
            Some(sync)
        } else {
            None
        }
    }

    /// Sync number `sync` came back, so every earlier one did too: their
    /// waiters' replies go to `ready`.
    pub fn synced(&mut self, sync: u64, ready: &mut Vec<(C, Outcome)>) {
        self.in_flight.retain(|&(conn, outcome, number)| {
            if number <= sync {
                ready.push((conn, outcome));
                false
            } else {
                true
            }
        });
    }

    /// Nothing waits and nothing is in flight.
    pub fn is_idle(&self) -> bool {
        self.waiting.is_empty() && self.in_flight.is_empty()
    }

    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize) {
        (self.waiting.len(), self.in_flight.len())
    }
}
