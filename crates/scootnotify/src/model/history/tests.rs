//! The history ring's contract: caps, `transient` exclusion, and the
//! unread count.

use crate::bus::hints::Urgency;
use crate::bus::image::Image;
use crate::bus::notify::Incoming;
use crate::config::Config;
use crate::model::store::{IdFile, Store};

use super::{History, HistoryEntry};

fn incoming(sender: &str, app: &str) -> Incoming {
    Incoming {
        app_name: app.to_owned(),
        replaces_id: 0,
        app_icon: String::new(),
        summary: "s".to_owned(),
        body: "b".to_owned(),
        actions: Vec::new(),
        hints: crate::bus::hints::Hints::default(),
        expire_timeout: -1,
        sender: sender.to_owned(),
    }
}

fn entry(seen: bool) -> HistoryEntry {
    HistoryEntry {
        app_name: "a".to_owned(),
        summary: "s".to_owned(),
        body: "b".to_owned(),
        urgency: Urgency::Normal,
        category: String::new(),
        desktop_entry: String::new(),
        icon: None,
        created_ms: 0,
        seen,
    }
}

#[test]
fn the_ring_caps_at_history_max_keeping_the_newest() {
    let mut history = History::default();
    for _ in 0..5 {
        history.push(entry(true), 3);
    }
    assert_eq!(history.len(), 3);
    // A zero cap keeps nothing.
    history.push(entry(true), 0);
    assert_eq!(history.len(), 3);
    // Truncating to a smaller reload cap drops the oldest only.
    history.truncate(2);
    assert_eq!(history.len(), 2);
    history.truncate(0);
    assert!(history.is_empty());
}

#[test]
fn unread_counts_only_the_unseen() {
    let mut history = History::default();
    assert_eq!(history.unread(), 0);
    history.push(entry(true), 64);
    history.push(entry(false), 64);
    history.push(entry(false), 64);
    assert_eq!(history.unread(), 2);
    history.mark_newest_seen();
    assert_eq!(history.unread(), 1);
    assert!(history.newest().is_some_and(|entry| entry.seen));
}

#[test]
fn pixels_never_cross_into_history() {
    let mut store = Store::new(Config::default(), IdFile::open(None));
    let mut in_ = incoming(":1.1", "a");
    in_.hints.image_data = Some(Image {
        width: 1,
        height: 1,
        pixels: vec![1, 2, 3, 4],
    });
    let start = std::time::Instant::now();
    let (arrival, _) = store.notify(in_, start, 0);
    let crate::model::store::Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    store.presented(id, start, 0);
    // Dismiss it so it enters history as a seen card.
    store.dismiss(id);
    let newest = store.history().newest().expect("a dismissal enters history");
    assert!(newest.seen);
    assert!(newest.icon.is_none());
}

#[test]
fn transient_never_enters_history() {
    use crate::model::store::Arrival;
    let mut store = Store::new(Config::default(), IdFile::open(None));
    let start = std::time::Instant::now();
    let mut in_ = incoming(":1.1", "a");
    in_.hints.transient = true;
    let (arrival, _) = store.notify(in_, start, 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    store.presented(id, start, 0);
    store.dismiss(id);
    assert!(store.history().is_empty());
}

#[test]
fn names_and_paths_cross_as_strings() {
    use crate::model::store::Arrival;
    let mut store = Store::new(Config::default(), IdFile::open(None));
    let start = std::time::Instant::now();
    let mut in_ = incoming(":1.1", "a");
    in_.app_icon = "mail-unread".to_owned();
    let (arrival, _) = store.notify(in_, start, 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    store.presented(id, start, 0);
    store.dismiss(id);
    let newest = store.history().newest().expect("enters history");
    assert!(matches!(newest.icon, Some(super::IconName::Name(_))));
    let mut pathed = incoming(":1.1", "b");
    pathed.hints.image_path = "/p.png".to_owned();
    let (arrival, _) = store.notify(pathed, start, 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    store.presented(id, start, 0);
    store.dismiss(id);
    let newest = store.history().newest().expect("enters history");
    assert!(matches!(newest.icon, Some(super::IconName::Path(_))));
}
