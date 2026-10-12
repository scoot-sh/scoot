---
title: "Research: HDR and color management (wp-color-management-v1, DRM HDR metadata)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-11"
---

# Research: HDR and color management (wp-color-management-v1, DRM HDR metadata)

> Resolved 2026-10-11: research pass complete, no code. Findings below
> (protocol state, client matrix, CPU-tier refuse policy, hardware
> requirements, staged plan with per-stage fork separation). Four
> follow-up stage tickets filed: `hdr-probe`, `hdr-10bit-scanout`,
> `hdr-color-protocol`, `hdr-output-signaling`.

Filed 2026-10-11. Serves **daily-drive**: HDR monitors and HDR video are common on desktops now; not needed for computer use.

## The gap

scoot has no HDR and no color management (checked 2026-10-11): no
`wp_color_management_v1` in `crates/scoot/src`, none listed on the protocols
page, and `gpu-vrr` records it as "a different order of work, not filed". The
pinned Smithay fork (`fdf424d6`) has no color-management protocol module and
no DRM `HDR_OUTPUT_METADATA` / `Colorspace` connector-property helpers (grep of
`src/` finds neither). The CPU tier composites 8-bit sRGB in pixman.

## What to do

A research pass, no code:

- Which protocol: `wp-color-management-v1` is the standard, so per CLAUDE.md
  it is the target. Check its state and which clients use it (mpv, Firefox,
  KWin and Mutter as references).
- What it needs: HDR metadata and colorspace on the DRM connector, a 10-bit or
  FP16 scanout format, and a GPU-tier color pipeline (transfer function and
  gamut conversion). Which of these scoot can do scoot-side without a
  Smithay fork change (forks are the last resort; see `dev/forks.md`).
- The CPU tier: say plainly that HDR is a GPU-tier feature, and what the CPU
  tier does with an HDR client (tone-map to SDR, or refuse). CPU and GPU stay
  equal tiers for what both can do.
- Hardware: which monitor and GPU could test it (the Asahi M2's panel and
  external displays; the VMs cannot).
- Output: a scoped plan with stages, or a decision not to do it yet.

## Not in this ticket

Implementation, and `wp_tearing_control_v1` (see `gpu-vrr`).

## Findings (research pass 2026-10-12, no code)

### Protocol: `wp-color-management-v1` is the target, still staging

- The vendored protocol source already carries it: `wayland-protocols`
  0.32.13 (pinned in `Cargo.lock`) ships
  `protocols/staging/color-management/color-management-v1.xml` with every
  interface at version 3 (`wp_color_manager_v1` v3 and friends), plus the
  companion `protocols/staging/color-representation/color-representation-v1.xml`
  for YUV-family formats. The generated `wp::color_management` Rust module
  exists in the registry source (`src/wp.rs:20`). So the XML and the
  generated server types are on hand; only the compositor-side dispatch is
  missing.
- The pinned Smithay fork (`fdf424d6`) has no server module for it: no
  color entry under `src/wayland/`, and a grep of `src/` for
  `color_management`, `color-management` and `wp_color` finds nothing
  (checked 2026-10-12 against
  `~/.cargo/git/checkouts/smithay-*/fdf424d`). The only color-protocol
  consumer in scoot is a comment: `content_type.rs` notes the content-type
  hint *could* serve HDR handling on a future GPU tier, and nothing reads it.
- Smithay upstream has a draft-only implementation: the protocol tracking
  issue (Smithay/smithay#781) lists `color-management-v1` as WIP via PR
  #1143 ("color management/representation protocols", draft, opened Sep
  2023) and `color-representation-v1` as blank. The maintainer's Aug 2025
  comment on #1143 says the PR is the bare minimum to expose the protocol
  and that the render-pipeline changes HDR actually needs "haven't been
  started at all", untested (checked 2026-10-12). There is no Smithay-side
  implementation to pick up at the next repin.

### What HDR needs, against what scoot has

| Requirement | scoot / pinned-Smithay state (checked 2026-10-12) |
| --- | --- |
| HDR metadata + colorspace on the DRM connector | Kernel side exists: `drm_connector_attach_hdr_output_metadata_property` (`HDR_OUTPUT_METADATA` blob), `drm_connector_attach_colorspace_property` (`Colorspace` enum) with `drm_mode_create_hdmi/dp_colorspace_property`, and `drm_connector_attach_max_bpc_property` (`max bpc` range) — all in the kernel KMS docs (docs.kernel.org GPU DRM KMS page). Smithay's `DrmCompositor` exposes no write API for any of them: its atomic commit path writes only `CRTC_ID` for connectors (`surface/atomic.rs:1176-1182`). Which drivers attach them was NOT verified from here (see unverified list). |
| 10-bit or FP16 scanout format | Import is not the blocker. Smithay's GLES3 memory import already takes `Abgr/Xbgr2101010` and `Abgr/Xbgr16161616f` (`renderer/gles/mod.rs:1016-1019`), and pixman lists the 2101010 shm formats (`renderer/pixman/mod.rs:47-68`). scoot's own swapchain is 8-bit only: `COLOR_FORMATS = [Argb8888, Xrgb8888]` in `crates/scoot/src/compositor/tty/scanout.rs:439-443`. Widening it is a scoot-side change through the already-public `DrmCompositor::with_format` / `set_format` (`drm/compositor/mod.rs:1310,2786`). |
| GPU-tier color pipeline (EOTF + gamut conversion) | Nothing exists in the pinned Smithay GLES renderer (no color hooks; grep clean). Route is scoot-side custom render elements doing PQ EOTF + gamut mapping in shader, bounded to HDR outputs — a fork addition is only justified if implementation hits private-renderer-state it cannot reach, to be evidenced then. |
| Protocol dispatch (`wp_color_manager_v1` + `color-representation-v1`) | Missing. Route is a hand-written scoot-side dispatch on the generated `wayland-protocols` server types (same pattern `gpu-vrr` already records for `wp_tearing_control_v1`: hand-write, since Smithay carries nothing). |

### Who uses it (references for the implementation stages)

Checked 2026-10-12. Full URLs verified directly are cited inline; the
rest came via the ArchWiki HDR page (last edited 2026-09-11) because
freedesktop GitLab was bot-blocked and web search was down that day — see
the unverified list for what still needs a direct check.

- **KWin / Plasma 6: shipped.** Static HDR metadata signaling with PQ EOTF
  and the display's preferred brightness, colorspace rec.2020, plus SDR
  brightness / SDR color-intensity controls
  (Xaver Hugl's blog, 2023-12-18, fetched directly). KWin fills the
  mastering metadata fields that Mutter leaves zeroed (ArchWiki).
- **Mutter / GNOME 48+: shipped, per-monitor toggle** in display settings
  (ArchWiki). Known gaps, cited via ArchWiki only: no scRGB /
  `frog-color-management-v1` for nested gamescope (issue 4083), and zeroed
  mastering primaries under `bt2100` (issue 4937, MR 5199).
- **Hyprland >= 0.55: shipped.** Auto-HDR on capable monitor + fullscreen
  content; desktop HDR needs `cm = "hdr"` + `bitdepth = 10` (ArchWiki;
  Hyprland wiki linked therein, not fetched directly).
- **sway / wlroots: experimental.** `render_bit_depth 10`, per-output `hdr`
  toggle, `WLR_RENDERER=vulkan` (ArchWiki). wlroots HDR10 tracking issue
  #3941 state not verified from here.
- **niri: no HDR.** Its `Configuration: Outputs` wiki (edited 2026-05-29,
  fetched 2026-10-12) has no HDR section at all; the only adjacent knob is
  `max-bpc`, marked "since next release". niri is Smithay-based like scoot,
  so there is no sibling implementation to borrow.
- **gamescope: experimental** (`--hdr-enabled`; needs scRGB /
  `xx-color-management` or `frog-color-management-v1` in the host
  compositor, which is why it stays washed out under GNOME) (ArchWiki).
- **mpv: ready.** `gpu-next` with `waylandvk` (default on Wayland), or
  `--vo=dmabuf-wayland`; needs a Vulkan WSI with HDR (mesa 25.1+ /
  nvidia-open 595.58.03+ include it, older NVIDIA needs the
  `vk-hdr-layer`) (ArchWiki).
- **Firefox: experimental since 138** behind `gfx.color_management.hdr`;
  stable still in progress (ArchWiki; bugzilla meta not opened directly).
- **Chromium: merged as of 141.0.7370.0** (ArchWiki; review link not
  opened directly).
- **X.org: no plans** (ArchWiki, via xserver issue 1037; not opened
  directly).

### The CPU tier: refuse, do not tone-map

The CPU tier composites 8-bit sRGB: its framebuffer is `Argb8888`
(`compositor/render/pixman.rs:24-27`, asserted `A8R8G8B8` at :82-98).
With `wp_color_manager_v1` simply not advertised on pixman backends, an
HDR-capable client takes its existing SDR fallback path — no crash, no
garbage, no work. Tone-mapping HDR down to SDR on the CPU is rejected:
it would run a full-frame float conversion per frame on the hottest path,
pixman has no EOTF/gamut ops for it, and it would add a CPU tone-mapping
dependency to serve sinks (webtop, no-GPU boxes) that have no HDR display
— against the speed-first engineering order and the point of the tier.
CPU and GPU stay equal tiers for everything both can do (all of SDR);
HDR is GPU-tier-only by hardware necessity (10-bit/FP16 scanout plus
connector metadata the dumb path has no sink for), the same shape as the
existing explicit-sync split (offered only on the `--tty` GPU scanout
tier). No ticket: the policy is recorded here.

### Hardware: what could test it

- Needed per output: an HDR10-capable sink (HDR Static Metadata block in
  the EDID — `edid-decode` vs `di-edid-decode` per the ArchWiki
  libdisplay-info 0.4.0 note), a driver that attaches
  `HDR_OUTPUT_METADATA` (check live with
  `modetest -c | grep -A6 HDR_OUTPUT_METADATA` while HDR is on), plus the
  `Colorspace` and `max bpc` properties and a 10-bit mode.
- The Asahi M2 Air (`Asahi.md`): `apple-drm` display controller, `eDP-1`
  internal panel, external displays already exercised on USB-C (`DP-1`).
  The internal panel's HDR capability is **not recorded in `Asahi.md`
  and was not verified from here** — check `drm_info` / EDID on the
  hardware before assuming the panel can test anything; the assumed sink
  is an external HDR display on USB-C.
- The VMs cannot test HDR end to end: no HDR sink, and no metadata
  plumbing (driver-level enumeration not verified from here). No machine
  was contacted for this research.

### Staged plan (recommended: proceed in stages, GPU-tier only)

Four small tickets, each stating what it serves (all daily-drive; HDR is
not needed for computer use). Negative-path-first wherever the hardware
is missing, after the `gpu-vrr` pattern.

| Stage (ticket) | scoot-side vs fork change | Alternatives considered |
| --- | --- | --- |
| 1. `hdr-probe`: read-only HDR-capability probe per connector (metadata / Colorspace / max bpc presence), reported in logs, HDR use refused with the reason named | scoot-side, no fork. Read-only DRM property/blob reads never enter Smithay's commit path, so there is no race with `DrmCompositor`; the only per-connector query precedent (`vrr_supported`) lives in Smithay only because VRR also needs commit-time control, which a probe does not. | A Smithay `DrmCompositor` query API: nothing to wrap for a read-only probe; rejected as fork surface with no function. |
| 2. `hdr-10bit-scanout`: opt-in 10-bit swapchain format (`[output]` key, default stays 8-bit), SDR content unchanged | scoot-side, no fork. `with_format` / `set_format` are already public (`drm/compositor/mod.rs:1310,2786`); the change is `scanout.rs:443` plus config plumbing. | None needed: the seam is public. |
| 3. `hdr-color-protocol`: server-side `wp-color-management-v1` (+ `color-representation-v1`) dispatch on the generated `wayland-protocols` types: accept surface image descriptions, report output descriptions from EDID | scoot-side, no fork. Smithay has no module to wrap (pinned rev verified; upstream only the #1143 draft), and there is no in-tree copy target — the same hand-written pattern `gpu-vrr` records for tearing-control. Waiting on upstream Smithay is the alternative and means waiting on a third party with an untested draft. | In-tree copy of a Smithay module: nothing exists to copy. Upstream-first: rejected as unbounded delay; re-check #1143 at implementation time and repin if it ever lands (`smithay-fork-repin`). |
| 4. `hdr-output-signaling`: write `HDR_OUTPUT_METADATA` / `Colorspace` / `max bpc` on the connector, PQ EOTF output, SDR-brightness handling | **Needs a Smithay fork change** (last resort, with `dev/forks.md` entry when filed). Smithay's atomic commit writes only `CRTC_ID` for connectors (`surface/atomic.rs:1176-1182`); no public connector-property write API exists. | Raw atomic commit from scoot on the same device: races Smithay's commits, which own DRM-master serialization — rejected. In-tree `DrmCompositor` copy (~4400 lines + plane assignment): rejected by size, same reasoning as the cursor-dmabuf entry. Follow the `use_vrr` precedent: small fork setters on `DrmCompositor`. |

### Claims not verified from here (web search down, GitLab bot-blocked 2026-10-12)

1. Current upstream state of `wp-color-management-v1` in
   wayland-protocols main beyond the vendored 0.32.13 XML.
2. wlroots HDR10 issue #3941 current state (cited via ArchWiki "See
   also" only).
3. Mutter 4083 / 4937 / MR 5199, Firefox bugzilla HDR meta + 1642854,
   Chromium review thread, X.org xserver#1037, COSMIC promise, Hyprland
   wiki details, Intel HDR incompleteness — all cited via ArchWiki only,
   none opened directly.
4. Which DRM drivers attach `HDR_OUTPUT_METADATA` / `Colorspace` /
   `max bpc` — needs `modetest`/`drm_info` on real hardware.
5. Whether the Asahi M2 internal panel has any HDR mode.
