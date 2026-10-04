//! The media module against the scripted bus: what it shows, which player
//! it shows, what it does with a hostile or broken one, and that it asks
//! the bus for nothing it does not need. The real daemon's half is
//! `daemon_tests.rs`.

use std::time::{Duration, Instant};

use super::fake::{Answer, Fake, MODULE};
use super::player::{Player, Status, select, short_name};
use super::{MAX_PLAYERS, Settings, start_with};
use crate::action::{Action, ModuleAction, Trigger};
use crate::dbus::link::Addr;
use crate::dbus::mpris::build::{self, entry};
use crate::dbus::mpris::{PLAYER, Props};
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, OutputView, Update};

const MPV: &str = "org.mpris.MediaPlayer2.mpv";
const VLC: &str = "org.mpris.MediaPlayer2.vlc";
const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };

fn up_with(settings: &Settings) -> (Harness, Fake) {
    let (stream, fake) = Fake::pair();
    let harness = Harness::new(start_with(settings, Addr::Stream(stream)));
    (harness, fake)
}

fn up() -> (Harness, Fake) {
    up_with(&Settings::default())
}

/// Turns of the module and the bus until both are quiet for a few polls;
/// whether the module reported a change meanwhile.
fn settle(harness: &mut Harness, fake: &mut Fake) -> bool {
    let mut changed = false;
    let mut quiet = 0;
    for _ in 0..400 {
        let served = fake.pump();
        let woke = harness.wait(Duration::from_millis(5));
        if woke == Some(Update::Changed) {
            changed = true;
        }
        if served == 0 && woke.is_none() && !fake.pending() {
            quiet += 1;
            if quiet >= 4 {
                break;
            }
        } else {
            quiet = 0;
        }
    }
    changed
}

/// As [`settle`], then keeps turning for `ms` more: for what a timer
/// delivers.
fn settle_for(harness: &mut Harness, fake: &mut Fake, ms: u64) -> bool {
    let mut changed = settle(harness, fake);
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        fake.pump();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            changed = true;
        }
    }
    changed
}

fn playing(title: &str, artist: &str) -> Vec<u8> {
    build::get_all("Playing", Some(title), &[artist])
}

fn action(name: &'static str) -> ModuleAction {
    ModuleAction::new(name, None)
}

fn shown(harness: &Harness) -> String {
    harness.view_on(Some("DP-1")).text().to_owned()
}

#[test]
fn no_bus_shows_nothing_and_polls_nothing() {
    let harness = Harness::new(start_with(&Settings::default(), Addr::Unusable));
    assert_eq!(harness.source_count(), 0);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    assert!(harness.view().tooltip().is_empty());
}

#[test]
fn a_bus_with_no_player_polls_the_socket_and_nothing_else() {
    let (mut harness, mut fake) = up();
    settle(&mut harness, &mut fake);
    // One source, the bus: no timer, no second fd. With nothing queued it
    // asks for readability only (no `OUT`, which would spin the poll).
    assert_eq!(harness.source_count(), 1);
    assert_eq!(harness.wait(Duration::from_millis(60)), None);
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
}

#[test]
fn the_bus_is_asked_for_the_mpris_namespace_and_the_player_interface_only() {
    let (mut harness, mut fake) = up();
    settle(&mut harness, &mut fake);
    assert_eq!(fake.matches.len(), 2, "{:?}", fake.matches);
    let names = &fake.matches[0];
    assert!(names.contains("sender='org.freedesktop.DBus'"), "{names}");
    assert!(names.contains("member='NameOwnerChanged'"), "{names}");
    assert!(
        names.contains("arg0namespace='org.mpris.MediaPlayer2'"),
        "{names}"
    );
    let props = &fake.matches[1];
    assert!(props.contains("member='PropertiesChanged'"), "{props}");
    assert!(props.contains("path='/org/mpris/MediaPlayer2'"), "{props}");
    assert!(
        props.contains("arg0='org.mpris.MediaPlayer2.Player'"),
        "{props}"
    );
    // Neither is a broad rule: a Seeked or another app's change reaches
    // the bar by neither.
    assert!(!fake.matches.iter().any(|rule| rule.contains("Seeked")));
}

#[test]
fn a_player_there_before_the_bar_is_found_and_shown() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.text(), "Ada - Song");
    assert_eq!(view.tooltip(), "mpv (playing): Ada - Song");
    assert_eq!(view.class(), Class::Normal);
    assert!(view.art().is_some(), "the play icon");
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["player"], "mpv");
    assert_eq!(value["bus_name"], MPV);
    assert_eq!(value["status"], "playing");
    assert_eq!(value["title"], "Song");
    assert_eq!(value["artist"], "Ada");
    assert_eq!(value["players"].as_array().map(Vec::len), Some(1));
    assert_eq!(fake.getalls(":1.20"), 1, "one read, to the owner");
}

#[test]
fn a_paused_player_is_dimmed_with_its_own_icon_and_a_stopped_one_is_not_shown() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let play_icon = harness.view().art().cloned();
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.class(), Class::Muted);
    assert_eq!(view.text(), "Ada - Song");
    assert_ne!(view.art().cloned(), play_icon, "pause is not play");
    fake.properties_changed(":1.20", &build::status_changed("Stopped"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert!(harness.view().is_empty(), "a stopped player shows nothing");
    assert_eq!(harness.value_on(None), None);
    // It returns when it plays again.
    fake.properties_changed(":1.20", &build::status_changed("Playing"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Ada - Song");
}

#[test]
fn a_track_change_is_one_signal_and_no_round_trip() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    // After a quiet spell, so the change is the first of its run: drawn at
    // once, not held for the draw timer.
    settle_for(&mut harness, &mut fake, 150);
    let body = build::changed(
        PLAYER,
        |w| {
            entry(w, "Metadata", "a{sv}", &|w| {
                build::metadata(w, Some("Next"), &["Bo", "Cy"])
            })
        },
        &[],
    );
    fake.properties_changed(":1.20", &body);
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Bo, Cy - Next");
    assert_eq!(fake.getalls(":1.20"), 1, "the signal carried the value");
}

/// A `PropertiesChanged` carrying a new title.
fn titled(n: usize) -> Vec<u8> {
    build::changed(
        PLAYER,
        |w| {
            entry(w, "Metadata", "a{sv}", &|w| {
                build::metadata(w, Some(&format!("T{n}")), &["Ada"])
            })
        },
        &[],
    )
}

#[test]
fn a_run_of_title_changes_is_drawn_ten_times_a_second_at_most_and_ends_on_the_last() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 150);
    for n in 0..400 {
        fake.properties_changed(":1.20", &titled(n));
    }
    let mut drawn = 0;
    let end = Instant::now() + Duration::from_millis(450);
    while Instant::now() < end {
        fake.pump();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            drawn += 1;
        }
    }
    // The first at once, then one a gap: five in 450 ms, a little over for
    // a gap's rounding, never one for each of the 400.
    assert!((1..=7).contains(&drawn), "{drawn} draws");
    // And the run ends on its last title, once the held one is drawn.
    settle_for(&mut harness, &mut fake, 250);
    assert_eq!(shown(&harness), "Ada - T399");
    assert_eq!(harness.source_count(), 1, "no timer once nothing is held");
}

#[test]
fn the_first_change_after_a_quiet_spell_is_drawn_at_once_and_waits_for_no_timer() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 200);
    fake.properties_changed(":1.20", &titled(1));
    assert!(settle(&mut harness, &mut fake), "drawn in the turn it came");
    assert_eq!(shown(&harness), "Ada - T1");
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn a_state_change_inside_the_gap_is_held_and_not_lost() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 200);
    fake.properties_changed(":1.20", &titled(1));
    settle(&mut harness, &mut fake);
    let first = Instant::now();
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    settle(&mut harness, &mut fake);
    if first.elapsed() < super::DRAW_GAP / 2 {
        assert_eq!(harness.source_count(), 2, "held for the draw timer");
    }
    // The held change is drawn when the timer fires, with the state as it is.
    assert!(settle_for(&mut harness, &mut fake, 250));
    let view = harness.view();
    assert_eq!(view.text(), "Ada - T1");
    assert_eq!(view.class(), Class::Muted);
    assert_eq!(harness.source_count(), 1, "and nothing is held after it");
}

#[test]
fn a_player_flapping_between_playing_and_paused_is_drawn_ten_times_a_second_at_most() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 150);
    // One signal a turn, as a player flapping at hundreds a second is heard
    // when the bar keeps up: a draw each, were they not held.
    let mut drawn = 0;
    let start = Instant::now();
    for n in 0..400 {
        let status = if n % 2 == 0 { "Paused" } else { "Playing" };
        fake.properties_changed(":1.20", &build::status_changed(status));
        fake.pump();
        if harness.wait(Duration::from_millis(1)) == Some(Update::Changed) {
            drawn += 1;
        }
    }
    let ceiling = start.elapsed().as_millis() as usize / 100 + 3;
    assert!(
        (1..=ceiling).contains(&drawn),
        "{drawn} draws in {:?}",
        start.elapsed()
    );
    settle_for(&mut harness, &mut fake, 250);
    // The last signal was Playing (399 is odd): the view ends on it.
    assert_eq!(harness.view().class(), Class::Normal);
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn the_module_emptying_is_never_held_behind_a_draw() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 200);
    fake.properties_changed(":1.20", &titled(1));
    settle(&mut harness, &mut fake);
    let first = Instant::now();
    fake.properties_changed(":1.20", &build::status_changed("Stopped"));
    let changed = settle(&mut harness, &mut fake);
    if first.elapsed() < super::DRAW_GAP / 2 {
        assert!(changed, "the last player stopping empties it at once");
        assert_eq!(harness.source_count(), 1);
    }
    assert!(harness.view().is_empty());
}

#[test]
fn the_bus_going_away_while_a_change_is_held_leaves_no_timer_and_nothing_shown() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 200);
    fake.properties_changed(":1.20", &titled(1));
    settle(&mut harness, &mut fake);
    fake.properties_changed(":1.20", &titled(2));
    settle(&mut harness, &mut fake);
    fake.hang_up();
    for _ in 0..50 {
        if harness.wait(Duration::from_millis(20)) == Some(Update::Changed) {
            break;
        }
    }
    assert!(harness.view().is_empty());
    assert_eq!(
        harness.source_count(),
        1,
        "the directory watch, no draw timer"
    );
}

#[test]
fn missing_metadata_shows_what_there_is_and_then_the_players_name() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", build::get_all("Playing", None, &["Ada"]));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Ada", "an artist and no title");
    fake.properties_changed(
        ":1.20",
        &build::changed(
            PLAYER,
            |w| {
                entry(w, "Metadata", "a{sv}", &|w| {
                    build::metadata(w, Some("Only"), &[])
                })
            },
            &[],
        ),
    );
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Only", "a title and no artist");
    fake.properties_changed(
        ":1.20",
        &build::changed(
            PLAYER,
            |w| entry(w, "Metadata", "a{sv}", &|w| build::metadata(w, None, &[])),
            &[],
        ),
    );
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "mpv", "nothing told: the player's name");
}

#[test]
fn position_and_volume_cost_no_redraw_and_no_round_trip() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let before = fake.getalls(":1.20");
    for i in 0..500u64 {
        let body = build::changed(
            PLAYER,
            |w| {
                entry(w, "Position", "x", &|w| w.u64(i * 1000));
                entry(w, "Volume", "d", &|w| w.u64(0x3fe0_0000_0000_0000));
            },
            &["Rate", "Shuffle"],
        );
        fake.properties_changed(":1.20", &body);
    }
    assert!(!settle(&mut harness, &mut fake), "no view change");
    assert_eq!(fake.getalls(":1.20"), before, "and nothing was asked");
    assert_eq!(shown(&harness), "Ada - Song");
}

#[test]
fn an_invalidated_property_is_asked_for_and_its_flood_is_rate_floored() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let before = fake.getalls(":1.20");
    // The player answers a different track from now on.
    fake.set_answer(":1.20", Answer::Props(playing("Changed", "Zed")));
    let invalidate = build::changed(PLAYER, |_| {}, &["Metadata"]);
    fake.properties_changed(":1.20", &invalidate);
    // Asked once the gap since the last read has passed (a timer).
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(fake.getalls(":1.20"), before + 1);
    assert_eq!(shown(&harness), "Zed - Changed");
    // A storm of invalidations: one read each 50 ms, not one each.
    let start = Instant::now();
    for _ in 0..2000 {
        fake.properties_changed(":1.20", &invalidate);
    }
    settle_for(&mut harness, &mut fake, 300);
    let asked = fake.getalls(":1.20") - before - 1;
    let ceiling = (start.elapsed().as_millis() / 50 + 3) as usize;
    assert!(
        asked >= 1 && asked <= ceiling,
        "{asked} reads in {:?}",
        start.elapsed()
    );
}

#[test]
fn a_player_that_appears_and_vanishes_by_the_bus_is_followed() {
    let (mut harness, mut fake) = up();
    settle(&mut harness, &mut fake);
    assert!(harness.view().is_empty());
    fake.add_unlisted(VLC, ":1.31", playing("Film", "Studio"));
    fake.name_owner_changed(VLC, "", ":1.31");
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert_eq!(shown(&harness), "Studio - Film");
    fake.name_owner_changed(VLC, ":1.31", "");
    assert!(settle_for(&mut harness, &mut fake, 150));
    assert!(harness.view().is_empty(), "gone with its owner");
}

#[test]
fn a_name_that_changes_hands_does_not_keep_the_old_players_track() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Old", "Ada"));
    settle(&mut harness, &mut fake);
    fake.script(
        MPV,
        ":1.55",
        false,
        Answer::Props(build::get_all("Paused", Some("New"), &[])),
    );
    fake.name_owner_changed(MPV, ":1.20", ":1.55");
    // The new owner is read once the refresh gap since the old one's read
    // has passed (a timer, not a poll).
    settle_for(&mut harness, &mut fake, 120);
    let view = harness.view();
    assert_eq!(view.text(), "New");
    assert_eq!(view.class(), Class::Muted);
    // And the old owner's signals are not the player's any more.
    fake.properties_changed(":1.20", &build::status_changed("Playing"));
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(harness.view().class(), Class::Muted);
}

#[test]
fn a_forged_owner_change_or_signal_is_not_believed() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    // A peer says the player is gone, and that another has arrived.
    fake.name_owner_changed_from(":1.66", MPV, ":1.20", "");
    fake.name_owner_changed_from(":1.66", VLC, "", ":1.66");
    // A stranger (not the player's connection) says it is paused, with a
    // title of its own, and one says it from the wrong object.
    fake.properties_changed(":1.66", &build::get_all("Paused", Some("Fake"), &[]));
    fake.properties_changed(":1.66", &build::status_changed("Paused"));
    fake.signal(
        ":1.20",
        "/elsewhere",
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        "sa{sv}as",
        &build::status_changed("Paused"),
    );
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "Ada - Song");
    assert_eq!(harness.view().class(), Class::Normal);
    assert_eq!(
        fake.getalls(":1.66"),
        0,
        "the forged arrival was never read"
    );
}

#[test]
fn a_signal_for_another_interface_or_in_the_wrong_shape_changes_nothing() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let other = build::changed(
        "org.mpris.MediaPlayer2",
        |w| {
            entry(w, "PlaybackStatus", "s", &|w| w.str("Paused"));
        },
        &[],
    );
    fake.properties_changed(":1.20", &other);
    fake.signal(
        ":1.20",
        "/org/mpris/MediaPlayer2",
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        "s",
        &[4, 0, 0, 0, b'x', b'x', b'x', b'x', 0],
    );
    fake.properties_changed(":1.20", &[0xff; 24]);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(harness.view().class(), Class::Normal);
}

#[test]
fn several_players_show_the_one_that_started_playing_last() {
    let (mut harness, mut fake) = up();
    fake.add_player(
        MPV,
        ":1.20",
        build::get_all("Paused", Some("Mpv song"), &[]),
    );
    fake.add_player(
        VLC,
        ":1.31",
        build::get_all("Paused", Some("Vlc song"), &[]),
    );
    settle(&mut harness, &mut fake);
    // Both paused and never played: the name decides, the same every run.
    assert_eq!(shown(&harness), "Mpv song");
    fake.properties_changed(":1.31", &build::status_changed("Playing"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Vlc song");
    fake.properties_changed(":1.20", &build::status_changed("Playing"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Mpv song", "the most recent to start");
    // The shown one pauses: the other, still playing, takes over.
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Vlc song");
    // Both paused: the one that played last (it stopped last).
    fake.properties_changed(":1.31", &build::status_changed("Paused"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Vlc song");
    fake.properties_changed(":1.20", &build::status_changed("Playing"));
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Mpv song", "it was the last to play");
    let players = harness.value_on(None).unwrap()["players"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(players, 2);
}

#[test]
fn the_preferred_player_wins_while_it_is_not_stopped() {
    let settings = Settings {
        player: Some("vlc".to_owned()),
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.add_player(MPV, ":1.20", playing("Mpv song", ""));
    fake.add_player(
        VLC,
        ":1.31",
        build::get_all("Paused", Some("Vlc song"), &[]),
    );
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Vlc song", "paused, but the one asked for");
    fake.properties_changed(":1.31", &build::status_changed("Stopped"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Mpv song", "a stopped one is not shown");
}

#[test]
fn one_connection_is_one_player_and_the_count_is_bounded() {
    let (mut harness, mut fake) = up();
    // Two names, one connection: the first stays.
    fake.add_player(MPV, ":1.20", playing("One", ""));
    fake.add_player(
        "org.mpris.MediaPlayer2.mpv.instance7",
        ":1.20",
        playing("Two", ""),
    );
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.value_on(None).unwrap()["players"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Many connections: at most MAX_PLAYERS held.
    for i in 0..MAX_PLAYERS + 6 {
        let name = format!("org.mpris.MediaPlayer2.p{i}");
        let owner = format!(":1.{}", 100 + i);
        fake.add_unlisted(&name, &owner, playing(&format!("T{i}"), ""));
        fake.name_owner_changed(&name, "", &owner);
    }
    settle(&mut harness, &mut fake);
    let held = harness.value_on(None).unwrap()["players"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(held, MAX_PLAYERS);
}

#[test]
fn one_connection_owning_many_names_does_not_hide_a_real_player_listed_after_it() {
    let (mut harness, mut fake) = up();
    for i in 0..300 {
        fake.add_player(
            &format!("org.mpris.MediaPlayer2.squat{i}"),
            ":1.99",
            playing("Squatter", "Eve"),
        );
    }
    fake.add_player(MPV, ":1.20", playing("Real", "Ada"));
    settle(&mut harness, &mut fake);
    let value = harness.value_on(None).unwrap();
    let players = value["players"].as_array().unwrap().len();
    assert_eq!(players, 2, "the squatter once, the real one: {value}");
    assert!(
        fake.getalls(":1.99") == 1,
        "the squatter is read once, not once a name"
    );
    assert_eq!(fake.getalls(":1.20"), 1);
}

#[test]
fn a_flood_of_listed_names_asks_a_bounded_number_of_questions() {
    let (mut harness, mut fake) = up();
    for i in 0..300 {
        fake.add_player(
            &format!("org.mpris.MediaPlayer2.n{i}"),
            &format!(":1.{}", 200 + i),
            playing(&format!("T{i}"), ""),
        );
    }
    settle(&mut harness, &mut fake);
    let asked: usize = (0..300)
        .map(|i| fake.getalls(&format!(":1.{}", 200 + i)))
        .sum();
    assert!(asked <= MAX_PLAYERS, "{asked} players read");
    assert!(harness.value_on(None).is_some());
}

#[test]
fn a_reply_meant_for_a_player_that_left_finds_nothing() {
    let (mut harness, mut fake) = up();
    fake.script(MPV, ":1.20", true, Answer::Silent);
    settle(&mut harness, &mut fake);
    let late = fake.unanswered(":1.20");
    assert_eq!(late.len(), 1);
    // It leaves and a new process takes the name; the old question is
    // answered late, with a track.
    fake.name_owner_changed(MPV, ":1.20", "");
    settle(&mut harness, &mut fake);
    fake.script(MPV, ":1.77", false, Answer::Silent);
    fake.name_owner_changed(MPV, "", ":1.77");
    settle(&mut harness, &mut fake);
    fake.reply_late(late[0], "a{sv}", &playing("Ghost", "Old"));
    assert!(!settle(&mut harness, &mut fake));
    assert!(
        harness.view().is_empty(),
        "the new owner has said nothing yet"
    );
}

#[test]
fn a_reply_from_the_old_owner_of_a_name_that_changed_hands_finds_nothing() {
    let (mut harness, mut fake) = up();
    fake.script(MPV, ":1.20", true, Answer::Silent);
    settle(&mut harness, &mut fake);
    let late = fake.unanswered(":1.20");
    assert_eq!(late.len(), 1);
    fake.script(MPV, ":1.55", false, Answer::Silent);
    fake.name_owner_changed(MPV, ":1.20", ":1.55");
    settle_for(&mut harness, &mut fake, 120);
    // The old owner's answer comes after the name moved on.
    fake.reply_late(late[0], "a{sv}", &playing("Ghost", "Old"));
    assert!(!settle(&mut harness, &mut fake));
    assert!(harness.view().is_empty(), "the new owner has said nothing");
}

#[test]
fn a_player_that_timed_out_is_kept_and_one_that_is_no_player_is_not() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    // A busy player: the bus says it did not answer in time.
    fake.set_answer(":1.20", Answer::Error("org.freedesktop.DBus.Error.NoReply"));
    fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(shown(&harness), "Ada - Song", "kept, with what it said");
    // It answers the next time it is asked.
    fake.set_answer(":1.20", Answer::Props(playing("Back", "Bo")));
    std::thread::sleep(Duration::from_millis(60));
    fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(shown(&harness), "Bo - Back");
}

#[test]
fn a_player_that_never_answers_costs_one_slot_and_is_asked_again_after_the_ttl() {
    let (mut harness, mut fake) = up();
    fake.script(MPV, ":1.20", true, Answer::Silent);
    settle(&mut harness, &mut fake);
    let asked_at = Instant::now();
    assert_eq!(fake.getalls(":1.20"), 1);
    let nudge = build::changed(PLAYER, |_| {}, &["PlaybackStatus"]);
    // While it is within the TTL a nudge asks nothing more. (The claim is
    // about elapsed time, so it is only made while the time held: a loaded
    // machine that stalled past the TTL checks the rest.)
    fake.properties_changed(":1.20", &nudge);
    settle_for(&mut harness, &mut fake, 100);
    if asked_at.elapsed() < super::FLIGHT_TTL {
        assert_eq!(fake.getalls(":1.20"), 1, "one in flight at a time");
    }
    // Past the TTL the forgotten question is asked again.
    std::thread::sleep(super::FLIGHT_TTL + Duration::from_millis(50));
    fake.properties_changed(":1.20", &nudge);
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(fake.getalls(":1.20"), 2);
    // Now it answers: shown.
    fake.set_answer(":1.20", Answer::Props(playing("Late", "Bo")));
    std::thread::sleep(super::FLIGHT_TTL + Duration::from_millis(50));
    fake.properties_changed(":1.20", &nudge);
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(shown(&harness), "Bo - Late");
}

#[test]
fn a_player_whose_reads_error_is_held_unshown_and_one_whose_reply_is_garbage_keeps_its_state() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    fake.script(
        VLC,
        ":1.31",
        true,
        Answer::Error("org.freedesktop.DBus.Error.UnknownMethod"),
    );
    settle(&mut harness, &mut fake);
    // The erroring one is held, stopped (it shows nothing, and is the first to
    // make room), not dropped.
    assert_eq!(held(&harness), [MPV, VLC]);
    assert_eq!(statuses_of(&harness), ["playing", "stopped"]);
    // A reply of the wrong signature, then one whose bytes are garbage:
    // each is dropped whole.
    for answer in [
        Answer::Raw("s", vec![1, 0, 0, 0, b'x', 0]),
        Answer::Raw("a{sv}", vec![0xff; 40]),
        Answer::Raw("a{sv}", Vec::new()),
    ] {
        fake.set_answer(":1.20", answer);
        fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
        settle_for(&mut harness, &mut fake, 80);
        assert_eq!(shown(&harness), "Ada - Song");
    }
}

#[test]
fn a_title_that_is_not_utf8_loses_the_answer_and_not_the_player() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    // A real bus refuses a string that is not UTF-8 before it is routed;
    // a client must not depend on that. The answer is dropped whole.
    let mut body = build::get_all("Playing", Some("Broken title"), &["Eve"]);
    let at = body.windows(6).position(|w| w == b"Broken").unwrap();
    body[at] = 0xff;
    body[at + 1] = 0xfe;
    fake.set_answer(":1.20", Answer::Raw("a{sv}", body));
    fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
    settle_for(&mut harness, &mut fake, 120);
    assert_eq!(shown(&harness), "Ada - Song", "the last good state stands");
    // And the same bytes in a signal's own dictionary.
    let mut signal = build::changed(
        PLAYER,
        |w| {
            entry(w, "Metadata", "a{sv}", &|w| {
                build::metadata(w, Some("Broken title"), &["Eve"])
            })
        },
        &[],
    );
    let at = signal.windows(6).position(|w| w == b"Broken").unwrap();
    signal[at] = 0xff;
    fake.properties_changed(":1.20", &signal);
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "Ada - Song");
}

#[test]
fn an_oversized_reply_loses_only_that_update() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    // A valid 2 MiB reply to the next read (a title of 2 MiB).
    let huge = "x".repeat(2 << 20);
    let mut body = crate::dbus::proto::Writer::with_cap(4 << 20);
    if let Some(cookie) = body.open_array(8) {
        entry(&mut body, "Metadata", "a{sv}", &|w| {
            if let Some(cookie) = w.open_array(8) {
                entry(w, "xesam:title", "s", &|w| w.str(&huge));
                w.close_array(cookie);
            }
        });
        body.close_array(cookie);
    }
    let body = body.take_body().unwrap();
    fake.set_answer(":1.20", Answer::Raw("a{sv}", body));
    fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
    settle_for(&mut harness, &mut fake, 100);
    assert_eq!(shown(&harness), "Ada - Song", "the old state stands");
    // The connection lives: a later answer applies.
    fake.set_answer(":1.20", Answer::Props(playing("After", "Bo")));
    std::thread::sleep(Duration::from_millis(60));
    fake.properties_changed(":1.20", &build::changed(PLAYER, |_| {}, &["Metadata"]));
    settle_for(&mut harness, &mut fake, 150);
    assert_eq!(shown(&harness), "Bo - After");
}

#[test]
fn hostile_text_is_cleaned_and_cut_where_it_is_stored() {
    let (mut harness, mut fake) = up();
    let long = "é".repeat(5000);
    fake.add_player(
        MPV,
        ":1.20",
        build::get_all(
            "Playing",
            Some(&format!("a\u{1b}[31mb\0c\n{long}")),
            &["\u{7}", "Ada\r\n", &long],
        ),
    );
    settle(&mut harness, &mut fake);
    let view = harness.view_on(None);
    let text = view.text();
    assert!(!text.chars().any(char::is_control), "{text:?}");
    // The bell cleaned to no artist at all, so no empty name or comma
    // between the two real ones; escapes lose their ESC and keep the rest.
    assert!(text.starts_with("Ada, é"), "{text:?}");
    assert!(text.contains(" - a[31mbc"), "{text:?}");
    assert!(
        text.len() <= 2 * super::player::MAX_FIELD + 3,
        "{} bytes",
        text.len()
    );
    assert!(!view.was_cut(), "within the view's bound");
    let value = harness.value_on(None).unwrap();
    assert!(value["title"].as_str().unwrap().len() <= super::player::MAX_FIELD);
    assert!(value["artist"].as_str().unwrap().len() <= super::player::MAX_FIELD);
}

#[test]
fn the_bus_going_away_empties_the_module() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    assert!(!harness.view().is_empty());
    fake.hang_up();
    let mut changed = false;
    for _ in 0..50 {
        if harness.wait(Duration::from_millis(20)) == Some(Update::Changed) {
            changed = true;
            break;
        }
    }
    assert!(changed, "the drop is a change");
    assert!(harness.view().is_empty());
    assert_eq!(harness.value_on(None), None);
    // A refused action says why.
    assert_eq!(
        harness.invoke(&DP1, &action("play-pause"), 1),
        Err(InvokeError::Refused("no session bus"))
    );
}

#[test]
fn a_refused_match_rule_is_said_and_the_module_still_runs() {
    let (mut harness, mut fake) = up();
    fake.refuse_matches = true;
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    assert_eq!(shown(&harness), "Ada - Song", "what was listed is shown");
}

#[test]
fn controls_go_to_the_player_shown_as_calls_that_want_no_reply() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    fake.add_player(VLC, ":1.31", build::get_all("Paused", Some("Film"), &[]));
    settle(&mut harness, &mut fake);
    let _ = fake.calls();
    for (name, member) in [("play-pause", "PlayPause"), ("next", "Next")] {
        assert_eq!(
            harness.invoke(&DP1, &action(name), 1),
            Ok(Update::Unchanged)
        );
        settle(&mut harness, &mut fake);
        let calls = fake.calls();
        assert_eq!(calls.len(), 1, "{name}: {calls:?}");
        assert_eq!(calls[0].destination, ":1.20", "the owner, not the name");
        assert_eq!(calls[0].path, "/org/mpris/MediaPlayer2");
        assert_eq!(calls[0].interface, PLAYER);
        assert_eq!(calls[0].member, member);
        assert_eq!(calls[0].flags & 1, 1, "NO_REPLY_EXPECTED");
        std::thread::sleep(Duration::from_millis(260));
    }
    assert_eq!(
        harness.invoke(&DP1, &action("previous"), 1),
        Ok(Update::Unchanged)
    );
    settle(&mut harness, &mut fake);
    assert_eq!(fake.calls()[0].member, "Previous");
}

#[test]
fn a_scroll_of_many_steps_and_a_fling_are_one_skip() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let _ = fake.calls();
    // One action covering thirty steps: one call.
    assert_eq!(
        harness.invoke(&DP1, &action("next"), 30),
        Ok(Update::Unchanged)
    );
    // Sixty actions a second, as a fling delivers: the gap holds them, and
    // says so (the pointer path limits how often that is said).
    let held = InvokeError::Refused("a skip within 250 ms of the last one is ignored");
    for _ in 0..20 {
        assert_eq!(harness.invoke(&DP1, &action("next"), 3), Err(held.clone()));
        assert_eq!(
            harness.invoke(&DP1, &action("previous"), 3),
            Err(held.clone())
        );
    }
    settle(&mut harness, &mut fake);
    assert_eq!(fake.calls().len(), 1);
    // Play/pause is not a skip: never held.
    for _ in 0..3 {
        assert_eq!(
            harness.invoke(&DP1, &action("play-pause"), 1),
            Ok(Update::Unchanged)
        );
    }
    settle(&mut harness, &mut fake);
    assert_eq!(fake.calls().len(), 3);
    // Past the gap a skip goes through again.
    std::thread::sleep(Duration::from_millis(260));
    assert_eq!(
        harness.invoke(&DP1, &action("previous"), 1),
        Ok(Update::Unchanged)
    );
}

#[test]
fn an_action_is_refused_with_its_reason() {
    let (mut harness, mut fake) = up();
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &action("next"), 1),
        Err(InvokeError::Refused("no player is playing or paused"))
    );
    assert_eq!(
        harness.invoke(&DP1, &action("seek"), 1),
        Err(InvokeError::Unknown)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("next", Some(3)), 1),
        Err(InvokeError::NoArg)
    );
    // A player that says it cannot.
    let body = build::get_all_with(|w| {
        entry(w, "PlaybackStatus", "s", &|w| w.str("Playing"));
        entry(w, "CanGoNext", "b", &|w| w.boolean(false));
        entry(w, "CanGoPrevious", "b", &|w| w.boolean(true));
    });
    fake.add_unlisted(MPV, ":1.20", body);
    fake.name_owner_changed(MPV, "", ":1.20");
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &action("next"), 1),
        Err(InvokeError::Refused("the player cannot skip forward"))
    );
    assert_eq!(
        harness.invoke(&DP1, &action("previous"), 1),
        Ok(Update::Unchanged)
    );
    fake.properties_changed(
        ":1.20",
        &build::changed(
            PLAYER,
            |w| entry(w, "CanControl", "b", &|w| w.boolean(false)),
            &[],
        ),
    );
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &action("play-pause"), 1),
        Err(InvokeError::Refused("the player takes no commands"))
    );
}

#[test]
fn clicks_and_scrolls_have_defaults_only_with_a_player() {
    use crate::density::Scale;
    use crate::modules::{ClickCtx, Input};
    use crate::text::Text;
    use ab_glyph::{FontArc, FontVec};
    let (mut harness, mut fake) = up();
    let font = FontArc::new(FontVec::try_from_vec(crate::testfont::build()).unwrap());
    let text = Text::new(font);
    let view = harness.view();
    let ctx = ClickCtx {
        output: OutputView { name: None },
        x: 10,
        view: &view,
        text: &text,
        em: 50.0,
        padding: 10,
        span_width: 400,
        height: 60,
        scale: Scale::Integer(1),
    };
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.input(&Input {
            trigger: Trigger::Click,
            at: &ctx
        }),
        None
    );
    fake.add_unlisted(MPV, ":1.20", playing("Song", "Ada"));
    fake.name_owner_changed(MPV, "", ":1.20");
    settle(&mut harness, &mut fake);
    let input = |trigger| harness.input(&Input { trigger, at: &ctx });
    let module = |name| Some(Action::Module(ModuleAction::new(name, None)));
    assert_eq!(input(Trigger::Click), module("play-pause"));
    assert_eq!(input(Trigger::RightClick), module("next"));
    assert_eq!(input(Trigger::ScrollDown), module("next"));
    assert_eq!(input(Trigger::MiddleClick), module("previous"));
    assert_eq!(input(Trigger::ScrollUp), module("previous"));
}

#[test]
fn the_icons_parse() {
    use crate::icon::path::{Vector, ViewBox};
    for path in [super::PLAY, super::PAUSE] {
        assert!(Vector::parse(path, ViewBox::default()).is_ok(), "{path}");
    }
}

fn player(name: &str, owner: &str, status: Status, active: u64) -> Player {
    let mut player = Player::new(1, name.to_owned(), owner.to_owned());
    player.status = status;
    player.active = active;
    player
}

#[test]
fn the_choice_of_player_is_an_order_not_luck() {
    let a = |s, n| player("org.mpris.MediaPlayer2.a", ":1.1", s, n);
    let b = |s, n| player("org.mpris.MediaPlayer2.b", ":1.2", s, n);
    assert_eq!(select(&[], None), None);
    assert_eq!(select(&[a(Status::Stopped, 9)], None), None);
    // Playing beats paused whatever played when; later beats earlier.
    assert_eq!(
        select(&[a(Status::Paused, 9), b(Status::Playing, 1)], None),
        Some(1)
    );
    assert_eq!(
        select(&[a(Status::Playing, 2), b(Status::Playing, 3)], None),
        Some(1)
    );
    assert_eq!(
        select(&[a(Status::Paused, 5), b(Status::Paused, 3)], None),
        Some(0)
    );
    // A tie goes to the smaller name, in whichever order they are held.
    assert_eq!(
        select(&[a(Status::Paused, 0), b(Status::Paused, 0)], None),
        Some(0)
    );
    assert_eq!(
        select(&[b(Status::Paused, 0), a(Status::Paused, 0)], None),
        Some(1)
    );
    // The preferred one, when it is not stopped.
    assert_eq!(
        select(&[a(Status::Playing, 9), b(Status::Paused, 1)], Some("b")),
        Some(1)
    );
    assert_eq!(
        select(&[a(Status::Playing, 9), b(Status::Stopped, 1)], Some("b")),
        Some(0)
    );
    assert_eq!(
        select(&[a(Status::Playing, 9), b(Status::Paused, 1)], Some("c")),
        Some(0)
    );
}

#[test]
fn short_names_and_status_words() {
    assert_eq!(short_name("org.mpris.MediaPlayer2.mpv"), "mpv");
    assert_eq!(short_name("org.mpris.MediaPlayer2.vlc.instance7389"), "vlc");
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.org.example.Player"),
        "org.example.Player"
    );
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.x.instanceabc"),
        "x.instanceabc"
    );
    assert_eq!(short_name("org.mpris.MediaPlayer2.instance5"), "instance5");
    // A second copy's own suffix: a pid, digits and underscores (Firefox),
    // a dash and a random string (mpv's script).
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.firefox.instance_1_87"),
        "firefox"
    );
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.mpv.instance-PTHiuoaF"),
        "mpv"
    );
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.x.instanceof"),
        "x.instanceof"
    );
    assert_eq!(short_name("org.mpris.MediaPlayer2.x.instance"), "x");
    assert_eq!(
        short_name("org.mpris.MediaPlayer2.x.instance.y"),
        "x.instance.y"
    );
    assert_eq!(Status::parse("Playing"), Status::Playing);
    assert_eq!(Status::parse("Paused"), Status::Paused);
    for odd in ["Stopped", "", "playing", "Buffering"] {
        assert_eq!(Status::parse(odd), Status::Stopped);
    }
}

#[test]
fn apply_reuses_its_buffers_and_reports_only_a_real_change() {
    let mut player = Player::new(1, MPV.to_owned(), ":1.20".to_owned());
    let mut scratch = String::new();
    let mut tick = 0;
    let body = playing("Song", "Ada");
    let props = crate::dbus::mpris::read_player_props(&body).unwrap();
    assert!(player.apply(&props, &mut scratch, &mut tick));
    let revision = player.rev;
    assert!(
        !player.apply(&props, &mut scratch, &mut tick),
        "the same again"
    );
    assert_eq!(player.rev, revision);
    assert_eq!(player.active, 1, "started playing once");
    let paused = build::status_changed("Paused");
    let props = crate::dbus::mpris::read_properties_changed(&paused)
        .unwrap()
        .props;
    assert!(player.apply(&props, &mut scratch, &mut tick));
    assert_eq!(player.active, 2, "and stopped once");
    let only_position = Props::default();
    assert!(!player.apply(&only_position, &mut scratch, &mut tick));
    assert_eq!(MODULE, ":1.7");
}

#[test]
fn a_line_longer_than_its_span_is_cut_after_the_icon_and_stays_inside_it() {
    use crate::density::Scale;
    use crate::modules::CustomDraw;
    use crate::paint::{Canvas, Span};
    use crate::text::Text;
    use crate::theme::Theme;
    use ab_glyph::{FontArc, FontVec};
    const EM: f32 = 50.0;
    const PAD: u32 = 8;
    let (mut harness, mut fake) = up();
    fake.add_player(
        MPV,
        ":1.20",
        playing("abcdefghijklmnopqrstuvwxyz", "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
    );
    settle(&mut harness, &mut fake);
    let view = harness.view();
    let mut font = Text::new(FontArc::new(
        FontVec::try_from_vec(crate::testfont::build()).unwrap(),
    ));
    let theme = Theme::default();
    let baseline = font.metrics(EM).baseline(60);
    let icon = crate::render::art_extent(&font, &view, EM);
    assert!(icon > 0, "the play icon takes room");
    let full = font.measure(None, view.text(), EM) + icon;
    let mut draw = |span: Span| {
        let mut pixels = vec![0u8; 60 * 1600 * 4];
        let mut canvas = Canvas::new(&mut pixels, 1600, 60).unwrap();
        let drew = harness.custom_draw(&mut CustomDraw {
            output: OutputView { name: None },
            view: &view,
            canvas: &mut canvas,
            text: &mut font,
            span,
            em: EM,
            baseline,
            padding: PAD,
            hovered: false,
            scale: Scale::Integer(1),
            theme: &theme,
        });
        (drew, pixels)
    };
    let ink_columns = |pixels: &[u8]| -> Vec<usize> {
        (0..1600)
            .filter(|x| (0..60).any(|y| pixels[(y * 1600 + x) * 4..][..4].iter().any(|b| *b != 0)))
            .collect()
    };
    // A span that holds the line whole: the plain draw stands.
    assert!(
        !draw(Span {
            x: 50,
            width: full + PAD * 2
        })
        .0
    );
    // A narrower one: drawn here, the icon first and the ellipsis last, and
    // nothing outside the span.
    let (drew, pixels) = draw(Span { x: 50, width: 300 });
    assert!(drew);
    let columns = ink_columns(&pixels);
    assert!(!columns.is_empty());
    assert!(
        columns.iter().all(|x| (50..350).contains(x)),
        "ink outside the span"
    );
    assert!(
        columns.iter().any(|x| *x < 50 + (PAD + icon) as usize),
        "the icon"
    );
    assert!(
        columns.iter().any(|x| *x >= 50 + (PAD + icon) as usize),
        "text after it"
    );
    // A span narrower than the icon and the padding: no panic, no ink
    // outside it.
    let (drew, pixels) = draw(Span { x: 50, width: 1 });
    assert!(drew);
    assert!(ink_columns(&pixels).iter().all(|x| *x == 50));
}

#[test]
fn a_frame_that_is_not_a_message_ends_the_connection_and_empties_the_module() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    assert!(!harness.view().is_empty());
    // A header whose lengths are no message's.
    fake.raw(&[
        b'l', 9, 0, 1, 0xff, 0xff, 0xff, 0xff, 1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
    ]);
    let mut changed = false;
    for _ in 0..50 {
        fake.pump();
        if harness.wait(Duration::from_millis(20)) == Some(Update::Changed) {
            changed = true;
            break;
        }
    }
    assert!(changed);
    assert!(harness.view().is_empty());
}

#[test]
fn an_owner_change_with_a_body_that_does_not_parse_is_ignored() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    fake.signal(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
        "sss",
        &[0xff; 12],
    );
    fake.signal(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
        "s",
        &[0, 0, 0, 0, 0],
    );
    assert!(!settle(&mut harness, &mut fake));
    assert_eq!(shown(&harness), "Ada - Song");
}

/// The bus names of the players held, sorted.
fn held(harness: &Harness) -> Vec<String> {
    let Some(value) = harness.value_on(None) else {
        return Vec::new();
    };
    let mut names: Vec<String> = value["players"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["bus_name"].as_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// A player announcing itself by `NameOwnerChanged`.
fn announce(fake: &mut Fake, name: &str, owner: &str, props: Vec<u8>) {
    fake.add_unlisted(name, owner, props);
    fake.name_owner_changed(name, "", owner);
}

fn pname(i: usize) -> String {
    format!("org.mpris.MediaPlayer2.p{i}")
}

fn fill(fake: &mut Fake) {
    for i in 0..MAX_PLAYERS {
        announce(
            fake,
            &pname(i),
            &format!(":1.{}", 100 + i),
            playing("Held", "Ada"),
        );
    }
}

#[test]
fn a_player_that_found_the_room_full_is_held_when_a_slot_frees() {
    let (mut harness, mut fake) = up();
    fill(&mut fake);
    settle_for(&mut harness, &mut fake, 200);
    announce(&mut fake, MPV, ":1.20", playing("Late", "Bo"));
    settle_for(&mut harness, &mut fake, 200);
    assert_eq!(held(&harness).len(), MAX_PLAYERS);
    assert!(!held(&harness).contains(&MPV.to_owned()), "no room yet");
    fake.name_owner_changed(&pname(3), ":1.103", "");
    settle_for(&mut harness, &mut fake, 300);
    let now = held(&harness);
    assert_eq!(now.len(), MAX_PLAYERS);
    assert!(
        now.contains(&MPV.to_owned()),
        "held once a slot freed: {now:?}"
    );
    assert!(!now.contains(&pname(3)));
}

#[test]
fn names_listed_behind_a_full_room_are_held_when_slots_free() {
    let (mut harness, mut fake) = up();
    for i in 0..MAX_PLAYERS + 4 {
        fake.add_player(
            &pname(i),
            &format!(":1.{}", 100 + i),
            playing("Held", "Ada"),
        );
    }
    settle_for(&mut harness, &mut fake, 300);
    let first = held(&harness);
    assert_eq!(first.len(), MAX_PLAYERS);
    // Every one that left is replaced, from the four that were waiting.
    for name in first.iter().take(4).cloned().collect::<Vec<_>>() {
        let owner = format!(
            ":1.{}",
            100 + name
                .trim_start_matches("org.mpris.MediaPlayer2.p")
                .parse::<usize>()
                .unwrap()
        );
        fake.name_owner_changed(&name, &owner, "");
    }
    settle_for(&mut harness, &mut fake, 400);
    let now = held(&harness);
    assert_eq!(now.len(), MAX_PLAYERS, "{now:?}");
    assert_eq!(
        now.iter().filter(|n| !first.contains(n)).count(),
        4,
        "{now:?}"
    );
}

#[test]
fn a_newcomer_takes_the_place_of_the_oldest_stopped_player() {
    let (mut harness, mut fake) = up();
    announce(
        &mut fake,
        &pname(0),
        ":1.100",
        build::get_all("Stopped", Some("Idle"), &[]),
    );
    for i in 1..MAX_PLAYERS {
        announce(
            &mut fake,
            &pname(i),
            &format!(":1.{}", 100 + i),
            playing("Held", "Ada"),
        );
    }
    settle_for(&mut harness, &mut fake, 300);
    assert!(held(&harness).contains(&pname(0)));
    announce(&mut fake, MPV, ":1.20", playing("New", "Bo"));
    settle_for(&mut harness, &mut fake, 300);
    let now = held(&harness);
    assert_eq!(now.len(), MAX_PLAYERS);
    assert!(now.contains(&MPV.to_owned()), "{now:?}");
    assert!(
        !now.contains(&pname(0)),
        "the stopped one made room: {now:?}"
    );
    // It is not lost: it is held again when a slot frees.
    fake.name_owner_changed(&pname(5), ":1.105", "");
    settle_for(&mut harness, &mut fake, 300);
    assert!(held(&harness).contains(&pname(0)), "{:?}", held(&harness));
}

#[test]
fn a_connection_releasing_one_name_keeps_its_player_under_the_other() {
    let (mut harness, mut fake) = up();
    let second = "org.mpris.MediaPlayer2.mpv.instance7";
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    fake.add_player(second, ":1.20", playing("Song", "Ada"));
    settle_for(&mut harness, &mut fake, 200);
    assert_eq!(held(&harness), [MPV]);
    fake.name_owner_changed(MPV, ":1.20", "");
    settle_for(&mut harness, &mut fake, 300);
    assert_eq!(
        held(&harness),
        [second],
        "still one player, under its other name"
    );
    assert_eq!(shown(&harness), "Ada - Song");
}

#[test]
fn the_list_of_waiting_names_is_bounded() {
    let (mut harness, mut fake) = up();
    fill(&mut fake);
    for i in 0..60 {
        announce(
            &mut fake,
            &pname(100 + i),
            &format!(":1.{}", 300 + i),
            playing("W", "x"),
        );
    }
    settle_for(&mut harness, &mut fake, 400);
    assert_eq!(held(&harness).len(), MAX_PLAYERS);
    // Free every slot: at most the waiting list's worth come in behind them.
    for i in 0..MAX_PLAYERS {
        fake.name_owner_changed(&pname(i), &format!(":1.{}", 100 + i), "");
    }
    settle_for(&mut harness, &mut fake, 600);
    let now = held(&harness);
    assert_eq!(now.len(), MAX_PLAYERS, "{now:?}");
    assert!(
        now.iter().all(|n| n.contains(".p1")),
        "only waiting names: {now:?}"
    );
}

#[test]
fn a_player_whose_first_read_errors_is_kept_and_shown_once_it_says_something() {
    let (mut harness, mut fake) = up();
    // It has the name before it exports the object.
    fake.script(
        MPV,
        ":1.20",
        true,
        Answer::Error("org.freedesktop.DBus.Error.UnknownObject"),
    );
    settle_for(&mut harness, &mut fake, 150);
    assert!(harness.view().is_empty());
    // It exports, and signals (a position alone is enough to be read again).
    fake.set_answer(":1.20", Answer::Props(playing("Song", "Ada")));
    let position = build::changed(PLAYER, |w| entry(w, "Position", "x", &|w| w.u64(5)), &[]);
    fake.properties_changed(":1.20", &position);
    settle_for(&mut harness, &mut fake, 250);
    assert_eq!(shown(&harness), "Ada - Song");
    // And it is read once, not on every signal after.
    let reads = fake.getalls(":1.20");
    for _ in 0..50 {
        fake.properties_changed(":1.20", &position);
    }
    settle_for(&mut harness, &mut fake, 100);
    assert_eq!(fake.getalls(":1.20"), reads);
}

fn statuses_of(harness: &Harness) -> Vec<String> {
    let value = harness.value_on(None).unwrap();
    let mut held: Vec<(String, String)> = value["players"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["bus_name"].as_str().unwrap().to_owned(),
                p["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    held.sort();
    held.into_iter().map(|(_, status)| status).collect()
}

#[test]
fn a_waiting_player_takes_the_place_of_a_held_one_that_stops() {
    let (mut harness, mut fake) = up();
    fill(&mut fake);
    settle_for(&mut harness, &mut fake, 200);
    announce(&mut fake, MPV, ":1.20", playing("Late", "Bo"));
    settle_for(&mut harness, &mut fake, 200);
    assert!(!held(&harness).contains(&MPV.to_owned()));
    // A held player stops: it shows nothing, so the one waiting has its room.
    fake.properties_changed(":1.103", &build::status_changed("Stopped"));
    settle_for(&mut harness, &mut fake, 300);
    let now = held(&harness);
    assert!(now.contains(&MPV.to_owned()), "{now:?}");
    assert!(!now.contains(&pname(3)), "{now:?}");
    assert_eq!(now.len(), MAX_PLAYERS);
    // The one that stopped is not lost: it is held again when a slot frees,
    // and it does not swap back by itself.
    settle_for(&mut harness, &mut fake, 300);
    assert!(!held(&harness).contains(&pname(3)));
    fake.name_owner_changed(&pname(5), ":1.105", "");
    settle_for(&mut harness, &mut fake, 300);
    assert!(held(&harness).contains(&pname(3)));
}

/// The tooltip is on screen only for a module that says it has one
/// (`Module::tooltips`: the bar takes a pointer for tooltips from that alone):
/// a playing player's view carries one, so the module must say so.
#[cfg(feature = "popup")]
#[test]
fn a_playing_players_tooltip_is_reachable() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let view = harness.view_on(Some("DP-1"));
    assert_eq!(view.tooltip(), "mpv (playing): Ada - Song");
    assert!(harness.tooltips(), "a tooltip nobody can hover to");
}

/// A reply skipped unread names no call: the unknown drop frees what
/// expired by age and lets every player waiting on an answer ask again
/// at its next signal, instead of staying stuck till a reap.
#[test]
fn an_unknown_drop_releases_every_player_waiting_on_an_answer() {
    use super::session;
    use crate::dbus::conn::{self, Event};
    use crate::dbus::testdaemon;
    let (client, mut daemon_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut live = session::start(conn::setup(client).unwrap());
    let _daemon = server.join().unwrap();
    // A player mid-read: asked, the answer not yet back.
    let mut player = Player::new(
        1,
        "org.mpris.MediaPlayer2.mpv".to_owned(),
        ":1.9".to_owned(),
    );
    player.asked = Some(Instant::now());
    live.players.push(player);
    live.apply(Event::Dropped {
        token: conn::DROPPED_UNKNOWN,
    });
    assert!(live.players[0].asked.is_none());
    assert!(live.players[0].stale);
}

#[test]
fn config_icons_win_over_the_built_in_vectors_per_state() {
    use crate::icon::Icon;
    let settings = Settings {
        icon_playing: Some(Icon::Glyph('>')),
        icon_paused: Some(Icon::Glyph('=')),
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let view = harness.view();
    assert_eq!(view.text(), "Ada - Song");
    assert_eq!(view.icon(), Some('>'));
    assert!(view.art().is_none(), "the config glyph replaces the vector");
    // Paused: its own glyph, dimmed as before.
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    let view = harness.view();
    assert_eq!(view.text(), "Ada - Song");
    assert_eq!(view.icon(), Some('='));
    assert_eq!(view.class(), Class::Muted);
}

#[test]
fn a_static_config_icon_replaces_both_built_ins() {
    use crate::icon::Icon;
    let settings = Settings {
        icon: Some(Icon::Glyph('M')),
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    assert_eq!(harness.view().icon(), Some('M'));
    assert!(harness.view().art().is_none());
    fake.properties_changed(":1.20", &build::status_changed("Paused"));
    assert!(settle_for(&mut harness, &mut fake, 150));
    // The static icon shows for both states, dimmed while paused.
    assert_eq!(harness.view().icon(), Some('M'));
    assert_eq!(harness.view().class(), Class::Muted);
}

#[test]
fn icon_only_draws_the_icon_with_the_text_in_the_tooltip() {
    use crate::icon::Icon;
    let settings = Settings {
        icon_playing: Some(Icon::Glyph('>')),
        icon_paused: Some(Icon::Glyph('=')),
        show_text: false,
        ..Settings::default()
    };
    let (mut harness, mut fake) = up_with(&settings);
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let view = harness.view();
    assert_eq!(view.text(), "");
    assert_eq!(view.icon(), Some('>'));
    // The tooltip already names the player, the state and the line.
    assert_eq!(view.tooltip(), "mpv (playing): Ada - Song");
}

#[test]
fn without_config_icons_the_built_ins_show_as_before() {
    let (mut harness, mut fake) = up();
    fake.add_player(MPV, ":1.20", playing("Song", "Ada"));
    settle(&mut harness, &mut fake);
    let view = harness.view();
    assert_eq!(view.text(), "Ada - Song");
    assert!(view.icon().is_none() && view.art().is_some());
}
