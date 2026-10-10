//! The `runtime-gbm` spike's view of the static libgbm stub
//! (`gbm-stub/gbm_stub.c`).
//!
//! Only compiled with the spike feature: the stub archive is what satisfies
//! gbm-sys's `-lgbm` there, so this symbol exists exactly then. The single
//! question it answers -- is the process currently forwarding GBM to a real
//! libgbm, or failing closed? -- is what the fail-closed test pins, and what
//! a future auto-detect rule would ask before choosing the GPU tier.

/// Whether the stub has a real libgbm loaded (1) or is failing closed (0).
/// Stable after the first call: the dlopen is attempted once per process.
///
/// Test-only for the spike: nothing in production asks yet (the
/// auto-detect rule in `docs/backlog/core/runtime-gbm-productionise.md`
/// will), and an unasked question must not ship as dead code. The `extern` block below
/// stays unconditional -- an unreferenced declaration warns nothing, and it
/// keeps the symbol's contract beside its stub.
#[cfg(test)]
pub(crate) fn real_loaded() -> bool {
    // SAFETY: plain C call with no Rust-side invariants; returns 0 or 1.
    unsafe { scoot_gbm_real_loaded() != 0 }
}

#[cfg(test)]
unsafe extern "C" {
    fn scoot_gbm_real_loaded() -> std::os::raw::c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub answers, whatever the box provides: with a system libgbm
    /// this is 1, without it 0. Either is a pass -- what would fail is a
    /// link error or a crash on the dlopen path.
    #[test]
    fn the_stub_answers_stably() {
        let first = real_loaded();
        // Printed, not asserted: 0 on a box without libgbm, 1 with one.
        // Either passes; the value in the log is what proves which
        // implementation answered on that machine (run with `-- --nocapture`).
        eprintln!("scoot_gbm_real_loaded={first}");
        assert_eq!(real_loaded(), first, "the probe must be stable");
    }

    /// Fail-closed where no DRM device can exist: `/dev/null` is never a
    /// DRM node, so `GbmDevice::new` must refuse it -- through the real
    /// libgbm's own refusal where one is installed, and through the
    /// stub's ENOSYS where none is. This is the runtime half of the
    /// DT_NEEDED proof: the GPU tier's constructor degrades to its
    /// existing `Err` rather than aborting the process.
    #[test]
    fn gbm_device_on_a_non_drm_fd_is_a_loud_refusal() {
        use smithay::backend::allocator::gbm::GbmDevice;

        let null = std::fs::File::open("/dev/null").expect("/dev/null opens");
        let error = GbmDevice::new(null).expect_err("/dev/null is never a GBM device");
        // Loud, not silent: the refusal carries the reason either way.
        assert!(!error.to_string().is_empty(), "{error:?}");
    }
}
