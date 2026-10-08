//! The tray module through the harness: the watcher and host sides
//! against the scripted bus ([`super::fake`]), from registration to
//! activation to a crash, plus what hostile items do (nothing, loudly or
//! not). Every test starts the module on a socketpair, the way the bar
//! starts it on the bus socket.

use std::time::Duration;

use ab_glyph::{FontArc, FontVec};

use super::fake::{self, Fake};
use super::{ID, MAX_ITEMS, Settings, start_connected, start_with};
use crate::action::{Action, ModuleAction, Trigger};
use crate::dbus::link::Addr;
use crate::density::Scale;
use crate::modules::harness::Harness;
use crate::modules::{ClickCtx, CustomDraw, Input, InvokeError, OutputView, Update, find};
use crate::paint::{Canvas, Span};
use crate::testfont;
use crate::text::Text;
use crate::theme::Theme;

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const EM: f32 = 24.0;
const PAD: u32 = 8;

const SERVICE: &str = "org.kde.StatusNotifierItem-100-1";
const OWNER: &str = ":1.50";

fn text() -> Text {
    let font = FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap());
    Text::new(font)
}

/// The module on a scripted bus, with no items yet.
fn started() -> (Harness, Fake) {
    let (stream, fake) = Fake::pair();
    (Harness::new(start_connected(stream)), fake)
}

/// Turns of the loop until `done` holds, or the test fails: the fake
/// bus served synchronously between the module's own turns, so there is
/// no timing and no thread past the set-up.
fn drive(harness: &mut Harness, fake: &mut Fake, mut done: impl FnMut(&Harness) -> bool) {
    let start = std::time::Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the bus never answered"
        );
        // Short waits: the chain converges turn by turn, and idle turns
        // cost little even on a loaded box.
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
}

/// Turns until the module shows `count` filled-in items: counted and
/// titled, since properties arrive a turn after the registration.
fn until_shown(harness: &mut Harness, fake: &mut Fake, count: usize) {
    drive(harness, fake, |harness| {
        harness
            .value_on(None)
            .and_then(|value| {
                value.get("items")?.as_array().map(|items| {
                    items.len() == count
                        && items.iter().all(|item| {
                            item.get("title")
                                .and_then(|title| title.as_str())
                                .is_some_and(|title| !title.is_empty())
                        })
                })
            })
            .unwrap_or(false)
    });
}

/// The item's index action for `name`.
fn action(name: &'static str, index: i32) -> Option<Action> {
    Some(Action::Module(ModuleAction::new(name, Some(index))))
}

#[test]
fn an_item_listed_before_start_is_picked_up() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["watcher"], "owner");
    assert_eq!(
        value["items"][0]["id"],
        format!("{SERVICE}/StatusNotifierItem")
    );
    assert_eq!(value["items"][0]["title"], "Player");
    assert_eq!(value["items"][0]["status"], "Active");
    let view = harness.view();
    assert!(view.text().is_empty());
    assert!(view.tooltip().contains("Player"), "{:?}", view.tooltip());
}

#[test]
fn a_registration_is_answered_and_shown() {
    let (mut harness, mut fake) = started();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let serial = fake.send_register_to("org.kde.StatusNotifierWatcher", OWNER, SERVICE);
    until_shown(&mut harness, &mut fake, 1);
    // The registration was answered: a return quoting the call. Turns
    // past showing, then the assertion (the closure cannot hold the
    // fake it drives).
    for _ in 0..5 {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
    assert!(
        fake.calls().iter().any(|call| {
            matches!(call.kind, crate::dbus::proto::Kind::MethodReturn)
                && call.reply_to == Some(serial)
        }),
        "no answer quoting {serial}"
    );
    let value = harness.value_on(None).unwrap();
    assert_eq!(
        value["items"][0]["id"],
        format!("{SERVICE}/StatusNotifierItem")
    );
}

#[test]
fn a_path_registration_lands_on_the_sender() {
    let (mut harness, mut fake) = started();
    fake.add_item(
        ":1.60",
        ":1.60",
        fake::item_body(
            "Custom",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    fake.send_register(":1.60", "/Custom/Item");
    until_shown(&mut harness, &mut fake, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"][0]["id"], ":1.60/Custom/Item");
}

#[test]
fn new_icon_re_reads_the_item() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let before = harness.view().tooltip().to_owned();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player2",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 30, 200, 30),
        ),
    );
    fake.send_item_signal(OWNER, "NewIcon");
    drive(&mut harness, &mut fake, |harness| {
        harness.view().tooltip() != before
    });
    assert!(harness.view().tooltip().contains("Player2"));
}

#[test]
fn a_crashed_item_disappears_with_its_owner() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    fake.send_name_owner_changed(SERVICE, OWNER, "");
    drive(&mut harness, &mut fake, |harness| {
        harness.value_on(None).is_none()
    });
    assert!(harness.view().is_empty());
}

#[test]
fn an_owner_vanishing_drops_the_item_too() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    // The unique name itself went away (the service row may linger):
    // matched by owner, dropped all the same.
    fake.send_name_owner_changed(OWNER, OWNER, "");
    drive(&mut harness, &mut fake, |harness| {
        harness.value_on(None).is_none()
    });
}

#[test]
fn a_click_activates_and_a_scroll_scrolls() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let font = text();
    let view = harness.view_on(DP1.name);
    let input = |trigger, x| {
        let ctx = ClickCtx {
            output: DP1,
            x,
            view: &view,
            text: &font,
            em: EM,
            padding: PAD,
            span_width: 400,
            height: 40,
            scale: Scale::Integer(1),
        };
        harness.input(&Input { trigger, at: &ctx })
    };
    assert_eq!(input(Trigger::Click, PAD + 1), action("activate", 0));
    assert_eq!(input(Trigger::MiddleClick, PAD + 1), action("secondary", 0));
    assert_eq!(input(Trigger::ScrollUp, PAD + 1), action("wheel-up", 0));
    assert_eq!(input(Trigger::ScrollDown, PAD + 1), action("wheel-down", 0));
    assert_eq!(input(Trigger::RightClick, PAD + 1), action("menu", 0));
    // Past the end, and in the gap, is nothing.
    assert_eq!(input(Trigger::Click, 399), None);

    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(0)), 1),
        Ok(Update::Unchanged)
    );
    // A queued activation flushes on the next turn, like every call.
    fake.pump();
    harness.wait(Duration::from_millis(200));
    fake.pump();
    let calls = fake.calls();
    let activate = calls
        .iter()
        .find(|call| call.member == "Activate")
        .expect("no Activate call");
    assert_eq!(activate.destination, SERVICE);
    assert_eq!(activate.path, "/StatusNotifierItem");
    assert_eq!(activate.signature, "ii");
    assert!(activate.serial > 0);

    // A touchpad flood is one bounded call: steps clamp at 64.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("wheel-up", Some(0)), 1000),
        Ok(Update::Unchanged)
    );
    fake.pump();
    harness.wait(Duration::from_millis(200));
    fake.pump();
    let calls = fake.calls();
    let scroll = calls
        .iter()
        .find(|call| call.member == "Scroll")
        .expect("no Scroll call");
    assert_eq!(scroll.signature, "is");
    // Up is negative, down positive: what GTK/Ayatana items read (a
    // positive vertical delta is a scroll down to them), as Waybar sends.
    let mut reader = crate::dbus::proto::Reader::le(&scroll.body);
    assert_eq!(reader.i32().unwrap(), -64);
    assert_eq!(reader.str().unwrap(), "vertical");

    // Down is the other sign, not the same call again.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("wheel-down", Some(0)), 3),
        Ok(Update::Unchanged)
    );
    fake.pump();
    harness.wait(Duration::from_millis(200));
    fake.pump();
    let calls = fake.calls();
    let scroll = calls
        .iter()
        .rfind(|call| call.member == "Scroll")
        .expect("no Scroll call");
    assert_eq!(
        crate::dbus::proto::Reader::le(&scroll.body).i32().unwrap(),
        3
    );

    // The menu opens (its layout is still in flight: the fake bus was
    // given no menu to answer with), and unknown stays unknown.
    #[cfg(feature = "popup")]
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    #[cfg(not(feature = "popup"))]
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Err(InvokeError::Refused("tray menus need the popup feature"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("frobnicate", Some(0)), 1),
        Err(InvokeError::Unknown)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", None), 1),
        Err(InvokeError::NeedsArg)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(7)), 1),
        Err(InvokeError::Refused("no such tray item"))
    );
}

#[test]
fn icons_draw_from_the_cache() {
    use rustix::event::PollFlags;
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Red", "Active", 4, 4, &fake::solid(4, 4, 255, 255, 0, 0)),
    );
    // Held directly (not through the harness, which owns opaquely), and
    // woken by hand: the fake answered during the blocking set-up, so a
    // few turns settle it.
    let mut module = start_connected(stream);
    for _ in 0..20 {
        let _ = module.on_ready(0, PollFlags::IN);
        fake.pump();
        let mut view = crate::modules::View::default();
        module.view(&DP1, &mut view);
        if view.tooltip().contains("Red") {
            break;
        }
    }
    let mut view = crate::modules::View::default();
    module.view(&DP1, &mut view);
    assert!(view.tooltip().contains("Red"));
    let mut font = text();
    let theme = Theme::default();
    let baseline = font.metrics(EM).baseline(40);
    let mut pixels = vec![0u8; 40 * 400 * 4];
    let mut canvas = Canvas::new(&mut pixels, 400, 40).unwrap();
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span: Span { x: 0, width: 400 },
        em: EM,
        baseline,
        padding: PAD,
        hovered: false,
        scale: Scale::Integer(1),
        theme: &theme,
    };
    assert!(module.custom_draw(&mut custom));
    // Opaque red, premultiplied: b and g near 0, r and a at max.
    assert!(
        pixels
            .chunks_exact(4)
            .any(|p| p[0] < 16 && p[1] < 16 && p[2] > 200 && p[3] > 200),
        "no red icon ink"
    );
}

#[test]
fn hostile_items_lose_only_themselves() {
    let (mut harness, mut fake) = started();
    // A service name that is not a name, and a path that is not a path:
    // refused with an error, never shown.
    fake.send_register(OWNER, "not a name");
    fake.send_register(OWNER, "/not a path");
    // An item whose properties never arrive (unknown service: the fake
    // errors `NameHasNoOwner`, and the module drops it).
    fake.send_register(OWNER, "org.kde.StatusNotifierItem-9-9");
    for _ in 0..5 {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
    assert!(harness.value_on(None).is_none());
    // A title of controls and 10 KiB of `x`: stripped and cut.
    let (stream, mut fake) = Fake::pair();
    let mut title = String::from("\u{0}ab\u{7f}");
    title.push_str(&"x".repeat(10_000));
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(&title, "Bogus", 4, 4, &fake::solid(4, 4, 255, 0, 0, 255)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let value = harness.value_on(None).unwrap();
    let shown = value["items"][0]["title"].as_str().unwrap();
    assert!(!shown.chars().any(char::is_control));
    assert!(shown.len() <= super::MAX_ITEM_TEXT);
    // Unknown statuses are passive, never a refusal.
    assert_eq!(value["items"][0]["status"], "Passive");
    // A pixmap whose bytes do not match its dimensions: the entry is
    // skipped, the item stays tracked (addressable by index) but takes
    // no room: nothing to draw.
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Shapeless", "Active", 4, 4, &[1, 2, 3]),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    assert!(harness.value_on(None).is_some());
}

#[test]
fn past_the_item_cap_registrations_are_ignored() {
    let (stream, mut fake) = Fake::pair();
    for n in 0..MAX_ITEMS + 4 {
        let service = format!("org.kde.StatusNotifierItem-1-{n}");
        fake.add_item(
            &service,
            OWNER,
            fake::item_body("Capped", "Active", 2, 2, &fake::solid(2, 2, 255, 9, 9, 9)),
        );
    }
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, MAX_ITEMS);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), MAX_ITEMS);
}

#[test]
fn host_mode_lists_the_other_watchers_items() {
    let (stream, mut fake) = Fake::pair();
    let id = format!("{SERVICE}/StatusNotifierItem");
    fake.set_watcher(":1.99", &[&id]);
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Hosted", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 255, 0)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["watcher"], "host");
    assert_eq!(value["items"][0]["title"], "Hosted");
    // Our host registration reached the other watcher.
    let hosts = fake.hosts();
    assert!(!hosts.is_empty(), "never registered as a host");
    // A late registration through the other watcher is picked up too.
    let late = "org.kde.StatusNotifierItem-2-2";
    fake.add_item(
        late,
        ":1.51",
        fake::item_body("Late", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 0, 255)),
    );
    // KDE's watcher announces the id form, `service/path`, not the bare
    // service its registrants said.
    fake.send_watcher_registered(&format!("{late}/StatusNotifierItem"));
    until_shown(&mut harness, &mut fake, 2);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"][1]["title"], "Late");
}

#[test]
fn new_status_arrives_with_the_signal() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    // The item changed state, then said so: the re-read reports it.
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "NeedsAttention",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    fake.send_new_status(OWNER, "NeedsAttention");
    drive(&mut harness, &mut fake, |harness| {
        harness.value_on(None).and_then(|value| {
            value
                .get("items")?
                .as_array()?
                .first()?
                .get("status")?
                .as_str()
                .map(str::to_owned)
        }) == Some("NeedsAttention".to_owned())
    });
}

#[test]
fn a_lost_watcher_is_taken_back() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    // Another bar takes the name: we become its host...
    fake.send_name_owner_changed("org.kde.StatusNotifierWatcher", fake::MODULE, ":1.99");
    drive(&mut harness, &mut fake, |harness| {
        harness
            .value_on(None)
            .and_then(|value| value.get("watcher")?.as_str().map(str::to_owned))
            == Some("host".to_owned())
    });
    // ...and when it leaves, we take the name back: the fake answers
    // primary owner, so the mode returns to owner (a `RequestName` to
    // the bus is answered, never recorded).
    fake.send_name_owner_changed("org.kde.StatusNotifierWatcher", ":1.99", "");
    drive(&mut harness, &mut fake, |harness| {
        harness
            .value_on(None)
            .and_then(|value| value.get("watcher")?.as_str().map(str::to_owned))
            == Some("owner".to_owned())
    });
}

#[test]
fn waiting_without_a_bus_costs_nothing_and_shows_nothing() {
    let settings = Settings::default();
    let path = std::env::temp_dir().join(format!(
        "scootbar-tray-no-bus-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    let harness = Harness::new(super::start_with(super::Addr::Path(path)));
    assert!(harness.source_count() <= 1);
    assert!(harness.view().is_empty());
    assert!(harness.value_on(None).is_none());
    let _ = settings;
}

#[test]
fn the_registry_lists_tray_with_its_actions() {
    let spec = find(ID).expect("the tray module is built");
    assert_eq!(spec.id, ID);
    for name in [
        "activate",
        "secondary",
        "wheel-up",
        "wheel-down",
        "menu",
        "menu-select",
        "menu-drill",
        "menu-back",
    ] {
        assert!(spec.action(name).is_some(), "no {name} action");
    }
    // The stand-in starts the connected module on the scripted bus, so
    // the contract drives the live path on a machine without any bus.
    let harness = Harness::new(super::stand_in(&crate::modules::Settings::default()));
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn the_watcher_object_answers() {
    use crate::dbus::proto::{Kind, Writer};
    let (stream, mut fake) = Fake::pair();
    let mut harness = Harness::new(start_connected(stream));
    drive(&mut harness, &mut fake, |harness| {
        harness.source_count() == 1
    });
    // `Ping` answers empty; every reply quotes the call.
    let serial = fake.send_call(
        fake::MODULE,
        ":1.60",
        "/StatusNotifierWatcher",
        "org.freedesktop.DBus.Peer",
        "Ping",
        "",
        &[],
    );
    for _ in 0..5 {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
    let ping = fake
        .calls()
        .into_iter()
        .find(|call| matches!(call.kind, Kind::MethodReturn) && call.reply_to == Some(serial))
        .expect("no Ping answer");
    assert_eq!(ping.signature, "");
    assert!(ping.body.is_empty());
    // `GetAll` answers the three properties.
    let mut body = Writer::new();
    body.str("org.kde.StatusNotifierWatcher");
    let bytes = body.take_body().unwrap();
    let serial = fake.send_call(
        fake::MODULE,
        ":1.60",
        "/StatusNotifierWatcher",
        "org.freedesktop.DBus.Properties",
        "GetAll",
        "s",
        &bytes,
    );
    for _ in 0..5 {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
    let gotten = fake
        .calls()
        .into_iter()
        .find(|call| matches!(call.kind, Kind::MethodReturn) && call.reply_to == Some(serial))
        .expect("no GetAll answer");
    assert_eq!(gotten.signature, "a{sv}");
    // The body is the bare dictionary (no variant wrapper: the header
    // already says `a{sv}`, and the daemon drops a mismatch with us).
    {
        use crate::dbus::proto::Reader;
        let mut reader = Reader::le(&gotten.body);
        let raw = reader.array_raw(8).expect("a dictionary");
        let mut entries = Reader::le(raw);
        let mut keys = Vec::new();
        while !entries.exhausted() {
            entries.enter_struct().expect("entry");
            keys.push(entries.str().expect("key").to_owned());
            let sig = entries.signature().expect("sig");
            entries.skip(sig).expect("value");
            entries.leave_struct();
        }
        assert!(reader.exhausted());
        assert_eq!(
            keys,
            [
                "RegisteredStatusNotifierItems",
                "IsStatusNotifierHostRegistered",
                "ProtocolVersion"
            ]
        );
    }
    // Unknown members and objects error, never silence.
    let serial = fake.send_call(
        fake::MODULE,
        ":1.60",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
        "Frobnicate",
        "",
        &[],
    );
    let serial2 = fake.send_call(
        fake::MODULE,
        ":1.60",
        "/Nope",
        "org.kde.StatusNotifierWatcher",
        "RegisterStatusNotifierItem",
        "s",
        &bytes,
    );
    // Turns until both errors arrive back (an answer takes a few
    // ready-driven turns each way).
    let start = std::time::Instant::now();
    let mut calls = Vec::new();
    while start.elapsed() < Duration::from_secs(30) {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
        calls = fake.calls();
        if [serial, serial2].iter().all(|wanted| {
            calls
                .iter()
                .any(|call| matches!(call.kind, Kind::Error) && call.reply_to == Some(*wanted))
        }) {
            break;
        }
    }
    for serial in [serial, serial2] {
        assert!(
            calls
                .iter()
                .any(|call| matches!(call.kind, Kind::Error) && call.reply_to == Some(serial)),
            "no error for {serial}"
        );
    }
}

/// A `GetAll` body marshalled by sd-bus, not by this crate's `Writer`
/// (`busctl call ... GetAll a{sv} ...`, captured with `dbus-monitor
/// --binary` on a private `dbus-daemon`; `fixtures/README.md` has the
/// command). D-Bus alignment is fully specified, so this is not a second
/// opinion on the padding: it holds shapes this crate's own tests did not
/// think to write. An unknown property whose variant holds an `a{sv}` of
/// 8-aligned arrays and structs, none of them starting at a multiple of
/// 8, was misread by the reader until this fixture met it; the same body
/// has an `a(iiay)` after a string at an odd offset and the tooltip
/// struct with its own pixmap array.
#[test]
fn a_getall_marshalled_by_sd_bus_is_read_whole() {
    use crate::dbus::proto::Message;
    let frame = include_bytes!("../../dbus/fixtures/sdbus-getall-call.bin");
    let message = Message::parse(frame).unwrap();
    assert_eq!(message.signature, "a{sv}");
    let mut item = super::Item::new(
        "org.example.Fixture/Item".to_owned(),
        "org.example.Fixture".to_owned(),
        "/Item".to_owned(),
        "org.example.Fixture".to_owned(),
    );
    assert!(super::fill(&mut item, message.body.rest()));
    assert_eq!(item.title, "hello");
    assert_eq!(item.status, super::Status::Active);
    assert_eq!(item.tooltip_title, "ttitle");
    assert_eq!(item.tooltip_text, "ttext");
    assert_eq!(item.menu, "/MenuBar");
    assert!(item.item_is_menu);
    // Both pixmap entries of `IconPixmap` (2x2 and 1x1), smallest first.
    let sides: Vec<u32> = item.icons.iter().map(|icon| icon.side()).collect();
    assert_eq!(sides, [1, 2]);
}

/// An item that sends only `IconName` (no `IconPixmap`) draws once the
/// theme lookup resolves it: this is the pasystray case, invisible
/// before the lookup landed. The fixture theme lives in a tmp dir, so
/// no machine theme is touched.
#[test]
fn an_icon_name_only_item_draws_once_the_theme_resolves_it() {
    use crate::dbus::proto::Writer;
    let root = std::env::temp_dir().join(format!("scootbar-tray-fill-{}", std::process::id()));
    let dir = root.join("hicolor/22x22/apps");
    std::fs::create_dir_all(&dir).unwrap();
    {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 4, 4);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[200u8, 30, 30, 255].repeat(16))
                .unwrap();
        }
        std::fs::write(dir.join("pasystray.png"), &bytes).unwrap();
    }
    // Status + Title + IconName, no IconPixmap: the previously invisible
    // shape. On the old code `fill` ignored the name and the item stayed
    // hidden; with the lookup it shows one icon.
    let mut body = Writer::new();
    let cookie = body.open_array(8).unwrap();
    for (key, sig, value) in [
        ("Status", "s", "Active"),
        ("Title", "s", "pasystray"),
        ("IconName", "s", "pasystray"),
    ] {
        assert!(body.open_struct());
        body.str(key);
        body.variant(sig);
        body.str(value);
        body.close_struct();
    }
    body.close_array(cookie);
    let bytes = body.take_body().unwrap();
    let mut item = super::Item::new(
        "org.example.Pasystray/Item".to_owned(),
        "org.example.Pasystray".to_owned(),
        "/Item".to_owned(),
        "org.example.Pasystray".to_owned(),
    );
    assert!(super::item::fill_with(
        &mut item,
        &bytes,
        std::slice::from_ref(&root)
    ));
    assert!(item.shown(), "the themed name resolves to a drawn icon");
    assert_eq!(item.icons.len(), 1);
    assert_eq!(item.icons[0].side(), 4);
    // A name with nothing installed stays tracked-but-hidden, as
    // before: never a panic, never a blank slot.
    let mut body = Writer::new();
    let cookie = body.open_array(8).unwrap();
    for (key, sig, value) in [
        ("Status", "s", "Active"),
        ("Title", "s", "ghost"),
        ("IconName", "s", "no-such-icon"),
    ] {
        assert!(body.open_struct());
        body.str(key);
        body.variant(sig);
        body.str(value);
        body.close_struct();
    }
    body.close_array(cookie);
    let bytes = body.take_body().unwrap();
    let mut missing = super::Item::new(
        "org.example.Ghost/Item".to_owned(),
        "org.example.Ghost".to_owned(),
        "/Item".to_owned(),
        "org.example.Ghost".to_owned(),
    );
    assert!(super::item::fill_with(
        &mut missing,
        &bytes,
        std::slice::from_ref(&root)
    ));
    assert!(!missing.shown());
    assert!(missing.icons.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// `NeedsAttention` prefers the attention name: the same item draws its
/// alarm icon while alarmed and its main one otherwise.
#[test]
fn an_attention_name_wins_while_needs_attention() {
    use crate::dbus::proto::Writer;
    let root = std::env::temp_dir().join(format!("scootbar-tray-attention-{}", std::process::id()));
    let dir = root.join("hicolor/22x22/apps");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, pixel) in [
        ("main", [30u8, 30, 200, 255]),
        ("alarm", [200, 30, 30, 255]),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&pixel.repeat(4)).unwrap();
        }
        std::fs::write(dir.join(format!("{name}.png")), &bytes).unwrap();
    }
    let answer = |status: &str| {
        let mut body = Writer::new();
        let cookie = body.open_array(8).unwrap();
        for (key, sig, value) in [
            ("Status", "s", status),
            ("Title", "s", "item"),
            ("IconName", "s", "main"),
            ("AttentionIconName", "s", "alarm"),
        ] {
            assert!(body.open_struct());
            body.str(key);
            body.variant(sig);
            body.str(value);
            body.close_struct();
        }
        body.close_array(cookie);
        body.take_body().unwrap()
    };
    let mut item = super::Item::new(
        "org.example.Alarm/Item".to_owned(),
        "org.example.Alarm".to_owned(),
        "/Item".to_owned(),
        "org.example.Alarm".to_owned(),
    );
    assert!(super::item::fill_with(
        &mut item,
        &answer("Active"),
        std::slice::from_ref(&root)
    ));
    let calm = item.icons[0].id();
    assert!(super::item::fill_with(
        &mut item,
        &answer("NeedsAttention"),
        std::slice::from_ref(&root)
    ));
    assert_ne!(
        item.icons[0].id(),
        calm,
        "the alarm icon replaces the main one"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Turns of the loop with no condition: lets every queued answer land.
fn settle(harness: &mut Harness, fake: &mut Fake) {
    for _ in 0..6 {
        fake.pump();
        harness.wait(Duration::from_millis(100));
        fake.pump();
    }
}

/// Measured the way the bar measures a span: `span_extra` is what the
/// icons take, 0 when none is drawn.
fn icon_span(harness: &Harness) -> u32 {
    let font = text();
    let view = harness.view_on(DP1.name);
    harness.span_extra(&crate::modules::Measure {
        output: DP1,
        view: &view,
        text: &font,
        em: EM,
        scale: Scale::Integer(1),
        height: 40,
    })
}

/// `Passive` is the spec's "hide me", and an item with no pixmap has
/// nothing to draw: both stay tracked (a `query` lists them, `activate N`
/// reaches them) but take no room in the bar, and a click lands on the
/// items that are drawn, not on a blank slot.
#[test]
fn a_passive_or_pixmapless_item_takes_no_room() {
    let (stream, mut fake) = Fake::pair();
    let pixmap = fake::solid(4, 4, 255, 200, 30, 30);
    // Ids sort by service: Active, then Passive, then without pixmap,
    // then a second Active one.
    fake.add_item(
        "org.kde.StatusNotifierItem-1-1",
        ":1.51",
        fake::item_body("A", "Active", 4, 4, &pixmap),
    );
    fake.add_item(
        "org.kde.StatusNotifierItem-1-2",
        ":1.52",
        fake::item_body("P", "Passive", 4, 4, &pixmap),
    );
    fake.add_item(
        "org.kde.StatusNotifierItem-1-3",
        ":1.53",
        fake::item_body("N", "Active", 4, 4, &[]),
    );
    fake.add_item(
        "org.kde.StatusNotifierItem-1-4",
        ":1.54",
        fake::item_body("B", "Active", 4, 4, &pixmap),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 4);
    let value = harness.value_on(None).unwrap();
    let shown: Vec<bool> = value["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["shown"].as_bool().unwrap())
        .collect();
    assert_eq!(shown, [true, false, false, true]);
    // Two icons' room, not four.
    let side = Text::art_side(EM);
    let gap = (side / 8).max(1);
    assert_eq!(icon_span(&harness), 2 * side + gap);
    // The second drawn slot is the fourth tracked item.
    let font = text();
    let view = harness.view_on(DP1.name);
    let ctx = ClickCtx {
        output: DP1,
        x: PAD + side + gap + 1,
        view: &view,
        text: &font,
        em: EM,
        padding: PAD,
        span_width: 400,
        height: 40,
        scale: Scale::Integer(1),
    };
    assert_eq!(
        harness.input(&Input {
            trigger: Trigger::Click,
            at: &ctx
        }),
        action("activate", 3)
    );
}

/// A signal storm from one item is one `GetAll` in flight at a time, not
/// a call per signal: its slots in the pending table are never the
/// whole table's.
#[test]
fn an_item_that_never_answers_holds_one_call_however_loud() {
    let (stream, mut fake) = Fake::pair();
    fake.add_silent(SERVICE, OWNER);
    let mut harness = Harness::new(start_connected(stream));
    settle(&mut harness, &mut fake);
    for _ in 0..200 {
        fake.send_item_signal(OWNER, "NewIcon");
    }
    settle(&mut harness, &mut fake);
    let asked = fake
        .calls()
        .iter()
        .filter(|call| call.member == "GetAll" && call.destination == SERVICE)
        .count();
    assert_eq!(asked, 1, "a GetAll per signal");
}

/// One service may hold only so many items: a buggy app registering
/// path after path loses its own extras, and another app's item still
/// gets a slot.
#[test]
fn one_service_cannot_fill_every_slot() {
    let (stream, mut fake) = Fake::pair();
    let pixmap = fake::solid(2, 2, 255, 9, 9, 9);
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Greedy", "Active", 2, 2, &pixmap),
    );
    fake.add_item(
        "org.kde.StatusNotifierItem-2-2",
        ":1.60",
        fake::item_body("Other", "Active", 2, 2, &pixmap),
    );
    let mut harness = Harness::new(start_connected(stream));
    for n in 0..MAX_ITEMS * 2 {
        // Registered by the sender's own name, on distinct paths.
        fake.send_register(SERVICE, &format!("/Item{n}"));
    }
    settle(&mut harness, &mut fake);
    until_shown(&mut harness, &mut fake, super::MAX_PER_SERVICE + 1);
    let value = harness.value_on(None).unwrap();
    let ids: Vec<&str> = value["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids.iter().filter(|id| id.starts_with(SERVICE)).count(),
        super::MAX_PER_SERVICE
    );
    assert!(
        ids.iter()
            .any(|id| id.starts_with("org.kde.StatusNotifierItem-2-2"))
    );
}

/// A full tray (the item cap, each with its own icon) draws from the
/// icon cache without evicting it: every frame after the first reads what
/// the first made. A cache that dropped everything at its bound would
/// re-scale thirty icons on every redraw.
#[test]
fn a_full_tray_draws_from_the_cache_without_evicting_it() {
    use rustix::event::PollFlags;
    let (stream, mut fake) = Fake::pair();
    for n in 0..MAX_ITEMS {
        let service = format!("org.kde.StatusNotifierItem-3-{n}");
        // A different color each, so every icon is its own cache entry.
        let shade = n as u8 * 7;
        fake.add_item(
            &service,
            OWNER,
            fake::item_body(
                "Full",
                "Active",
                4,
                4,
                &fake::solid(4, 4, 255, shade, 255 - shade, 40),
            ),
        );
    }
    let mut module = start_connected(stream);
    for _ in 0..40 {
        let _ = module.on_ready(0, PollFlags::IN);
        fake.pump();
    }
    let mut font = text();
    let theme = Theme::default();
    let baseline = font.metrics(EM).baseline(40);
    let (width, height) = (1400u32, 40u32);
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut view = crate::modules::View::default();
    module.view(&DP1, &mut view);
    let frame = |font: &mut Text, pixels: &mut [u8]| {
        let mut canvas = Canvas::new(pixels, width, height).unwrap();
        let mut custom = CustomDraw {
            output: DP1,
            view: &view,
            canvas: &mut canvas,
            text: font,
            span: Span { x: 0, width },
            em: EM,
            baseline,
            padding: PAD,
            hovered: false,
            scale: Scale::Integer(1),
            theme: &theme,
        };
        assert!(module.custom_draw(&mut custom));
    };
    frame(&mut font, &mut pixels);
    let first = font.icons_cached();
    assert_eq!(first.0, MAX_ITEMS, "an entry per item");
    frame(&mut font, &mut pixels);
    assert_eq!(font.icons_cached(), first, "the second frame evicted");
}

/// A bus that takes the bar in and drops it at once, every time (it
/// dislikes something sent): the bar redials a few times and then waits
/// for the socket to be made anew, instead of spinning on it.
#[test]
fn a_bus_that_keeps_dropping_us_is_not_redialled_forever() {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = std::env::temp_dir().join(format!("scootbar-quick-death-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bus");
    let listener = UnixListener::bind(&path).unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    // Accepts, serves the set-up, then drops the connection: forever (the
    // thread ends with the test process; no connection past the guard
    // means it is parked in `accept`).
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            count.fetch_add(1, Ordering::SeqCst);
            crate::dbus::testdaemon::serve_setup(&mut stream);
        }
    });
    let mut harness = Harness::new(start_with_path(path.clone()));
    // Turns until the third dial (a deadline, not a count: a loaded
    // machine is slow, not wrong).
    let start = std::time::Instant::now();
    while accepted.load(Ordering::SeqCst) < 3 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{} dials",
            accepted.load(Ordering::SeqCst)
        );
        harness.wait(Duration::from_millis(20));
    }
    // The latch waits before the fourth: the gap from the third dial is
    // the link's retry wait, never a hot loop (which would dial in
    // milliseconds) and never nothing.
    let latched = std::time::Instant::now();
    while accepted.load(Ordering::SeqCst) < 4 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{} dials",
            accepted.load(Ordering::SeqCst)
        );
        harness.wait(Duration::from_millis(20));
    }
    assert!(
        latched.elapsed() >= Duration::from_millis(200),
        "the fourth dial came {:?} after the third: no wait",
        latched.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn start_with_path(path: std::path::PathBuf) -> Box<dyn crate::modules::Module> {
    start_with(Addr::Path(path))
}

/// What a watcher announces: the id form (KDE's, and ours), and what a
/// registration may say; anything else is ignored, not shown.
#[test]
fn an_announced_registration_is_split_in_every_form_it_comes_in() {
    use super::watcher::split_announced;
    let sender = ":1.99";
    let id = split_announced(sender, "org.kde.StatusNotifierItem-1-1/StatusNotifierItem");
    assert_eq!(
        id,
        Ok((
            "org.kde.StatusNotifierItem-1-1".to_owned(),
            "/StatusNotifierItem".to_owned()
        ))
    );
    assert_eq!(
        split_announced(sender, ":1.42/org/ayatana/Item"),
        Ok((":1.42".to_owned(), "/org/ayatana/Item".to_owned()))
    );
    // A bare service takes the default path; a bare path, the sender.
    assert_eq!(
        split_announced(sender, "org.example.App"),
        Ok((
            "org.example.App".to_owned(),
            "/StatusNotifierItem".to_owned()
        ))
    );
    assert_eq!(
        split_announced(sender, "/Item"),
        Ok((sender.to_owned(), "/Item".to_owned()))
    );
    for hostile in [
        "",
        "a b/Item",
        "org.example.App/it em",
        "no-dot/Item",
        "x//y",
    ] {
        assert!(split_announced(sender, hostile).is_err(), "{hostile:?}");
    }
}

/// An item that announces a change as fast as it is read costs a bounded
/// number of reads: one per refresh gap, not one per signal and answer.
#[test]
fn an_item_that_announces_nonstop_is_read_at_most_every_gap() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Busy", "Active", 4, 4, &fake::solid(4, 4, 255, 9, 9, 9)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    let started = std::time::Instant::now();
    let before = fake.getall_count(SERVICE);
    while started.elapsed() < Duration::from_secs(1) {
        fake.send_item_signal(OWNER, "NewIcon");
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
    let asked = fake.getall_count(SERVICE) - before;
    // 20 a second at the 50 ms gap, with slack for a loaded machine's late
    // wakeups; without the gap this loop reads on every answer (hundreds).
    assert!((5..=30).contains(&asked), "{asked} reads in a second");
}

/// The scripted item's answer at generation `n`: the same icon, a new
/// title, so every re-read moves the view.
fn versioned(n: usize) -> Vec<u8> {
    fake::item_body(
        &format!("V{n}"),
        "Active",
        2,
        2,
        &fake::solid(2, 2, 255, 9, 9, 9),
    )
}

/// Whether the module shows one item titled `want`.
fn title_is(harness: &Harness, want: &str) -> bool {
    harness
        .value_on(None)
        .and_then(|value| {
            value.get("items")?.as_array().and_then(|items| {
                items
                    .first()?
                    .get("title")?
                    .as_str()
                    .map(|title| title == want)
            })
        })
        .unwrap_or(false)
}

/// A runaway item announcing as fast as it is read is redrawn ten times
/// a second at most, not once per re-read: content changes past the
/// first after a quiet spell wait out the draw gap, however many signals
/// arrived in the turn. The 50 ms re-read floor stays the backstop
/// across turns.
#[test]
fn a_runaway_item_is_redrawn_ten_times_a_second_at_most() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, versioned(0));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    // A new generation a turn, as a runaway item announcing at hundreds
    // a second is heard when the bar keeps up: a redraw each, were they
    // not held.
    let mut drawn = 0;
    let mut n = 0;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        n += 1;
        fake.add_item(SERVICE, OWNER, versioned(n));
        fake.send_item_signal(OWNER, "NewIcon");
        fake.pump();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            drawn += 1;
        }
        fake.pump();
    }
    // One a gap: six in 600 ms, a little over for a gap's rounding,
    // never one per floor window.
    assert!((1..=9).contains(&drawn), "{drawn} draws");
    // And the run ends on its last generation, once the held one is drawn.
    drive(&mut harness, &mut fake, |harness| {
        title_is(harness, &format!("V{n}"))
    });
    // The held redraw lands on the draw timer: drain, then nothing is held.
    let drained = std::time::Instant::now();
    while drained.elapsed() < Duration::from_millis(250) {
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
    assert_eq!(harness.source_count(), 1, "no timer once nothing is held");
}

/// A second content change inside the draw gap is held, not drawn and
/// not lost: the answer lands, the turn reports nothing, the draw timer
/// is the module's only extra source, and the timer's turn draws the
/// latest generation.
#[test]
fn a_second_change_inside_the_gap_is_held_and_not_lost() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, versioned(0));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    fake.add_item(SERVICE, OWNER, versioned(1));
    fake.send_item_signal(OWNER, "NewIcon");
    drive(&mut harness, &mut fake, |harness| title_is(harness, "V1"));
    let first = std::time::Instant::now();
    // Inside the gap (and past the re-read floor, through the coalesce
    // timer): the re-read goes out, its answer lands, but no redraw yet.
    fake.add_item(SERVICE, OWNER, versioned(2));
    fake.send_item_signal(OWNER, "NewIcon");
    let reads = fake.getall_count(SERVICE);
    let start = std::time::Instant::now();
    while fake.getall_count(SERVICE) == reads {
        assert!(start.elapsed() < Duration::from_secs(10), "never re-read");
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
    for _ in 0..4 {
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
    if first.elapsed() < super::DRAW_GAP {
        assert!(title_is(&harness, "V2"), "the answer landed");
        assert_eq!(harness.source_count(), 2, "held for the draw timer");
    }
    // The held change is drawn when the timer fires, with the state as
    // it is, and nothing is held after it.
    drive(&mut harness, &mut fake, |harness| title_is(harness, "V2"));
    let start = std::time::Instant::now();
    while harness.source_count() != 1 {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "a timer still held"
        );
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
}

/// The icon is drawn at the output's device pixels at a fractional scale:
/// its ink spans `art_side(em)` on a side, `em` being what the scale made
/// of the font size (1.5 at 16 logical is 24 device pixels).
#[test]
fn an_icon_is_drawn_at_the_device_size_a_fractional_scale_makes() {
    use rustix::event::PollFlags;
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body(
            "Frac",
            "Active",
            32,
            32,
            &fake::solid(32, 32, 255, 255, 0, 0),
        ),
    );
    let mut module = start_connected(stream);
    for _ in 0..20 {
        let _ = module.on_ready(0, PollFlags::IN);
        fake.pump();
    }
    let mut font = text();
    let theme = Theme::default();
    let em = 16.0 * 1.5;
    let side = Text::art_side(em);
    let baseline = font.metrics(em).baseline(60);
    let mut pixels = vec![0u8; 60 * 200 * 4];
    let mut view = crate::modules::View::default();
    module.view(&DP1, &mut view);
    let mut canvas = Canvas::new(&mut pixels, 200, 60).unwrap();
    let mut custom = CustomDraw {
        output: DP1,
        view: &view,
        canvas: &mut canvas,
        text: &mut font,
        span: Span { x: 0, width: 200 },
        em,
        baseline,
        padding: PAD,
        hovered: false,
        scale: Scale::Fractional(180),
        theme: &theme,
    };
    assert!(module.custom_draw(&mut custom));
    let (mut left, mut right, mut top, mut bottom) = (u32::MAX, 0, u32::MAX, 0);
    for (n, p) in pixels.chunks_exact(4).enumerate() {
        if p[3] > 200 {
            let (x, y) = (n as u32 % 200, n as u32 / 200);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
        }
    }
    assert_eq!(right - left + 1, side, "ink width");
    assert_eq!(bottom - top + 1, side, "ink height");
    assert_eq!(left, PAD, "starts at the padding");
}

/// Only the bus says who owns what: a signal that says an item's owner
/// left, from anyone else, is not believed (and neither is one about the
/// watcher name, which would have the bar re-take it).
#[test]
fn a_forged_owner_change_is_not_believed() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Steady", "Active", 4, 4, &fake::solid(4, 4, 255, 9, 9, 9)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    fake.send_name_owner_changed_from(":1.66", SERVICE, OWNER, "");
    fake.send_name_owner_changed_from(":1.66", OWNER, OWNER, "");
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.value_on(None).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The bus itself, saying it, is.
    fake.send_name_owner_changed(SERVICE, OWNER, "");
    settle(&mut harness, &mut fake);
    assert!(harness.value_on(None).is_none());
}

/// In host mode only the watcher hosted against speaks for it: another
/// peer announcing an item (or its leaving) changes nothing.
#[test]
fn a_host_does_not_believe_a_peer_that_is_not_the_watcher() {
    let (stream, mut fake) = Fake::pair();
    let id = format!("{SERVICE}/StatusNotifierItem");
    fake.set_watcher(":1.99", &[&id]);
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Hosted", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 255, 0)),
    );
    let late = "org.kde.StatusNotifierItem-2-2";
    fake.add_item(
        late,
        ":1.51",
        fake::item_body("Late", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 0, 255)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    fake.send_watcher_registered_from(":1.66", &format!("{late}/StatusNotifierItem"));
    settle(&mut harness, &mut fake);
    assert_eq!(
        harness.value_on(None).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fake.send_watcher_registered_from(":1.99", &format!("{late}/StatusNotifierItem"));
    until_shown(&mut harness, &mut fake, 2);
}

/// A bus that refuses the bar the watcher name does not leave the tray
/// dead: it hosts against whoever has it, as when the name is owned
/// elsewhere.
#[test]
fn a_denied_watcher_name_falls_back_to_hosting() {
    let (stream, mut fake) = Fake::pair();
    fake.deny_names();
    let id = format!("{SERVICE}/StatusNotifierItem");
    fake.set_watcher(":1.99", &[&id]);
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body("Hosted", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 255, 0)),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake, 1);
    assert_eq!(harness.value_on(None).unwrap()["watcher"], "host");
}

/// One registrant owning many names is one peer: it gets the per-service
/// share, not the whole tray.
#[test]
fn one_registrant_cannot_fill_the_tray_with_names() {
    let (stream, mut fake) = Fake::pair();
    let pixmap = fake::solid(2, 2, 255, 9, 9, 9);
    let mut harness = Harness::new(start_connected(stream));
    for n in 0..12 {
        let service = format!("org.kde.StatusNotifierItem-8-{n}");
        fake.add_unlisted(
            &service,
            &format!(":1.8{n}"),
            fake::item_body("Many", "Active", 2, 2, &pixmap),
        );
        fake.send_register(":1.77", &service);
    }
    settle(&mut harness, &mut fake);
    settle(&mut harness, &mut fake);
    let count = harness.value_on(None).unwrap()["items"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(count, super::MAX_PER_SERVICE);
}

/// After the bus keeps dropping the bar it tries again on a timer, one
/// dial at a time, so one hostile answer cannot turn the tray off for
/// the session (the latch used to clear only when the socket file was
/// made anew).
#[test]
fn a_bus_that_kept_dropping_us_is_tried_again_later() {
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = std::env::temp_dir().join(format!("scootbar-retry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bus");
    let listener = UnixListener::bind(&path).unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            count.fetch_add(1, Ordering::SeqCst);
            crate::dbus::testdaemon::serve_setup(&mut stream);
        }
    });
    let mut harness = Harness::new(start_with_path(path));
    let start = std::time::Instant::now();
    // The three quick deaths, then (at the retry) a fourth dial.
    while accepted.load(Ordering::SeqCst) < 4 {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "{} dials",
            accepted.load(Ordering::SeqCst)
        );
        harness.wait(Duration::from_millis(50));
    }
    // The fourth dial waited out the link's retry (300 ms in tests):
    // a floor a slow box can only grow, never shrink past.
    assert!(
        start.elapsed() >= Duration::from_millis(200),
        "the fourth dial came after {:?}: no wait",
        start.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A reply skipped unread names no call: the unknown drop frees what
/// expired by age and lets every item waiting on an answer ask again at
/// its next signal, instead of staying stuck till a reap.
#[test]
fn an_unknown_drop_releases_every_item_waiting_on_an_answer() {
    use super::item::Item;
    use super::watcher::{Flight, Op, setup};
    use crate::dbus::conn::{self, Event};
    use crate::dbus::testdaemon;
    let (client, mut daemon_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut live = setup(conn::setup(client).unwrap());
    let _daemon = server.join().unwrap();
    // An item mid-read: a Props flight out, the answer not yet back.
    let id = "org.example.App/StatusNotifierItem".to_owned();
    live.items.push(Item::new(
        id.clone(),
        "org.example.App".to_owned(),
        "/StatusNotifierItem".to_owned(),
        ":1.9".to_owned(),
    ));
    live.items[0].fetching = true;
    live.flights.push(Some(Flight { op: Op::Props(id) }));
    let _ = live.apply(Event::Dropped {
        token: conn::DROPPED_UNKNOWN,
    });
    assert!(!live.items[0].fetching);
    assert!(live.items[0].stale);
}

/// A host that disconnects is no longer a host: its entry is pruned when
/// its owner name goes away, so a later re-registration announces again
/// instead of being swallowed as a duplicate.
#[test]
fn a_host_that_disconnects_is_pruned() {
    use super::WATCHER_KDE;
    use super::watcher::setup;
    use crate::dbus::conn;
    use crate::dbus::proto::Writer;
    use crate::dbus::testdaemon;
    let (client, mut daemon_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut live = setup(conn::setup(client).unwrap());
    let _daemon = server.join().unwrap();
    // A peer registers as host: answered and recorded beside our own.
    let _ = live.on_call(
        ":1.9",
        "/StatusNotifierWatcher",
        WATCHER_KDE,
        "RegisterStatusNotifierHost",
        7,
        "",
        &[],
    );
    assert_eq!(live.hosts.len(), 2);
    // The peer goes away: the bus says its name has no owner anymore.
    let mut body = Writer::new();
    body.str(":1.9");
    body.str(":1.9");
    body.str("");
    let _ = live.on_name_owner_changed("sss", &body.take_body().unwrap());
    assert_eq!(live.hosts.len(), 1);
    assert!(live.hosts.iter().all(|host| host != ":1.9"));
}

/// The per-service cap counts by service OR registrant: a peer
/// squatting another app's service name crowds that name out (kept
/// deliberately: by AND, one peer could hold 8 under every name), while
/// an unrelated name and registrant are unaffected, and the squatter's
/// own further names are refused too.
#[test]
fn the_per_service_cap_counts_service_or_registrant() {
    use super::MAX_PER_SERVICE;
    use super::item::Item;
    use super::watcher::setup;
    use crate::dbus::conn;
    use crate::dbus::testdaemon;
    let (client, mut daemon_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let server = std::thread::spawn(move || {
        testdaemon::serve_setup(&mut daemon_end);
        daemon_end
    });
    let mut live = setup(conn::setup(client).unwrap());
    let _daemon = server.join().unwrap();
    // Eight items under the victim's name, all from one peer.
    for n in 0..MAX_PER_SERVICE {
        live.items.push(Item::new(
            format!("org.victim.App{n}/StatusNotifierItem"),
            "org.victim.App".to_owned(),
            "/StatusNotifierItem".to_owned(),
            ":1.9".to_owned(),
        ));
    }
    // The victim's own next item under its name is crowded out, and so is
    // the squatter's under any other name; an unrelated pair fits.
    assert!(!live.has_room("org.victim.App", ":1.1"));
    assert!(!live.has_room("org.other.App", ":1.9"));
    assert!(live.has_room("org.other.App", ":1.1"));
}
