//! One item's DBusMenu (`com.canonical.dbusmenu`): the open menu's
//! state, the calls that move it, and its mapping onto popup content.
//!
//! A menu is transient: it opens from `menu N` or a right click
//! ([`super::Tray::invoke`]), shows what the last `GetLayout` said, and
//! is gone on a row's click (which sends `Event clicked`), a back-out
//! past the root, the item vanishing, or a first load that never parses.
//! A later answer that never parses keeps the last state: a hostile item
//! loses its updates, not the bar, and never a panic or a hang.
//!
//! ## Mapping
//!
//! The popup has text rows and buttons, no submenu, checkmark or divider
//! widget ([`crate::popup`]), so the mapping is text-only:
//!
//! - a submenu row is a button ending in ` >` (drills in, with a
//!   `< Back` row on top of every level past the root: nesting in place,
//!   not flattened, so every level of a deep tree stays addressable; a
//!   level longer than the popup holds is still cut like any module's
//!   ([`fill_popup`]): the extras are dropped silently and have no row
//!   to click);
//! - a toggle's state is an ASCII prefix (`[x] `/`[ ] ` for a checkmark,
//!   `(o) `/`( ) ` for a radio): always in the font, unlike a glyph;
//! - a separator is an empty text row (a gap between groups, with no
//!   font bet on a box-drawing run);
//! - a disabled row is plain text (non-interactive, as it is);
//! - icons in menu items are not drawn in this version (said in
//!   `docs/scootbar/cli.md`).

use std::borrow::Cow;
use std::time::Instant;

use super::item::clean;
use crate::dbus::proto::{MAX_MENU_DEPTH, MenuItem, MenuToggle, Writer};
#[cfg(feature = "popup")]
use crate::popup::Content;

/// The DBusMenu interface, the item's `Menu` path's.
pub(super) const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// The row `Event`'s data: a variant no item reads for `clicked` (every
/// implementation takes the id and the name and ignores the rest), so an
/// `i32` zero — parseable by any variant reader.
const EVENT_DATA_SIG: &str = "i";

/// The properties a `GetLayout` asks for: what the mapping reads, and
/// `icon-name` (walked for shape, not drawn: icons in menu items wait for
/// a later version).
const LAYOUT_PROPS: [&str; 8] = [
    "type",
    "label",
    "enabled",
    "visible",
    "toggle-type",
    "toggle-state",
    "children-display",
    "icon-name",
];

/// The button actions the rows carry: what [`super::Tray::invoke`]
/// answers. The popup performs them like any binding, so an agent's
/// `invoke tray menu-select N` and a row click cannot diverge.
#[cfg(feature = "popup")]
pub(super) const SELECT: &str = "menu-select";
#[cfg(feature = "popup")]
pub(super) const DRILL: &str = "menu-drill";
#[cfg(feature = "popup")]
pub(super) const BACK: &str = "menu-back";

/// One menu row, owned: a [`MenuItem`] cleaned (labels through
/// [`clean`], invisible rows dropped at fill time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MenuNode {
    pub(super) id: i32,
    // Drawn only: without the popup nothing reads them.
    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
    pub(super) label: String,
    pub(super) enabled: bool,
    pub(super) separator: bool,
    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
    pub(super) toggle: MenuToggle,
    #[cfg_attr(not(feature = "popup"), allow(dead_code))]
    pub(super) toggle_on: bool,
    pub(super) submenu: bool,
    pub(super) children: Vec<MenuNode>,
}

impl MenuNode {
    /// Cleans one parsed item: label sanitized like item text, a
    /// `-1` (indeterminate) toggle state reading as off. `None` for an
    /// invisible row (never drawn, never counted).
    fn of(item: &MenuItem<'_>) -> Option<Self> {
        if !item.visible {
            return None;
        }
        Some(Self {
            id: item.id,
            label: owned_label(item.label),
            enabled: item.enabled,
            separator: item.separator,
            toggle: item.toggle,
            toggle_on: item.toggle_state == 1,
            submenu: item.submenu,
            children: item.children.iter().filter_map(Self::of).collect(),
        })
    }

    /// Whether the row opens a level: asked for submenu display, or
    /// holding children that arrived.
    pub(super) fn opens(&self) -> bool {
        self.submenu || !self.children.is_empty()
    }
}

/// A cleaned label: mnemonic markers stripped (a lone `_` marks the
/// accelerator and is not drawn, `__` is a literal underscore), through
/// [`clean`] like item text.
fn owned_label(label: Option<&str>) -> String {
    clean(&unmnemonic(label.unwrap_or("")))
}

/// Strips one level of mnemonic marking: `__` to `_`, a lone `_` away.
fn unmnemonic(label: &str) -> Cow<'_, str> {
    if !label.contains('_') {
        return Cow::Borrowed(label);
    }
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(char) = chars.next() {
        if char != '_' {
            out.push(char);
            continue;
        }
        match chars.next() {
            Some('_') => out.push('_'),
            Some(next) => {
                out.push(next);
            }
            None => {}
        }
    }
    Cow::Owned(out)
}

/// The toggle's text prefix, or none for a plain row.
#[cfg(feature = "popup")]
fn toggle_prefix(node: &MenuNode) -> &'static str {
    match node.toggle {
        MenuToggle::Check => {
            if node.toggle_on {
                "[x] "
            } else {
                "[ ] "
            }
        }
        MenuToggle::Radio => {
            if node.toggle_on {
                "(o) "
            } else {
                "( ) "
            }
        }
        MenuToggle::None => "",
    }
}

/// The open menu: whose, at what revision, and the drill trail (dbusmenu
/// ids from the root; empty is the root level).
pub(super) struct MenuOpen {
    /// The item's id (`service + path`, the watcher's format).
    pub(super) item: String,
    /// Who `GetLayout` went to (the item's service).
    pub(super) service: String,
    /// The menu object path (the item's `Menu`).
    pub(super) path: String,
    /// The item's owner when the menu opened: update signals are
    /// accepted only from this sender.
    pub(super) owner: String,
    pub(super) revision: u32,
    pub(super) root: Vec<MenuNode>,
    pub(super) trail: Vec<i32>,
    /// A `GetLayout` is in flight: signals meanwhile only mark it stale.
    pub(super) fetching: bool,
    /// When the last `GetLayout` went out: re-reads share the items'
    /// floor (see [`super::MIN_REFRESH_GAP`]).
    pub(super) last_asked: Option<Instant>,
    /// A signal arrived while `fetching`: read again when it answers.
    pub(super) stale: bool,
}

impl MenuOpen {
    /// Opens blank: the first `GetLayout` fills it.
    #[cfg(feature = "popup")]
    pub(super) fn new(item: &str, service: &str, path: &str, owner: &str) -> Self {
        Self {
            item: item.to_owned(),
            service: service.to_owned(),
            path: path.to_owned(),
            owner: owner.to_owned(),
            revision: 0,
            root: Vec::new(),
            trail: Vec::new(),
            fetching: false,
            last_asked: None,
            stale: false,
        }
    }

    /// The rows of the level shown: the trail walked from the root.
    /// `None` when the trail names what is gone (a changed layout
    /// shortens it on fill).
    pub(super) fn level(&self) -> Option<&[MenuNode]> {
        let mut level = self.root.as_slice();
        for id in &self.trail {
            level = level
                .iter()
                .find(|node| node.id == *id)?
                .children
                .as_slice();
        }
        Some(level)
    }

    /// Finds the row with dbusmenu `id` in the level shown.
    pub(super) fn find(&self, id: i32) -> Option<&MenuNode> {
        self.level()?.iter().find(|node| node.id == id)
    }
}

/// The `GetLayout(parent, depth, props)` body: the whole tree from
/// `parent`, bounded like the parse ([`MAX_MENU_DEPTH`]).
pub(super) fn get_layout_body(parent: i32) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.i32(parent);
    body.i32(MAX_MENU_DEPTH as i32);
    let cookie = body.open_array(4)?;
    for prop in LAYOUT_PROPS {
        body.str(prop);
    }
    body.close_array(cookie);
    body.take_body()
}

/// The `AboutToShow(id)` body.
pub(super) fn about_body(id: i32) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.i32(id);
    body.take_body()
}

/// The `Event(id, "clicked", data, timestamp)` body: the id, the name, a
/// zero `i32` variant no item reads, and a zero timestamp.
pub(super) fn event_body(id: i32) -> Option<Vec<u8>> {
    let mut body = Writer::new();
    body.i32(id);
    body.str("clicked");
    body.variant(EVENT_DATA_SIG);
    body.i32(0);
    body.u32(0);
    body.take_body()
}

/// Applies a `GetLayout` answer to the open menu: the tree replaced
/// whole (cleaned), the trail shortened past what is gone. `parent == 0`
/// replaces the root; any other parent fills one lazy submenu's
/// children in place.
pub(super) fn fill_menu(menu: &mut MenuOpen, parent: i32, revision: u32, items: &[MenuItem<'_>]) {
    menu.revision = revision;
    if parent == 0 {
        menu.root = items.iter().filter_map(MenuNode::of).collect();
        shorten_trail(menu);
    } else if let Some(node) = find_deep_mut(&mut menu.root, parent) {
        node.children = items.iter().filter_map(MenuNode::of).collect();
    }
}

/// Shortens the drill trail past rows the new layout no longer holds.
fn shorten_trail(menu: &mut MenuOpen) {
    let mut level = menu.root.as_slice();
    let mut kept = 0;
    for id in menu.trail.iter() {
        let Some(node) = level.iter().find(|node| node.id == *id) else {
            break;
        };
        level = node.children.as_slice();
        kept += 1;
    }
    menu.trail.truncate(kept);
}

fn find_deep_mut(nodes: &mut [MenuNode], id: i32) -> Option<&mut MenuNode> {
    for node in nodes.iter_mut() {
        if node.id == id {
            return Some(node);
        }
        if let Some(found) = find_deep_mut(&mut node.children, id) {
            return Some(found);
        }
    }
    None
}

/// Fills the popup with the level shown: a `< Back` row past the root,
/// then the rows. Separators are empty text rows (gaps), disabled rows
/// plain text, live rows buttons. A level longer than the popup holds
/// ([`crate::popup::MAX_WIDGETS`], less the back row) is cut: the
/// extras are dropped like any module's, silently — submenus keep a deep
/// tree addressable level by level instead. `false` with nothing to show
/// yet (the first `GetLayout` is still in flight): the popup shows a
/// `...` line instead (see [`fill_loading`]).
#[cfg(feature = "popup")]
pub(super) fn fill_popup(menu: &MenuOpen, content: &mut Content) -> bool {
    let Some(level) = menu.level() else {
        return false;
    };
    if level.is_empty() {
        return false;
    }
    if !menu.trail.is_empty() {
        content.button(format_args!("< Back"), BACK, None, false, false);
    }
    for node in level {
        if node.separator {
            content.text(format_args!(""));
            continue;
        }
        let prefix = toggle_prefix(node);
        if !node.enabled {
            content.text(format_args!("{prefix}{}", node.label));
        } else if node.opens() {
            content.button(
                format_args!("{prefix}{} >", node.label),
                DRILL,
                Some(node.id),
                false,
                false,
            );
        } else {
            // A row that acts and goes away: the popup closes on
            // release, and the state closes with it (see `menu_select`).
            content.button(
                format_args!("{prefix}{}", node.label),
                SELECT,
                Some(node.id),
                false,
                true,
            );
        }
    }
    true
}

/// The popup while the first `GetLayout` is in flight: one `...` line,
/// so the menu has a surface to re-fill when the layout lands.
#[cfg(feature = "popup")]
pub(super) fn fill_loading(content: &mut Content) {
    content.text(format_args!("..."));
}
