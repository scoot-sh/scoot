//! The MPRIS readers: real shapes read right, hostile and wrong ones
//! refused or skipped, and nothing panics on any prefix or mutation of a
//! valid body.

use super::build::{changed, entry, get_all, get_all_with, metadata, state, status_changed};
use super::*;
use crate::dbus::proto::Writer;

#[test]
fn a_real_get_all_is_read_and_what_the_bar_does_not_show_is_skipped() {
    let body = get_all("Playing", Some("Song"), &["Ada", "Bo"]);
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.status, Some("Playing"));
    assert_eq!(props.can_go_next, Some(true));
    assert_eq!(props.can_go_previous, Some(true));
    assert_eq!(props.can_control, Some(true));
    let track = props.metadata.unwrap();
    assert_eq!(track.title, Some("Song"));
    assert_eq!(track.artists, ["Ada", "Bo"]);
}

#[test]
fn a_player_with_no_track_says_so_with_an_empty_dictionary() {
    let body = get_all_with(|body| {
        entry(body, "PlaybackStatus", "s", &|w| w.str("Stopped"));
        entry(body, "Metadata", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                w.close_array(cookie);
            }
        });
    });
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.metadata, Some(Metadata::default()));
    assert_eq!(props.status, Some("Stopped"));
    // And one that sent no metadata at all says nothing about it.
    let body = get_all_with(|body| entry(body, "PlaybackStatus", "s", &|w| w.str("Paused")));
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.metadata, None);
    assert_eq!(props.can_go_next, None);
}

#[test]
fn an_artist_sent_as_one_string_is_a_list_of_one() {
    let body = get_all_with(|body| {
        entry(body, "Metadata", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                entry(w, "xesam:artist", "s", &|w| w.str("Solo"));
                w.close_array(cookie);
            }
        });
    });
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.metadata.unwrap().artists, ["Solo"]);
}

#[test]
fn a_property_of_the_wrong_type_loses_itself_and_nothing_else() {
    let body = get_all_with(|body| {
        // The status as an integer, the title as an integer, the artist as
        // an integer list, a flag as a string: each skipped.
        entry(body, "PlaybackStatus", "i", &|w| w.i32(7));
        entry(body, "CanGoNext", "s", &|w| w.str("yes"));
        entry(body, "Metadata", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                entry(w, "xesam:title", "i", &|w| w.i32(9));
                entry(w, "xesam:artist", "ai", &|w| {
                    if let Some(cookie) = w.open_array(4) {
                        w.i32(1);
                        w.close_array(cookie);
                    }
                });
                entry(w, "xesam:album", "s", &|w| w.str("kept out"));
                w.close_array(cookie);
            }
        });
        entry(body, "CanControl", "b", &|w| w.boolean(false));
    });
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.status, None);
    assert_eq!(props.can_go_next, None);
    assert_eq!(props.can_control, Some(false));
    assert_eq!(props.metadata, Some(Metadata::default()));
}

#[test]
fn unknown_and_nested_properties_are_walked_past() {
    let body = get_all_with(|body| {
        entry(body, "Zeta", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                entry(w, "Deep", "(ytd)", &|w| {
                    w.open_struct();
                    w.u8(1);
                    w.u64(2);
                    w.u64(3);
                    w.close_struct();
                });
                w.close_array(cookie);
            }
        });
        entry(body, "Future", "a(xx)", &|w| {
            if let Some(cookie) = w.open_array(8) {
                w.open_struct();
                w.u64(1);
                w.u64(2);
                w.close_struct();
                w.close_array(cookie);
            }
        });
        entry(body, "PlaybackStatus", "s", &|w| w.str("Paused"));
    });
    assert_eq!(
        read_player_props(&body).unwrap().status,
        Some("Paused"),
        "a property after the unknown ones is still found"
    );
}

#[test]
fn counts_are_bounded() {
    // Too many properties: refused whole.
    let body = get_all_with(|body| {
        for i in 0..=proto::MAX_PROPERTIES {
            entry(body, &format!("P{i}"), "b", &|w| w.boolean(true));
        }
    });
    assert!(read_player_props(&body).is_err());
    let body = get_all_with(|body| {
        for i in 0..proto::MAX_PROPERTIES {
            entry(body, &format!("P{i}"), "b", &|w| w.boolean(true));
        }
    });
    assert!(read_player_props(&body).is_ok());
    // Too many metadata entries: refused whole.
    let body = get_all_with(|body| {
        entry(body, "Metadata", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                for i in 0..=MAX_METADATA {
                    entry(w, &format!("k{i}"), "b", &|w| w.boolean(false));
                }
                w.close_array(cookie);
            }
        });
    });
    assert!(read_player_props(&body).is_err());
    // Too many artists: the first ones kept, the rest left out.
    let many: Vec<String> = (0..MAX_ARTISTS + 9).map(|i| format!("a{i}")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let body = get_all("Playing", Some("t"), &refs);
    let artists = read_player_props(&body).unwrap().metadata.unwrap().artists;
    assert_eq!(artists.len(), MAX_ARTISTS);
    assert_eq!(artists[0], "a0");
}

#[test]
fn a_huge_title_is_borrowed_whole_for_the_consumer_to_cut() {
    let huge = "ab".repeat(400_000);
    let body = get_all("Playing", Some(&huge), &[]);
    let props = read_player_props(&body).unwrap();
    assert_eq!(props.metadata.unwrap().title.map(str::len), Some(800_000));
}

#[test]
fn a_misshapen_body_is_refused_never_a_panic() {
    let good = get_all("Playing", Some("Song"), &["Ada"]);
    // Every prefix: the length word promises more than there is.
    for end in 0..good.len() {
        let _ = read_player_props(&good[..end]);
    }
    assert!(read_player_props(&good[..good.len() - 1]).is_err());
    assert!(read_player_props(&[]).is_err());
    // Trailing bytes after the dictionary: refused.
    let mut trailing = good.clone();
    trailing.extend_from_slice(&[0, 0, 0, 0]);
    assert!(read_player_props(&trailing).is_err());
    // A string that is not UTF-8 refuses the answer: the player keeps its
    // last state (a real bus would not deliver one: daemons validate).
    let mut bad = get_all("Playing", Some("Song"), &[]);
    let at = bad.windows(4).position(|w| w == b"Song").unwrap();
    bad[at] = 0xff;
    assert!(read_player_props(&bad).is_err());
    // A boolean that is neither 0 nor 1.
    let mut flag = get_all_with(|body| entry(body, "CanControl", "b", &|w| w.boolean(true)));
    let last = flag.len() - 4;
    flag[last] = 7;
    assert!(read_player_props(&flag).is_err());
}

#[test]
fn every_single_byte_mutation_of_a_valid_body_is_a_refusal_or_a_read() {
    let good = get_all("Playing", Some("Song"), &["Ada", "Bo"]);
    for at in 0..good.len() {
        for value in [0x00, 0x01, 0x7f, 0x80, 0xff] {
            let mut body = good.clone();
            body[at] = value;
            let _ = read_player_props(&body);
            let signal = changed(
                PLAYER,
                |w| state(w, "Playing", Some("Song"), &["Ada"]),
                &["Metadata"],
            );
            let mut signal = signal;
            let at = at % signal.len();
            signal[at] = value;
            let _ = read_properties_changed(&signal);
        }
    }
}

#[test]
fn a_deterministic_storm_of_random_bodies_never_panics() {
    // xorshift: the same bytes every run, so a failure is reproducible.
    let mut state_word = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        state_word ^= state_word << 13;
        state_word ^= state_word >> 7;
        state_word ^= state_word << 17;
        state_word
    };
    let seeds = [
        get_all("Playing", Some("Song"), &["Ada"]),
        changed(
            PLAYER,
            |w| state(w, "Paused", None, &[]),
            &["PlaybackStatus", "Metadata"],
        ),
    ];
    for round in 0..20_000 {
        let mut body = seeds[round % seeds.len()].clone();
        let flips = 1 + next() as usize % 6;
        for _ in 0..flips {
            let at = next() as usize % body.len();
            body[at] = next() as u8;
        }
        if next() % 4 == 0 {
            body.truncate(next() as usize % (body.len() + 1));
        }
        let _ = read_player_props(&body);
        let _ = read_properties_changed(&body);
    }
}

#[test]
fn a_properties_changed_signal_carries_values_and_invalidations() {
    let body = changed(
        PLAYER,
        |body| {
            entry(body, "PlaybackStatus", "s", &|w| w.str("Playing"));
            entry(body, "Metadata", "a{sv}", &|w| {
                metadata(w, Some("T"), &["A"])
            });
            entry(body, "Position", "x", &|w| w.u64(5));
        },
        &["Volume"],
    );
    let signal = read_properties_changed(&body).unwrap();
    assert_eq!(signal.interface, PLAYER);
    assert_eq!(signal.props.status, Some("Playing"));
    assert_eq!(signal.props.metadata.unwrap().title, Some("T"));
    // Only a property the bar holds counts as invalidated: the volume
    // changing with no value is no reason to ask the player anything.
    assert!(!signal.invalidated);
    for name in ["PlaybackStatus", "Metadata", "CanGoNext", "CanControl"] {
        let body = changed(PLAYER, |_| {}, &[name]);
        assert!(
            read_properties_changed(&body).unwrap().invalidated,
            "{name}"
        );
    }
    let body = changed(PLAYER, |_| {}, &["Position", "Volume", "Rate", "Shuffle"]);
    assert!(!read_properties_changed(&body).unwrap().invalidated);
}

#[test]
fn a_position_only_signal_reads_to_nothing() {
    let body = changed(
        PLAYER,
        |body| entry(body, "Position", "x", &|w| w.u64(123_456)),
        &[],
    );
    let signal = read_properties_changed(&body).unwrap();
    assert_eq!(signal.props, Props::default());
    assert!(!signal.invalidated);
}

#[test]
fn another_interfaces_signal_is_not_walked() {
    // Its dictionary is garbage to the bar: the answer is read off the
    // first string alone, so it cannot be refused for what it holds.
    let mut body = Writer::new();
    body.str("org.mpris.MediaPlayer2");
    body.raw(&[0xff; 16]);
    let body = body.take_body().unwrap();
    let signal = read_properties_changed(&body).unwrap();
    assert_eq!(signal.interface, "org.mpris.MediaPlayer2");
    assert_eq!(signal.props, Props::default());
    assert!(!signal.invalidated);
}

#[test]
fn a_signal_with_too_many_invalidated_names_is_refused() {
    let names: Vec<String> = (0..=MAX_INVALIDATED).map(|i| format!("N{i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let body = changed(PLAYER, |_| {}, &refs);
    assert!(read_properties_changed(&body).is_err());
}

#[test]
fn the_status_signal_builder_reads_back() {
    let body = status_changed("Paused");
    let signal = read_properties_changed(&body).unwrap();
    assert_eq!(signal.props.status, Some("Paused"));
}

/// What sd-bus marshalled (`../fixtures/README.md`): a reader tested only
/// against this crate's own writer agrees with it, right or wrong.
#[test]
fn what_sd_bus_marshalled_is_read_right() {
    use crate::dbus::proto::Message;
    let call = include_bytes!("../fixtures/sdbus-mpris-getall-call.bin");
    let message = Message::parse(call).unwrap();
    assert_eq!(message.signature, "a{sv}");
    let props = read_player_props(message.body.rest()).unwrap();
    assert_eq!(props.status, Some("Playing"));
    assert_eq!(props.can_go_next, Some(true));
    assert_eq!(props.can_go_previous, Some(false));
    assert_eq!(props.can_control, Some(true));
    let track = props.metadata.unwrap();
    assert_eq!(track.title, Some("Sønġ ♪"));
    assert_eq!(track.artists, ["Ada", "Bo"]);

    let signal = include_bytes!("../fixtures/sdbus-mpris-changed-signal.bin");
    let message = Message::parse(signal).unwrap();
    assert_eq!(message.signature, "sa{sv}as");
    let changed = read_properties_changed(message.body.rest()).unwrap();
    assert_eq!(changed.interface, PLAYER);
    assert_eq!(changed.props.status, Some("Paused"));
    let track = changed.props.metadata.unwrap();
    assert_eq!(track.title, Some("Second"));
    assert_eq!(track.artists, ["Cy"]);
    assert!(changed.invalidated, "CanGoNext was invalidated");
}
