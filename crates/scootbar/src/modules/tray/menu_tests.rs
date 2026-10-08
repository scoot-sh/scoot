//! The tray's menus through the harness: opening from `menu` and a
//! right click, rows that click, drill and back out, updates that
//! re-fill, and what hostile menus do (nothing, past closing
//! themselves). Every test starts the module on the scripted bus, like
//! the tray's own tests.

use std::time::Duration;

use ab_glyph::{FontArc, FontVec};

use super::fake::{self, Fake};
use super::start_connected;
use crate::action::{Action, ModuleAction, Trigger};
use crate::dbus::proto::{Reader, Writer};
use crate::density::Scale;
use crate::modules::harness::Harness;
use crate::modules::{ClickCtx, Input, InvokeError, OutputView, Update};
use crate::popup::{Content, Kind};
use crate::testfont;
use crate::text::Text;

const DP1: OutputView<'static> = OutputView { name: Some("DP-1") };
const EM: f32 = 24.0;
const PAD: u32 = 8;

const SERVICE: &str = "org.kde.StatusNotifierItem-100-1";
const OWNER: &str = ":1.50";
const MENU: &str = "/Menu";

fn text() -> Text {
    let font = FontArc::new(FontVec::try_from_vec(testfont::build()).unwrap());
    Text::new(font)
}

/// The module on a scripted bus with one shown item that has a menu.
fn started() -> (Harness, Fake) {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body_menu(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
            MENU,
            false,
        ),
    );
    fake.add_menu(SERVICE, MENU, tree(1, false));
    (Harness::new(start_connected(stream)), fake)
}

/// Turns of the loop until `done` holds, or the test fails.
fn drive(harness: &mut Harness, fake: &mut Fake, mut done: impl FnMut(&mut Harness) -> bool) {
    let start = std::time::Instant::now();
    while !done(&mut *harness) {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the bus never answered"
        );
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
}

/// A few turns with no condition: lets queued signals arrive.
fn turns(harness: &mut Harness, fake: &mut Fake, count: usize) {
    for _ in 0..count {
        fake.pump();
        harness.wait(Duration::from_millis(200));
        fake.pump();
    }
}

/// Turns until the module shows its item (properties arrive a turn after
/// the registration).
fn until_shown(harness: &mut Harness, fake: &mut Fake) {
    drive(harness, fake, |harness| {
        harness
            .value_on(None)
            .and_then(|value| {
                value.get("items")?.as_array().map(|items| {
                    items.len() == 1
                        && items[0].get("title").and_then(|title| title.as_str()) == Some("Player")
                })
            })
            .unwrap_or(false)
    });
}

/// Turns until the module shows two titled items.
fn until_shown_two(harness: &mut Harness, fake: &mut Fake) {
    drive(harness, fake, |harness| {
        harness
            .value_on(None)
            .and_then(|value| {
                value.get("items")?.as_array().map(|items| {
                    items.len() == 2
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

fn label(text: &str) -> impl Fn(&mut Writer) + '_ {
    move |w: &mut Writer| {
        fake::layout_prop(w, "label", "s", &|w| w.str(text));
    }
}

/// The scripted menu: Open, a separator, a disabled Save, a checked
/// Marks, a More submenu (loaded, or lazy when asked), mnemonic and
/// escaped labels, and two radio rows.
fn tree(revision: u32, lazy: bool) -> Vec<u8> {
    fake::layout_reply(revision, &|w| {
        fake::layout_node(w, 0, &|_| {}, &|w| {
            fake::layout_kid(w, &|w| {
                fake::layout_node(w, 10, &label("Open"), &|_| {});
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    11,
                    &|w| {
                        fake::layout_prop(w, "type", "s", &|w| w.str("separator"));
                    },
                    &|_| {},
                );
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    12,
                    &|w| {
                        label("Save")(w);
                        fake::layout_prop(w, "enabled", "b", &|w| w.boolean(false));
                    },
                    &|_| {},
                );
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    13,
                    &|w| {
                        label("Marks")(w);
                        fake::layout_prop(w, "toggle-type", "s", &|w| w.str("checkmark"));
                        fake::layout_prop(w, "toggle-state", "i", &|w| w.i32(1));
                    },
                    &|_| {},
                );
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    14,
                    &|w| {
                        label("More")(w);
                        fake::layout_prop(w, "children-display", "s", &|w| {
                            w.str("submenu");
                        });
                    },
                    &|w| {
                        if !lazy {
                            fake::layout_kid(w, &|w| {
                                fake::layout_node(w, 15, &label("Deep"), &|_| {});
                            });
                        }
                    },
                );
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(w, 16, &label("_File"), &|_| {});
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(w, 17, &label("a__b"), &|_| {});
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    18,
                    &|w| {
                        label("Choice")(w);
                        fake::layout_prop(w, "toggle-type", "s", &|w| w.str("radio"));
                        fake::layout_prop(w, "toggle-state", "i", &|w| w.i32(1));
                    },
                    &|_| {},
                );
            });
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    19,
                    &|w| {
                        label("Other")(w);
                        fake::layout_prop(w, "toggle-type", "s", &|w| w.str("radio"));
                        fake::layout_prop(w, "toggle-state", "i", &|w| w.i32(0));
                    },
                    &|_| {},
                );
            });
        });
    })
}

/// The popup's rows: the text of every widget, buttons named with their
/// action, id and whether they close the popup.
fn rows(harness: &mut Harness) -> Option<Vec<String>> {
    let mut content = Content::default();
    if !harness.popup(&mut content) {
        return None;
    }
    Some(
        content
            .widgets()
            .iter()
            .map(|widget| {
                let label = content.label(widget).to_owned();
                match widget.kind {
                    Kind::Text => format!("text:{label}"),
                    Kind::Button {
                        action,
                        arg,
                        closes,
                        ..
                    } => {
                        format!(
                            "button:{action}:{}:{}:{label}",
                            arg.unwrap_or(-1),
                            closes as u8
                        )
                    }
                    Kind::Slider { .. } => "slider".to_owned(),
                }
            })
            .collect(),
    )
}

/// Turns until the popup shows `want`, or the test fails.
fn until_rows(harness: &mut Harness, fake: &mut Fake, want: &[&str]) {
    let want: Vec<String> = want.iter().map(|row| row.to_string()).collect();
    drive(harness, fake, |harness| {
        rows(harness).as_ref() == Some(&want)
    });
}

fn root_rows() -> Vec<&'static str> {
    vec![
        "button:menu-select:10:1:Open",
        "text:",
        "text:Save",
        "button:menu-select:13:1:[x] Marks",
        "button:menu-drill:14:0:More >",
        "button:menu-select:16:1:File",
        "button:menu-select:17:1:a_b",
        "button:menu-select:18:1:(o) Choice",
        "button:menu-select:19:1:( ) Other",
    ]
}

/// The `Event` calls the module made at the item, in order: the clicked
/// ids.
fn events(fake: &mut Fake) -> Vec<i32> {
    fake.calls()
        .iter()
        .filter(|call| call.member == "Event")
        .map(|call| {
            let mut reader = Reader::le(&call.body);
            let id = reader.i32().unwrap();
            assert_eq!(call.destination, SERVICE);
            assert_eq!(call.path, MENU);
            assert_eq!(call.signature, "isvu");
            assert_eq!(reader.str().unwrap(), "clicked");
            id
        })
        .collect()
}

/// The scripted menu's layout at generation `n`: one row, a new label,
/// so every re-read moves what the popup shows.
fn tree_label(n: usize) -> Vec<u8> {
    fake::layout_reply(n as u32 + 1, &|w| {
        fake::layout_node(w, 0, &|_| {}, &|w| {
            fake::layout_kid(w, &|w| {
                fake::layout_node(w, 10, &label(&format!("Row{n}")), &|_| {});
            });
        });
    })
}

/// A menu flooding `LayoutUpdated` while open is re-filled ten times a
/// second at most, not once per re-read: layout changes past the first
/// wait out the draw gap the same way item icons do.
#[test]
fn a_flooding_menu_is_redrawn_ten_times_a_second_at_most() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // A new label a turn, as a menu flooding updates while open is heard
    // when the bar keeps up: a re-fill each, were they not held.
    let mut drawn = 0;
    let mut n = 0;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        n += 1;
        fake.add_menu(SERVICE, MENU, tree_label(n));
        fake.send_layout_updated(OWNER, MENU, n as u32 + 1);
        fake.pump();
        if harness.wait(Duration::from_millis(5)) == Some(Update::Changed) {
            drawn += 1;
        }
        fake.pump();
    }
    // One a gap: six in 600 ms, a little over for a gap's rounding,
    // never one per floor window.
    assert!((1..=9).contains(&drawn), "{drawn} re-fills");
    // And the run ends on its last label, once the held one is drawn.
    let last = format!("Row{n}");
    drive(&mut harness, &mut fake, |harness| {
        rows(harness)
            .is_some_and(|rows| rows.first().is_some_and(|row| row.ends_with(last.as_str())))
    });
    // The held re-fill lands on the draw timer: drain, then nothing is held.
    let drained = std::time::Instant::now();
    while drained.elapsed() < Duration::from_millis(250) {
        fake.pump();
        harness.wait(Duration::from_millis(5));
        fake.pump();
    }
    assert_eq!(harness.source_count(), 1, "no timer once nothing is held");
}

#[test]
fn a_menu_opens_from_invoke_and_a_row_click_sends_clicked() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    // The open asked for the popup surface, once.
    assert!(harness.wants_popup());
    assert!(!harness.wants_popup());
    // `AboutToShow` went out with the first `GetLayout`: the module's
    // queued calls flush on its own turn. (An answered `GetLayout` is
    // not recorded, like an answered `GetAll`: the count says it went.)
    turns(&mut harness, &mut fake, 1);
    let calls = fake.calls();
    assert!(calls.iter().any(|call| call.member == "AboutToShow"));
    assert!(fake.layout_count(SERVICE, MENU) >= 1);
    until_rows(&mut harness, &mut fake, &root_rows());
    // A row click reaches the item as `Event clicked` and closes the
    // menu: the popup is gone with it.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(10)), 1),
        Ok(Update::Changed)
    );
    // The click flushes on the next turn, like every call.
    turns(&mut harness, &mut fake, 1);
    assert_eq!(events(&mut fake), [10]);
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
}

#[test]
fn a_right_click_and_an_item_is_menu_click_open_menus() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body_menu(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
            MENU,
            false,
        ),
    );
    fake.add_menu(SERVICE, MENU, tree(1, false));
    fake.add_item(
        "org.kde.StatusNotifierItem-100-2",
        ":1.51",
        fake::item_body_menu(
            "Menu",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 30, 200, 30),
            MENU,
            true,
        ),
    );
    fake.add_menu("org.kde.StatusNotifierItem-100-2", MENU, tree(1, false));
    let mut harness = Harness::new(start_connected(stream));
    until_shown_two(&mut harness, &mut fake);
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
    let action =
        |name: &'static str, index: i32| Some(Action::Module(ModuleAction::new(name, Some(index))));
    // A right click opens the menu; a click on an item that is its own
    // menu does the same instead of activating.
    assert_eq!(input(Trigger::RightClick, PAD + 1), action("menu", 0));
    assert_eq!(input(Trigger::Click, PAD + 1), action("activate", 0));
    // Past the first icon: the second item. Icons are 24-device-pixel
    // squares (`art_side` at this em) with an eighth-pixel gap.
    let side = crate::text::Text::art_side(EM);
    let stride = side + side / 8;
    assert_eq!(input(Trigger::Click, PAD + stride + 1), action("menu", 1));
}

#[test]
fn a_submenu_drills_in_and_backs_out() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // Into the loaded submenu: a back row over its rows.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(14)), 1),
        Ok(Update::Changed)
    );
    until_rows(
        &mut harness,
        &mut fake,
        &[
            "button:menu-back:-1:0:< Back",
            "button:menu-select:15:1:Deep",
        ],
    );
    // Clicking through: the submenu row's own id reaches the item.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(15)), 1),
        Ok(Update::Changed)
    );
    turns(&mut harness, &mut fake, 1);
    assert_eq!(events(&mut fake), [15]);
    // Again, then back out: the root, then no menu at all.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(14)), 1),
        Ok(Update::Changed)
    );
    until_rows(
        &mut harness,
        &mut fake,
        &[
            "button:menu-back:-1:0:< Back",
            "button:menu-select:15:1:Deep",
        ],
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-back", None), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-back", None), 1),
        Ok(Update::Changed)
    );
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
}

#[test]
fn a_level_past_the_widget_cap_drops_its_extras() {
    // 20 rows a level where the popup holds 16 widgets: the root shows
    // 16, a submenu Back plus 15, and the rest have no row to click.
    // Nesting keeps every *level* addressable, not every row: a future
    // change to this cut must update this test deliberately.
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body_menu(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
            MENU,
            false,
        ),
    );
    fake.add_menu(SERVICE, MENU, wide_submenu_tree(1, 20));
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    // The root: the submenu row, then the first 15 leaves (16
    // widgets); leaves 25..29 are drawn nowhere.
    let mut want = vec!["button:menu-drill:100:0:More >".to_owned()];
    for id in 10..25 {
        want.push(format!("button:menu-select:{id}:1:Row"));
    }
    let want_ref: Vec<&str> = want.iter().map(String::as_str).collect();
    until_rows(&mut harness, &mut fake, &want_ref);
    // Into the submenu: Back plus its first 15 rows; 215..219 are
    // drawn nowhere.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(100)), 1),
        Ok(Update::Changed)
    );
    let mut want = vec!["button:menu-back:-1:0:< Back".to_owned()];
    for id in 200..215 {
        want.push(format!("button:menu-select:{id}:1:Row"));
    }
    let want_ref: Vec<&str> = want.iter().map(String::as_str).collect();
    until_rows(&mut harness, &mut fake, &want_ref);
}

#[test]
fn a_lazy_submenu_is_asked_for_when_drilled_into() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body_menu(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
            MENU,
            false,
        ),
    );
    fake.add_menu(SERVICE, MENU, tree(1, true));
    // The submenu's own answer, asked for by parent id.
    fake.add_menu_at(
        SERVICE,
        MENU,
        14,
        fake::layout_reply(2, &|w| {
            fake::layout_node(w, 14, &|_| {}, &|w| {
                fake::layout_kid(w, &|w| {
                    fake::layout_node(w, 15, &label("Deep"), &|_| {});
                });
            });
        }),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // No children arrived, so drilling asks: `AboutToShow(14)` and
    // `GetLayout(14)`.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(14)), 1),
        Ok(Update::Changed)
    );
    turns(&mut harness, &mut fake, 2);
    let parents: Vec<i32> = fake
        .layout_calls()
        .into_iter()
        .map(|(_, _, parent)| parent)
        .collect();
    assert!(parents.contains(&14), "{parents:?}");
    // The children filled in place: the drill shows them now.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(14)), 1),
        Ok(Update::Changed)
    );
    until_rows(
        &mut harness,
        &mut fake,
        &[
            "button:menu-back:-1:0:< Back",
            "button:menu-select:15:1:Deep",
        ],
    );
}

#[test]
fn a_layout_update_refills_the_open_menu() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    assert_eq!(fake.layout_count(SERVICE, MENU), 1);
    // The item renames a row: the popup re-fills from the re-read layout.
    fake.add_menu(
        SERVICE,
        MENU,
        fake::layout_reply(2, &|w| {
            fake::layout_node(w, 0, &|_| {}, &|w| {
                fake::layout_kid(w, &|w| {
                    fake::layout_node(w, 10, &label("Eject"), &|_| {});
                });
            });
        }),
    );
    fake.send_layout_updated(OWNER, MENU, 2);
    until_rows(&mut harness, &mut fake, &["button:menu-select:10:1:Eject"]);
    assert_eq!(fake.layout_count(SERVICE, MENU), 2);
    // `ItemsPropertiesUpdated` re-reads the same way.
    fake.send_items_updated(OWNER, MENU);
    turns(&mut harness, &mut fake, 4);
    assert!(fake.layout_count(SERVICE, MENU) >= 3);
}

#[test]
fn update_signals_from_anyone_else_are_ignored() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // Another peer's update names the same menu: not the owner, nothing.
    fake.send_layout_updated(":9.99", MENU, 2);
    fake.send_items_updated(":9.99", MENU);
    turns(&mut harness, &mut fake, 4);
    assert_eq!(fake.layout_count(SERVICE, MENU), 1);
    until_rows(&mut harness, &mut fake, &root_rows());
}

#[test]
fn a_hostile_menu_loses_only_itself() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    // Deeper than the bound: the menu closes, the item stays shown.
    fake.add_menu(SERVICE, MENU, deep_tree(1, 12));
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
    assert!(events(&mut fake).is_empty());
    // Wider than the cap: the same.
    fake.add_menu(SERVICE, MENU, wide_tree(1, 70));
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
    // Truncated bytes: the same.
    let mut cut = tree(1, false);
    cut.truncate(cut.len() / 2);
    fake.add_menu(SERVICE, MENU, cut);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
    // The bar is alive: the item still shown, clicks still work.
    assert_eq!(
        harness
            .value_on(None)
            .and_then(|value| value["items"][0]["title"].as_str().map(str::to_owned)),
        Some("Player".to_owned())
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("activate", Some(0)), 1),
        Ok(Update::Unchanged)
    );
}

/// A tree nested `depth` past the root.
fn deep_tree(revision: u32, depth: usize) -> Vec<u8> {
    fn at(w: &mut Writer, left: usize) {
        if left == 0 {
            fake::layout_node(w, 1, &label("Leaf"), &|_| {});
        } else {
            fake::layout_node(w, 1, &label("Level"), &|w| {
                fake::layout_kid(w, &|w| at(w, left - 1));
            });
        }
    }
    fake::layout_reply(revision, &|w| {
        fake::layout_node(w, 0, &|_| {}, &|w| {
            at(w, depth);
        });
    })
}

/// A root with `kids` sibling rows.
fn wide_tree(revision: u32, kids: usize) -> Vec<u8> {
    fake::layout_reply(revision, &|w| {
        fake::layout_node(w, 0, &|_| {}, &|w| {
            for id in 0..kids {
                fake::layout_kid(w, &|w| {
                    fake::layout_node(w, id as i32, &label("Row"), &|_| {});
                });
            }
        });
    })
}

/// A submenu row first, then `kids` leaf rows at the root, with `kids`
/// leaf rows inside the submenu: both levels run past the popup's
/// widget cap, so both are cut.
fn wide_submenu_tree(revision: u32, kids: usize) -> Vec<u8> {
    fake::layout_reply(revision, &|w| {
        fake::layout_node(w, 0, &|_| {}, &|w| {
            fake::layout_kid(w, &|w| {
                fake::layout_node(
                    w,
                    100,
                    &|w| {
                        label("More")(w);
                        fake::layout_prop(w, "children-display", "s", &|w| {
                            w.str("submenu");
                        });
                    },
                    &|w| {
                        for id in 0..kids {
                            fake::layout_kid(w, &|w| {
                                fake::layout_node(w, 200 + id as i32, &label("Row"), &|_| {});
                            });
                        }
                    },
                );
            });
            for id in 0..kids {
                fake::layout_kid(w, &|w| {
                    fake::layout_node(w, 10 + id as i32, &label("Row"), &|_| {});
                });
            }
        });
    })
}

#[test]
fn a_flooding_menu_is_read_behind_the_floor() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // Ten updates at once: the first re-reads, the rest wait out the
    // floor on the shared timer.
    for revision in 2..12 {
        fake.send_layout_updated(OWNER, MENU, revision);
    }
    drive(&mut harness, &mut fake, |harness| rows(harness).is_some());
    turns(&mut harness, &mut fake, 5);
    assert!(
        fake.layout_count(SERVICE, MENU) <= 3,
        "reads: {}",
        fake.layout_count(SERVICE, MENU)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
}

#[test]
fn an_item_without_a_menu_falls_back_to_context_menu() {
    let (stream, mut fake) = Fake::pair();
    fake.add_item(
        SERVICE,
        OWNER,
        fake::item_body_menu(
            "Player",
            "Active",
            4,
            4,
            &fake::solid(4, 4, 255, 200, 30, 30),
            "",
            false,
        ),
    );
    let mut harness = Harness::new(start_connected(stream));
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Unchanged)
    );
    // No popup asked for, no layout read: the fallback went out on the
    // next turn.
    assert!(!harness.wants_popup());
    turns(&mut harness, &mut fake, 1);
    let calls = fake.calls();
    let fallback = calls
        .iter()
        .find(|call| call.member == "ContextMenu")
        .expect("no ContextMenu call");
    assert_eq!(fallback.destination, SERVICE);
    assert_eq!(fallback.path, "/StatusNotifierItem");
    assert_eq!(fallback.signature, "ii");
    assert!(!calls.iter().any(|call| call.member == "GetLayout"));
    let mut content = Content::default();
    assert!(!harness.popup(&mut content));
}

#[test]
fn menu_actions_refuse_loudly() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    // No such item; nothing open; nothing to drill or back out of.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(7)), 1),
        Err(InvokeError::Refused("no such tray item"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(10)), 1),
        Err(InvokeError::Refused("no such menu row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(14)), 1),
        Err(InvokeError::Refused("no such submenu"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-back", None), 1),
        Err(InvokeError::Refused("no menu is open"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", None), 1),
        Err(InvokeError::NeedsArg)
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", None), 1),
        Err(InvokeError::NeedsArg)
    );
    // Open, then refuse what is not a live row: the separator, the
    // disabled row, a drill of a leaf, a click on a submenu parent.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(11)), 1),
        Err(InvokeError::Refused("no such menu row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(12)), 1),
        Err(InvokeError::Refused("no such menu row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(14)), 1),
        Err(InvokeError::Refused("no such menu row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(400)), 1),
        Err(InvokeError::Refused("no such menu row"))
    );
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-drill", Some(10)), 1),
        Err(InvokeError::Refused("no such submenu"))
    );
    assert_eq!(events(&mut fake), Vec::<i32>::new());
}

#[test]
fn a_menu_follows_its_item_across_a_name_change() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // The well-known name changes hands: the open menu follows the new
    // owner (one re-read), and only the new owner's updates move it.
    fake.send_name_owner_changed(SERVICE, OWNER, ":1.60");
    drive(&mut harness, &mut fake, |harness| rows(harness).is_some());
    turns(&mut harness, &mut fake, 3);
    assert_eq!(fake.layout_count(SERVICE, MENU), 2);
    fake.send_layout_updated(OWNER, MENU, 2);
    turns(&mut harness, &mut fake, 2);
    assert_eq!(fake.layout_count(SERVICE, MENU), 2);
    fake.send_layout_updated(":1.60", MENU, 2);
    drive(&mut harness, &mut fake, |harness| rows(harness).is_some());
    turns(&mut harness, &mut fake, 3);
    assert_eq!(fake.layout_count(SERVICE, MENU), 3);
    until_rows(&mut harness, &mut fake, &root_rows());
}

#[test]
fn an_item_vanishing_closes_its_menu() {
    let (mut harness, mut fake) = started();
    until_shown(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &root_rows());
    // The owner is gone without unregistering: the item and its menu go.
    fake.send_name_owner_changed(SERVICE, OWNER, "");
    drive(&mut harness, &mut fake, |harness| rows(harness).is_none());
}

#[test]
fn opening_a_second_menu_replaces_the_first() {
    let (stream, mut fake) = Fake::pair();
    for (n, owner) in [(1, OWNER), (2, ":1.51")] {
        let service = format!("org.kde.StatusNotifierItem-100-{n}");
        fake.add_item(
            &service,
            owner,
            fake::item_body_menu(
                if n == 1 { "Player" } else { "Other" },
                "Active",
                4,
                4,
                &fake::solid(4, 4, 255, 200, 30, 30),
                MENU,
                false,
            ),
        );
        fake.add_menu(
            &service,
            MENU,
            fake::layout_reply(n, &|w| {
                fake::layout_node(w, 0, &|_| {}, &|w| {
                    fake::layout_kid(w, &|w| {
                        let name = if n == 1 { "Open" } else { "Quit" };
                        fake::layout_node(w, 10, &label(name), &|_| {});
                    });
                });
            }),
        );
    }
    let mut harness = Harness::new(start_connected(stream));
    until_shown_two(&mut harness, &mut fake);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(0)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &["button:menu-select:10:1:Open"]);
    // The second menu replaces the first: one layout in flight, one
    // popup, and a click reaches the second item.
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu", Some(1)), 1),
        Ok(Update::Changed)
    );
    until_rows(&mut harness, &mut fake, &["button:menu-select:10:1:Quit"]);
    assert_eq!(
        harness.invoke(&DP1, &ModuleAction::new("menu-select", Some(10)), 1),
        Ok(Update::Changed)
    );
    turns(&mut harness, &mut fake, 1);
    let calls = fake.calls();
    let quit = calls
        .iter()
        .rev()
        .find(|call| call.member == "Event")
        .expect("no Event call")
        .destination
        .clone();
    assert_eq!(quit, "org.kde.StatusNotifierItem-100-2");
}
