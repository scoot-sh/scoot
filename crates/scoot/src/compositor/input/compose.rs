//! Dead-key sequences for [`State::type_text`](super::State::type_text).
//!
//! Most characters come off a single key at some shift level, which is what
//! [`modifiers::plan`](super::modifiers) resolves. The rest -- `é` on a
//! plain `de` layout, `~` on a Nordic one -- are two keypresses with a state
//! machine in between (`dead_acute`, then `e`; `dead_tilde`, then space),
//! not a key with a level. Driving those needs the session's compose table
//! and a second resolution path for when the single-key lookup fails.
//!
//! How it works, per `type_text` request: on the first character the direct
//! path cannot type, [`table_from_session_locale`] loads the compose table
//! for the session locale (what a person typing on this machine gets, and
//! what the client on the other end decodes with), and [`build_map`] feeds
//! every dead key the *active* layout carries followed by every keysym on
//! that layout through it, recording which pairs compose to a single
//! character. [`plan_sequence`] then answers one character from that map,
//! planning both halves through [`modifiers::plan`](super::modifiers) -- so
//! a half the active layout cannot hold (or a map built for another group,
//! or no table at all) degrades to the same loud refusal as before, never
//! to a silently wrong key.
//!
//! Three deliberate boundaries, matching the ticket
//! (`docs/backlog/resolved/msg-type-dead-keys-compose-done.md`):
//!
//! - Only two-key sequences whose first key is *dead* are searched. That is
//!   where the default table's two-key sequences live (a bare apostrophe
//!   followed by `e` composes to nothing -- measured, not assumed), and it
//!   bounds the scan to dead-keys times keysyms, about half a millisecond
//!   on the dev VM. Three-key `Multi_key`-led sequences are not driven even
//!   on a layout that carries a Compose key: searching them multiplies the
//!   scan by the keysym count again, for sessions that explicitly opted into
//!   a key most layouts do not have.
//! - Only the active layout (xkb group) is ever consulted. Dead keys are
//!   collected from its keymap and both halves are planned in it, so a
//!   character that lives one group switch away stays refused -- the
//!   session's layout is user state, and silently switching it to type one
//!   character is worse than saying no.
//! - The table is built at most once per request, and never on the direct
//!   path: plain text pays nothing for this. The build is a few milliseconds
//!   of file parsing plus the scan above, on a request that already failed
//!   to type something -- comfortably inside the event-loop budget the
//!   per-request character cap was sized for.
//!
//! Nothing here allocates on the Rust heap: the map and both scratch lists
//! are fixed-size stack arrays. (Two bounded exceptions, both inherent to
//! the libraries: naming a keysym goes through a short-lived `String`, and
//! the compose table itself lives on the C heap. Both happen at most once
//! per request, and only on one that already hit an untypable character.)
//! A cap reached truncates rather than overflows -- a dropped sequence is a
//! loud refusal, never a wrong key -- and each cap carries a `debug_assert`
//! so a layout that outgrows one fails loudly in development.

use std::ffi::OsString;

use smithay::input::keyboard::{Keycode, Keysym, xkb};

use super::modifiers::{self, KeyPlan, ModifierKeys};

/// How many dead keys one layout may contribute sequences from. Real layouts
/// stay in the low teens (`de` carries 13); past this the rest are ignored,
/// which refuses rather than mistypes.
const MAX_DEADS: usize = 32;

/// How many distinct keysyms one layout may contribute as sequence halves.
/// `de` carries ~600 across all its levels; past this the rest are ignored.
const MAX_SYMS: usize = 1024;

/// How many composed characters one request may remember. `de` composes
/// ~380 dead-led pairs; past this the rest are ignored.
const MAX_SEQUENCES: usize = 512;

/// Loads the compose table for the session locale: the sequences a person
/// typing on this machine would get, which is what the toolkit on the other
/// end of the socket decodes with.
///
/// The locale is [`session_locale`] of the process environment. `None` when
/// no table compiles for it: compose is unavailable, and `type_text` keeps
/// the refusals it always gave. A fresh context per call, like the test
/// helpers build their keymaps with: this runs at most once per request, and
/// only on one that already needs it.
pub(super) fn table_from_session_locale() -> Option<xkb::compose::Table> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let locale = session_locale(|name| std::env::var_os(name));
    xkb::compose::Table::new_from_locale(&context, &locale, xkb::compose::COMPILE_NO_FLAGS).ok()
}

/// The locale compose resolves against: the first of `LC_ALL`, `LC_CTYPE`,
/// `LANG` that is set *and non-empty* -- the order `setlocale(LC_CTYPE, "")`
/// consults, and like it, treating an empty variable as unset (so
/// `LC_ALL= LANG=C.UTF-8` resolves to `C.UTF-8`, the locale clients run in)
/// -- defaulting to `C` when none is.
///
/// `var` looks a variable up by name; taking it as a parameter keeps the
/// choice testable without mutating the process environment.
fn session_locale(var: impl Fn(&str) -> Option<OsString>) -> OsString {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(var)
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("C"))
}

/// Every two-key dead-led sequence the active layout can produce, as the
/// character each one composes to plus the two keysyms to press.
///
/// Built once per `type_text` request by [`build_map`]; [`plan_sequence`]
/// answers characters from it. `entries` is unordered and may hold several
/// pairs for one character -- the first plannable one wins, in keymap order,
/// so the answer is deterministic per layout.
pub(super) struct SequenceMap {
    entries: [(char, Keysym, Keysym); MAX_SEQUENCES],
    len: usize,
}

/// Feeds every dead key the active layout carries followed by every keysym
/// on it through `table`, recording the pairs that compose to a single
/// character.
///
/// Both halves come from `layout` of `keymap` and nothing else: a dead key
/// from another group would plan a key the client decodes differently, which
/// is the exact failure this module exists to stop. `NoSymbol` levels are
/// skipped -- they press no character -- as is anything past the caps above
/// (see the module doc: truncation refuses, never mistypes).
pub(super) fn build_map(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    table: &xkb::compose::Table,
) -> SequenceMap {
    let mut map = SequenceMap {
        // Never read: only `entries[..len]` is ever consulted.
        entries: [('\0', Keysym::NoSymbol, Keysym::NoSymbol); MAX_SEQUENCES],
        len: 0,
    };
    let mut deads = [Keysym::NoSymbol; MAX_DEADS];
    let mut dead_len = 0usize;
    let mut syms = [Keysym::NoSymbol; MAX_SYMS];
    let mut sym_len = 0usize;
    for code in (keymap.min_keycode().raw()..=keymap.max_keycode().raw()).map(Keycode::new) {
        for level in 0..keymap.num_levels_for_key(code, layout) {
            for &sym in keymap.key_get_syms_by_level(code, layout, level) {
                if sym == Keysym::NoSymbol {
                    continue;
                }
                // One of two bounded heap allocations in this module (the
                // other is the one locale `OsString` `session_locale`
                // returns, once per fallback request -- an empty variable
                // it skips copies no bytes): naming
                // a keysym to test the `dead_` prefix, once per distinct
                // keysym per request that needs the fallback. Matching the
                // raw keysym range instead would avoid it but freeze
                // xkeyboard-config's current numbering into this code.
                if sym_len < MAX_SYMS && !syms[..sym_len].contains(&sym) {
                    if xkb::keysym_get_name(sym).starts_with("dead_") && dead_len < MAX_DEADS {
                        deads[dead_len] = sym;
                        dead_len += 1;
                    }
                    syms[sym_len] = sym;
                    sym_len += 1;
                }
            }
        }
    }
    debug_assert!(
        dead_len < MAX_DEADS,
        "a layout carries more dead keys than the sequence map holds"
    );
    debug_assert!(
        sym_len < MAX_SYMS,
        "a layout carries more distinct keysyms than the sequence map holds"
    );
    let mut state = xkb::compose::State::new(table, xkb::compose::STATE_NO_FLAGS);
    for &dead in &deads[..dead_len] {
        for &base in &syms[..sym_len] {
            state.reset();
            if state.feed(dead) == xkb::compose::FeedResult::Ignored {
                continue;
            }
            if state.status() != xkb::compose::Status::Composing {
                continue;
            }
            let _ = state.feed(base);
            if state.status() != xkb::compose::Status::Composed {
                continue;
            }
            // `keysym`, not `utf8`: a multi-character result has no single
            // keysym and reports none, which filters it out for free, and a
            // keysym without Unicode reports 0 below -- both shapes have no
            // single character `type_text` could be asked for.
            let Some(composed) = state.keysym() else {
                continue;
            };
            let raw = xkb::keysym_to_utf32(composed);
            let Some(character) = (raw != 0).then(|| char::from_u32(raw)).flatten() else {
                continue;
            };
            if map.len < MAX_SEQUENCES {
                map.entries[map.len] = (character, dead, base);
                map.len += 1;
            }
        }
    }
    debug_assert!(
        map.len < MAX_SEQUENCES,
        "a layout composes more dead-led pairs than the sequence map holds"
    );
    map
}

/// The two keypresses that type `character`: the dead key and its base, each
/// planned in `layout` of `keymap` the way a direct character would be.
///
/// Tries every recorded pair for the character in map order and returns the
/// first whose halves both plan -- a half the layout cannot hold (or a map
/// built for another layout, whose keysyms plan nowhere here) just means
/// the next pair is tried. `None` when no pair plans: the caller reports its
/// original direct-path refusal, which is still the honest answer.
/// `modifier_keys` is the caller's shared probe cache, as in
/// [`modifiers::plan`](super::modifiers).
pub(super) fn plan_sequence(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    map: &SequenceMap,
    character: char,
    modifier_keys: &mut Option<ModifierKeys>,
) -> Option<(KeyPlan, KeyPlan)> {
    map.entries[..map.len]
        .iter()
        .filter(|(composed, _, _)| *composed == character)
        .filter_map(|(_, dead, base)| {
            let dead = modifiers::plan(keymap, layout, *dead, modifier_keys).ok()?;
            let base = modifiers::plan(keymap, layout, *base, modifier_keys).ok()?;
            Some((dead, base))
        })
        .next()
}

#[cfg(test)]
mod tests;
