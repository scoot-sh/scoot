//! MPRIS wire shapes: what the media module reads of a player's
//! `org.mpris.MediaPlayer2.Player` interface, over [`super::proto`]'s
//! readers. Like `proto`, it uses nothing but `std` and is compiled by the
//! fuzz crate (`crates/scootbar/fuzz`) unchanged.
//!
//! The same typed walk reads both shapes a player sends its state in: the
//! `a{sv}` a `GetAll` answers with, and the `sa{sv}as` body of a
//! `PropertiesChanged` signal. Every property the bar does not show (the
//! position, the volume, the rate, the track id, the art URL, anything a
//! later MPRIS adds) is skipped by its signature, never parsed into a
//! value: the position changes with no signal and a chatty player may
//! send it constantly, so it must cost the walk nothing but a skip, and
//! the art URL is a string the bar does not fetch.
//!
//! Strings come back borrowed and unbounded (a player can send a megabyte
//! as a title, inside the 1 MiB message the client takes); the consumer
//! cuts them where it stores them. Counts are bounded here: properties,
//! metadata entries and invalidated names past their limits refuse the whole
//! answer, so a dictionary of a million one-byte keys is never walked; an
//! artist list is walked in full (its length is bounded by the 1 MiB
//! message) and only the first [`MAX_ARTISTS`] are kept.
//! A property of the wrong type for its name (a `PlaybackStatus` that is
//! an integer) is skipped like an unknown one: the player loses that
//! property, not the whole answer. A misshapen body is refused whole
//! (`Err(())`) and the caller keeps the player's last state.

use super::proto::{self, Reader};

#[cfg(test)]
pub(crate) mod build;
#[cfg(test)]
mod tests;

/// The Player interface, where the state lives.
pub const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// The prefix of a player's well-known bus name.
pub const NAME_PREFIX: &str = "org.mpris.MediaPlayer2.";
/// The one object every player serves.
pub const PATH: &str = "/org/mpris/MediaPlayer2";

/// The most `Metadata` entries walked: a real player sends ten.
pub const MAX_METADATA: usize = 128;
/// The most artists kept of `xesam:artist`: the rest are left out.
pub const MAX_ARTISTS: usize = 16;
/// The most names read from a signal's invalidated list.
pub const MAX_INVALIDATED: usize = 128;

/// What the bar takes from a player's properties, borrowing the body. Each
/// field is `None` when the player did not send it (or sent it with a type
/// MPRIS does not give it).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Props<'a> {
    /// `Playing`, `Paused` or `Stopped` (anything else is the consumer's
    /// to treat as stopped).
    pub status: Option<&'a str>,
    /// The whole `Metadata` dictionary, as far as the bar reads it. `Some`
    /// with nothing in it is a player saying it has no track.
    pub metadata: Option<Metadata<'a>>,
    pub can_go_next: Option<bool>,
    pub can_go_previous: Option<bool>,
    pub can_control: Option<bool>,
}

/// The track: the two strings the bar shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Metadata<'a> {
    pub title: Option<&'a str>,
    /// `xesam:artist`, which the spec makes a list of strings; a player
    /// that sends one string is read as a list of one. At most
    /// [`MAX_ARTISTS`].
    pub artists: Vec<&'a str>,
}

/// A `PropertiesChanged` signal's body.
#[derive(Debug, PartialEq, Eq)]
pub struct Changed<'a> {
    pub interface: &'a str,
    /// The values the signal carried: empty unless the interface is
    /// [`PLAYER`].
    pub props: Props<'a>,
    /// A property the bar holds was invalidated: the signal said it
    /// changed without saying to what, so the bar must ask.
    pub invalidated: bool,
}

/// Reads a `GetAll` body (`a{sv}`). `Err(())` refuses the whole answer.
pub fn read_player_props(body: &[u8]) -> Result<Props<'_>, ()> {
    let mut reader = Reader::le(body);
    let mut entries = reader.elements(8)?;
    let props = walk(&mut entries)?;
    if !reader.exhausted() {
        return Err(());
    }
    Ok(props)
}

/// Reads a `PropertiesChanged` body (`sa{sv}as`). A signal for another
/// interface is answered after its first string: its dictionary is not the
/// bar's to walk.
pub fn read_properties_changed(body: &[u8]) -> Result<Changed<'_>, ()> {
    let mut reader = Reader::le(body);
    let interface = reader.str()?;
    if interface != PLAYER {
        return Ok(Changed {
            interface,
            props: Props::default(),
            invalidated: false,
        });
    }
    let mut entries = reader.elements(8)?;
    let props = walk(&mut entries)?;
    let mut names = reader.elements(4)?;
    let mut invalidated = false;
    let mut seen = 0;
    while !names.exhausted() {
        seen += 1;
        if seen > MAX_INVALIDATED {
            return Err(());
        }
        invalidated |= matches!(
            names.str()?,
            "PlaybackStatus" | "Metadata" | "CanGoNext" | "CanGoPrevious" | "CanControl"
        );
    }
    if !reader.exhausted() {
        return Err(());
    }
    Ok(Changed {
        interface,
        props,
        invalidated,
    })
}

/// One property dictionary's entries, to its end: the properties the bar
/// shows typed, every other one (and one of the wrong type) skipped by its
/// signature, so a property a later MPRIS adds cannot break the walk.
fn walk<'a>(entries: &mut Reader<'a>) -> Result<Props<'a>, ()> {
    let mut props = Props::default();
    let mut seen = 0;
    while !entries.exhausted() {
        seen += 1;
        if seen > proto::MAX_PROPERTIES {
            return Err(());
        }
        entries.enter_struct()?;
        let key = entries.str()?;
        let sig = entries.signature()?;
        match (key, sig) {
            ("PlaybackStatus", "s") => props.status = Some(entries.str()?),
            ("Metadata", "a{sv}") => props.metadata = Some(read_metadata(entries)?),
            ("CanGoNext", "b") => props.can_go_next = Some(entries.boolean()?),
            ("CanGoPrevious", "b") => props.can_go_previous = Some(entries.boolean()?),
            ("CanControl", "b") => props.can_control = Some(entries.boolean()?),
            _ => entries.skip(sig)?,
        }
        entries.leave_struct();
    }
    Ok(props)
}

/// The `Metadata` dictionary: the title and the artists, the rest skipped
/// (`mpris:artUrl` among it: the bar never fetches a URL a player names).
fn read_metadata<'a>(entries: &mut Reader<'a>) -> Result<Metadata<'a>, ()> {
    let mut dict = entries.elements(8)?;
    let mut metadata = Metadata::default();
    let mut seen = 0;
    while !dict.exhausted() {
        seen += 1;
        if seen > MAX_METADATA {
            return Err(());
        }
        dict.enter_struct()?;
        let key = dict.str()?;
        let sig = dict.signature()?;
        match (key, sig) {
            ("xesam:title", "s") => metadata.title = Some(dict.str()?),
            ("xesam:artist", "as") => {
                let mut list = dict.elements(4)?;
                metadata.artists.clear();
                while !list.exhausted() {
                    let artist = list.str()?;
                    if metadata.artists.len() < MAX_ARTISTS {
                        metadata.artists.push(artist);
                    }
                }
            }
            ("xesam:artist", "s") => {
                metadata.artists.clear();
                metadata.artists.push(dict.str()?);
            }
            _ => dict.skip(sig)?,
        }
        dict.leave_struct();
    }
    Ok(metadata)
}
