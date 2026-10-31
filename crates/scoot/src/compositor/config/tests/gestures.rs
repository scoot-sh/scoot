//! `[gestures]` through the whole-file loader: the table reaches
//! `LoadedConfig`, the default emission names it and loads back to the
//! defaults, and a bad table does not take the file down with it. (The
//! per-key parsing is pinned in `gesture/config.rs`.)

use super::*;
use crate::compositor::gesture::{GestureConfig, HorizontalMode, VerticalMode};

fn load(text: &str) -> LoadedConfig {
    let (_dir, path) = write_temp(text);
    load_from(&path, true).expect("the file loads").0
}

#[test]
fn a_file_without_the_table_runs_the_defaults() {
    assert_eq!(load("").gestures, GestureConfig::default());
    assert_eq!(LoadedConfig::defaults().gestures, GestureConfig::default());
}

#[test]
fn the_table_reaches_the_loaded_config() {
    let loaded = load(
        "[gestures]\nswipe_fingers = 4\nhorizontal = \"none\"\nvertical = \"focus-window\"\n\
         natural = false\nsensitivity = 1.5\n",
    );
    assert_eq!(loaded.gestures.horizontal, HorizontalMode::None);
    assert_eq!(loaded.gestures.vertical, VerticalMode::FocusWindow);
    assert!(!loaded.gestures.natural);
    assert_eq!(loaded.gestures.sensitivity, 1.5);
}

#[test]
fn a_bad_value_costs_that_key_and_not_the_rest_of_the_file() {
    let loaded = load("[gestures]\nswipe_fingers = 7\nnatural = false\n\n[scroll]\nstep = 40\n");
    assert_eq!(loaded.gestures.fingers, 3);
    assert!(!loaded.gestures.natural);
    assert_eq!(loaded.scroll_step, 40.0);
}

#[test]
fn the_default_config_emission_names_the_gestures_table_and_loads_back() {
    let emitted = default_config_toml();
    assert!(emitted.contains("\n[gestures]\n"));
    for line in [
        "# swipe_fingers = 3",
        "# horizontal = \"focus-column\"",
        "# vertical = \"focus-workspace\"",
        "# natural = true",
        "# sensitivity = 1.0",
    ] {
        assert!(emitted.contains(line), "the emission lost `{line}`");
    }
    let (_dir, path) = write_temp(&emitted);
    let (loaded, _) = load_from(&path, true).expect("the emission loads");
    assert_eq!(loaded.gestures, GestureConfig::default());
}
