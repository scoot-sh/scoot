//! The store's contract: ids, replace, close, expiry, caps and rate
//! limits — every conformance row the server half owns.

use std::time::{Duration, Instant};

use crate::bus::hints::{Hints, Urgency};
use crate::bus::notify::Incoming;
use crate::config::Config;

use super::{Arrival, CloseReason, Icon, IdFile, Store, classify_icon};

fn now() -> Instant {
    Instant::now()
}

fn incoming(sender: &str, app: &str) -> Incoming {
    Incoming {
        app_name: app.to_owned(),
        replaces_id: 0,
        app_icon: String::new(),
        summary: "s".to_owned(),
        body: "b".to_owned(),
        actions: Vec::new(),
        hints: Hints::default(),
        expire_timeout: -1,
        sender: sender.to_owned(),
    }
}

fn urgent(sender: &str, app: &str, urgency: Urgency) -> Incoming {
    let mut in_ = incoming(sender, app);
    in_.hints.urgency = urgency;
    in_
}

fn store() -> Store {
    Store::new(Config::default(), IdFile::open(None))
}

fn shown(store: &mut Store, sender: &str, app: &str) -> u32 {
    let start = now();
    let (arrival, _) = store.notify(incoming(sender, app), start, 0);
    let Arrival::Shown { id, replaced } = arrival else {
        panic!("a fresh card shows");
    };
    assert!(!replaced);
    store.presented(id, start, 0);
    id
}

#[test]
fn ids_start_at_one_and_never_reuse_zero_or_live() {
    let mut store = store();
    let first = shown(&mut store, ":1.1", "a");
    assert_eq!(first, 1);
    let second = shown(&mut store, ":1.1", "a");
    assert_eq!(second, 2);
    // Closing frees the number, but the counter never goes back: the
    // next id is fresh, never the freed one.
    store.close(second, ":1.1", "a").unwrap();
    let third = shown(&mut store, ":1.1", "a");
    assert_eq!(third, 3);
}

#[test]
fn replace_swaps_in_place_for_the_same_sender() {
    let mut store = store();
    let id = shown(&mut store, ":1.1", "Mail");
    let mut update = incoming(":1.1", "Mail");
    update.replaces_id = id;
    update.summary = "new".to_owned();
    let (arrival, evicted) = store.notify(update, now(), 0);
    assert_eq!(arrival, Arrival::Shown { id, replaced: true });
    assert_eq!(evicted, None);
    assert_eq!(store.get(id).unwrap().summary, "new");
    // No second card.
    assert_eq!(store.live_ids(), vec![id]);
}

#[test]
fn replace_works_for_the_same_app_from_a_new_connection() {
    let mut store = store();
    // `notify-send -r` reconnects every time: a new unique name, the
    // same app name.
    let id = shown(&mut store, ":1.1", "Volume");
    let mut update = incoming(":1.9", "Volume");
    update.replaces_id = id;
    let (arrival, _) = store.notify(update, now(), 0);
    assert_eq!(arrival, Arrival::Shown { id, replaced: true });
}

#[test]
fn a_strangers_replaces_id_is_a_fresh_card() {
    let mut store = store();
    let id = shown(&mut store, ":1.1", "Mail");
    let mut stranger = incoming(":1.2", "Evil");
    stranger.replaces_id = id;
    let (arrival, _) = store.notify(stranger, now(), 0);
    let Arrival::Shown { id: fresh, replaced } = arrival else {
        panic!("a stranger gets a card");
    };
    assert!(!replaced && fresh != id);
}

#[test]
fn an_unknown_replaces_id_is_a_fresh_card() {
    let mut store = store();
    let mut in_ = incoming(":1.1", "a");
    in_.replaces_id = 999;
    let (arrival, _) = store.notify(in_, now(), 0);
    assert!(matches!(arrival, Arrival::Shown { replaced: false, .. }));
}

#[test]
fn close_needs_the_sender_or_the_app() {
    let mut store = store();
    let id = shown(&mut store, ":1.1", "Mail");
    // A stranger closes nothing.
    assert!(store.close(id, ":1.2", "Evil").is_none());
    assert!(store.get(id).is_some());
    // The same app from a new connection closes.
    let closure = store.close(id, ":1.9", "Mail").unwrap();
    assert_eq!(closure.reason, CloseReason::Closed);
    assert_eq!(closure.reason.code(), 3);
    assert!(store.get(id).is_none());
    // Twice is unknown.
    assert!(store.close(id, ":1.1", "Mail").is_none());
}

#[test]
fn close_of_an_unknown_id_is_unknown() {
    let mut store = store();
    assert!(store.close(4242, ":1.1", "a").is_none());
}

#[test]
fn expiry_fires_from_presentation_with_reason_1() {
    let mut store = store();
    let start = now();
    // Presented at once: due 10 s on (normal default).
    let (arrival, _) = store.notify(incoming(":1.1", "a"), start, 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    // Not presented: the clock never starts, however long passes.
    assert!(store.expired(start + Duration::from_secs(3600)).is_empty());
    store.presented(id, start, 0);
    assert!(store.expired(start + Duration::from_millis(9999)).is_empty());
    let fired = store.expired(start + Duration::from_millis(10_000));
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].reason, CloseReason::Expired);
    assert_eq!(fired[0].reason.code(), 1);
    assert!(store.get(id).is_none());
}

#[test]
fn critical_never_expires_unless_asked() {
    let mut store = store();
    let start = now();
    let (arrival, _) = store.notify(urgent(":1.1", "a", Urgency::Critical), start, 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    store.presented(id, start, 0);
    assert!(store.expired(start + Duration::from_secs(86_400)).is_empty());
    // Asked with a positive timeout, it is honored.
    let mut asked = urgent(":1.1", "b", Urgency::Critical);
    asked.expire_timeout = 100;
    let (arrival, _) = store.notify(asked, start, 0);
    let Arrival::Shown { id: second, .. } = arrival else {
        panic!("shows");
    };
    store.presented(second, start, 0);
    assert_eq!(store.expired(start + Duration::from_millis(100)).len(), 1);
}

#[test]
fn the_live_cap_evicts_oldest_non_critical_first() {
    let mut config = Config::default();
    config.max_live = 3;
    let mut store = Store::new(config, IdFile::open(None));
    let start = now();
    let a = shown(&mut store, ":1.1", "a");
    let _ = shown(&mut store, ":1.1", "b");
    // A critical among them: the cap takes the oldest non-critical.
    let (arrival, _) = store.notify(urgent(":1.1", "c", Urgency::Critical), start, 0);
    let Arrival::Shown { id: critical, .. } = arrival else {
        panic!("shows");
    };
    store.presented(critical, start, 0);
    let (arrival, evicted) = store.notify(incoming(":1.1", "d"), start, 0);
    assert!(matches!(arrival, Arrival::Shown { .. }));
    let evicted = evicted.unwrap();
    assert_eq!(evicted.id, a);
    assert_eq!(evicted.reason, CloseReason::Evicted);
    assert_eq!(evicted.reason.code(), 4);
    assert!(store.get(critical).is_some());
}

#[test]
fn an_all_critical_store_still_evicts_the_oldest() {
    let mut config = Config::default();
    config.max_live = 2;
    let mut store = Store::new(config, IdFile::open(None));
    let start = now();
    let (arrival, _) = store.notify(urgent(":1.1", "a", Urgency::Critical), start, 0);
    let Arrival::Shown { id: first, .. } = arrival else {
        panic!("shows");
    };
    store.presented(first, start, 0);
    let (arrival, _) = store.notify(urgent(":1.1", "b", Urgency::Critical), start, 0);
    let Arrival::Shown { id: second, .. } = arrival else {
        panic!("shows");
    };
    store.presented(second, start, 0);
    let (_, evicted) = store.notify(urgent(":1.1", "c", Urgency::Critical), start, 0);
    assert_eq!(evicted.unwrap().id, first);
}

#[test]
fn the_bucket_answers_but_does_not_show() {
    let mut config = Config::default();
    config.rate_per_sec = 2;
    config.rate_burst = 2;
    let mut store = Store::new(config, IdFile::open(None));
    let start = now();
    for _ in 0..2 {
        let (arrival, _) = store.notify(incoming(":1.7", "Flood"), start, 0);
        assert!(matches!(arrival, Arrival::Shown { .. }));
    }
    let (arrival, _) = store.notify(incoming(":1.7", "Flood"), start, 0);
    let Arrival::Suppressed { id } = arrival else {
        panic!("the third call in the same instant suppresses");
    };
    assert!(store.get(id).is_none());
    assert_eq!(store.suppressed(":1.7"), 1);
    // Another sender is unaffected.
    let (arrival, _) = store.notify(incoming(":1.8", "Calm"), start, 0);
    assert!(matches!(arrival, Arrival::Shown { .. }));
    // And time refills the bucket.
    let (arrival, _) = store.notify(incoming(":1.7", "Flood"), start + Duration::from_secs(2), 0);
    assert!(matches!(arrival, Arrival::Shown { .. }));
}

/// Draining the bucket then refilling it pins both halves: reverting
/// `take` to always true leaves the suppress arm failing, reverting the
/// refill to zero leaves the recovery arm failing.
#[test]
fn invoke_closes_unless_resident() {
    use crate::bus::notify::Action;
    let mut store = store();
    let mut in_ = incoming(":1.1", "a");
    in_.actions = vec![
        Action {
            key: "default".to_owned(),
            label: "Open".to_owned(),
        },
        Action {
            key: "x".to_owned(),
            label: "X".to_owned(),
        },
    ];
    let (arrival, _) = store.notify(in_, now(), 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    let invocation = store.invoke(id, "default").unwrap();
    assert_eq!(invocation.action, "default");
    assert_eq!(invocation.closed.unwrap().reason, CloseReason::Dismissed);
    assert!(store.get(id).is_none());
    // Unknown ids and unknown keys invoke nothing.
    assert!(store.invoke(999, "default").is_none());
    let id = shown(&mut store, ":1.1", "b");
    assert!(store.invoke(id, "nope").is_none());
    assert!(store.dismiss(4242).is_none());
    let closure = store.dismiss(id).unwrap();
    assert_eq!(closure.reason.code(), 2);
}

#[test]
fn a_resident_card_survives_its_invoke() {
    use crate::bus::notify::Action;
    let mut store = store();
    let mut in_ = incoming(":1.1", "a");
    in_.hints.resident = true;
    in_.actions = vec![Action {
        key: "default".to_owned(),
        label: "Open".to_owned(),
    }];
    let (arrival, _) = store.notify(in_, now(), 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    let invocation = store.invoke(id, "default").unwrap();
    assert!(invocation.closed.is_none());
    assert!(store.get(id).is_some());
}

#[test]
fn icon_shapes_classify() {
    assert_eq!(classify_icon(""), Icon::None);
    assert_eq!(classify_icon("/a/b.png"), Icon::Path("/a/b.png".to_owned()));
    assert_eq!(
        classify_icon("file:///a/b%20c.png"),
        Icon::Path("/a/b c.png".to_owned())
    );
    assert_eq!(classify_icon("file:///x"), Icon::Path("/x".to_owned()));
    assert_eq!(classify_icon("http://x/y.png"), Icon::None);
    assert_eq!(classify_icon("file://host/a.png"), Icon::None);
    assert_eq!(classify_icon("mail-unread"), Icon::Name("mail-unread".to_owned()));
    assert_eq!(classify_icon(".hidden"), Icon::None);
    assert_eq!(classify_icon("a/b"), Icon::None);
    assert_eq!(classify_icon(&"n".repeat(129)), Icon::None);
}

#[test]
fn icon_precedence_is_data_over_path_over_name() {
    use crate::bus::image::Image;
    let mut store = store();
    let mut in_ = incoming(":1.1", "a");
    in_.app_icon = "themename".to_owned();
    in_.hints.image_path = "/p.png".to_owned();
    in_.hints.image_data = Some(Image {
        width: 1,
        height: 1,
        pixels: vec![1, 2, 3, 4],
    });
    let (arrival, _) = store.notify(in_, now(), 0);
    let Arrival::Shown { id, .. } = arrival else {
        panic!("shows");
    };
    assert!(matches!(store.get(id).unwrap().icon, Icon::Data(_)));
}

#[test]
fn id_blocks_persist_across_an_open() {
    let dir = std::env::temp_dir().join(format!("scootnotify-ids-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scootnotify-test.ids");
    let mut first = IdFile::open(Some(path.clone()));
    assert!(first.persistent());
    assert_eq!(first.allocate(), 1);
    assert_eq!(first.allocate(), 2);
    // A second open (the restart) continues past the block, never at 1.
    let mut second = IdFile::open(Some(path.clone()));
    assert_eq!(second.allocate(), 1025);
    // A corrupt file degrades to 1, never a panic or a refusal.
    std::fs::write(&path, "garbage\n").unwrap();
    assert_eq!(IdFile::open(Some(path)).allocate(), 1);
    // No directory at all is ephemeral but functional.
    let mut ephemeral = IdFile::open(None);
    assert!(!ephemeral.persistent());
    assert_eq!(ephemeral.allocate(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
