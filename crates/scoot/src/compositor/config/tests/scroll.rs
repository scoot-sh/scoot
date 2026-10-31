//! `[binds]` scroll triggers and `[scroll]`: parsing, validation, the
//! `binds` listing and the reload comparison.

use super::*;
use crate::compositor::binds::snapshot;
use crate::compositor::scroll_binds::{DEFAULT_STEP, MAX_STEP, MIN_STEP};
use scoot_ipc::Response;

const SUPER: Modifiers = Modifiers {
    super_: true,
    shift: false,
    ctrl: false,
    alt: false,
};

fn load(text: &str) -> LoadedConfig {
    let (_dir, path) = write_temp(text);
    load_from(&path, true).expect("the file loads").0
}

#[test]
fn no_xkb_keysym_is_named_like_a_scroll_direction() {
    // The `scroll-` namespace is free only while this holds.
    for dir in [
        ScrollDir::Left,
        ScrollDir::Right,
        ScrollDir::Up,
        ScrollDir::Down,
    ] {
        assert_eq!(keysym_named(dir.name()), None, "{}", dir.name());
    }
}

#[test]
fn all_four_directions_parse_with_modifiers_in_any_case() {
    for (text, dir) in [
        ("super+scroll-left", ScrollDir::Left),
        ("Super+Scroll-Right", ScrollDir::Right),
        ("SUPER+SCROLL-UP", ScrollDir::Up),
        ("super + scroll-down", ScrollDir::Down),
    ] {
        assert_eq!(
            parse_combo(text),
            Ok((SUPER, Trigger::Scroll(dir))),
            "{text}"
        );
    }
    assert_eq!(
        parse_combo("scroll-down"),
        Ok((Modifiers::default(), Trigger::Scroll(ScrollDir::Down)))
    );
}

#[test]
fn a_bad_direction_names_the_valid_ones() {
    for bad in [
        "super+scroll-sideways",
        "scroll-",
        "super+scroll-",
        "Scroll-x",
    ] {
        let error = parse_combo(bad).expect_err(bad);
        assert!(
            error.contains("scroll-left, scroll-right, scroll-up or scroll-down"),
            "{bad}: {error}"
        );
    }
    // Not a scroll name at all: still an ordinary unknown key.
    assert_eq!(
        parse_combo("super+scrol"),
        Err("unknown key `scrol`".to_owned())
    );
}

#[test]
fn scroll_binds_load_and_match_only_through_match_scroll() {
    let loaded = load(
        r#"
        [binds]
        "super+scroll-left" = "focus-column left"
        "super+scroll-down" = "focus-window down"
        "#,
    );
    let table = &loaded.keybindings;
    assert!(table.has_scroll_binds());
    assert_eq!(
        table.match_scroll(ScrollDir::Left, SUPER),
        Some((
            &Bound::Action(Action::FocusColumn(Horizontal::Left)),
            BindFlags::default()
        ))
    );
    assert!(table.match_scroll(ScrollDir::Right, SUPER).is_none());
    assert!(
        table
            .match_scroll(ScrollDir::Left, Modifiers::default())
            .is_none(),
        "modifiers must match exactly"
    );
    assert!(
        loaded.skipped_binds.is_empty(),
        "{:?}",
        loaded.skipped_binds
    );
}

#[test]
fn a_bad_scroll_bind_is_skipped_and_listed_without_losing_the_rest() {
    let loaded = load(
        r#"
        [binds]
        "super+scroll-sideways" = "close"
        "super+scroll-up" = "focus-window up"
        "#,
    );
    assert!(
        loaded
            .keybindings
            .match_scroll(ScrollDir::Up, SUPER)
            .is_some()
    );
    assert_eq!(loaded.skipped_binds.len(), 1);
    assert_eq!(loaded.skipped_binds[0].bind, "super+scroll-sideways");
    assert!(loaded.skipped_binds[0].reason.contains("scroll-sideways"));
}

#[test]
fn two_spellings_of_one_scroll_combo_collide() {
    let loaded = load(
        r#"
        [binds]
        "super+scroll-down" = "focus-window down"
        "Logo+SCROLL-DOWN" = "close"
        "#,
    );
    assert!(!loaded.keybindings.has_scroll_binds());
    assert_eq!(loaded.skipped_binds.len(), 2);
}

#[test]
fn repeat_on_a_scroll_bind_is_cleared_and_the_lock_flag_follows_spawn_only() {
    let (_, _, bind, flags) = parse_bind(
        "super+scroll-down",
        &table_value("focus-window down", true, true),
    )
    .expect("the bind itself is valid");
    assert!(matches!(bind, ParsedBind::Bind(_)));
    assert!(!flags.repeat, "repeat means nothing for a scroll");
    assert!(
        !flags.allow_when_locked,
        "only a spawn may run while locked"
    );
    let (_, _, _, flags) = parse_bind("super+scroll-down", &table_value("spawn true", false, true))
        .expect("a flagged spawn");
    assert!(flags.allow_when_locked);
}

#[test]
fn unbinding_a_scroll_combo_nothing_holds_is_skipped_with_a_reason() {
    let loaded = load("[binds]\n\"super+scroll-down\" = \"none\"\n");
    assert_eq!(loaded.skipped_binds.len(), 1);
    assert!(loaded.skipped_binds[0].reason.contains("unbinds nothing"));
}

#[test]
fn the_binds_reply_lists_a_scroll_bind_as_config_and_round_trips_its_name() {
    let loaded = load("[binds]\n\"super+scroll-down\" = \"focus-window down\"\n");
    let Response::Binds { bindings, .. } = snapshot(&loaded.keybindings, &loaded.skipped_binds)
    else {
        panic!("a binds reply");
    };
    let row = bindings
        .iter()
        .find(|row| row.combo == "super+scroll-down")
        .expect("a row for the scroll bind");
    assert_eq!(row.action, "focus-window down");
    assert_eq!(row.source, "config");
    assert_eq!(
        parse_combo(&row.combo),
        Ok((SUPER, Trigger::Scroll(ScrollDir::Down)))
    );
}

#[test]
fn a_reload_comparison_sees_a_scroll_bind_come_and_go() {
    let defaults = Keybindings::default();
    let with = load("[binds]\n\"super+scroll-down\" = \"focus-window down\"\n").keybindings;
    let other = load("[binds]\n\"super+scroll-down\" = \"focus-window up\"\n").keybindings;
    assert!(!with.same_bindings_as(&defaults));
    assert!(!defaults.same_bindings_as(&with));
    assert!(!with.same_bindings_as(&other));
    assert!(with.same_bindings_as(&with.clone()));
}

#[test]
fn removing_the_last_scroll_bind_clears_the_fast_path_flag() {
    let mut table = Keybindings::default();
    table.insert_trigger(
        SUPER,
        Trigger::Scroll(ScrollDir::Up),
        Bound::Action(Action::CloseFocused),
        BindFlags::default(),
    );
    // Replacing in place must not double count.
    table.insert_trigger(
        SUPER,
        Trigger::Scroll(ScrollDir::Up),
        Bound::Action(Action::ToggleFullscreen),
        BindFlags::default(),
    );
    assert!(table.has_scroll_binds());
    assert!(
        table
            .remove_trigger(SUPER, Trigger::Scroll(ScrollDir::Up))
            .is_some()
    );
    assert!(!table.has_scroll_binds());
    assert!(
        table
            .remove_trigger(SUPER, Trigger::Scroll(ScrollDir::Up))
            .is_none()
    );
}

#[test]
fn the_default_table_holds_no_scroll_bind() {
    let defaults = Keybindings::default();
    assert!(!defaults.has_scroll_binds());
    assert_eq!(defaults.iter().count(), defaults.iter_triggers().count());
}

#[test]
fn step_defaults_and_accepts_integers_and_floats() {
    assert_eq!(load("").scroll_step, DEFAULT_STEP);
    assert_eq!(load("[scroll]\nstep = 40\n").scroll_step, 40.0);
    assert_eq!(load("[scroll]\nstep = 22.5\n").scroll_step, 22.5);
}

#[test]
fn a_bad_step_falls_back_or_clamps() {
    assert_eq!(
        load("[scroll]\nstep = \"fast\"\n").scroll_step,
        DEFAULT_STEP
    );
    assert_eq!(load("[scroll]\nstep = nan\n").scroll_step, DEFAULT_STEP);
    assert_eq!(load("[scroll]\nstep = inf\n").scroll_step, DEFAULT_STEP);
    assert_eq!(load("[scroll]\nstep = 0\n").scroll_step, MIN_STEP);
    assert_eq!(load("[scroll]\nstep = -5\n").scroll_step, MIN_STEP);
    assert_eq!(load("[scroll]\nstep = 1e9\n").scroll_step, MAX_STEP);
}

#[test]
fn an_unknown_scroll_key_is_refused_like_any_table() {
    let (_dir, path) = write_temp("[scroll]\nstepp = 40\n");
    let (loaded, _) = load_from(&path, true).expect("falls back to defaults");
    assert_eq!(loaded.scroll_step, DEFAULT_STEP);
}

#[test]
fn the_default_config_emission_names_the_scroll_table_and_its_default() {
    let emitted = default_config_toml();
    assert!(emitted.contains("\n[scroll]\n"));
    assert!(emitted.contains(&format!("# step = {DEFAULT_STEP}")));
    // And the emission still loads to the defaults.
    let (_dir, path) = write_temp(&emitted);
    let (loaded, _) = load_from(&path, true).expect("the emission loads");
    assert_eq!(loaded.scroll_step, DEFAULT_STEP);
}
