//! Tests for the dead-key fallback.
//!
//! The compose table here is built from an inline buffer, not the session
//! locale, so these mean the same thing on any machine: what is asserted is
//! which *keymap* sequences the scan finds and plans, with the table held
//! fixed. The one thing this cannot pin is which sequences the session
//! table carries -- that is the end-to-end tests' job, against the real
//! locale on real hardware.

use super::*;

/// The sequences these tests may use: exactly the ones the end-to-end tests
/// need, plus one accented capital. Deliberately small rather than the
/// session's full table, so a missing entry here is a missing test, not a
/// machine without a locale installed.
const TEST_COMPOSE: &str = r#"
<dead_acute> <e> : "é" eacute
<dead_acute> <E> : "É" Eacute
<dead_acute> <space> : "'" apostrophe
<dead_grave> <space> : "`" grave
<dead_circumflex> <space> : "^" asciicircum
<dead_tilde> <space> : "~" asciitilde
"#;

/// Compiles [`TEST_COMPOSE`]. Panics rather than returning `None`: a table
/// that does not compile is a broken test, not a skipped one.
fn table() -> xkb::compose::Table {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::compose::Table::new_from_buffer(
        &context,
        TEST_COMPOSE,
        "en_US.UTF-8",
        xkb::compose::FORMAT_TEXT_V1,
        xkb::compose::COMPILE_NO_FLAGS,
    )
    .expect("the inline compose table should compile")
}

/// Compiles one of xkeyboard-config's real layouts, the way
/// `modifiers/tests.rs` does (kept local: that helper lives in a private
/// test module this one cannot reach, and it is ten lines).
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

const FIRST: xkb::LayoutIndex = 0;

/// The keysym pressing `plan`'s key (with its modifiers held) produces in
/// `layout` -- what the client on the other end decodes. The only question
/// that settles whether a planned half is the right key.
fn pressed_sym(keymap: &xkb::Keymap, layout: xkb::LayoutIndex, plan: &KeyPlan) -> Keysym {
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

/// `é` on `de`: the dead key plus its base, found in the map and pressing
/// exactly the keysyms the sequence names.
#[test]
fn a_dead_key_plus_its_base_resolves_to_the_composed_character() {
    let de = keymap("de", "");
    let table = table();
    let map = build_map(&de, FIRST, &table);
    let Some((dead, base)) = plan_sequence(&de, FIRST, &map, 'é', &mut None) else {
        panic!("`é` should resolve to a dead-key sequence on `de`");
    };
    assert_eq!(
        pressed_sym(&de, FIRST, &dead),
        Keysym::dead_acute,
        "`é` should press dead_acute first"
    );
    assert_eq!(
        pressed_sym(&de, FIRST, &base),
        Keysym::e,
        "`é` should press `e` second"
    );
}

/// `us` carries no dead keys at all (measured in the ticket's probe), so its
/// map is empty and `é` has no sequence there -- the refusal the end-to-end
/// test pins.
#[test]
fn a_layout_with_no_dead_keys_builds_an_empty_map() {
    let us = keymap("us", "");
    let table = table();
    let map = build_map(&us, FIRST, &table);
    assert_eq!(map.len, 0, "a layout with no dead keys composes nothing");
    assert_eq!(
        plan_sequence(&us, FIRST, &map, 'é', &mut None),
        None,
        "`é` should have no sequence on plain `us`"
    );
}

/// The ASCII gaps from the ticket: `~` on `se`, `^` and `` ` `` on `de`,
/// each via its dead key plus space -- the keys a person would press.
#[test]
fn dead_plus_space_covers_the_dead_ascii_gaps() {
    let table = table();
    let se = keymap("se", "");
    let se_map = build_map(&se, FIRST, &table);
    let (dead, base) =
        plan_sequence(&se, FIRST, &se_map, '~', &mut None).expect("`~` should resolve on `se`");
    assert_eq!(pressed_sym(&se, FIRST, &dead), Keysym::dead_tilde);
    assert_eq!(pressed_sym(&se, FIRST, &base), Keysym::space);
    let de = keymap("de", "");
    let de_map = build_map(&de, FIRST, &table);
    let (dead, base) =
        plan_sequence(&de, FIRST, &de_map, '^', &mut None).expect("`^` should resolve on `de`");
    assert_eq!(pressed_sym(&de, FIRST, &dead), Keysym::dead_circumflex);
    assert_eq!(pressed_sym(&de, FIRST, &base), Keysym::space);
    let (dead, base) =
        plan_sequence(&de, FIRST, &de_map, '`', &mut None).expect("`` ` `` should resolve on `de`");
    assert_eq!(pressed_sym(&de, FIRST, &dead), Keysym::dead_grave);
    assert_eq!(pressed_sym(&de, FIRST, &base), Keysym::space);
}

/// Every recorded sequence replays to its character through the table that
/// built it: feed the dead key's keysym, feed the base's, read back the
/// composition. The self-consistency half -- the client half is the
/// end-to-end tests'.
#[test]
fn every_recorded_sequence_replays_to_its_character() {
    let de = keymap("de", "");
    let table = table();
    let map = build_map(&de, FIRST, &table);
    assert!(
        map.len > 0,
        "`de` should compose at least one dead-led pair from the test table"
    );
    let mut state = xkb::compose::State::new(&table, xkb::compose::STATE_NO_FLAGS);
    for (character, dead, base) in map.entries[..map.len].iter() {
        state.reset();
        let _ = state.feed(*dead);
        assert_eq!(
            state.status(),
            xkb::compose::Status::Composing,
            "{character}’s dead key should start a sequence"
        );
        let _ = state.feed(*base);
        assert_eq!(
            state.status(),
            xkb::compose::Status::Composed,
            "{character}’s pair should compose"
        );
        assert_eq!(
            state.utf8().as_deref(),
            Some(character.to_string()).as_deref(),
            "{character}’s pair should compose back to it"
        );
    }
}

/// A character the table never heard of has no sequence, however many dead
/// keys the layout carries. (`α`: no dead-led pair in the test table yields
/// it, and the table -- unlike the session's -- cannot gain one.)
#[test]
fn an_unlisted_character_has_no_sequence() {
    let de = keymap("de", "");
    let table = table();
    let map = build_map(&de, FIRST, &table);
    assert_eq!(
        plan_sequence(&de, FIRST, &map, 'α', &mut None),
        None,
        "`α` should have no dead-led sequence"
    );
}

/// Every ASCII character the direct path refuses on each of the fourteen
/// Latin layouts `modifiers/tests.rs` sweeps resolves through the fallback
/// instead: the refused characters there are exactly the dead keys
/// (`dead_circumflex`/`dead_grave` for `^`/`` ` ``, `dead_tilde` for `~`),
/// and dead-plus-space is in every session table. Pinned here so
/// `docs/ipc.md` can claim all of printable ASCII on all fourteen, with the
/// table above standing in for the session's.
///
/// The table pairing is load-bearing: each refused character needs its
/// dead-plus-space pair *in the table*. A session locale whose table lacked
/// one would refuse that character the way it always did -- loud, not
/// wrong.
#[test]
fn every_dead_ascii_gap_resolves_on_every_latin_layout() {
    const LAYOUTS: [(&str, &str, &str); 14] = [
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
    let table = table();
    for (layout, variant, refused) in LAYOUTS {
        let keymap = keymap(layout, variant);
        let map = build_map(&keymap, FIRST, &table);
        for character in refused.chars() {
            assert!(
                plan_sequence(&keymap, FIRST, &map, character, &mut None).is_some(),
                "`{character}` should resolve through the fallback on `{layout}({variant})`"
            );
        }
    }
}

/// A map built for one layout degrades to refusal on another, rather than
/// typing its keys: `de`'s `é` sequence planned on `us`, where `dead_acute`
/// is nowhere, is `None` -- the caller then reports its original refusal.
/// This is what makes a mid-string layout change (or any stale map) safe.
#[test]
fn a_sequence_from_another_layout_plans_to_nothing() {
    let de = keymap("de", "");
    let us = keymap("us", "");
    let table = table();
    let map = build_map(&de, FIRST, &table);
    assert_eq!(
        plan_sequence(&us, FIRST, &map, 'é', &mut None),
        None,
        "`de`'s `é` sequence should not plan on `us`"
    );
}

/// Sequences come only from the group they are built in. On the two-group
/// `de(neo),de` keymap, every recorded pair plans in group 0 -- a pair
/// smuggled in from group 1 would plan a key the client, sitting in group
/// 0, decodes differently.
#[test]
fn sequences_come_only_from_the_active_group() {
    let two = keymap("de,de", "neo,");
    let table = table();
    let map = build_map(&two, FIRST, &table);
    for (character, _, _) in map.entries[..map.len].iter() {
        assert!(
            plan_sequence(&two, FIRST, &map, *character, &mut None).is_some(),
            "`{character}`'s recorded pair should plan in the group it was found in"
        );
    }
    // And the inactive group's umlaut is not reachable through the group-0
    // map at all: with `us,de`, group 0 is `us`, which contributes no dead
    // keys, so `ü` -- one group switch away -- has no sequence here.
    let us_de = keymap("us,de", "");
    let us_map = build_map(&us_de, FIRST, &table);
    assert_eq!(
        plan_sequence(&us_de, FIRST, &us_map, 'ü', &mut None),
        None,
        "`ü` should have no sequence while group 0 is `us`"
    );
}

/// Looks names up in a fixed list, the way [`session_locale`] reads the
/// environment -- without touching the real one.
fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        vars.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[test]
fn no_locale_variable_resolves_to_c() {
    assert_eq!(session_locale(env(&[])), "C");
}

#[test]
fn lang_alone_is_the_locale() {
    assert_eq!(
        session_locale(env(&[("LANG", "de_DE.UTF-8")])),
        "de_DE.UTF-8"
    );
}

#[test]
fn lc_ctype_beats_lang() {
    let vars = [("LANG", "de_DE.UTF-8"), ("LC_CTYPE", "sv_SE.UTF-8")];
    assert_eq!(session_locale(env(&vars)), "sv_SE.UTF-8");
}

#[test]
fn lc_all_beats_lc_ctype_and_lang() {
    let vars = [
        ("LANG", "de_DE.UTF-8"),
        ("LC_CTYPE", "sv_SE.UTF-8"),
        ("LC_ALL", "fr_FR.UTF-8"),
    ];
    assert_eq!(session_locale(env(&vars)), "fr_FR.UTF-8");
}

#[test]
fn an_empty_lc_all_falls_through() {
    // The reported case: `LC_ALL= LANG=C.UTF-8` runs clients in C.UTF-8,
    // so compose must resolve there too, not against "".
    let vars = [("LC_ALL", ""), ("LANG", "C.UTF-8")];
    assert_eq!(session_locale(env(&vars)), "C.UTF-8");
    let vars = [
        ("LC_ALL", ""),
        ("LC_CTYPE", "sv_SE.UTF-8"),
        ("LANG", "C.UTF-8"),
    ];
    assert_eq!(session_locale(env(&vars)), "sv_SE.UTF-8");
}

#[test]
fn an_empty_lc_ctype_falls_through_to_lang() {
    let vars = [("LC_CTYPE", ""), ("LANG", "de_DE.UTF-8")];
    assert_eq!(session_locale(env(&vars)), "de_DE.UTF-8");
}

#[test]
fn every_variable_empty_resolves_to_c() {
    let vars = [("LC_ALL", ""), ("LC_CTYPE", ""), ("LANG", "")];
    assert_eq!(session_locale(env(&vars)), "C");
}
