//! The guard's pieces: what the bar asks it for, how it reads that, and
//! when it declines to be the CLI. That it ends a command with the bar is
//! checked on a running bar in `tests/exec.rs`.

use std::ffi::OsString;

use super::*;

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn the_guarded_command_line_is_the_marker_the_bar_and_the_command() {
    let command = guarded(
        "/proc/self/exe",
        4242,
        "sh",
        &["-c".into(), "echo hi".into()],
    );
    assert_eq!(command.get_program(), "/proc/self/exe");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, [MARKER, "4242", "sh", "-c", "echo hi"]);
}

#[test]
fn the_guard_reads_what_the_bar_wrote() {
    let args = os(&["4242", "sh", "-c", "echo hi"]);
    let plan = parse(&args).expect("a plan");
    assert_eq!(plan.bar, Pid::from_raw(4242).unwrap());
    assert_eq!(plan.program, "sh");
    assert_eq!(plan.args, &os(&["-c", "echo hi"])[..]);
    // A command with no arguments is a command.
    let args = os(&["7", "true"]);
    assert_eq!(parse(&args).expect("a plan").args.len(), 0);
}

#[test]
fn a_malformed_request_is_refused_not_guessed_at() {
    for args in [
        os(&[]),
        os(&["4242"]),
        os(&["x", "sh"]),
        os(&["-1", "sh"]),
        os(&["0", "sh"]),
        os(&["99999999999", "sh"]),
    ] {
        assert_eq!(parse(&args), None, "{args:?}");
    }
}

#[test]
fn an_argument_that_is_not_utf8_in_the_pid_place_is_refused() {
    use std::os::unix::ffi::OsStringExt;
    let args = vec![OsString::from_vec(vec![0xff, 0xfe]), OsString::from("sh")];
    assert_eq!(parse(&args), None);
}

#[test]
fn only_the_marker_makes_the_binary_a_guard() {
    // Anything else is the CLI's, and is left to it untouched.
    assert!(run(os(&[])).is_none());
    assert!(run(os(&["daemon"])).is_none());
    assert!(run(os(&["msg", MARKER])).is_none());
    assert!(run(os(&["--help"])).is_none());
}

#[test]
fn a_marker_with_no_command_exits_unsuccessfully_without_running_anything() {
    assert!(run(os(&[MARKER])).is_some());
    assert!(run(os(&[MARKER, "12"])).is_some());
}

#[test]
fn the_unit_tests_start_the_command_directly() {
    // `/proc/self/exe` is the test harness here: the guard would run the
    // tests again.
    let command = command("sleep", &["1".into()]);
    assert_eq!(command.get_program(), "sleep");
}
