//! The tray module through the harness: the watcher and host sides
//! against the scripted bus ([`super::fake`]), from registration to
//! activation to a crash, plus what hostile items do (nothing, loudly or
//! not). Every test starts the module on a socketpair, the way the bar
//! starts it on the bus socket.

use std::time::Duration;

use ab_glyph::{FontArc, FontVec};

use super::fake::{self, Fake};
use super::{ID, MAX_ITEMS, Settings, start_connected};
use crate::action::{Action, ModuleAction, Trigger};
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

/// Turns of the loop until `done` holds, or the test fails: what the
/// daemon's loop does with real bus traffic.
fn drive(harness: &mut Harness, mut done: impl FnMut(&Harness) -> bool) {
    let start = std::time::Instant::now();
    while !done(harness) {
        assert!(start.elapsed() < Duration::from_secs(10), "the bus never answered");
        harness.wait(Duration::from_secs(2));
    }
}

/// Turns until the module shows `count` items.
fn until_shown(harness: &mut Harness, count: usize) {
    drive(harness, |harness| {
        harness
            .value_on(None)
            .and_then(|value| value.get("items")?.as_array().map(|items| items.len() == count))
            .unwrap_or(false)
    });
}

/// The item's index action for `name`.
fn action(name: &'static str, index: i32) -> Option<Action> {
    Some(Action::Module(ModuleAction::new(name, Some(index))))
}

#[test]
fn an_item_listed_before_start_is_picked_up() {
    let (stream, fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["watcher"], "owner");
    assert_eq!(value["items"][0]["id"], format!("{SERVICE}/StatusNotifierItem"));
    assert_eq!(value["items"][0]["title"], "Player");
    assert_eq!(value["items"][0]["status"], "Active");
    let view = harness.view();
    assert!(view.text().is_empty());
    assert!(view.tooltip().contains("Player"), "{:?}", view.tooltip());
}

#[test]
fn a_registration_is_answered_and_shown() {
    let (mut harness, mut fake) = started();
    let serial = fake.send_register(OWNER, SERVICE);
    until_shown(&mut harness, 1);
    // The registration was answered: a return quoting the call.
    drive(&mut harness, |_| {
        fake.calls().iter().any(|call| {
            matches!(call.kind, crate::dbus::proto::Kind::MethodReturn) && call.reply_to == Some(serial)
        })
    });
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"][0]["id"], format!("{SERVICE}/StatusNotifierItem"));
}

#[test]
fn a_path_registration_lands_on_the_sender() {
    let (mut harness, mut fake) = started();
    fake.send_register(":1.60", "/Custom/Item");
    until_shown(&mut harness, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"][0]["id"], ":1.60/Custom/Item");
}

#[test]
fn new_icon_re_reads_the_item() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    let before = harness.view().tooltip().to_owned();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player2", "Active", 4, 4, &fake::solid(4, 4, 255, 30, 200, 30)));
    fake.send_item_signal(OWNER, "NewIcon");
    drive(&mut harness, |harness| harness.view().tooltip() != before);
    assert!(harness.view().tooltip().contains("Player2"));
}

#[test]
fn a_crashed_item_disappears_with_its_owner() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    fake.send_name_owner_changed(SERVICE, OWNER, "");
    drive(&mut harness, |harness| harness.value_on(None).is_none());
    assert!(harness.view().is_empty());
}

#[test]
fn an_owner_vanishing_drops_the_item_too() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    // The unique name itself went away (the service row may linger):
    // matched by owner, dropped all the same.
    fake.send_name_owner_changed(OWNER, OWNER, "");
    drive(&mut harness, |harness| harness.value_on(None).is_none());
}

#[test]
fn a_click_activates_and_a_scroll_scrolls() {
    let (stream, fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
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
    assert_eq!(input(Trigger::ScrollUp, PAD + 1), action("scroll-up", 0));
    assert_eq!(input(Trigger::ScrollDown, PAD + 1), action("scroll-down", 0));
    assert_eq!(input(Trigger::RightClick, PAD + 1), None);
    // Past the end, and in the gap, is nothing.
    assert_eq!(input(Trigger::Click, 399), None);

    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(0)), 1),
        Ok(Update::Unchanged)
    );
    let calls = fake.calls();
    let activate = calls.iter().find(|call| call.member == "Activate").expect("no Activate call");
    assert_eq!(activate.destination, SERVICE);
    assert_eq!(activate.path, "/StatusNotifierItem");
    assert_eq!(activate.signature, "ii");
    assert!(activate.serial > 0);

    // A touchpad flood is one bounded call: steps clamp at 64.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("scroll-up", Some(0)), 1000),
        Ok(Update::Unchanged)
    );
    let calls = fake.calls();
    let scroll = calls.iter().find(|call| call.member == "Scroll").expect("no Scroll call");
    assert_eq!(scroll.signature, "is");
    let mut reader = crate::dbus::proto::Reader::le(&scroll.body);
    assert_eq!(reader.i32().unwrap(), 64);
    assert_eq!(reader.str().unwrap(), "vertical");

    // The menu waits on popups: refused loudly, and unknown stays unknown.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Err(InvokeError::Refused("tray menus wait on the popups entry"))
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
    let (stream, fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Red", "Active", 4, 4, &fake::solid(4, 4, 255, 255, 0, 0)));
    // Held directly (not through the harness, which owns opaquely), and
    // woken by hand: the fake answered during the blocking set-up, so a
    // few turns settle it.
    let mut module = start_connected(stream);
    for _ in 0..20 {
        let _ = module.on_ready(0, PollFlags::IN);
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
        pixels.chunks_exact(4).any(|p| p[0] < 16 && p[1] < 16 && p[2] > 200 && p[3] > 200),
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
        harness.wait(Duration::from_secs(2));
    }
    assert!(harness.value_on(None).is_none());
    // A title of controls and 10 KiB of `x`: stripped and cut.
    let (stream, fake) = Fake::pair();
    let mut title = String::from("\u{0}ab\u{7f}");
    title.push_str(&"x".repeat(10_000));
    fake.add_item(SERVICE, OWNER, fake::item_body(&title, "Bogus", 4, 4, &fake::solid(4, 4, 255, 0, 0, 255)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    let value = harness.value_on(None).unwrap();
    let shown = value["items"][0]["title"].as_str().unwrap();
    assert!(!shown.chars().any(char::is_control));
    assert!(shown.len() <= super::MAX_ITEM_TEXT);
    // Unknown statuses are passive, never a refusal.
    assert_eq!(value["items"][0]["status"], "Passive");
    // A pixmap whose bytes do not match its dimensions: the entry is
    // skipped, the item stays, iconless but clickable.
    let (stream, fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Shapeless", "Active", 4, 4, &[1, 2, 3]));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    assert!(harness.value_on(None).is_some());
}

#[test]
fn past_the_item_cap_registrations_are_ignored() {
    let (stream, fake) = Fake::pair();
    for n in 0..MAX_ITEMS + 4 {
        let service = format!("org.kde.StatusNotifierItem-1-{n}");
        fake.add_item(&service, OWNER, fake::item_body("Capped", "Active", 2, 2, &fake::solid(2, 2, 255, 9, 9, 9)));
    }
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, MAX_ITEMS);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["items"].as_array().unwrap().len(), MAX_ITEMS);
}

#[test]
fn host_mode_lists_the_other_watchers_items() {
    let (stream, fake) = Fake::pair();
    let id = format!("{SERVICE}/StatusNotifierItem");
    fake.set_watcher(":1.99", &[&id]);
    fake.add_item(SERVICE, OWNER, fake::item_body("Hosted", "Active", 4, 4, &fake::solid(4, 4, 255, 0, 255, 0)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    let value = harness.value_on(None).unwrap();
    assert_eq!(value["watcher"], "host");
    assert_eq!(value["items"][0]["title"], "Hosted");
    // Our host registration reached the other watcher.
    let hosts = fake.hosts();
    assert!(!hosts.is_empty(), "never registered as a host");
}

#[test]
fn a_lost_watcher_is_taken_back() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(SERVICE, OWNER, fake::item_body("Player", "Active", 4, 4, &fake::solid(4, 4, 255, 200, 30, 30)));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, 1);
    // Another bar takes the name: we become its host...
    fake.send_name_owner_changed("org.kde.StatusNotifierWatcher", fake::MODULE, ":1.99");
    drive(&mut harness, |harness| {
        harness.value_on(None).and_then(|value| value.get("watcher")?.as_str().map(str::to_owned))
            == Some("host".to_owned())
    });
    // ...and when it leaves, we ask for the name back.
    fake.send_name_owner_changed("org.kde.StatusNotifierWatcher", ":1.99", "");
    drive(&mut harness, |_| {
        fake.calls().iter().any(|call| call.member == "RequestName")
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
    let harness = Harness::new(super::start_with(super::BusAddr::Path(path)));
    assert!(harness.source_count() <= 1);
    assert!(harness.view().is_empty());
    assert!(harness.value_on(None).is_none());
    let _ = settings;
}

#[test]
fn the_registry_lists_tray_with_its_actions() {
    let spec = find(ID).expect("the tray module is built");
    assert_eq!(spec.id, ID);
    for name in ["activate", "secondary", "scroll-up", "scroll-down", "menu"] {
        assert!(spec.action(name).is_some(), "no {name} action");
    }
    // The stand-in starts the connected module on the scripted bus, so
    // the contract drives the live path on a machine without any bus.
    let harness = Harness::new(super::stand_in(&crate::modules::Settings::default()));
    assert_eq!(harness.source_count(), 1);
}

#[test]
fn debug_raw_sasl() {
    use std::io::{Read, Write};
    use std::time::Duration;
    let (mut stream, _fake) = Fake::pair();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.write_all(&[0]).unwrap();
    eprintln!("wrote NUL");
    stream.write_all(b"AUTH EXTERNAL\r\n").unwrap();
    eprintln!("wrote AUTH");
    let mut buf = [0u8; 64];
    match stream.read(&mut buf) {
        Ok(n) => eprintln!("read {n}: {:?}", String::from_utf8_lossy(&buf[..n])),
        Err(error) => eprintln!("read failed: {error:?}"),
    }
}
