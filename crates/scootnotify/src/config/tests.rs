//! The config's contract: defaults, refusals, and the 64 KiB cap.

use super::{Config, default_path, load, parse};

#[test]
fn an_empty_file_is_the_defaults() {
    assert_eq!(parse(b"").unwrap(), Config::default());
}

#[test]
fn every_key_sets() {
    let config = parse(
        b"timeout_low_ms = 1000\ntimeout_normal_ms = 2000\ntimeout_max_ms = 3000\n\
          max_live = 7\nrate_per_sec = 3\nrate_burst = 9\n\
          width_px = 400\nmax_visible = 3\nmargin_px = 8\nanchor = \"bottom-left\"\n\
          output = \"HDMI-1\"\nbody_lines = 3\nborder_px = 1\nradius = 4\n\
          pause_when_idle_secs = 30\n",
    )
    .unwrap();
    assert_eq!(config.timeout_low_ms, 1000);
    assert_eq!(config.timeout_normal_ms, 2000);
    assert_eq!(config.timeout_max_ms, 3000);
    assert_eq!(config.max_live, 7);
    assert_eq!(config.rate_per_sec, 3);
    assert_eq!(config.rate_burst, 9);
    assert_eq!(config.width_px, 400);
    assert_eq!(config.max_visible, 3);
    assert_eq!(config.margin_px, 8);
    assert_eq!(config.anchor, super::Anchor::BottomLeft);
    assert_eq!(config.output.as_deref(), Some("HDMI-1"));
    assert_eq!(config.body_lines, 3);
    assert_eq!(config.border_px, 1);
    assert_eq!(config.radius, 4);
    assert_eq!(config.pause_when_idle_secs, 30);
}

#[test]
fn an_unknown_key_is_refused() {
    let error = parse(b"timeout_bogus_ms = 1\n").unwrap_err();
    assert!(error.contains("timeout_bogus_ms"), "{error}");
}

#[test]
fn not_utf8_is_refused() {
    assert!(parse(b"time\xffout = 1\n").is_err());
}

#[test]
fn not_toml_is_refused() {
    assert!(parse(b"[[[\n").is_err());
}

#[test]
fn a_wrong_type_is_refused() {
    let error = parse(b"max_live = \"many\"\n").unwrap_err();
    assert!(error.contains("max_live"), "{error}");
}

#[test]
fn out_of_range_numbers_name_their_key() {
    for (text, key) in [
        ("max_live = 0\n", "max_live"),
        ("max_live = 1025\n", "max_live"),
        ("rate_per_sec = 0\n", "rate_per_sec"),
        ("rate_burst = 1001\n", "rate_burst"),
        ("timeout_max_ms = 0\n", "timeout_max_ms"),
        (
            "timeout_low_ms = 11\ntimeout_max_ms = 10\n",
            "timeout_low_ms",
        ),
        (
            "timeout_low_ms = 1\ntimeout_normal_ms = 11\ntimeout_max_ms = 10\n",
            "timeout_normal_ms",
        ),
        ("width_px = 9\n", "width_px"),
        ("width_px = 1025\n", "width_px"),
        ("max_visible = 0\n", "max_visible"),
        ("max_visible = 17\n", "max_visible"),
        ("margin_px = 129\n", "margin_px"),
        ("anchor = \"middle\"\n", "anchor"),
        ("output = \"\"\n", "output"),
        ("body_lines = 0\n", "body_lines"),
        ("body_lines = 17\n", "body_lines"),
        ("border_px = 9\n", "border_px"),
        ("radius = 65\n", "radius"),
        ("pause_when_idle_secs = 3601\n", "pause_when_idle_secs"),
    ] {
        let error = parse(text.as_bytes()).unwrap_err();
        assert!(error.contains(key), "{text}: {error}");
    }
}

/// The popup rows' boundaries hold: zero border, zero idle pause (both
/// meaningful off states), and every anchor spelling.
#[test]
fn the_popup_boundary_rows_hold() {
    assert!(parse(b"border_px = 0\n").is_ok());
    assert!(parse(b"pause_when_idle_secs = 0\n").is_ok());
    for anchor in ["top-left", "top-right", "bottom-left", "bottom-right"] {
        let text = format!("anchor = \"{anchor}\"\n");
        assert!(parse(text.as_bytes()).is_ok(), "{anchor}");
    }
}

#[test]
fn past_64_kib_is_refused_before_parsing() {
    let mut big = vec![b'#'; 64 * 1024 + 1];
    big[0] = b'\n';
    let error = parse(&big).unwrap_err();
    assert!(error.contains("64"), "{error}");
    // Exactly the cap still parses (as comments).
    let exact = vec![b'#'; 64 * 1024];
    assert!(parse(&exact).is_ok());
}

/// Reverting the unknown-key refusal to a silent ignore leaves the typo
/// test failing: the pin is load-bearing, not a grep for a flag.
#[test]
fn the_boundary_rows_hold() {
    assert!(parse(b"max_live = 1\n").is_ok());
    assert!(parse(b"max_live = 1024\n").is_ok());
    assert!(parse(b"rate_per_sec = 1000\nrate_burst = 1000\n").is_ok());
}

#[test]
fn the_default_path_sits_beside_the_bars() {
    let path = default_path();
    assert!(path.ends_with("scoot/notify.toml"), "{}", path.display());
}

#[test]
fn a_missing_file_is_an_error_for_the_caller_to_treat_as_defaults() {
    let missing = std::path::Path::new("/nonexistent-dir-7f3a/notify.toml");
    assert!(load(missing).is_err());
}
