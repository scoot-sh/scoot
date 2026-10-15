---
title: "The compositor reads a look layer under the user's config"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-15"
---

# The compositor reads a look layer under the user's config

Filed 2026-10-12, child of [`live-looks`](live-looks.md). Serves **both**:
the ring, background, cursor and wallpaper are the most visible half of a
look, and the screenshots agents take. The look-scoped reload
([`look-scoped-reload`](look-scoped-reload.md)) and the per-session state
([`look-session-state`](look-session-state.md)) are split out.

## The gap

scoot reads one file (`config::reload_from`, `config.rs:756-767`). A bad
color in it falls back to `Appearance::default()` (`config.rs:194-206`),
which under a look layer would show neither the user's color nor the look's.
Ring and background changes run `apply()` (`reload.rs:308-321`).
`[wallpaper]` is fire-and-forget (`reload.rs:125-133`).

## What to do

Per the design doc's sections 4.1, 4.3 and 4.10:

- Read `<state>/look/scoot.toml` (`<state>` is `$XDG_STATE_HOME/scoot`, else
  `~/.local/state/scoot`, until `look-session-state` adds the private
  states) and merge it under the user's file **on raw keys, before
  derivation**, by the three rules of section 4.3:
  1. every key per key;
  2. `[wallpaper]`, with its per-output and per-workspace tables, as the one
     unit (the color+image trap,
     `docs/backlog/resolved/stylix-wallpaper-color-trap-done.md:17-24`);
  3. a derived key follows the layer that supplied its base: the user's
     `focus_ring_inactive_width` if set; else, if the user set
     `focus_ring_width`, the look's inactive width is dropped so it follows
     the user's (`config.rs:209-210`); else the look's.
- The layer is parsed strictly against scoot's allowlist from `scoot-look`:
  a forbidden key or a bad value refuses the whole layer (startup: no layer,
  an error log; reload: refused, everything kept).
- A bad *user* value for a key the layer provides falls back to the layer's
  value, and the reply reports it as invalid and shadowing.
- The reply lists shadowed keys and gains an option to wait for the
  wallpaper run.
- Emit `EventKind::Look` when a reload sees the layer's identity change (the
  rendered look's hash in its `render.json`; not `<state>/look.json`, which
  is the tool's record of the live choice).
- Ring, background and corner changes request a redraw without `apply()`
  (measure first).
- Measure the cursor rebuild on the loop thread in a release build at 24,
  48 and 256 px (PR #572 reports 36.4 ms at 16 px and 1.82 s at 256 px in a
  debug build); if it exceeds a frame at common sizes, build the bitmaps on a
  worker and swap them in.
- D15's condition: measure the per-frame cost of rounded corners on the
  default CPU (pixman) renderer at 1, 10 and 50 windows; the shipped looks
  carry a nonzero radius only if it is acceptable. If it is, raise the
  shipped looks' `[shape] radius` and `[opacity] bar` from the 0 and 1.0
  `look-shipped` lifted to the examples' values (in their comments) in this
  same PR; if not, leave them and document the trade-off.
- Docs in the same PR: the config reference gains the look layer and its
  precedence.

## Evidence to record

Tests:
- per-key precedence, including a user `cursor_size` alone keeping the
  look's cursor theme and accent;
- the wallpaper unit (a user `color` with a look `image` yields the user's
  section, a look-only section passes through whole);
- the derived pair (user active width alone: the inactive width follows the
  user's, not the look's; user inactive width alone: the look's active width
  stays);
- a forbidden key and a bad value refused whole; a bad user color falling
  back to the layer; a dangling link at startup; the event fired once per
  identity change.

Benchmarks before/after on headless with 1, 10 and 50 windows: a ring-only
reload with and without `apply()`, frame times across it, the cursor rebuild
times, and the rounded-corner frame cost on pixman.

## Not in this ticket

The look-scoped reload, per-session state, the CLI, scootbar, IPC requests.

The design this implements against: [live looks](../../../dev/research/live-looks-design.md).

## Resolution (PR #613, branch `look-layer-scoot`)

Landed as designed, with four deviations noted in the PR: no `--state`
flag yet (the CLI is `look-apply-cli`'s; the resolver takes an explicit
dir for it), a missing link stays silent rather than warning (absent is
every install's default state), the merge runs on pre-derivation struct
values instead of raw TOML tables (the stack-overflow trap `config.rs`
measures for passthrough-table parses), and the cursor worker fired: the
release-build measurement showed 23.4 ms at 24 px, past a frame at common
sizes. D15's pixman condition passed (~+5-6% per frame at 1, 10 and 50
windows), so the shipped looks' radius and bar opacity rose to the
examples' values in the same PR (rebased onto `look-shipped`, which landed
mid-ticket). Evidence: per-key/unit/derived-pair tests, whole-layer
refusals, bad-user-color fallback, dangling-link startup, the `Look` event
firing once per identity change; ring-reload, cursor-rebuild and
rounded-corner benchmarks on headless at 1/10/50 windows with raw numbers
in the PR. One unrelated failure stands:
`xwayland refused_cost::a_refused_managed_window_costs_the_server_nothing`
(273 fds vs ≤264) fails deterministically on the dev VM in code paths this
diff does not touch; unverified against main for lack of VM disk.
