//! The `auto` renderer policy: what the user asked for, and what the
//! session actually composites with.
//!
//! Pure by construction: no Smithay types, no I/O, no allocation in the
//! decision itself. Everything here runs at startup only -- `on_device`
//! once per candidate device, `after_renderer` once per session -- never
//! per frame. The wiring that calls it (`compositor::run`, `tty::init`,
//! `try_scanout`) needs real DRM hardware, so it is covered by the
//! hardware evidence in the PR; the logic halves are pinned here.
//!
//! Vocabulary: the user asks for `cpu`, `gpu` or `auto`
//! ([`RendererRequest`]); the session resolves to a [`RendererKind`]
//! (`cpu` = pixman, `gpu` = GLES). `gpu` means the GLES renderer (scanout
//! on `--tty`).

use crate::cli::{DEFAULT_REQUEST, RendererKind, RendererRequest, RequestSource, renamed_renderer};

/// Which backend this decision is for. Headless and nested have no DRM
/// device, so [`before_device`] resolves them fully; `--tty` needs the
/// device first, so it defers to [`on_device`] and [`after_renderer`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Headless,
    Nested,
    Tty,
}

/// What [`before_device`] answered: either resolved already, or deferred
/// until a DRM device exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Early {
    /// Resolved before any device exists (explicit requests on every
    /// backend, `auto` on headless/nested, `auto` on `--tty` in a build
    /// without the scanout tier).
    Resolved(RendererKind, Reason),
    /// `--tty` in a build with the scanout tier and an `auto` request:
    /// decide per device in [`on_device`].
    DecideOnDevice(RendererRequest),
}

/// What [`on_device`] answered for one candidate device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DevicePlan {
    /// Composite on the CPU; `tty::init` never touches GBM or EGL.
    Pixman(Reason),
    /// Try the GPU scanout tier. `reject_software` is true only for the
    /// first head under an `auto` request: a software `GL_RENDERER` there
    /// falls back to the CPU renderer. Later heads (hotplug, reconnect)
    /// and forced `gpu` requests never reject software -- the tier is
    /// already decided, and a forced request is explicit.
    TryGpu { reject_software: bool },
}

/// A small `Copy` plan per head, so `build_heads`/`build_head`/
/// `try_scanout` take the decision rather than the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HeadPlan {
    /// Whether to attempt the GPU scanout tier at all.
    pub(crate) try_gpu: bool,
    /// Whether a software `GL_RENDERER` refuses the tier (first head
    /// under `auto` only).
    pub(crate) reject_software: bool,
}

impl HeadPlan {
    /// The plan for a head that never tries the GPU tier.
    pub(crate) const CPU: Self = Self {
        try_gpu: false,
        reject_software: false,
    };

    /// The plan for a head that tries the GPU tier without a software
    /// rejection (forced `gpu`, later heads of a `gpu` session).
    pub(crate) const GPU: Self = Self {
        try_gpu: true,
        reject_software: false,
    };
}

impl From<DevicePlan> for HeadPlan {
    fn from(plan: DevicePlan) -> Self {
        match plan {
            DevicePlan::Pixman(_) => Self::CPU,
            DevicePlan::TryGpu { reject_software } => Self {
                try_gpu: true,
                reject_software,
            },
        }
    }
}

/// Why the session composites with the tier it does. A `Copy` enum with a
/// `Display` of `&'static str`; variable detail (the KMS driver name, the
/// GL renderer string) rides as separate log fields, never inside the
/// reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// An explicit `cpu`/`gpu` request that was honoured (or a forced
    /// `gpu` that built on hardware).
    Requested,
    /// `auto` on headless: offscreen GLES reads every frame back, so the
    /// CPU renderer is as fast or faster here.
    HeadlessReadsBack,
    /// `auto` on nested: the GPU hand-off to the host is not yet measured
    /// against the CPU renderer (`renderer-auto-nested`); `--renderer gpu`
    /// tries it.
    NestedUnmeasured,
    /// `auto` on `--tty` in a build without the scanout tier.
    NoScanoutTier,
    /// `auto` on `--tty` where libgbm could not be loaded.
    NoGbm,
    /// `auto` on `--tty` where libEGL could not be loaded.
    NoEgl,
    /// `auto` on `--tty` where the display device has no 3D engine.
    DisplayOnly,
    /// `auto` on `--tty` on a virtual GPU: the CPU renderer is usually
    /// faster there; `--renderer gpu` overrides.
    VirtualGpu,
    /// `auto` (or forced `gpu`) on `--tty` where the GPU tier refused to
    /// build; the existing warning line says what refused.
    GpuBuildFailed,
    /// `auto` on `--tty` where the GPU stack turned out to be a software
    /// rasteriser. Only constructed with the scanout tier (see
    /// `after_renderer`); allowed dead without it, the way `drm_syncobj`
    /// keeps its scanout-only items.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
    SoftwareRenderer,
    /// `auto` on `--tty` where the GPU stack is real hardware.
    #[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
    HardwareRenderer,
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Requested => "requested",
            Self::HeadlessReadsBack => "headless-reads-back",
            Self::NestedUnmeasured => "nested-unmeasured",
            Self::NoScanoutTier => "no-scanout-tier",
            Self::NoGbm => "no-gbm",
            Self::NoEgl => "no-egl",
            Self::DisplayOnly => "display-only",
            Self::VirtualGpu => "virtual-gpu",
            Self::GpuBuildFailed => "gpu-build-failed",
            Self::SoftwareRenderer => "software-renderer",
            Self::HardwareRenderer => "hardware-renderer",
        })
    }
}

/// Which class of KMS device a DRM driver name belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KmsClass {
    /// A display device with no 3D engine: Mesa would fall back to a
    /// software rasteriser, so `auto` stays on the CPU renderer.
    DisplayOnly,
    /// A paravirtual GPU: the CPU renderer is usually faster here (measured
    /// on virtio-gpu without virgl); `auto` stays on it, `--renderer gpu`
    /// overrides.
    VirtualGpu,
    /// Anything else, including every real GPU and anything unrecognised
    /// (an unknown name must never refuse the GPU tier on its own -- the
    /// `GL_RENDERER` check is what proves the stack is real).
    Other,
}

/// The KMS driver class of `driver` (exact match, case-sensitive: kernel
/// driver names are lowercase by convention).
///
/// Every name below was verified against the kernel source's `.name =` in
/// `struct drm_driver` (torvalds/linux, master, 2026-10-11); the comment
/// cites the file and line per name. Two names from the original proposal
/// were dropped as unverifiable rather than guessed: `cirrus`
/// (`drivers/gpu/drm/tiny/cirrus.c` no longer exists -- only
/// `cirrus-qemu.c` does) and `xlnx` (`drivers/gpu/drm/xlnx/` holds only
/// the ZynqMP display driver, whose name is `zynqmp-dpsub`).
/// The Asahi display driver's name (`apple-drm` per sysfs) is deliberately
/// absent: it must classify [`KmsClass::Other`].
pub(crate) fn kms_class(driver: &str) -> KmsClass {
    // Display-only (no 3D engine):
    // - "simpledrm": drivers/gpu/drm/sysfb/simpledrm.c:37
    //   (`#define DRIVER_NAME "simpledrm"`, used at .name, :893)
    // - "efidrm": drivers/gpu/drm/sysfb/efidrm.c:33
    // - "vesadrm": drivers/gpu/drm/sysfb/vesadrm.c:35
    // - "ofdrm": drivers/gpu/drm/sysfb/ofdrm.c:32
    // - "bochs-drm": drivers/gpu/drm/tiny/bochs.c:820 (literal `.name`)
    // - "cirrus-qemu": drivers/gpu/drm/tiny/cirrus-qemu.c:52
    //   (`#define DRIVER_NAME "cirrus-qemu"`, used at .name, :568 and :667)
    // - "qxl": drivers/gpu/drm/qxl/qxl_drv.h:55
    //   (`#define DRIVER_NAME "qxl"`, used at .name, qxl_drv.c:278,309)
    // - "vboxvideo": drivers/gpu/drm/vboxvideo/vbox_drv.h:26
    //   (`#define DRIVER_NAME "vboxvideo"`, used at .name, vbox_drv.c:175,190)
    // - "hyperv_drm": drivers/gpu/drm/hyperv/hyperv_drm_drv.c:65,231
    //   (`.name = KBUILD_MODNAME`, and hyperv/Makefile builds hyperv_drm.o,
    //   so the module -- and driver -- name is "hyperv_drm")
    // - "vkms": drivers/gpu/drm/vkms/vkms_drv.c:36
    //   (`#define DRIVER_NAME "vkms"`, used at .name, :99)
    // - "udl": drivers/gpu/drm/udl/udl_drv.h:27
    //   (`#define DRIVER_NAME "udl"`, used at .name, udl_drv.c:63; udl_drv.c:139
    //   is the USB driver name, the same literal)
    // - "gm12u320": drivers/gpu/drm/tiny/gm12u320.c:36
    //   (`#define DRIVER_NAME "gm12u320"`, used at .name, :611; :736 repeats
    //   the literal for the USB driver)
    // - "ast": drivers/gpu/drm/ast/ast_drv.h:46
    //   (`#define DRIVER_NAME "ast"`, used at .name, ast_drv.c:161,604)
    // - "mgag200": drivers/gpu/drm/mgag200/mgag200_drv.h:26
    //   (`#define DRIVER_NAME "mgag200"`, used at .name, mgag200_drv.c:98,304)
    const DISPLAY_ONLY: &[&str] = &[
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
    ];
    // Virtual GPUs:
    // - "virtio_gpu": drivers/gpu/drm/virtio/virtgpu_drv.h:48
    //   (`#define DRIVER_NAME "virtio_gpu"`, used at .name, virtgpu_drv.c:354)
    // - "vmwgfx": drivers/gpu/drm/vmwgfx/vmwgfx_drv.h:40
    //   (`#define VMWGFX_DRIVER_NAME "vmwgfx"`, used at .name, vmwgfx_drv.c:1603,1611)
    const VIRTUAL_GPU: &[&str] = &["virtio_gpu", "vmwgfx"];
    if DISPLAY_ONLY.contains(&driver) {
        KmsClass::DisplayOnly
    } else if VIRTUAL_GPU.contains(&driver) {
        KmsClass::VirtualGpu
    } else {
        KmsClass::Other
    }
}

/// Whether a `GL_RENDERER` string names a software rasteriser (ASCII
/// case-insensitive substring match). Runs once per session at startup,
/// so the small scan costs nothing.
///
/// Read only by the scanout tier's software check (see `after_renderer`);
/// allowed dead without it, the way `drm_syncobj` keeps its scanout-only
/// items.
#[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
pub(crate) fn is_software_gl_renderer(renderer: &str) -> bool {
    const SOFTWARE: &[&str] = &["llvmpipe", "softpipe", "swrast", "software rasterizer"];
    let lower = renderer.to_ascii_lowercase();
    SOFTWARE.iter().any(|mark| lower.contains(mark))
}

/// Resolve the request from its three sources: flag > `SCOOT_RENDERER` >
/// config file > [`DEFAULT_REQUEST`].
///
/// `env` is the raw `SCOOT_RENDERER` value (`None` when unset), passed in
/// rather than read here so tests never mutate the process environment.
/// Returns the request, where it came from, and -- only for an unparseable
/// env value -- the warning text naming the variable and the value. A bad
/// env value falls through to the file (it never refuses startup, because
/// `--tty` must never be locked out); a bad flag value stays a refusal at
/// the parser, so it cannot reach here.
pub(crate) fn resolve_request(
    flag: Option<RendererRequest>,
    env: Option<&str>,
    file: Option<RendererRequest>,
) -> (RendererRequest, RequestSource, Option<String>) {
    if let Some(request) = flag {
        return (request, RequestSource::Flag, None);
    }
    if let Some(value) = env {
        if let Some(request) = RendererRequest::parse(value) {
            return (request, RequestSource::Env, None);
        }
        let warning = match renamed_renderer(value) {
            Some(new) => format!(
                "ignoring SCOOT_RENDERER={value}: {value} was renamed to {new}; \
                 using the config file or the default instead"
            ),
            None => format!(
                "ignoring SCOOT_RENDERER={value}: expected cpu, gpu or auto; \
                 using the config file or the default instead"
            ),
        };
        let (request, source) = match file {
            Some(request) => (request, RequestSource::Config),
            None => (DEFAULT_REQUEST, RequestSource::Default),
        };
        return (request, source, Some(warning));
    }
    match file {
        Some(request) => (request, RequestSource::Config, None),
        None => (DEFAULT_REQUEST, RequestSource::Default, None),
    }
}

/// Decide before any device exists (in `compositor::run`).
///
/// Explicit requests resolve immediately on every backend. `auto`
/// resolves on headless/nested (there is no device to wait for); on
/// `--tty` it defers to [`on_device`], unless the build has no scanout
/// tier at all.
pub(crate) fn before_device(
    request: RendererRequest,
    session: Session,
    build_has_tier: bool,
) -> Early {
    match (request, session) {
        (RendererRequest::Cpu, _) => Early::Resolved(RendererKind::Pixman, Reason::Requested),
        (RendererRequest::Gpu, _) => Early::Resolved(RendererKind::Gles, Reason::Requested),
        (RendererRequest::Auto, Session::Headless) => {
            Early::Resolved(RendererKind::Pixman, Reason::HeadlessReadsBack)
        }
        (RendererRequest::Auto, Session::Nested) => {
            Early::Resolved(RendererKind::Pixman, Reason::NestedUnmeasured)
        }
        (RendererRequest::Auto, Session::Tty) if !build_has_tier => {
            Early::Resolved(RendererKind::Pixman, Reason::NoScanoutTier)
        }
        (RendererRequest::Auto, Session::Tty) => Early::DecideOnDevice(request),
    }
}

/// Decide per DRM device, in `tty::open_device` after `DrmDevice::new`.
/// `kms_driver` is the `DRM_IOCTL_VERSION` name (or `""` when the ioctl
/// failed, which classifies [`KmsClass::Other`]). `gbm_loadable` is
/// whether libgbm can be loaded; `egl_loadable` is whether libEGL can
/// (`gles::lib_loadable`). First match wins.
pub(crate) fn on_device(
    request: RendererRequest,
    kms_driver: &str,
    gbm_loadable: bool,
    egl_loadable: bool,
) -> DevicePlan {
    match request {
        RendererRequest::Cpu => DevicePlan::Pixman(Reason::Requested),
        RendererRequest::Gpu => DevicePlan::TryGpu {
            reject_software: false,
        },
        RendererRequest::Auto => {
            if !gbm_loadable {
                DevicePlan::Pixman(Reason::NoGbm)
            } else if !egl_loadable {
                DevicePlan::Pixman(Reason::NoEgl)
            } else {
                match kms_class(kms_driver) {
                    KmsClass::DisplayOnly => DevicePlan::Pixman(Reason::DisplayOnly),
                    KmsClass::VirtualGpu => DevicePlan::Pixman(Reason::VirtualGpu),
                    KmsClass::Other => DevicePlan::TryGpu {
                        reject_software: true,
                    },
                }
            }
        }
    }
}

/// Decide after the scanout GLES renderer is built, before its surface is
/// consumed: `Ok` keeps the GPU tier, `Err` falls back to the CPU tier
/// (reusing the same surface). `reject_software` is the trying plan's
/// flag; `gl_renderer` is the `GL_RENDERER` string (`None` when it could
/// not be read, which never refuses -- an unreadable string is not proof
/// of software).
///
/// Read only by the scanout tier (see `try_scanout`); allowed dead without
/// it, the way `drm_syncobj` keeps its scanout-only items.
#[cfg_attr(not(feature = "gpu-scanout"), allow(dead_code))]
pub(crate) fn after_renderer(
    reject_software: bool,
    gl_renderer: Option<&str>,
) -> Result<Reason, Reason> {
    if !reject_software {
        return Ok(Reason::Requested);
    }
    match gl_renderer {
        Some(name) if is_software_gl_renderer(name) => Err(Reason::SoftwareRenderer),
        _ => Ok(Reason::HardwareRenderer),
    }
}

/// Map a session's resolved tier to the trying plan for a rebuild that must
/// keep it (seat reconnect's restore path): the CPU renderer plans dumb
/// buffers without touching GBM or EGL; the scanout tier tries with no
/// software rejection, since the tier is already decided.
pub(crate) fn device_plan_for_resolved_tier(tier: RendererKind) -> DevicePlan {
    match tier {
        RendererKind::Pixman => DevicePlan::Pixman(Reason::Requested),
        RendererKind::Gles => DevicePlan::TryGpu {
            reject_software: false,
        },
    }
}

/// Map a session's resolved tier to the plan for a later head (hotplug, seat
/// reconnect): same device, same renderer string, so no software rejection
/// -- the tier is already decided, and the one-tier-per-session rule stays.
pub(crate) fn plan_for_resolved_tier(tier: RendererKind) -> HeadPlan {
    match tier {
        RendererKind::Pixman => HeadPlan::CPU,
        RendererKind::Gles => HeadPlan::GPU,
    }
}

#[cfg(test)]
#[path = "policy/tests.rs"]
mod tests;
