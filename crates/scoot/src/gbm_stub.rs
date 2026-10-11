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
/// Asked by the `auto` renderer policy before choosing the GPU tier (see
/// `compositor::render::policy::on_device`): without a loadable libgbm
/// there is no GPU stack to win on, so `auto` stays on the CPU renderer.
pub(crate) fn real_loaded() -> bool {
    // SAFETY: plain C call with no Rust-side invariants; returns 0 or 1.
    unsafe { scoot_gbm_real_loaded() != 0 }
}

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

    /// The stub's contract on a fd that can never be a DRM device.
    /// Without a system libgbm (fail-closed) this must be a loud `Err` --
    /// the runtime half of the DT_NEEDED proof. With a real libgbm the call
    /// forwards to mesa, whose answer is mesa's business: it refuses most
    /// bad fds, but `/dev/null` gets a zombie device that fails later
    /// (measured: `gbm_create_device` succeeds with "failed to get driver
    /// name" on stderr). Either way the process must survive the call --
    /// no abort, no hang -- and scoot's own `try_scanout` turns any later
    /// refusal into its warned dumb-buffer fallback.
    #[test]
    fn gbm_device_on_a_non_drm_fd_is_survived() {
        use smithay::backend::allocator::gbm::GbmDevice;

        let null = std::fs::File::open("/dev/null").expect("/dev/null opens");
        let answer = GbmDevice::new(null);
        if real_loaded() {
            // Forwarded: mesa answered (Ok or Err, both are its answer).
            eprintln!("forwarded answer: {}", answer.is_ok());
        } else {
            // Fail-closed: the stub refused loudly.
            let error = answer.expect_err("/dev/null must be refused fail-closed");
            assert!(!error.to_string().is_empty(), "{error:?}");
        }
    }
}
