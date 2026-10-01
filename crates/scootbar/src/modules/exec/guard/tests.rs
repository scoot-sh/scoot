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

#[test]
fn a_bar_that_cannot_start_the_guard_names_what_it_tried_to_start() {
    // No /proc: `/proc/self/exe` is not there to run, and the message must
    // not blame the user's program (which the guard, never started, would
    // have run).
    let mut command = guarded("/nonexistent/proc/self/exe", 4242, "sh", &[]);
    let error = command.spawn().expect_err("no such file");
    let said = spawn_error(&command, "sh", &error);
    assert!(
        said.contains("cannot start `/nonexistent/proc/self/exe`"),
        "{said}"
    );
    assert!(said.contains("to run `sh`"), "{said}");
    assert!(said.contains("needs /proc mounted"), "{said}");
    assert!(!said.contains("cannot run `sh`"), "{said}");
}

#[test]
fn another_failure_to_start_the_guard_gets_no_proc_hint() {
    let command = guarded("/proc/self/exe", 1, "sh", &[]);
    let error = io::Error::from_raw_os_error(rustix::io::Errno::NOMEM.raw_os_error());
    let said = spawn_error(&command, "sh", &error);
    assert!(said.starts_with("cannot start `/proc/self/exe`"), "{said}");
    assert!(!said.contains("/proc mounted"), "{said}");
}

#[test]
fn a_command_started_directly_is_named_as_before() {
    let mut command = direct("scootbar-no-such-program", &[]);
    let error = command.spawn().expect_err("no such file");
    let said = spawn_error(&command, "scootbar-no-such-program", &error);
    assert!(
        said.starts_with("cannot run `scootbar-no-such-program`: "),
        "{said}"
    );
}

#[test]
fn the_guards_statuses_have_the_words_a_shells_do() {
    assert_eq!(meaning(127), Some(", command not found"));
    assert_eq!(meaning(126), Some(", not executable"));
    assert!(meaning(125).is_some_and(|m| m.contains("see the line above")));
    for code in [0, 1, 2, 124, 128, 255, 256, -1] {
        assert_eq!(meaning(code), None, "{code}");
    }
}
