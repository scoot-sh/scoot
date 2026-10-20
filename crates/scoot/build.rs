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
    // `+whole-archive` (not a bare `static=`): a bare static archive only
    // satisfies symbols that are undefined at the point the linker reaches
    // it under `--as-needed`, so its position in rustc's link line decides
    // whether the GBM symbols resolve -- and nothing contracts that
    // position. Arch's toolchain orders it where nothing is pending yet,
    // and the link dies on undefined `gbm_*` (seen on arch.yml while the
    // ubuntu and nix links stayed green). Whole-archive pulls every stub
    // object wherever the archive lands, still with no DT_NEEDED (static
    // objects, not a shared library). `cc` also emits its own plain
    // `static=gbm` for the same archive; the second pull finds every
    // symbol already defined and takes nothing, so the pair is harmless.
    // The ldd gate in CI (`nix develop` Linux job) still proves no
    // DT_NEEDED libgbm on every change.
    //
    // Linux-only like the libinput block above (the stub is meaningless
    // where `backend_gbm` cannot link anyway): without the gate a macOS
    // default-features build would demand a C compiler for nothing.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_FEATURE_GPU_SCANOUT").is_ok()
    {
        cc::Build::new().file("gbm-stub/gbm_stub.c").compile("gbm");
        println!("cargo:rustc-link-lib=static:+whole-archive=gbm");
    }
}
