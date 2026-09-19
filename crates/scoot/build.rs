//! Only needed for `--tty`'s `backend_libinput`: unlike every other
//! pkg-config-based sys crate this project links against (libseat-sys,
//! udev-sys, xkbcommon-sys), `input-sys` (libinput's raw FFI bindings) has
//! no build-script logic of its own to locate libinput's shared library --
//! it only picks which prebuilt bindings to use for the installed version.
//! On NixOS the actual `.so` lives under the package's own `/nix/store`
//! path, which is on no default linker search path, so without this the
//! final link step fails with "cannot find -linput" even though
//! `pkg-config --exists libinput` succeeds. See the `Cargo.toml` comment on
//! the `pkg-config` build-dependency this uses.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    match pkg_config::Config::new().probe("libinput") {
        Ok(library) => {
            for path in library.link_paths {
                println!("cargo:rustc-link-search=native={}", path.display());
            }
        }
        Err(error) => {
            // Don't fail the build over this -- a system where libinput
            // truly isn't installed will still fail at the link step with
            // its own (if less friendly) error, same as before this file
            // existed.
            println!("cargo:warning=could not locate libinput via pkg-config: {error}");
        }
    }
}
