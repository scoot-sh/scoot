//! Globs, rule validation and the map-time decision, as pure functions.

use super::*;

fn glob(pattern: &str) -> Glob {
    Glob::new(pattern).expect("a valid pattern")
}

fn rule(app_id: Option<&str>, title: Option<&str>, float: Option<bool>) -> WindowRuleConfig {
    WindowRuleConfig {
        match_app_id: app_id.map(str::to_owned),
        match_title: title.map(str::to_owned),
        float,
        size: None,
    }
}

fn rules(written: &[WindowRuleConfig]) -> FloatingRules {
    let (rules, skipped) = FloatingRules::from_config(None, written);
    assert!(skipped.is_empty(), "{skipped:?}");
    rules
}

fn signals<'a>(app_id: &'a str, title: &'a str) -> MapSignals<'a> {
    MapSignals {
        app_id,
        title,
        ..MapSignals::default()
    }
}

#[test]
fn a_glob_matches_the_whole_string() {
    assert!(glob("foot").matches("foot"));
    assert!(!glob("foot").matches("footclient"));
    assert!(!glob("foot").matches("xfoot"));
    assert!(glob("foot*").matches("footclient"));
    assert!(glob("*Preferences*").matches("Firefox Preferences"));
    assert!(glob("*Preferences*").matches("Preferences"));
    assert!(
        !glob("*Preferences*").matches("preferences"),
        "case-sensitive"
    );
    assert!(glob("a?c").matches("abc"));
    assert!(!glob("a?c").matches("ac"));
    assert!(glob("*").matches(""));
    assert!(glob("").matches(""));
    assert!(!glob("").matches("x"));
    assert!(glob("**a**").matches("xxaxx"));
}

#[test]
fn a_glob_backtracks_past_a_false_start() {
    // The first `b` after the star is not the one the pattern needs.
    assert!(glob("*bc").matches("abbc"));
    assert!(glob("a*b*c").matches("aXbYbZc"));
    assert!(!glob("a*b*c").matches("aXbYbZ"));
    assert!(glob("*.txt").matches("notes.old.txt"));
}

#[test]
fn a_question_mark_is_one_character_not_one_byte() {
    assert!(glob("caf?").matches("café"));
    assert!(glob("?").matches("é"));
    assert!(!glob("??").matches("é"));
}

#[test]
fn the_worst_case_pattern_finishes() {
    // Many stars and a text that never matches: the one-backtrack-point
    // matcher is quadratic at worst, not exponential.
    let pattern = "*a".repeat(MAX_PATTERN_LEN / 2 - 1);
    let text = "a".repeat(4096);
    let start = std::time::Instant::now();
    assert!(!glob(&(pattern + "b")).matches(&text));
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn the_heuristics_float_dialogs_transients_and_fixed_sizes() {
    let rules = FloatingRules::default();
    let decide = |dialog, parent, fixed_size| {
        rules.decide(&MapSignals {
            dialog,
            parent,
            fixed_size,
            ..MapSignals::default()
        })
    };
    assert_eq!(decide(false, false, false).reason, Reason::Default);
    assert!(!decide(false, false, false).float);
    assert_eq!(decide(true, true, true).reason, Reason::Dialog);
    assert_eq!(decide(false, true, true).reason, Reason::Parent);
    assert_eq!(decide(false, false, true).reason, Reason::FixedSize);
    assert!(decide(false, false, true).float);
}

#[test]
fn auto_off_turns_every_heuristic_off() {
    let (rules, _) = FloatingRules::from_config(
        Some(FloatingConfig { auto: Some(false) }),
        &[rule(Some("foot"), None, Some(true))],
    );
    let dialog = MapSignals {
        dialog: true,
        parent: true,
        fixed_size: true,
        ..MapSignals::default()
    };
    assert!(!rules.decide(&dialog).float);
    // Rules still apply.
    assert!(rules.decide(&signals("foot", "")).float);
}

#[test]
fn a_rule_floats_a_matching_window_and_float_false_overrides_a_heuristic() {
    let rules = rules(&[
        rule(Some("foot"), None, Some(true)),
        rule(None, Some("*Save*"), Some(false)),
    ]);
    let foot = rules.decide(&signals("foot", "~"));
    assert!(foot.float);
    assert_eq!(foot.reason, Reason::Rule(1));
    assert!(!rules.decide(&signals("footclient", "~")).float);
    let save = MapSignals {
        dialog: true,
        ..signals("org.gnome.Nautilus", "Save As")
    };
    let decision = rules.decide(&save);
    assert!(!decision.float, "{decision:?}");
    assert_eq!(decision.reason, Reason::Rule(2));
}

#[test]
fn both_matchers_must_match() {
    let rules = rules(&[rule(Some("firefox"), Some("*Library*"), Some(true))]);
    assert!(rules.decide(&signals("firefox", "Library")).float);
    assert!(!rules.decide(&signals("firefox", "Mozilla Firefox")).float);
    assert!(!rules.decide(&signals("chromium", "Library")).float);
}

#[test]
fn later_rules_override_earlier_ones_field_by_field() {
    let mut sized = rule(Some("*"), None, None);
    sized.size = Some([800, 600]);
    let rules = rules(&[
        rule(Some("mpv"), None, Some(true)),
        sized,
        rule(Some("mpv"), Some("*nofloat*"), Some(false)),
    ]);
    let mpv = rules.decide(&signals("mpv", "video"));
    assert!(mpv.float);
    assert_eq!(mpv.size, Some(Size::new(800, 600)));
    assert_eq!(mpv.reason, Reason::Rule(1));
    // Overridden back to tiling: no size is asked for a tiled window.
    let tiled = rules.decide(&signals("mpv", "a nofloat video"));
    assert!(!tiled.float);
    assert_eq!(tiled.size, None);
    // A size alone floats nothing.
    assert!(!rules.decide(&signals("foot", "")).float);
}

#[test]
fn unusable_rules_are_skipped_and_named_by_their_place_in_the_file() {
    let mut zero = rule(Some("a"), None, Some(true));
    zero.size = Some([0, 100]);
    let mut huge = rule(Some("a"), None, Some(true));
    huge.size = Some([100, i64::MAX]);
    let mut negative = rule(Some("a"), None, None);
    negative.size = Some([-1, 100]);
    let long = rule(Some(&"x".repeat(MAX_PATTERN_LEN + 1)), None, Some(true));
    let written = [
        rule(None, None, Some(true)),
        rule(Some("a"), None, None),
        zero,
        huge,
        negative,
        long,
        rule(Some("kept"), None, Some(true)),
    ];
    let (rules, skipped) = FloatingRules::from_config(None, &written);
    assert_eq!(skipped.len(), 6, "{skipped:?}");
    for (index, message) in skipped.iter().enumerate() {
        assert!(
            message.starts_with(&format!("window_rule #{} (", index + 1)),
            "{message}"
        );
    }
    assert_eq!(rules.rules.len(), 1);
    // The kept rule keeps its file position, 7, for the log.
    assert_eq!(rules.decide(&signals("kept", "")).reason, Reason::Rule(7));
}

#[test]
fn the_largest_sizes_are_accepted() {
    let mut largest = rule(Some("a"), None, Some(true));
    largest.size = Some([MAX_RULE_SIZE, 1]);
    let rules = rules(&[largest]);
    assert_eq!(
        rules.decide(&signals("a", "")).size,
        Some(Size::new(65_535, 1))
    );
}
