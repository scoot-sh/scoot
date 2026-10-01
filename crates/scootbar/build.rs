//! Gives the linker scootbar's hot-text order file, when the linker takes it.
//!
//! `orderfile/hot-text.ld` lists the functions a running bar executes, as
//! globs, for GNU ld's `--section-ordering-file` (binutils 2.43 and later): the
//! linker places them first in `.text`, in a row. Without it the same
//! functions are spread over every 64 KiB of the code and the kernel, which
//! maps a file's pages 64 KiB at a time on a read fault, makes nearly all of
//! it resident. Why, how the file is made and what it saves:
//! `scripts/scootbar-orderfile/orderfile.py` and `docs/scootbar/README.md`
//! ("M4 usage optimization").
//!
//! **It is an optimization and never a requirement.** The flag is passed
//! only after a probe has linked a one-line program with this very file
//! through the real link path: this build script runs `$RUSTC`, the compiler
//! Cargo uses, with the same `--target`, the same `RUSTFLAGS` and the
//! configured linker, and `-C link-arg=-Wl,--section-ordering-file=...`. So
//! `rustc` itself picks the linker driver and whether it brings its own
//! `lld` (the default for `x86_64-unknown-linux-gnu` since Rust 1.90, which a
//! build script cannot see by asking `cc`), and every spelling of a linker
//! flag (`-C`, `-Cx=y`, `--codegen`, `-fuse-ld=...`, `-Zlinker-features`,
//! `link-self-contained`) means what it means for the bar. A linker without
//! the option (binutils before 2.43, lld, mold) fails the probe and the bar
//! links as it always did, as it does for a file the linker will not read.
//! gold takes the option as a no-op. The decision is made again whenever
//! what it rests on changes: see [`triggers`].
//!
//! Visible, never silent when something is wrong: a probe that cannot link
//! even without the flag (a broken toolchain, which the real link will report
//! in its own words) says so as a `cargo:warning`; `SCOOTBAR_ORDERFILE_VERBOSE`
//! says what was decided and why, whatever the answer. Skipped without a
//! word, because the answer is simply no: not Linux, `SCOOTBAR_NO_ORDERFILE`
//! set (to see what the layout costs), a build that is not a release build
//! (`cargo build`, `cargo test` and rust-analyzer are exactly what they
//! were: no probe, and no trigger that would rebuild them), a linker that
//! does not take the option, and a path the `-Wl,` syntax cannot carry (a
//! comma).
//!
//! The pure parts have unit tests, which Cargo does not run for a build
//! script: `rustc --edition 2024 --test crates/scootbar/build.rs -o /tmp/t && /tmp/t`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The order file, relative to the crate.
const ORDER_FILE: &str = "orderfile/hot-text.ld";

/// What was decided, and for the verbose note, why.
#[derive(Debug, PartialEq)]
enum Decision {
    /// Pass this link argument.
    Apply(String),
    /// Link as before; the answer is no, and the reason is for the verbose note.
    Skip(String),
    /// Link as before, and say so: the probe itself did not work.
    Broken(String),
}

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let probing = probes();
    let driver = link_driver().filter(|_| probing);
    for trigger in triggers(&target, probing.then_some(driver.as_deref())) {
        println!("{trigger}");
    }
    let decision = decide();
    match &decision {
        Decision::Apply(flag) => println!("cargo:rustc-link-arg-bins={flag}"),
        Decision::Broken(why) => {
            println!("cargo:warning=scootbar: hot-text order file skipped: {why}")
        }
        Decision::Skip(_) => {}
    }
    if env::var_os("SCOOTBAR_ORDERFILE_VERBOSE").is_some() {
        let note = match &decision {
            Decision::Apply(flag) => format!("applied ({flag})"),
            Decision::Skip(why) | Decision::Broken(why) => format!("skipped: {why}"),
        };
        println!("cargo:warning=scootbar: hot-text order file {note}");
    }
}

/// The `cargo:` lines that make Cargo run this script again: whatever the
/// decision rests on. The probe is the real link, so it depends on the linker
/// (found through `PATH`, named by `RUSTC_LINKER` or `CARGO_TARGET_*_LINKER`,
/// or inside the toolchain, which `RUSTC` names and whose version Cargo
/// already tracks), on the flags that shape the link, and on this script's
/// own switches. Without these a linker swapped under an unchanged tree kept
/// the answer given for the old one. `probe` is `None` when the build does
/// not probe (nothing then depends on the linker); else the driver's file,
/// watched for an in-place upgrade, `None` inside when the linker is `rustc`'s
/// own.
fn triggers(target: &str, probe: Option<Option<&Path>>) -> Vec<String> {
    let target_env = target.to_uppercase().replace(['-', '.'], "_");
    let mut lines = vec![
        "cargo:rerun-if-changed=build.rs".to_string(),
        format!("cargo:rerun-if-changed={ORDER_FILE}"),
    ];
    for var in ["SCOOTBAR_NO_ORDERFILE", "SCOOTBAR_ORDERFILE_VERBOSE"] {
        lines.push(format!("cargo:rerun-if-env-changed={var}"));
    }
    let Some(linker) = probe else { return lines };
    for var in ["PATH", "RUSTC", "RUSTC_LINKER", "CARGO_ENCODED_RUSTFLAGS"] {
        lines.push(format!("cargo:rerun-if-env-changed={var}"));
    }
    for suffix in ["LINKER", "RUSTFLAGS"] {
        lines.push(format!(
            "cargo:rerun-if-env-changed=CARGO_TARGET_{target_env}_{suffix}"
        ));
    }
    if let Some(path) = linker {
        lines.push(format!("cargo:rerun-if-changed={}", path.display()));
    }
    lines
}

/// The linker driver `rustc` will start unless it brings its own: the one
/// `RUSTC_LINKER` or a `-C linker` flag names, else `cc`, found on `PATH`.
fn link_driver() -> Option<PathBuf> {
    let flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let name = linker_named(&flags)
        .or_else(|| env::var("RUSTC_LINKER").ok())
        .unwrap_or_else(|| "cc".to_string());
    which(&name, env::var_os("PATH").as_deref())
}

/// The value of the last `-C linker=` in the encoded flags, in any spelling.
fn linker_named(encoded: &str) -> Option<String> {
    let flags: Vec<&str> = encoded.split('\x1f').collect();
    let mut found = None;
    for (i, flag) in flags.iter().enumerate() {
        let option = match *flag {
            "-C" | "--codegen" => flags.get(i + 1).copied(),
            _ => flag
                .strip_prefix("-C")
                .or_else(|| flag.strip_prefix("--codegen=")),
        };
        if let Some(value) = option.and_then(|option| option.strip_prefix("linker=")) {
            found = Some(value.to_string());
        }
    }
    found
}

/// `name` as a file: itself when it has a `/`, else the first match on `path`.
fn which(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    if name.contains('/') {
        return Some(PathBuf::from(name));
    }
    env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Whether this build runs the probe at all: a release build for Linux. Only
/// then does anything depend on the linker, so only then is it worth a
/// trigger (a trigger that fires rebuilds the crate).
fn probes() -> bool {
    env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && env::var("PROFILE").as_deref() == Ok("release")
}

fn decide() -> Decision {
    let skip = |why: &str| Decision::Skip(why.to_string());
    if env::var_os("SCOOTBAR_NO_ORDERFILE").is_some() {
        return skip("SCOOTBAR_NO_ORDERFILE is set");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return skip("not a Linux target");
    }
    // Only a release build, where the memory it saves is the one that is
    // run: the builds of the edit-compile-test loop (`cargo build`, `cargo
    // test`, rust-analyzer) neither pay for the probe nor are touched.
    if !probes() {
        return skip("not a release build");
    }
    let (Some(out), Some(manifest), Ok(target)) = (
        env::var_os("OUT_DIR").map(PathBuf::from),
        env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from),
        env::var("TARGET"),
    ) else {
        return skip("Cargo did not say where to build");
    };
    let Ok(bytes) = fs::read(manifest.join(ORDER_FILE)) else {
        return skip("the order file cannot be read");
    };
    // The linker is given a copy named for its contents: Cargo relinks a
    // binary only when its link arguments change, so a changed file under
    // the same path would leave the old order in a binary it did not
    // rebuild. (It also keeps the manifest's path, which may hold a comma,
    // out of the `-Wl,` argument.)
    let file = out.join(format!("hot-text-{:016x}.ld", fnv1a(&bytes)));
    if fs::write(&file, &bytes).is_err() {
        return skip("a copy of the order file cannot be written");
    }
    let Some(path) = file.to_str().filter(|path| !path.contains(',')) else {
        return skip("the build directory's path cannot go in a -Wl, argument");
    };
    let probe = Probe {
        rustc: env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()),
        target,
        linker: env::var("RUSTC_LINKER").ok(),
        flags: env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default(),
        dir: out,
    };
    let flag = format!("-Wl,--section-ordering-file={path}");
    match probe.links(Some(&flag)) {
        Ok(()) => Decision::Apply(flag),
        // Refused. Is it the flag, or the probe?
        Err(refused) => match probe.links(None) {
            Ok(()) => skip(&format!("the linker does not take the option ({refused})")),
            Err(broken) => Decision::Broken(format!(
                "a one-line program does not link even without it, so nothing is known about the linker ({broken})"
            )),
        },
    }
}

/// FNV-1a, 64 bits: a name for a file's contents, nothing more.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A one-line program, linked the way Cargo links the bar.
struct Probe {
    rustc: std::ffi::OsString,
    target: String,
    /// `RUSTC_LINKER`: `target.<triple>.linker` from Cargo's configuration,
    /// which Cargo hands `rustc` as `-C linker=` (the flags may override it).
    linker: Option<String>,
    /// `CARGO_ENCODED_RUSTFLAGS`: flags separated by `\x1f`.
    flags: String,
    /// Where the program and its output go.
    dir: PathBuf,
}

impl Probe {
    /// What `rustc` is given, before `-o`: the target, the configured linker,
    /// then every `RUSTFLAGS` flag as written, so that `rustc` reads each
    /// spelling and a later flag overrides an earlier one as it does for the
    /// bar, and last the order-file argument.
    fn args(&self, flag: Option<&str>) -> Vec<String> {
        let mut args: Vec<String> = [
            "--edition",
            "2021",
            "--crate-type",
            "bin",
            "--crate-name",
            "order_probe",
        ]
        .map(String::from)
        .to_vec();
        args.extend(["--target".to_string(), self.target.clone()]);
        if let Some(linker) = &self.linker {
            args.extend(["-C".to_string(), format!("linker={linker}")]);
        }
        args.extend(
            self.flags
                .split('\x1f')
                .filter(|f| !f.is_empty())
                .map(String::from),
        );
        // The probe is not the bar: its warnings and lints are nobody's.
        args.extend(["--cap-lints".to_string(), "allow".to_string()]);
        if let Some(flag) = flag {
            args.extend(["-C".to_string(), format!("link-arg={flag}")]);
        }
        args
    }

    /// Links the probe, `Err` with the first line `rustc` said when it fails.
    fn links(&self, flag: Option<&str>) -> Result<(), String> {
        let source = self.dir.join("order-probe.rs");
        let exe = self.dir.join("order-probe");
        fs::write(&source, "fn main() {}\n").map_err(|e| e.to_string())?;
        let out = Command::new(&self.rustc)
            .args(self.args(flag))
            .arg("-o")
            .arg(&exe)
            .arg(&source)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", self.rustc.to_string_lossy()))?;
        let _ = fs::remove_file(&exe);
        if out.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        // The linker's own complaint ("ld.lld: error: unknown argument ...")
        // is what to show, in one line, not rustc's summary of it.
        let line = stderr
            .lines()
            .rev()
            .find(|line| {
                (line.contains("rror") || line.contains("unknown") || line.contains("unrecognized"))
                    && !line.contains("aborting due to")
                    && !line.contains("collect2")
                    && !line.contains("linking with")
            })
            .or_else(|| stderr.lines().next())
            .unwrap_or("no output");
        Err(line.trim().chars().take(200).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// The compiler's own host, the one target it is sure to have installed.
    fn host() -> String {
        let out = Command::new("rustc").arg("-vV").output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("host: ").map(String::from))
            .unwrap()
    }

    fn probe(flags: &[&str]) -> Probe {
        Probe {
            rustc: "rustc".into(),
            target: host(),
            linker: None,
            flags: flags.join("\x1f"),
            dir: env::temp_dir(),
        }
    }

    /// A scratch directory of this test's own.
    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("scootbar-build-rs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A linker driver that logs its arguments, makes its output file (the
    /// `-o` argument) and exits with `code`.
    fn fake_linker(dir: &Path, code: i32) -> PathBuf {
        let path = dir.join(format!("fake-linker-{code}"));
        let log = dir.join("log");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >>{}\n\
                 prev=; for a in \"$@\"; do [ \"$prev\" = -o ] && : >\"$a\"; prev=$a; done\nexit {code}\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Every argument the fake linker was started with, over all its runs.
    fn logged(dir: &Path) -> Vec<String> {
        fs::read_to_string(dir.join("log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn the_name_of_a_file_follows_its_contents() {
        // The published test vectors of FNV-1a (64 bits).
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"one order"), fnv1a(b"another order"));
    }

    #[test]
    fn the_probe_gets_the_flags_as_written_and_the_flag_last() {
        // `rustc` reads the spellings; the probe must not rewrite them.
        for flags in [
            vec!["-C", "link-arg=-fuse-ld=lld"],
            vec!["-Clink-arg=-fuse-ld=lld"],
            vec!["--codegen", "link-arg=-fuse-ld=lld"],
            vec!["--codegen=link-arg=-fuse-ld=lld"],
            vec!["-Clink-args=-fuse-ld=mold -static"],
            vec!["-Zlinker-features=+lld"],
            vec!["-C", "link-self-contained=+linker"],
        ] {
            let args = probe(&flags).args(Some("-Wl,--x"));
            let at = args
                .windows(flags.len())
                .position(|w| w == flags)
                .unwrap_or_else(|| panic!("{flags:?} in {args:?}"));
            assert!(at > 0);
            assert_eq!(
                &args[args.len() - 2..],
                ["-C", "link-arg=-Wl,--x"],
                "{flags:?}"
            );
        }
        assert!(!probe(&[]).args(None).iter().any(|a| a.contains("link-arg")));
    }

    #[test]
    fn the_configured_linker_comes_before_the_flags_so_a_flag_overrides_it() {
        let mut p = probe(&["-C", "linker=clang"]);
        p.linker = Some("gcc".into());
        let args = p.args(None);
        let configured = args.iter().position(|a| a == "linker=gcc").unwrap();
        let flagged = args.iter().position(|a| a == "linker=clang").unwrap();
        assert!(configured < flagged);
    }

    #[test]
    fn the_target_is_passed() {
        let mut p = probe(&[]);
        p.target = "aarch64-unknown-linux-musl".into();
        let args = p.args(None);
        let at = args.iter().position(|a| a == "--target").unwrap();
        assert_eq!(args[at + 1], "aarch64-unknown-linux-musl");
    }

    #[test]
    fn each_spelling_reaches_the_linker_the_bar_would_get() {
        // The fake linker is `rustc`'s driver here, so what it logs is what a
        // real linker driver would be handed, including `-fuse-ld` from every
        // spelling of a link argument. This is the case a trial by `cc` missed.
        let dir = scratch("spellings");
        let linker = fake_linker(&dir, 1);
        for (i, spelling) in [
            vec!["-C".to_string(), "link-arg=-fuse-ld=lld".to_string()],
            vec!["-Clink-arg=-fuse-ld=lld".to_string()],
            vec!["--codegen".to_string(), "link-arg=-fuse-ld=lld".to_string()],
            vec!["--codegen=link-arg=-fuse-ld=lld".to_string()],
        ]
        .iter()
        .enumerate()
        {
            let mut flags = spelling.clone();
            flags.splice(
                0..0,
                ["-C".to_string(), format!("linker={}", linker.display())],
            );
            let p = Probe {
                flags: flags.join("\x1f"),
                dir: dir.clone(),
                ..probe(&[])
            };
            assert!(p.links(Some("-Wl,--section-ordering-file=/x")).is_err());
            let log = logged(&dir);
            assert!(
                log.iter().any(|a| a == "-fuse-ld=lld"),
                "spelling {i} {spelling:?}: {log:?}"
            );
            assert!(log.iter().any(|a| a == "-Wl,--section-ordering-file=/x"));
            let _ = fs::remove_file(dir.join("log"));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_driver_from_cargos_configuration_is_the_one_probed() {
        let dir = scratch("configured");
        let linker = fake_linker(&dir, 1);
        let p = Probe {
            linker: Some(linker.display().to_string()),
            dir: dir.clone(),
            ..probe(&[])
        };
        assert!(p.links(Some("-Wl,--x")).is_err());
        assert!(logged(&dir).iter().any(|a| a == "-Wl,--x"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_linker_that_takes_the_flag_passes_and_one_that_refuses_fails() {
        let dir = scratch("verdict");
        let (yes, no) = (fake_linker(&dir, 0), fake_linker(&dir, 1));
        let with = |linker: &Path| Probe {
            flags: ["-C".to_string(), format!("linker={}", linker.display())].join("\x1f"),
            dir: dir.clone(),
            ..probe(&[])
        };
        assert_eq!(with(&yes).links(Some("-Wl,--x")), Ok(()));
        assert!(with(&no).links(Some("-Wl,--x")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_compiler_is_an_error_not_a_panic() {
        let p = Probe {
            rustc: "/nonexistent/rustc".into(),
            ..probe(&[])
        };
        assert!(p.links(None).is_err());
    }

    #[test]
    fn a_linker_flag_is_found_in_any_spelling_and_the_last_wins() {
        let f = |list: &[&str]| list.join("\x1f");
        assert_eq!(linker_named(&f(&["-C", "linker=a"])).as_deref(), Some("a"));
        assert_eq!(linker_named(&f(&["-Clinker=b"])).as_deref(), Some("b"));
        assert_eq!(
            linker_named(&f(&["--codegen", "linker=c"])).as_deref(),
            Some("c")
        );
        assert_eq!(
            linker_named(&f(&["--codegen=linker=d"])).as_deref(),
            Some("d")
        );
        assert_eq!(
            linker_named(&f(&["-Clinker=a", "-C", "linker=z"])).as_deref(),
            Some("z")
        );
        assert_eq!(linker_named(&f(&["-C", "link-arg=-fuse-ld=lld"])), None);
        assert_eq!(linker_named(""), None);
    }

    #[test]
    fn the_decision_is_made_again_when_what_it_rests_on_changes() {
        // A linker swapped under an unchanged tree used to keep the answer
        // given for the old one: no trigger named PATH.
        let t = triggers(
            "x86_64-unknown-linux-gnu",
            Some(Some(Path::new("/usr/bin/cc"))),
        );
        for wanted in [
            "cargo:rerun-if-env-changed=PATH",
            "cargo:rerun-if-env-changed=RUSTC",
            "cargo:rerun-if-env-changed=RUSTC_LINKER",
            "cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS",
            "cargo:rerun-if-env-changed=SCOOTBAR_NO_ORDERFILE",
            "cargo:rerun-if-env-changed=SCOOTBAR_ORDERFILE_VERBOSE",
            "cargo:rerun-if-env-changed=CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "cargo:rerun-if-env-changed=CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
            "cargo:rerun-if-changed=/usr/bin/cc",
            "cargo:rerun-if-changed=orderfile/hot-text.ld",
            "cargo:rerun-if-changed=build.rs",
        ] {
            assert!(t.iter().any(|l| l == wanted), "{wanted} in {t:?}");
        }
        assert!(
            !triggers("x86_64-unknown-linux-gnu", None)
                .iter()
                .any(|l| l.contains("/usr/bin/cc"))
        );
    }

    #[test]
    fn a_name_is_looked_up_on_the_path_not_in_the_directory() {
        let dir = scratch("which");
        let tool = dir.join("cc-here");
        fs::write(&tool, "").unwrap();
        let path = env::join_paths([Path::new("/nonexistent"), dir.as_path()]).unwrap();
        assert_eq!(which("cc-here", Some(&path)), Some(tool));
        assert_eq!(which("cc-absent", Some(&path)), None);
        assert_eq!(which("/opt/x/ld", None), Some(PathBuf::from("/opt/x/ld")));
        assert_eq!(which("cc", None), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
