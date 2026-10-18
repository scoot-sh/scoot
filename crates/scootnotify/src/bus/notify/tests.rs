//! The `Notify` contract: the eight arguments, every cap, and hostile
//! shapes refused as `InvalidArgs`.

use scoot_dbus::proto::Reader;

use super::super::testutils::{Val, notify_bytes};
use super::{
    MAX_ACTION_LABEL, MAX_ACTION_PAIRS, MAX_APP_ICON, MAX_APP_NAME, MAX_BODY, MAX_SUMMARY,
    NOTIFY_SIGNATURE, drop_controls, fuzz_check, parse,
};

fn parse_of(bytes: &[u8]) -> super::Incoming {
    let mut reader = Reader::le(bytes);
    parse(&mut reader, ":1.77").unwrap()
}

#[test]
fn all_eight_arguments_land() {
    let incoming = parse_of(&notify_bytes(
        "Mail",
        42,
        "mail-unread",
        "Hi",
        "a <b>body</b>",
        &["default", "Open", "x", "Y"],
        &[("value", Val::I32(3))],
        -1,
    ));
    assert_eq!(incoming.app_name, "Mail");
    assert_eq!(incoming.replaces_id, 42);
    assert_eq!(incoming.app_icon, "mail-unread");
    assert_eq!(incoming.summary, "Hi");
    assert_eq!(incoming.body, "a <b>body</b>");
    assert_eq!(incoming.actions.len(), 2);
    assert_eq!(incoming.actions[0].key, "default");
    assert_eq!(incoming.actions[0].label, "Open");
    assert_eq!(incoming.hints.value, Some(3));
    assert_eq!(incoming.expire_timeout, -1);
    assert_eq!(incoming.sender, ":1.77");
}

#[test]
fn the_signature_is_the_specs() {
    assert_eq!(NOTIFY_SIGNATURE, "susssasa{sv}i");
}

#[test]
fn fields_cut_never_refuse() {
    let long = "w".repeat(20_000);
    let actions: Vec<&str> = (0..40).map(|_| "a").collect();
    let incoming = parse_of(&notify_bytes(&long, 0, &long, &long, &long, &actions, &[], 0));
    assert_eq!(incoming.app_name.len(), MAX_APP_NAME);
    assert_eq!(incoming.app_icon.len(), MAX_APP_ICON);
    assert_eq!(incoming.summary.len(), MAX_SUMMARY);
    assert_eq!(incoming.body.len(), MAX_BODY);
    assert_eq!(incoming.actions.len(), MAX_ACTION_PAIRS);
    // Cut at a char boundary: a multibyte tail never splits.
    let wide = "é".repeat(9000);
    let incoming = parse_of(&notify_bytes("a", 0, "", "", &wide, &[], &[], 0));
    assert!(incoming.body.len() <= MAX_BODY);
    assert!(core::str::from_utf8(incoming.body.as_bytes()).is_ok());
}

#[test]
fn control_characters_drop() {
    let incoming = parse_of(&notify_bytes("a\x07b", 0, "", "x\x1by", "p\x7fq", &[], &[], 0));
    assert_eq!(incoming.app_name, "ab");
    assert_eq!(incoming.summary, "xy");
    assert_eq!(incoming.body, "pq");
    assert_eq!(drop_controls("a\tb\nc"), "a\tb\nc");
}

#[test]
fn an_odd_action_string_is_ignored() {
    let incoming = parse_of(&notify_bytes("a", 0, "", "", "", &["only"], &[], 0));
    assert!(incoming.actions.is_empty());
}

#[test]
fn misshapen_bodies_are_invalid_args() {
    let good = notify_bytes("a", 0, "", "", "", &[], &[], 0);
    // Truncated, trailing garbage, and an empty body.
    for bytes in [&good[..good.len() - 1], b"junk", b""] {
        let mut reader = Reader::le(bytes);
        assert!(parse(&mut reader, ":1.1").is_err());
    }
    // Trailing bytes past the eight arguments.
    let mut trailing = good.clone();
    trailing.push(0);
    let mut reader = Reader::le(&trailing);
    assert!(parse(&mut reader, ":1.1").is_err());
}

/// Reverting the action cap to unbounded leaves the fields test failing
/// at 20 pairs: the bound is load-bearing.
#[test]
fn long_keys_and_labels_cut() {
    let key = "k".repeat(100);
    let label = "l".repeat(300);
    let incoming = parse_of(&notify_bytes("a", 0, "", "", "", &[&key, &label], &[], 0));
    assert_eq!(incoming.actions[0].key.len(), super::MAX_ACTION_KEY);
    assert_eq!(incoming.actions[0].label.len(), MAX_ACTION_LABEL);
}

#[test]
fn the_check_spots_a_seed() {
    let bytes = notify_bytes("a", 0, "", "s", "b", &[], &[], -1);
    assert!(fuzz_check(&bytes));
    assert!(!fuzz_check(b"\x00\x01"));
}

#[test]
fn the_committed_corpus_replays_on_stable() {
    crate::replay_corpus("notify", true, fuzz_check);
}
