---
title: "auto renderer request: gpu on real hardware, cpu in VMs and headless (pure policy, default unchanged)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-10"
---

# auto renderer request: gpu on real hardware, cpu in VMs and headless (pure policy, default unchanged)

Filed 2026-10-10. Serves **daily-drive**: one build that picks the best
tier for the user's case — gpu where a real accelerated stack exists, cpu
everywhere else — with no flag to remember, and the user can always
override either way. (It also serves computer use indirectly —
webtop/nested sessions keep cpu — but the case for it is a laptop that
just does the right thing.)

## Vocabulary

User-facing renderer names are `cpu | gpu | auto` (flag `--renderer`,
`SCOOT_RENDERER`, `[renderer] backend`) — breaking, no aliases. This
ticket uses the new names throughout: `cpu` is the pixman tier, `gpu` is
the GLES/scanout tier. `pixman`/`gles` below name only the internal tier
(`RendererKind::Pixman/Gles`, the `PixmanRenderer`/GLES pipeline) or the
current code's spellings, which still read `pixman|gles` until the rename
in this PR lands. The rename itself is this PR's work, in flight by
another agent — do not touch that code or PR from any other ticket.

## The gap

There is no `auto`. `--renderer` takes only `pixman|gles` today
(`crates/scoot/src/cli.rs:30`, `cli.rs:821`), `[renderer] backend` takes
only `"pixman"`/`"gles"`, `RendererKind` is the resolved tier with no
request state, and there is no `SCOOT_RENDERER` env var. So a user who
moves between real hardware (where GPU scanout costs 4.2–5.1x less
compositor CPU, `docs/backlog/resolved/gpu-vs-cpu-measured-done.md`) and a
VM (where GPU scanout is ~1.5x dearer than dumb buffers, same record) must
hand-pass the right flag per machine. Worse, a wrong `gpu` on a
GPU-less box is a startup error or a silent llvmpipe session, and a wrong
`cpu` on real hardware leaves the measured win on the table.

A second, pre-existing bug rides along: `reload.rs:715` compares the
file's `renderer` against the *resolved* `state.renderer`. A `--tty`
session whose file says `gpu` but which fell back to cpu
(`tty::init` rewrites `state.renderer`), or a `--renderer cpu` run
whose file says `gpu`, refuses `renderer.backend` on every reload of an
unchanged file. The diff must be request vs request.

## What to do

**Default stays cpu.** This ticket adds the `auto` request only;
`DEFAULT_REQUEST = Cpu`, and the flip to `Auto` is stage C's
deliberate doctrine change with user sign-off. `--renderer auto`,
`SCOOT_RENDERER=auto`, or `[renderer] backend = "auto"` gives gpu on
real hardware and cpu in VMs — in a `gpu-scanout` build today.

**Types.** `RendererKind { Pixman, Gles }` stays exactly as is: the
*resolved* tier read by `Backend::renderer()` (`render.rs:462`),
`State::renderer`, `Tty::renderer()` (`tty/mod.rs:1356`),
`hotplug.rs:651`, `reconnect.rs:129/178`. New `RendererRequest { Auto,
Cpu, Gpu }` in `cli.rs` beside it, with one `parse` and one `as_str`
shared by flag, env var and config file, plus `pub const DEFAULT_REQUEST:
RendererRequest = Cpu` (every caller that defaults uses it) and
`RequestSource { Flag, Env, Config, Default }` for the log line.

**Precedence (one pure function, tested exhaustively):** flag >
`SCOOT_RENDERER` > `[renderer] backend` > `DEFAULT_REQUEST`. The env var
is read once in `compositor::run` and passed in as `Option<&OsStr>`
(tests never mutate the process environment). An unparseable env value
warns naming variable and value, then is ignored — it never refuses
startup, because `--tty` must never be locked out. A bad flag value stays
a refusal (`cli.rs:816-823`). The env var is inherited by children
including a nested `scoot`: document that, don't strip it.

**Policy (`crates/scoot/src/compositor/render/policy.rs`, tests in
`render/policy/tests.rs`).** Pure: no Smithay types, no I/O, no
allocation in the decision itself.

```rust
pub(crate) enum Session { Headless, Nested, Tty }
pub(crate) fn before_device(req: RendererRequest, session: Session, build_has_tier: bool) -> Early;
pub(crate) fn on_device(req: RendererRequest, kms_driver: &str, gbm_loadable: bool, egl_loadable: bool) -> DevicePlan;
pub(crate) fn after_renderer(plan_reject_software: bool, gl_renderer: &str) -> Result<Reason, Reason>;
pub(crate) fn kms_class(driver: &str) -> KmsClass;     // DisplayOnly | VirtualGpu | Other
pub(crate) fn is_software_gl_renderer(s: &str) -> bool; // ASCII case-insensitive substring
```

`Reason` is a `Copy` enum with a `Display` of `&'static str`; driver name
and GL renderer string ride as separate log fields. `on_device` runs once
per candidate device at startup, `after_renderer` once per session —
nothing here runs per frame.

**Decision table (first match wins).** `cpu` → cpu (`requested`).
Headless + `auto` → cpu, always (`headless-reads-back`: offscreen GLES
reads every frame back; measured llvmpipe 18–31x slower, M2 parity).
Nested + `auto` → cpu for now (`nested-unmeasured`; the rematch is
`renderer-auto-nested`). Headless/nested + `gpu` → gpu or the existing
startup error naming each EGL failure (`gles.rs:194-281`, unchanged).
TTY + `auto`: no scanout tier in the build → cpu (`no-scanout-tier`);
GBM not loadable → cpu (`no-gbm`); `libEGL.so.1` not loadable via
`gles::lib_loadable` (`gles.rs:352`) → cpu (`no-egl`); display-only
driver → cpu (`display-only`); virtual GPU → cpu (`virtual-gpu`,
override with `--renderer gpu`); otherwise try GPU with
`reject_software: true`. While trying: a `GbmDevice`/`ScanoutBackend`/
`ScanoutPresenter` refusal (existing warns, `tty/mod.rs:987-1047`) →
cpu (`gpu-build-failed` + existing warn line); software `GL_RENDERER`
→ cpu (`software-renderer`); hardware renderer → gpu scanout
(`hardware-renderer`, measured 4.2–5.1x less CPU on the M2; Intel/AMD
unmeasured — record the row as unmeasured alongside the choice, not as a
default claim).
TTY + `gpu` behaves as today: try, never reject software, loud `warn!`
+ dumb buffers on refusal (lockout rule, `tty/mod.rs:958-972`).

**Signals (all startup-only).** KMS driver name from DRM_IOCTL_VERSION
(`drm_fd.get_driver()?.name()`; on error use `""` → `Other`).
`DisplayOnly = ["simpledrm", "efidrm", "vesadrm", "ofdrm", "bochs-drm",
"cirrus-qemu", "cirrus", "qxl", "vboxvideo", "hyperv_drm", "vkms", "udl",
"gm12u320", "ast", "mgag200", "xlnx"]`, `VirtualGpu = ["virtio_gpu",
"vmwgfx"]` — **every one of these names is UNVERIFIED**: the implementer
verifies each against the kernel's `drivers/gpu/drm/**` `.name =` in
`struct drm_driver` and against `drm_info` on the dev VM (expect
`virtio_gpu`), cites the source line per name in a comment, and drops any
name that cannot be verified rather than guessing. The Asahi display
driver's DRM name is UNVERIFIED (sysfs says `apple-drm`); it must classify
`Other`, and the list must never contain it. GBM loadable: in a
`runtime-gbm` build, `crate::gbm_stub::real_loaded()` (make it
non-`#[cfg(test)]` *only if* PR #547 is on main when this starts;
otherwise hard-code the plain-build case and stage A wires the probe); in
a plain `gpu-scanout` build, `true`. `GL_RENDERER` via a new
`ScanoutBackend::gl_renderer(&mut self) -> Option<String>` using
`with_context` (pinned fork `gles/mod.rs:1958`), read in `try_scanout`
right after `ScanoutBackend::new` succeeds and before
`ScanoutPresenter::new` consumes the surface — a software verdict returns
`Err(Some(Box::new(surface)))` so the dumb tier reuses the same surface,
and dropping `backend` frees the EGL context. Do **not** use
`EGLDevice::is_software()` (`gles.rs:34-41`: a virtio node served by
kms_swrast answers `false`). Software substrings, case-insensitive:
`llvmpipe`, `softpipe`, `swrast`, `software rasterizer`; fixtures include
a zink-on-llvmpipe string (software) and a `virgl (...)` string
(hardware).

**Multi-head / multi-GPU.** `on_device` runs per candidate device in
`open_device`; `after_renderer` runs for the **first** head only, later
heads (incl. `hotplug.rs:651`, `reconnect.rs:592`) get `TryGpu {
reject_software: false }` if the session is Gles else Pixman — same
device, same renderer string, and the one-tier-per-session rule
(`tty/mod.rs:651-659`, `916-922`) stays. Device *choice* is unchanged
(`gpu::first_usable`); the policy never moves devices in search of a GPU.
`--gpu PATH` / `[tty] gpu` names the *device* only; the policy runs on it
as usual.

**Logging.** Exactly one INFO line per session: `tracing::info!(requested
= %req, source = %src, tier = %kind, reason = %reason, driver = %drv,
gl_renderer = %glr, "renderer chosen")`, with `tier` spelled `cpu|gpu`
and absent fields as `-` (headless/nested in `compositor::run`; tty in `tty::init` after
`state.renderer = renderer`, `tty/mod.rs:349`). Candidate attempts that
fall through log at `debug!`. The per-connector `drm: driving this device
… scanout=…` line stays. `auto` never fails — every no-GPU outcome is
cpu plus a reason. Under `auto`, `gpu-build-failed` warns downgrade to
`info!` (auto expected the possibility); under `gpu` they stay `warn!`.
Forced `cpu` never touches EGL or GBM (assert via the plan value:
`on_device` is not consulted when the request is Cpu).

**File-by-file.** `cli.rs`: request/source/default types,
`CompositorOptions::renderer: Option<RendererRequest>` (`cli.rs:562`),
`fn renderer` parses `auto|cpu|gpu` with error text naming all three,
usage line `cli.rs:30` and every `FlagDoc takes: "pixman|gles"` (~200,
~235, ~270) become `auto|cpu|gpu`, default text `"cpu (or
$SCOOT_RENDERER, or the config file)"`, new `ENVIRONMENT` row
`("SCOOT_RENDERER", "renderer request when --renderer is absent: auto,
cpu or gpu; beats [renderer] backend")` (`cli.rs:146`); extend the
usage/round-trip drift tests, keep the `RendererKind::default() ==
Pixman` pin. `config.rs`: `into_kind` → `into_request`, template comment
(~971) mentions `auto`/env/precedence, module-doc paragraph (41-53) gains
"`auto` never fails startup"; round-trip + unknown-name-warns tests.
`render.rs`: `mod policy;`, `resolve`/`resolve_with` (`render.rs:143-180`)
become thin wrappers over `before_device` + precedence, keeping the
no-scanout-tier warning wording (tests at `render/tests.rs:254-310`
pin it). `mod.rs:164`: read `SCOOT_RENDERER` once, resolve
request+source, `before_device`; headless/nested set `state.renderer`
from `Early::Resolved` + INFO line; tty passes the request into
`tty::init`; store `state.startup_renderer_request = loaded.renderer`
(same idea as `startup_gpu`, line 190). `state.rs`: new `pub(super)
startup_renderer_request: Option<RendererRequest>`, `None` in
`State::new`; `State::renderer`'s doc (477-485) stays the resolved tier.
`reload.rs:715`: compare `fresh.renderer !=
self.startup_renderer_request` (request vs request), plus the three
tests (file `auto` + resolved gpu unchanged → no refusal; file `gpu` +
tty fallback unchanged → no refusal; `cpu` → `auto` → refused "takes
effect on restart"). `tty/mod.rs`: `init` takes the request,
`open_device` (660) calls `on_device` after `DrmDevice::new` (738);
`build_heads`/`build_head`/`try_scanout` take a `DevicePlan` (or small
`Copy` `HeadPlan { try_gpu, reject_software }`) instead of `wanted:
RendererKind`; `try_scanout` (975) does the `GL_RENDERER` check after
`ScanoutBackend::new` (998); `Device` gains final `Reason` + two detail
strings; update `init` (290-312) and `open_device` (631-659) docs.
`hotplug.rs:651`, `reconnect.rs:129,592`: map resolved `RendererKind` to
the plan (Gles → try with `reject_software: false`, Pixman → cpu), no
behavior change. `scanout.rs`: `gl_renderer`. `gbm_stub.rs`: unwire the
`#[cfg(test)]` only if #547 has merged. `scripts/tty-tier-bench.sh`:
round A currently relies on the default while round B passes
`--renderer gles` explicitly (line ~396), so give both rounds explicit
flags (`--renderer cpu` / `--renderer gpu`) via new `TIER_A_ARGS` /
`TIER_B_ARGS` overrides (same defaults plus those flags), and update the
header comment to the new spellings. Docs (docs-bar standard): `backends.md` "Which
renderer draws the frames" gains `auto`, the per-backend behavior, the
user-worded table, the INFO line, `SCOOT_RENDERER`/precedence, and two
`> **Symptom:**` boxes (cpu-on-GPU-laptop → read `reason=`; cpu-in-VM
→ intended, `--renderer gpu` overrides) — "the default is cpu" stays
true; `configure.md` `[renderer] backend` accepts `"auto"` (type, default
`"cpu"`, restart-only, example, precedence); `install.md`
`#which-build-do-i-need` gains one sentence (`scoot-gpu` + `--renderer
auto` picks gpu on hardware, cpu in VMs; CPU/GPU axis stays);
`reference/cli.md` gains the row if hand-rendered. README: no change.

**Tests (all headless, nextest).** `policy/tests.rs`: one test per table
row over `Session × RendererRequest × build_has_tier` for
`before_device`; driver × gbm × egl for `on_device`; forced cpu never
`TryGpu`; forced gpu on `virtio_gpu` → `TryGpu { reject_software: false
}`; every listed driver name classifies, and `apple`, `apple-drm`,
`asahi`, `i915`, `xe`, `amdgpu`, `nouveau`, `nvidia-drm`, `""` classify
`Other`; software/hardware renderer-string fixtures incl. `virgl (...)`
and zink-on-llvmpipe; `Reason` `Display` pinned where it names an
override. Precedence: all 4 sources pairwise, explicit `--renderer
cpu` beats `SCOOT_RENDERER=gpu` + `backend = "gpu"`, invalid env
falls through with the warning text, `DEFAULT_REQUEST == Cpu` pinned.
The `cli`/`config`/`reload` tests above. Headless end-to-end: a `State`
started with `Auto` on headless ends `Backend::renderer() == Pixman` (via
the existing `test_support` session builders). No test for the
`on_device`/`after_renderer` *wiring* — it needs real DRM; the hardware
evidence below covers it, the pure halves carry the logic.

**Gates and evidence (exact SHA, raw output).** Dev VM, tar-shipped copy,
own `CARGO_TARGET_DIR`: `cargo nextest run -p scoot -p scoot-core -p
scoot-ipc -p scootctl`, `cargo nextest run -p scoot --features
gpu-scanout`, both clippy lines (plain + `gpu-scanout`), `cargo fmt
--check -p scoot`, `scripts/smoke-test.sh`, `RENDERER=auto
scripts/smoke-test.sh` (the script already passes `RENDERER` through as
`--renderer`; update only its comment to the new spellings),
`scripts/backlog check`; macOS `cargo check -p scoot` + clippy. Dev VM
`--tty`: the INFO line for `--renderer auto` (expect
`tier=cpu reason=virtual-gpu driver=virtio_gpu`),
`SCOOT_RENDERER=gpu` (expect forced gpu, `scanout="gpu"`), and
`SCOOT_RENDERER=gpu --renderer cpu` (expect cpu, source flag).
M2 `--tty`: `--renderer auto` INFO line + `scanout="gpu"` on eDP-1, `scoot
msg outputs` (`live: true`), one screenshot path; `--renderer cpu`
(source flag); `[renderer] backend = "auto"` with no flag (source
config). Startup cost: process start → `scoot is up`, 5 runs each,
`cpu` vs `auto`, dev VM and M2, raw times (on a software path `auto`
pays one GLES context build+teardown — record it). Hot path: `git diff
origin/main -- crates/scoot/src/compositor/render.rs` shows no change in
`draw_frame`/`draw_frame_with`, presenters untouched — state it with the
stat.

## Not in this ticket

The default stays `cpu` (the flip is stage C, needs user sign-off on
the CLAUDE.md amendment). No runtime swap (see `runtime-renderer-swap`).
No flipping the virtual-GPU or nested rows to the gpu tier (see the two
`renderer-auto-*` research tickets). No GPU hotplug (device fixed at
startup; out of scope, say so in the docs). No render-node probing, no
`EGLDevice::is_software()`, no Smithay fork changes, no new dependency.
`scripts/asahi-test4.sh` needs no change.

## Resolution (2026-10-10)

Landed in #552 (`feat(scoot)!:`): `--renderer cpu|gpu|auto`, `SCOOT_RENDERER`, `[renderer] backend`, the pure policy in `render/policy.rs`, the reload request-vs-request fix, and the breaking rename with no aliases (old `pixman`/`gles` values are refused with a message naming the new one). Default stays `cpu`. Independent review found three missed rename leftovers, fixed in the same PR. Unverified: the dev VM `virtual-gpu` auto line, and `~/nixos-config` references to the old values.
