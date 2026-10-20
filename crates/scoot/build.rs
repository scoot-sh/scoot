//! Two jobs: the version label (below), and, for `--tty`'s
//! `backend_libinput`, locating libinput. Unlike every other
//! pkg-config-based sys crate this project links against (libseat-sys,
//! udev-sys, xkbcommon-sys), `input-sys` (libinput's raw FFI bindings) has
//! no build-script logic of its own to locate libinput's shared library --
//! it only picks which prebuilt bindings to use for the installed version.
//! On NixOS the actual `.so` lives under the package's own `/nix/store`
//! path, which is on no default linker search path, so without this the
//! final link step fails with "cannot find -linput" even though
//! `pkg-config --exists libinput` succeeds. See the `Cargo.toml` comment on
//! the `pkg-config` build-dependency this uses.

// `SCOOT_BUILD_VERSION`, the version `--version` and the IPC `version`
// reply print, on every platform (`crates/scoot-ipc/build_label.rs` says
// how it is worked out).
#[path = "../scoot-ipc/build_label.rs"]
mod build_label;

fn main() {
    build_label::emit();
    // `emit` scopes reruns to what it names, so name the stub source too.
    println!("cargo:rerun-if-changed=gbm-stub/gbm_stub.c");
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

    // The `gpu-scanout` static libgbm ABI stub (see `gbm-stub/gbm_stub.c` and
    // the feature's comment in `Cargo.toml`): satisfies gbm-sys's `-lgbm`
    // without emitting a DT_NEEDED entry, forwarding to the real libgbm by
    // dlopen only when a GBM entry point is actually called. Gated on the
    // scanout feature, so the `--no-default-features` minimal build compiles
    // no C at all (and needs no C compiler).
    //
    // `cc` compiles the stub into OUT_DIR and links it statically (its
    // `static=gbm` arrives alongside gbm-sys's plain `-lgbm`; with the
    // linker's default --as-needed the archive resolves the GBM symbols
    // and no DT_NEEDED is emitted -- verified both with and without a
    // system libgbm present). Ordering assumption, recorded for the debugger
    // of a future `ldd`-gate failure: `static=gbm` must precede the plain
    // `-lgbm` under `--as-needed`, and nothing in cargo/rustc contracts that
    // order. A flipped order fails loudly -- a `DT_NEEDED libgbm` the gate
    // catches (the archive simply goes unpulled) -- never silently.
    //
    // Linux-only like the libinput block above (the stub is meaningless
    // where `backend_gbm` cannot link anyway): without the gate a macOS
    // default-features build would demand a C compiler for nothing.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_FEATURE_GPU_SCANOUT").is_ok()
    {
        cc::Build::new().file("gbm-stub/gbm_stub.c").compile("gbm");
    }
}
