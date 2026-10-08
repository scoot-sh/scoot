//! `[[outputs]]` resolution, lookup, and the reload diff -- all pure.

use serde::Deserialize;

use super::{EntriesDiff, ModeRequests, OutputEntries, OutputEntry, OutputEntryConfig};

/// Just the `[[outputs]]` array, parsed the way `config.rs`'s `FileConfig`
/// parses it (the same field attributes), so these tests read real TOML.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    outputs: Vec<OutputEntryConfig>,
}

fn entries(toml_text: &str) -> OutputEntries {
    let file: File = toml::from_str(toml_text).expect("valid toml");
    OutputEntries::resolve(file.outputs)
}

fn entry(
    name: &str,
    scale: Option<f64>,
    mode: Option<(u16, u16)>,
    position: Option<(i32, i32)>,
) -> OutputEntry {
    OutputEntry {
        name: name.to_owned(),
        scale,
        mode,
        position,
    }
}

#[test]
fn no_entries_is_empty_and_everything_takes_the_default() {
    let resolved = entries("");
    assert!(resolved.is_empty());
    assert_eq!(resolved.scale_for("eDP-1", 1.5), 1.5);
    assert_eq!(resolved.mode_for("eDP-1"), None);
    assert_eq!(resolved.position_for("eDP-1"), None);
}

#[test]
fn an_entry_overrides_its_own_output_only() {
    let resolved = entries(
        "[[outputs]]\nname = \"eDP-1\"\nscale = 2.0\n\n\
         [[outputs]]\nname = \"DP-1\"\nmode = \"1280x720\"\n",
    );
    assert_eq!(resolved.scale_for("eDP-1", 1.5), 2.0);
    assert_eq!(resolved.mode_for("eDP-1"), None);
    // DP-1 sets only a mode, so its scale is the default.
    assert_eq!(resolved.scale_for("DP-1", 1.5), 1.5);
    assert_eq!(resolved.mode_for("DP-1"), Some((1280, 720)));
    // An output with no entry is untouched.
    assert_eq!(resolved.scale_for("HDMI-A-1", 1.5), 1.5);
    assert_eq!(resolved.mode_for("HDMI-A-1"), None);
    assert_eq!(resolved.position_for("HDMI-A-1"), None);
}

#[test]
fn names_match_exactly() {
    let resolved = entries("[[outputs]]\nname = \"DP-1\"\nscale = 2\n");
    for other in ["dp-1", "DP-10", "DP-", " DP-1", "DP-1 "] {
        assert_eq!(
            resolved.scale_for(other, 1.0),
            1.0,
            "{other:?} matched DP-1"
        );
    }
    // An integer scale parses as a float, like `[output] scale = 2`.
    assert_eq!(resolved.scale_for("DP-1", 1.0), 2.0);
}

#[test]
fn an_entry_scale_resolves_like_the_default_one() {
    // Clamped into range, then to 120ths -- `[output] scale`'s own rules.
    let resolved = entries(
        "[[outputs]]\nname = \"a\"\nscale = 10.0\n\
         [[outputs]]\nname = \"b\"\nscale = 0.1\n\
         [[outputs]]\nname = \"c\"\nscale = 1.33\n",
    );
    assert_eq!(resolved.scale_for("a", 1.0), 4.0);
    assert_eq!(resolved.scale_for("b", 1.0), 0.5);
    assert_eq!(resolved.scale_for("c", 1.0), 160.0 / 120.0);
}

#[test]
fn a_non_finite_entry_scale_keeps_the_default_not_one() {
    // Unlike `[output] scale`, whose fallback is 1.0: an entry has a
    // default to fall back to, and that is what its absence would mean.
    let resolved = entries(
        "[[outputs]]\nname = \"a\"\nscale = nan\nmode = \"800x600\"\n\
         [[outputs]]\nname = \"b\"\nscale = inf\n",
    );
    assert_eq!(resolved.scale_for("a", 1.5), 1.5);
    assert_eq!(resolved.mode_for("a"), Some((800, 600)));
    // `b` set nothing usable at all, so it is not an entry.
    assert_eq!(resolved.iter().count(), 1);
}

#[test]
fn a_mode_that_is_not_wxh_is_dropped_from_its_entry() {
    for bad in [
        "1920",
        "1920x",
        "x1080",
        "0x1080",
        "1920x0",
        "70000x1080",
        "-1x5",
        "1920X1080",
    ] {
        let resolved = entries(&format!(
            "[[outputs]]\nname = \"a\"\nscale = 2\nmode = \"{bad}\"\n"
        ));
        assert_eq!(resolved.mode_for("a"), None, "{bad:?} parsed as a mode");
        // The rest of the entry stands.
        assert_eq!(resolved.scale_for("a", 1.0), 2.0, "{bad:?} cost the scale");
    }
    // The largest size a u16 axis holds is accepted, like `--mode`.
    let resolved = entries("[[outputs]]\nname = \"a\"\nmode = \"65535x65535\"\n");
    assert_eq!(resolved.mode_for("a"), Some((65535, 65535)));
}

#[test]
fn the_first_entry_for_a_name_wins_and_empty_names_are_skipped() {
    let resolved = entries(
        "[[outputs]]\nname = \"\"\nscale = 3\n\
         [[outputs]]\nname = \"DP-1\"\nscale = 2\n\
         [[outputs]]\nname = \"DP-1\"\nscale = 3\nmode = \"640x480\"\n",
    );
    assert_eq!(resolved.iter().count(), 1);
    assert_eq!(resolved.scale_for("DP-1", 1.0), 2.0);
    // The second entry's mode is not merged into the first.
    assert_eq!(resolved.mode_for("DP-1"), None);
    assert_eq!(resolved.position_for("DP-1"), None);
    assert_eq!(resolved.scale_for("", 1.0), 1.0);
}

#[test]
fn an_entry_that_sets_nothing_is_dropped() {
    let resolved = entries("[[outputs]]\nname = \"DP-1\"\n");
    assert!(resolved.is_empty());
}

#[test]
fn an_entry_needs_a_name_and_knows_its_keys() {
    // No `name`: nothing to match, so it is malformed like an unknown key
    // -- the whole file's parse fails, the same rule as every other table.
    assert!(toml::from_str::<File>("[[outputs]]\nscale = 2\n").is_err());
    assert!(toml::from_str::<File>("[[outputs]]\nname = \"a\"\nscales = 2\n").is_err());
    // `match` was the design sketch's illustrative key; it is not the key.
    assert!(toml::from_str::<File>("[[outputs]]\nmatch = \"a\"\nscale = 2\n").is_err());
}

#[test]
fn mode_requests_prefer_the_entry_then_the_flag() {
    let resolved = entries(
        "[[outputs]]\nname = \"DP-1\"\nmode = \"1280x720\"\n\
         [[outputs]]\nname = \"eDP-1\"\nscale = 2\n",
    );
    let with_flag = ModeRequests::new(Some((1920, 1080)), &resolved);
    assert_eq!(with_flag.for_output("DP-1"), Some((1280, 720)));
    // eDP-1's entry sets no mode, so the flag applies to it.
    assert_eq!(with_flag.for_output("eDP-1"), Some((1920, 1080)));
    assert_eq!(with_flag.for_output("HDMI-A-1"), Some((1920, 1080)));

    let without_flag = ModeRequests::new(None, &resolved);
    assert_eq!(without_flag.for_output("DP-1"), Some((1280, 720)));
    assert_eq!(without_flag.for_output("eDP-1"), None);

    // No entries and no flag: exactly the old "preferred everywhere".
    let nothing = ModeRequests::new(None, &OutputEntries::default());
    assert_eq!(nothing, ModeRequests::default());
    assert_eq!(nothing.for_output("DP-1"), None);
}

#[test]
fn the_reload_diff_names_each_changed_field_once() {
    let live = entries(
        "[[outputs]]\nname = \"eDP-1\"\nscale = 2\n\
         [[outputs]]\nname = \"DP-1\"\nscale = 1\nmode = \"1280x720\"\n",
    );
    // Unchanged: nothing.
    assert_eq!(live.diff(&live.clone()), EntriesDiff::default());

    let fresh = entries(
        "[[outputs]]\nname = \"DP-1\"\nscale = 1.5\nmode = \"1920x1080\"\n\
         [[outputs]]\nname = \"HDMI-A-1\"\nscale = 1.25\n",
    );
    let diff = live.diff(&fresh);
    // eDP-1's entry went away (its scale reverts to the default), DP-1's
    // scale and mode both moved, HDMI-A-1 is new and sets only a scale.
    assert_eq!(diff.scales, ["eDP-1", "DP-1", "HDMI-A-1"]);
    assert_eq!(diff.modes, ["DP-1"]);
    assert!(diff.positions.is_empty(), "{diff:?}");
}

#[test]
fn the_reload_diff_names_a_changed_position() {
    let live = entries(
        "[[outputs]]\nname = \"DP-1\"\nposition = [-1920, 0]\n\
          [[outputs]]\nname = \"eDP-1\"\nscale = 2\n",
    );
    // Unchanged: nothing, in no list.
    assert_eq!(live.diff(&live.clone()), EntriesDiff::default());

    // Only the position moves: only it reports.
    let moved = entries(
        "[[outputs]]\nname = \"DP-1\"\nposition = [0, -1080]\n\
          [[outputs]]\nname = \"eDP-1\"\nscale = 2\n",
    );
    let diff = live.diff(&moved);
    assert!(diff.scales.is_empty(), "{diff:?}");
    assert!(diff.modes.is_empty(), "{diff:?}");
    assert_eq!(diff.positions, ["DP-1"]);

    // The entry goes away: its position reverts to packed, which reports.
    let dropped = entries("[[outputs]]\nname = \"eDP-1\"\nscale = 2\n");
    let diff = live.diff(&dropped);
    assert_eq!(diff.positions, ["DP-1"]);
}

#[test]
fn a_position_places_an_output_including_negative_origins() {
    let resolved = entries(
        "[[outputs]]\nname = \"DP-1\"\nposition = [-1920, 0]\n\
          [[outputs]]\nname = \"HDMI-A-1\"\nposition = [0, -1080]\n\
          [[outputs]]\nname = \"eDP-1\"\nscale = 2\n",
    );
    assert_eq!(resolved.position_for("DP-1"), Some((-1920, 0)));
    assert_eq!(resolved.position_for("HDMI-A-1"), Some((0, -1080)));
    // An entry that sets only a scale leaves packing alone.
    assert_eq!(resolved.position_for("eDP-1"), None);
}

#[test]
fn a_mistyped_position_costs_only_the_position() {
    // A string, a lone number, a triple, a float and an out-of-range
    // integer each drop the position with a warning, never the entry and
    // never the file: the scale beside it still resolves.
    for bad in [
        "position = \"left\"",
        "position = 5",
        "position = [0]",
        "position = [0, 0, 0]",
        "position = [0.5, 0]",
        "position = [2147483648, 0]",
    ] {
        let resolved = entries(&format!("[[outputs]]\nname = \"a\"\nscale = 2\n{bad}\n"));
        assert_eq!(
            resolved.position_for("a"),
            None,
            "{bad:?} parsed as a position"
        );
        // The rest of the entry stands.
        assert_eq!(resolved.scale_for("a", 1.0), 2.0, "{bad:?} cost the scale");
    }
    // The widest pair an i32 axis holds is accepted.
    let resolved = entries("[[outputs]]\nname = \"a\"\nposition = [-2147483648, 2147483647]\n");
    assert_eq!(resolved.position_for("a"), Some((i32::MIN, i32::MAX)));
}

#[test]
fn an_entry_that_sets_only_a_position_is_kept() {
    // Like a mode-only entry: the position alone makes it an entry.
    let resolved = entries("[[outputs]]\nname = \"DP-1\"\nposition = [-1920, 0]\n");
    assert_eq!(resolved.iter().count(), 1);
    assert_eq!(resolved.position_for("DP-1"), Some((-1920, 0)));
}

#[test]
fn a_reload_stores_the_fresh_entries_whole_modes_included() {
    // Modes apply live now, so there is nothing to hold back: what a
    // reload stores is the fresh list itself, and a second reload of the
    // same file diffs nothing against it.
    let live = entries(
        "[[outputs]]\nname = \"DP-1\"\nscale = 1\nmode = \"1280x720\"\n\
         [[outputs]]\nname = \"HDMI-A-1\"\nmode = \"800x600\"\n",
    );
    let fresh = entries(
        "[[outputs]]\nname = \"DP-1\"\nscale = 1.5\nmode = \"1920x1080\"\n\
         [[outputs]]\nname = \"eDP-1\"\nposition = [-1920, 0]\n",
    );
    assert_eq!(
        fresh.iter().cloned().collect::<Vec<_>>(),
        [
            entry("DP-1", Some(1.5), Some((1920, 1080)), None),
            entry("eDP-1", None, None, Some((-1920, 0))),
        ]
    );
    assert_eq!(fresh.diff(&fresh.clone()), EntriesDiff::default());
    // Against the live list every field that moved reports, once.
    let diff = live.diff(&fresh);
    assert_eq!(diff.scales, ["DP-1"]);
    assert_eq!(diff.modes, ["DP-1", "HDMI-A-1"]);
    assert_eq!(diff.positions, ["eDP-1"]);
}

#[test]
fn mode_requests_follow_a_reload_while_the_flag_default_stays() {
    // `--tty`'s stored requests after a reload: the flag half is untouched
    // (no reload re-reads a flag), the per-output half is the fresh list.
    let live = entries("[[outputs]]\nname = \"DP-1\"\nmode = \"1280x720\"\n");
    let mut requests = ModeRequests::new(Some((1920, 1080)), &live);
    assert_eq!(requests.for_output("DP-1"), Some((1280, 720)));

    let fresh = entries(
        "[[outputs]]\nname = \"DP-1\"\nmode = \"1920x1080\"\n\
         [[outputs]]\nname = \"eDP-1\"\nmode = \"2560x1600\"\n",
    );
    requests.update(&fresh);
    assert_eq!(requests.for_output("DP-1"), Some((1920, 1080)));
    assert_eq!(requests.for_output("eDP-1"), Some((2560, 1600)));
    // The flag still covers the output with no entry.
    assert_eq!(requests.for_output("HDMI-A-1"), Some((1920, 1080)));

    // Dropping every entry returns to the flag everywhere.
    requests.update(&OutputEntries::default());
    assert_eq!(requests.for_output("DP-1"), Some((1920, 1080)));
}
