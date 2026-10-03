//! The media module against a real `dbus-daemon`, with players that are
//! second connections of this client: the path from a name appearing to a
//! track on the bar, a control reaching the player, a crash, and what the
//! daemon's own match filtering keeps from waking the bar at all.
//!
//! (`SCOOTBAR_REQUIRE_DBUS_DAEMON` makes a machine without a daemon fail,
//! not skip.)

use std::time::{Duration, Instant};

use super::{Settings, start_with};
use crate::dbus::conn::{self, Conn, Event};
use crate::dbus::link::Addr;
use crate::dbus::mpris::build::{self, entry};
use crate::dbus::mpris::{PATH, PLAYER};
use crate::dbus::proto::Writer;
use crate::dbus::testdaemon::Daemon;
use crate::modules::harness::Harness;
use crate::modules::{Class, InvokeError, ModuleAction, OutputView, Update};

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const MPV: &str = "org.mpris.MediaPlayer2.mpv";
const VLC: &str = "org.mpris.MediaPlayer2.vlc";

/// A player: a connection that owns `name`, answers `GetAll` from its
/// state, records the controls it is sent, and emits signals on demand.
struct Peer {
    conn: Conn,
    props: Vec<u8>,
    /// `(sender, member)` of the controls received.
    controls: Vec<(String, String)>,
    getalls: usize,
}

impl Peer {
    /// Connects without a name: a stranger.
    fn stranger(daemon: &Daemon) -> Self {
        Self {
            conn: conn::connect(&daemon.path()).expect("the peer connects"),
            props: Vec::new(),
            controls: Vec::new(),
            getalls: 0,
        }
    }

    /// Connects and owns `name`, with `props` as its state.
    fn new(daemon: &Daemon, name: &str, props: Vec<u8>) -> Self {
        let mut peer = Self::stranger(daemon);
        peer.props = props;
        let mut body = Writer::new();
        body.str(name);
        body.u32(0);
        peer.conn
            .call(
                conn::BUS_NAME,
                conn::BUS_PATH,
                conn::BUS_INTERFACE,
                "RequestName",
                "su",
                &body.take_body().unwrap(),
                0,
                1,
            )
            .unwrap();
        let start = Instant::now();
        loop {
            assert!(start.elapsed() < Duration::from_secs(10), "no name");
            let (events, _) = peer.conn.pump();
            if events
                .iter()
                .any(|e| matches!(e, Event::Reply { token: 1, .. }))
            {
                return peer;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Serves what arrived: `GetAll` answered (addressed to its sender,
    /// which the daemon requires), controls recorded.
    fn serve(&mut self) {
        let (events, _) = self.conn.pump();
        for event in events {
            let Event::MethodCall {
                sender,
                member,
                serial,
                ..
            } = event
            else {
                continue;
            };
            if member == "GetAll" {
                self.getalls += 1;
                let props = self.props.clone();
                self.conn.reply_return(&sender, serial, "a{sv}", &props);
            } else {
                self.controls.push((sender, member));
            }
        }
    }

    fn emit(&mut self, body: &[u8]) {
        self.conn.signal(
            PATH,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            "sa{sv}as",
            body,
        );
        let _ = self.conn.pump();
    }

    /// Says the status changed, as a player does: its own state moves with
    /// what it says, so a read after the signal answers the same (a stale
    /// `GetAll` reply landing after the signal would undo it, which no
    /// single-threaded player can do).
    fn status(&mut self, status: &str) {
        if let Ok(props) = crate::dbus::mpris::read_player_props(&self.props) {
            let track = props.metadata.as_ref();
            let title = track.and_then(|m| m.title);
            let artists = track.map(|m| m.artists.clone()).unwrap_or_default();
            self.props = build::get_all(status, title, &artists);
        }
        self.emit(&build::status_changed(status));
    }
}

/// Turns of the module and of every peer until `done` holds, or the test
/// fails after ten seconds.
fn drive(harness: &mut Harness, peers: &mut [&mut Peer], mut done: impl FnMut(&Harness) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "never got there; the module held {:?}, showing {:?}",
            harness.value_on(None),
            harness.view().text()
        );
        for peer in peers.iter_mut() {
            peer.serve();
        }
        let _ = harness.wait(Duration::from_millis(10));
    }
}

fn module(daemon: &Daemon) -> Harness {
    Harness::new(start_with(&Settings::default(), Addr::Path(daemon.path())))
}

fn text(harness: &Harness) -> String {
    harness.view_on(Some("DP-1")).text().to_owned()
}

/// The statuses of the players held, in the order `query` lists them (bus
/// name order is not promised, so sorted by it here).
fn statuses(harness: &Harness) -> Vec<String> {
    let Some(value) = harness.value_on(None) else {
        return Vec::new();
    };
    let mut held: Vec<(String, String)> = value["players"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                p["bus_name"].as_str().unwrap_or("").to_owned(),
                p["status"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    held.sort();
    held.into_iter().map(|(_, status)| status).collect()
}

fn playing(title: &str) -> Vec<u8> {
    build::get_all("Playing", Some(title), &["Ada"])
}

/// A player appears after the bar, plays, changes track, pauses, and is
/// killed without releasing its name: the bar follows each by a signal.
#[test]
fn a_player_appears_plays_changes_track_and_dies() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = module(&daemon);
    // The set-up's calls are answered before the first player exists.
    let _ = harness.wait(Duration::from_millis(100));
    assert!(harness.view().is_empty());
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    assert_eq!(harness.view().class(), Class::Normal);
    // A track change is one signal carrying the value: no read.
    let reads = mpv.getalls;
    mpv.emit(&build::changed(
        PLAYER,
        |w| {
            entry(w, "Metadata", "a{sv}", &|w| {
                build::metadata(w, Some("Second"), &["Bo"])
            })
        },
        &[],
    ));
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Bo - Second");
    assert_eq!(mpv.getalls, reads);
    // A property it only invalidated is read.
    mpv.props = playing("Third");
    mpv.emit(&build::changed(PLAYER, |_| {}, &["Metadata"]));
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Third");
    mpv.status("Paused");
    drive(&mut harness, &mut [&mut mpv], |h| {
        h.view().class() == Class::Muted
    });
    // Killed without a word: the daemon releases its name.
    drop(mpv);
    drive(&mut harness, &mut [], |h| h.view().is_empty());
}

/// A player that was already there is found by listing the bus.
#[test]
fn a_player_there_before_the_bar_is_found_by_listing_the_bus() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Early"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Early");
    assert_eq!(mpv.getalls, 1);
}

/// The controls reach the player shown, from the bar's connection, one
/// call each, wanting no reply.
#[test]
fn the_controls_reach_the_player() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("play-pause", None), 1),
        Ok(Update::Unchanged)
    );
    controls(&mut harness, &mut mpv, 1);
    std::thread::sleep(Duration::from_millis(260));
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("next", None), 1),
        Ok(Update::Unchanged)
    );
    controls(&mut harness, &mut mpv, 2);
    let members: Vec<&str> = mpv.controls.iter().map(|(_, m)| m.as_str()).collect();
    assert_eq!(members, ["PlayPause", "Next"]);
    // Both from the same connection: the bar's.
    assert_eq!(mpv.controls[0].0, mpv.controls[1].0);
    assert!(mpv.controls[0].0.starts_with(":1."));
}

/// Turns until `peer` has been sent `count` controls.
fn controls(harness: &mut Harness, peer: &mut Peer, count: usize) {
    let start = Instant::now();
    while peer.controls.len() < count {
        assert!(start.elapsed() < Duration::from_secs(10), "no control");
        peer.serve();
        let _ = harness.wait(Duration::from_millis(10));
    }
}

/// Two players: the one that most recently started playing is shown, the
/// controls go to it, and when it stops the other takes over.
#[test]
fn two_players_follow_the_most_recently_playing() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(
        &daemon,
        MPV,
        build::get_all("Paused", Some("Mpv song"), &[]),
    );
    let mut vlc = Peer::new(
        &daemon,
        VLC,
        build::get_all("Paused", Some("Vlc song"), &[]),
    );
    let mut harness = module(&daemon);
    // Both read and held, paused: the one first by name is shown.
    drive(&mut harness, &mut [&mut mpv, &mut vlc], |h| {
        statuses(h) == ["paused", "paused"]
    });
    assert_eq!(text(&harness), "Mpv song");
    // Recency is the order the bar hears of it, and signals from two
    // connections have no order between them: wait for each to be heard
    // before the next is said.
    mpv.status("Playing");
    drive(&mut harness, &mut [&mut mpv, &mut vlc], |h| {
        statuses(h) == ["playing", "paused"]
    });
    vlc.status("Playing");
    drive(&mut harness, &mut [&mut mpv, &mut vlc], |h| {
        text(h) == "Vlc song"
    });
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("play-pause", None), 1),
        Ok(Update::Unchanged)
    );
    controls(&mut harness, &mut vlc, 1);
    mpv.serve();
    assert!(mpv.controls.is_empty(), "and mpv was not");
    vlc.status("Stopped");
    drive(&mut harness, &mut [&mut mpv, &mut vlc], |h| {
        text(h) == "Mpv song"
    });
}

/// What the bus's own filtering keeps from the bar: names that are not
/// MPRIS, signals on other objects and interfaces. A module with a quiet
/// bus must not wake, however busy the bus is with other things.
#[test]
fn unrelated_bus_traffic_wakes_nothing() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut harness = module(&daemon);
    // The set-up's replies, a few wakes, then quiet.
    while harness.wait(Duration::from_millis(100)).is_some() {}
    // Apps coming and going on other names.
    let mut noise = Vec::new();
    for i in 0..20 {
        noise.push(Peer::new(
            &daemon,
            &format!("org.example.App{i}"),
            Vec::new(),
        ));
    }
    drop(noise);
    // Signals on other objects, other interfaces, other properties.
    let mut other = Peer::new(&daemon, "org.example.Other", Vec::new());
    for _ in 0..50 {
        other.conn.signal(
            "/org/other",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            "sa{sv}as",
            &build::status_changed("Playing"),
        );
        other.conn.signal(
            PATH,
            "org.mpris.MediaPlayer2.TrackList",
            "TrackAdded",
            "",
            &[],
        );
        other.conn.signal(PATH, PLAYER, "Seeked", "", &[]);
    }
    let _ = other.conn.pump();
    let start = Instant::now();
    let mut woke = 0;
    while start.elapsed() < Duration::from_millis(400) {
        if harness.wait(Duration::from_millis(20)).is_some() {
            woke += 1;
        }
    }
    assert_eq!(woke, 0, "the bar woke for traffic that is not a player's");
    assert!(harness.view().is_empty());
}

/// A peer that owns no player name says the player is paused: the bus
/// delivers it (it matches), and the bar does not believe it.
#[test]
fn a_stranger_cannot_speak_for_a_player() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    let mut stranger = Peer::stranger(&daemon);
    stranger.status("Paused");
    stranger.emit(&build::get_all("Paused", Some("Forged"), &["Eve"]));
    // A signal naming the bus as its sender is not the bus's: the sender
    // header is the daemon's, and it says who sent it.
    let mut body = Writer::new();
    body.str(MPV);
    body.str(mpv.conn.unique());
    body.str("");
    stranger.conn.signal(
        "/org/freedesktop/DBus",
        conn::BUS_INTERFACE,
        "NameOwnerChanged",
        "sss",
        &body.take_body().unwrap(),
    );
    let _ = stranger.conn.pump();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(300) {
        mpv.serve();
        let _ = harness.wait(Duration::from_millis(20));
    }
    assert_eq!(text(&harness), "Ada - Song");
    assert_eq!(harness.view().class(), Class::Normal);
}

/// An owner that never answers `GetAll` costs the bar nothing: it shows
/// nothing, stays responsive, and a player that does answer is shown.
#[test]
fn a_player_that_never_answers_is_ignored_and_the_bar_stays_live() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let _silent = Peer::new(&daemon, VLC, Vec::new());
    let mut harness = module(&daemon);
    let _ = harness.wait(Duration::from_millis(100));
    assert!(harness.view().is_empty());
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
}

/// A player that signals its position constantly (sixty times a second for
/// a while, here as fast as the loop takes them): no redraw, no read.
#[test]
fn a_player_that_reports_its_position_constantly_costs_no_redraw() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    let reads = mpv.getalls;
    let mut changed = 0;
    for i in 0..5_000u64 {
        mpv.emit(&build::changed(
            PLAYER,
            |w| entry(w, "Position", "x", &|w| w.u64(i)),
            &[],
        ));
        if i % 50 == 0 && harness.wait(Duration::from_millis(1)) == Some(Update::Changed) {
            changed += 1;
        }
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        if harness.wait(Duration::from_millis(20)) == Some(Update::Changed) {
            changed += 1;
        }
    }
    assert_eq!(changed, 0);
    mpv.serve();
    assert_eq!(mpv.getalls, reads);
    assert_eq!(text(&harness), "Ada - Song");
}

/// A player that rewrites its title constantly (3,000 times as fast as its
/// connection goes) is drawn about ten times a second, and ends on its last.
#[test]
fn a_player_that_rewrites_its_title_constantly_is_drawn_ten_times_a_second() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    // A quiet spell, so the first change is the first draw of the run.
    let quiet = Instant::now();
    while quiet.elapsed() < Duration::from_millis(150) {
        let _ = harness.wait(Duration::from_millis(10));
    }
    let run = Instant::now();
    let mut drawn = 0;
    for n in 0..3_000 {
        mpv.emit(&build::changed(
            PLAYER,
            |w| {
                entry(w, "Metadata", "a{sv}", &|w| {
                    build::metadata(w, Some(&format!("T{n}")), &["Ada"])
                })
            },
            &[],
        ));
        if n % 20 == 0 && harness.wait(Duration::from_millis(1)) == Some(Update::Changed) {
            drawn += 1;
        }
    }
    let end = Instant::now() + Duration::from_millis(400);
    while Instant::now() < end {
        if harness.wait(Duration::from_millis(10)) == Some(Update::Changed) {
            drawn += 1;
        }
    }
    let ceiling = run.elapsed().as_millis() as usize / 100 + 3;
    assert!(
        (1..=ceiling).contains(&drawn),
        "{drawn} draws in {:?}",
        run.elapsed()
    );
    // The last title is shown once the bar has heard it (the daemon may still
    // hold some of the 3,000): waited for, not assumed.
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - T2999");
}

/// The bus going away empties the module and an action says why.
#[test]
fn the_daemon_going_away_empties_the_module() {
    let Some(daemon) = Daemon::spawn() else {
        return;
    };
    let mut mpv = Peer::new(&daemon, MPV, playing("Song"));
    let mut harness = module(&daemon);
    drive(&mut harness, &mut [&mut mpv], |h| text(h) == "Ada - Song");
    drop(mpv);
    drop(daemon);
    drive(&mut harness, &mut [], |h| h.view().is_empty());
    assert!(matches!(
        harness.invoke(&DP1, &ModuleAction::new("next", None), 1),
        Err(InvokeError::Refused(_))
    ));
}
