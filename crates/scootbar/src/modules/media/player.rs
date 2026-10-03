//! One player: what it shows, parsed from its properties, and which of
//! several is the one to show.
//!
//! Everything a player says is untrusted bytes from a same-user peer:
//! strings are cleaned once, at parse time (control characters stripped,
//! cut at [`MAX_FIELD`] bytes on a character boundary) into buffers the
//! player keeps, so a track change allocates nothing past the first.

use std::time::Instant;

use crate::dbus::mpris::{NAME_PREFIX, Props};

/// A cleaned title or artist is at most this many bytes: the view bounds
/// the line it makes of them again ([`crate::modules::MAX_TEXT`]).
pub(super) const MAX_FIELD: usize = 120;

/// What a player is doing. Anything the player sends that is not
/// `Playing` or `Paused` is stopped, so a state this client does not know
/// shows nothing rather than something wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Stopped,
    Paused,
    Playing,
}

impl Status {
    pub(super) fn parse(text: &str) -> Self {
        match text {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        }
    }

    /// The word `query` reports.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Paused => "paused",
            Self::Playing => "playing",
        }
    }
}

/// One player on the bus.
pub(super) struct Player {
    /// Unique among the players the module has held (a name that leaves
    /// and returns is a new one): what a reply in flight is matched to, so
    /// an answer meant for the old owner never lands on the new.
    pub(super) id: u64,
    /// The well-known name (`org.mpris.MediaPlayer2.mpv`).
    pub(super) name: String,
    /// Its owner's unique name: signals come from it, calls go to it.
    pub(super) owner: String,
    pub(super) status: Status,
    pub(super) title: String,
    pub(super) artist: String,
    /// Until a player says otherwise it may be asked: a refusal is for a
    /// player that said it cannot.
    pub(super) can_go_next: bool,
    pub(super) can_go_previous: bool,
    pub(super) can_control: bool,
    /// When it last started or stopped playing: the clock's tick (0 for a
    /// player that never has), so "most recently playing" is an order and
    /// not a timestamp two events can share.
    pub(super) active: u64,
    /// Bumped when what is shown of it changes.
    pub(super) rev: u64,
    /// When its `GetAll` went out, while one is in flight.
    pub(super) asked: Option<Instant>,
    /// When the last `GetAll` went out: reads are no closer than
    /// [`super::MIN_REFRESH_GAP`].
    pub(super) last_asked: Option<Instant>,
    /// A signal arrived while one was in flight: read again when it
    /// answers.
    pub(super) stale: bool,
}

impl Player {
    pub(super) fn new(id: u64, name: String, owner: String) -> Self {
        Self {
            id,
            name,
            owner,
            status: Status::Stopped,
            title: String::new(),
            artist: String::new(),
            can_go_next: true,
            can_go_previous: true,
            can_control: true,
            active: 0,
            rev: 0,
            asked: None,
            last_asked: None,
            stale: false,
        }
    }

    /// The name shown and matched by: `mpv` for
    /// `org.mpris.MediaPlayer2.mpv` and for
    /// `org.mpris.MediaPlayer2.mpv.instance1234`.
    pub(super) fn short(&self) -> &str {
        short_name(&self.name)
    }

    /// Applies properties (a `GetAll`'s, or a signal's): says whether
    /// anything shown moved. `scratch` is the module's one reused buffer,
    /// so an unchanged title costs no allocation and a changed one reuses
    /// the old title's.
    pub(super) fn apply(
        &mut self,
        props: &Props<'_>,
        scratch: &mut String,
        tick: &mut u64,
    ) -> bool {
        let mut changed = false;
        if let Some(status) = props.status {
            let status = Status::parse(status);
            if status != self.status {
                // Starting or ceasing to play is activity: what "the one
                // that played last" ranks by. A player found paused has
                // played nothing, so it ties with the other such ones and
                // the name decides (discovery order would not be stable).
                if status == Status::Playing || self.status == Status::Playing {
                    *tick += 1;
                    self.active = *tick;
                }
                self.status = status;
                changed = true;
            }
        }
        if let Some(metadata) = &props.metadata {
            clean_into(scratch, metadata.title.unwrap_or(""));
            if *scratch != self.title {
                std::mem::swap(scratch, &mut self.title);
                changed = true;
            }
            scratch.clear();
            for artist in &metadata.artists {
                let before = scratch.len();
                if before > 0 {
                    if before + 2 >= MAX_FIELD {
                        break;
                    }
                    scratch.push_str(", ");
                }
                let start = scratch.len();
                push_clean(scratch, artist, MAX_FIELD - start);
                if scratch.len() == start {
                    // An artist that cleaned to nothing leaves no comma.
                    scratch.truncate(before);
                }
            }
            if *scratch != self.artist {
                std::mem::swap(scratch, &mut self.artist);
                changed = true;
            }
            scratch.clear();
        }
        if let Some(can) = props.can_go_next {
            self.can_go_next = can;
        }
        if let Some(can) = props.can_go_previous {
            self.can_go_previous = can;
        }
        if let Some(can) = props.can_control {
            self.can_control = can;
        }
        if changed {
            self.rev = self.rev.wrapping_add(1);
        }
        changed
    }

    /// Forgets the track and the state: a new process took over the name,
    /// and what the old one said is not its.
    pub(super) fn reset(&mut self) {
        self.status = Status::Stopped;
        self.title.clear();
        self.artist.clear();
        self.can_go_next = true;
        self.can_go_previous = true;
        self.can_control = true;
        self.rev = self.rev.wrapping_add(1);
    }
}

/// `text` into `out` (cleared first), cleaned.
fn clean_into(out: &mut String, text: &str) {
    out.clear();
    push_clean(out, text, MAX_FIELD);
}

/// Appends `text` without control characters, at most `room` bytes, cut on
/// a character boundary. Reads no further than it keeps (and the
/// controls it drops), so a megabyte title costs a bounded walk, not a
/// megabyte of copy.
fn push_clean(out: &mut String, text: &str, room: usize) {
    let end = out.len() + room;
    for c in text.chars() {
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > end {
            break;
        }
        out.push(c);
    }
}

/// `mpv` for `org.mpris.MediaPlayer2.mpv.instance1234`, `firefox` for
/// `org.mpris.MediaPlayer2.firefox.instance_1_87` and `mpv` for
/// `org.mpris.MediaPlayer2.mpv.instance-PTHiuoaF` (what the mpv script
/// registers for a second copy).
pub(super) fn short_name(name: &str) -> &str {
    let rest = name.strip_prefix(NAME_PREFIX).unwrap_or(name);
    // A second copy of a player names itself `NAME.instance` and a suffix
    // of its own making: a pid, or digits and underscores, or a dash and
    // something. The last name element is that when it starts with
    // `instance` and what follows is empty or starts with a digit, `_` or
    // `-` (so `instanceof` is a name, not an instance).
    match rest.rsplit_once('.') {
        Some((short, last))
            if !short.is_empty()
                && last.strip_prefix("instance").is_some_and(|suffix| {
                    suffix
                        .bytes()
                        .next()
                        .is_none_or(|b| b.is_ascii_digit() || b == b'_' || b == b'-')
                }) =>
        {
            short
        }
        _ => rest,
    }
}

/// Whether `preferred` (the config's `player`) names `player`: its short
/// name, exactly.
pub(super) fn is_preferred(player: &Player, preferred: Option<&str>) -> bool {
    preferred.is_some_and(|wanted| player.short() == wanted)
}

/// The player to show: not stopped (a stopped player shows nothing); the
/// one the config prefers if it is among them; else the one playing, the
/// most recently started of several; else the paused one that played most
/// recently; ties (never played, found together) by name, so the answer
/// is the same on every run. `None` shows nothing.
pub(super) fn select(players: &[Player], preferred: Option<&str>) -> Option<usize> {
    let key = |player: &Player| {
        (
            is_preferred(player, preferred),
            player.status == Status::Playing,
            player.active,
        )
    };
    players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.status != Status::Stopped)
        .max_by(|(_, a), (_, b)| {
            key(a)
                .cmp(&key(b))
                // The smaller name wins a tie, so reverse it for `max_by`.
                .then_with(|| b.name.cmp(&a.name))
        })
        .map(|(index, _)| index)
}
