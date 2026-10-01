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
//! only after a trial link of an empty program with this very file has
//! worked through the linker `rustc` will use, given the link arguments the
//! `RUSTFLAGS` add, so a linker without the option (binutils before 2.43,
//! lld, mold, gold) links as it always did, and so does a file the linker
//! will not read. Any other reason to skip it is below; none fails the build.
//!
//! - not Linux, or a cross build (the trial runs the host's linker);
//! - `SCOOTBAR_NO_ORDERFILE` set, to see what the layout costs;
//! - `RUSTFLAGS` with an unstable `-Z` flag or `link-self-contained`, which
//!   change how the link is done in ways the trial does not repeat;
//! - a path the `-Wl,` syntax cannot carry (a comma).
//!
//! The pure parts have unit tests, which Cargo does not run for a build
//! script: `rustc --edition 2024 --test crates/scootbar/build.rs -o /tmp/t && /tmp/t`.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The order file, relative to the crate.
const ORDER_FILE: &str = "orderfile/hot-text.ld";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ORDER_FILE}");
    println!("cargo:rerun-if-env-changed=SCOOTBAR_NO_ORDERFILE");
    println!("cargo:rerun-if-env-changed=RUSTC_LINKER");

    if let Some(flag) = link_flag() {
        println!("cargo:rustc-link-arg-bins={flag}");
    }
}

/// `-Wl,--section-ordering-file=PATH` when it should be passed.
fn link_flag() -> Option<String> {
    if env::var_os("SCOOTBAR_NO_ORDERFILE").is_some()
        || env::var("CARGO_CFG_TARGET_OS").ok()? != "linux"
        || env::var("TARGET").ok()? != env::var("HOST").ok()?
    {
        return None;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR")?);
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR")?).join(ORDER_FILE);
    let bytes = fs::read(source).ok()?;
    // The linker is given a copy named for its contents: Cargo relinks a
    // binary only when its link arguments change, so a changed file under
    // the same path would leave the old order in a binary it did not
    // rebuild. (It also keeps the manifest's path, which may hold a comma,
    // out of the `-Wl,` argument.)
    let file = out.join(format!("hot-text-{:016x}.ld", fnv1a(&bytes)));
    fs::write(&file, &bytes).ok()?;
    let path = file.to_str().filter(|path| !path.contains(','))?;
    let mut link = Link::from_flags(&env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default())?;
    if let Some(linker) = env::var("RUSTC_LINKER")
        .ok()
        .filter(|_| link.driver.is_none())
    {
        link.driver = Some(linker);
    }
    let flag = format!("-Wl,--section-ordering-file={path}");
    link.takes(&flag, &out).then_some(flag)
}

/// FNV-1a, 64 bits: a name for a file's contents, nothing more.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// How `rustc` will start the linker: the driver (`cc` unless the flags or
/// Cargo's configuration name another) and the arguments `RUSTFLAGS` add.
#[derive(Debug, Default, PartialEq)]
struct Link {
    driver: Option<String>,
    args: Vec<String>,
}

impl Link {
    /// Reads `CARGO_ENCODED_RUSTFLAGS` (flags separated by `\x1f`) for what
    /// shapes the link: `-C linker=`, `-C link-arg=`, `-C link-args=`, in either
    /// spelling (`-C x=y` or `-Cx=y`). `None` when the flags do more than that
    /// to the link (an unstable `-Z` flag, `link-self-contained`), so that
    /// nothing is guessed.
    fn from_flags(encoded: &str) -> Option<Self> {
        let flags: Vec<&str> = encoded
            .split('\x1f')
            .filter(|flag| !flag.is_empty())
            .collect();
        let mut link = Self::default();
        let mut i = 0;
        while i < flags.len() {
            let flag = flags[i];
            i += 1;
            if flag.starts_with("-Z") {
                return None;
            }
            let option = if flag == "-C" {
                let next = flags.get(i).copied().unwrap_or_default();
                i += 1;
                next
            } else if let Some(option) = flag.strip_prefix("-C") {
                option
            } else {
                continue;
            };
            let (key, value) = option.split_once('=').unwrap_or((option, ""));
            match key {
                "linker" => link.driver = Some(value.to_string()),
                "link-arg" => link.args.push(value.to_string()),
                "link-args" => link
                    .args
                    .extend(value.split_whitespace().map(str::to_string)),
                key if key.starts_with("link-self-contained") || key.starts_with("linker-") => {
                    return None;
                }
                _ => {}
            }
        }
        Some(link)
    }

    /// Links an empty program with `flag`, the way `rustc` will link the
    /// bar, and says whether that worked.
    fn takes(&self, flag: &str, out: &Path) -> bool {
        let exe = out.join("order-probe");
        let child = Command::new(self.driver.as_deref().unwrap_or("cc"))
            .args(["-x", "c", "-", "-o"])
            .arg(&exe)
            .args(&self.args)
            .arg(flag)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        if let Some(mut stdin) = child.stdin.take() {
            // Dropped at the end of the block, so the compiler sees the end
            // of its input.
            if stdin.write_all(b"int main(void) { return 0; }\n").is_err() {
                return false;
            }
        }
        let worked = child.wait().is_ok_and(|status| status.success());
        let _ = fs::remove_file(&exe);
        worked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(list: &[&str]) -> String {
        list.join("\x1f")
    }

    #[test]
    fn the_name_of_a_file_follows_its_contents() {
        // The published test vectors of FNV-1a (64 bits).
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(fnv1a(b"one order"), fnv1a(b"another order"));
    }

    #[test]
    fn no_flags_is_the_default_driver_and_no_arguments() {
        assert_eq!(Link::from_flags(""), Some(Link::default()));
    }

    #[test]
    fn a_link_arg_in_either_spelling_is_passed_to_the_trial() {
        let link = Link::from_flags(&flags(&[
            "-C",
            "link-arg=-fuse-ld=lld",
            "-Clink-arg=-Wl,--as-needed",
        ]));
        assert_eq!(link.unwrap().args, ["-fuse-ld=lld", "-Wl,--as-needed"]);
    }

    #[test]
    fn link_args_splits_on_whitespace() {
        let link = Link::from_flags(&flags(&["-Clink-args=-B/opt/mold  -static-libgcc"])).unwrap();
        assert_eq!(link.args, ["-B/opt/mold", "-static-libgcc"]);
    }

    #[test]
    fn a_linker_flag_names_the_driver() {
        let link = Link::from_flags(&flags(&["-C", "linker=clang"])).unwrap();
        assert_eq!(link.driver.as_deref(), Some("clang"));
    }

    #[test]
    fn flags_that_do_not_touch_the_link_are_ignored() {
        let link = Link::from_flags(&flags(&[
            "-C",
            "target-cpu=native",
            "--cfg",
            "x",
            "-Dwarnings",
            "-Copt-level=2",
        ]));
        assert_eq!(link, Some(Link::default()));
    }

    #[test]
    fn what_the_trial_cannot_repeat_is_not_guessed_at() {
        assert_eq!(Link::from_flags(&flags(&["-Zlinker-features=+lld"])), None);
        assert_eq!(
            Link::from_flags(&flags(&["-C", "link-self-contained=+linker"])),
            None
        );
        assert_eq!(Link::from_flags(&flags(&["-Clinker-flavor=gcc"])), None);
    }

    #[test]
    fn a_missing_driver_is_a_failed_trial_not_a_panic() {
        let link = Link {
            driver: Some("/nonexistent/linker".into()),
            args: vec![],
        };
        assert!(!link.takes("-Wl,--x", &env::temp_dir()));
    }

    #[test]
    fn a_linker_that_refuses_the_flag_fails_the_trial() {
        // `cc` is on any machine that links Rust; no linker has this option.
        let link = Link::default();
        assert!(!link.takes("-Wl,--no-such-option-at-all", &env::temp_dir()));
    }
}
