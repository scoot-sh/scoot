---
title: "Screen capture, toplevel half — CLOSED UNREACHABLE: stock quickshell never binds `ext_foreign_toplevel_image_capture_source_manager_v1`."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Screen capture, toplevel half — CLOSED UNREACHABLE (phase-1 probe, no build).

## The entry as filed

> Split out of
> [`screencopy-capture-done.md`](./screencopy-capture-done.md) when the
> output half shipped, the same way PR #49 split output-management
> reconfiguration and PR #47 split the wlr foreign-toplevel list — so what
> shipped and what did not are two separate records rather than one half-true
> one.
>
> flexwm now advertises `ext_image_copy_capture_manager_v1` and
> `ext_output_image_capture_source_manager_v1`, so a client can capture the
> **output**. It does **not** advertise
> `ext_foreign_toplevel_image_capture_source_manager_v1`, so a client cannot
> capture one **window** on its own. That is the half a launcher's per-window
> thumbnails need (DMS `TileItem.qml`'s `ScreencopyView`, Noctalia's
> equivalent); the workspace-overview preview is the output half and works
> today.
>
> Deliberately not advertised-and-refused: a client that can create a source
> and is then told `stopped` has to discover the refusal at runtime, where one
> that never sees the global takes its own fallback path immediately.
>
> Output capture reads a region back out of the one framebuffer the compositor
> already drew. A toplevel capture cannot: the protocol asks for *that window's*
> content, not the part of the screen it happens to occupy (which may be
> obscured, clipped by the output edge, or scrolled off it entirely in a
> scrolling-column layout). So it needs, in rough order of size: a second
> render target per session; a constraint-refresh path of its own driven by
> window resizes; mid-session teardown in `remove_window`; a stated answer for
> a locked session (refuse outright while locked); and a bridge from
> `ForeignToplevelHandle` to `WindowId` via the `<generation>-<window id>`
> identifier.
>
> **One measurement to take first:** `create_source` takes an
> `ext_foreign_toplevel_handle_v1`, i.e. a handle from
> `ext-foreign-toplevel-list-v1`. PR #50 measured that stock quickshell 0.3.1 —
> the build both DMS and Noctalia run on — is offered that global and never
> binds it, binding `zwlr_foreign_toplevel_manager_v1` instead. Its binary does
> carry both sets of symbols, so it may bind the `ext-` list specifically for
> the capture path even though its `ToplevelManager` does not. **Measure that
> before building anything.**

## Phase-1 verdict: NO — do not build

Nothing binds it across real thumbnail sessions, and version-exact upstream
source shows there is no code path that could: quickshell 0.3.1 routes a
`Toplevel` capture source **exclusively** to `hyprland-toplevel-export-v1`.
The per-window source manager is unreachable for the motivating client, so
none of the five build items above were started. That is a successful outcome
of the gated ticket, not a failure — the gate existed to prevent exactly this
build.

The fallback the ticket names — per-window thumbnails as a region capture out
of the output (`wlr-screencopy` has no toplevel source either) — is filed as
[`../protocols/screencopy-shell-thumbnails-fallback.md`](../protocols/screencopy-shell-thumbnails-fallback.md),
with the second measurement this probe turned up (quickshell's buffer-readiness
gate, below) as its own "measure first".

## What was measured, and how

Three legs, each recorded raw. All live work on the dev VM
(`ssh -p 2222 dev@localhost`, NixOS/QEMU), building through the 9p mount at
`/mnt/flexwm` with `CARGO_TARGET_DIR=/var/cargo-target`. Both VMs were up
before this work and neither was started, stopped or restarted by it. Every
`flexwm` process started here was killed by its probe script's own `trap`
(`pgrep` confirmed clean afterwards); the `--tty` seat was never claimed —
every run here is `--headless`.

### Leg 1 — live: the global advertised, a real thumbnail session, zero binds

A probe build advertised `ext_foreign_toplevel_image_capture_source_manager_v1`
with Smithay's real `ToplevelCaptureSourceState`/`ToplevelCaptureSourceHandler`
at the pinned rev and **no render target behind it**: `toplevel_source_created`
logged `PROBE: client created an ext toplevel capture source` and stored
nothing, and such a source reaches `capture_constraints` with no output user
data, i.e. `None`, i.e. the session is `stopped`. That advertised-but-stopped
shape is the refused form the output half deliberately avoids shipping — this
was measurement scaffolding, reverted before anything was committed (the exact
diff is kept below so the measurement is reproducible). No feature code was
written: no render target, no constraint refresh, no teardown, no bridge.

The probe diff (base commit `4a70798`, uncommitted working tree; `git diff`
across `crates/flexwm/src/compositor/screencopy.rs`, 47 insertions,
8 deletions):

```diff
 use smithay::reexports::wayland_server::protocol::wl_shm;
 use smithay::utils::{Buffer as BufferCoords, Clock, IsAlive, Monotonic, Rectangle, Transform};
+use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
 use smithay::wayland::image_capture_source::{
     ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
-    OutputCaptureSourceState,
+    OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
 };
```

plus a `toplevel_sources: ToplevelCaptureSourceState` field on `Screencopy`
(constructed in `new()` via `ToplevelCaptureSourceState::new::<State>(dh)`),
and:

```rust
impl ToplevelCaptureSourceHandler for State {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.screencopy.toplevel_sources
    }

    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: ForeignToplevelHandle,
    ) {
        tracing::warn!(
            source_id = source.id(),
            "PROBE: client created an ext toplevel capture source"
        );
        let _ = toplevel;
    }
}
```

Force-clean built (`cargo clean -p flexwm && cargo build -p flexwm`: real
`Compiling flexwm` line, `Finished in 28.86s` — not the sub-2s 9p phantom)
and copied out of the shared target dir before any probe ran, so no other
agent's tree could substitute the binary under it:

```
/var/tmp/flexwm-toplevel-probe   (probe build, global advertised)
/home/dev/screencopy-evidence/flexwm-74557ec   (PR #52's output-half build: control, global absent)
```

The client is the exact build PR #50 measured:

```
Quickshell 0.3.1 (revision tag-v0.3.1, distributed by Nixpkgs)
store path: /nix/store/hnw9kk48z8jqp0pqha5gwnpyawpcxq34-quickshell-0.3.1
bin/.quickshell-wrapped sha256:
784914e56fc5d112ebedfa0475c35a59bf1d3b0af6300c3665244b94e46b78b4
```

It was driven with a QML session shaped like the motivating clients — a
`ScreencopyView` bound to a live `ToplevelManager` toplevel with
`captureFrame()` called (the DMS `TileItem` shape), plus a `ScreencopyView`
over `Quickshell.screens[0]` (the overview shape) as a positive control —
inside a real `PanelWindow`, with two real `foot` windows mapped, for 20s,
under `WAYLAND_DEBUG=1`. Full drive script kept on the dev VM as
`/var/tmp/qs-toplevel-probe.sh` (takes the flexwm binary and a tag:
`/var/tmp/qs-toplevel-probe.sh /var/tmp/flexwm-toplevel-probe probe`); the
QML it drives:

```qml
import Quickshell
import Quickshell.Wayland
import QtQuick

ShellRoot {
    id: root
    function dump(tag) {
        const list = ToplevelManager.toplevels.values;
        console.log("QS:", tag, "count =", list.length);
        for (var i = 0; i < list.length; i++)
            console.log("QS:   [" + i + "] appId=" + list[i].appId + " title=" + list[i].title);
    }
    Component.onCompleted: dump("initial")
    Connections {
        target: ToplevelManager.toplevels
        function onValuesChanged() { root.dump("changed") }
    }
    PanelWindow {
        anchors { top: true; left: true; right: true }
        implicitHeight: 200
        color: "#303030"
        ScreencopyView {
            id: winview
            anchors { left: parent.left; top: parent.top }
            width: 300; height: 180
            live: false
            onHasContentChanged: console.log("QS: winview hasContent=" + hasContent + " sourceSize=" + sourceSize)
        }
        ScreencopyView {
            id: outview
            anchors { right: parent.right; top: parent.top }
            width: 300; height: 180
            live: false
            captureSource: Quickshell.screens[0]
            onHasContentChanged: console.log("QS: outview hasContent=" + hasContent + " sourceSize=" + sourceSize)
        }
        Timer {
            interval: 3000; running: true; repeat: false
            onTriggered: {
                const list = ToplevelManager.toplevels.values;
                console.log("QS: binding winview, toplevel count=" + list.length);
                if (list.length > 0) {
                    winview.captureSource = list[0];
                    console.log("QS: winview captureSource set to " + list[0].title + ", captureFrame()");
                    winview.captureFrame();
                }
                console.log("QS: outview captureFrame()");
                outview.captureFrame();
            }
        }
    }
    Timer { interval: 16000; running: true; repeat: false; onTriggered: root.dump("final") }
}
```

### The wire evidence (probe run, `/tmp/qs-tlprobe-wire-probe.log`)

The global is advertised — the probe binary is proven to be the one running:

```
{Default Queue} wl_registry#2.global(10, "ext_foreign_toplevel_image_capture_source_manager_v1", 1)
```

(seen 5x across the main + three mesa registries; `wayland-info` against the
same session concurs: `ext_foreign_toplevel_image_capture_source_manager_v1`,
version 1, name 10, alongside the `ext-` list, the wlr manager, and both
output-capture globals.)

quickshell's `ToplevelManager` populated normally over the wlr protocol, and
both views were driven:

```
DEBUG qml: QS: changed count = 2
DEBUG qml: QS:   [0] appId=foot title=alpha
DEBUG qml: QS:   [1] appId=foot title=beta
DEBUG qml: QS: binding winview, toplevel count=2
DEBUG qml: QS: winview captureSource set to alpha, captureFrame()
DEBUG qml: QS: outview captureFrame()
DEBUG qml: QS: final count = 2
```

And then, across the whole 20s session:

- `grep -a "bind.*ext_foreign_toplevel_image" /tmp/qs-tlprobe-wire-probe.log`
  → **empty**. The advertised global was never bound.
- `grep -a "create_source" /tmp/qs-tlprobe-wire-probe.log` → **empty**. No
  toplevel source was ever created.
- `grep -a "PROBE" /tmp/flexwm-tlprobe-probe.log` → **empty**. Nothing reached
  the server-side handler either.
- `grep -a "hyprland" /tmp/qs-tlprobe-wire-probe.log` → **0 lines**: the
  compositor never advertises `hyprland_toplevel_export_manager_v1`, so there
  is nothing to show — recorded for completeness, not as a finding.
- The bind set is byte-identical with the global advertised and without it:
  the same 18 globals (`wl_compositor`, `wl_subcompositor`,
  `wp_fractional_scale_manager_v1`, `wp_cursor_shape_manager_v1`,
  `wp_viewporter`, `wl_shm`, `zxdg_output_manager_v1`,
  `wl_data_device_manager`, `zwp_primary_selection_device_manager_v1`,
  `zwp_text_input_manager_v3`, `wl_seat`, `wl_output`, `xdg_wm_base`,
  `zxdg_decoration_manager_v1`, `xdg_activation_v1`,
  `xdg_toplevel_icon_manager_v1`, `zwlr_foreign_toplevel_manager_v1`,
  `zwlr_layer_shell_v1`) in the probe run and in the control run against
  `flexwm-74557ec` (`diff` of the two sorted bind lists: no output).
  Advertising the global changed nothing observable about the client.

### Leg 2 — static: there is no code path that could have bound it

Version-exact quickshell source (`git.outfoxxed.me/quickshell/quickshell`,
tag `v0.3.1` — the revision string the nix build itself reports —
`src/wayland/screencopy/manager.cpp`, `ScreencopyManager::createContext`):

```cpp
ScreencopyContext* ScreencopyManager::createContext(QObject* object, bool paintCursors) {
	if (auto* screen = qobject_cast<QuickshellScreenInfo*>(object)) {
#if SCREENCOPY_ICC
		// ... icc::IccOutputSourceManager::captureOutput ...
#endif
#if SCREENCOPY_WLR
		// ... wlr::WlrScreencopyManager::captureOutput ...
#endif
#if SCREENCOPY_HYPRLAND_TOPLEVEL
	} else if (auto* toplevel = qobject_cast<toplevel::Toplevel*>(object)) {
		auto* manager = hyprland::HyprlandScreencopyManager::instance();
		if (manager->isActive()) {
			return manager->captureToplevel(toplevel->implHandle(), paintCursors);
		}
#endif
	}
	return nullptr;
}
```

A `Toplevel` capture source goes **only** to the Hyprland path. There is no
`ext_foreign_toplevel_image_capture_source_manager_v1` branch anywhere in the
function — the protocol the ticket asked about is not an option the client
considers, so advertising the global cannot reach any fallback: the view logs
"Capture source set to non captureable object." and shows nothing.

The nix binary's own symbol inventory agrees (same method as PR #50's wlr/ext
split — interface tables vs. real client code):

- `qs::wayland::screencopy::icc::` = `IccManager`, `IccOutputSourceManager`
  (`captureOutput` only), `IccScreencopyContext` — the ext path is
  **output-only**;
- the only `captureToplevel` in the binary is
  `qs::wayland::screencopy::hyprland::HyprlandScreencopyManager::captureToplevel(qs::wayland::toplevel::wlr::ToplevelHandle*, bool)`;
- the `QtWayland::ext_foreign_toplevel_image_capture_source_manager_v1`
  class (with its `create_source` wrapper) exists but is unused
  qtwaylandscanner boilerplate — the scanner generates a client class for
  every protocol XML in the tree, used or not. Symbol presence was the
  ticket's reason to suspect a path; the class inventory is why the suspicion
  does not hold.

quickshell's own 0.3.1 docs say the same in as many words
(`quickshell.org/docs/v0.3.1/types/Quickshell.Wayland/ScreencopyView`,
`captureSource`): `ShellScreen` needs `wlr-screencopy` or
`ext-image-copy-capture-v1` + `ext-capture-source-v1`; `Toplevel` needs
`hyprland-toplevel-export-v1`.

Re-checked against upstream `master` on 2026-09-17 while writing this up:
`manager.cpp` routes identically (Toplevel → Hyprland only, no ext branch).
A future quickshell could grow the branch — that is a re-probe trigger, not a
reason to build now.

### Leg 3 — the confound, stated plainly rather than papered over

The live leg alone cannot distinguish "no code path" from "never got far
enough to try", because quickshell never instantiated **any** capture manager
in either run — not even for the output control:

```
ERROR quickshell.wayland.buffer.dmabuf: Failed to find render device: no render or primary node found.
 WARN quickshell.wayland.buffer: Render format initialization failed. All buffers will fall back to SHM.
```

followed by two `Cannot capture frame, as no recording context is ready.`
warnings (one per view) and zero `frameCaptured`/`hasContent` lines. Version-exact
source for why (`src/wayland/buffer/manager.cpp`, tag `v0.3.1`):

```cpp
bool WlBufferManager::isReady() const {
	return this->p->mDmabufFormatsReady && this->p->mRenderFormatsReady;
}
```

and `dmabufReady()` (the only setter of `mDmabufFormatsReady`) fires solely
from `LinuxDmabufFeedback::..._done()` — i.e. only after real
`zwp_linux_dmabuf_v1` feedback events from the compositor, which flexwm
truthfully never advertises (no GPU, no dma-buf, no DRM node; the string
`linux_dmabuf` appears **zero** times in either wire log). `ScreencopyView`
creates its context exclusively from `onBuffersReady`, so on this compositor
no `ScreencopyView` — output or toplevel — ever reaches
`ScreencopyManager::createContext`, and no capture global of any kind is ever
bound. The SHM fallback the warning names applies to *buffer creation*
downstream of readiness, not to readiness itself; no environment variable
bypasses the dmabuf half of `isReady()`.

So the wire absence is over-determined: legs 1 and 2 each suffice, and leg 3
explains why leg 1 looks the way it does. The verdict does not rest on the
live leg alone — legs 2 (exact-version source) and the binary inventory break
the tie the confound leaves. The confound is itself a finding with wider
blast radius than this ticket: **quickshell's workspace-overview
`ScreencopyView` cannot display on flexwm either**, even though the output
protocol it needs shipped in PR #52 and `grim` proves the protocol works. The
compositor-side question that unlocks every quickshell `ScreencopyView` —
whether a minimal, truthful `zwp_linux_dmabuf_v1` advertisement can flip that
readiness flag on a GPU-less compositor, and whether the icc output path then
displays over shm — is the named first measurement of the fallback item, not
a claim made here.

## What this means for the shells

- DMS / Noctalia per-window thumbnails (`ScreencopyView` over a `Toplevel`):
  unreachable via this protocol (no client code path) *and* unreachable via
  the protocol quickshell does speak (`hyprland-toplevel-export-v1`, which
  flexwm does not advertise — and per `CLAUDE.md`'s standards rule a
  Hyprland-specific protocol needs its own measured justification, which this
  probe was designed to produce for the *standard* one and did not).
- DMS / Noctalia overview previews (`ScreencopyView` over a `ShellScreen`):
  protocol present since PR #52, widget still blank — gated on the dmabuf
  readiness finding above, filed with the fallback item.

## What was NOT done

No render target, no constraint-refresh path, no `remove_window` teardown, no
lock answer, no handle bridge — the whole "rough order of size" list stands
unbuilt, deliberately. The probe diff above was reverted; the merged tree is
`main` plus docs only (`git status` clean of `.rs` changes), so there is no
advertised-but-`stopped` global, no new test surface, and no behaviour change
to benchmark: `cargo test` / `nextest` / `clippy -D warnings` / `fmt --check`
were re-run against the final tree to confirm it is exactly as clean as the
base. PR opened as docs-only; `screencopy.rs`'s module doc ("Output capture
only, deliberately") and its `capture_constraints` comment already describe
the shipped shape and needed no change.
