//! One tray item: what it shows, parsed from its `GetAll` answer.
//!
//! Everything an item says is untrusted bytes from a same-user peer:
//! strings are sanitized once, here, at parse time (controls stripped, cut
//! like view text), pixmaps are bounded, converted to premultiplied once
//! per icon version, and an answer that does not parse is dropped whole,
//! the item keeping its last state.

use std::sync::Arc;
use std::time::Instant;

use super::{ITEM_KDE, MAX_ITEM_TEXT, MAX_STORED_ICONS, MAX_STORED_SIDE};
use crate::dbus::proto::{Pixmap, Writer, check_path, read_item_props};
use crate::icon::tray::TrayIcon;

/// One item: who it is, what it shows, and its converted icons.
pub(super) struct Item {
    /// `service + path`, the KDE watcher's id format and the sort key.
    pub(super) id: String,
    /// The bus name calls go to.
    pub(super) service: String,
    /// The object path calls go to.
    pub(super) path: String,
    /// The service's unique name: signal matching and vanish tracking.
    /// `None` until the first `GetNameOwner` answers.
    pub(super) owner: Option<String>,
    pub(super) status: Status,
    pub(super) title: String,
    pub(super) tooltip_title: String,
    pub(super) tooltip_text: String,
    pub(super) menu: String,
    pub(super) item_is_menu: bool,
    /// A `GetAll` is in flight for this item: signals meanwhile only mark
    /// it stale, so a chatty item never queues a call per signal.
    pub(super) fetching: bool,
    /// When the last `GetAll` went out: the gap between reads is
    /// [`MIN_REFRESH_GAP`].
    pub(super) last_asked: Option<Instant>,
    /// A signal arrived while `fetching`: read again when it answers.
    pub(super) stale: bool,
    /// The converted icons, smallest first, at most [`MAX_STORED_ICONS`].
    /// Each id hashes its pixels (see `convert`), so an unchanged icon
    /// compares equal and the icon cache hits every steady frame.
    pub(super) icons: Vec<Arc<TrayIcon>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Status {
    Passive,
    Active,
    NeedsAttention,
}

impl Status {
    pub(super) fn parse(text: &str) -> Self {
        match text {
            "Active" => Self::Active,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Passive,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Passive => "Passive",
            Self::Active => "Active",
            Self::NeedsAttention => "NeedsAttention",
        }
    }
}

/// Item strings sanitized once, at parse time: controls stripped (the
/// contract test's rule), cut at [`MAX_ITEM_TEXT`] bytes on a character
/// boundary.
pub(super) fn clean(text: &str) -> String {
    let stripped: String = text.chars().filter(|c| !c.is_control()).collect();
    if stripped.len() <= MAX_ITEM_TEXT {
        return stripped;
    }
    let mut cut = MAX_ITEM_TEXT;
    while !stripped.is_char_boundary(cut) {
        cut -= 1;
    }
    stripped[..cut].to_owned()
}

impl Item {
    pub(super) fn new(id: String, service: String, path: String) -> Self {
        Self {
            id,
            service,
            path,
            owner: None,
            // Active until the item says otherwise: one that omits the
            // property is shown, one that says `Passive` is hidden.
            status: Status::Active,
            fetching: false,
            last_asked: None,
            stale: false,
            title: String::new(),
            tooltip_title: String::new(),
            tooltip_text: String::new(),
            menu: String::new(),
            item_is_menu: false,
            icons: Vec::new(),
        }
    }

    /// Whether the bar draws this item: its status is not `Passive` (the
    /// spec's "hide me") and it sent a pixmap to draw. An item with only
    /// a themed icon name is tracked and clickable by index, but takes no
    /// room: there is no icon-theme lookup yet, and a blank slot is worse
    /// than none.
    pub(super) fn shown(&self) -> bool {
        self.status != Status::Passive && !self.icons.is_empty()
    }

    /// Picks the icon for `side` device pixels: the smallest kept entry
    /// at or past it, else the largest kept — exact device pixels where
    /// one matches, a smooth scale otherwise. Icons are stored smallest
    /// first (see `convert`).
    pub(super) fn icon_for(&self, side: u32) -> Option<&Arc<TrayIcon>> {
        let mut largest = None;
        for icon in &self.icons {
            largest = Some(icon);
            if icon.side() >= side {
                return Some(icon);
            }
        }
        largest
    }
}

/// What the view is drawn from: everything `fill` sets that the screen
/// shows. Compared before and after each answer, so the bar redraws on a
/// real change and nothing else.
#[derive(PartialEq, Eq)]
pub(super) struct Fingerprint {
    pub(super) status: Status,
    pub(super) title: String,
    pub(super) tooltip_title: String,
    pub(super) tooltip_text: String,
    pub(super) menu: String,
    pub(super) item_is_menu: bool,
    pub(super) icons: Vec<Arc<TrayIcon>>,
}

impl Fingerprint {
    pub(super) fn of(item: &Item) -> Self {
        Self {
            status: item.status,
            title: item.title.clone(),
            tooltip_title: item.tooltip_title.clone(),
            tooltip_text: item.tooltip_text.clone(),
            menu: item.menu.clone(),
            item_is_menu: item.item_is_menu,
            icons: item.icons.clone(),
        }
    }
}

/// The `Properties.GetAll` body for the item interface.
pub(super) fn get_all_body() -> Vec<u8> {
    let mut body = Writer::new();
    body.str(ITEM_KDE);
    body.take_body().unwrap_or_default()
}

/// Whether a bus name is an item's well-known name (the KDE or
/// freedesktop prefix): what the start-up enumeration picks up. Items
/// behind plain unique names register explicitly and cannot be listed —
/// the KDE watcher's own limitation.
pub(super) fn is_item_name(name: &str) -> bool {
    name.starts_with("org.kde.StatusNotifierItem")
        || name.starts_with("org.freedesktop.StatusNotifierItem")
}

/// Applies a `GetAll` body to the item: sanitized strings and converted
/// icons, from [`read_item_props`] (the walk the fuzz target runs).
/// `false` drops the answer (the item keeps its last state).
pub(super) fn fill(item: &mut Item, body: &[u8]) -> bool {
    let Ok(props) = read_item_props(body) else {
        return false;
    };
    if let Some(status) = props.status {
        item.status = Status::parse(status);
    }
    if let Some(title) = props.title {
        item.title = clean(title);
    }
    if let Some((title, text)) = props.tooltip {
        item.tooltip_title = clean(title);
        item.tooltip_text = clean(text);
    }
    if let Some(menu) = props.menu {
        // A path the bar will one day call: kept only when it is one, and
        // short (a hostile item's megabyte "path" is not stored).
        if menu.len() <= MAX_ITEM_TEXT && check_path(menu).is_ok() {
            item.menu = menu.to_owned();
        }
    }
    if let Some(item_is_menu) = props.item_is_menu {
        item.item_is_menu = item_is_menu;
    }
    if let Some(pixmaps) = props.pixmaps {
        item.icons = convert(&item.id, &pixmaps);
    }
    true
}

/// Converts the kept pixmap entries to premultiplied icons, smallest
/// first: entries past [`MAX_STORED_SIDE`] are left for the icon cache
/// to scale from the single smallest kept, at most [`MAX_STORED_ICONS`]
/// entries. Each id hashes its pixels, so an unchanged icon compares
/// equal and the cache hits.
pub(super) fn convert(id: &str, pixmaps: &[Pixmap<'_>]) -> Vec<Arc<TrayIcon>> {
    let mut kept: Vec<&Pixmap<'_>> = pixmaps
        .iter()
        .filter(|pixmap| pixmap.width <= MAX_STORED_SIDE && pixmap.height <= MAX_STORED_SIDE)
        .collect();
    kept.sort_by_key(|pixmap| pixmap.width.max(pixmap.height));
    if kept.is_empty() {
        kept = pixmaps
            .iter()
            .min_by_key(|pixmap| pixmap.width.max(pixmap.height))
            .into_iter()
            .collect();
    }
    kept.truncate(MAX_STORED_ICONS);
    kept.into_iter()
        .filter_map(|pixmap| {
            let key = fnv(id, pixmap.width, pixmap.height, pixmap.pixels);
            TrayIcon::take(key, pixmap.width, pixmap.height, pixmap.pixels).map(Arc::new)
        })
        .collect()
}

/// FNV-1a over the icon's identity and pixels: the cache key. Collisions
/// show a stale icon until the next `NewIcon`; 64 bits over a handful of
/// icons makes that a non-event.
pub(super) fn fnv(id: &str, width: u32, height: u32, pixels: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in id
        .bytes()
        .chain(width.to_le_bytes())
        .chain(height.to_le_bytes())
        .chain(pixels.iter().copied())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
