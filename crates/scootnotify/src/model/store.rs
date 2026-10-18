//! The live set: ids, replace, caps, rate limits, and close reasons.
//!
//! Pure state, no I/O (the scoot-core discipline): the daemon moves
//! bytes and clocks, this module decides. Every rule the conformance
//! table states for the server half lives here:
//!
//! - ids: a u32 counter, 0 skipped, live ids skipped, reserved in blocks
//!   of 1024 in `$XDG_RUNTIME_DIR`, so a crash restart never reissues an
//!   id a client may still hold;
//! - `replaces_id` honored for the same unique sender or the same
//!   `app_name` (a mismatch is a fresh notification), swapped in place
//!   with the expiry restarted, an unknown id a fresh notification;
//! - `CloseNotification` for the same sender or app (a mismatch is an
//!   unknown id), closing with reason 3;
//! - expiry resolved from presentation (a stub presenter in tests);
//! - global cap: at most `max_live` live, past it the oldest
//!   non-critical closes with reason 4 (the oldest critical, when only
//!   criticals are live — something must give, and the signal says so);
//! - rate limits: a per-sender token bucket (20/s, burst 40); past it a
//!   `Notify` still gets its id, but the card is counted unshown, never
//!   stored. A replace bypasses the bucket (a volume OSD holding one
//!   card must not trip it).

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::bus::hints::Urgency;
use crate::bus::image::Image;
use crate::bus::notify::{Action, Incoming};
use crate::config::Config;

use super::expiry::{Clock, resolve_timeout};

/// Ids reserved per file write: a crash loses at most a block's
/// worth of numbers, never a reissue.
pub const ID_BLOCK: u64 = 1024;

/// Rate-bucket senders past this count prune the idle ones: unique names
/// are per-connection (every `notify-send` is a new one), so an
/// unbounded map would be the flood's memory.
pub const MAX_SENDERS: usize = 1024;

/// `NotificationClosed` reasons, the spec's §9.2.1 four: the id is
/// invalidated before the signal is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// 1: the expiry fired.
    Expired,
    /// 2: dismissed by the user (a click, a keybind, `msg dismiss` —
    /// kept for the UI and control tickets; unreachable headless).
    Dismissed,
    /// 3: `CloseNotification`.
    Closed,
    /// 4: evicted by the live cap, or the daemon shutting down cleanly.
    Evicted,
}

impl CloseReason {
    /// The signal's wire code.
    pub fn code(self) -> u32 {
        match self {
            Self::Expired => 1,
            Self::Dismissed => 2,
            Self::Closed => 3,
            Self::Evicted => 4,
        }
    }
}

/// One icon slot per card, chosen `image-data` > `image-path` >
/// `app_icon`: the only image the daemon keeps is the downscaled one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    None,
    Data(Image),
    /// A `file://` URI or absolute path, validated at shape (loaded at
    /// draw time, in the popups ticket).
    Path(String),
    /// A theme name (looked up through the launcher's index when it
    /// exists; until then tried as a name).
    Name(String),
}

/// Classifies `app_icon`/`image-path` by shape: an absolute path or a
/// `file://` URI with percent-decoding, else a theme name (at most 128
/// B, no `/`, no NUL, no leading dot). Every other scheme means no
/// image. Empty means none.
pub fn classify_icon(text: &str) -> Icon {
    if text.is_empty() {
        return Icon::None;
    }
    if let Some(rest) = text.strip_prefix("file://") {
        return match percent_decode(rest) {
            Some(path) if path.starts_with('/') => Icon::Path(path),
            _ => Icon::None,
        };
    }
    if text.starts_with('/') {
        return Icon::Path(text.to_owned());
    }
    if text.contains("://") || text.contains(':') {
        return Icon::None;
    }
    if text.len() > 128 || text.contains('/') || text.contains('\0') || text.starts_with('.') {
        return Icon::None;
    }
    Icon::Name(text.to_owned())
}

/// Percent-decodes `text` (`%xx` hex, a malformed escape kept as-is):
/// `None` on non-UTF-8, which is no path.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                (bytes[at + 1] as char).to_digit(16),
                (bytes[at + 2] as char).to_digit(16),
            )
        {
            out.push((high * 16 + low) as u8);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// One live notification: everything kept, bounded at parse.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    /// The unique sender at arrival: the replaces/close policy.
    pub sender: String,
    pub icon: Icon,
    pub summary: String,
    /// The raw body; markup parses at draw time.
    pub body: String,
    /// Markup stripped: the JSON view and the history ring keep this.
    pub plain_body: String,
    pub actions: Vec<Action>,
    pub urgency: Urgency,
    pub category: String,
    pub desktop_entry: String,
    pub transient: bool,
    pub resident: bool,
    pub value: Option<u8>,
    /// Insertion order: the eviction oldest-first rule.
    pub seq: u64,
    /// The resolved expiry in ms (`None` never): presentation starts
    /// this; a replace re-resolves it.
    pub timeout_ms: Option<u32>,
    /// CLOCK_REALTIME ms at arrival: the JSON `created`.
    pub created_ms: u64,
    /// CLOCK_REALTIME ms at first presentation, `None` until shown.
    pub shown_ms: Option<u64>,
    pub clock: Clock,
}

/// What one `Notify` did: shown (possibly replacing), or counted unshown
/// past the sender's bucket — with its id either way, since the method
/// must answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    Shown {
        id: u32,
        replaced: bool,
    },
    /// Past the rate limit: the id is spent, the card never stored, and
    /// `suppressed` counts it for the summary card.
    Suppressed {
        id: u32,
    },
}

/// A closed card: the id (already invalidated) with its reason, for the
/// `NotificationClosed` signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closure {
    pub id: u32,
    pub reason: CloseReason,
}

/// An invoked action: the signal's payload, plus the close it caused
/// (none for `resident` cards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub id: u32,
    pub action: String,
    pub closed: Option<Closure>,
}

/// The id reservation in `$XDG_RUNTIME_DIR/scootnotify-<display>.ids`:
/// the file holds the next block's start (every id below it reserved),
/// written on open and whenever a block is crossed. Missing, corrupt or
/// unwritable means ephemeral ids from 1 — unique in-run, never
/// persisted — said by [`IdFile::persistent`].
#[derive(Debug)]
pub struct IdFile {
    path: Option<std::path::PathBuf>,
    next: u32,
    end: u64,
}

impl IdFile {
    /// Opens (or creates) the reservation at `path`: reads the base,
    /// reserves the next block. `None` is an ephemeral counter.
    pub fn open(path: Option<std::path::PathBuf>) -> Self {
        let Some(path) = path else {
            return Self {
                path: None,
                next: 1,
                end: u64::from(u32::MAX),
            };
        };
        let base = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            .filter(|base| *base >= 1)
            .unwrap_or(1);
        let file = Self {
            path: Some(path),
            next: base.min(u64::from(u32::MAX)) as u32,
            end: base.saturating_add(ID_BLOCK),
        };
        file.write_end();
        file
    }

    /// Whether reservations persist across a restart.
    pub fn persistent(&self) -> bool {
        self.path.is_some()
    }

    /// The next reserved id: 0 never issues (the spec forbids it).
    pub fn allocate(&mut self) -> u32 {
        if self.next == 0 {
            self.next = 1;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        if u64::from(self.next) >= self.end {
            self.end = self.end.saturating_add(ID_BLOCK);
            self.write_end();
        }
        id
    }

    /// Writes the block end: best effort — a failure degrades to
    /// ephemeral (the in-run counter still never reuses a live id),
    /// never to an error.
    fn write_end(&self) {
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, format!("{}\n", self.end));
        }
    }
}

/// One sender's token bucket: `rate` tokens per second to `burst`.
#[derive(Debug, Clone)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Bucket {
    fn new(now: Instant, burst: u32) -> Self {
        Self {
            tokens: f64::from(burst),
            last: now,
        }
    }

    /// Takes one token: `true` when the card may show.
    fn take(&mut self, now: Instant, rate: u32, burst: u32) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * f64::from(rate)).min(f64::from(burst));
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The live set. Times come from the caller (`now` monotonic for clocks
/// and buckets, `now_ms` realtime for JSON), so tests drive it without
/// sleeping.
#[derive(Debug)]
pub struct Store {
    config: Config,
    live: HashMap<u32, Notification>,
    /// Insertion order (oldest first): the eviction rule walks this.
    order: VecDeque<u32>,
    ids: IdFile,
    buckets: HashMap<String, Bucket>,
    /// Cards counted unshown per sender, for the summary card.
    suppressed: HashMap<String, u64>,
    seq: u64,
}

impl Store {
    /// An empty store over `config`, reserving ids through `ids`.
    pub fn new(config: Config, ids: IdFile) -> Self {
        Self {
            config,
            live: HashMap::new(),
            order: VecDeque::new(),
            ids,
            buckets: HashMap::new(),
            suppressed: HashMap::new(),
            seq: 0,
        }
    }

    /// Live ids, for the no-reuse-across-restart test and the flood row.
    pub fn live_ids(&self) -> Vec<u32> {
        self.live.keys().copied().collect()
    }

    /// Cards counted unshown for `sender` (rate-limited past the
    /// bucket): the summary card reads this.
    pub fn suppressed(&self, sender: &str) -> u64 {
        self.suppressed.get(sender).copied().unwrap_or(0)
    }

    /// One `Notify`: replaced in place, stored fresh, or counted
    /// unshown. Returns the arrival and, when the global cap gave, the
    /// eviction (already invalidated, signal first).
    pub fn notify(
        &mut self,
        incoming: Incoming,
        now: Instant,
        now_ms: u64,
    ) -> (Arrival, Option<Closure>) {
        // A replace for the same unique sender or the same app:
        // swapped in place, expiry restarted, never a new id.
        if incoming.replaces_id != 0
            && let Some(current) = self.live.get(&incoming.replaces_id)
            && (current.sender == incoming.sender || current.app_name == incoming.app_name)
        {
            let id = incoming.replaces_id;
            let timeout = resolve_timeout(
                incoming.expire_timeout,
                incoming.hints.urgency,
                &self.config,
            );
            let icon = icon_of(&incoming);
            let plain = crate::markup::plain(&crate::markup::parse(&incoming.body));
            if let Some(current) = self.live.get_mut(&id) {
                current.app_name = incoming.app_name;
                current.sender = incoming.sender;
                current.icon = icon;
                current.summary = incoming.summary;
                current.body = incoming.body;
                current.plain_body = plain;
                current.actions = incoming.actions;
                current.urgency = incoming.hints.urgency;
                current.category = incoming.hints.category;
                current.desktop_entry = incoming.hints.desktop_entry;
                current.transient = incoming.hints.transient;
                current.resident = incoming.hints.resident;
                current.value = incoming.hints.value;
                current.timeout_ms = timeout;
                current.clock.restart(now, timeout);
            }
            return (Arrival::Shown { id, replaced: true }, None);
        }
        // Fresh: the bucket decides shown or counted.
        if !self.bucket(&incoming.sender, now) {
            let id = self.alloc_id();
            *self.suppressed.entry(incoming.sender).or_insert(0) += 1;
            return (Arrival::Suppressed { id }, None);
        }
        let evicted = self.evict_if_capped();
        let id = self.alloc_id();
        let timeout = resolve_timeout(
            incoming.expire_timeout,
            incoming.hints.urgency,
            &self.config,
        );
        self.seq += 1;
        let icon = icon_of(&incoming);
        let plain = crate::markup::plain(&crate::markup::parse(&incoming.body));
        self.order.push_back(id);
        self.live.insert(
            id,
            Notification {
                id,
                app_name: incoming.app_name,
                sender: incoming.sender,
                icon,
                summary: incoming.summary,
                body: incoming.body,
                plain_body: plain,
                actions: incoming.actions,
                urgency: incoming.hints.urgency,
                category: incoming.hints.category,
                desktop_entry: incoming.hints.desktop_entry,
                transient: incoming.hints.transient,
                resident: incoming.hints.resident,
                value: incoming.hints.value,
                seq: self.seq,
                timeout_ms: timeout,
                created_ms: now_ms,
                shown_ms: None,
                clock: Clock::waiting(),
            },
        );
        (
            Arrival::Shown {
                id,
                replaced: false,
            },
            evicted,
        )
    }

    /// First presentation of `id`: the expiry clock starts. The daemon
    /// calls this at once (headless: arrival is display); the popups
    /// ticket moves it to the frame callback.
    pub fn presented(&mut self, id: u32, now: Instant, now_ms: u64) {
        if let Some(current) = self.live.get_mut(&id) {
            let timeout = current.timeout_ms;
            current.clock.presented(now, timeout);
            if current.shown_ms.is_none() {
                current.shown_ms = Some(now_ms);
            }
        }
    }

    /// Closes `id` for `CloseNotification` from (`caller_sender`,
    /// `caller_app`): the same unique sender or the same app closes with
    /// reason 3; anything else (unknown id, a stranger's id) is unknown,
    /// and the caller gets the empty error. Already invalidated on
    /// return.
    pub fn close(&mut self, id: u32, caller_sender: &str, caller_app: &str) -> Option<Closure> {
        let owned = self.live.get(&id).is_some_and(|current| {
            current.sender == caller_sender || current.app_name == caller_app
        });
        if !owned {
            return None;
        }
        self.remove(id, CloseReason::Closed)
    }

    /// Dismisses `id` (reason 2): the user's click, keybind or
    /// `msg dismiss`. Kept for the UI and control tickets.
    pub fn dismiss(&mut self, id: u32) -> Option<Closure> {
        if !self.live.contains_key(&id) {
            return None;
        }
        self.remove(id, CloseReason::Dismissed)
    }

    /// Invokes `action` on `id`: the `ActionInvoked` payload, closing
    /// with reason 2 unless `resident`. Unknown ids and unknown keys
    /// invoke nothing. Kept for the control ticket.
    pub fn invoke(&mut self, id: u32, action: &str) -> Option<Invocation> {
        let resident = self.live.get(&id)?.resident;
        if !self
            .live
            .get(&id)?
            .actions
            .iter()
            .any(|pair| pair.key == action)
        {
            return None;
        }
        let closed = if resident {
            None
        } else {
            self.remove(id, CloseReason::Dismissed)
        };
        Some(Invocation {
            id,
            action: action.to_owned(),
            closed,
        })
    }

    /// Fires due expiries (reason 1): invalidated before their signals.
    pub fn expired(&mut self, now: Instant) -> Vec<Closure> {
        let due: Vec<u32> = self
            .live
            .iter()
            .filter(|(_, current)| current.clock.due(now))
            .map(|(id, _)| *id)
            .collect();
        due.into_iter()
            .filter_map(|id| self.remove(id, CloseReason::Expired))
            .collect()
    }

    /// The earliest running deadline: the timerfd arms to this, and
    /// disarms when there is none.
    pub fn earliest_deadline(&self) -> Option<Instant> {
        self.live
            .values()
            .filter_map(|current| current.clock.deadline())
            .min()
    }

    /// Pauses every clock (idle began) and resumes them after.
    pub fn pause_all(&mut self, now: Instant) {
        for current in self.live.values_mut() {
            current.clock.pause(now);
        }
    }

    /// Resumes every clock (idle ended).
    pub fn resume_all(&mut self, now: Instant) {
        for current in self.live.values_mut() {
            current.clock.resume(now);
        }
    }

    /// Reads one card: the JSON view and the tests use this.
    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.live.get(&id)
    }

    /// Removes `id` with `reason`, invalidating before the signal.
    fn remove(&mut self, id: u32, reason: CloseReason) -> Option<Closure> {
        self.live.remove(&id)?;
        self.order.retain(|queued| *queued != id);
        Some(Closure { id, reason })
    }

    /// The next id no live card holds: 0 skipped, live skipped.
    fn alloc_id(&mut self) -> u32 {
        loop {
            let id = self.ids.allocate();
            if id != 0 && !self.live.contains_key(&id) {
                return id;
            }
        }
    }

    /// Evicts for the global cap: the oldest non-critical, or the oldest
    /// when only criticals are live.
    fn evict_if_capped(&mut self) -> Option<Closure> {
        if self.live.len() < self.config.max_live as usize {
            return None;
        }
        let victim = self
            .order
            .iter()
            .find(|id| {
                self.live
                    .get(*id)
                    .is_some_and(|current| current.urgency != Urgency::Critical)
            })
            .or_else(|| self.order.front())
            .copied()?;
        self.remove(victim, CloseReason::Evicted)
    }

    /// The sender's bucket say-so, pruning idle senders past the bound.
    fn bucket(&mut self, sender: &str, now: Instant) -> bool {
        if self.buckets.len() > MAX_SENDERS {
            self.buckets.retain(|_, bucket| {
                now.saturating_duration_since(bucket.last) < Duration::from_secs(60)
            });
        }
        let (rate, burst) = (self.config.rate_per_sec, self.config.rate_burst);
        self.buckets
            .entry(sender.to_owned())
            .or_insert_with(|| Bucket::new(now, burst))
            .take(now, rate, burst)
    }
}

/// The icon for an arrival: `image-data` over `image-path` over
/// `app_icon`, the spec's §5.1 order (with the deprecated `image_data`
/// and `image_path` spellings read as their modern twins where they
/// arrive — they share the hint keys here).
fn icon_of(incoming: &Incoming) -> Icon {
    if let Some(image) = &incoming.hints.image_data {
        return Icon::Data(image.clone());
    }
    if !incoming.hints.image_path.is_empty() {
        let icon = classify_icon(&incoming.hints.image_path);
        if icon != Icon::None {
            return icon;
        }
    }
    classify_icon(&incoming.app_icon)
}

#[cfg(test)]
mod tests;
