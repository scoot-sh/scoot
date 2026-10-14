//! Sets `SCOOT_BUILD_VERSION`, the version `--version` and the IPC
//! `version` reply print (`crates/scoot-ipc/build_label.rs` says how it is
//! worked out).

#[path = "../scoot-ipc/build_label.rs"]
mod build_label;

fn main() {
    build_label::emit();
}
