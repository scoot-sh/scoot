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
    // Linked as an object, by path: the stub must be in the final binary
    // with no DT_NEEDED entry, however the toolchain orders its link
    // line. A static archive only satisfies symbols undefined where the
    // linker reaches it under `--as-needed`, so its position decides --
    // and nothing contracts that position. A bare object file is
    // unconditional: always taken, from any position, exactly once.
    // Compiled with the same flags `compile()` would use (same `cc`
    // compiler object, only `-c -o` added), then passed as a plain
    // input -- never a `-l`, so there is no modifier for rustc to
    // collide and no archive for `--as-needed` to drop.
    //
    // gbm-sys still emits its own plain `-lgbm` (its `#[link]`
    // attribute), which must RESOLVE even though it must never provide:
    // with no system libgbm anywhere the link dies on "unable to find
    // library -lgbm". So an empty libgbm.a sits beside the object,
    // added to the search path: `-lgbm` finds the archive, pulls no
    // member from it, emits no DT_NEEDED. Empty means the eight magic
    // bytes and nothing else, written directly -- no archiver to
    // locate.
    //
    // No LTO bytecode in the object, ever: distro `CFLAGS` may carry
    // `-flto` (makepkg's default is `-flto=auto`), which `cc` inherits
    // and which bakes `.gnu.lto_*` sections into anything it compiles.
    // Under GNU ld that links fine (the Fedora RPM builds prove it),
    // but under `ld.lld` GCC's `liblto_plugin` claims the file and its
    // symbols never materialize -- every `gbm_*` undefined with a valid
    // object on the line (proven on arch.yml: `--trace-symbol` showed
    // the reference with no definition, and the same link with
    // `-fno-use-linker-plugin` succeeded). The Arch PKGBUILDs therefore
    // carry `!lto` (packaging/arch/README.md): only the C compile drops
    // LTO there, Rust's own `lto = "fat"` is untouched. A from-source
    // build with `-flto` CFLAGS under lld would hit the same wall; that
    // combination is the distro's to configure, and it fails loudly at
    // link time, never silently.
    //
    // The ldd gate in CI (`nix develop` Linux job) still proves no
    // DT_NEEDED libgbm on every change.
    //
    // Linux-only like the libinput block above (the stub is meaningless
    // where `backend_gbm` cannot link anyway): without the gate a macOS
    // default-features build would demand a C compiler for nothing.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var("CARGO_FEATURE_GPU_SCANOUT").is_ok()
    {
        let out = std::env::var("OUT_DIR").expect("OUT_DIR is set for build scripts");
        let dir = std::path::Path::new(&out);
        let compiler = cc::Build::new()
            .file("gbm-stub/gbm_stub.c")
            .try_get_compiler()
            .expect("a C compiler for the gbm stub");
        let object = dir.join("gbm_stub.o");
        let status = compiler
            .to_command()
            .arg("-c")
            .arg("gbm-stub/gbm_stub.c")
            .arg("-o")
            .arg(&object)
            .status()
            .expect("could not run the C compiler for the gbm stub");
        assert!(status.success(), "the gbm stub did not compile");
        println!("cargo:rustc-link-arg={}", object.display());
        // An empty GNU archive: the magic alone, no members.
        std::fs::write(dir.join("libgbm.a"), b"!<arch>\n")
            .expect("could not stage the empty libgbm.a");
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}
