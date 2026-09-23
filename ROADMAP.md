# scoot roadmap

The ordered list of milestones, worked through with the cycle in
`CLAUDE.md`. This file is the **index**: the live ordered work and a status
table. The detail lives next to it, one file per item, so neither has to be
scrolled through to find the other:

- [`docs/roadmap/`](docs/roadmap/) — the 18 shipped milestones (plus 5a),
  one file each, verbatim review findings and hardware evidence included.
- [`docs/backlog/`](docs/backlog/) — everything unscheduled, split by area,
  with resolve history under `docs/backlog/resolved/`.

Entries carry YAML frontmatter so the sets are filterable without parsing
prose: `rg -l 'status: "open"' docs/backlog`, `rg -l 'priority: "high"'
docs/backlog`, `rg -l 'area: "protocols"' docs/roadmap`.

## Milestone status

| # | Milestone | Status |
| - | --------- | ------ |
| 1 | [Nested backend](docs/roadmap/01-nested-backend.md) | done |
| 2 | [Keybindings layer](docs/roadmap/02-keybindings-layer.md) | done |
| 3 | [Real tty/DRM backend](docs/roadmap/03-tty-drm-backend.md) | done |
| 4a | [Config file](docs/roadmap/04a-config-file.md) | done |
| 4b | [Window decorations](docs/roadmap/04b-decorations.md) | done |
| 5 | [Cursor rendering for `--tty`](docs/roadmap/05-cursor-rendering.md) | done |
| 5b | [VT-switch-back `EPERM`](docs/roadmap/05b-vt-switch-eperm.md) | done |
| 6 | [Real GPU rendering pipeline](docs/roadmap/06-gpu-pipeline.md) | done — **verified on a real GPU 2026-09-21** (one claim left: `Modifier::Invalid` on an `Invalid`-only driver) |
| 7–18 | [Backlog-driven hardening and protocol work](docs/roadmap/) | done |
| 19 | [Multi-output](docs/roadmap/19-multi-output.md) | **in progress (phases A–D + F done, E hardware-gated)** |

Item 6 (a **GLES** renderer as an optional alternative to pixman, selected
per-backend, with GPU-free operation kept as a hard requirement) was the last
item on the original ordered list, and all four of its stages have landed —
which means **every milestone on that list is now built**.

**It has now run on a real GPU, and two of the three open claims are
settled** (2026-09-21, Apple M2 under Asahi Linux — `Asahi.md`'s Test 4).
Before that, no part of this milestone had: the dev VM's EGL device answers
`is_software() == false` and is then served by llvmpipe, so every number
behind these four stages was a software rasteriser's, and three things were
claimed rather than shown. Where they stand now:

- **GPU scanout is faster than the CPU path — shown.** 4–5x less compositor
  CPU under damage (20.8% of a core → 4.3% under *large-damage* pointer
  motion, the injected path jumping ~900×600 logical pixels per event, not a
  small cursor-rect move; 52.0% → 14.0% under a full relayout), the same
  pixels bar the cursor's antialiasing, ~0.2 W *less* power, 7–16 MB more
  RSS, and no measurable CPU at idle on either tier. The VM's extrapolation
  was right about the direction. Its *reasoning* does not transfer verbatim,
  though, and it would be wrong to say the deleted read-back explains this
  number: the dumb tier never reads back — it composites with pixman and
  memcpys the damaged region into a DRM dumb buffer (`tty/dumb.rs`,
  `tty/buffers.rs`). The 18–31x read-back cost was the *gles-offscreen*
  tier's. What this A/B measures is the GPU rasterising instead of the CPU
  **plus** the deleted memcpy, and it cannot separate the two.
- **The split render/display topology works — shown, and more cheaply than
  designed for.** One `GbmDevice` serving allocator, exporter and EGL is
  enough on the machine where AGX owns the render node and `apple,dcp` owns
  the connectors; the separable construction reserved for that case is not
  needed.
- **The `Modifier::Invalid` widening on a driver that reports
  `Invalid`-only — still unshown.** No such driver has been met.

Scanout drives every plane it can claim — cursor plane active where exposed,
overlay planes enumerated per CRTC, `ALLOW_SCANOUT` passed with its capture
fix — which is phase 2 of `docs/backlog/resolved/gpu-scanout-planes-done.md`,
now complete. No window leaves the primary plane yet (no scanout candidates;
the exporter admits client dma-bufs since the exporter widening, but the
primary still requires a swapchain format/modifier match no client buffer
meets -- [format gate](docs/backlog/core/gpu-primary-direct-format-gate.md)),
so captures stay correct. Two of its four stages have landed
(PR #129: the renderer seam, pixman still the only implementation, provably
zero behaviour change; PR #130: the GLES pipeline behind
`--renderer pixman|gles` and `[renderer] backend`, off by default, every
pixel-readback suite passing byte-identically under both renderers) and
stage 3 landed split in two, because the `DrmCompositor` path and the
structural room it needs are not reviewable as one diff: PR #133 (the
`gpu-scanout` Cargo feature, off by default because `backend_gbm` is a
link-time libgbm dependency and GPU-free operation is a hard requirement, plus
the dumb presenter lifted out of `Tty`) and PR #135 stacked on it (GPU scanout
for `--tty --renderer gles`: the frame composited straight into the buffer the
CRTC scans out, with no read-back and no dumb-buffer memcpy). Stage 4, the
last, has landed (PR #147): the `zwp_linux_dmabuf_v1` tranche is derived from whatever
renderer the session actually built rather than hard-coded to what the CPU
renderer can map — which closes the case of an EGL display with no dma-buf
import capability at all, where the old fixed pair would have been advertised
and every GL client killed through `create_immed` for believing it. It also
moves the global's creation from `State::new` (where no renderer exists yet)
to `headless::init_named`, which no client can observe because nothing
dispatches wayland in between. Two claims that entry used to
make were checked and corrected in the same PR: the "render-target/presentation
split" it called the seam a GPU renderer slots into **did not exist** (one
monolithic `State::render()` hard-wired to pixman three ways), and the pinned
Smithay rev has **no Vulkan renderer at all** — `src/backend/vulkan/` is an
allocator. Everything since item 7 has been pulled forward from
[`docs/backlog/`](docs/backlog/) rather than the order above, at the user's
direction or because a live crash-DoS or daily-driver gap jumped the queue —
each item's own file records why it landed when it did.

## Recently shipped (since 2026-09-15)

- **[Subsurface depth bound](docs/backlog/resolved/subsurface-depth-bound-done.md)**
  (2026-09-23, PR #227) — a client could crash the compositor by nesting
  `wl_subsurface`s deeply (every Smithay surface-tree walk recurses per
  level; on `main` 10000 overflowed a 2 MB stack in release, 30000 an
  8 MB one after a 108 s stall, and in debug a chain 8588 deep overflowed
  inside Smithay's own `is_ancestor`, during creation, before anything
  scoot sees). A guard in
  `dispatch.rs` refuses a `get_subsurface` with `bad_parent` before
  Smithay links anything, when the new parent's depth plus the height of
  the subtree being attached would exceed 64 (`subsurface_depth.rs`). The
  height is what makes it sound: a subsurface can be re-attached with its
  subtree after `wl_subsurface.destroy` or its parent's destruction, and a
  role-less surface can be given children first, so a parent-depth cap
  alone is bypassed bottom-up (measured: that alone overflowed). Frame
  cost at the cap unchanged; mpv (which builds its two-level tree
  bottom-up), foot, weston's demo and GTK 4's demo video player unaffected live. Filed:
  Smithay's missing `bad_surface` for a second `wl_subsurface`; many
  desync subsurfaces side by side stall roughly quadratically.

- **[Popup depth bound](docs/backlog/resolved/popup-depth-bound-done.md)**
  (2026-09-23, PR #226) — a client could crash the compositor with a deep
  acyclic chain of `xdg_popup`s (Smithay's popup tree recurses per level;
  on `main` 10000 overflowed a 2 MB stack in release, 3000 froze it for a
  minute). Chains are capped at 64 in `new_popup` by a bounded walk, and
  the cap is made sound by refusing every way an admitted chain could
  grow (`popup_parent.rs`): `xdg_surface.already_constructed` and
  `xdg_wm_base.not_the_topmost_popup`, which the pinned Smithay does not
  enforce, plus a parent with no live role object, plus a layer surface
  adopting anything but a fresh parentless popup (review found that
  re-adoption cut a chain short under a node that stayed deep). Every
  chain and every popup-tree node is now at most 64 deep (65 with an
  input method's candidate window, a leaf). Also fixed: a
  surface made a popup again had a same-flush child inserted under its
  dead tree node (invisible, then disconnected, and nestable without
  bound). GTK 3 measured live closing submenus first in every path; frame
  cost with a max-depth chain unchanged. Filed: many side-by-side popups
  stall quadratically; deep subsurface nesting overflows the stack.

- **[Popup constraint adjustment](docs/backlog/resolved/popup-constraint-adjustment-done.md)**
  (2026-09-23, PR #225) — the regression PR #224 made visible (a menu at a
  shared output edge cut off) and the older one at every outer edge: an
  `xdg_popup` is flipped, slid or resized per its positioner into a target
  (`popup_constraint.rs`) — a window's popup into its output's usable area
  (it draws under a `top` bar), or the whole output while that window
  covers it fullscreen; a layer surface's popup into its whole output —
  measured from the immediate parent, so submenus fit too. Applied at the
  initial configure (the first point a layer popup has a parent) and on
  `reposition`; inputs beyond 2^24 are left unadjusted, which is what keeps
  Smithay's arithmetic from overflowing. Verified live against a GTK3
  context menu (asks for all six adjustments). Also fixed, pre-existing:
  a popup whose parent chain loops back to itself hung the compositor in
  Smithay's root walk; now refused (`popup_parent.rs`). Follow-up filed:
  [reactive re-constraining](docs/backlog/core/popup-reactive-reconstrain.md).

- **[Windows stay on their own output](docs/backlog/resolved/windows-bleed-across-outputs-done.md)**
  (2026-09-23, PR #224) — a window is drawn and takes input only on the
  output it is placed on (`output_clip.rs`): each frame gathers only its
  own output's windows and rings, in output-local coordinates; both hit
  sites filter by an output stamp `apply()` writes beside `map_element`;
  popups go with their parent (cut at a shared edge, as at an outer one,
  until the popup constraint adjustment above).
  Also fixed: the ring and rounded clip were built in global coordinates,
  so no output after the first ever showed a ring. Single-output frames
  byte-identical under both renderers. Follow-ups filed:
  [popup constraint adjustment](docs/backlog/resolved/popup-constraint-adjustment-done.md),
  [output membership by geometry](docs/backlog/core/output-membership-by-geometry.md).

- **[Client fullscreen](docs/backlog/resolved/client-fullscreen-done.md)**
  (2026-09-22, PR #223) — a per-window fullscreen state
  in `scoot-core`: covers the output (gaps, ring, bar zones) while its
  column is focused, keeps its strip slot, restores the arrangement exactly;
  consume/expel/moves end it. Wired to xdg `set_fullscreen` (output hint for
  the focused window only, discarded on unmap), the wlr request + `fullscreen`
  state bit (v2+), IPC `toggle-fullscreen`/`set-fullscreen` + snapshot field,
  and `Super+f`. The `top` layer is hidden from drawing, pointer and
  keyboard under a covering window; `overlay` and the lock stay above. Also
  fixed frame learning pairing a commit with the latest configure sent
  rather than the one acked. Unblocks
  [scanout candidates](docs/backlog/core/gpu-scanout-candidates.md).

- **[XWayland Phase 1 skeleton](docs/backlog/protocols/xwayland-support.md)**
  (2026-09-22, PR #221) — opt-in `--xwayland`/`[xwayland]` (default off,
  own cargo feature): server spawn + abstract socket + READY ordering,
  handler impls (activation steal refused by default), loud Wayland-only
  fallback, `DISPLAY` plumbing, no-token-for-X deferred to Phase 2 with
  documented policy (review accepted: nil exposure today). Mapping
  exclusion proven three ways (constructor absence). Review clean; two
  lows fixed in-round (WM-failure `DISPLAY` withdrawal, emission guard).
  Ticket stays OPEN for mapping → focus gate → clipboard → capture.
  Remainder filed as [WM-failure
  pin](./docs/backlog/protocols/xwayland-phase1-wm-failure-pin.md).

- **[XWayland Phase 0 spike](docs/backlog/protocols/xwayland-support.md)**
  (2026-09-22, PR #220, docs-only) — break-site inventory re-verified at
  the pinned rev (two ticket corrections: single `XWayland::spawn`, no
  `delegate_xwayland_shell!`), live measurements (READY ~70ms, 55MB RSS,
  30/30 storm, steal refused), opt-in flag + config key decided (default
  off), conditional GO with top-3 risks + scoped Phase 1. Ticket stays
  OPEN; review found no blocking issues.

- **[GPU scanout: framebuffer exporter widened, force path proven
  live](docs/backlog/resolved/gpu-direct-scanout-exporter-done.md)**
  (2026-09-22, branch `gpu-direct-scanout-exporter`) — exporter
  `NodeFilter::None` → `All` (`Node(..)` would be inert: a client dma-buf's
  node is set only by v6 `set_sampling_device`, which no measured client
  sends); force/refusal halves pinned against the code that runs
  (`Captures::capture_target`/`record`, `ForceComposite`) and watched firing
  live on the dev VM with an uncommitted `ANY`-bit experiment (primary
  scanning out the client's `XR24`/`LINEAR` fb, captures forced and
  correct, paused capture refused loudly). Gating finding: primary-direct
  stays unreachable on every machine measured because Smithay's primary assignment
  compares whole `Format`s (opaque fourcc vs `AR24` swapchain; `LINEAR` vs
  implicit modifier) — filed as the [format
  gate](docs/backlog/core/gpu-primary-direct-format-gate.md). Also: forced
  frames drop the `ANY` bit too; `capture_pixels_for` renders before
  forcing, not after.

- **[GPU scanout `ALLOW_SCANOUT` + capture fix
  (step 3)](docs/backlog/resolved/gpu-scanout-planes-done.md)** (2026-09-22,
  branch `allow-scanout-capture-fix`, PR #218) — the flag WITH its capture
  fix, never apart: direct frames mark the recording
  (`ScanoutFrame::primary_direct` → `Captures::note_direct`), captures
  served off a marked recording force one composite-only frame first
  (`State::ensure_scanout_capture_current`, both IPC `screenshot` and
  `ext-image-copy-capture-v1`), loud refusal where the force cannot draw.
  Gating answer first: direct is unreachable twice over (no candidates;
  exporter stays `NodeFilter::None`), so the flag is assignment-inert and
  the fix is proven by pins + pinned-source trace + live byte-identity
  (cursor moves, VT cycle, cross-tier diff confined to the cursor box,
  `UnknownPlane` 0). Ticket CLOSED; README bullet rewritten (cursor-half
  staleness from #216 cleared too). Coordinator-filed, no gh issue.

- **[GPU scanout overlay planes
  (step 2)](docs/backlog/resolved/gpu-scanout-planes-done.md)** (2026-09-22,
  PR #217) — overlay list populated per CRTC (rides whole; no candidates
  exist yet so nothing can be claimed — verified no production surface is
  `ScanoutCandidate`, hence "at most the cursor ever goes missing" holds).
  Virtio has zero overlay planes (fallback proven live). Ticket stays OPEN
  for `ALLOW_SCANOUT` + capture fix.

- **[GPU scanout cursor plane
  (step 1)](docs/backlog/resolved/gpu-scanout-planes-done.md)** (2026-09-22,
  PR #216) — KMS cursor plane driven where the CRTC exposes one
  (`ALLOW_CURSOR_PLANE_SCANOUT` only, overlay dropped, `ALLOW_SCANOUT`
  still out); virtio-gpu needed `CURSOR_PLANE_HOTSPOT` pre-`DrmDevice::new`
  (review caught the post-`new` ordering blacking the screen — screenshots
  are blind to scanout health, commit-health assertions now required).
  Plane-active on virtio (FB-tracked, captures cursorless as documented).
  Ticket stays OPEN for overlay → `ALLOW_SCANOUT` + capture fix.

- **[Config reload autostart
  follow-ups](docs/backlog/resolved/reload-autostart-followups-done.md)**
  (2026-09-22, PR #215) — the three PR #214 review findings, one small PR:
  a failed spawn refuses by name and retries on the next reload
  (per-entry snapshot; `applied` means the entry started), the locked-skip
  message promises only a decision (`pending entries are decided on the
  first unlocked reload`), and the removal-while-locked cancellation is
  pinned. No `PROTOCOL_VERSION` bump (strings are payload; wire shape
  pinned). Coordinator-filed ticket, no gh issue.
- **[Config reload complete: autostart spawn-delta + restart
  wording](docs/backlog/resolved/config-reload-full-done.md)**
  (2026-09-22, PR #214) — only entries the session has not seen run (new
  `spawn` entries once each through startup's `act` path; a reloaded `quit`
  refused by name, never acted on; locked reloads skip and defer to the
  first unlocked one), and the `tty.gpu` / `renderer.backend` refusals now
  name restart (`takes effect on restart`). End state: live except those
  two fields. No `PROTOCOL_VERSION` bump (strings are payload; wire shape
  pinned). Review pending; ticket moved to `resolved/`.

- **[Config reload phase 3: `output.scale`
  live](docs/backlog/core/config-reload-full.md)** (2026-09-22, PR #213) —
  re-advertise + fractional companion re-send + geometry refile + `apply()`;
  `--nested` still refuses non-1.0; outputs recompacted order-preservingly
  (review found preserve-positions diverged from fresh startup — fixed
  in-round with a fail-first pin). No protocol bump. Ticket stays OPEN for
  autostart → reword.

- **[Config reload phase 2: `column_widths`
  live](docs/backlog/core/config-reload-full.md)** (2026-09-22, PR #211) —
  clamping `min(preset, len-1)` after `validated()`; shorter lists pull OOB
  columns in, longer lists touch nothing; set-column-width/cycle follow.
  Review clean with live 1176→582px proof. Ticket stays OPEN for scale →
  autostart → reword.

- **[Config reload phases 0+1: applier split +
  cursor](docs/backlog/core/config-reload-full.md)** (2026-09-21, PR #209)
  — `apply_reload` split into per-field appliers (behavior-preserving);
  cursor theme/size/color now reload live with idempotent second reloads
  and SIGHUP inheritance; phases 2+ refusal strings byte-identical, no
  protocol bump. Review caught a false under-lock doc claim (fixed in-round).
  Ticket stays OPEN for `column_widths` → scale → autostart → reword.

- **[Multi-output remainder G+H: pointer-output placement + default output
  binds](docs/backlog/core/multi-output-remainder.md)** (2026-09-21,
  PR #208) — new windows file under the pointer's output (shared helper, so
  wlr announce and core filing agree; primary fallback), and four new
  default binds (`Super+comma/period` focus, `+Shift` move; 36→40).
  Single-output byte-identical by construction. Review clean with its own
  live `--headless --outputs 2` spot-checks. Remainder file stays OPEN for
  hardware-gated E1/E2.

- **[Ring/content corner alignment at fractional
  scale](docs/backlog/resolved/rounded-ring-content-fractional-mismatch-done.md)**
  (2026-09-21, gh #205, PR #207) — the ticket's radius-rounding hypothesis
  falsified (2.0 fails like 1.5); the real bug was `src: None` sampling the
  top-left logical sub-rect of physical-pixel ring-strip buffers. Explicit
  full-buffer `src` on both strips; GLES fixed by the same renderer-generic
  change; ±3% cost delta against ±7% baseline drift. Review clean with its
  own texel-by-texel live spot-check.

- **[Set a column's width
  directly](docs/backlog/resolved/set-column-width-done.md)**
  (2026-09-21, gh #204, PR #206) — `set-column-width N` (0-based into
  `[layout] column_widths`, out-of-range ignored without disturbing learned
  widths) through the workspace-index path (core → IPC → shared grammar →
  config emit → `configuration.md`/`ipc.md`); no default bind, no
  `PROTOCOL_VERSION` bump, toggle/memory still out. Review clean; CI red
  once on `cargo fmt --all` (fixed whitespace-only), full green before
  merge.

- **[`--print-default-config --write`
  convenience](docs/backlog/resolved/default-config-write-done.md)**
  (2026-09-21) — the stdout emitter's follow-up: `--write` places the same
  emission at the default location (parents created, `0o600`, symlinks
  refused as themselves), refusing loudly instead of overwriting — one
  winner under concurrency. Coordinator-directed, no gh issue.

- **[SIGHUP trigger for config
  reload](docs/backlog/resolved/reload-sighup-trigger-done.md)**
  (2026-09-21) — `kill -HUP` drives the shared reload path (no reply
  channel; summary to the log), composes with the SIGCHLD reaper, children
  keep default HUP. Coordinator-directed, no gh issue.

- **[README + docs consistency
  re-audit](docs/backlog/resolved/readme-rereview-done.md)**
  (2026-09-21) — docs-only: every user-facing surface since PR #159
  re-verified against `README.md` and `docs/` (both `--help` outputs
  and `--print-default-config` run live, including a Linux emission
  from the dev VM). Twelve of thirteen surfaces already consistent;
  six fixes: README "Not yet" retitled past one-output, three stale
  one-output sentences in `docs/`, the `nix.md` live-defaults paste
  (missing `corner_radius`, `cursor_theme`/`gpu` shown as defaults
  while unset), one word in the CI paragraph. Coordinator-directed,
  no gh issue.

- **[NixOS-conversion docs gaps (gh
  #178)](docs/backlog/resolved/nixos-conversion-docs-gaps-done.md)**
  (2026-09-21) — docs-only: the CHANGELOG rename entry gains its
  session-identity half (`XDG_CURRENT_DESKTOP`, `DesktopNames`,
  `scoot-portals.conf` — framed as the converter's own files, since none
  of the old names ever existed in-tree) plus the `yackey-labs/flexwm` →
  `scoot-sh/scoot` move; `docs/nix.md` gains a migration section (wrapper
  binary, orphaned-config fail-safe warning, module split), an end-to-end
  session example on the `session.command` surface (eval-rendered, not
  booted), a packaged-renderer pointer on the live-defaults reference, and
  the `scoot msg reload` one-liner. Two premises stale at fix time: the
  roadmap row (#198 fixed it) and the renderer premise (#198's GPU tiers).

- **[Nix package reaches both GPU tiers (gh
  #177)](docs/backlog/resolved/nix-gpu-tiers-done.md)**
  (2026-09-21) — `packages.scoot` force-links libEGL (nixpkgs niri's
  `--no-as-needed` trick, Linux-only) so packaged `--renderer gles`
  reaches the OS's EGL drivers instead of panicking in Smithay's
  `dlopen` (proven on the dev VM: `the GLES renderer is up` over
  llvmpipe); new `packages.scoot-gpu` carries `gpu-scanout` (`ldd`
  shows libgbm, default still doesn't); a `dlopen` pre-flight turns a
  missing libEGL into the designed startup error (proven live with the
  library hidden in a mount namespace). Mesa ICDs stay the host OS's
  by decision; Test 4 on the Asahi hardware is now answered (2026-09-21 --
  see milestone 6 above and `Asahi.md`).

- **[CI guards the Nix packaging (gh
  #173)](docs/backlog/resolved/ci-nix-packaging-done.md)**
  (2026-09-21) — `nix flake check -L` every PR in both jobs (each its own
  systems) plus `nix fmt --check` over all tracked `.nix`, proven to fire
  on a module typo cargo can't see; `nix build .#scoot .#scootctl`
  main-only (5m10s cold, no reusable cache — per-push tax declined).
  Workflow + docs only, zero `.rs`.

- **[NixOS `session.command` (gh
  #171)](docs/backlog/resolved/nixos-session-command-done.md)**
  (2026-09-21) — the greeter entry's `Exec=` is no longer fixed: a
  null-default option takes the full command line (bare `--tty`
  byte-identical by default, `-- COMMAND` append and wrapper-script
  path both expressible), paired with the HM `sessionScript` docs both
  sides, pinned in `nix/tests.nix`. Flake-only, zero `.rs`.

- **[`scoot --version` / `scootctl --version` (gh
  #176)](docs/backlog/resolved/cli-version-flag-done.md)**
  (2026-09-21) — both print `scoot <version> (ipc protocol <N>)` from one
  shared helper with no session needed; the bare `version` word stays the
  remote IPC request. Nine CLI tests, no wire change.

- **[Flake polish: `homeModules` alias, Darwin default, nixfmt (gh
  #175)](docs/backlog/resolved/flake-polish-done.md)**
  (2026-09-21) — `homeModules` alias over legacy
  `homeManagerModules`, Darwin HM default `null` (files-only), formatted
  `compositor-deps.nix`, un-aliased formatter. Flake-only, zero `.rs`.
- **[home-manager `sessionScript` follows `configFile` (gh
  #174)](docs/backlog/resolved/hm-session-script-path-done.md)**
  (2026-09-21) — the script path is derived from the config's directory
  (`<dirOf configFile>/session.sh`), so a relocated config keeps its
  script beside it instead of silently splitting the pair; the default
  still renders `scoot/session.sh` byte-identically, and `hmRelocated`
  now pins the pairing. Flake-only, zero `.rs`.

- **[`packages.scoot` no longer ships `scootctl` (gh
  #172)](docs/backlog/resolved/scoot-package-ships-scootctl-done.md)**
  (2026-09-21) — `cargoBuildFlags = [ "-p" "scoot" ]`, packaging matching
  the documented split: `packages.scoot` bin carries only `scoot` on Linux
  and Darwin alike (the 529440-byte redundant client is gone),
  `scootctl`/wrappers/`apps`/module checks all still resolve, live IPC
  proven against the packaged pair. Flake-only, zero `.rs`.
- **[Top-layer bar content with no toplevels (gh
  #183)](docs/backlog/resolved/layer-content-without-toplevels-done.md)**
  (2026-09-21) — closed as could-not-reproduce, no compositor change: a
  real exclusive-zone bar with a committed buffer draws with zero
  toplevels on current `main` *and* on the reported rev `4f8707c` alike
  (live `--nested` screenshot matrix — magenta fraction 1 in all three
  cells at both revs — plus harness pixel pins for the mapped and the
  bufferless zero-window cases, sensitivity-proven). The field symptom is
  the designed bufferless-bar shape (zone reserved, nothing painted), so
  quickshell had almost certainly committed no bar buffer while the
  workspace was empty;
  shared root with #182 refuted on the render half as well.
- **[Layer-shell pointer clicks (gh
  #182)](docs/backlog/resolved/layer-shell-pointer-clicks-done.md)**
  (2026-09-21) — closed as could-not-reproduce, no compositor change: the
  issue's exact `--nested` probes (`pointer click` at two bar points plus
  right-click, over a real exclusive-zone bar) deliver `enter` + press +
  release on current `main` and on the reported rev alike, with and
  without a toplevel mapped. Kept as a wire-level pinning test (bar
  left/right/middle plus the launcher overlay); shared root with #183
  refuted for the input path.
- **[Fractional-scale ring-hole
  drift](docs/backlog/resolved/ring-hole-fractional-drift-done.md)**
  (2026-09-21) — the PR #184 follow-up: the painted-ring reuse check
  compares the in-canvas paint offsets (`plan.inner`/`plan.outer`) alongside
  the strip canvases, closing the ≤1px ring-hole offset a canvas-matching
  move could leave at fractional scales until the next key change. Still
  allocation-free, integer scales provably untouched (a dedicated
  never-repaints sweep); pinned fail-first plus an in-harness brute-force
  sweep over 1.25/1.5/1.75/1.33.
- **[Rounded window
  corners](docs/backlog/resolved/rounded-window-corners-done.md)**
  (2026-09-20) — `[appearance] corner_radius` (logical px, default `0` =
  square, live-reloadable): each window's toplevel tree is wrapped in a
  `Rounded` element cutting the corner staircases out of every draw while
  shrinking the opaque region to match (the two are one atomic change —
  either alone is a slowdown or a stale-corner bug), the focus ring follows
  as two painted strips plus the solid side bars, popups stay square by
  decision. Pixman cost measured small (+9% medians on a three-window
  session, ~0% on the overlap scene, nothing at the default); no renderer
  gating — the staircase works byte-identically on GLES with no shader, and
  the llvmpipe GLES numbers are disclosed as llvmpipe artifacts, not a GPU
  verdict.

- **[Test socket paths overflow macOS `SUN_LEN` under a long
  `$TMPDIR`](docs/backlog/resolved/fixture-socket-sun-len-done.md)**
  (2026-09-20, test-only) — the PR #180 review finding: the
  `msg_broken_pipe` fixture's ~62-char socket filename plus the dev
  Mac's 49-byte `$TMPDIR` exceeded macOS's 104-byte limit (reproduced
  `InvalidInput` pre-fix). Now `scoot-ep-{pid}-{nanos-lo32}-{tag}.sock`
  (32–34 chars, 81–83 total); 4/4 Mac green, full Linux gate green.
  macOS CI is check-only, so coverage is local-only by construction.
- **[Bind-before-spawn in the `msg_broken_pipe`
  fixture](docs/backlog/resolved/broken-pipe-bind-race-done.md)**
  (2026-09-20, test-only) — the PR #179 residual: `serve_once` takes an
  already-bound listener, so program-order `bind` → spawn closes the
  bind-vs-connect race (before-control 21/22 with one exact-mechanism
  30.01s red, after 22/22 green); 30s deadline, nextest backstop and 2s
  regression test all unchanged.
- **[`msg_broken_pipe`'s unbounded `accept()` wedged the suite on a transient connect failure](docs/backlog/resolved/msg-broken-pipe-accept-hang-done.md)**
  (2026-09-20, test-only + one config stanza) — 30s accept deadline in the fixture (loud `TimedOut` naming the cause, preserved across the join) plus a `.config/nextest.toml` backstop (`period = 60s, terminate-after = 2`, sized ~15x above the 7.72s measured max); both proven to bite, full greens under both runners.
- **[Pin the budget cause by message in the two bypass-loop cap
  tests](docs/backlog/resolved/bypass-loop-cap-cause-pin-done.md)**
  (2026-09-20, test-only) — the PR #169 review finding: the two
  bypass-loop cap tests asserted code+interface only while pressure and
  budget refusals share both, so under `prlimit 650` they passed green
  while guarding the pressure cause (reproduced). Both sites now pin the
  budget by message per the coordinator's per-call-site decision (the
  helper's code-only stance stands for its other users); pins proven to
  bite in both directions, 1211-passed nextest green.
- **[Two more dispatch floods in the same fd-pressure class (`immed`,
  second-client)](docs/backlog/resolved/dispatch-flood-remainder-flake-done.md)**
  (2026-09-20, test-only) — the PR #168 follow-up: the `immed` twin takes
  the same two-line treatment (test-thread lock + 896-free headroom,
  budget-cause pin by message), the second-client fill takes headroom only
  (it asserts success, so no kill to discriminate). Both reproduced red
  under `prlimit 650` first (wrong-cause code 1; mid-fill refusal into the
  10s deadline), both fail fast-loud at 0.00s after; 6/6 full-binary greens
  plus 4/4 at 16 threads plus 2/2 nextest (1211 passed).
- **[Dispatch flood tests die on fd-pressure kills under a pressured
  process table](docs/backlog/resolved/dispatch-flood-fd-pressure-flake-done.md)**
  (2026-09-20, test-only) — the dmabuf fill-phase kill landed on the
  wrong cause under `prlimit 650` (reproduced: code 1 on `wl_shm_pool`
  with the pressure message where code 7 on the dmabuf params is
  expected). Both dispatch halves now run under the icon test's
  treatment: raised `RLIMIT_NOFILE` ceiling with verified headroom (896
  free for the 512-retaining fill, 384 for the fd-less single-pixel
  flood — fast loud panic, never a skip), budget-cause pinning by
  message (required where twins share code+object), and the single-pixel
  flood finally taking the shared `FD_FLOOD_LOCK`. The `immed` twin
  reproduces identically and is left for a follow-up per the ticket's
  scope — [since resolved](docs/backlog/resolved/dispatch-flood-remainder-flake-done.md)
  (that entry also covers the second-client fill, the other red in the
  same pressured-suite run).
- **[Icon live-buffer-budget flood flake under `cargo
   test`](docs/backlog/resolved/icon-buffer-budget-fd-pressure-flake-done.md)**
  (2026-09-20, test-only) — the 512-fd flood sat near the process-wide
  fd-pressure boundary by design, so neighbours' fds tipped the refusal
  into the flood (10s `PATIENCE` timeout, never nextest). Now the flood
  raises soft `RLIMIT_NOFILE` to 4096 with verified headroom (fast loud
  panic naming the numbers where the table cannot fit it, never a
  silent skip), the 513th refusal is pinned to the budget by its message
  (same code/object as the pressure refusal), and the 512-count is pinned
  again by a deterministic test opening no fd at all.
- **[No documented flake-consumer path, no home-manager/NixOS
  module](docs/backlog/resolved/flake-consumer-and-home-manager-done.md)**
  (2026-09-20) — both halves: README Install carries the consumer
  snippet plus a pointer, new `docs/nix.md` owns the fuller section
  (modules, platform notes, failure modes, live-defaults reference
  pasted from a real emission), and thin `programs.scoot` modules ship
  on both sides — free-form `settings` via `pkgs.formats.toml` (typed
  schema deliberately refused: it would go stale and lie), an opt-in
  login-screen entry that only ever adds alongside existing sessions
  (default off, per the never-strand rule), and `scoot-portals.conf`
  installed by default (closing the portals-ticket remainder).
  Hermetic eval + rendered-content checks under `nix flake check`;
  module-rendered configs proven live through a real `--headless`
  session (empty, quoting-needing binds, wrong-type fail-safe). No
  compositor/client code touched.
- **[No way to emit a default config
  file](docs/backlog/resolved/default-config-command-done.md)**
  (2026-09-20) — `scoot --print-default-config` writes a commented starting
  file to stdout (never a path, so it cannot clobber), generated from the
  live defaults, parsing back to them (commented keys) with commented scalar
  values pinned to the live defaults by their own test; EPIPE-quiet like
  `--help`, byte-identical across runs, every default bind spelled back in
  loader-accepted form.
- **[The rename entry is closed](docs/backlog/resolved/rename-flex-family-done.md)**
  (2026-09-20, no code — coordinator verification): both halves landed
  earlier (PR #128 the `flexwm` → `scoot` rename, PR #157 the `scootctl`
  split), the `.claude/agents/` role files were renamed by the user, and
  every remaining `flexwm` spelling in the tree is deliberate (evidence
  archives, VM migration text, the on-disk checkout path the 9p share
  points at).
- **[No config reload](docs/backlog/resolved/config-reload-done.md)**
  (2026-09-20) — the biggest remaining daily-driver gap, and the README
  "Not yet" line that named it: `scootctl reload` (new session-level
  `reload` request, `scoot msg` alias included) re-reads the same file
  startup used and re-applies gap, appearance and keybindings live
  (arrangement recomputed, render requested), refusing scale, gpu,
  renderer backend, autostart, cursor fields and column widths explicitly
  in the reply's `refused` list — never silently ignored. A failed reload
  (unreadable, malformed, unknown field) keeps the running config and
  reports loudly; two empty lists mean "changed nothing". The `--tty`
  `Ctrl+Alt+F1..F12` recovery path is un-strippable by reload (shared
  `enforce_vt_binds` with startup), a held key neither wedges nor drops
  across the table swap, and reload applies under session lock
  (deliberate: nothing applied can disclose locked content). New
  `Response::Reloaded` moved `PROTOCOL_VERSION` 2 → 3; the request half
  degrades to error + continue on older servers. Live proof on the dev
  VM: gap + colour + bind change, screenshot pixel diff, injected-key
  bind fire, malformed-file untouched.

- **[Workspace shortcuts: numbered binds plus
  move-to-index](docs/backlog/resolved/workspace-index-keybindings-done.md)**
  (2026-09-20) — both halves of the two-gaps-that-look-like-one ticket:
  `Super+1`..`Super+9` bound to the existing `focus-workspace-index`
  (binds only, no behavior change), and a new
  `move-window-to-workspace-index N` action through the full stack (core →
  `scoot-ipc` variant/conversion → shared `scootctl::action` grammar →
  `Super+Shift+1`..`9` defaults) so a window — or an agent placing one —
  jumps to an absolute workspace instead of stepping. Out-of-range is a
  no-op on both (neither creates nor clamps; the window stays put),
  mirroring what `FocusWorkspaceIndex` already did. Additive wire format,
  no `PROTOCOL_VERSION` bump. Live proof on the dev VM: IPC move,
  injected-key binds, out-of-range no-ops, empty-session moves.

- **[Nothing says how to start a bar, a launcher or a browser at session
  start](docs/backlog/resolved/startup-programs-and-autostart-done.md)**
  (2026-09-20) — "Starting a session" documented (`session.sh` plus the
  webtop `/defaults/startwm.sh` variant; config declares the baseline, the
  script carries the behavior), and `[autostart] commands` built as a flat
  list of action strings through the shared `scootctl::action` parser:
  fail-open per entry (a typo costs the entry, never the `--tty` session),
  entries first in file order, then the `--` command, no supervision (the
  ticket's exclusion stands; reaping already landed). Home-manager stays a
  pointer for the flake ticket. Live proof on the dev VM: marker processes
  observed under the session, malformed entry logged with the session up,
  ordering confirmed in the spawn log.

- **[`XDG_CURRENT_DESKTOP` is set nowhere, so a portal has no backend to
  pick](docs/backlog/resolved/session-environment-and-portals-done.md)**
  (2026-09-20) — `XDG_CURRENT_DESKTOP=scoot` exported unconditionally (a child
  talks to this compositor, so it names this compositor even under `--nested`),
  `XDG_SESSION_TYPE`/`XDG_SESSION_DESKTOP` filled only where no logind set
  them, through one pure `resolve` applied both in-process and per spawned
  child. Activation propagation decided as the session script's job
  (documented, both systemd and s6 shapes); `resources/scoot-portals.conf`
  shipped with capture via `wlr` — the ticket's "no backend speaks ext"
  premise proved stale in upstream source (xdg-desktop-portal-wlr ≥ 0.8.0
  implements `ext-image-copy-capture`; its Screenshot portal shells out to
  ext-only `grim`). Flake install wiring filed as remainder; live portal
  proof impossible on the dev VM (no portal stack), stated not claimed.
- **[`scoot --help` hides `--renderer` under `--tty`](docs/backlog/resolved/cli-help-tty-missing-renderer-done.md)**
  (2026-09-20) — one-line `--help` fix plus a usage-vs-parser pinning test; no behavior change, no README change (it was already right).
- **[README audit +
  tightening](docs/backlog/resolved/readme-rewrite-done.md)**
  (2026-09-20) — the last item of the 2026-09-19 directed queue, worked as an
  audit rather than a rewrite: PR #132's front-door shape was verified
  against its own relocate-don't-drop constraint item by item (keys vs
  defaults, config example vs `Config::default()`, both `--help` outputs
  run live, all 31 protocol versions against a live `wayland-info` dump,
  all 15 README anchors resolved mechanically). Five doc corrections (the
  CI paragraph's ldd/macOS/nix-shell scope, the "one exception" that has
  been two since the renderer startup-error landed, the
  `--headless`/`--nested`-only claim wrong in `gpu-scanout` builds), dmabuf
  given its own subsection anchor, and one code bug filed-not-fixed
  (`scoot --help` hides `--renderer` under `--tty`). Docs-only, no wire,
  no `PROTOCOL_VERSION` change.
- **[`smoke-test.sh` no longer defaults to someone else's
  binary](docs/backlog/resolved/smoke-test-binary-default-done.md)**
  (2026-09-20) — the shared-target-dir default handed three agents a binary
  from another branch in one session, silently. `SCOOT`/`SCOOTCTL` now
  default to the invoking tree (`$CARGO_TARGET_DIR`, else the script's own
  repo) with `SCOOTCTL` always paired next to `SCOOT` (same build, never a
  split-brain pair); any unresolvable binary fails loudly before anything
  launches, and the header prints path + source + mtime. Proven live: a
  default run, a bare-name-on-PATH run from a foreign cwd under a
  spaced `SMOKE_PREFIX`, and a concurrent pair with disjoint binaries and
  disjoint headers. No README change (no smoke env contract there).
- **[Split the CLI out into
  `scootctl`](docs/backlog/resolved/scootctl-split-done.md)**
  (2026-09-20) — the second half of the `rename-flex-family` ticket. The
  IPC client is now its own `scootctl` lib+bin crate; `scoot msg ...` stays
  as a permanent alias that parses and runs through it, so the two entry
  points cannot drift (pinned by a 29-case parse-equivalence unit test and
  a smoke-test section comparing live replies byte for byte, screenshot
  PNGs included). Agents reach for `scootctl`; the macOS flake default is
  now the client with zero cfg gating. No wire change, no
  `PROTOCOL_VERSION` bump.
- **[Every spawned child becomes a
  zombie](docs/backlog/resolved/spawned-children-never-reaped-done.md)**
  (2026-09-20) — `State::spawn` dropped the `Child` and nothing installed a
  `SIGCHLD` handler, so every `--` startup command, `spawn` bind and
  `scoot msg action spawn` stayed `<defunct>` for the session's life
  (confirmed live: four spawns, four zombies). Now a `sigaction` handler
  writes one counter increment to a wake eventfd and the loop drains exactly
  the pids `spawn` tracks — never `waitpid(-1)`, which would steal the
  unit-test binary's own forked children under `cargo test`. The ticket's
  measured execve table decided the mechanism and is written down next to
  the code: `SIG_IGN` survives exec (breaks children's own `wait()`),
  the signal mask survives exec with libstd resetting SIGPIPE only (rules
  out signalfd without a `pre_exec` reset in every child), a caught handler
  is reset for free. Pinned by an in-harness burst test (fail-first:
  eight `Z`s with the install neutered) and a child-disposition test
  (`SIG_DFL` + empty mask, sensitivity proven by forcing `SIG_IGN`).
  Live probe: 16 spawns, zero zombies, `SigBlk` empty. No README change
  (internal reliability fix, no user-facing surface).
- **[Host configures coalesced to one resize per frame](docs/backlog/resolved/coalesce-host-configures-done.md)**
  (2026-09-20) — a drag's configure-per-pixel-step no longer rebuilds the
  pool and the render target per step: a later configure only overwrites a
  two-`i32`, allocation-free queue slot, and the next render drains at most
  one size per frame tick (first configure stays immediate, same-size stays
  ignored). Live flood (79 distinct sizes, no settle): 81 modes pre-fix, 25
  post-fix release. Settled resizes cost one frame-tick wait more (18ms →
  31ms end-to-end release); gles rebuilds confirmed at 15.5ms apiece, which
  is what makes one-per-tick load-bearing there. The ticket's watch-for
  proved real — a failed resize's never-rendered size stayed in
  `Output::modes` and the next refresh announced it — and is fixed by
  `delete_mode` on the failure path (fail-first wire proofs); the
  already-bound `wl_output` transient is inherent (no un-prefer) and stated,
  not fixed. No README change (internal perf fix, no user-facing surface).
- **[The two webtop field reports](docs/backlog/resolved/nested-follow-host-resize-done.md)**
  (issues #144/#145, 2026-09-19) — shipped together, being one field report
  from the deployment `README.md` names. **`--nested` now follows the host
  window's size for the life of the session** instead of acting only on the
  first configure: every `xdg_surface::Configure` is classified by one pure
  function into first-configure / resize / nothing-to-do, with the
  third keeping a host's same-size configures (activation, maximize, a
  focus change) from rebuilding a working render target. The design
  question the entry posed is answered *in the code*, as two entry points
  over one private core rather than one function inferring its own failure
  policy: `apply_first_configure` stops the loop, `apply_resize` logs and
  keeps the session — and it returns no `Result`, so "fatal" is not
  something a caller can reach for by accident.
  Two latent bugs fell out of writing that down. `replace_render_target`
  resized the render target *before* allocating the new pool, so the one
  failure the non-fatal path exists for (a bigger pool that would not
  allocate) left the two at different sizes — `present()`'s guard drops
  every frame in that state, i.e. a live session whose window never updates
  again; the order is now reversed so `Err` really means nothing moved. And
  `State::resize_output` (shared with `--tty` hotplug) published the new
  mode to `wl_output` clients before the render target was rebuilt, leaving
  them believing a size nothing renders at when it failed; the advertised
  mode is now put back. Both pinned by tests, the second fail-first.
  **And a clean disconnect now logs at `DEBUG`**, so an idle webtop session
  stops emitting two lines a second (Selkies polls `wl-paste` every 500 ms,
  each poll a fresh connection); `ProtocolError` stays at `warn!`, which is
  where all the diagnostic value was. The audit for other per-connection
  INFO lines came back empty by reading *and* by measurement — ten
  `wayland-info` connect/bind/disconnect cycles add zero lines to an
  `info`-level log.
- **[`ext-idle-notify-v1` + `idle-inhibit-unstable-v1`](docs/backlog/resolved/ext-idle-notify-resolved.md)**
  (PR #38, 2026-09-15) — the automatic trigger session-lock had no other way
  to get. A `swayidle`-style daemon can now idle, resume and re-idle the
  seat (field-proven live); inhibitors hold it awake.
- **[The IPC bundle](docs/backlog/resolved/protocol-bundle-resolved.md)**
  (PR #39, 2026-09-15) — `OutputSnapshot.usable`, a focus-workspace-N action
  (`focus-workspace-index N`), and an ambient `locked` flag on every `Ok`
  reply. Landed with **no** `PROTOCOL_VERSION` bump: all three are
  defaulted/additive.
- **[The four protocols `foot` warned about](docs/backlog/resolved/foot-protocol-warnings-done.md)**
  (issue #40, PR #41, 2026-09-15) — `wp-cursor-shape-v1` (ten
  procedurally-drawn fallback shapes, plus real xcursor-theme loading so a
  themed session keeps its real artwork instead of regressing to line art),
  `xdg-activation-v1`, `xdg-toplevel-icon-v1`, and `text-input-v3` +
  `input-method-v2` (the IME popup). Verified before/after against `foot`'s
  own warnings.
- **[`xdg-activation-v1` had no input-serial gate](docs/backlog/resolved/activation-serial-validation-done.md)**
  (PR #42, 2026-09-16) — found by independent review of PR #41. An
  unfocused client with no user interaction at all could self-activate its
  own surface; a token now has to name a real, recent key/button event
  actually delivered to the requesting client, not just be young and under
  the count cap.
- **[`--tty` background "not painted"](docs/backlog/resolved/tty-background-not-painted-done.md)**
  (PR #43, 2026-09-16) — misdiagnosis, not a rendering bug: the background
  was always painted, and the smoke test's sample pixel sat on the cursor
  (the one thing only `--tty` draws). Fixed in the test script; no
  compositor code changed.
- **[Popup input](docs/backlog/resolved/xdg-popup-input-resolved.md)**
  (PR #44, 2026-09-16) — `xdg_popup.grab` is honoured, with a stated focus
  precedence (lock > exclusive layer surface > popup grab > window /
  click-focused layer surface): locking dismisses any open popup grab and
  refuses new ones, so a menu left open at lock time cannot receive the
  password. Layer-parented popups turned out to already work.
- **[Shared test harness + `cargo-nextest`](docs/backlog/resolved/large-test-file-organization-done.md)**
  (PR #45, 2026-09-16) — extracted the real-client harness five of the
  largest test files each reimplemented (707 fewer duplicated lines), split
  the two biggest by concern, adopted `cargo-nextest` alongside `cargo
  test`. Infrastructure, not a protocol or user-facing change.
- **[`ext-foreign-toplevel-list-v1`](docs/backlog/resolved/foreign-toplevel-list-done.md)**
  (PR #47, 2026-09-16) — the window list an external taskbar, dock or
  alt-tab switcher reads, alongside the workspace list `ext-workspace-v1`
  already gave them. Enumeration only (the protocol has no control
  requests); the identifier is `<generation>-<window id>`, so a client can
  go from a toplevel it found here to `scoot msg action focus-window-id
  N`. Measured caveat, filed as its own item and
  [since resolved](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md):
  quickshell — and so DMS and Noctalia — binds the *wlr* protocol and
  ignores this one.
- **[Output management, read half](docs/backlog/resolved/output-management-read-only-done.md)**
  (PR #49, 2026-09-16) — `zwlr_output_manager_v1` (version 4), what a shell's
  Settings → Display page and `wlr-randr` read the screen's modes, position,
  scale and transform from. The wlr protocol rather than an `ext-` one only
  because no successor exists at the pinned rev — checked, not assumed — and
  Smithay carries no helper for either, so the handler layer is hand-written
  against the generated wlr bindings, the shape `gamma_control.rs` already
  uses. Every value is read from the same `Output` that configures
  `wl_output`, with a test that binds both on one connection and compares
  them. **Read-only by design**: `apply`/`test` always answer `failed`, and
  reconfiguration is
  [closed as a deliberate refusal](docs/backlog/resolved/output-management-reconfiguration-done.md)
  — scoot has one output with a fixed mode, position and scale, so a
  `succeeded` that changed nothing would be a settings page that lies.
- **[`wlr-foreign-toplevel-management-unstable-v1`](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md)**
  (PR #50, 2026-09-16) — the other half of PR #47 above, and the window
  list DMS and Noctalia actually read. Published alongside the `ext-` one
  rather than instead of it, from the same three window-lifecycle events, so
  the two are one list described twice. Enumeration (title, app id,
  `output_enter`), `activate` and `close`, and `activated` as the only state
  bit scoot can honestly answer — minimize/maximize/fullscreen are accepted
  and ignored (fullscreen since honoured, with its state bit: client
  fullscreen, 2026-09-22), because the core has no concept of any of them and deciding
  what they mean in a scrolling-column layout is layout design, not wire
  format. No Smithay support at the pinned rev, so hand-rolled against the
  generated wlr bindings like `output_management.rs`. Verified live against
  the real quickshell: the window list populates, a panel click focuses the
  right window, and `close` closes it.
- **[`--tty` follows DRM hotplug](docs/backlog/resolved/tty-drm-hotplug-done.md)**
  (issue #48, PR #51, 2026-09-16) — `--tty` no longer mode-sets once and
  ignores the display afterwards: a udev monitor re-runs the connector/mode
  choice on every DRM `change` event, so unplugging the connector it is on
  falls back to another instead of going black until restart, and a VM host
  resizing or rescaling its window is followed rather than scaled. Still one
  output, deliberately: a hotplug re-runs the *same* single-connector choice
  startup makes, it does not start driving a second screen. The mode is no
  longer fixed for the process's life, which narrows (but, per the
  [reconfiguration record](docs/backlog/resolved/output-management-reconfiguration-done.md),
  does not close) the deliberate refusal of client-driven output changes.
  Two paths (a new mode list, and falling back to a *different* connector)
  could not be reproduced on the QEMU dev VM and still want confirmation on
  the vfkit/laptop hardware that filed the issue, which is why #48 is
  referenced rather than closed.
- **[Screen capture for clients](docs/backlog/resolved/screencopy-capture-done.md)**
  (PR #52, 2026-09-16) — `ext-image-copy-capture-v1` with
  `ext-image-capture-source-v1`, the standard path `grim`, a screen recorder
  and a shell's workspace-overview preview read the screen through. The
  `ext-` protocol and *not* `wlr-screencopy` alongside it, the opposite call
  to PR #50 and for the same reason — measured: `grim` 1.5.0 speaks only the
  `ext-` one, and quickshell 0.3.1 speaks it too. Unlike the three protocol
  items before it, Smithay implements this one, so scoot writes handlers.
  **Output capture only**: a per-window source needs a second render target
  per session and was [its own
  item](docs/backlog/resolved/screencopy-toplevel-capture-done.md) — probed
  2026-09-17 and closed unreachable without building it (stock quickshell
  routes per-window thumbnails only to `hyprland-toplevel-export-v1`). A capture is
  parked and served from the frame tick rather than copied on request, which
  bounds it to one per session per frame and lets a repeat capture of an
  unchanged screen wait — both of which the protocol explicitly allows. The
  lock guarantee is inherited from `render()` rather than re-checked, plus
  one guard for the locked-but-not-yet-blanked window `scoot msg screenshot`
  still has. Two upstream gaps found and worked around: Smithay never sweeps
  its own session list (an unbounded, client-driven leak) and never raises
  `duplicate_frame`. `scoot msg screenshot` is unchanged.
- **[Activation / IPC focus leaves the keyboard on a clicked layer surface](docs/backlog/resolved/activation-clicked-layer-keyboard-done.md)**
  (PR #53, 2026-09-16) — the identical bug PR #50 fixed in its own `activate`,
  pre-existing in `xdg-activation-v1`'s `request_activation` and worse in
  `Request::Action` (no self-correcting unmap; the primary agent focus
  path). Both now spend the click; IPC spends it for the whole focus family
  and nothing else (pinned by a boundary test). Each new test confirmed to
  fail unfixed before being kept.
- **[`ext-workspace-v1` activation leaves the keyboard on a clicked layer surface](docs/backlog/resolved/ext-workspace-clicked-layer-keyboard-done.md)**
  (PR #54, 2026-09-16) — the client-protocol half PR #53 left out per scope,
  confirmed real rather than not-a-bug by two fail-first tests. The fix
  spends the click before `act` (refusing a locked session first), including
  on the already-active early return — which now runs the keyboard half, so
  the path agrees with IPC `FocusWorkspaceIndex` rather than differing by
  transport. The agent-facing IPC half was already fixed, so no agent loop
  silently mistypes; this closes the panel-that-stays-mapped exposure.
- **[Popup-grab keyboard holder over IPC](docs/backlog/resolved/popup-grab-focus-divergence-done.md)**
  (PR #55, 2026-09-16) — a window-focus change still does not dismiss an
  active popup grab (that deliberate guarantee stands), but `msg windows`
  now reports `popup_grab` per window, read off the held grab's root, so an
   agent can tell "window B is focused" from "B is focused but A's menu holds
   the keyboard". Additive field, no version bump; README's computer-use
   section states the agent rule and the asymmetric-`false` cases.
- **[Popup grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)**
  (PR #56, 2026-09-16) — filed from PR #44's own review: a grab now has to
  name a real, recent key/button/`enter` event delivered to the grabbing
  client (or continue its own open menu), so a background client can no
  longer take the keyboard on its own say-so. The `enter` half is the
  load-bearing subtlety — Qt passes its last-seen serial, which for a
  hover-opened menu is an enter — and the session half (nested submenus and
  same-flush menu replacements reusing the opening serial past the window)
  was measured live against real Qt and GTK menus, both proven still
  taking the keyboard. `start_drag` needed its own analysis and has since
  landed as [its own item](docs/backlog/resolved/dnd-grab-serial-validation-done.md).
- **[Screenshot encode off the event loop](docs/backlog/resolved/screenshot-encode-off-thread-resolved.md)**
  (PR #57, 2026-09-17) — the PNG encode (plus swizzle and reply framing)
  moved to a single FIFO worker; render and read-back stay on-loop. Loop
  stall per capture ~12ms → ~2ms derived, total latency unchanged by
  design. New, disclosed semantics: per-connection ordering via
  refused-with-retry, four captures max globally, `wait-idle` untouched.
- **[Toplevel screen capture, closed unreachable](docs/backlog/resolved/screencopy-toplevel-capture-done.md)**
  (PR #58, 2026-09-17) — phase-1 probe, no build: stock quickshell 0.3.1
  routes a `Toplevel` capture source exclusively to
  `hyprland-toplevel-export-v1` (version-exact source + binary inventory +
  live wire evidence), so the ext toplevel-source manager would never be
  bound. Fallback filed as shell thumbnails without a toplevel protocol
  (below). A second, wider gate found by the same probe: quickshell never
  instantiates *any* capture manager without `linux_dmabuf` feedback.
- **[Dmabuf-readiness probe: YES](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md)**
  (PR #59, 2026-09-17) — measurement only, zero executable change: a
  minimal `zwp_linux_dmabuf_v1` advertisement flips quickshell's readiness
  flag and the shipped ext output-capture path displays over shm. Honesty
  verdict independently re-derived from the protocol XML (answering
  `failed` is the designed fallback, not a violation); full blast-radius
  matrix deferred to the follow-up's acceptance, measured not reasoned.
- **[Minimal, honest `zwp_linux_dmabuf_v1`](docs/backlog/resolved/linux-dmabuf-advertisement-done.md)**
  (PR #60, 2026-09-17) — the follow-up the dmabuf-readiness probe gated:
  default feedback with the real scanout `dev_t` (`0` where no DRM node
  exists, logged once) plus the two LINEAR formats the shm pipeline serves,
  and imports answered `failed`. **⚠️ Superseded, and its central claim was
  wrong** — see [the dmabuf advertisement killed every GL
  client](docs/backlog/resolved/dmabuf-advertised-but-never-imported-done.md)
  below. This entry read `failed` as "the protocol's own non-fatal fallback,
  which is the only truthful answer a pixman/shm compositor has": true of the
  asynchronous `create`, false of `create_immed`, where it is a fatal
  `InvalidWlBuffer` — so the advertisement steered Mesa onto a path that
  killed every GL client. It was also not the only truthful answer, because
  `PixmanRenderer` could import all along. What stands from this entry is the
  measurement: on headless, no-node, `--nested` and `--tty`, quickshell's
  overview `ScreencopyView` displays over shm in all four. The gap it flagged
  in its own last sentence — no genuinely dmabuf-allocating client on the
  GPU-less dev VM, so that half wire-test proven rather than live — is
  precisely where the regression hid until real GPU hardware ran it.
- **[`scoot msg` EPIPE panic](docs/backlog/resolved/msg-client-broken-pipe-done.md)**
  (PR #62, 2026-09-17) — `msg ... | head` died with exit 101
  (`println!` panics on EPIPE; Rust ignores SIGPIPE). New `output.rs` maps
  a closed stdout to a quiet exit 0 at all six client-binary stdio sites
  (reply, screenshot bytes/summary, warning, `--help`, error report) —
  explicit per-write handling rather than a process-wide SIGPIPE
  disposition, which would have handed the co-resident compositor a
  crash-on-disconnect. A dead *socket* peer still exits 1. Fail-first
  integration test (fake server, ~1 MiB reply, reader closed); no protocol,
  IPC, or compositor changes, no README change (the error-response
  contract is untouched).
- **[`wl_data_device.start_drag` serial validation](docs/backlog/resolved/dnd-grab-serial-validation-done.md)**
  — the DnD half the popup gate (PR #56) deliberately left open. A drag now
  has to name the live implicit grab's press serial as delivered to the
  data source's own client (strict `contains`, not the popup's looser
  `contains_seen` — an `enter` can only be a live grab's serial while a
  popup grab holds the seat), so a background client can no longer convert
  someone else's held button into its own drag. Proven live against a real
  GTK drag source (`button(94)` → `start_drag(..., 94)`), with fail-first
  harness tests pinning the refusal and the dispatch floor beneath it. No
  README change (a refused drag is silent by protocol; the only
  legitimate-hit shape is a press held past the 10s window).
- **[An IME keyboard grab makes the activation gate credit a client that
  received nothing](docs/backlog/resolved/interaction-serial-ime-grab-done.md)**
  — RESOLVED 2026-09-17 as decide + pin, no behavior change: crediting the
  focused window is the intended outcome (the user is typing into it; the
  keystrokes arrive as composed text), and each of the ticket's three options
  was worse than the status quo. Pinned by two harness tests driving a real
  `grab_keyboard` (token creation for the focused window, refusal for the
  IME, creation + redemption end to end), each confirmed to fail with the
  not-recording behavior temporarily in place. No README change (no
  user-facing behavior changes).
- **[`spawn` hands its child no `XDG_ACTIVATION_TOKEN`](docs/backlog/resolved/activation-token-for-spawned-children-done.md)**
  (PR #65, 2026-09-17) — `State::spawn` (every keybinding and IPC `spawn`)
  mints via `create_external_token` and sets it on the child's `Command`,
  under both existing bounds: 30s freshness stamped at spawn, one of the
  same 64 slots swept the same way, and a full table meaning no token rather
  than an eviction (so an agent loop spawning apps cannot break interactive
  tokens). Inherited values are removed first; a spawn that never starts
  pulls its token back out. Seven harness tests around a real spawned child
  (env presence, slow-cold-start redeem, single-use, cap, sweep, failure,
  lock refusal), each behavior-changing one confirmed to fail unfixed; the
  smoke test asserts a live `foot`'s environ carries the token.
- **[IPC focus actions run a full `apply` even when nothing moves](docs/backlog/resolved/focus-action-no-op-fast-path-done.md)**
  (PR #67, 2026-09-17) — the fast path PR #54 gave `ext-workspace-v1`,
  mirrored per IPC focus variant: an already-there action spends the
  clicked-layer click and runs only the keyboard half instead of `act`.
  `FocusColumn`/`FocusWindow` steps deliberately stay on the full path
  (their no-op-ness needs column/stack positions the core doesn't expose).
  Measured live on the dev VM: sequential no-op latency ~1.6–1.8x better,
  flood throughput ~20k to ~38k req/s. No README change (purely internal
  latency, same non-difference PR #54 accepted).
- **[`ext-workspace-v1`'s cross-client check took two backend locks per
  manager, per `wl_output` bind](docs/backlog/resolved/ext-workspace-client-lookup-per-bind-done.md)**
  (PR #68, 2026-09-17) — filed from PR #50's review, fixed exactly as
  filed: the filter is now `ObjectId::same_client_as` (a lock-free field
  comparison, and the exact predicate the wayland-backend panic tests),
  copying `wlr_toplevel_output_bound`'s shape including its comment. Pinned
  by a cross-client test proven fail-first (neutered filter panics in
  `wayland-backend`, not a failed assertion). Measured ~10x on the
  predicate but ~300ns absolute — noise at session scale, so merged on
  correctness-clarity grounds, not a performance claim. No README change
  (same events, same order, same clients).
- **[A large `msg type` blocks the event
  loop](docs/backlog/resolved/msg-type-blocks-event-loop-resolved.md)** —
  RESOLVED 2026-09-17: `type` text capped at 16,384 characters per request,
  refused rather than delayed (the screenshot-rate-limit shape), sized by
  measurement on the dev VM (release `--headless`: ~2us/char plain,
  ~4.3us/char shifted, so the worst case costs ~75ms — comfortably
  sub-second). Counted in characters, not bytes; the other injected-input
  requests were audited and need no cap (`key` is one combo, `spawn` is one
  process). README's refusal list carries the new bound.
- **[Unbounded screencopy sessions and frames](docs/backlog/resolved/screencopy-session-cap-done.md)**
  (PR #70, 2026-09-17) — split as filed. Frames per session are capped: a
  `create_frame` past 16 live frames per client is refused pre-delegation
  with the protocol's own `duplicate_frame` error (per client, not per
  session — the pinned Smithay rev exposes no session identity
  pre-delegation — and a protocol error, not a silent ignore, which would
  leave an uninitialized object that panics the compositor). Sessions per
  client are closed as won't-fix on the per-surface precedent, with the
  residual cross-client walk latency stated honestly; they join the shared
  per-client accounting whenever the sibling entries land it. Fail-first
  harness tests (flood refusal, innocent-second-client, cycling drain,
  frames-outliving-session), ~1us added per `create_frame` measured noise,
   live `grim` unaffected. README's capture section carries the new bound.
- **[Screen capture's `Xrgb8888` alpha forcing, now
  conditional](docs/backlog/resolved/screencopy-xrgb-alpha-forcing-done.md)**
  (PR #71, 2026-09-17) — the ticket PR #52's review filed rather than decided
  mid-review: the `u128` opacity pass was most of what an `Xrgb8888` capture
  cost. Resolved as the ticket's third option — force the fourth byte only
  while `[appearance] background_color`'s alpha is below 1.0 (exact `< 1.0`,
  no epsilon; read per frame tick, not cached), which costs nothing in the
  default opaque configuration and keeps the guarantee where it was needed.
  Pinned byte-identical over opaque backgrounds, proven still-forcing over
  translucent ones. Measured on the dev VM: ~4.7% off a release `grim`
  capture in the default configuration. `Argb8888` untouched.
- **[An `exclusive` layer surface's own popup
  grab](docs/backlog/resolved/popup-grab-exclusive-self-dismiss-done.md)**
  — filed from PR #44's review: a launcher's own dropdown flashed open
  and instantly closed, refused and pre-empted by the very surface that
  opened it. Both checks now compare against the grab's root (the grant
  site reuses `find_popup_root_surface`; the pre-emption site reads the
  held grab's start-data focus the way PR #55 does), so a *different*
  exclusive surface still wins while the root itself never outranks its
  own menu. Fail-first harness tests for grant, pre-emption (incl. a
  nested submenu off the same root) and both unmap edges (no resurrection
  once dismissed, the root's own unmap leaving its menu up); README's
  rule-3 caveat removed now the claim is true as written. No hot-path
  benchmark (per-grab path plus one `Option` compare per focus
  derivation).
- **[Nothing bounded how many manager/list objects one client may
  bind](docs/backlog/resolved/ext-workspace-object-binding-cap-done.md)** —
  one shared per-client budget (8 binds across `ext_workspace_manager_v1`,
  `ext_foreign_toplevel_list_v1`, `zwlr_foreign_toplevel_manager_v1` and
  `zwlr_output_manager_v1`), sized at 2x the one-per-global legitimate max
  against the worst multiplier (self-created windows), refused with each
  global's own `finished` rather than a protocol error, released
  idempotently across `stop`/destroy/disconnect. Two findings on the way:
  a destructor `finished` sent inside `bind` panics wayland-backend's bind
  epilogue (so refusals defer to loop idle), and Smithay's
  `ForeignToplevelListState` exposes no bind seam — the ext list is now
  scoot-owned, mirroring its wlr twin, with byte-identical wire behavior
  pinned by the existing suite. Fail-first harness tests per global plus
  isolation, drain, floor and same-batch stop/destroy shapes; bind-storm
  before/after (64 binds × 50 windows: 64 registered before, 8 after) and
  live quickshell (1 bind per connection, window list unaffected). README
  carries the per-global refusal semantics.
- **[No upper bound on *total* shm reservation per
  client](docs/backlog/resolved/shm-pool-count-cap-done.md)** — half
  landed, half proven unimplementable at the pinned rev. The byte total the
  ticket asked for cannot be built: the new pool's id is sealed inside
  `New<WlShmPool>`, `ShmPoolUserData` exposes no size, `Pool` is
  unexported, and no server API enumerates a client's objects (all
  re-verified in source) — closed as NEEDS-UPSTREAM behind a size
  accessor. What landed instead is a per-client *live-pool count* (128,
  claimed at `create_pool`, released on destroy/disconnect): it caps
  per-connection fds and mappings (one fd minimum per live pool; ~1000
  pools exhaust a 1024-fd `RLIMIT_NOFILE` for everyone), not bytes. Sized
  from wire measurement (`foot` 2×512 MiB arenas, Qt 2×~4 MiB, idle shell
  0; ~40 reasoned for a heavy multi-window app), refused with
  `InvalidStride` like the per-pool cap. Eight fail-first harness tests
  (flood, headroom, isolation, composition ×2, bad-fd, drain, resize-pin --
  the bad-fd one pins review's deterministic leak: a probe mapping the exact
  call Smithay is about to make now gates the claim); one
  harness race found and fixed (pools read vs disconnect cleanup).
  Residual filed as its own item: Wayland connections are unbounded, so
  per-connection bounds multiply ([Wayland connection
   cap](docs/backlog/resolved/wayland-connection-cap-done.md)). README carries
  the new bound.
- **[The first click on a fresh lock screen, before the mouse has moved,
  reaches nobody](docs/backlog/resolved/session-lock-first-click-done.md)**
  (PR #76) — fixed as filed: pointer focus is re-derived on the commit
  that maps a lock surface (`new_surface`'s own derivation runs while it
  is still unmapped, so the hit test found nothing and `wl_pointer.enter`
  waited for the first mouse move, +4975ms on real `--tty`). Recognition
  is one branch per non-window, non-layer commit while unlocked plus one
  typemap probe while locked; zero new state. Four harness tests (two
  fail-first, one pinning the already-working unlock symmetry, one pinning
  no focus before mapped); window-commit throughput unchanged
  (before/after ranges overlapping). No live `--tty` re-measurement —
  stated as an environment call in the resolved record, with harness wire
  evidence instead. README's not-guaranteed bullet removed.
- **[An IME popup over a lock screen](docs/backlog/resolved/ime-popup-over-lock-screen-done.md)**
  — the locked render path gathers each current lock surface's popup tree
  and sends it frame callbacks, so a passphrase that needs an IME gets its
  candidate window at the caret. The trust call is recorded, not implicit:
  an IME is trusted with pixels for the focused field's candidate window
  (composition already routes every keystroke through it), while background
  xdg and IME popups stay hidden and callback-starved — the PR #44 password
  guarantee through the new element source, pinned by exclusion tests
  alongside the inclusion one.
- **[An already-bound `wl_output` client is never told a `--nested` resize's
  new mode is
  preferred](docs/backlog/resolved/wl-output-preferred-flag-on-late-mode-done.md)**
  — fixed as filed: `set_mode` marks the new mode preferred *before*
  `change_current_state` sends it (the ticket's batching alternative checked
  against the pinned rev and ruled out — the send is synchronous, so call
  order is what the wire sees). One fix covers `--nested` and `--tty`
  (both reach `set_mode` through `resize_output`); pinned by a fail-first
  harness test asserting the `wl_output` and `wlr-output-management`
  resize batches agree. Live `--nested` steady state confirmed under cage;
  the transient itself is harness wire evidence (external clients cannot
  win the bind-before-configure race — stated, not papered over). No
  README change (no probed client reads the bit).
- **[`[binds]` capital
  letters](docs/backlog/resolved/binds-capital-letter-done.md)** — a
  `[binds]` entry naming a capital letter (`"A" = "close"`) now binds the
  unshifted key instead of parsing, loading, and never firing. The fold
  already existed whole-name; this scopes it to single ASCII letters only
  (whole-name silently redirected `"OE"`/Œ to `"oe"`/œ — measured), warns
  naming the bind (`"A"` means plain `a`, not `shift+a`), and leaves
  `keysym_named` untouched so `scoot msg key A` keeps refusing. Fail-first
   unit tests per direction plus a permanent smoke-test section (live
   `--headless`: warn logged, injected `a` closes the window).
- **[`--width`/`--height`
  bounds](docs/backlog/resolved/width-height-bounded-done.md)**
  (PR #82, 2026-09-17) — refused
  past 65535 per axis at parse (the most DRM itself can report for a mode
  axis, with room to spare past 16K hardware), which closes the chain that
  made item 12(b)'s output-derived `min_size` limit vacuous at ~2×10⁹
  (worst case now 131070 a side at the output-scale floor, ~15000x inside
  `i32`). `Rect::inset`/`right()`/`bottom()`, `scroll_into_view` and the
  arrange on-screen test saturate instead of overflowing, each pinned by a
  fail-first test; window-count-derived products audited and left under the
  existing `MAX_GAP` disclosure.
- **[`locked` waits for vblank confirmation, not just a
  render](docs/backlog/resolved/session-lock-vblank-confirm-done.md)** —
  resolved 2026-09-17 (PR #84): under `--tty` `locked` waits for the vblank of the
  flip carrying the blanked frame (flip-sequence tracked, so a lock raced
  with an in-flight flip confirms on the next one), with a one-second
  fallback confirming anyway rather than hanging a switched-away locker;
  headless/nested confirm on render unchanged. Verified live on the dev
  VM (completions fire; three consecutive locks each vblank-confirmed).
  Found alongside, pre-existing and filed separately: a lock surface
  mapped after the confirming frame never appears on live `--tty`
  ([surface-not-drawn-live](docs/backlog/resolved/session-lock-surface-not-drawn-live-done.md))
   — CLOSED UNREPRODUCED 2026-09-17 (six green live sessions + mechanism
   audit; likely stale-socket artifact, VM swept).
- **[`scripts/smoke-test.sh` hardcoded temp
  paths](docs/backlog/resolved/smoke-test-temp-prefix-done.md)** —
  RESOLVED 2026-09-17: every socket, log, screenshot, config and scratch
  path derives from `$SMOKE_PREFIX` (unset = byte-identical legacy
  defaults, so existing `SOCKET`/`LOG` callers see no change); two
   concurrent prefixed runs proven green with fully disjoint file sets.
- **[The pointer starts at the output's origin, not
  centred](docs/backlog/resolved/pointer-starts-at-origin-done.md)** —
  RESOLVED 2026-09-17: `State::place_pointer_at_output_centre`, called
  once from `headless::init_named` so all three backends place it at the
  same init point, through `pointer_move_quietly` (no idle-timer reset,
  no focus derived, no interaction serial minted). Logical extent, so a
  scale other than 1 centres correctly; resize and VT-switch
  reactivation deliberately leave the pointer alone. Live `--tty`
  screenshot proves the cursor bitmap at the output centre; the smoke
  test's park-the-pointer workaround stays.
- **[Cursor frames while
  VT-paused](docs/backlog/resolved/cursor-frame-callback-when-paused-done.md)**
  — RESOLVED 2026-09-17: `render()` and the frame-callback loops are gated
  on DRM master, so a VT-paused `--tty` session renders nothing and wakes
  no clients (paused burn under a 600-motion driver: 10–12 → 7–8 jiffies
  release, 31–39 → 21–24 debug; frozen-framebuffer A/B proves no render
  runs while paused). Reactivation still mode-sets and repaints
  byte-identically; lock-while-paused and grab/IME-across-pause are
  construction-verified (no live clients for either on the dev VM).
- **[`single-pixel-buffer-v1`](docs/backlog/resolved/single-pixel-buffer-done.md)**
  — RESOLVED 2026-09-17: `wp_single_pixel_buffer_manager_v1` advertised
  (Smithay carries the whole protocol at the pinned rev, so this is three
  lines of state plus docs and tests, no hand-rolled handler). Five
  fail-first harness tests around a real client (RGBA read-back, 1x1
  dimensions, viewport-scaled render pinned pixel-for-pixel, manager
  destroy, attached-buffer destroy, zero shm-pool budget claimed); real
  `foot` binds the global live.
- **[`relative-pointer-unstable-v1` + pointer
  constraints](docs/backlog/resolved/relative-pointer-done.md)** —
  RESOLVED 2026-09-17: `zwp_relative_pointer_manager_v1` advertised
  (Smithay carries both halves at the pinned rev, same three-lines-plus
  shape), with the motion core feeding unclipped deltas on every focused
  move. Two corrections to the bundle entry's assumptions, both verified
  in source: the constraints global was never advertised (only the trait
  bound existed), and relative events are focus-gated per the protocol,
  not lock-gated. Sixteen fail-first harness tests pin exact vectors
  (lock hold/resume, confine hold/resume, pre-accel pairs, edge
  unclipping, per-client streams, lock-before-focus engagement, regional
  per-axis clamp and gating, session-lock round-trip freeze-through); live
  `wayland-info` + `foot` prove the advertisement, `foot` binds neither
  (it uses neither protocol). Motion-path benchmark: release unfocused
  noise, focused +~190ns/event (~1.2% of one 16ms frame per second at
   1000Hz).
- **[`presentation-time` (`wp_presentation`)](docs/backlog/resolved/presentation-time-done.md)**
  — the last child of the general-gaps bundle: precise frame-timing
  feedback for smooth video/animation clients (Smithay carries the whole
  protocol at the pinned rev, same three-lines-plus shape). Each presented
  frame is stamped with its backend's honest handoff time
  (render-complete on headless, host-commit on nested, flip-issue on tty),
  only displayed surfaces are stamped, and a rendered-but-dropped frame
  stamps nothing. Seven fail-first harness tests (sane fields, monotonic
  timestamps, supersede-discarded, unmapped/locked absence, disconnect);
  live `foot` proves the advertisement. The bundle
  (`docs/backlog/protocols/protocol-gaps-general.md`) is now fully done.
- **[A held pointer lock across a session
  lock](docs/backlog/resolved/pointer-lock-session-lock-done.md)** (PR #92,
  2026-09-17) — the password-leak-class regression PR #89 introduced: the
  lock transition's focus refresh is a zero-delta move, which a held
  constraint resolves to `Held`, so deltas, buttons and axis kept streaming
  to the game while the lock surface never got `enter` (and the cited
  chord recovery never runs while locked). The transition now deactivates
  the focus surface's held constraint first — the game sees `unlocked`,
  the persistent entry re-arms on unlock with no new request — and the test
  that pinned the freeze as intended is rewritten as an explicit behavior
  correction. Confinement needed no fix (fail-open + leave-deactivation;
  pinned), and the adjacent per-event region clone is gone (two short
  borrows; overlapping bench ranges, structurally allocation-free).
- **[A screencopy frame parked for a session lock's blank was never re-armed
  once the vblank
  confirms](docs/backlog/resolved/screencopy-parked-across-lock-confirm-done.md)**
  (PR #93, 2026-09-17) — the regression PR #84 introduced moving lock
  confirmation onto the DRM vblank: the blank tick drops the frame timer
  (a parked capture is none of its re-arm conditions) and neither confirm
  path restarted it, so the parked frame sat undelivered until some later
  commit re-armed the ticker — forever, for a lock-and-never-commit locker.
  Fixed as one `ensure_ticking()` in each confirming branch (the parked-in-
  the-re-arm-set alternative would pin the timer at 60Hz for every static-
  desktop preview); five fail-first harness tests pin the re-arm on both
  paths, the one-tick steady-state cost, multi-session delivery and the
  abandoned-locker shape. No README change (bug fix within protocol-
  permitted behavior, no user-facing surface).
- **[Noctalia re-probe: gap 1 closed in the
  field](docs/backlog/resolved/noctalia-reprobe-done.md)** (2026-09-18,
  probe + docs, no code) — the P0 lock-teardown kill the 09-14 probe
  field-confirmed now survives two full lock → PAM auth →
  `unlock_and_destroy` cycles against the real quickshell client (pid
  unchanged, live screenshots, zero kill-signature lines; the exact
  fatal destroy + null-commit sequence is on the wire with the
  connection surviving). Dismissal sweep (12 + Escape), toast, foot,
  workspace pill all live; gaps 2–8 re-check as resolved, upstream, or
  deliberate, so the ticket is resolved with nothing new filed.
- **[DMS re-probe: gap 1 stays closed in the
  field](docs/backlog/resolved/dms-reprobe-done.md)** (2026-09-18,
  probe + docs, no code) — the sibling re-probe: the exact spotlight
  teardown plus two full lock → PAM auth → `unlock_and_destroy`
  cycles against the real quickshell client (pid unchanged throughout,
  live screenshots after every step, zero kill-signature lines; both
  fatal sequences are on the wire with the connection surviving, so
  DMS's unlock path does not trip the lock-role signature Noctalia
  died on). Dismissal sweep (spotlight, clipboard, notifications
  modal, dash), toast, DankDash, two `foot` windows (one IPC-spawned,
  one DMS-launched and focused — gap 8's launch question answered);
  gaps 2–8 re-check as resolved, upstream, deliberate, or shell-side
  presentation, and the 512 live-buffer bound is untouched, so the
  ticket is resolved with nothing new filed.
- **[The live-pool cap never bounded fds or
  mappings](docs/backlog/resolved/shm-pool-cap-misses-retained-fds-done.md)**
  — RESOLVED 2026-09-17: the pool count's fd/mapping claims corrected
  everywhere (it bounds live pool objects + the address-space envelope),
  and the real fix landed as a per-client live-`wl_buffer` cap (512,
  uniform across shm/dmabuf/single-pixel factories, refused with a
  protocol error per interface, released on destroy/disconnect) that
  catches exactly the create-pool/create-buffer/destroy-pool bypass. Sized
  from wire measurement (`foot` holds 2 live buffers steady); the
   per-connection retained-fd bound is now 512 buffers + 128 pools, and
   `wayland-connection-cap`'s math is updated to say so.
- **[No cap on Wayland connection
  count](docs/backlog/resolved/wayland-connection-cap-done.md)** —
  RESOLVED 2026-09-18: verify-first re-derivation found the ticket
  undersold the bug — an `EMFILE` on the Wayland listener did not deny
  sockets, it exited the whole compositor (Smithay's source propagates
  `accept` errors out of `run()`; proven live pre-fix with prlimit).
  Fixed as a Wayland accept source that sheds like the IPC one (shared
  spare/classify primitives, EOF to the shed client, immediate recovery),
  fail-first suite plus live pre/post kill pair. The count itself is
  closed as an accepted tradeoff mirroring the IPC sibling (any usable
  count admits the two greedy connections that fill the 1024-fd table);
  a global fd/buffer ceiling is filed as its own low-priority follow-up
   since it would kill innocents for others' greed.
- **[An IME keyboard grab blocks every popup
  grab](docs/backlog/resolved/popup-grab-blocked-by-ime-grab-done.md)** —
  RESOLVED 2026-09-18: verify-first found the reverse order unhandled (an
  IME grabbing while a menu held the keyboard left it mapped with no
  keyboard, `popup_done` never sent). A live grab displaced by a foreign
  keyboard grab is now dismissed on the next dispatch, so either order ends
  with no menu while the IME holds the seat; precedence (lock > exclusive
  layer > IME grab > popup grab) pinned by two fail-first harness tests and
  documented in the README. The per-surface IME-scoping design stays open
  as filed (needs a Smithay-side design that does not exist).
- **[`xdg-toplevel-icon-v1` pixel-buffer
  icons](docs/backlog/resolved/toplevel-icon-buffers-done.md)** (PR #100,
  2026-09-18) — verify-first close, no behavior change. The
  ticket's three suspicions checked against the pinned sources all came
  back clean (the protocol leaves `release` unused by design, Smithay
  kills only the offending client with `NoBuffer`, icon buffers claim
  against the same 512 live-buffer budget as every other `wl_buffer`,
  and neither foreign-toplevel list protocol has an icon event to expose
   pixels through). Name-only IPC stands as the honest scope; four
  fail-first harness tests pin the buffer half (counting, kill
  containment, disconnect drain, cap fill).
- **[Lock blanks immediately instead of waiting for the first
  surface](docs/backlog/resolved/session-lock-blank-timing-done.md)** —
  RESOLVED 2026-09-18 (decide + pin, no behavior change): the accept →
  input-captured → first-frame-blanks → `locked`-after-blank ordering
  verified in-harness (the desktop-visible-while-input-locked window is
  the deliberate, secure direction; the niri-shape wait stays declined),
  pinned by a new combined fail-first test.
- **[Lock manager global offered to every
  client](docs/backlog/resolved/session-lock-global-restriction-done.md)** —
  CLOSED 2026-09-18 as an accepted tradeoff (no code): the protocol names
  no privilege mechanism, the pinned Smithay filter is hide-from-registry
  only, and even security-context support would not key a locker
  allow-list (it marks sandboxes, not lockers) — so the ticket's own
  `blocked:` line was wrong. Ordinary-client lock + second-client
  takeover stay pinned by the existing harness suite.
- **[Lock surfaces are per-output; scoot has one
  output](docs/backlog/resolved/session-lock-per-output-done.md)** —
  RESOLVED 2026-09-18 (PR #103, pin + document, no behavior change): first blanked
  frame on the one output confirms whatever the surface count, every
  admitted surface shares its size (new two-surface suite: first on top,
  keyboard on first, resize reaching all), `OUTPUT_ID`'s doc lists the four
  sites multi-output must revisit. Duplicate-bind admission stays open as
  its own entry.
- **[One physical output can hold unboundedly many lock surfaces if the lock
  client binds `wl_output` more than
  once](docs/backlog/resolved/session-lock-duplicate-output-done.md)** —
  RESOLVED 2026-09-18 (refuse, no Smithay patch): the admission question PR
  #103 punted here. The protocol mandates `duplicate_output` per *output*,
  Smithay enforces it per *resource*, so scoot refuses a second live
  surface for an already-covered output with code 3 on the lock, keyed on
  the physical `Output`; destroying the surface frees it for a rebuild.
   Neither probed shell ever holds two at once. Supersedes PR #103's
   two-surface composition pins; the single-surface per-output pins stand.
- **[A layer surface that commits but never attaches a buffer holds its
  exclusive zone](docs/backlog/resolved/layer-surface-bufferless-exclusive-zone-done.md)** —
  RESOLVED 2026-09-18 (decide + pin, no behavior change): the zone applies
  from the buffer-less initial commit the protocol mandates, not the first
  buffer — every healthy bar passes through that state, so windows never
  jump when the first buffer lands. Filtering would mean forking Smithay's
  `arrange` geometry (re-verified at the pinned rev: no mapped/buffer check).
  Three fail-first harness tests pin the edges (never-draws held until
  disconnect, buffer-less destroy, post-buffer zone drop); no timeout by
  design, lifetime bounded by disconnect/destroy.
- **[Niche protocol gaps, triaged per
  sub-item](docs/backlog/resolved/protocol-gaps-niche-done.md)** —
  RESOLVED 2026-09-18: all nine sub-items of the niche bundle get minimum
  honest dispositions. `wp_alpha_modifier_v1` and
  `wp_content_type_manager_v1` implemented (Smithay carries both; the
  factor blends end to end through pixman, the hint is stored and honestly
   ignored), each with fail-first harness tests; `tablet-v2` filed as its
   own entry, since implemented and recorded as
   ([tablet-v2](docs/backlog/resolved/tablet-v2-done.md)); screencopy,
   output-management and the VT-pause cursor item already resolved;
   security-context and the cursor `Vec` closed deliberate; cursor-hotspot
   offset closed needs-upstream (since overturned and fixed scoot-side —
   see below).
- **[A `present()` skipped for an in-flight flip consumes that frame's
  damage](docs/backlog/resolved/present-skip-eats-frame-damage-done.md)**
  — RESOLVED 2026-09-18: verify-first found the filed shape self-heals
  (an unwritten skip's retry re-reads a larger age, so the damage history
  still covers it — pinned, not "fixed"), while the adjacent refused-flip
  shape loses twice (the freed slot keeps a fresh age so the retry reads
  age 1 and draws nothing, and no vblank is owed so no retry is even
  triggered). The commit-failure arm now clears the failed slot's age and
   arms a bounded timer retry (3 consecutive refusals, then quiet until
   genuine damage); the in-flight path is untouched. Fail-first unit tests
   plus a Smithay-contract pin; headless render throughput unchanged
   (overlapping before/after ranges).
- **[`wl_surface.offset` on a cursor surface moves the
  hotspot](docs/backlog/resolved/cursor-surface-offset-hotspot-done.md)**
  — RESOLVED 2026-09-18: PR #106's NEEDS-UPSTREAM triage re-derived and
  overturned — Smithay core never adjusts the hotspot, but Smithay's own
  anvil does the decrement compositor-side in its shell commit hook, so
  scoot can too. `Cursor::note_surface_commit` (one call site at the end
  of `CompositorHandler::commit`) decrements by this commit's
  `buffer_delta`, saturating rather than wrapping (both operands are
  client-controlled `i32`) and gated on the active cursor surface. Four
  fail-first harness tests around a real client (offset + accumulation
  incl. negative hotspot, post-re-set, `i32::MIN` saturation, other-surface
  negative control); no README change (protocol fix, no user-facing
  surface).
- **[`Cursor::element`'s one-element fallback
  `Vec`](docs/backlog/resolved/cursor-element-per-frame-alloc-done.md)**
  — CLOSED DELIBERATE 2026-09-18 with measured numbers (PR #106's triage
  re-derived, not relayed): the fallback `vec![...]` is already the minimal
  allocation (448 bytes at capacity exactly 1, ~90–120ns net per call
  release on the dev VM), firing 0 times/sec at idle and at most ~62.5/sec
  during dirty `--tty` frames — ~7µs/s against a 16ms frame budget. The
  ticket's push-into-a-local-`Vec` shape allocates 4x the bytes (1792,
  measured: Rust's minimum non-zero capacity), and a persistent buffer's
  signature ripple costs more than it saves while `render()` keeps several
  per-frame `Vec`s regardless. Pinned by two capacity tests (fail-first
  proven against a wasteful variant); no compositor code changed, no README
  change (no user-facing surface).
- **[Parked-captures poll flake](docs/backlog/resolved/screencopy-parked-poll-flake-done.md)**
  — RESOLVED 2026-09-18 (PR #110, test-only, no production change): the filed
  mechanism was corrected by per-step serial instrumentation — the trip is
  `Ready` at an *unmoving* serial (a `delivered`-lag at park time, the
  pre-map frame consumed by an earlier tick), not an advance between park
  and poll, so serving it is correct behavior. Fixed with a quiescence wait
  plus a synchronize-then-assert retry (one retry proven max, bounded at
  three); the delivered-frames pin still fails with the re-arm neutered.
  Post-fix: 40/40 targeted + 12/12 full-binary under the oversubscription
  that tripped 6/40 + 1/12 pre-fix, full set green, smoke 17 ok. Found
  alongside and filed separately (low, load-only): an activation
  taskbar-click settle flake under the same abusive load — since resolved
  (next entry).
- **[Activation taskbar-click settle flake](docs/backlog/resolved/activation-taskbar-click-settle-flake-done.md)**
  — RESOLVED 2026-09-18 (test-only, no production change): verify-first
  found three distinct load-only trip mechanisms at the same test, not the
  one filed — settle-insufficiency (fixed with a bounded settle-until loop
  on the click's own hit test), a racing client `activate` spending the
  click before the assert (fixed with synchronous press-asserts, no dispatch
  in between), and `focus_before` read after the racing dispatch (moved
  before the release). Post-fix: 80/80 targeted + 4/4 full-binary under the
  oversubscription that tripped 2/80 pre-fix, full set green, smoke 17 ok.
  No second flake anywhere in the rounds, so nothing new filed.
- **[`--tty` quit's DRM "restore previous state"
  EPERM](docs/backlog/resolved/drm-teardown-restore-eperm-done.md)**
  — RESOLVED 2026-09-18: strace-proven to be our own teardown racing
  itself, not seatd acting first — the restore-on-drop lives behind an
  `Arc` cloned into the event loop's DRM notifier, so it runs after the
  seat socket closes and seatd revokes master (the errno is `EACCES`, not
  `EPERM`). `Tty` now pauses the device in its own `Drop`, which is
  Smithay's supported don't-touch-the-fd-on-drop, making shutdown
  deterministically quiet. Fail-first live (6/6 quits logged it pre-fix,
  0/6 post-fix) plus quit-while-paused and pause/reactivate repaint
  edges; no unit test (no `Tty` outside `--tty`), no README change (a log
  line disappearing is not a user-facing surface). (PR #111)
- **[`--tty` hotplug follows the connector across CRTCs](docs/backlog/resolved/tty-connector-switch-crtc-done.md)**
  — RESOLVED 2026-09-18: a refused in-place connector move now rebuilds the
  `DrmSurface` on another CRTC (build-first-swap-on-success, so the ticket's
  anticipated `Option<DrmSurface>` refactor proved unnecessary and `surface`
  is never `None`) instead of staying on the connector that went away; total
  failure keeps today's stay-put-and-retry. Gamma LUT length re-read per
  CRTC, live control failed only on change. Four fail-first harness tests
  pin the outcome plumbing; the switch itself is unverified live
  (single-CRTC dev VM) and the legacy blind-probe limit is stated in the
  record.
- **[`--tty`'s `O_CLOEXEC` request was a no-op at the libseat
  layer](docs/backlog/resolved/tty-o-cloexec-noop-done.md)**
  — RESOLVED 2026-09-18 (PR #114): the dead flag is removed
   (`LibSeatSession::open` takes `_flags` at the pinned rev, re-verified in
   source), and the guarantee it appeared to give is pinned by a real-spawn
   test — a close-on-exec marker never reaches the child, while a marker
   without the bit provably does (kept as the test's permanent positive
   control, which is also the fail-first record). Residual filed separately:
   a per-source close-on-exec audit (low).
- **[Same-VT no-op VT-switch
  warning](docs/backlog/resolved/vt-switch-same-vt-warning-done.md)**
  — RESOLVED 2026-09-18: a same-VT `change_vt` no longer sends the hedged
  IPC `Warning` — it is answered as a quiet `IgnoredSameVt` (plain `Ok`)
  without calling libseat, decided per call against the kernel's live
  displayed VT (`/sys/class/tty/tty0/active`) rather than the ticket's
  anticipated init-time `Tty` field, so there is no new session-state
  semantics to audit. Unknown display falls back to asking (the cosmetic
  false positive, never a skipped real switch). Live away/back still
  pauses, reactivates with modeset and repaints byte-identically; the
  paused-retry `IgnoredPaused` guard is unregressed.
- **[`flake.nix` description drift](docs/backlog/resolved/flake-description-drift-done.md)**
  — RESOLVED 2026-09-18 (flake metadata only, no code): top-level
  `description` names both (compositor on Linux, `scoot msg` client on
  macOS); `meta.description` is per system (Linux reads
  `crates/scoot/Cargo.toml`, Darwin names the client), so `nix search` no
  longer advertises a compositor macOS never runs. A fully single-sourced
  fix is loader-impossible (top level must be a syntactic attrset,
  `description` a string literal — both proven live), so the top level stays
  one literal by fiat. Sibling packaging tickets untouched.
- **[Nix `src` fileset](docs/backlog/resolved/nix-src-fileset-done.md)**
  — RESOLVED 2026-09-18 (`flake.nix` only, no code): `src = self` is now
  a `lib.fileset` union of `Cargo.toml`, `Cargo.lock`, `crates/`, after
  re-verifying nothing the build reads lives outside it. The old
  working-tree copy also dragged `target/` and `.git` along, so `src`
  drops from ~1.1 GB to ~3.3 MB in-store, and doc/target edits no longer
  move the derivation. Proven by `nix build` (aarch64-darwin); the
  `x86_64-darwin` sibling ticket stays open and untouched.
- **[`x86_64-darwin` in `systems`](docs/backlog/resolved/flake-x86-darwin-system-done.md)**
  — RESOLVED 2026-09-18 as deliberate exclusion (`flake.nix` systems
  list plus one README sentence, no code): the pinned nixpkgs (26.11)
  throws at `legacyPackages.x86_64-darwin` before any per-system
  definition is reached, so the system is dropped rather than repinning
  the whole tree to 26.05; the Darwin client path is one
  arch-independent `cfg(not(target_os = "linux"))`, so `cargo build`
  from source stays open on Intel Macs. `nix flake check --all-systems`
  is green on the three remaining systems.
- **[The dmabuf advertisement killed every GL client](docs/backlog/resolved/dmabuf-advertised-but-never-imported-done.md)**
  — RESOLVED 2026-09-18 (branch `fix/dmabuf-real-import`), regression from
  `599be4e` found live on Asahi: scoot advertised `zwp_linux_dmabuf_v1`
  and answered every import `failed`, which for `create_immed` is a fatal
  `InvalidWlBuffer` — so Mesa took the advertised dmabuf path over `wl_shm`
  and died, and noctalia v5 could not start a session at all. scoot now
  really imports: `PixmanRenderer` `mmap`s a single-plane LINEAR dmabuf on
  the CPU, so a GPU-rendering client works with no GPU on the compositor
  side and no `LIBGL_ALWAYS_SOFTWARE=1`. Four cross-site consequences came
  with it — the per-client buffer cap now claims on the async `create` path
  (and hands the unit back on a refusal, the one refusal a client survives),
  the feedback table is pinned by test to what the renderer can actually
  import, `main_device` names the render node rather than a primary node,
  and scoot issues the per-commit `DMA_BUF_IOCTL_SYNC` bracket the pinned
  rev does not (it syncs once at import and never again).
- **[Drawing-tablet input](docs/backlog/resolved/tablet-v2-done.md)** —
  RESOLVED 2026-09-18: `zwp_tablet_manager_v2` (version 1, Smithay's
  maximum at the pinned rev) advertised honestly, with the input epic the
  niche bundle refused to fake -- libinput tool-event plumbing, a
  `TabletSeat` driving tool lifetimes, and tool focus/cursor routed
  through the existing pointer/click paths (a pen moves the cursor, a tap
  clicks like a mouse click, pressure rides the axis events, barrel
  buttons are tool-only). Pads/strips/rings stay deferred upstream
  (Smithay carries no pad objects at the pinned rev). Eight fail-first
  harness tests with synthetic tool events (6 fail unadvertised), live
  `wayland-info` advertisement; no tablet-tool hardware on the dev VM
  (its QEMU tablet is pointer-only), so the libinput arms are
  review-verified and real tool types untested -- stated in the record.
- **[A global fd/buffer ceiling across Wayland connections](docs/backlog/resolved/wayland-global-fd-ceiling-done.md)** —
  RESOLVED 2026-09-18: the connection-cap verdict's deferred half -- a
  compositor-wide pressure ceiling with a designed refusal form. Past 128
  free fds newcomers shed (Wayland EOF, IPC refused with a reason naming
  the pressure), and past-grace creations (128 live buffers, 64 live
  pools) are refused with the interfaces' own protocol errors, so the kill
  always lands on a contributor and never an innocent bar. Thirteen
  fail-first tests; proven live with a 374-connection horde (52 sheds,
  foot untouched, immediate recovery on drain).
- **[Pin the fd-pressure grace conjunction's boundaries](docs/backlog/resolved/fd-pressure-grace-boundary-pins-done.md)** —
  RESOLVED 2026-09-18 (pin, no behavior change — filed from PR #123
  review): `pressure_refusal`'s pure conjunction is now
  `pressure_refusal_for`, pinned at both operators and both graces (129th
  buffer / 65th pool first to refuse; either half alone passes, so the
  kill always lands on a contributor). Five fail-first tests (each
  operator neuter proven to fail them, the `||` neuter additionally
  tripping five pre-existing flood canaries); the ceiling record's 400
  stands as "two at grace", the `>`-permits-grace+1 maximum of 404 now
  stated in both module docs.
- **[Every compositor fd carries close-on-exec](docs/backlog/resolved/spawn-fd-cloexec-audit-done.md)** —
  RESOLVED 2026-09-18 (audit + test-only, no production change): the
  per-source close-on-exec audit PR #114 filed — every fd source verified
  at creation (event-loop, channel, listener/spare/accepted-socket, seatd,
  shm/dmabuf receipt, sealed memfds, libinput/udev, transient file reads),
  measured live on headless and `--tty` with an IPC-spawned child
  inheriting nothing; the spawn pin now covers socket, `try_clone` and
  eventfd markers, each proven sensitive by neutering.
- **Both Apple-Silicon-blocked questions, answered on the reporter's own
  hardware** (2026-09-18, docs-only — Apple M2 `apple,t8112`, NixOS aarch64;
  runbook and evidence in [`Asahi.md`](Asahi.md)).
  [The `--tty` DRM device search](docs/backlog/resolved/tty-gpu-config-key-done.md)
  **works there unattended** — it rejects the `asahi` render node
  (`os error 95`) and drives the `apple-drm` display controller, in a
  daily-driven session with no `--gpu` and no `[tty] gpu` — so the residual
  that key carried is closed and `--gpu` is *not* the first thing to reach
  for on Apple Silicon.
  [Ghostty at `[output] scale = 1.5`](docs/backlog/resolved/ghostty-fails-at-1-5-done.md)
  is **RESOLVED as not reproducible**: refuted under `--headless` at both
  scales on the real AGX GPU and then in the original `--tty`-on-`eDP-1`
  configuration itself. The cause was never captured, so it closes with a
  revisit condition rather than a diagnosis.
  Still open and still needing that machine: issue #48's connector fallback,
  which needs an external display (this one has only `eDP-1`).
- **[The spawn close-on-exec pin asserted fd numbers, not fd
  identity](docs/backlog/resolved/spawn-fd-number-identity-flake-done.md)**
  — RESOLVED 2026-09-19 (test-only, no production change): CI run
  35460052576 went red on a docs-only PR at
  `a_spawned_child_inherits_no_close_on_exec_fd`, in the `cargo test` step
  and not the nextest one — the asymmetry that step exists for. A raw fd
  number names no particular open file description, so the assertion was
  wrong both ways: `execve` frees the close-on-exec markers' numbers and the
  child's own fds take the lowest free ones (a marker at fd 3 comes back as
  the child's fd 3), while from the other side a neighbour test's plain fd
  in the shared process sits on a marker's number. Markers are now
  identified — file markers by a unique canonical path, the sockets and the
  `try_clone` by `socket:[inode]` from `fstat`, the eventfd by a distinctive
  `eventfd-count` (the "no eventfd at all" superset check was rejected: one
  foreign non-close-on-exec eventfd would re-open the same flake). The child
  also reads `/proc/$$/fd`, not `/proc/self/fd` — which was reporting `ls`'s
  own table — and renames its listing into place, because `read_probe`
  returns on the first non-empty read. Fail-first made deterministic by
  parking an unrelated fd on the marker's number (pre-fix red every time,
  post-fix green), and each marker re-proven sensitive by neutering. Found
  alongside and filed separately (low, load-only, and reproduced at pristine
  `main`): the icon live-buffer-budget flood is
  [refused by process-wide fd pressure](docs/backlog/resolved/icon-buffer-budget-fd-pressure-flake-done.md)
  when a neighbour test holds fds at the wrong moment — RESOLVED
  2026-09-20 (test-only: raised ceiling with verified headroom, budget
  refusal pinned by message, 512-count pinned by a deterministic
  fd-free test).

## What's next

The backlog is the source of truth for what to pick up; this is the current
read of it, not a commitment. Both shell probes are now resolved
outright — [DMS gaps](docs/backlog/resolved/dms-reprobe-done.md)
(re-probed 2026-09-18: the spotlight teardown plus two full lock →
auth → unlock cycles survive in the field, DMS's unlock path doesn't
trip the lock-role signature, everything else re-checks as
resolved/upstream/deliberate/shell-side, nothing new filed) and the
[Noctalia probe](docs/backlog/resolved/noctalia-reprobe-done.md)
(re-probed 2026-09-18: the lock-teardown kill survives two full
lock → auth → unlock cycles in the field, everything else re-checks as
resolved/upstream/deliberate, nothing new filed). What's left of both
was already filed individually under `docs/backlog/protocols/` at
medium priority — the effective top of what's actually open.

1. **Medium-priority protocol gaps**, mostly what's left of the DMS/Noctalia
   probes (see "Shell enablement" below for their recommended order):
    [screencopy's toplevel half](docs/backlog/resolved/screencopy-toplevel-capture-done.md)
    is CLOSED UNREACHABLE (phase-1 probe, no build — stock quickshell routes
    per-window thumbnails only to `hyprland-toplevel-export-v1`); the
    thumbnail fallback
    [shell thumbnails without a toplevel protocol](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md)
    is CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build — the overview
    preview lights up on shipped `main`, the screen-source + clip-crop
    recipe is proven live pixel-for-pixel, and DMS's `TileItem.qml`
    hard-requires a `Toplevel` source, so the shells must change; current
    Noctalia has no per-window live-thumbnail view at all).
   [Popup grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)
   is also done — a grab now has to name a real, recent key/button/enter
   event delivered to the grabbing client (or continue its own open menu).
   It was filed from PR #44's own review alongside
   [a window-focus change doesn't dismiss an active popup
   grab](docs/backlog/resolved/popup-grab-focus-divergence-done.md), which
   shipped as an IPC `popup_grab` field in PR #55.
2. **[IPC connection cap and the half-closed-connection
   leak](docs/backlog/resolved/ipc-connection-cap-resolved.md)** — RESOLVED
   2026-09-16: 64 concurrent connections, refused with a reason past that,
   a write-stall deadline that drops a peer which has stopped reading (the
   leak a cap alone could not free), and — found by the review of that — a
   cap on how long a `wait-idle` may park a connection, without which 64
   parked waiters from a since-crashed agent wedged the whole control
   channel. The third concern that entry bundled — [capture and encode on
   the event-loop
   thread](docs/backlog/resolved/screenshot-encode-off-thread-resolved.md) —
   is RESOLVED 2026-09-17: the PNG encode (plus swizzle and reply framing)
   moved to a single FIFO worker, with per-connection ordering held by
   refusing anything else on a connection with a capture in flight, a bound
   of four captures across the compositor, and `wait-idle` untouched. The
   first of the two adjacent things found while reviewing the cap work --
   [the accept loop and
   `EMFILE`](docs/backlog/resolved/accept-loop-emfile-resolved.md) -- is
   RESOLVED 2026-09-17: the loop matches on the error kind
   (`WouldBlock`/`Interrupted` end it quietly as before, anything else is
   loud), fd exhaustion sheds one pending connection per turn through a spare
   fd (no back-off, no added delay once pressure lifts), and a dead listener
   deregisters rather than spinning. The other -- [what a
   shared connection table costs an innocent
   client](docs/backlog/resolved/connection-cap-denies-the-same-user-done.md) -- is
   closed as an accepted tradeoff (no workload has hit it; kept as the landing spot).
3. Small, unblocked low-priority fixes: [`msg key` modifier
    resolution](docs/backlog/resolved/msg-key-modifier-resolution-done.md)
    (PR #83, resolved — modifiers resolve through the keymap probe like
    `type_text`; toggle-option layouts accept combos, `msg key A` still
    refuses),
   [`[binds]` capital
   letters](docs/backlog/resolved/binds-capital-letter-done.md) (resolved —
   single-ASCII-letter fold at parse time, warn on normalize, `msg key A`
   still refuses),
   [`--width/--height`
   bounds](docs/backlog/resolved/width-height-bounded-done.md) (resolved —
   refused past 65535 per axis at parse, layout siblings saturate),
   [`msg type` dead keys / compose](docs/backlog/resolved/msg-type-dead-keys-compose-done.md)
   (PR #121, resolved — 2-key dead-led sequences typed person-style
   through the session-locale table, per-char atomic, prefix-typed;
   inactive groups, lock/latch, `Multi_key` and plain-`us` `é` stay loud
   refusals).
4. [Split the CLI out into
   `scootctl`](docs/backlog/resolved/rename-flex-family-done.md) — the rename half of
   that entry LANDED 2026-09-18 (PR #128): `flexwm` → **`scoot`** across
   crates, binary,
   socket (`scoot.sock`/`$SCOOT_SOCKET`), config path (`~/.config/scoot/`),
   flake and docs, with no fallback to the old
   names; `flex` was dropped for a crates.io and GNU-flex collision, and the
   repo moved to `scoot-sh/scoot`. `docs/backlog/resolved/` and the numbered
   `docs/roadmap/` files keep the old name on purpose — they record evidence
   (nix store paths, typed strings, screenshot paths) a rename would falsify.
   What remains open here is the `scootctl` crate split, which wants its own
   design pass.

## Shell enablement (DMS / Noctalia probes, 2026-09-14)

Two Quickshell shells were probed end to end
([DMS gaps](docs/backlog/resolved/dms-reprobe-done.md),
[Noctalia results](docs/backlog/resolved/noctalia-reprobe-done.md) —
both re-probed 2026-09-18 and resolved: every overlay dismissal and
lock → auth → unlock cycle survives in the field, everything else
re-checks as resolved/upstream/deliberate/shell-side) after
landing gamma-control, both data-controls and primary selection
(PR #32) plus two destroy-teardown kill fixes (PRs #34, #36). Both
shells render fully; Noctalia is the better target (generic
`ext-workspace-v1` backend, richer IPC). Remaining gaps, in the
probes' recommended order:

1. [Popup input](docs/backlog/resolved/xdg-popup-input-resolved.md)
   — RESOLVED 2026-09-16: `xdg_popup.grab` is honoured, so a menu takes
   the keyboard, Escape/arrow keys/typeahead reach it, and clicking
   outside dismisses it (the only thing that ever sends `popup_done`).
   The focus precedence it sits at is stated once in `popup.rs`: the
   session lock and an `exclusive` layer surface both pre-empt a grab
   (dismissing the menu rather than silently outranking it), and the
   grab wins over the focused window and over a click-focused
   `on_demand` layer surface — so a launcher stays typeable and a bar's
   own dropdown is not dismissed by the bar that opened it.
   Layer-parented popups turned out already to work; implementing the
   handler the entry asked for would have put two tree nodes on one
   surface (measured). Follow-up filed, and since resolved:
   [grab serial validation](docs/backlog/resolved/popup-grab-serial-validation-done.md)
   — a grab now names a real, recent key/button/enter event or continues
   its own open menu, proven live against real Qt and GTK menus.
   Neither probed shell exercises any of this directly: DMS and Noctalia
   both route their own menus through layer surfaces, with zero
   `xdg_popup` wire traffic in either probe — the first real client for
   this work is an ordinary GTK/Qt toolkit menu, not DMS or Noctalia.
2. [Idle](docs/backlog/resolved/ext-idle-notify-resolved.md) — RESOLVED
   2026-09-15: auto-lock's trigger exists and is field-proven with real
   swayidle; what remains is the user's own daemon config, not compositor
   work.
3. [`foreign-toplevel`](docs/backlog/resolved/foreign-toplevel-list-done.md)
   — RESOLVED 2026-09-16, in two halves. The successor the entry told its
   reader to check for exists (`ext-foreign-toplevel-list-v1`, PR #47) and is
   implemented, so a standards-following taskbar or switcher can list windows
   and map each one back to its `scoot msg windows` id — but it did *not*
   unlock these two shells: quickshell 0.3.1 is offered the global and never
   binds it (measured, wire-level), because its `ToplevelManager` is a wlr
   client. [`wlr-foreign-toplevel-management`](docs/backlog/resolved/wlr-foreign-toplevel-management-done.md)
   (PR #50) closes that half — list, click-to-focus and close, all verified
   live against the real quickshell. Minimise/maximise remain no-ops, since
   scoot's core has no concept of either.
4. [`output-management`](docs/backlog/resolved/output-management-read-only-done.md)
   — HALF-RESOLVED 2026-09-16 (PR #49): the query half a shell's display page
   binds is implemented (`wlr-output-management-unstable-v1` v4 — no `ext-`
   successor exists at the pinned rev, so the standing preference had nothing
   to prefer). Reconfiguration is deliberately refused and
   [closed as an accepted tradeoff](docs/backlog/resolved/output-management-reconfiguration-done.md)
   (2026-09-18, no code — no authorization concept in the protocol, per-backend
   honesty gaps, no shell demand): `failed` stays the honest answer.
5. [`screencopy / image-capture`](docs/backlog/resolved/screencopy-capture-done.md)
    — HALF-RESOLVED 2026-09-16 (PR #52): `ext-image-copy-capture-v1` with
    `ext-image-capture-source-v1` for **output** capture, which is the
    workspace-overview preview half. The per-window thumbnail half was
    [probed and closed unreachable](docs/backlog/resolved/screencopy-toplevel-capture-done.md)
    rather than built (stock quickshell 0.3.1 speaks only
    `hyprland-toplevel-export-v1` for a `Toplevel` source); its fallback,
    [shell thumbnails without a toplevel protocol](docs/backlog/resolved/screencopy-shell-thumbnails-fallback-done.md),
    is CLOSED NEEDS-UPSTREAM 2026-09-17 (measured, no build): the overview
    preview lights up with real pixels on shipped `main` (PR #60's
    advertisement, re-driven), and the screen-source + clip-crop recipe for
    per-window thumbnails is proven live pixel-for-pixel — but DMS's
    `TileItem.qml` hard-requires a `Toplevel` source and current Noctalia
    has no per-window live-thumbnail view, so the change belongs upstream.
    IPC screenshots stay regardless.
6. DMS unlock-path re-probe — done 2026-09-14 (see the re-probe note
   in the DMS gaps entry): lock → auth → unlock teardown survives on
   current `main`, and so does a spotlight open/Escape-dismiss cycle.
   Both shells' destroy kills are now proven fixed, not inferred.
   Re-probed 2026-09-18 and resolved outright
   ([record](docs/backlog/resolved/dms-reprobe-done.md)): the exact
   spotlight teardown plus two full lock → auth → unlock cycles
   survive, DMS's unlock path doesn't trip the lock-role signature,
   gap 8's launch question is answered (DMS-spawned app maps
   focused), and "No compositor detected" stands recorded as
   upstream.

Small follow-ups already filed alongside:
[`gamma-control`](docs/backlog/resolved/gamma-control-followups-done.md)
(resolved, PR #78),
[`layer-destroy`](docs/backlog/resolved/layer-destroy-review-followup-done.md)
(resolved, PR #77).

Item 6 (the GPU pipeline) remains the one *ordered* milestone still open, now
through all four stages: the renderer seam (PR #129), the GLES pipeline
behind `--renderer` (PR #130), `DrmCompositor` scanout for `--tty`
(PRs #133 and #135), and the renderer-derived `zwp_linux_dmabuf_v1`
tranche (PR #147) — all merged, none of it yet run on a real GPU. It had
never been ahead of
the daily-drivability and correctness work the backlog keeps producing, and
that trade can be revisited at any time — stage 1 was picked up when it was
because it is a pure refactor with no behaviour change, so it cost the backlog
nothing and removed the one thing that made every later stage unreviewable,
and stages 2 and 3 are opt-in and off by default for the same reason —
stage 3 doubly so, behind both a Cargo feature and a flag. Stage 4 is the one
that is *not* opt-in, because it cannot be: what a compositor promises its
dma-buf clients is a promise every session makes, and the point of the stage
is that the promise stops being one renderer's answer given on another
renderer's behalf.

## History

The pre-split `ROADMAP.md` was 4,314 lines: ~2,817 of milestone write-ups
(17 of 18 already done) in front of ~1,489 of backlog. It was split on
2026-09-14 so the live work is readable without scrolling past an archive.
No entry's prose was rewritten in the move — each file is the original
entry with only its list marker and continuation indent removed, plus
frontmatter. The split was verified by re-deriving every body from
`ROADMAP.md`'s git history and diffing: zero differences across all 21
milestone and 57 backlog files.
