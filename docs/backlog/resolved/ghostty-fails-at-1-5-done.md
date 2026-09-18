---
title: "Ghostty fails to load at `[output] scale = 1.5` (works at 2.0) — RESOLVED as NOT REPRODUCIBLE: refuted on the reporter's own Asahi machine under every backend that can test it, including the original `--tty` configuration."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Ghostty fails to load at `[output] scale = 1.5` (works at 2.0)

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14, right after
`[output] scale` (PR #30) landed:

- `[output] scale = 2.0` → Ghostty loads and renders correctly.
- `[output] scale = 1.5` → **Ghostty does not load.**
- `foot` works at both `1.5` and `2.0`.

## RESOLVED — not reproducible (2026-09-18)

**Refuted on the reporting hardware under every backend capable of testing
it, including the original `--tty` one.** Both candidate hypotheses this
entry named are dead, and the last untested configuration has now been run.
The root cause was never identified and there is nothing left to investigate
without a fresh reproduction, so this is archived rather than left open.

The closing evidence, in the order it was obtained:

1. `--headless` at 1.5 and 2.0 on the real AGX GPU — maps and draws at both,
   zero EGL/GL errors, zero protocol errors. Refutes the GPU/GL hypothesis
   (the leading one) and the Ghostty-version hypothesis. See "Asahi hardware
   run" below.
2. `--nested` — cannot test this at all; flexwm refuses output scaling there
   by design. Recorded so the attempt is not repeated.
3. **`--tty` at `scale = 1.5` on the real `eDP-1` — the original
   configuration — works.** See "The original configuration, finally tested"
   below.

**Revisit condition:** if the symptom is ever seen again, capture
`~/.local/state/flexwm/session.log` (the session now logs there; see the
nixos-config `flexwm-session` wrapper) plus `WAYLAND_DEBUG=1` Ghostty stderr,
and reopen with both. Without a reproduction there is no next step.

## What was found and fixed (a real, separate protocol gap)

Output scaling created the `wl_compositor` global with `CompositorState::new`
(version 5). `wl_surface.preferred_buffer_scale` is a **v6** event, and
Smithay's `send_surface_state` (`wayland/compositor/mod.rs:411` in the pinned
rev) early-returns for `version() < 6` and has no caller anywhere in Smithay.
So a client that opts into `wp_fractional_scale_v1` received the exact
fractional `preferred_scale` but never the integer companion event.

That gap is real and is now closed (PR #31): `CompositorState::new_v6` plus a
`send_surface_state` call from `CompositorHandler::new_surface`, so a v6
client receives **both** `preferred_scale` (`1.5`) and
`preferred_buffer_scale` (`ceil(1.5) = 2`). Regression-tested on the dev VM
with a real client, red/green.

## Why the PR #31 fix did not close this (history)

**The fix does not explain the reported symptom, and review found evidence it
may not fix it.** GTK4's own source
(`gdk/wayland/gdksurface-wayland.c`, `surface_preferred_buffer_scale`) returns
early and ignores the event whenever a `wp_fractional_scale_v1` object exists —
which it always does now that flexwm advertises that global. So a GTK4/Ghostty
client likely discards the very event this PR adds.

What that means:

- The integer companion was worth sending for protocol completeness (it is
  what wlroots does), and that part is correct and tested.
- It is **not established** that sending it makes Ghostty load at `1.5`. The
  root cause of the reported failure is therefore **unknown** as of this entry.

## Local reproduction attempt (2026-09-14) — did NOT reproduce

Attempted on the dev VM rather than guessing, per step 3 above. Ghostty **is**
installable there (`nix-shell -p ghostty` → 1.3.1). With `main` at `6cf1106`
(includes #30 and #31), under `--headless` at a virtual output:

| client | scale 1.5 | scale 2.0 |
| --- | --- | --- |
| `ghostty --gtk-single-instance=false` 1.3.1, kept alive | **maps** (logical 409×510) | maps (360×376) |
| `gtk4-demo` (GTK 4.22.4) | maps | maps |
| `foot` | maps | maps |

Ghostty rendered real content at 1.5 (a 1280×800 screenshot at scale 1.5 was
35,623 bytes of drawn frame, not a blank backdrop). The logical rects shrinking
as scale rises (409×510 at 1.5 vs 360×376 at 2.0; a fresh run gave 302×376 at
2.0 with a different window title/decoration) shows GTK4/Ghostty are honoring
the fractional scale — flexwm's protocol side is delivering.

**So this is not reproducible on the dev VM, and the earlier theory that it is
output-scaling's fault is unsupported.** Three harness artifacts during this
investigation each looked like the bug and were not:

- A fixed 4–5 s wait was too short for GTK4/Ghostty to map under software GL
  (they need ~7 s; foot needs ~0.5 s) — "no windows mapped" was timing.
- A first binary tested predated the output-scaling feature — an apples-to-
  oranges comparison that briefly suggested a pre-existing GTK4 bug.
- `ghostty -e true` exits immediately, so the window opens and closes in a
  fraction of a second; polling after it was gone read as "doesn't load" at
  both scales. Keeping the process alive (`-e sh -c "sleep 60"`) makes it
  deterministic.

Anyone re-running this must **poll for the window** and **keep the client
alive**; a fixed sleep plus a command that exits will produce a false failure.

## Asahi hardware run (2026-09-18) — did NOT reproduce

Run on the reporter's own machine per [`Asahi.md`](../../../Asahi.md) Test 1:
Apple M2 (`apple,t8112`, j413), NixOS aarch64, Ghostty **1.3.1**, flexwm built
from `main` at `f688ac9`. `--headless --width 1280 --height 800`, client kept
alive with `-e sh -c "sleep 60"`, 12 s settle.

| | scale 1.5 | scale 2.0 |
| --- | --- | --- |
| window mapped | **yes**, `com.mitchellh.ghostty`, logical 409×510 | yes, 360×376 |
| `wp_fractional_scale_v1.preferred_scale` | `180` (= 1.5) | `240` (= 2.0) |
| `wl_surface.preferred_buffer_scale` | `2` | `2` |
| attach / commit / presented events | 21 / 22 / 42 | 21 / 22 / 42 |
| `wl_display.error` (protocol errors) | 0 | 0 |
| client-side EGL/GL errors | none | none |

The logical rects reproduce the dev VM's numbers **exactly** (409×510 and
360×376), which cross-checks the methodology across two very different
machines. Screenshots show drawn content — terminal background, focus border,
live cursor block — not a blank backdrop.

### Both hypotheses this entry named are now refuted

- **(2) The GPU/GL path — the leading hypothesis — is refuted.** This run went
  through the real Asahi AGX stack, not software rendering. Per log, by
  `grep -c` so anyone can re-derive them:

  | metric | 1.5 | 2.0 |
  | --- | --- | --- |
  | lines matching `dmabuf` | 70 | 70 |
  | lines matching `zwp_linux_dmabuf_v1` | 24 | 24 |
  | `create_params` / `create_immed` | 6 / 6 | 6 / 6 |

  Mesa's EGL queue binds `zwp_linux_dmabuf_v1` v4 and attaches the resulting
  `wl_buffer`s, and the Ghostty process holds `/dev/dri/renderD128` (the
  `asahi` render node) open. That is precisely the path the dev VM
  structurally could not exercise, and it produced no EGL/GL error and no
  failure at either scale.
- **(1) Ghostty version is refuted.** The reporter's machine runs 1.3.1 — the
  same version as the dev VM. And it ran it *on the day of the report*: NixOS
  system generation 26, dated 2026-09-14, already carried `ghostty-1.3.1`, as
  has every generation since. So version divergence was never the variable,
  and unlike the other rows this one is refuted for the original stack rather
  than only the current one.

### Also tested: an older binary, to rule out a recent fix masking the bug

At the time of the run the live session was still on an **older** flexwm
build than `main` at `f688ac9` (it has since been rebuilt onto `f688ac9`
itself — see the re-confirmation below). A differential run of that older
binary at 1.5 also maps Ghostty (409×510, 42 frames presented, and the same
70 / 24 / 6 dmabuf counts as both runs above — all three logs are identical
on every one of those metrics). So the non-reproduction is not an artifact of
recent work masking
the bug — in particular it is not the dmabuf-import fix (`ce15c09`) that
landed the same day.

That fix could not have been the original cause in any case: the dmabuf
advertisement that broke GL clients landed 2026-09-17
([`../resolved/dmabuf-advertised-but-never-imported-done.md`](../resolved/dmabuf-advertised-but-never-imported-done.md)),
**three days after** this was reported on 2026-09-14.

Later the same day the machine was rebuilt onto `f688ac9` and logged back
in, so the live session now runs the very binary these measurements were
taken against. It still comes up on `eDP-1` at `scale = 2.0` — which is the
workaround, not the test. The `--tty`-at-1.5 question below is therefore
still open, and is now one `home.nix` edit away rather than a rebuild away.

### Why `--tty` still had to be tested separately

The original report was against the user's real **`--tty`** desktop session on
`eDP-1`; everything above is `--headless`. That gap mattered less while the
GL-path hypothesis stood — `Asahi.md` justified `--headless` on the grounds
that the client's GL stack is identical either way, which is true and is why
the hypothesis could be refuted there. But refuting it is exactly what made
the backend difference the largest remaining variable rather than a
negligible one.

**`--nested` cannot substitute.** It was tried on 2026-09-18 and flexwm
refuses fractional scale there by design:

```
WARN flexwm::compositor: output scaling is not supported under --nested
     (the host compositor owns the window's scale); using 1.0 configured=1.5
```

The client saw only `preferred_scale(120)`, so the run says nothing about 1.5.
`--headless` and `--tty` are therefore the only two backends that can test
this at all.

### The original configuration, finally tested — it works

On 2026-09-18 the machine was rebuilt with `[output] scale = 1.5` in
`home.nix` and rebooted into it. This is the reported configuration exactly:
`--tty`, real DRM modesetting, real `eDP-1`, fractional scale.

```
flexwm msg outputs  ->  eDP-1, scale 1.5, logical 1707x1067
flexwm msg windows  ->  com.mitchellh.ghostty, "◐ Asahi.md validation"
```

**Ghostty mapped and was used interactively** — the session doing the
verifying was itself running inside that Ghostty window at 1.5. Logical
1707×1067 is 2560/1.5 × 1600/1.5, so the fractional scale was genuinely
applied and not silently rounded.

The session log (now captured; see the revisit condition at the top) contains
nothing relevant to this entry: no protocol error, no EGL/GL error, no client
failure. It carries four warnings at startup, none of them Ghostty-related —
two from flexwm and two from Smithay:

| source | warning |
| --- | --- |
| flexwm | `drm: device unusable path=/dev/dri/card1 … (os error 95)` — expected; the fallback working |
| flexwm | `crtc reports an unusable gamma size; advertising 256 instead size=0` |
| Smithay | `Unable to become drm master, assuming unprivileged mode` — expected on any non-root `--tty` |
| Smithay | `Failed to destroy old mode property blob` — benign, first modeset |

The gamma one is worth extracting as a **separate Apple Silicon finding**:
`apple-drm` reports a zero-length gamma LUT, so flexwm advertises a 256-entry
`zwlr_gamma_control` ramp this hardware does not actually have. Irrelevant to
Ghostty and not a defect this entry covers, but it is exactly the sort of
hardware-specific fact `Asahi.md` exists to capture, and anyone doing
night-light/gamma work on Apple Silicon should start from it.

### Adjacent measurement: fractional scale is not slower to composite

Worth recording because it was the last plausible mechanism for a
"doesn't load" that was really a "too slow to appear". It isn't one.
`--headless` at 2560×1600 with an identical fixed-rate (20 fps) `foot` client,
flexwm's own `utime+stime` from `/proc/<pid>/stat` over a 12 s window after an
8 s settle:

| scale | run 1 | run 2 (independent re-run, review) |
| --- | --- | --- |
| 2.0 | 11.2% | 13.1% |
| 1.5 | 11.3% | 11.9% |

**Read this as "indistinguishable", not as a 0.1-point difference.** These are
single samples and the run-to-run spread is ~1–2 points — an order of
magnitude larger than the within-run gap, and the re-run put the *fractional*
arm cheaper, reversing the sign. The honest claim the data supports is that
fractional scale costs no more than integer scale, not that it costs 0.1 point
more. Static screen reproduced exactly at 0.0% in both runs, so damage-limited
redraw is working and there is no spin. On the live `--tty` session flexwm
sits around 48% CPU while a client animates continuously at 2560×1600 —
that is the honest cost of CPU compositing 4.1 megapixels at display refresh,
it is **not** scale-dependent, and it is not evidence for this entry.

## What is left, after everything above

Every mechanism this entry proposed has been tested and eliminated:

A note on how strong these verdicts are. Only the first was tested against
the **original** 2026-09-14 stack; every other row was measured on the
2026-09-18 stack, four days and several rebuilds later. "Not reproducible on
the 2026-09-18 stack" is the honest reading of those, not "could never have
happened" — which is why this entry closes as not reproducible rather than as
a misreport.

| candidate | verdict |
| --- | --- |
| Ghostty version | **refuted outright** — 1.3.1 on the dev VM, and generation 26 dated 2026-09-14 (the report day) already carried 1.3.1, as has every generation since |
| GPU/GL path (the leading one) | not reproducible on the 2026-09-18 stack — real AGX, dmabuf, no EGL/GL error at either scale |
| `--tty`-specific interaction at fractional scale | not reproducible on the 2026-09-18 stack — the original configuration was run and works |
| fractional scale being too slow to appear | not supported — CPU cost is indistinguishable between scales, see the measurement above |
| `wl_output` version / other global differences | ruled out — same build, byte-identical logical geometry |

What remains is unfalsifiable from here: **something environmental at the
time of the report that has since changed** — a Ghostty config, a GTK/Mesa
version, or one of the many compositor changes between 2026-09-14 and
2026-09-18. There is no evidence pointing at any specific one, and no test
that would distinguish them without a live reproduction.

Recorded plainly, because the alternative framing would be dishonest: the
user's original observation is not disputed, and this does **not** conclude
it was a misreport. It concludes that the symptom is gone and its cause was
never captured. That is a real outcome, not a diagnosis.

## What not to do next

Do not ship a compositor change for this. Nothing in three separate
investigations named a flexwm defect, and the protocol work (#30, #31) stands
on its own merits regardless of this symptom.

## Related

- [`Asahi.md`](../../../Asahi.md) — the hardware runbook this entry gated on.
- `docs/backlog/resolved/output-scaling-done.md` — the scaling feature itself.
- `docs/backlog/resolved/fractional-scale-integer-companion-done.md` —
  the integer-companion mechanism, fixed and tested; kept there for the
  diagnosis even though this symptom it was thought to explain did not
  resolve.
