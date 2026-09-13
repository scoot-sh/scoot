# flexwm — project instructions

## Vision and fixed decisions

flexwm is a Rust/Smithay Wayland compositor meant to be incredibly
lightweight, fast and beautiful; niri-like (scrolling columns); with full IPC
so an agent can do simple computer use in a VM; and able to run without a GPU
so it works inside linuxserver webtop (nested in pixelflux's Smithay
compositor via `/defaults/startwm.sh`). Later it should also drive macOS
window layout OmniWM-style through the Accessibility API.

- **Computer use is the whole game here, not one goal among several.**
  (Explicit user statement, 2026-09-12.) flexwm exists to be lightweight,
  fast, and GPU-optional *in service of* being the best possible target for
  an agent doing computer use — not as independent goals of equal weight.
  When prioritizing what to work on next, weigh a candidate item by whether
  it actually serves agent-driven automation (IPC richness, reliability,
  ergonomics, targeting fidelity) before weighing it as general compositor
  completeness (visual polish, protocol coverage for its own sake). This
  doesn't override the engineering-standards priority order below
  (correctness and architecture still come before beauty on any given
  change) — it's about which *feature* to pick up next, not how carefully
  to build it once chosen. Concretely: the background/off-focus window
  input backlog item below is a direct example of a computer-use-serving
  feature that should be weighed accordingly against general-purpose items
  like layer-shell or GPU rendering, which don't serve this goal directly.
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
- **"Compositor," not "window manager," in anything user-facing** (`README.md`,
  crate descriptions, `--help` text). Wayland has no separate window-manager
  protocol the way X11 did — whatever composites client surfaces is also
  whatever arranges them, so there's no such thing as a standalone Wayland
  window manager process. Matches how niri/sway/Hyprland self-describe
  ("compositor" first, "tiling window manager" as the behavior description).
  Exception: `flexwm-core`'s own crate description genuinely is just
  "window management state and layout" — it has no compositor concerns at
  all (no Wayland, no I/O, per the platform-independence note above), so
  that wording is accurate as-is and shouldn't change to match this rule.
- **Where an emerging or established Wayland protocol standard exists,
  implement that rather than a bespoke/compositor-specific alternative,
  unless there's a concrete reason it doesn't fit.** E.g. `ext-workspace-v1`
  (compositor-agnostic) over the older one-off wlr workspace protocols it
  supersedes. This is also why `flexwm-ipc`'s own protocol is scoped to
  what genuinely needs to be flexwm-specific (agent-driven input injection,
  screenshots, window/action queries) rather than reinventing something a
  standard Wayland protocol already covers for external tools like bars —
  see the `wlr-layer-shell-unstable-v1`/`ext-workspace-v1` backlog entries
  in `ROADMAP.md` for where that line falls in practice.

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

**Merge authority is delegated, standing on the review gate — not a
per-PR ask.** (Confirmed explicitly by the user, 2026-09-12, after several
PRs of per-PR check-ins: "Yes. For the last time. You get to be the final
gate after reviewing the review agent's diagnosis." This supersedes any
earlier "ask every time" framing.) The coordinating session is the final
gate: once `flexwm-reviewer` (or an equivalently rigorous pass) has reported
back and the coordinating session has actually read and weighed its
diagnosis — not just seen "no blocking findings" and skipped straight past
it — merge without asking again. `gh pr merge` is unblocked mechanically via
the `Bash(gh pr merge:*)` rule in `.claude/settings.json` (the auto-mode
classifier denies it by default otherwise, reason "Merge Without Review").
The judgment that still matters, every time: did the review actually happen
and come back clean (or did its findings get addressed and re-verified,
not just noted)? A blocking finding, an unclean test run, or a review that
couldn't actually verify something important (and said so) means don't
merge and explain why — the gate is real, it's just no longer a chat
round-trip when it passes.

**Note on self-modification**: Claude Code refuses to let an agent write to
its own `.claude/` config (settings, agent definitions) in normal auto-mode,
even on direct user instruction in chat — a deliberate guardrail (confirmed:
the protected-path check runs *before* `permissions.allow` is even
evaluated, so no settings-file rule can lift it). Two real paths around it,
neither a workaround of the guardrail's intent: (1) the user runs the
command themselves, directly; (2) the user runs the session in
`--dangerously-skip-permissions` (bypass) mode, which the Claude Code docs
say is meant only for isolated/disposable environments, not routine use —
treat write access gained this way as scoped to the specific task at hand,
not a standing license, and keep the same care around destructive/hard-to-
reverse actions regardless (the safety net just isn't there to catch a
mistake). If asking the user to paste a multi-line heredoc themselves:
this failed non-obviously once when the user was typing from a phone —
long single-newline-dense text (like a YAML frontmatter block) got
silently corrupted (lines merged, then later whole spans of body text lost)
somewhere in the mobile input path. Prefer very short, single-line commands
when the user is likely on mobile, and always ask them to verify the result
(e.g. `cat -et` or a line count) rather than assuming a paste landed intact.

## Verification and evidence (avoiding redundant hardware work)

The standard verification set for any compositor change: `cargo test -p
flexwm`, `cargo clippy -p flexwm --all-targets -- -D warnings`, `cargo fmt
--check -p flexwm`, and `scripts/smoke-test.sh` (backend-agnostic IPC-driven
end-to-end test; set `MODE=--nested` to run under a host compositor, real
`--tty` hardware needs the actual binary launched there — see the script's
own header comment).

The first three are cheap to re-run in full, every time, no exceptions —
`cargo`'s own incremental build cache means re-running them after a build
that already succeeded costs almost nothing (a from-scratch 93-test run
finishes in ~0.01s once compiled), and this is exactly where a fabricated or
merely-wrong "it works" self-report gets caught cheaply. Independent review
never skips these on the theory that the implementer already ran them.

Real hardware bug-bash and benchmarking (VT-switch cycles, jiffies-delta
sampling, IPC screenshot capture) is the genuinely expensive part — real
wall-clock time, SSH round trips, and state that can't be cached by `cargo`.
Treat this the way a task runner treats a cache: the implementer's report is
the cache entry, keyed by the exact commit/tree state it was captured
against, and it's only valid unmodified. Concretely:
- **The implementer records, not narrates**: the exact commands run, the
  exact commit SHA (or "uncommitted, working tree as of \<describe\>") the
  verification was run against, and raw output/artifacts (real screenshot
  file paths, raw sampled numbers) — not a paraphrased summary. A claim with
  no reproducible evidence attached is treated as unverified.
- **The reviewer checks the cache key first**: if the code has changed at
  all since that SHA (including a review-driven fix), the recorded evidence
  for whatever it touched is stale and must be redone — don't trust
  hardware verification against code that no longer exists.
- **If the key still matches, spot-check rather than fully redo**: re-run at
  least one representative scenario live (ideally the one most central to
  the diff's risk) to rule out fabrication, and audit the rest of the
  recorded evidence for methodological soundness (did they actually wait for
  idle before sampling? is the screenshot's content consistent with the
  claim?) rather than mechanically repeating every scenario end to end.

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

- Three agent roles are documented in `.claude/agents/`:
  `flexwm-orchestrator.md` (the coordinating session's own role — delegate,
  gate on review, merge, repeat), `flexwm-implementer.md` (implements one
  item per invocation, full per-feature cycle, never merges), and
  `flexwm-reviewer.md` (independent review gate, never edits or merges).
  Read the relevant one before acting in that role.
- Delegate implementation to a **fresh, non-fork subagent**
  (`flexwm-implementer`, or general-purpose if it doesn't fit) to keep the
  coordinating session's own context lean, per standing user preference —
  but prefer a fresh subagent over `subagent_type: fork` for this. A fork
  inherits and re-sends the entire growing conversation transcript every
  time it's spawned; fine occasionally, but it compounds into real token
  cost across a long session working through many roadmap/backlog items.
  Reserve fork for cases that genuinely need the parent's full context. The
  coordinating session is the orchestrator and the actual final gate — not a
  pass-through for subagent self-reports, implementer or reviewer.
- A fork or agent that returns near-instantly with zero tool calls and just
  restates the task back as a status summary has not done the work — resume
  it with an explicit instruction to actually use its tools, don't accept
  the report.
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
- **Never run `git checkout`/`git commit`/other branch-mutating commands in
  the shared main checkout (`/Users/steveyackey/code/flexwm`) while a
  background implementer agent might still be active there.** They're
  instructed to work directly in that checkout, not a worktree (see
  `flexwm-implementer.md` — the dev VM's 9p mount points at that exact
  directory, which a worktree elsewhere can't reach without a manual
  copy-over). A background agent's own `git checkout -b` can silently switch
  the branch a concurrent orchestrator action lands on — this actually
  happened once: an orchestrator-side wording fix landed on the
  implementer's in-progress branch instead of the intended one, and
  `gh pr merge --delete-branch`'s local cleanup step touches whatever branch
  is checked out too. No work was lost (the stray commit was recovered via
  `git reflog` and cherry-picked to the right place), but it was avoidable.
  If the orchestrating session needs to touch the repo while any
  implementer might still be running (fixing a small nit directly, updating
  `ROADMAP.md`, merging a PR), do it from a throwaway `git worktree add
  /tmp/<name> <branch>` instead of the shared checkout, and remove it after
  pushing. Only skip the worktree once confirmed no implementer is active.
