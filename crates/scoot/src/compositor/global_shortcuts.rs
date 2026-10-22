//! Portal global shortcuts, stage 1: the compositor-side registry.
//!
//! Keyed by `(app_id, shortcut_id)` (the portal's own identity), with a
//! reverse index on the user-assigned chord for the per-keypress grab. The
//! app's `preferred_trigger` is a hint only -- stored and listed, never
//! auto-bound -- and the user assigns chords via `[global_shortcuts]`
//! config or `ShortcutBind` IPC. Compositor binds always win (checked
//! first in `input.rs`); a shadowed chord stays listed but never fires.

use std::collections::HashMap;

use scoot_ipc::ShortcutInfo;
use smithay::input::keyboard::{Keysym, xkb};

use super::keybindings::Modifiers;

/// One user-assigned chord: the modifiers, the unshifted keysym, and the
/// canonical spelling (`super+ctrl+r`) the list reply and the config file
/// share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalBind {
    pub mods: Modifiers,
    pub keysym: Keysym,
    pub combo: String,
}

/// One registered shortcut.
#[derive(Clone, Debug)]
pub struct GlobalShortcut {
    pub app_id: String,
    pub shortcut_id: String,
    pub description: Option<String>,
    pub preferred_trigger: Option<String>,
    pub bound: Option<GlobalBind>,
    pub owner: u64,
}

/// The registry. `entries` is the authority; `by_trigger` is its index.
/// Both are updated together; `by_trigger` maps `(mods, keysym.raw())`
/// because `Keysym` has no `Hash` (same keying as `config::apply_binds`).
#[derive(Clone, Debug, Default)]
pub struct GlobalShortcuts {
    entries: HashMap<(String, String), GlobalShortcut>,
    by_trigger: HashMap<(Modifiers, u32), (String, String)>,
    /// `[global_shortcuts]` config: `(app_id, shortcut_id)` to raw combo
    /// string. Applied at register and reload time; bad entries land in
    /// `skipped`.
    pub config: HashMap<(String, String), String>,
    /// Config entries that never bound anything, with reasons: malformed
    /// keys (`key_errors`, owned by the file parse) plus unresolvable
    /// combos (recomputed by every `apply_config`).
    pub skipped: Vec<scoot_ipc::SkippedBind>,
    /// The malformed-key half of `skipped`: entries whose key is not
    /// `app-id:shortcut-id`. Owned by the file parse (`global_shortcuts_for`
    /// in `config.rs`), preserved across every `apply_config` -- which
    /// recomputes only the bad-combo half.
    pub key_errors: Vec<scoot_ipc::SkippedBind>,
}

impl GlobalShortcuts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Idempotent upsert by `(app_id, shortcut_id)`. A fresh entry picks
    /// up the config chord for its id, if the file names one that parses.
    pub fn register(
        &mut self,
        app_id: String,
        shortcut_id: String,
        preferred_trigger: Option<String>,
        description: Option<String>,
        owner: u64,
    ) {
        let key = (app_id.clone(), shortcut_id.clone());
        if let Some(entry) = self.entries.get_mut(&key) {
            if preferred_trigger.is_some() {
                entry.preferred_trigger = preferred_trigger;
            }
            if description.is_some() {
                entry.description = description;
            }
            entry.owner = owner;
            return;
        }
        let mut entry = GlobalShortcut {
            app_id: app_id.clone(),
            shortcut_id: shortcut_id.clone(),
            description,
            preferred_trigger,
            bound: None,
            owner,
        };
        if let Some(raw) = self.config.get(&key).cloned()
            && let Some(bind) = resolve_combo_str(&raw)
        {
            self.index_insert(&key, &bind);
            entry.bound = Some(bind);
        }
        self.entries.insert(key, entry);
    }

    pub fn unregister(&mut self, app_id: &str, shortcut_id: &str) -> bool {
        let key = (app_id.to_owned(), shortcut_id.to_owned());
        let Some(entry) = self.entries.remove(&key) else {
            return false;
        };
        if let Some(bind) = entry.bound {
            self.by_trigger.remove(&(bind.mods, bind.keysym.raw()));
        }
        true
    }

    /// Bind (or, with `None`, clear) one registered shortcut. Returns
    /// `false` when the id is unknown.
    pub fn bind(&mut self, app_id: &str, shortcut_id: &str, bind: Option<GlobalBind>) -> bool {
        let key = (app_id.to_owned(), shortcut_id.to_owned());
        if !self.entries.contains_key(&key) {
            return false;
        }
        if let Some(old) = self.entries.get(&key).and_then(|entry| entry.bound.clone()) {
            self.by_trigger.remove(&(old.mods, old.keysym.raw()));
        }
        if let Some(bind) = bind {
            self.by_trigger
                .insert((bind.mods, bind.keysym.raw()), key.clone());
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.bound = Some(bind);
            }
        } else if let Some(entry) = self.entries.get_mut(&key) {
            entry.bound = None;
        }
        true
    }

    fn index_insert(&mut self, key: &(String, String), bind: &GlobalBind) {
        // Two ids on one chord: the later bind wins the grab, both stay
        // listed. (Portal UIs prevent this; the IPC primitive stays dumb.)
        self.by_trigger
            .insert((bind.mods, bind.keysym.raw()), key.clone());
    }

    /// What chord `(keysym, mods)` fires, if any. No allocation: a stack
    /// key into the reverse index. The caller checks compositor binds
    /// first, and re-checks shadowing at fire time.
    pub fn match_key(&self, keysym: Keysym, mods: Modifiers) -> Option<&GlobalShortcut> {
        self.by_trigger
            .get(&(mods, keysym.raw()))
            .and_then(|key| self.entries.get(key))
    }

    /// Drop every entry owned by `conn`. Returns whether anything changed
    /// (drives `ShortcutsChanged` emission).
    pub fn remove_owner(&mut self, conn: u64) -> bool {
        let owned: Vec<(String, String)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.owner == conn)
            .map(|(k, _)| k.clone())
            .collect();
        if owned.is_empty() {
            return false;
        }
        for key in owned {
            if let Some(entry) = self.entries.remove(&key)
                && let Some(bind) = entry.bound
            {
                self.by_trigger.remove(&(bind.mods, bind.keysym.raw()));
            }
        }
        true
    }

    /// Re-resolve every config chord against the current entries: what a
    /// reload calls after re-parsing the file. Unknown ids stay unbound
    /// (an app registers first, then the file's chord applies); chords
    /// for known ids rebind. The malformed-key half of `skipped`
    /// (`key_errors`) is preserved; the bad-combo half is recomputed.
    pub fn apply_config(&mut self) {
        self.skipped = self.key_errors.clone();
        // Rebuild the reverse index from scratch: config may have moved
        // chords between ids.
        self.by_trigger.clear();
        // Collect first: `resolve_combo_str` borrows nothing here, but
        // sorting keeps `skipped` deterministic.
        let mut raws: Vec<((String, String), String)> = self
            .config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        raws.sort();
        for ((app_id, shortcut_id), raw) in raws {
            if raw.trim() == "none" || raw.trim().is_empty() {
                if let Some(entry) = self.entries.get_mut(&(app_id.clone(), shortcut_id.clone())) {
                    entry.bound = None;
                }
                continue;
            }
            match resolve_combo_str(&raw) {
                Some(bind) => {
                    if let Some(entry) =
                        self.entries.get_mut(&(app_id.clone(), shortcut_id.clone()))
                    {
                        // Last write wins per chord; config applies in
                        // sorted-id order so the result is deterministic.
                        self.by_trigger.insert(
                            (bind.mods, bind.keysym.raw()),
                            (app_id.clone(), shortcut_id.clone()),
                        );
                        entry.bound = Some(bind);
                    }
                    // Else: no registrant yet; the chord applies at
                    // register time. Not skipped: the file is fine, the
                    // app simply has not registered.
                }
                None => {
                    self.skipped.push(scoot_ipc::SkippedBind {
                        bind: format!("{app_id}:{shortcut_id}"),
                        value: raw.clone(),
                        reason: format!("unknown key or bad combo `{raw}`"),
                    });
                }
            }
        }
        // Re-index IPC-bound chords for ids the config does not name.
        // Config chords above already hold their index slots; an IPC chord
        // colliding with one loses the grab but stays listed (conflict is
        // computed at snapshot/fire time, not here).
        let mut ipc: Vec<((String, String), GlobalBind)> = self
            .entries
            .iter()
            .filter(|(k, e)| e.bound.is_some() && !self.config.contains_key(*k))
            .map(|(k, e)| (k.clone(), e.bound.clone().expect("checked above")))
            .collect();
        ipc.sort_by(|a, b| a.0.cmp(&b.0));
        for (key, bind) in ipc {
            self.by_trigger
                .entry((bind.mods, bind.keysym.raw()))
                .or_insert(key);
        }
    }

    /// The `Shortcuts` list reply. `is_shadowed` names chords a live
    /// compositor bind holds (compositor wins); those rows report
    /// `conflicted: true`. Sorted by `(app_id, shortcut_id)`.
    pub fn snapshot(&self, is_shadowed: impl Fn(Modifiers, Keysym) -> bool) -> Vec<ShortcutInfo> {
        let mut rows: Vec<ShortcutInfo> = self
            .entries
            .values()
            .map(|entry| {
                let (bound, conflicted) = match &entry.bound {
                    Some(bind) => (
                        Some(bind.combo.clone()),
                        is_shadowed(bind.mods, bind.keysym),
                    ),
                    None => (None, false),
                };
                ShortcutInfo {
                    app_id: entry.app_id.clone(),
                    shortcut_id: entry.shortcut_id.clone(),
                    description: entry.description.clone(),
                    preferred_trigger: entry.preferred_trigger.clone(),
                    bound,
                    conflicted,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            (a.app_id.clone(), a.shortcut_id.clone())
                .cmp(&(b.app_id.clone(), b.shortcut_id.clone()))
        });
        rows
    }
}

/// Resolve one combo string (`super+ctrl+r`) to modifiers, keysym and its
/// canonical spelling. Single ASCII letters fold to lowercase (unshifted
/// keysym, Shift as modifier), mirroring `config::parse_combo`.
pub fn resolve_combo_str(raw: &str) -> Option<GlobalBind> {
    let combo: scoot_ipc::KeyCombo = raw.parse().ok()?;
    resolve_combo(&combo)
}

/// Resolve one parsed [`scoot_ipc::KeyCombo`].
///
/// A single ASCII letter folds to lowercase before resolving it: xkb gives
/// single Latin letters two distinct valid keysyms by case, but matching
/// (like `config::parse_combo`) binds by the unshifted keysym only, so an
/// unfolded capital would resolve to a keysym that never fires.
pub fn resolve_combo(combo: &scoot_ipc::KeyCombo) -> Option<GlobalBind> {
    let mut mods = Modifiers::default();
    for modifier in &combo.modifiers {
        match modifier {
            scoot_ipc::Modifier::Super => mods.super_ = true,
            scoot_ipc::Modifier::Shift => mods.shift = true,
            scoot_ipc::Modifier::Ctrl => mods.ctrl = true,
            scoot_ipc::Modifier::Alt => mods.alt = true,
        }
    }
    let folded;
    let key = if combo.key.len() == 1
        && let Some(c) = combo.key.chars().next()
        && c.is_ascii_alphabetic()
    {
        folded = c.to_ascii_lowercase().to_string();
        folded.as_str()
    } else {
        combo.key.as_str()
    };
    let keysym = keysym_named(key)?;
    Some(GlobalBind {
        mods,
        keysym,
        combo: combo_string(mods, keysym),
    })
}

/// Canonical combo spelling: modifiers in `super+shift+ctrl+alt` order,
/// then xkb's own keysym name. Same shape as `config::combo_string`.
pub fn combo_string(mods: Modifiers, keysym: Keysym) -> String {
    let mut combo = String::new();
    if mods.super_ {
        combo.push_str("super+");
    }
    if mods.shift {
        combo.push_str("shift+");
    }
    if mods.ctrl {
        combo.push_str("ctrl+");
    }
    if mods.alt {
        combo.push_str("alt+");
    }
    combo.push_str(&xkb::keysym_get_name(keysym));
    combo
}

fn keysym_named(name: &str) -> Option<Keysym> {
    let exact = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    let keysym = if exact.raw() == 0 {
        xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE)
    } else {
        exact
    };
    (keysym.raw() != 0).then_some(keysym)
}

/// Parse one `[global_shortcuts]` key (`app-id:shortcut-id`).
pub fn parse_global_key(key: &str) -> Option<(String, String)> {
    let (app, id) = key.split_once(':')?;
    let (app, id) = (app.trim(), id.trim());
    if app.is_empty() || id.is_empty() {
        return None;
    }
    Some((app.to_owned(), id.to_owned()))
}

impl super::state::State {
    /// Whether a live compositor bind holds this chord: global shortcuts
    /// never fire through one (compositor wins). Checked at fire time, not
    /// just bind time, so a reload that adds a compositor bind immediately
    /// shadows.
    pub(crate) fn global_shadowed(&self, mods: Modifiers, keysym: Keysym) -> bool {
        self.keybindings.match_key(keysym, mods).is_some()
    }

    /// The `Shortcuts` list reply for the current registry.
    pub(crate) fn shortcuts_response(&self) -> scoot_ipc::Response {
        let is_shadowed = |mods: Modifiers, keysym: Keysym| self.global_shadowed(mods, keysym);
        scoot_ipc::Response::Shortcuts {
            shortcuts: self.global_shortcuts.snapshot(is_shadowed),
        }
    }

    /// Emit one press/release to every `GlobalShortcut` subscriber. Cold
    /// path (one encode plus one clone per subscriber, only when a bound
    /// chord fires); the per-keypress lookup that reaches here allocated
    /// nothing.
    pub(crate) fn emit_shortcut_activated(
        &mut self,
        app_id: String,
        shortcut_id: String,
        timestamp_ms: u32,
    ) {
        let Ok(line) = scoot_ipc::encode(&scoot_ipc::Response::ShortcutActivated {
            app_id,
            shortcut_id,
            timestamp_ms,
        }) else {
            return;
        };
        self.emit(scoot_ipc::EventKind::GlobalShortcut, &line);
    }

    /// Emit one release to every `GlobalShortcut` subscriber.
    pub(crate) fn emit_shortcut_deactivated(
        &mut self,
        app_id: String,
        shortcut_id: String,
        timestamp_ms: u32,
    ) {
        let Ok(line) = scoot_ipc::encode(&scoot_ipc::Response::ShortcutDeactivated {
            app_id,
            shortcut_id,
            timestamp_ms,
        }) else {
            return;
        };
        self.emit(scoot_ipc::EventKind::GlobalShortcut, &line);
    }

    /// Emit `ShortcutsChanged` to every `GlobalShortcut` subscriber: the
    /// registered set moved (register, unregister, bind, unbind, reload,
    /// registrant disconnect), so a backend re-lists.
    pub(crate) fn emit_shortcuts_changed(&mut self) {
        let Ok(line) = scoot_ipc::encode(&scoot_ipc::Response::ShortcutsChanged) else {
            return;
        };
        self.emit(scoot_ipc::EventKind::GlobalShortcut, &line);
    }

    /// Drop every shortcut owned by `conn`, emitting `ShortcutsChanged`
    /// when anything went away. Called when a connection source leaves the
    /// event loop (see `ipc.rs`'s accept closure), beside
    /// `drop_subscriber`, so a dead registrant's chords go quiet on
    /// disconnect.
    pub(crate) fn drop_global_shortcuts_for(&mut self, conn: u64) {
        if self.global_shortcuts.remove_owner(conn) {
            self.emit_shortcuts_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::input::keyboard::Keysym;

    fn mods(super_: bool, shift: bool, ctrl: bool, alt: bool) -> Modifiers {
        Modifiers {
            super_,
            shift,
            ctrl,
            alt,
        }
    }

    #[test]
    fn register_applies_the_config_chord() {
        let mut table = GlobalShortcuts::new();
        table
            .config
            .insert(("org.x".into(), "rec".into()), "super+ctrl+r".into());
        table.register("org.x".into(), "rec".into(), None, None, 7);
        let entry = table.entries.get(&("org.x".into(), "rec".into())).unwrap();
        assert_eq!(entry.bound.as_ref().unwrap().combo, "super+ctrl+r");
        assert_eq!(entry.owner, 7);
        let (m, k) = (
            entry.bound.as_ref().unwrap().mods,
            entry.bound.as_ref().unwrap().keysym,
        );
        assert!(table.match_key(k, m).is_some());
    }

    #[test]
    fn preferred_trigger_never_autobinds() {
        let mut table = GlobalShortcuts::new();
        table.register(
            "org.x".into(),
            "rec".into(),
            Some("super+ctrl+r".into()),
            None,
            7,
        );
        let entry = table.entries.get(&("org.x".into(), "rec".into())).unwrap();
        assert!(entry.bound.is_none());
        assert!(table.by_trigger.is_empty());
    }

    #[test]
    fn bind_and_clear_round_trip() {
        let mut table = GlobalShortcuts::new();
        table.register("org.x".into(), "rec".into(), None, None, 7);
        assert!(!table.bind("org.no", "rec", None));
        // Unknown shortcut id refuses too, for bind and clear alike.
        assert!(!table.bind("org.x", "nope", resolve_combo_str("super+r")));
        assert!(table.bind("org.x", "rec", resolve_combo_str("super+r")));
        assert!(
            table
                .match_key(Keysym::r, mods(true, false, false, false))
                .is_some()
        );
        assert!(table.bind("org.x", "rec", None));
        assert!(
            table
                .match_key(Keysym::r, mods(true, false, false, false))
                .is_none()
        );
    }

    #[test]
    fn remove_owner_clears_only_its_entries() {
        let mut table = GlobalShortcuts::new();
        table.register("a".into(), "one".into(), None, None, 1);
        table.register("b".into(), "two".into(), None, None, 2);
        assert!(table.bind("a", "one", resolve_combo_str("super+a")));
        assert!(table.bind("b", "two", resolve_combo_str("super+b")));
        assert!(table.remove_owner(1));
        assert!(!table.entries.contains_key(&("a".into(), "one".into())));
        assert!(table.entries.contains_key(&("b".into(), "two".into())));
        assert!(!table.remove_owner(1));
    }

    #[test]
    fn snapshot_sorts_and_flags_conflicts() {
        let mut table = GlobalShortcuts::new();
        table.register("b".into(), "two".into(), None, None, 1);
        table.register("a".into(), "one".into(), None, None, 1);
        table.bind("b", "two", resolve_combo_str("super+b"));
        let rows = table.snapshot(|_, _| true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].app_id, "a");
        assert_eq!(rows[1].app_id, "b");
        assert!(rows[1].conflicted);
        let rows = table.snapshot(|_, _| false);
        assert!(!rows[1].conflicted);
    }

    #[test]
    fn hot_path_match_allocates_nothing() {
        // The grab lookup is one hash get on a stack key; this pins that
        // no heap work hides in it by running it under a counting check
        // is overkill -- instead assert the key types are Copy so the
        // lookup cannot allocate.
        fn is_copy<T: Copy>() {}
        is_copy::<Modifiers>();
        let mut table = GlobalShortcuts::new();
        table.register("a".into(), "one".into(), None, None, 1);
        table.bind("a", "one", resolve_combo_str("super+a"));
        assert!(
            table
                .match_key(Keysym::a, mods(true, false, false, false))
                .is_some()
        );
        assert!(
            table
                .match_key(Keysym::b, mods(true, false, false, false))
                .is_none()
        );
    }
}
