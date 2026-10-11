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

    // The `runtime-gbm` spike (see `gbm-stub/gbm_stub.c` and the feature's
    // comment in `Cargo.toml`): a static libgbm ABI stub that satisfies
    // gbm-sys's `-lgbm` without emitting a DT_NEEDED entry, forwarding to
    // the real libgbm by dlopen only when a GBM entry point is actually
    // called. Gated on the spike feature alone, so the `gpu-scanout`
    // feature's link behavior is byte-identical with and without this.
    // `cc` compiles the stub into OUT_DIR and links it statically (its
    // `static=gbm` arrives alongside gbm-sys's plain `-lgbm`; with the
    // linker's default --as-needed the archive resolves the GBM symbols
    // and no DT_NEEDED is emitted -- verified both with and without a
    // system libgbm present).
    //
    // Linux-only like the libinput block above (the stub is meaningless
    // where `backend_gbm` cannot link anyway): without the gate a macOS
    // `--features runtime-gbm` build would demand a C compiler for nothing.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_FEATURE_RUNTIME_GBM").is_ok()
    {
        cc::Build::new().file("gbm-stub/gbm_stub.c").compile("gbm");
    }
}
