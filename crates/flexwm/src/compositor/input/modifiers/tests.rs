//! Tests for the keycode/level/modifier resolution the parent module does.
//!
//! These compile *real* keymaps out of xkeyboard-config -- the same data
//! flexwm's own seat compiles at startup -- rather than hand-building a
//! fake one. The bug this module exists to fix was precisely a
//! misunderstanding of what real keymap data says (Smithay's
//! `raw_syms_for_key_in_layout` is level 0 only, so it can never report a
//! keysym that needs a modifier), and a hand-written fixture would have
//! encoded the same misunderstanding and passed.
//!
//! Three layouts, because `us` alone cannot tell a correct implementation
//! from a lucky one: every shifted character there needs Shift, so a
//! hard-coded "hold Shift" would pass all of it. `de` puts `@` three levels
//! up behind AltGr, and `de(neo)` puts that same third-level modifier on a
//! different key again -- neither of which anything here names.

use super::*;

/// Compiles one of xkeyboard-config's real layouts, with the default rules
/// and model (what `XkbConfig::default` -- and so flexwm's own seat -- uses).
fn keymap(layout: &str, variant: &str) -> xkb::Keymap {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        variant,
        None,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .unwrap_or_else(|| panic!("xkeyboard-config should provide the `{layout}` layout"))
}

/// The layout every keymap here is compiled with exactly one of.
const FIRST: xkb::LayoutIndex = 0;

/// What [`plan`] came up with for one character, unwrapped for readability.
fn plan_for(keymap: &xkb::Keymap, character: char) -> KeyPlan {
    let mut modifier_keys = None;
    plan(
        keymap,
        FIRST,
        xkb::utf32_to_keysym(character as u32),
        &mut modifier_keys,
    )
    .unwrap_or_else(|error| panic!("`{character}` should be typable: {error:?}"))
}

/// Every keysym `code` carries, at every level -- how these tests name a key
/// without hard-coding evdev keycodes.
fn syms_of(keymap: &xkb::Keymap, code: Keycode) -> Vec<Keysym> {
    (0..keymap.num_levels_for_key(code, FIRST))
        .flat_map(|level| keymap.key_get_syms_by_level(code, FIRST, level))
        .copied()
        .collect()
}

#[test]
fn an_unshifted_character_holds_nothing() {
    let keymap = keymap("us", "");
    for character in ['a', 'z', '1', '-', '=', '[', ';', ',', '/'] {
        let plan = plan_for(&keymap, character);
        assert_eq!(
            plan.modifiers.as_slice(),
            [],
            "`{character}` should need no modifier held"
        );
    }
}

/// The actual reported bug: every one of these came out as its unshifted
/// twin, silently.
#[test]
fn every_shifted_character_holds_shift() {
    let keymap = keymap("us", "");
    let shift = *plan_for(&keymap, 'A')
        .modifiers
        .as_slice()
        .first()
        .expect("a capital letter needs a modifier held");
    assert!(
        syms_of(&keymap, shift).contains(&Keysym::Shift_L),
        "the key held for `A` should be Shift"
    );
    for character in [
        'A', 'Z', '!', '_', '?', '~', ':', '|', '@', '#', '%', '^', '&', '*', '(', ')', '+', '{',
        '}', '<', '>', '"',
    ] {
        assert_eq!(
            plan_for(&keymap, character).modifiers.as_slice(),
            [shift],
            "`{character}` should be typed with Shift held"
        );
    }
}

/// The key is the *same* key as the unshifted character's -- a capital comes
/// from the letter key plus Shift, not from some other key that happens to
/// carry the keysym.
#[test]
fn a_capital_presses_the_lowercase_letters_own_key() {
    let keymap = keymap("us", "");
    for (lower, upper) in [('a', 'A'), ('q', 'Q'), ('z', 'Z')] {
        assert_eq!(
            plan_for(&keymap, upper).code,
            plan_for(&keymap, lower).code,
            "`{upper}` should press the `{lower}` key"
        );
    }
}

/// Caps Lock reaches an alphabetic key's upper level too, and
/// `key_get_mods_for_level` offers it as an alternative to Shift. Pressing
/// and releasing it around a character would leave it *on* for everything
/// afterwards, so it must never be the key chosen -- and the probe must not
/// record it as able to hold anything in the first place.
#[test]
fn caps_lock_is_never_held_to_type_a_capital() {
    let keymap = keymap("us", "");
    let held = plan_for(&keymap, 'A').modifiers;
    for &code in held.as_slice() {
        let syms = syms_of(&keymap, code);
        assert!(
            !syms.contains(&Keysym::Caps_Lock),
            "typing `A` must not press Caps Lock"
        );
    }
    let probed = ModifierKeys::probe(&keymap);
    // `Lock` is real modifier index 1; nothing may claim to hold it.
    assert_eq!(probed.keys[1], None, "a locking key is not a holdable one");
    assert!(
        probed.keys[0].is_some(),
        "Shift is holdable and must have been found"
    );
}

/// The probe walks the whole keymap, so it must not run for text that needs
/// no modifiers at all -- which is most text.
#[test]
fn the_keymap_is_only_probed_once_a_modifier_is_actually_needed() {
    let keymap = keymap("us", "");
    let mut modifier_keys = None;
    for character in ['h', 'e', 'l', 'l', 'o'] {
        plan(
            &keymap,
            FIRST,
            xkb::utf32_to_keysym(character as u32),
            &mut modifier_keys,
        )
        .expect("lowercase text is typable");
    }
    assert!(
        modifier_keys.is_none(),
        "lowercase text should never pay for the keymap walk"
    );
    plan(
        &keymap,
        FIRST,
        xkb::utf32_to_keysym('H' as u32),
        &mut modifier_keys,
    )
    .expect("a capital is typable");
    assert!(
        modifier_keys.is_some(),
        "the first shifted character should have built the table"
    );
}

/// A character no key in the layout carries is reported as such, rather than
/// as some other character.
#[test]
fn a_character_absent_from_the_layout_is_refused() {
    let keymap = keymap("us", "");
    let mut modifier_keys = None;
    // Not `€`: the evdev keymap really does carry `EuroSign`, on a key no
    // physical US keyboard has -- which is a fine key to press, and exactly
    // the sort of thing this module asks the keymap about rather than
    // assuming.
    for character in ['é', 'ß', 'π', '中'] {
        assert_eq!(
            plan(
                &keymap,
                FIRST,
                xkb::utf32_to_keysym(character as u32),
                &mut modifier_keys
            ),
            Err(Untypable::NoKey),
            "`{character}` is not on a US layout"
        );
    }
}

/// A layout that needs more than Shift. `@` sits on the third level of the
/// `q` key on a German layout, behind AltGr -- a case a Shift-only
/// implementation gets wrong in the same silent way the original bug did.
#[test]
fn a_third_level_character_holds_that_layouts_altgr() {
    let keymap = keymap("de", "");
    let plan = plan_for(&keymap, '@');
    let [altgr] = plan.modifiers.as_slice() else {
        panic!("`@` on a German layout needs exactly one modifier held");
    };
    assert!(
        syms_of(&keymap, *altgr).contains(&Keysym::ISO_Level3_Shift),
        "the key held for `@` should be AltGr"
    );
    assert!(
        syms_of(&keymap, plan.code).contains(&Keysym::q),
        "`@` is on the `q` key"
    );
    // ...and the same layout's capitals still just need Shift.
    let shift = plan_for(&keymap, 'Q').modifiers;
    let [shift] = shift.as_slice() else {
        panic!("a capital needs exactly one modifier held");
    };
    assert!(syms_of(&keymap, *shift).contains(&Keysym::Shift_L));
    assert_ne!(*shift, *altgr);
}

/// The same character on a layout that puts the *same* modifier on a
/// different key: `de` reaches `@` by holding the right-hand Alt, `de(neo)`
/// by holding the key in the `#` position. Which key it is comes out of the
/// keymap, so both are right without either being named anywhere.
#[test]
fn a_third_level_character_follows_the_layouts_own_modifier_key() {
    let de = keymap("de", "");
    let neo = keymap("de", "neo");
    let de_plan = plan_for(&de, '@');
    let [de_held] = de_plan.modifiers.as_slice() else {
        panic!("`@` on `de` needs exactly one modifier held");
    };
    let neo_plan = plan_for(&neo, '@');
    let [neo_held] = neo_plan.modifiers.as_slice() else {
        panic!("`@` on `de(neo)` needs exactly one modifier held");
    };
    assert_ne!(
        de_held, neo_held,
        "the two layouts put their third-level modifier on different keys"
    );
    // Both are a third-level shift, and on `de(neo)` it is the one the
    // probe recorded for Mod5 (real modifier index 7) -- the point being
    // that neither the key nor the modifier index was assumed anywhere.
    assert!(syms_of(&de, *de_held).contains(&Keysym::ISO_Level3_Shift));
    assert!(syms_of(&neo, *neo_held).contains(&Keysym::ISO_Level3_Shift));
    assert_eq!(Some(*neo_held), ModifierKeys::probe(&neo).keys[7]);
}

/// The invariant the whole probe rests on, checked against three real
/// layouts: a key it recorded depresses exactly the one modifier it was
/// recorded for, and leaves nothing behind when it comes back up.
///
/// That second half is what keeps Caps Lock, Num Lock and
/// `ISO_Level3_Latch` out: they would "work" for one character and then
/// silently capitalize, or shift, everything typed afterwards -- a far worse
/// outcome than refusing the character.
#[test]
fn every_probed_key_holds_exactly_its_own_modifier_and_releases_cleanly() {
    for (layout, variant) in [("us", ""), ("de", ""), ("de", "neo")] {
        let keymap = keymap(layout, variant);
        let probed = ModifierKeys::probe(&keymap);
        let mut state = xkb::State::new(&keymap);
        for (index, key) in probed.keys.iter().enumerate() {
            let Some(code) = *key else { continue };
            let _ = state.update_key(code, xkb::KeyDirection::Down);
            let what = format!("{layout}({variant}) key {code:?} for modifier {index}");
            assert_eq!(
                state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
                1 << index,
                "{what} should depress exactly that modifier"
            );
            assert_eq!(
                state.serialize_mods(xkb::STATE_MODS_LATCHED),
                0,
                "{what} latches"
            );
            assert_eq!(
                state.serialize_mods(xkb::STATE_MODS_LOCKED),
                0,
                "{what} locks"
            );
            let _ = state.update_key(code, xkb::KeyDirection::Up);
            assert_eq!(
                state.serialize_mods(xkb::STATE_MODS_EFFECTIVE),
                0,
                "{what} left the keyboard modified after its release"
            );
        }
    }
}

/// `hold` is handed whatever `key_get_mods_for_level` returns. Nothing
/// observed sets a bit past the real modifiers, but the bound is not this
/// module's to enforce, so the lookup must not index blindly.
#[test]
fn a_modifier_past_the_real_ones_is_unproducible_rather_than_a_panic() {
    let probed = ModifierKeys::probe(&keymap("us", ""));
    assert_eq!(probed.hold(1 << 20), None);
    assert_eq!(probed.hold(xkb::ModMask::MAX), None);
    // The empty mask is the everyday case: hold nothing.
    assert_eq!(probed.hold(0).map(|held| held.len), Some(0));
}
