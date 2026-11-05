//! The panel's rows for the open windows, and their icons.
//!
//! Each window becomes one [`Entry`] the apps list machinery ranks and
//! draws as it does an application: the name is the title, the generic
//! name the `app_id` (so typing `firefox` finds a window titled by its
//! page), and the id `win:N` carries the window's number back to the
//! pick. Nothing here is on a keystroke path: the rows are built once,
//! at open.
//!
//! The icon is the desktop entry the `app_id` names, found in order: a
//! desktop id equal to `app_id` plus `.desktop`, a `StartupWMClass`
//! equal to it, then the same two ignoring ASCII case (apps disagree on
//! the case of an `app_id` and the file that ships for it). A matched
//! entry with no `Icon=` draws the letter tile. With no match at all
//! the `app_id` itself is tried as an icon name (many apps ship an icon
//! named for their `app_id`), and a name the icon lookup would refuse
//! draws the tile.

use crate::index::Entry;

/// The prefix of a window row's id: `win:NNNN`, the window's number in
/// the list the compositor announced, zero-padded to the width of the
/// cap. The ranker breaks ties between equal titles by comparing ids as
/// text, so the padding keeps that the announce order (`win:10` would
/// otherwise sort before `win:2`).
pub const ID_PREFIX: &str = "win:";

/// The title shown for a window with neither a title nor an `app_id`.
const UNTITLED: &str = "Untitled";

/// The window number a row id carries (leading zeros allowed), or `None`
/// for any other id.
pub fn window_number(id: &str) -> Option<usize> {
    id.strip_prefix(ID_PREFIX)?.parse().ok()
}

/// One row per window. `windows` is `(number, title, app_id)` per open
/// window in the compositor's order; `apps` is the desktop index
/// (hidden entries included: a `NoDisplay` app still owns its windows'
/// icon).
pub fn to_entries<'a>(
    windows: impl IntoIterator<Item = (usize, &'a str, &'a str)>,
    apps: &[Entry],
) -> Vec<Entry> {
    windows
        .into_iter()
        .map(|(number, title, app_id)| row(number, title, app_id, apps))
        .collect()
}

fn row(number: usize, title: &str, app_id: &str, apps: &[Entry]) -> Entry {
    let title = clean(title);
    let app_id = clean(app_id);
    let name = if !title.is_empty() {
        title
    } else if !app_id.is_empty() {
        app_id.clone()
    } else {
        UNTITLED.to_string()
    };
    Entry {
        id: format!("{ID_PREFIX}{number:04}"),
        name,
        generic_name: (!app_id.is_empty()).then(|| app_id.clone()),
        comment: None,
        keywords: Vec::new(),
        icon: icon_for(&app_id, apps),
        exec: Vec::new(),
        terminal: false,
        dbus_activatable: false,
        actions: Vec::new(),
        path: std::path::PathBuf::new(),
        workdir: None,
        startup_wm_class: None,
        hidden: None,
    }
}

/// Control characters (a newline in a title would draw as a glyph box
/// or break the row) become spaces; the ends are trimmed.
fn clean(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|char| if char.is_control() { ' ' } else { char })
        .collect();
    spaced.trim().to_string()
}

/// The `Icon=` for `app_id`: see the module docs for the order.
pub fn icon_for(app_id: &str, apps: &[Entry]) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    let by_id = |exact: bool| {
        apps.iter().find(|entry| {
            entry.id.strip_suffix(".desktop").is_some_and(|stem| {
                if exact {
                    stem == app_id
                } else {
                    stem.eq_ignore_ascii_case(app_id)
                }
            })
        })
    };
    let by_class = |exact: bool| {
        apps.iter().find(|entry| {
            entry.startup_wm_class.as_deref().is_some_and(|class| {
                if exact {
                    class == app_id
                } else {
                    class.eq_ignore_ascii_case(app_id)
                }
            })
        })
    };
    let matched = by_id(true)
        .or_else(|| by_class(true))
        .or_else(|| by_id(false))
        .or_else(|| by_class(false));
    match matched {
        Some(entry) => entry.icon.clone(),
        None => crate::icons::is_valid_name(app_id).then(|| app_id.to_string()),
    }
}
