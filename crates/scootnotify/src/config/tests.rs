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
          max_live = 7\nrate_per_sec = 3\nrate_burst = 9\n",
    )
    .unwrap();
    assert_eq!(config.timeout_low_ms, 1000);
    assert_eq!(config.timeout_normal_ms, 2000);
    assert_eq!(config.timeout_max_ms, 3000);
    assert_eq!(config.max_live, 7);
    assert_eq!(config.rate_per_sec, 3);
    assert_eq!(config.rate_burst, 9);
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
        ("timeout_low_ms = 999999\ntimeout_max_ms = 3\n", "timeout_low_ms"),
        (
            "timeout_normal_ms = 999999\ntimeout_max_ms = 3\n",
            "timeout_normal_ms",
        ),
    ] {
        let error = parse(text.as_bytes()).unwrap_err();
        assert!(error.contains(key), "{text}: {error}");
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
