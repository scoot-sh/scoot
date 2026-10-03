//! Test builders for MPRIS bodies: what a player marshals, written with
//! this crate's [`Writer`]. (`../fixtures/` holds the bodies a different
//! marshaller wrote, so the reader is not tested only against the writer
//! it agrees with.)

use super::PLAYER;
use crate::dbus::proto::Writer;

/// One `a{sv}` entry: the key, the variant's signature, then the value
/// `write` makes.
pub fn entry(body: &mut Writer, key: &str, sig: &str, write: &dyn Fn(&mut Writer)) {
    body.open_struct();
    body.str(key);
    body.variant(sig);
    write(body);
    body.close_struct();
}

/// The `Metadata` value (`a{sv}`) of a track: the title and artists, with
/// the entries a real player sends beside them (a track id, a length, an
/// art URL, an album) for the walk to skip.
pub fn metadata(body: &mut Writer, title: Option<&str>, artists: &[&str]) {
    let Some(cookie) = body.open_array(8) else {
        return;
    };
    entry(body, "mpris:trackid", "o", &|w| w.str("/org/mpris/track/1"));
    entry(body, "mpris:length", "x", &|w| w.u64(215_000_000));
    entry(body, "mpris:artUrl", "s", &|w| {
        w.str("file:///tmp/cover.jpg")
    });
    entry(body, "xesam:album", "s", &|w| w.str("An Album"));
    if let Some(title) = title {
        entry(body, "xesam:title", "s", &|w| w.str(title));
    }
    if !artists.is_empty() {
        entry(body, "xesam:artist", "as", &|w| {
            if let Some(cookie) = w.open_array(4) {
                for artist in artists {
                    w.str(artist);
                }
                w.close_array(cookie);
            }
        });
    }
    body.close_array(cookie);
}

/// The properties a player holds, written into an open `a{sv}`: the ones
/// the bar reads, and the ones it must skip.
pub fn state(body: &mut Writer, status: &str, title: Option<&str>, artists: &[&str]) {
    entry(body, "PlaybackStatus", "s", &|w| w.str(status));
    entry(body, "Metadata", "a{sv}", &|w| metadata(w, title, artists));
    entry(body, "Position", "x", &|w| w.u64(42_000_000));
    entry(body, "Volume", "d", &|w| w.u64(0x3ff0_0000_0000_0000));
    entry(body, "Rate", "d", &|w| w.u64(0x3ff0_0000_0000_0000));
    entry(body, "CanGoNext", "b", &|w| w.boolean(true));
    entry(body, "CanGoPrevious", "b", &|w| w.boolean(true));
    entry(body, "CanControl", "b", &|w| w.boolean(true));
}

/// A `GetAll` answer body for a player in `status` on a track.
pub fn get_all(status: &str, title: Option<&str>, artists: &[&str]) -> Vec<u8> {
    get_all_with(|body| state(body, status, title, artists))
}

/// A `GetAll` answer body whose entries `entries` writes.
pub fn get_all_with(entries: impl Fn(&mut Writer)) -> Vec<u8> {
    let mut body = Writer::new();
    if let Some(cookie) = body.open_array(8) {
        entries(&mut body);
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// A `PropertiesChanged` body for `interface`: `entries` fills the changed
/// dictionary, `invalidated` names the properties that changed with no
/// value.
pub fn changed(interface: &str, entries: impl Fn(&mut Writer), invalidated: &[&str]) -> Vec<u8> {
    let mut body = Writer::new();
    body.str(interface);
    if let Some(cookie) = body.open_array(8) {
        entries(&mut body);
        body.close_array(cookie);
    }
    if let Some(cookie) = body.open_array(4) {
        for name in invalidated {
            body.str(name);
        }
        body.close_array(cookie);
    }
    body.take_body().unwrap_or_default()
}

/// A `PropertiesChanged` body for the Player interface carrying only a
/// status.
pub fn status_changed(status: &str) -> Vec<u8> {
    changed(
        PLAYER,
        |body| entry(body, "PlaybackStatus", "s", &|w| w.str(status)),
        &[],
    )
}
