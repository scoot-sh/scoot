---
title: "Per-output scale/mode configuration surface (after milestone 19)"
status: "open"
area: "core"
priority: "low"
blocked: "two-connector hardware to verify against — do not build blind"
---

# Per-output scale/mode configuration surface (after milestone 19)

Left open deliberately through milestone 19 phases A–D + F (see
`../roadmap/19-multi-output.md`, "Explicitly not in this milestone"):
every output beyond the first inherits the session's single `--width` /
`--height` / `[output] scale`, and `wlr-output-management` `apply`/`test`
stay refused as documented. This entry tracks what a real surface would
need, once there is hardware to test it against — do not build it blind.

**Status 2026-09-22: the three "Questions to settle first" below are now
answered against the current tree (Phase 3 scale reload + multi-output G/H
landed), so this file is a build-ready spec, not an open design question.
It stays OPEN because answering is not verifying: the ground rules still
gate any build on two-connector hardware.**

## Decided design (2026-09-22, grounded in the current tree)

### 1. Where per-output geometry lives: `[[outputs]]` list-of-tables in the config file; CLI stays the single-output default

**Decision: per-output geometry lives in the config file as an additive
`[[outputs]]` array-of-tables, each entry naming which output it matches
plus any of `width` / `height` / `mode` / `scale` it overrides. The
existing single `[output] scale` (and `--width` / `--height` / `--mode`)
remain the session default that any output without its own entry — and, at
first, every output — inherits. No `--outputs`-as-a-list shape, no
runtime-only `apply` path.**

Reasons, against the two constraints the ticket named:

- **Config-is-state.** The resolved record is
  `../resolved/startup-programs-and-autostart-done.md`: "config declares
  the session baseline, the script carries the behavior" (`:24`, and
  `docs/configuration.md:198`). A runtime-only `apply` surface would make
  the running session unreproducible from the file across a restart — the
  exact split that record settled. So the file is the ask path, and any
  future `apply`/`test` writes through to the same per-output values (see
  question 3), never a parallel store.
- **Phase 3 changed the mechanism answer.** When filed, scale was
  startup-only; now `apply_scale_reload` (`crates/scoot/src/compositor/
  reload.rs:362-379`) diffs one scalar and drives `rescale_outputs`
  (`crates/scoot/src/compositor/headless.rs:924-1004`), which already
  iterates *every* output, re-advertises each through the same `set_mode`
  startup uses (`headless.rs:278-304`), and recompacts them
  order-preservingly onto the new logical widths. Generalizing that walk
  from one scalar to a per-output map is additive on an existing iteration,
  not a new mechanism — which is what makes the config-file shape cheap and
  kills the "runtime-only" option's one advantage (that no reload path
  existed to extend).
- **Least parser churn, additive-or-nothing.** `FileConfig`
  (`crates/scoot/src/compositor/config.rs:319-336`) gains one optional
  field (`outputs: Vec<OutputEntry>`-shaped, default empty); `OutputConfig`
  (`config.rs:97-136`), `from_file_with_vt` (`config.rs:402-423`), and the
  `[output] scale` default path stay untouched. With zero `[[outputs]]`
  entries the load is byte-identical to today by construction — the same
  guarantee the milestone demands for single-output. The rejected shapes
  cost more: `--outputs` as a geometry list couples entry order to
  `Outputs::add` order (`crates/scoot/src/compositor/outputs.rs:91-106`)
  and cannot name `--tty` connectors at all (CLI `width`/`height` are
  already silently ignored there — `crates/scoot/src/cli.rs:209-223` —
  while `--mode` picks per connector, `cli.rs:237-249`); `[output.<n>]`
  indexed subtables collide with the existing scalar `[output] scale` table
  and would need match-key disambiguation anyway.
- **CLI keeps one size.** `--width`/`--height`/`--outputs`
  (`cli.rs:179-193`, defaults `1600x1000`/`1`, `cli.rs:255-257`) keep
  describing the primary output and the headless clone count
  (`add_output`, `headless.rs:153-208`, builds outputs 2..N at the same
  size). Per-output divergence is a config-file affair; flags stay the
  quick single-output knobs.

Sketch (field names illustrative, not frozen):

```toml
[output]
scale = 1.0            # session default, as today

[[outputs]]
match = "HDMI-A-1"     # connector name (--tty) or headless output name
scale = 2.0
# width / height / mode overrides here when E-phase hardware says what is real
```

**Still open (one sub-question, needs hardware, not more reading): the
`match` precedence** — connector name first with positional fallback, or
position first? `--tty` names come from `connector_mode`
(`tty/gpu.rs:621-624`); headless names are `OUTPUT_NAME` /
caller-supplied (`headless.rs:40`, `headless.rs:153-208`). Which key is
stable across a hotplug reseat can only be measured with two connectors
present (plug order, rescan renames). Do not freeze the precedence without
that measurement; the container shape above is unaffected either way.

### 2. Scale per output: re-examine the bind path — enumeration, not a solution

**Decision: per-output scale is a `State`-level map (defaulting every
output to the session `[output] scale`) consumed by the existing per-output
iteration points; the render side already reads per-output scale and needs
no change. What follows is the exact re-examination list — each item names
the code that breaks when two outputs disagree, and none of it is solved
here.**

Today there is one scalar: `State::output_scale` (`crates/scoot/src/
compositor/state.rs:245`) plus its precomputed integer companion
(`state.rs:254`), set once at startup (`state.rs:782-783`) and re-applied
live by Phase 3. The walk points already exist per output; the *value* does
not. Item by item:

1. **`wl_output.scale` advertisement — already per output object, needs a
   per-output value.** Smithay sends the integer on every
   `change_current_state` and on every client bind (see `output_scale.rs:
   13-18`); `set_mode` (`headless.rs:278-304`) applies one session scale to
   whichever output it is called for, and `rescale_outputs`
   (`headless.rs:924-1004`) passes the same `scale` to all. With two scales,
   the walk must pass each output its own — mechanism unchanged, argument
   per output.
2. **Fractional `preferred_scale` is per surface, not per output — the
   straddle question.** `new_fractional_scale`
   (`output_scale.rs:201-203`) sends `self.output_scale` at bind time; the
   render path already threads the real number per output
   (`render.rs:754,855` read `output.current_scale().fractional_scale()`).
   A surface spanning two outputs with different scales gets one
   `preferred_scale`: which output wins (pointer's? most-overlapped?
   first-entered?) is undecided and must be decided explicitly — today the
   question is unaskable because every output agrees.
3. **The integer companion rides along.** `send_preferred_buffer_scale`
   (`output_scale.rs:190-194`) accompanies every fractional send; wherever
   (2) lands, the companion follows the same winning output, or
   `wl_output.scale` and `preferred_buffer_scale` disagree for that
   surface (the agreement `integer_scale`, `output_scale.rs:125-127`, pins
   today is session-global). Name the bind-time caller too: `new_surface`
   (`handlers.rs:72-73`) passes the session-global integer to still
   role-less surfaces — under per-output scales a new surface keeps the
   session integer until the next reload re-send, the same staleness class
   as the documented role-less gap.
4. **The IPC snapshot must fill scale per output.** `OutputSnapshot` is
   built per output (`ipc.rs:665-690`, per-output `name`/`rect`/`usable`)
   but fills `scale: self.output_scale` (`ipc.rs:681`), the session-global
   scalar. Under mixed scales an agent doing physical↔logical targeting
   math off that field mis-places 2x on the second output — the
   agent-facing surface, so it matters to the computer-use half. Fill per
   output from the new map; no wire change, the field exists.
5. **The reload re-send walk must group roots by output.**
   `resend_output_scale` (`output_scale.rs:245-275`) collects window
   toplevels, per-output layer surfaces, lock surfaces and the cursor into
   one `Vec` and sends one scale to all. Per output it must send each root
   its output's scale: layers are already grouped per output in the walk;
   windows group via the pointer-output filing (`shell.rs:32-42`) /
   `output_of_window`; lock surfaces group per output (phase C made them
   per-output); the cursor takes the pointer's output. The documented
   role-less-surface gap (`output_scale.rs` re-send doc) multiplies by the
   number of distinct scales — still healed by the next reload, still worth
   restating, not re-solving.
6. **Cursor hotspot follows the pointer's output scale.**
   `surface_hotspot` is logical (`cursor.rs:540-549`); `element_location`
   scales pointer-minus-hotspot into physical (`cursor.rs:562-571`). One
   global cursor surface means one scale per frame: the output under the
   pointer, recomputed on output crossing — including the fractional exact-
   subtraction the doc pins (`cursor.rs:556-561`), which must hold per
   output, not just session-wide.
7. **Already per-output, no change:** `logical_size`
   (`output_scale.rs:142-153`) derives per output from its own mode+scale;
   `add_output` steps off the previous output's *logical* geometry
   (`headless.rs:163-179`) — mixed scales tile correctly through the same
   code; output-management heads refresh per output
   (`headless.rs:1001`, `resize_output`'s `refresh_output_heads` precedent).
8. **`--nested` stays scale-1-only and is out of scope for per-output.**
   The host owns the scale (`output_scale.rs:45-48`; reload refuses under
   nested, `reload.rs:362-379`), and there is exactly one output — nothing
   here applies.

Cost note: the per-surface walk stays a cold reload-path walk (as
documented at `output_scale.rs:236-239`); per-output grouping adds one
output lookup per root, not a hot-path allocation.

### 3. `apply`/`test`: refusal stands; per-output config lands first, `apply`/`test` follow — not together

**Decision: correct the ticket's "the two land together or not at all."
Per-output config (`[[outputs]]`) lands first, alone, through the file +
reload path. `apply`/`test` stay refused (`failed`, per
`output_management/configuration.rs:6,23,90`) until the landing condition
below fires, and that condition is unchanged by anything in this spec.**

Precise landing condition (from
`../resolved/output-management-reconfiguration-done.md`, restated as the
gate, not re-derived):

1. **Config keys must exist first** — i.e. this spec's `[[outputs]]`
   entries (mode/size per output, scale per output, position). An `apply`
   that can only re-assert what the file already says is the lying success
   the refusal exists to prevent (`configuration.rs:23-40` reasoning); a
   configuration that can ask for something real (a mode the connector
   lists, a position among several outputs, a scale the reload path
   applies) is what makes `succeeded` honest.
2. **Authorization shape, if any:** the protocol has no authorization
   concept and scoot has no privilege model to attach one to (no
   `security-context-v1` — finding 3 of the reconfiguration record). So
   either (a) `security-context-v1` or equivalent lands and writes are
   restricted to privileged clients, or (b) an explicit decision, with its
   own review, accepts any-client-may-modeset *with* the strobe guard
   (rapid successive applies visibly blank the screen per modeset — finding
   2's hostile-display argument) and the per-backend honesty matrix
   (`--nested` cannot honor a foreign mode; custom modes are unhonorable
   on `--tty`; `disable_head` on one output must not black-screen the
   session) worked out per property. There is nothing to restrict writes
   *to* today — that gap closes on no config-file roadmap.
3. **Simultaneous vs follow: follow.** The file is the ask path
   (config-is-state, question 1); `apply`/`test` are a second transport to
   the same values. The reconfiguration record's own revisit triggers
   already say this: real multi-output landing, a real client attempting
   writes (both probed shells read only), a privilege model landing. None
   of those fires when `[[outputs]]` lands. When one does, reopen from
   that record.

## Ground rules (updated 2026-09-22 — what Phase 3 and G/H changed)

- **No building without two-connector hardware to verify against** (the
  same gate as phase E: `multi-output-remainder.md:18-51`). `--headless
  --outputs N` carries harness shape but not modesetting truth —
  unchanged, still the gate. What Phase 3 *did* change is how much can be
  specified without that hardware: the rescale walk (`headless.rs:924-1004`)
  and the re-send walk (`output_scale.rs:245-275`) already iterate per
  output, so the spec above extends existing iterations rather than
  inventing blind mechanisms. Spec without hardware: fine. Build without
  hardware: still refused.
- **Single-output behavior stays byte-identical; additive or nothing** —
  unchanged, still the rule. Concretely: zero `[[outputs]]` entries loads
  exactly today's session (`config.rs:402-423` untouched on that path);
  exactly-1.0-everywhere recompacts each output onto its own origin with
  no re-announce (`headless.rs:946-956`); one output keeps the current
  `OutputId(1)` (`outputs.rs:56-59`).
- **What G/H settled since filing:** new windows file under the pointer's
  output (`shell.rs:32-42`, PR #208) — so per-output placement policy is
  *decided* and the `match` question above is only about config identity,
  not about where windows go. Default output binds exist (PR #208) — so
  `focus-output`/`move-window-to-output` need no new wire for the config
  surface to be testable.
- **`--nested` needs no per-output geometry:** it follows the host
  window's size for the session's life (single output, host-owned scale).
  The surface is headless-clone-count + `--tty`-multi-CRTC only.
