---
title: "Ghostty fails to load at `[output] scale = 1.5` (works at 2.0); did NOT reproduce on the reporter's own Asahi machine, refuting both leading hypotheses — root cause still unidentified"
status: "open"
area: "protocols"
priority: "low"
blocked: "needs a --tty session at scale 1.5 on the reporter's machine; --headless and --nested are both now exhausted"
---

# Ghostty fails to load at `[output] scale = 1.5` (works at 2.0)

Reported by the user on their Asahi Linux (M2) laptop, 2026-09-14, right after
`[output] scale` (PR #30) landed:

- `[output] scale = 2.0` → Ghostty loads and renders correctly.
- `[output] scale = 1.5` → **Ghostty does not load.**
- `foot` works at both `1.5` and `2.0`.

## Status after the Asahi run (2026-09-18)

**It did not reproduce on the reporting hardware, and both candidate
hypotheses below are now refuted.** Priority drops from high to low: the
symptom is not observable on the machine that produced it, and the user
already daily-drives the `scale = 2.0` workaround. It is kept open rather
than resolved only because the root cause was never identified and one
configuration — the original one — remains untested. See "Asahi hardware run
(2026-09-18)" below.

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

## Why this entry is still open, not resolved

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
  through the real Asahi AGX stack, not software rendering: 88
  `zwp_linux_dmabuf_v1` references per run, and the Ghostty process holds
  `/dev/dri/renderD128` (the `asahi` render node) open. That is precisely the
  path the dev VM structurally could not exercise, and it produced no EGL/GL
  error and no failure at either scale.
- **(1) Ghostty version is refuted.** The reporter's machine runs 1.3.1 — the
  same version as the dev VM, so version divergence was never the variable.

### Also tested: an older binary, to rule out a recent fix masking the bug

At the time of the run the live session was still on an **older** flexwm
build than `main` at `f688ac9` (it has since been rebuilt onto `f688ac9`
itself — see the re-confirmation below). A differential run of that older
binary at 1.5 also maps Ghostty (409×510, 70 dmabuf references, 42 frames
presented). So the non-reproduction is not an artifact of recent work masking
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

### The one configuration still untested, stated plainly

The original report was against the user's real **`--tty`** desktop session on
`eDP-1`. Everything above is `--headless`. That gap mattered less while the
GL-path hypothesis stood — `Asahi.md` justified `--headless` on the grounds
that the client's GL stack is identical either way, which is true and is why
the hypothesis could be refuted there. But with the GL path eliminated, the
backend difference is now the largest remaining variable rather than a
negligible one.

**`--nested` cannot substitute.** It was tried on 2026-09-18 and flexwm
refuses fractional scale there by design:

```
WARN flexwm::compositor: output scaling is not supported under --nested
     (the host compositor owns the window's scale); using 1.0 configured=1.5
```

The client saw only `preferred_scale(120)`, so the run says nothing about 1.5.
`--headless` and `--tty` are therefore the only two backends that can test
this at all, and `--headless` is now exhausted.

## What this narrows it to

With (1) and (2) refuted and the dev VM and Asahi runs agreeing to the pixel,
what is left is narrow:

1. **A `--tty`-specific interaction at fractional scale** — real DRM
   modesetting and eDP-1's actual geometry (2560×1600 physical; 1706×1066
   logical at 1.5, versus the 854×534 the headless probe used). Untested.
2. **Something environmental at the time of the report that has since
   changed** — a Ghostty config, a GTK/Mesa version, or one of the many
   compositor changes between 2026-09-14 and 2026-09-18. If so there is
   nothing left to fix and this closes on the next clean `--tty` run.
3. **A misreport or a transient** — possible but unevidenced, and not worth
   assuming over the user's direct observation.

This entry previously carried a third candidate, "`wl_output` version or
other global differences", marked *less likely*. It is dropped rather than
lost: the dev VM and the Asahi machine run the same flexwm build and now
produce byte-identical logical geometry and the same advertised globals, so
a global-set difference between them is ruled out. A `--headless`-vs-`--tty`
difference in what `wl_output` reports is *not* ruled out, and is folded
into candidate 1 above.

## What to do next

1. **The cheap decisive test, which only the user can run:** set
   `[output] scale = 1.5` in `~/.config/flexwm/config.toml` and restart the
   session. `Ctrl+Alt+F<vt>` remains the recovery path, so this is low-risk.
   If Ghostty maps, close this entry; if it does not, capture the session's
   stderr (it goes to the VT — redirect it to a file) and the entry finally
   has its reproduction.
2. Do not ship a compositor change for this until that run names a flexwm
   defect. The protocol work (#30, #31) stands on its own merits regardless.

## Related

- [`Asahi.md`](../../../Asahi.md) — the hardware runbook this entry gated on.
- `docs/backlog/resolved/output-scaling-done.md` — the scaling feature itself.
- `docs/backlog/resolved/fractional-scale-integer-companion-done.md` —
  the integer-companion mechanism, fixed and tested; kept there for the
  diagnosis even though this symptom it was thought to explain did not
  resolve.
