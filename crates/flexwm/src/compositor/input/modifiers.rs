//! Which key to press, and what to hold down around it, to type a keysym.
//!
//! A keymap maps one *keycode* to several keysyms, one per shift *level*:
//! on a plain US layout the `a` key carries `a` at level 0 and `A` at level
//! 1, the `1` key carries `1` and `!`, and a German layout's `q` key carries
//! `q`, `Q` and `@` (that last one three levels up, behind AltGr). Which
//! level a press lands on is decided by the modifiers held at that moment,
//! so typing a character is two questions, not one: *which key* carries it,
//! and *which modifiers* have to be held for that key to reach the level it
//! sits at.
//!
//! flexwm used to answer only the first one -- it asked Smithay's
//! `raw_syms_for_key_in_layout`, which is hard-coded to level 0, whether the
//! keysym needed Shift, and that call can only ever answer "no" for a keysym
//! that needs *any* modifier (level 0 is by definition the unmodified one).
//! So `flexwm msg type "AbC"` pressed the right three keys with nothing held
//! and delivered `abc`: no error, just quietly the wrong text.
//!
//! Everything here works off the keymap alone, never off the live keyboard
//! state, and answers both questions in one walk:
//!
//! - [`plan`] finds the keycode *and* the level in a single scan, so the two
//!   can't disagree the way two separate lookups did.
//! - [`ModifierKeys::probe`] asks the keymap which key actually depresses
//!   each modifier, instead of naming one per modifier and hoping. Layouts
//!   move them around (`de` reaches its third level from the right-hand Alt
//!   key, `de(neo)` from the key in the `#` position) and some modifier keys
//!   carry no keysym to be named by at all, while the keymap knows all of it
//!   already. It also settles the question a name table cannot: whether a
//!   key *holds* its modifier or latches/locks it, which is the difference
//!   between typing one capital letter and turning Caps Lock on for good.
//!
//! Nothing here allocates: the probe walks the keymap in place and every
//! result is a fixed-size, `Copy` value, so typing a string costs no heap
//! traffic on the IPC path however long the string is.

use smithay::input::keyboard::{Keycode, Keysym, xkb};

#[cfg(test)]
mod tests;

/// How many *real* modifiers xkb has: `Shift`, `Lock`, `Control` and
/// `Mod1`..`Mod5`. Every mask that reaches this module is a mask over those
/// eight -- a keymap's named "virtual" modifiers (`Alt`, `Super`,
/// `LevelThree`, ...) are resolved to real ones when the keymap is compiled.
/// Used as a bound, not as an assumption: a set bit past it is treated as a
/// modifier nothing here can produce (see [`ModifierKeys::hold`]), not as an
/// index to look up.
const REAL_MODIFIERS: usize = 8;

/// How many modifier combinations [`plan`] will consider for one level.
///
/// `xkb_keymap_key_get_mods_for_level` fills a caller-provided buffer and
/// silently drops whatever doesn't fit, so this is a real cap and not just a
/// capacity: it has to be comfortably above what any key type produces. Real
/// keymaps stay in single digits (the widest stock type, `EIGHT_LEVEL`, has
/// eight entries spread across its levels); 32 leaves room for an exotic one
/// without putting a kilobyte on the stack.
const MAX_MASKS: usize = 32;

/// What one character costs: the key to press, and the modifiers to hold
/// while pressing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyPlan {
    /// The key carrying the keysym.
    pub(super) code: Keycode,
    /// Held before `code` goes down and released after it comes up, in
    /// [`HeldKeys::as_slice`] order.
    pub(super) modifiers: HeldKeys,
}

/// Why a character can't be typed on the active layout.
///
/// Kept apart because they mean different things to whoever is driving:
/// [`Untypable::NoKey`] is "this layout has no such character at all" (pick
/// another layout, or another character), while
/// [`Untypable::NoModifiers`] is "it's there, but only behind something
/// flexwm won't press."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Untypable {
    /// No key in the layout carries the keysym at any level.
    NoKey,
    /// A key carries it, but every modifier combination that reaches its
    /// level needs a modifier no key can simply hold down -- in practice a
    /// locking or latching one, e.g. a level only Caps Lock reaches.
    NoModifiers,
}

/// The modifier keys to hold around one keypress. At most one per real
/// modifier, so the array is sized by construction and can never overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HeldKeys {
    codes: [Keycode; REAL_MODIFIERS],
    len: usize,
}

impl Default for HeldKeys {
    fn default() -> Self {
        Self {
            // Never read: `as_slice` only ever exposes the first `len`.
            codes: [Keycode::new(0); REAL_MODIFIERS],
            len: 0,
        }
    }
}

impl HeldKeys {
    /// The keys to hold, in the order they should be pressed.
    pub(super) fn as_slice(&self) -> &[Keycode] {
        &self.codes[..self.len]
    }

    fn push(&mut self, code: Keycode) {
        // Unreachable by construction -- `hold` pushes at most one key per
        // real modifier, and there are exactly `REAL_MODIFIERS` of those --
        // but a silent overwrite would be a wrong-modifier bug, so drop the
        // key instead and let the caller's own check fail loudly.
        if let Some(slot) = self.codes.get_mut(self.len) {
            *slot = code;
            self.len += 1;
        }
    }
}

/// Which key, if any, holds each real modifier down.
///
/// Built by pressing every key in a throwaway xkb state and watching what it
/// does to the modifier mask -- the keymap's own answer to "what do I press
/// to get Mod5 on *this* layout", which is not something a fixed
/// keysym-per-modifier table can know.
///
/// Deliberately only records keys that can be *held*: a key that latches or
/// locks its modifier (Caps Lock, Num Lock, `ISO_Level3_Latch`) is rejected,
/// because press-and-release around a character would leave it toggled on
/// for everything typed afterwards, which is a far worse outcome than
/// failing to type one character.
pub(super) struct ModifierKeys {
    /// Indexed by real modifier index (`Shift` = 0, `Lock` = 1, ...).
    keys: [Option<Keycode>; REAL_MODIFIERS],
}

impl ModifierKeys {
    /// Walks the whole keymap once. Costs a few hundred FFI calls, which is
    /// why [`plan`] builds this lazily and at most once per string typed,
    /// rather than once per character.
    pub(super) fn probe(keymap: &xkb::Keymap) -> Self {
        let mut keys = [None; REAL_MODIFIERS];
        let mut state = xkb::State::new(keymap);
        for code in (keymap.min_keycode().raw()..=keymap.max_keycode().raw()).map(Keycode::new) {
            let _ = state.update_key(code, xkb::KeyDirection::Down);
            let depressed = state.serialize_mods(xkb::STATE_MODS_DEPRESSED);
            let held_only = state.serialize_mods(xkb::STATE_MODS_LATCHED) == 0
                && state.serialize_mods(xkb::STATE_MODS_LOCKED) == 0;
            let _ = state.update_key(code, xkb::KeyDirection::Up);
            // Caps Lock sets its modifier in the *depressed* mask too while
            // it's down, so "did anything survive the release" is the only
            // reliable way to tell a plain modifier from a latch or a lock.
            // A key that failed it also left this state dirty, so start the
            // rest of the walk from a clean one.
            if state.serialize_mods(xkb::STATE_MODS_EFFECTIVE) != 0 {
                state = xkb::State::new(keymap);
                continue;
            }
            if !held_only || depressed.count_ones() != 1 {
                continue;
            }
            // Exactly one bit is set, so this is the modifier it holds.
            let index = depressed.trailing_zeros() as usize;
            if let Some(slot) = keys.get_mut(index)
                && slot.is_none()
            {
                // First key wins, and keycodes ascend, so this is the
                // left-hand one of a pair -- the same choice a person makes
                // without thinking about it.
                *slot = Some(code);
            }
        }
        Self { keys }
    }

    /// The keys to hold to produce exactly `mask`, or `None` if any modifier
    /// in it has no key that can hold it.
    ///
    /// Only single-modifier keys are considered. A key that sets two
    /// modifiers at once could in principle cover part of a mask, but every
    /// keymap that needs one of these levels also has the plain modifier
    /// keys for it, and "held Ctrl as a side effect of typing `@`" is a
    /// worse failure than a clear error.
    fn hold(&self, mask: xkb::ModMask) -> Option<HeldKeys> {
        let mut held = HeldKeys::default();
        let mut remaining = mask;
        while remaining != 0 {
            let index = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            // `get` rather than `[]`: a mask bit past the real modifiers is
            // a modifier this can't produce, not a panic.
            let code = (*self.keys.get(index)?)?;
            held.push(code);
        }
        (held.len == mask.count_ones() as usize).then_some(held)
    }
}

/// Plans the keypress that types `keysym` on `layout` of `keymap`.
///
/// `modifier_keys` is the caller's own one-string cache for
/// [`ModifierKeys::probe`]: it is filled in on the first character that
/// needs a modifier and reused for the rest, so a lowercase string never
/// pays for the walk at all and a mixed-case one pays once.
pub(super) fn plan(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    keysym: Keysym,
    modifier_keys: &mut Option<ModifierKeys>,
) -> Result<KeyPlan, Untypable> {
    let (code, level) = key_for(keymap, layout, keysym).ok_or(Untypable::NoKey)?;
    let mut masks = [xkb::ModMask::default(); MAX_MASKS];
    let count = keymap
        .key_get_mods_for_level(code, layout, level, &mut masks)
        .min(MAX_MASKS);
    let masks = &masks[..count];
    // The overwhelming majority of characters typed: an empty mask reaches
    // the level, so nothing has to be held and the probe is never run.
    // Checked rather than assumed of level 0 -- "the lowest level needs no
    // modifiers" is a convention of how keymaps are written, not a rule
    // xkbcommon enforces.
    if masks.contains(&0) {
        return Ok(KeyPlan {
            code,
            modifiers: HeldKeys::default(),
        });
    }
    let modifier_keys = modifier_keys.get_or_insert_with(|| ModifierKeys::probe(keymap));
    // Several combinations can reach one level -- an alphabetic key's upper
    // level is reached by Shift *or* by Caps Lock -- so take the cheapest
    // one that can actually be held down, which is what drops Caps Lock in
    // favour of Shift without naming either.
    let modifiers = masks
        .iter()
        .filter_map(|&mask| modifier_keys.hold(mask))
        .min_by_key(|held| held.len)
        .ok_or(Untypable::NoModifiers)?;
    Ok(KeyPlan { code, modifiers })
}

/// The key carrying `keysym` on `layout`, and the level it sits at.
///
/// Same scan Smithay's own `KeyboardHandle::keycode_for_keysym` does (lowest
/// keycode that carries the keysym at any level), extended to report *which*
/// level it found it at -- which is the half flexwm was missing, and the
/// reason this doesn't just call Smithay's version and then go looking for
/// the level a second time.
fn key_for(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    keysym: Keysym,
) -> Option<(Keycode, xkb::LevelIndex)> {
    (keymap.min_keycode().raw()..=keymap.max_keycode().raw())
        .map(Keycode::new)
        .find_map(|code| {
            (0..keymap.num_levels_for_key(code, layout))
                .find(|&level| {
                    keymap
                        .key_get_syms_by_level(code, layout, level)
                        .contains(&keysym)
                })
                .map(|level| (code, level))
        })
}
