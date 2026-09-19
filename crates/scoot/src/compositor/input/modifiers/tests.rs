//! Tests for the keycode/level/modifier resolution the parent module does.
//!
//! These compile *real* keymaps out of xkeyboard-config -- the same data
//! scoot's own seat compiles at startup -- rather than hand-building a
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
//!
//! And two *groups*, for the same reason one layout was not enough. Every
//! test here used to compile a single-group keymap, where the group being
//! resolved is always group 0 -- which is exactly the case where a probe
//! that forgot to say which group it meant still gets the right answer. The
//! multi-group tests below are the ones that can tell those apart.

use super::*;
use scoot_ipc::Modifier;

/// Compiles one of xkeyboard-config's real layouts, with the default rules
/// and model (what `XkbConfig::default` -- and so scoot's own seat -- uses).
fn keymap(layout: &str, variant: &str) -> xkb::Keymap {
    keymap_with(layout, variant, None)
}

/// [`keymap`] plus an xkb `options` string, which is how a session asks for
/// a group-switching key (`grp:caps_toggle` and friends).
fn keymap_with(layout: &str, variant: &str, options: Option<&str>) -> xkb::Keymap {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        layout,
        variant,
        options.map(str::to_owned),
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .unwrap_or_else(|| panic!("xkeyboard-config should provide the `{layout}` layout"))
}

/// The layout single-group keymaps here are compiled with exactly one of.
const FIRST: xkb::LayoutIndex = 0;

/// The second group of the two-group keymaps below -- the one a session is
/// in after one press of its group-switch key, and the one nothing in this
/// module may confuse with [`FIRST`].
const SECOND: xkb::LayoutIndex = 1;

/// What [`plan`] came up with for one character, unwrapped for readability.
fn plan_for(keymap: &xkb::Keymap, character: char) -> KeyPlan {
    plan_in(keymap, FIRST, character)
        .unwrap_or_else(|error| panic!("`{character}` should be typable: {error:?}"))
}

/// [`plan_for`] against a chosen group, with the error kept.
fn plan_in(
    keymap: &xkb::Keymap,
    layout: xkb::LayoutIndex,
    character: char,
) -> Result<KeyPlan, Untypable> {
    let mut modifier_keys = None;
    plan(
        keymap,
        layout,
        xkb::utf32_to_keysym(character as u32),
        &mut modifier_keys,
    )
}

/// The keysym a client would decode from one [`KeyPlan`]: press the
/// modifiers, press the key, read what came out. The only question that
/// actually settles whether a plan is right -- every wrong answer this
/// module has produced looked plausible as a keycode.
///
/// The state is pinned to `layout` for the same reason [`ModifierKeys::probe`]
/// is: a fresh one starts in group 0, which on a two-group keymap decodes a
/// different character from the same keycode.
fn typed_sym(keymap: &xkb::Keymap, layout: xkb::LayoutIndex, plan: &KeyPlan) -> Keysym {
    let mut state = xkb::State::new(keymap);
    state.update_mask(0, 0, 0, 0, 0, layout);
    for &code in plan.modifiers.as_slice() {
        let _ = state.update_key(code, xkb::KeyDirection::Down);
    }
    let sym = state.key_get_one_sym(plan.code);
    for &code in plan.modifiers.as_slice().iter().rev() {
        let _ = state.update_key(code, xkb::KeyDirection::Up);
    }
    sym
}

/// Every keysym `code` carries, at every level -- how these tests name a key
/// without hard-coding evdev keycodes.
fn syms_of(keymap: &xkb::Keymap, code: Keycode) -> Vec<Keysym> {
    syms_of_in(keymap, FIRST, code)
}

/// [`syms_of`] against a chosen group.
fn syms_of_in(keymap: &xkb::Keymap, layout: xkb::LayoutIndex, code: Keycode) -> Vec<Keysym> {
    (0..keymap.num_levels_for_key(code, layout))
        .flat_map(|level| keymap.key_get_syms_by_level(code, layout, level))
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
    let probed = ModifierKeys::probe(&keymap, FIRST);
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
    assert_eq!(Some(*neo_held), ModifierKeys::probe(&neo, FIRST).keys[7]);
}

/// The invariant the whole probe rests on, checked against three real
/// layouts and both groups of a two-group one: a key it recorded depresses
/// exactly the one modifier it was recorded for, and leaves nothing behind
/// when it comes back up.
///
/// That second half is what keeps Caps Lock, Num Lock and
/// `ISO_Level3_Latch` out: they would "work" for one character and then
/// silently capitalize, or shift, everything typed afterwards -- a far worse
/// outcome than refusing the character.
///
/// The verifying state is pinned to the same group the probe ran in. Without
/// that it would be checking a different group's actions than the ones being
/// asserted about -- the very confusion this is guarding.
#[test]
fn every_probed_key_holds_exactly_its_own_modifier_and_releases_cleanly() {
    for (layout, variant, group) in [
        ("us", "", FIRST),
        ("de", "", FIRST),
        ("de", "neo", FIRST),
        ("de,de", "neo,", FIRST),
        ("de,de", "neo,", SECOND),
    ] {
        let keymap = keymap(layout, variant);
        let probed = ModifierKeys::probe(&keymap, group);
        let mut state = xkb::State::new(&keymap);
        state.update_mask(0, 0, 0, 0, 0, group);
        for (index, key) in probed.keys.iter().enumerate() {
            let Some(code) = *key else { continue };
            let _ = state.update_key(code, xkb::KeyDirection::Down);
            let what =
                format!("{layout}({variant}) group {group} key {code:?} for modifier {index}");
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

// -------------------------------------------------------------------------
// More than one group
// -------------------------------------------------------------------------

/// A two-group keymap of the shape a `de,de` + `grp:` session runs:
/// `de(neo)` as group 0, plain `de` as group 1. They reach their third level
/// -- where `@` lives on both -- from *different* keys, which is what makes
/// answering in the wrong group produce a wrong character rather than an
/// error.
fn two_groups() -> xkb::Keymap {
    keymap("de,de", "neo,")
}

/// The blocking bug this test exists for: the probe used to walk in group 0
/// whatever group `plan` was resolving in, so on this keymap `@` in group 1
/// planned the `q` key (right) plus group *0*'s third-level key (wrong).
/// Pressing that pair typed `#q` -- neither key doing what was intended, and
/// no error anywhere.
#[test]
fn a_modifier_is_probed_in_the_group_the_character_is_resolved_in() {
    let two = two_groups();
    let at = xkb::utf32_to_keysym('@' as u32);
    let first = plan_in(&two, FIRST, '@').expect("`@` is on group 0");
    let second = plan_in(&two, SECOND, '@').expect("`@` is on group 1");
    // What a client decodes is the whole question, and it is the one the
    // old code got wrong while looking right at every intermediate step.
    assert_eq!(
        typed_sym(&two, FIRST, &first),
        at,
        "group 0 should type `@`"
    );
    assert_eq!(
        typed_sym(&two, SECOND, &second),
        at,
        "group 1 should type `@`"
    );
    // The groups genuinely need different keys held, which is what makes
    // answering in the wrong one produce a character instead of an error.
    assert_ne!(
        first.modifiers, second.modifiers,
        "the two groups reach their third level from different keys"
    );
    // The reported symptom, reproduced exactly: group 0's third-level key
    // is an ordinary `#` key in group 1, so the group-0 answer used in
    // group 1 holds nothing and types `#` and then `q`.
    let wrong = KeyPlan {
        code: second.code,
        modifiers: first.modifiers,
    };
    let [group_zero_modifier] = first.modifiers.as_slice() else {
        panic!("`@` on `de(neo)` needs exactly one modifier held");
    };
    assert_eq!(
        typed_sym(
            &two,
            SECOND,
            &KeyPlan {
                code: *group_zero_modifier,
                modifiers: HeldKeys::default(),
            }
        ),
        Keysym::numbersign,
        "group 0's third-level key is a plain `#` key in group 1"
    );
    assert_eq!(
        typed_sym(&two, SECOND, &wrong),
        Keysym::q,
        "and the key `@` sits on falls through to its unmodified level"
    );
}

/// The same fix, against the second way the walk could drift: a `grp:`
/// option puts a group-switching action on an ordinary key, so pressing
/// every key in turn moves the walk itself between groups. Whether that
/// corrupted the table used to depend on where in the keycode order the
/// toggle sat relative to the modifier keys, which is exactly the kind of
/// thing to pin down with a test rather than reason about.
#[test]
fn a_group_switching_key_encountered_mid_walk_does_not_corrupt_the_probe() {
    let at = xkb::utf32_to_keysym('@' as u32);
    for option in [
        "grp:caps_toggle",
        "grp:sclk_toggle",
        "grp:menu_toggle",
        "grp:shift_caps_toggle",
        // Changes group only *while held*, so the walk visits a key whose
        // press and release land in different groups.
        "grp:caps_switch",
        "grp:caps_select",
        "grp:alt_shift_toggle",
    ] {
        let two = keymap_with("de,de", "neo,", Some(option));
        for group in [FIRST, SECOND] {
            let plan = plan_in(&two, group, '@')
                .unwrap_or_else(|error| panic!("`{option}` group {group}: `@` refused: {error:?}"));
            assert_eq!(
                typed_sym(&two, group, &plan),
                at,
                "`{option}`: group {group} should still type `@`"
            );
        }
    }
}

/// The probe is cached across a whole typed string while the layout is
/// re-read per character, so the cache has to know which group it answered
/// for. A stale one is the same wrong-group bug arriving by a different
/// route.
#[test]
fn a_probe_taken_in_another_group_is_not_reused() {
    let two = two_groups();
    let at = xkb::utf32_to_keysym('@' as u32);
    let mut cache = None;
    // Fills the cache from group 0...
    let first = plan(&two, FIRST, at, &mut cache).expect("`@` is on group 0");
    assert_eq!(Ok(first), plan_in(&two, FIRST, '@'));
    // ...which must not be the answer group 1 gets from the same cache.
    let second = plan(&two, SECOND, at, &mut cache).expect("`@` is on group 1");
    assert_eq!(Ok(second), plan_in(&two, SECOND, '@'));
    assert_eq!(typed_sym(&two, SECOND, &second), at);
    assert_ne!(first.modifiers, second.modifiers);
    // And going back re-probes rather than keeping group 1's table.
    assert_eq!(plan(&two, FIRST, at, &mut cache), Ok(first));
}

// -------------------------------------------------------------------------
// Across real layouts
// -------------------------------------------------------------------------

/// The layouts the sweep below runs, and -- measured, not assumed -- exactly
/// which printable ASCII characters each one refuses.
///
/// Chosen to span the ways a Latin layout can differ: dead keys (`de`, `es`,
/// the Nordics), AltGr levels (`de`, `pl`), digits behind Shift (`fr`), an
/// `us` variant that makes punctuation dead (`us(intl)`), and one that
/// rearranges nearly everything (`de(neo)`).
///
/// The refusals are what keeps the documentation honest: "all of ASCII on
/// any Latin layout" is false, and `~` -- shell paths, globs, regexes -- is
/// among the casualties on four of these. If an xkeyboard-config update
/// moves one of these characters, this table is what has to be updated
/// alongside the claim in
/// `docs/backlog/resolved/msg-type-dead-keys-compose-done.md`.
const LATIN_LAYOUTS: [(&str, &str, &str); 14] = [
    ("us", "", ""),
    ("us", "intl", ""),
    ("gb", "", ""),
    ("de", "", "^`"),
    ("de", "neo", ""),
    ("fr", "", ""),
    ("fr", "oss", ""),
    ("es", "", "^`"),
    ("it", "", ""),
    ("pt", "", "^`~"),
    ("se", "", "^`~"),
    ("no", "", "^`~"),
    ("dk", "", "^`~"),
    ("pl", "", ""),
];

/// The invariant that actually matters, swept over every printable ASCII
/// character on fourteen real layouts: whatever [`plan`] plans, pressing it
/// decodes back to the character that was asked for. Never a different one.
///
/// Refusing is always allowed -- some of these characters really are
/// unreachable as a single keypress (see the test below) -- so this asserts
/// "planned implies correct", which is exactly the property the original bug
/// broke and the property a wrong-group probe breaks again.
#[test]
fn every_planned_character_decodes_back_to_itself_on_every_latin_layout() {
    for (layout, variant, _) in LATIN_LAYOUTS {
        let keymap = keymap(layout, variant);
        for character in ' '..='~' {
            let Ok(plan) = plan_in(&keymap, FIRST, character) else {
                continue;
            };
            assert_eq!(
                typed_sym(&keymap, FIRST, &plan),
                xkb::utf32_to_keysym(character as u32),
                "{layout}({variant}) planned `{character}` as {plan:?}, which types something else"
            );
        }
    }
}

/// What the sweep above deliberately tolerates, pinned down so the docs can
/// say it accurately: "every ASCII character on any Latin layout" is not
/// true. A layout that puts a diacritic on a key makes that key *dead* --
/// it carries `dead_tilde`, not `asciitilde`, and no combination of held
/// modifiers produces the plain character from it.
///
/// Refusal is the right answer (the alternative is typing an accented letter
/// the caller never asked for), but it is layout-dependent, and `~` in
/// particular matters: shell paths, globs and regexes are full of it.
#[test]
fn exactly_the_dead_key_characters_are_refused_on_each_latin_layout() {
    for (layout, variant, expected) in LATIN_LAYOUTS {
        let keymap = keymap(layout, variant);
        let refused: String = (' '..='~')
            .filter(|&character| plan_in(&keymap, FIRST, character).is_err())
            .collect();
        assert_eq!(
            refused, expected,
            "{layout}({variant}) refuses a different set of ASCII than recorded"
        );
        for character in refused.chars() {
            assert_eq!(
                plan_in(&keymap, FIRST, character),
                Err(Untypable::NoKey),
                "{layout}({variant}) should refuse `{character}` as absent, not as unholdable"
            );
        }
    }
}

// -------------------------------------------------------------------------
// Naming a key rather than a character
// -------------------------------------------------------------------------

/// What `scoot msg key exclam` should get: a refusal. The key carrying
/// `exclam` on a US layout is the `1` key, and pressing it with nothing held
/// -- which is all `press` promises to do -- types `1`.
#[test]
fn a_keysym_only_reachable_above_level_zero_is_not_offered_as_a_key_to_press() {
    let us = keymap("us", "");
    for name in [
        Keysym::exclam,
        Keysym::at,
        Keysym::asciitilde,
        Keysym::underscore,
        Keysym::question,
        Keysym::colon,
        Keysym::bar,
        Keysym::braceleft,
        Keysym::A,
    ] {
        assert_eq!(
            named_key(&us, FIRST, name),
            NamedKey::OnlyModified,
            "{name:?} is only above level 0 on `us` and must not be pressed bare"
        );
    }
}

/// The everyday case, and the two ends of it: a key that types itself, and a
/// modifier key, both of which `press` resolves through the same lookup.
#[test]
fn a_keysym_on_the_unmodified_level_is_the_key_that_carries_it() {
    let us = keymap("us", "");
    for (name, expected) in [
        (Keysym::a, Keysym::a),
        (Keysym::_1, Keysym::_1),
        (Keysym::Return, Keysym::Return),
        (Keysym::Shift_L, Keysym::Shift_L),
        (Keysym::Control_L, Keysym::Control_L),
        (Keysym::Super_L, Keysym::Super_L),
    ] {
        let NamedKey::Unmodified(code) = named_key(&us, FIRST, name) else {
            panic!("{name:?} should be on the unmodified level of a US layout");
        };
        assert!(
            keymap_level_zero(&us, FIRST, code).contains(&expected),
            "{name:?} resolved to a key that does not carry it unmodified"
        );
    }
}

/// A key genuinely absent from the layout is a different answer from one
/// that is present but out of reach -- `press` reports them differently,
/// because the fix for each is different.
#[test]
fn a_keysym_absent_from_the_layout_is_reported_as_absent() {
    let us = keymap("us", "");
    for name in [Keysym::eacute, Keysym::ssharp, Keysym::Greek_pi] {
        assert_eq!(named_key(&us, FIRST, name), NamedKey::Absent);
    }
}

/// The same question, asked of the group a session is actually in: a name
/// pressable in one group can be out of reach in another, and answering
/// from the wrong one is how `press` would press the wrong key.
#[test]
fn a_named_key_is_resolved_in_the_active_group() {
    let two = two_groups();
    // `y` sits on the unmodified level of plain `de` (group 1) but nowhere
    // near it on `de(neo)` (group 0), which rearranges the alphabet.
    let NamedKey::Unmodified(second) = named_key(&two, SECOND, Keysym::y) else {
        panic!("`y` is on plain `de`'s unmodified level");
    };
    let NamedKey::Unmodified(first) = named_key(&two, FIRST, Keysym::y) else {
        panic!("`y` is on `de(neo)`'s unmodified level too, on another key");
    };
    assert_ne!(
        first, second,
        "the two groups put `y` on different keys, so the answers must differ"
    );
    assert_eq!(
        named_key(&keymap("de", ""), FIRST, Keysym::y),
        NamedKey::Unmodified(second),
        "group 1 is `de`, and must be answered as `de`"
    );
}

/// The keysyms `code` carries with nothing held.
fn keymap_level_zero(keymap: &xkb::Keymap, layout: xkb::LayoutIndex, code: Keycode) -> Vec<Keysym> {
    keymap.key_get_syms_by_level(code, layout, 0).to_vec()
}

/// `hold` is handed whatever `key_get_mods_for_level` returns. Nothing
/// observed sets a bit past the real modifiers, but the bound is not this
/// module's to enforce, so the lookup must not index blindly.
#[test]
fn a_modifier_past_the_real_ones_is_unproducible_rather_than_a_panic() {
    let probed = ModifierKeys::probe(&keymap("us", ""), FIRST);
    assert_eq!(probed.hold(1 << 20), None);
    assert_eq!(probed.hold(xkb::ModMask::MAX), None);
    // The empty mask is the everyday case: hold nothing.
    assert_eq!(probed.hold(0).map(|held| held.len), Some(0));
}

// -------------------------------------------------------------------------
// Modifiers that moved off their `_L` key (`msg-key-modifier-resolution`)
// -------------------------------------------------------------------------

/// The ticket's concrete failure, at the keymap level: `grp:lshift_toggle`
/// turns the left Shift into a group-switch key, so no key in the layout
/// carries `Shift_L` at any level -- the hard-coded lookup `resolve_combo`
/// used answers `Absent`, and every `msg key shift+...` combo is refused --
/// while the probe still finds the real Shift on the right-hand key.
///
/// Single-group `us` reproduces it: the toggle does not need a second group
/// to take `Shift_L` off the keymap.
#[test]
fn shift_is_still_holdable_when_the_left_shift_becomes_a_group_toggle() {
    let toggled = keymap_with("us", "", Some("grp:lshift_toggle"));
    assert_eq!(
        named_key(&toggled, FIRST, Keysym::Shift_L),
        NamedKey::Absent,
        "with `grp:lshift_toggle`, no key carries `Shift_L`, which is what the hard-coded lookup refused on"
    );
    let probed = ModifierKeys::probe(&toggled, FIRST);
    let index = toggled.mod_get_index("Shift") as usize;
    let shift = probed.keys[index].expect("Shift must still be holdable");
    assert!(
        syms_of(&toggled, shift).contains(&Keysym::Shift_R),
        "the remaining Shift should be the right-hand key"
    );
}

/// The `ctrl` half of the same bug: `grp:lctrl_toggle` takes `Control_L`
/// off the keymap, leaving the real Control on the right-hand key only.
#[test]
fn control_is_still_holdable_when_the_left_control_becomes_a_group_toggle() {
    let toggled = keymap_with("us", "", Some("grp:lctrl_toggle"));
    assert_eq!(
        named_key(&toggled, FIRST, Keysym::Control_L),
        NamedKey::Absent,
        "with `grp:lctrl_toggle`, no key carries `Control_L`, which is what the hard-coded lookup refused on"
    );
    let probed = ModifierKeys::probe(&toggled, FIRST);
    let index = toggled.mod_get_index("Control") as usize;
    let control = probed.keys[index].expect("Control must still be holdable");
    assert!(
        syms_of(&toggled, control).contains(&Keysym::Control_R),
        "the remaining Control should be the right-hand key"
    );
}

/// Both shifts present: the probe records the lowest keycode first, which is
/// the left-hand key -- the same choice a person makes without thinking.
/// Pinned so a future change to the walk order is a deliberate one.
#[test]
fn with_both_shifts_present_the_probe_holds_the_left_hand_key() {
    let us = keymap("us", "");
    let probed = ModifierKeys::probe(&us, FIRST);
    let index = us.mod_get_index("Shift") as usize;
    let shift = probed.keys[index].expect("Shift is holdable on `us`");
    assert!(
        syms_of(&us, shift).contains(&Keysym::Shift_L),
        "with both present, the probed Shift should be the left-hand key"
    );
}

/// The name table behind [`modifier_key`]: each IPC modifier means the real
/// modifier clients decode under the same name. `Shift`/`Control` are xkb's
/// own; `Alt` is `Mod1` and `Super` is `Mod4` -- the names Smithay's own
/// `ModifiersState` reads, so the probed key sets precisely what a toolkit
/// calls that modifier. A wrong arm here refuses (no key for that real
/// modifier) or holds the wrong key, which the live `alt+Tab` / `super+h`
/// test in the parent module would catch as wrong text.
#[test]
fn ipc_modifiers_name_the_modifiers_clients_decode() {
    assert_eq!(real_mod_name(Modifier::Ctrl), xkb::MOD_NAME_CTRL);
    assert_eq!(real_mod_name(Modifier::Shift), xkb::MOD_NAME_SHIFT);
    assert_eq!(real_mod_name(Modifier::Alt), xkb::MOD_NAME_ALT);
    assert_eq!(real_mod_name(Modifier::Super), xkb::MOD_NAME_LOGO);
}

/// A modifier with no holdable key resolves to nothing -- the `None` the
/// caller turns into its honest "no key" refusal. No stock xkeyboard-config
/// layout in the sweep above drops a whole modifier (even the toggles leave
/// the other hand), so this pins the path with a probe that found nothing
/// rather than with a layout that cannot be compiled here.
#[test]
fn a_modifier_with_no_holdable_key_resolves_to_nothing() {
    let us = keymap("us", "");
    let mut empty = Some(ModifierKeys {
        layout: FIRST,
        keys: [None; REAL_MODIFIERS],
    });
    for modifier in [
        Modifier::Ctrl,
        Modifier::Shift,
        Modifier::Alt,
        Modifier::Super,
    ] {
        assert_eq!(
            modifier_key(&us, FIRST, &mut empty, modifier),
            None,
            "{modifier:?} should resolve to nothing against an empty probe"
        );
    }
    // ... while the same keymap with a real probe resolves all four, so the
    // `None` above is the empty table, not the lookup.
    let mut probed = None;
    for modifier in [
        Modifier::Ctrl,
        Modifier::Shift,
        Modifier::Alt,
        Modifier::Super,
    ] {
        assert!(
            modifier_key(&us, FIRST, &mut probed, modifier).is_some(),
            "{modifier:?} should resolve on a plain `us` layout"
        );
    }
}
