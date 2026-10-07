//! The tray module: the StatusNotifierItem watcher and host, over the
//! shared D-Bus client.
//!
//! The watcher is core infrastructure of the bar, not just a module: the
//! bar owns the `org.kde.StatusNotifierWatcher` name (and its
//! `org.freedesktop` twin), re-acquires it if it is lost, answers item
//! registrations itself, and picks up items that registered before the
//! bar started (the bus is enumerated at connect). Where another process
//! already owns the name, the bar runs as a host against it instead —
//! items still appear, clicks still work — and takes over when the owner
//! leaves. Item menus (`ContextMenu`, the DBusMenu protocol) open through
//! [popups](../../../../docs/scootbar/backlog/resolved/popups-done.md):
//! a right click (or the `menu` action) reads the item's layout with the
//! DBusMenu client (`GetLayout`, `Event clicked`, `AboutToShow`, the
//! update signals, bounded like the rest) and draws it as popup rows
//! (see [`menu`] for the mapping). A malformed or hostile menu loses
//! only itself, never a panic or a hang, never unbounded allocation.
//!
//! ## States
//!
//! `Waiting` owns an inotify fd on the bus socket's directory and shows
//! nothing: no bus, no items, one watch. `Live` owns the bus socket and
//! shows what registered, in `service + path` order (the KDE watcher's
//! id format). A dead connection drops back to waiting with nothing
//! shown; a bus that never exists costs one watch fd at most, and none
//! where even the watch cannot be armed (a reload starts over, as the
//! volume module's does).
//!
//! ## Events, all asynchronous past connect
//!
//! The set-up (auth, `Hello`, `RequestName`, match rules, enumeration)
//! is a handful of blocking round trips on a local socket, like the
//! volume module's handshake. Past that nothing blocks: `GetAll` answers
//! arrive as pending-call replies on a later turn, `NewIcon` and friends
//! re-read the item then, and `NameOwnerChanged` (tracked centrally, one
//! map for items and watcher names alike) drops vanished items at once —
//! an app that crashes without unregistering disappears with its owner.
//!
//! ## Untrusted bytes, bounded icons
//!
//! Item strings are sanitized once, at parse time (controls stripped,
//! cut like view text). Pixmaps arrive as raw `ARGB32` over the bus:
//! entries past 64 pixels a side are left for the icon cache to scale
//! (at most one kept), at most eight entries an item, each converted to
//! premultiplied once per icon version and drawn from the shared icon
//! cache at the output's real size — icons at exact device pixels,
//! without a per-frame allocation. An item that sends only a themed
//! `IconName` is resolved through the `hicolor` theme lookup instead
//! (PNG only, hostile names and oversized files refused; see [`theme`]),
//! once per answer and likewise cached. A malformed or hostile item loses
//! itself (its reply is dropped, its entry skipped), never the bar.

use std::fmt::Write;
use std::os::fd::AsFd;

use rustix::event::PollFlags;

use super::{
    ActionSpec, ArgKind, Init, Input, InvokeError, Module, OutputView, Sources, Update, View,
};
use crate::action::{ModuleAction, Trigger};
use crate::dbus::conn;
use crate::dbus::link::{Addr, Link};
use crate::dbus::proto::Writer;
use crate::icon::Art;
use crate::text::Text;

mod item;
mod menu;
pub(super) mod theme;
mod watcher;

#[cfg(test)]
use item::{Item, Status, fill};
use watcher::{Live, Mode, setup};

#[cfg(test)]
mod daemon_tests;
#[cfg(all(test, feature = "popup"))]
mod menu_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod fake;

/// The id `--left`, `--center` and `--right` name it by.
pub const ID: &str = "tray";

/// The actions a binding or an agent may name. Every one takes the item
/// index (`activate 0`); a click, middle click or scroll supplies its own.
pub const ACTIONS: &[ActionSpec] = &[
    ActionSpec {
        name: "activate",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "secondary",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "wheel-up",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "wheel-down",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu-select",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu-drill",
        arg: ArgKind::Required,
    },
    ActionSpec {
        name: "menu-back",
        arg: ArgKind::None,
    },
];

/// The watcher's names, object and interfaces: the KDE one apps speak,
/// and the freedesktop twin the spec names. Both are owned when free;
/// the KDE one decides the mode.
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_KDE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_FDO: &str = "org.freedesktop.StatusNotifierWatcher";
/// The item interfaces, KDE first (what items send), then the twin.
const ITEM_KDE: &str = "org.kde.StatusNotifierItem";
const ITEM_FDO: &str = "org.freedesktop.StatusNotifierItem";
/// The item properties interface.
const ITEM_PROPERTIES: &str = "org.freedesktop.DBus.Properties";
/// The default item path, for a registration naming a service only.
const ITEM_DEFAULT_PATH: &str = "/StatusNotifierItem";
/// What the watcher reports as its protocol version (KDE answers 0).
const PROTOCOL_VERSION: u32 = 0;

/// The most items shown: past this a registration is answered and then
/// ignored, said once per newcomer on stderr, never polled again.
pub const MAX_ITEMS: usize = 32;
/// The most items one service may hold of those: a buggy app registering
/// path after path loses its own extras, not everyone else's slots.
const MAX_PER_SERVICE: usize = 8;
/// A call unanswered this long is forgotten when its slot is wanted
/// (no bus times a call out by default: a stock dbus-daemon session.conf
/// and dbus-broker were both measured at minutes without one, so an item
/// that never answers `GetAll` would hold a slot for good).
const FLIGHT_TTL: std::time::Duration = std::time::Duration::from_secs(30);
/// The longest side of a stored pixmap entry, in pixels: entries past it
/// are left for the icon cache to scale from the single smallest kept.
const MAX_STORED_SIDE: u32 = 64;
/// The most pixmap entries converted per item: an animation past this
/// still shows its first entries.
const MAX_STORED_ICONS: usize = 8;
/// Item strings kept this long at most, in bytes: titles and tooltip
/// text are cut like view text, once, at parse time.
const MAX_ITEM_TEXT: usize = 128;
/// An item is read (`GetAll`) no oftener than this: one that announces a
/// change as fast as it is read (a buggy app, a hostile one) costs 20
/// round trips and redraws a second, not a thousand.
const MIN_REFRESH_GAP: std::time::Duration = std::time::Duration::from_millis(50);
/// A scroll's delta past this magnitude is clamped to it: a touchpad
/// flood is one bounded call, never an accumulated one.
const MAX_SCROLL_DELTA: u32 = 64;

/// The module's options: none of its own yet, so the table holds the
/// margin and the interaction keys like every module's.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {}

#[cfg(feature = "tray")]
pub fn init(settings: &super::Settings) -> Init {
    let _ = settings;
    let addr = match conn::bus_path() {
        Ok(path) => Addr::Path(path),
        Err(()) => {
            crate::print::warn(format_args!(
                "scootbar: tray: DBUS_SESSION_BUS_ADDRESS names no filesystem path \
                 (abstract and other transports are not dialled); no tray"
            ));
            Addr::Unusable
        }
    };
    Init::Available(start_with(addr))
}

/// Tests only: the module started as if the probe had found a bus — a
/// scripted one on a socketpair, so the contract test drives the connected
/// path on a machine without any bus. The fake's handle is leaked on
/// purpose (a thread and a socketpair per stand-in): the signature
/// cannot hand a guard back, and the contract calls it once.
#[cfg(test)]
pub(super) fn stand_in(settings: &super::Settings) -> Box<dyn Module> {
    let _ = settings;
    let (stream, fake) = fake::Fake::pair();
    std::mem::forget(fake);
    start_connected(stream)
}

/// Starts the module: the bus session held by the shared link (dial now
/// when the bus is there, wait on its directory when it is not).
/// Always available: a bus may appear at any time, and waiting costs one
/// inotify fd at most.
fn start_with(addr: Addr) -> Box<dyn Module> {
    Box::new(Tray {
        link: Link::start("tray", addr, setup),
    })
}

/// Starts the module on an already-open stream: the fake bus's end.
/// A stream the tests hand over is the only bus there is (a real session
/// bus on the machine running them must not be redialled after a drop),
/// so the link watches a fixed nonexistent path for it.
#[cfg(test)]
fn start_connected(stream: std::os::unix::net::UnixStream) -> Box<dyn Module> {
    start_with(Addr::Stream(stream))
}

/// The module: the bus session, held across the bus coming and going.
struct Tray {
    link: Link<Live>,
}
impl Tray {
    /// The item index `x` (device pixels from the span's left, `padding`
    /// inside) lands on at `em`, or `None` in a gap or past the end.
    fn hit(x: u32, padding: u32, em: f32, count: usize) -> Option<usize> {
        let side = Text::art_side(em) as usize;
        let gap = gap(side);
        let at = (x as usize).saturating_sub(padding as usize);
        let stride = side + gap;
        if stride == 0 {
            return None;
        }
        let index = at / stride;
        if index < count && at % stride < side {
            Some(index)
        } else {
            None
        }
    }
}

/// The gap between neighbouring icons: an eighth of the slot, at least
/// one device pixel.
fn gap(side: usize) -> usize {
    (side / 8).max(1)
}

impl Module for Tray {
    /// Its view carries a tooltip.
    #[cfg(feature = "popup")]
    fn tooltips(&self) -> bool {
        true
    }

    /// The link's fds (the bus socket, or the directory watch), then the
    /// refresh timer while an item waits out its gap. No other timer,
    /// ever: every refresh is bus-driven, and an idle module wakes
    /// nothing.
    fn sources<'fd>(&'fd self, sources: &mut Sources<'_, 'fd>) {
        self.link.watch(&mut |fd, events| {
            sources.add(fd, events);
        });
        if let Some(timer) = self.link.live().and_then(|live| live.coalesce.as_ref()) {
            sources.add(timer.as_fd(), PollFlags::IN);
        }
    }

    fn on_ready(&mut self, source: usize, events: PollFlags) -> Update {
        let own = self.link.source_count();
        if source >= own {
            // The refresh timer, past the link's sources: items that
            // waited out the gap are read again (answered later, through
            // the bus).
            if source == own && self.link.live().is_some_and(|live| live.coalesce.is_some()) {
                if let Some(live) = self.link.live_mut() {
                    live.on_coalesce();
                }
            }
            return Update::Unchanged;
        }
        // The link reports a dropped session as changed (what it held is
        // gone); the tray only moves when icons left with it.
        let had = self.link.live().is_some_and(|live| !live.items.is_empty());
        let changed = self.link.on_ready(source, events, &mut |live, event| {
            live.apply(event) == Update::Changed
        });
        if self.link.live().is_none() {
            if had {
                Update::Changed
            } else {
                Update::Unchanged
            }
        } else if changed {
            Update::Changed
        } else {
            Update::Unchanged
        }
    }

    /// Nothing textual: the icons are drawn by [`Module::custom_draw`]
    /// and sized by [`Module::span_extra`], so an empty view with icons
    /// still takes no text. The tooltip lists the shown titles that fit.
    fn view(&self, _output: &OutputView<'_>, view: &mut View) {
        let Some(live) = self.link.live() else {
            return;
        };
        for item in live.items.iter().filter(|item| item.shown()) {
            let title = if item.title.is_empty() {
                &item.tooltip_title
            } else {
                &item.title
            };
            if title.is_empty() {
                continue;
            }
            let sep = if view.tooltip().is_empty() { "" } else { ", " };
            if view.tooltip().len() + sep.len() + title.len() > super::MAX_TEXT {
                break;
            }
            let _ = write!(view.tooltip_mut(), "{sep}{title}");
        }
    }

    /// The icons' width past the (empty) text: one slot a shown item, and
    /// a gap between neighbours. Zero with no items, so the module hides
    /// like any empty one.
    fn span_extra(&self, measure: &super::Measure<'_>) -> u32 {
        let Some(live) = self.link.live() else {
            return 0;
        };
        let count = live.shown_count();
        if count == 0 {
            return 0;
        }
        let side = Text::art_side(measure.em) as usize;
        (count * side + (count - 1) * gap(side)) as u32
    }

    /// Draws each item's icon at the output's device size, from the
    /// shared icon cache (a `NewIcon` misses once; steady frames hit).
    /// `true`: the loop draws nothing more for this span.
    fn custom_draw(&self, ctx: &mut super::CustomDraw<'_, '_>) -> bool {
        let Some(live) = self.link.live() else {
            return false;
        };
        if live.shown_count() == 0 {
            return false;
        }
        let side = Text::art_side(ctx.em);
        let stride = side as usize + gap(side as usize);
        let top = (i64::from(ctx.canvas.height()) - i64::from(side)) / 2;
        let mut x = i64::from(ctx.span.x) + i64::from(ctx.padding);
        for item in live.items.iter().filter(|item| item.shown()) {
            if let Some(icon) = item.icon_for(side) {
                let art = Art::Tray(icon.clone());
                if let Some(crate::icon::Bitmap::Premultiplied(pixels)) =
                    ctx.text.bitmap(&art, side)
                {
                    let pixels: &[u8] = pixels;
                    let edge = side as usize;
                    for (gy, row) in pixels.chunks_exact(edge * 4).enumerate() {
                        for (gx, pixel) in row.chunks_exact(4).enumerate() {
                            let pixel = [pixel[0], pixel[1], pixel[2], pixel[3]];
                            ctx.canvas.blend_premultiplied(
                                x + gx as i64,
                                top + gy as i64,
                                pixel,
                                ctx.span,
                            );
                        }
                    }
                }
            }
            x += stride as i64;
        }
        true
    }

    /// A click activates, a middle click secondarily, a scroll scrolls —
    /// each on the item under the pointer, with no binding at all. A
    /// right click opens the item's menu (as `menu N` does); a click on
    /// an item that is its own menu does the same instead of activating.
    fn on_input(&self, input: &Input<'_>) -> Option<crate::action::Action> {
        let live = self.link.live()?;
        let slot = Self::hit(
            input.at.x,
            input.at.padding,
            input.at.em,
            live.shown_count(),
        )?;
        let index = live.nth_shown(slot)?;
        let name = match input.trigger {
            Trigger::Click => {
                if live.at(index as i32).is_some_and(|item| item.item_is_menu) {
                    "menu"
                } else {
                    "activate"
                }
            }
            Trigger::MiddleClick => "secondary",
            Trigger::ScrollUp => "wheel-up",
            Trigger::ScrollDown => "wheel-down",
            Trigger::RightClick => "menu",
        };
        Some(crate::action::Action::Module(ModuleAction::new(
            name,
            Some(index as i32),
        )))
    }

    /// Carries out the item actions: `activate`, `secondary` and the two
    /// scrolls call the item (never blocking the bar); `menu` opens the
    /// item's menu in a popup (or calls `ContextMenu` where the item has
    /// no menu to read); the popup's own rows click, drill and back out
    /// through `menu-select`, `menu-drill` and `menu-back`. Every one but
    /// `menu-back` takes a number: the item's index for `menu`, the
    /// row's dbusmenu id for the rows.
    fn invoke(
        &mut self,
        _output: &OutputView<'_>,
        action: &ModuleAction,
        steps: u32,
    ) -> Result<Update, InvokeError> {
        let name = match &*action.name {
            "activate" | "secondary" | "wheel-up" | "wheel-down" | "menu" | "menu-select"
            | "menu-drill" | "menu-back" => &*action.name,
            _ => return Err(InvokeError::Unknown),
        };
        let Some(live) = self.link.live_mut() else {
            return Err(InvokeError::Refused("no bus to call on"));
        };
        if name == "menu-select" {
            let id = action.arg.ok_or(InvokeError::NeedsArg)?;
            return if live.menu_select(id) {
                Ok(Update::Changed)
            } else {
                Err(InvokeError::Refused("no such menu row"))
            };
        }
        if name == "menu-drill" {
            let id = action.arg.ok_or(InvokeError::NeedsArg)?;
            return if live.menu_drill(id) {
                Ok(Update::Changed)
            } else {
                Err(InvokeError::Refused("no such submenu"))
            };
        }
        if name == "menu-back" {
            return if live.menu_back() {
                Ok(Update::Changed)
            } else {
                Err(InvokeError::Refused("no menu is open"))
            };
        }
        let index = action.arg.ok_or(InvokeError::NeedsArg)?;
        let Some(item) = live.at(index) else {
            return Err(InvokeError::Refused("no such tray item"));
        };
        if name == "menu" {
            if item.menu.is_empty() {
                // No menu to read: the item's own fallback, at no
                // position (a bar has no screen coordinates to give).
                let (service, path) = (item.service.clone(), item.path.clone());
                let mut body = Writer::new();
                body.i32(0);
                body.i32(0);
                let Some(bytes) = body.take_body() else {
                    return Err(InvokeError::Refused("the call does not fit"));
                };
                live.fire(&service, &path, ITEM_KDE, "ContextMenu", "ii", &bytes);
                return Ok(Update::Unchanged);
            }
            #[cfg(not(feature = "popup"))]
            return Err(InvokeError::Refused("tray menus need the popup feature"));
            #[cfg(feature = "popup")]
            {
                let id = item.id.clone();
                if live.open_menu(&id) {
                    live.menu_popup = true;
                    return Ok(Update::Changed);
                }
                return Err(InvokeError::Refused("the item is not verified yet"));
            }
        }
        let (service, path) = (item.service.clone(), item.path.clone());
        let mut body = Writer::new();
        let (member, signature) = match name {
            "activate" | "secondary" => {
                body.i32(0);
                body.i32(0);
                if name == "activate" {
                    ("Activate", "ii")
                } else {
                    ("SecondaryActivate", "ii")
                }
            }
            "wheel-up" | "wheel-down" => {
                if steps == 0 {
                    return Ok(Update::Unchanged);
                }
                // The step count, clamped: a touchpad flood is one bounded
                // call. Up is negative, down positive, as GTK/Ayatana
                // items read it (a positive vertical delta is a scroll
                // down to them) and as Waybar sends it; the SNI spec is
                // silent on the sign, and Plasma/Qt use the opposite.
                let delta = steps.min(MAX_SCROLL_DELTA) as i32;
                body.i32(if name == "wheel-up" { -delta } else { delta });
                body.str("vertical");
                ("Scroll", "is")
            }
            // Every other name returned above: unreachable.
            _ => return Err(InvokeError::Unknown),
        };
        let Some(bytes) = body.take_body() else {
            return Err(InvokeError::Refused("the call does not fit"));
        };
        live.fire(&service, &path, ITEM_KDE, member, signature, &bytes);
        Ok(Update::Unchanged)
    }

    /// A click activates with no binding at all.
    fn handles_input(&self) -> bool {
        true
    }

    /// The open menu, drawn as popup rows (see [`menu`]): asked when the
    /// popup opens and whenever the view changes while it is open, so a
    /// layout update re-fills it. `false` with no menu open closes one.
    #[cfg(feature = "popup")]
    fn popup(&mut self, _output: &OutputView<'_>, content: &mut crate::popup::Content) -> bool {
        let Some(menu) = self.link.live().and_then(|live| live.menu.as_ref()) else {
            return false;
        };
        if menu::fill_popup(menu, content) {
            return true;
        }
        // Nothing parsed yet, and the first read still out: a `...`
        // line holds the surface for the layout to re-fill.
        if menu.fetching {
            menu::fill_loading(content);
            return true;
        }
        false
    }

    /// Whether the last `invoke` asked for the popup surface (the `menu`
    /// action): taken once, so every later action does not reopen it.
    #[cfg(feature = "popup")]
    fn wants_popup(&mut self) -> bool {
        self.link
            .live_mut()
            .map(|live| core::mem::take(&mut live.menu_popup))
            .unwrap_or(false)
    }

    /// What `query` reports: the mode and the shown items, or nothing
    /// while nothing is shown.
    fn value(&self, _output: &OutputView<'_>) -> Option<serde_json::Value> {
        let live = self.link.live()?;
        if live.items.is_empty() {
            return None;
        }
        let mode = match live.mode {
            Mode::Owner => "owner",
            Mode::Host => "host",
        };
        Some(serde_json::json!({
            "watcher": mode,
            "items": live.items.iter().map(|item| serde_json::json!({
                "id": item.id,
                "title": item.title,
                "status": item.status.name(),
                "shown": item.shown(),
            })).collect::<Vec<_>>(),
        }))
    }
}
