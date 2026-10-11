//! Tests for the `auto` renderer policy: every table row, every driver
//! name, every renderer string, and the request precedence. Pure, so all
//! headless under nextest.

use super::*;
use crate::cli::{DEFAULT_REQUEST, RendererKind, RendererRequest, RequestSource};

// -- `before_device`: Session x Request x build_has_tier --------------------

#[test]
fn an_explicit_request_resolves_before_any_device_on_every_backend() {
    use RendererRequest::{Cpu, Gpu};
    for session in [Session::Headless, Session::Nested, Session::Tty] {
        for build_has_tier in [false, true] {
            assert_eq!(
                before_device(Cpu, session, build_has_tier),
                Early::Resolved(RendererKind::Pixman, Reason::Requested),
                "{session:?} cpu"
            );
            assert_eq!(
                before_device(Gpu, session, build_has_tier),
                Early::Resolved(RendererKind::Gles, Reason::Requested),
                "{session:?} gpu"
            );
        }
    }
}

#[test]
fn auto_is_pixman_on_headless_and_nested_whatever_the_build_carries() {
    for session in [Session::Headless, Session::Nested] {
        for build_has_tier in [false, true] {
            let early = before_device(RendererRequest::Auto, session, build_has_tier);
            let (kind, reason) = match early {
                Early::Resolved(kind, reason) => (kind, reason),
                Early::DecideOnDevice(_) => {
                    panic!("{session:?} auto must resolve before any device")
                }
            };
            assert_eq!(kind, RendererKind::Pixman, "{session:?}");
            let expected = match session {
                Session::Headless => Reason::HeadlessReadsBack,
                Session::Nested => Reason::NestedUnmeasured,
                Session::Tty => unreachable!(),
            };
            assert_eq!(reason, expected, "{session:?}");
        }
    }
}

#[test]
fn auto_on_tty_needs_the_scanout_tier_to_defer() {
    assert_eq!(
        before_device(RendererRequest::Auto, Session::Tty, false),
        Early::Resolved(RendererKind::Pixman, Reason::NoScanoutTier),
    );
    assert_eq!(
        before_device(RendererRequest::Auto, Session::Tty, true),
        Early::DecideOnDevice(RendererRequest::Auto),
    );
}

// -- `on_device`: request x gbm x egl x driver -------------------------------

#[test]
fn a_forced_cpu_request_never_tries_the_gpu() {
    // Whatever the device claims: forced cpu never touches GBM or EGL, so
    // `on_device` is not even consulted for it in `open_device` -- and
    // would answer pixman if it were.
    for driver in ["", "virtio_gpu", "amdgpu", "simpledrm"] {
        for (gbm, egl) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(
                on_device(RendererRequest::Cpu, driver, gbm, egl),
                DevicePlan::Pixman(Reason::Requested),
                "{driver} gbm={gbm} egl={egl}"
            );
        }
    }
}

#[test]
fn a_forced_gpu_request_tries_without_a_software_rejection() {
    // Even on a virtual GPU: the request is explicit, so the stack is
    // tried as-is and a software renderer is kept, never refused.
    for driver in ["virtio_gpu", "simpledrm", ""] {
        assert_eq!(
            on_device(RendererRequest::Gpu, driver, true, true),
            DevicePlan::TryGpu {
                reject_software: false,
            },
            "{driver}"
        );
    }
}

#[test]
fn auto_without_gbm_or_egl_is_pixman_with_the_loader_reason() {
    assert_eq!(
        on_device(RendererRequest::Auto, "amdgpu", false, true),
        DevicePlan::Pixman(Reason::NoGbm),
    );
    assert_eq!(
        on_device(RendererRequest::Auto, "amdgpu", true, false),
        DevicePlan::Pixman(Reason::NoEgl),
    );
    // The GBM check runs first: with neither loader the reason names GBM.
    assert_eq!(
        on_device(RendererRequest::Auto, "amdgpu", false, false),
        DevicePlan::Pixman(Reason::NoGbm),
    );
}

#[test]
fn auto_on_display_only_and_virtual_devices_is_pixman() {
    for driver in [
        "simpledrm",
        "efidrm",
        "vesadrm",
        "ofdrm",
        "bochs-drm",
        "cirrus-qemu",
        "qxl",
        "vboxvideo",
        "hyperv_drm",
        "vkms",
        "udl",
        "gm12u320",
        "ast",
        "mgag200",
    ] {
        assert_eq!(
            on_device(RendererRequest::Auto, driver, true, true),
            DevicePlan::Pixman(Reason::DisplayOnly),
            "{driver}"
        );
    }
    for driver in ["virtio_gpu", "vmwgfx"] {
        assert_eq!(
            on_device(RendererRequest::Auto, driver, true, true),
            DevicePlan::Pixman(Reason::VirtualGpu),
            "{driver}"
        );
    }
}

#[test]
fn auto_on_anything_else_tries_the_gpu_with_software_rejection() {
    // Real GPUs, unknown names (which must never refuse the tier on their
    // own), the empty name (a failed ioctl), and -- critically -- the Asahi
    // display name, which must never be classified away from the GPU.
    for driver in [
        "",
        "apple-drm",
        "apple",
        "asahi",
        "i915",
        "xe",
        "amdgpu",
        "radeon",
        "nouveau",
        "nvidia-drm",
        "nvidia",
        "msm",
        "panthor",
        "v3d",
        "vc4",
        "etnaviv",
        "lima",
        "panfrost",
        "something-new",
        "SIMPLEDRM",
        "Virtio_Gpu",
    ] {
        assert_eq!(
            on_device(RendererRequest::Auto, driver, true, true),
            DevicePlan::TryGpu {
                reject_software: true,
            },
            "{driver}"
        );
    }
}

// -- driver classification ----------------------------------------------------

#[test]
fn every_listed_driver_classifies_and_nothing_else_does() {
    for driver in [
        "simpledrm",
        "efidrm",
        "vesadrm",
        "ofdrm",
        "bochs-drm",
        "cirrus-qemu",
        "qxl",
        "vboxvideo",
        "hyperv_drm",
        "vkms",
        "udl",
        "gm12u320",
        "ast",
        "mgag200",
    ] {
        assert_eq!(kms_class(driver), KmsClass::DisplayOnly, "{driver}");
    }
    for driver in ["virtio_gpu", "vmwgfx"] {
        assert_eq!(kms_class(driver), KmsClass::VirtualGpu, "{driver}");
    }
    // Real hardware, the Asahi display controller under every spelling the
    // project has seen, a failed ioctl, and near-miss capitalisations:
    // all `Other`, so the `GL_RENDERER` check decides.
    for driver in [
        "apple",
        "apple-drm",
        "asahi",
        "i915",
        "xe",
        "amdgpu",
        "nouveau",
        "nvidia-drm",
        "",
    ] {
        assert_eq!(kms_class(driver), KmsClass::Other, "{driver}");
    }
}

// -- GL renderer strings -------------------------------------------------------

#[test]
fn software_renderer_strings_are_refused_and_hardware_kept() {
    // Real Mesa output shapes, from the project's own logs and Mesa's
    // naming: each software string must classify software...
    for name in [
        "llvmpipe (LLVM 19.1.5, 256 bits)",
        "softpipe",
        "kms_swrast",
        "SGI Software Rasterizer",
        "OpenGL ES 3.1 (llvmpipe)",
        // zink on llvmpipe still contains the llvmpipe mark: software.
        "zink Vulkan 1.3(llvmpipe (LLVM 19.1.5, 256 bits))",
    ] {
        assert!(is_software_gl_renderer(name), "{name}");
    }
    // ...while real hardware -- including virgl, which must classify as
    // hardware (it reaches here only through a forced or unknown driver,
    // since virtio_gpu is filtered earlier) -- must not.
    for name in [
        "Apple M2",
        "virgl (Mesa XA on VirtIO GPU)",
        "Mali-G78",
        "Adreno (TM) 740",
        "Mesa Intel(R) Xe Graphics",
        "AMD Radeon RX 7800 XT",
        "NVIDIA GeForce RTX 4070",
    ] {
        assert!(
            !is_software_gl_renderer(name),
            "{name} must read as hardware"
        );
    }
    // Case-insensitivity: the match is ASCII case-folded.
    assert!(is_software_gl_renderer("LLVMPipe"));
    assert!(is_software_gl_renderer("SOFTWARE RASTERIZER"));
}

#[test]
fn after_renderer_rejects_only_software_under_a_trying_plan() {
    // Forced `gpu` (and later heads): never reject, even a software string.
    assert_eq!(
        after_renderer(false, Some("llvmpipe (LLVM 19.1.5, 256 bits)")),
        Ok(Reason::Requested),
    );
    assert_eq!(after_renderer(false, None), Ok(Reason::Requested));
    // Trying under `auto`: software falls back, hardware keeps the tier,
    // and an unreadable string never refuses (it is not proof of software).
    assert_eq!(
        after_renderer(true, Some("llvmpipe (LLVM 19.1.5, 256 bits)")),
        Err(Reason::SoftwareRenderer),
    );
    assert_eq!(
        after_renderer(true, Some("Apple M2")),
        Ok(Reason::HardwareRenderer),
    );
    assert_eq!(
        after_renderer(true, Some("virgl (Mesa XA on VirtIO GPU)")),
        Ok(Reason::HardwareRenderer),
    );
    assert_eq!(after_renderer(true, None), Ok(Reason::HardwareRenderer),);
}

// -- reason wording --------------------------------------------------------------

#[test]
fn reason_strings_are_pinned() {
    // The `reason=` field of the `renderer chosen` line; the override
    // sentences live in the docs and the warn lines, so these stay short
    // machine-readable identifiers.
    for (reason, text) in [
        (Reason::Requested, "requested"),
        (Reason::HeadlessReadsBack, "headless-reads-back"),
        (Reason::NestedUnmeasured, "nested-unmeasured"),
        (Reason::NoScanoutTier, "no-scanout-tier"),
        (Reason::NoGbm, "no-gbm"),
        (Reason::NoEgl, "no-egl"),
        (Reason::DisplayOnly, "display-only"),
        (Reason::VirtualGpu, "virtual-gpu"),
        (Reason::GpuBuildFailed, "gpu-build-failed"),
        (Reason::SoftwareRenderer, "software-renderer"),
        (Reason::HardwareRenderer, "hardware-renderer"),
    ] {
        assert_eq!(reason.to_string(), text);
    }
}

// -- request precedence ------------------------------------------------------------

#[test]
fn the_flag_beats_env_beats_file_beats_default() {
    use RendererRequest::{Auto, Cpu, Gpu};
    // Each source alone.
    assert_eq!(
        resolve_request(Some(Gpu), None, None),
        (Gpu, RequestSource::Flag, None),
    );
    assert_eq!(
        resolve_request(None, Some("gpu"), None),
        (Gpu, RequestSource::Env, None),
    );
    assert_eq!(
        resolve_request(None, None, Some(Gpu)),
        (Gpu, RequestSource::Config, None),
    );
    assert_eq!(
        resolve_request(None, None, None),
        (DEFAULT_REQUEST, RequestSource::Default, None),
    );
    // Pairwise: flag beats env beats file.
    assert_eq!(
        resolve_request(Some(Cpu), Some("gpu"), Some(Gpu)),
        (Cpu, RequestSource::Flag, None),
    );
    assert_eq!(
        resolve_request(None, Some("gpu"), Some(Cpu)),
        (Gpu, RequestSource::Env, None),
    );
    assert_eq!(
        resolve_request(Some(Cpu), Some("gpu"), None),
        (Cpu, RequestSource::Flag, None),
    );
    assert_eq!(
        resolve_request(Some(Auto), None, Some(Gpu)),
        (Auto, RequestSource::Flag, None),
    );
}

#[test]
fn an_explicit_cpu_flag_beats_a_gpu_env_and_file() {
    assert_eq!(
        resolve_request(
            Some(RendererRequest::Cpu),
            Some("gpu"),
            Some(RendererRequest::Gpu),
        ),
        (RendererRequest::Cpu, RequestSource::Flag, None),
    );
}

#[test]
fn an_invalid_env_value_warns_and_falls_through_to_the_file() {
    let (request, source, warning) =
        resolve_request(None, Some("glse"), Some(RendererRequest::Gpu));
    assert_eq!(request, RendererRequest::Gpu);
    assert_eq!(source, RequestSource::Config);
    let warning = warning.expect("a bad env value warns");
    assert!(warning.contains("SCOOT_RENDERER"), "{warning}");
    assert!(warning.contains("glse"), "{warning}");

    // ...and to the default when the file names nothing either.
    let (request, source, warning) = resolve_request(None, Some("glse"), None);
    assert_eq!(request, DEFAULT_REQUEST);
    assert_eq!(source, RequestSource::Default);
    assert!(warning.is_some());

    // The old spellings name the new value in the warning.
    let (_, _, warning) = resolve_request(None, Some("pixman"), None);
    let warning = warning.expect("an old env value warns");
    assert!(warning.contains("cpu"), "{warning}");
    let (_, _, warning) = resolve_request(None, Some("gles"), None);
    let warning = warning.expect("an old env value warns");
    assert!(warning.contains("gpu"), "{warning}");
}

#[test]
fn the_default_request_is_the_cpu_tier() {
    assert_eq!(DEFAULT_REQUEST, RendererRequest::Cpu);
}

#[test]
fn later_heads_follow_the_resolved_tier_without_software_rejection() {
    assert_eq!(plan_for_resolved_tier(RendererKind::Gles), HeadPlan::GPU,);
    assert_eq!(plan_for_resolved_tier(RendererKind::Pixman), HeadPlan::CPU,);
    assert_eq!(
        device_plan_for_resolved_tier(RendererKind::Gles),
        DevicePlan::TryGpu {
            reject_software: false,
        },
    );
    assert_eq!(
        device_plan_for_resolved_tier(RendererKind::Pixman),
        DevicePlan::Pixman(Reason::Requested),
    );
}
