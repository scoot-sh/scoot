# flexwm — project instructions

## Vision and fixed decisions

flexwm is a Rust/Smithay Wayland window manager meant to be incredibly
lightweight, fast and beautiful; niri-like (scrolling columns); with full IPC
so an agent can do simple computer use in a VM; and able to run without a GPU
so it works inside linuxserver webtop (nested in pixelflux's Smithay
compositor via `/defaults/startwm.sh`). Later it should also drive macOS
window layout OmniWM-style through the Accessibility API.

- **License: MIT.** niri (GPL-3.0) and OmniWM (GPL-2.0-only) may inspire
  design, but their code — including cursor-theme assets — must not be
  copied. Check licenses before borrowing anything from either.
- **Backends, in order: headless, then nested (wl_shm + pixman), then tty
  (dumb buffers + pixman).** pixman is the default renderer; GL/GPU is an
  optional later tier, not a replacement — GPU-free operation is a hard
  requirement for webtop/no-GPU boxes, not something a GPU path supersedes.
- **`flexwm-core` stays platform-independent** (events and actions in,
  arrangement out; no Wayland, no I/O). A future macOS Accessibility-API
  adapter depends on this. Don't touch it without a measured reason once it's
  fuzz-tested and reviewed clean.
- Smithay is pinned to a specific git rev (see `Cargo.toml`) — verify claims
  about its behavior against the pinned source, not general Smithay
  knowledge, since APIs and semantics shift across revs.
- Prefer CPU-friendly rendering and damage-limited redraws throughout.

## Engineering standards

Write clean, decoupled, idiomatic Rust: well-organized modules, no very large
files. Add and run tests as each piece lands, not batched at the end.

**Priority order when things trade off:** speed/efficiency, bug-free,
well-architected, loosely-coupled code first — beauty (visual polish,
decorations, theming) second. If a visual feature would cost meaningful
performance, correctness risk, or coupling, the engineering bar wins; polish
waits or gets a cheaper implementation, not the other way around.

**This is a compositor people depend on to get their actual work done — it
must be snappy, always, and reliable, always.** Not aspirational: a concrete
bar every change is held to. Avoid unnecessary heap allocation on any hot or
per-event/per-frame path (input dispatch, the render loop, IPC dispatch) —
reuse and pool (persistent buffers built once, updated in place) rather than
allocating per frame or per event, the pattern this codebase already
establishes for render buffers. Treat a plausible crash/hang/panic path with
the same severity as data loss: a compositor crash takes every client
application's unsaved state down with it. Explicitly consider edge cases on
every feature (zero/one window, output-edge clamping, malformed input,
hardware/session failures, events at their real maximum rate) rather than
just the happy path, and benchmark before/after whenever a change touches a
hot path, whether or not it was asked for.

**Per-feature cycle**, repeated until stable before moving to the next thing:
build → test → bug bash → optimize → benchmark → independent review → commit
→ PR → **stop for user merge approval** (see below) → merge. Not
"make it work, polish later" — treat a feature as done only once it's passed
the full cycle. Split modules before they sprawl (tests in their own file),
run `cargo test`, `clippy -D warnings` and `fmt --check` after each step.
Beyond "does it work": actively look for bugs, not just the happy path;
measure performance with real before/after numbers; update `README.md`'s
Status/Running sections the same PR a feature lands in, not later — it drifts
stale fast otherwise.

**Independent review is mandatory, and it's a real gate, not a formality.**
Use the `flexwm-reviewer` subagent (`.claude/agents/flexwm-reviewer.md`) —
or, if it doesn't exist yet in this checkout, an equivalently-instructed
review pass — before any merge. The bar is stellar, principal-level-engineer
code. A subagent's "tests pass" self-report is a claim to independently
re-derive, not a verdict to relay: re-run build/test/clippy/fmt yourself
against real hardware where relevant, and trace what any new or changed
field/flag actually means everywhere it's read or written, not just at its
introduction site. Concrete worked example (see `ROADMAP.md` item 5b): a fix
gated `change_vt` on `Tty::active`, which passed every test and looked clean
on a surface read — but `active` meant two different things at its two write
sites (DRM master lost vs. session genuinely paused), and conflating them
silently broke the project's one hardware recovery path. That class of bug is
invisible to "does it compile and pass tests" and only shows up from reading
what a field means across every site that touches it.

**Merging needs an explicit user check-in, every time — not a one-time
unblock.** `gh pr merge` run autonomously gets denied by Claude Code's own
auto-mode classifier ("Merge Without Review") unless a Bash permission rule
in `.claude/settings.json` allows it (`Bash(gh pr merge:*)`) — and even then,
treat that as removing interactive friction for the *mechanical* merge
action, not as standing authorization to merge without review. A verbal
"you're approved to merge" in chat is a one-time approval for that specific
PR, not a policy change: PR it, tell the user it's ready with the review
findings summarized, and wait for either an explicit go-ahead or them
merging it themselves.

**Note on self-modification**: Claude Code refuses to let an agent write to
its own `.claude/` config (settings, agent definitions) even on direct user
instruction in chat — this is a deliberate guardrail, not a bug, and
shouldn't be routed around via another tool. If `.claude/` needs a change,
the user runs the command themselves in their own terminal (not via the
in-chat `!` prefix for anything multi-line — heredocs pasted that way don't
reliably execute; a real terminal session is the reliable path).

## Local scratch/handoff state

`HANDOFF.md` at the repo root is gitignored — the standing convention for
transient, session-specific state (open PRs awaiting merge, which VMs are up,
what's in flight right now) that goes stale within hours and isn't meant to
be shared or reviewed. It's distinct from this file (durable, shared,
git-tracked doctrine) and from a user's own Claude memory (durable,
user-specific preferences that persist across projects). Keep it current
as you work, and keep it short — anything durable belongs in `CLAUDE.md` or
`ROADMAP.md` instead, not duplicated here.

## Process notes

- Delegate implementation to fork/subagents to keep the coordinating
  session's own context lean, per standing user preference. The coordinating
  session is the orchestrator and the actual final gate — not a pass-through
  for subagent self-reports.
- A fork that returns near-instantly with zero tool calls and just restates
  the task back as a status summary has not done the work — resume it with
  an explicit instruction to actually use its tools, don't accept the report.
- Both dev VMs (linux-builder on `:31022`, the flexwm dev VM on
  `ssh -p 2222 dev@localhost`) may already be running — check
  (`nc -z localhost 31022` / `nc -z localhost 2222`) before starting either.
  See `vm/README.md` for setup, boot, and troubleshooting, including the
  `NIX_SSL_CERT_FILE` and tty-suspend/disk-lock gotchas.
- Don't restart, kill, or otherwise manage either VM process from an agent
  session by default — if one turns out to be down, say so and ask, rather
  than assuming ownership of the process's lifecycle. (Exception: the user
  has occasionally asked directly, in which case be explicit that doing so
  makes the VM a child of that Claude Code session's process tree.)
