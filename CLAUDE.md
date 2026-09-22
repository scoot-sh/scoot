# scoot — project instructions

## Vision and fixed decisions

scoot is a Rust/Smithay Wayland compositor meant to be incredibly
lightweight, fast and beautiful; niri-like (scrolling columns); with full IPC
so an agent can do simple computer use in a VM; and able to run without a GPU
so it works inside linuxserver webtop (nested in pixelflux's Smithay
compositor via `/defaults/startwm.sh`). Later it should also drive macOS
window layout OmniWM-style through the Accessibility API.

- **Computer use is a major goal, not *the* goal — scoot also has to be real
  enough to daily-drive.** (User statement 2026-09-12, correcting an
  earlier, too-absolute framing: computer use is "the whole mission for a
  part of things," not the entire mission.) Two priorities coexist when
  picking what to work on next: agent-driven automation (IPC richness,
  reliability, targeting fidelity) *and* general desktop completeness (bars,
  launchers, notifications, workspace switching — what daily use needs).
  Neither silently wins; a candidate item's case should say which it serves.
  This governs *which feature* to pick up next, not how carefully to build it
  — the engineering priority order below still stands per change.
- **Daily-drivability includes not putting the user's own setup at risk.**
  Trying scoot on real hardware (vs. the disposable dev VM) needs an
  explicit, low-risk path — `--nested` inside an existing session first, a
  real `--tty` session reachable by a VT switch back to whatever the user
  normally runs — never something that could strand them out of their own
  desktop. Treat any request to deploy scoot onto a user's real, primary
  machine as consequential and hard to reverse: require explicit confirmation,
  scoped to what was actually asked.
- **License: MIT.** niri (GPL-3.0) and OmniWM (GPL-2.0-only) may inspire
  design, but their code — including cursor-theme assets — must not be
  copied. Check licenses before borrowing from either.
- **Backends, in order: headless, then nested (wl_shm + pixman), then tty
  (dumb buffers + pixman).** pixman is the default renderer; GL/GPU is an
  optional later tier, not a replacement — GPU-free operation is a hard
  requirement for webtop/no-GPU boxes, not something a GPU path supersedes.
- **`scoot-core` stays platform-independent** (events and actions in,
  arrangement out; no Wayland, no I/O). A future macOS Accessibility-API
  adapter depends on this. Don't touch it without a measured reason once it's
  fuzz-tested and reviewed clean.
- Smithay is pinned to a specific git rev (see `Cargo.toml`) — verify claims
  about its behavior against the pinned source, not general Smithay
  knowledge; APIs and semantics shift across revs.
- Prefer CPU-friendly rendering and damage-limited redraws throughout.
- **"Compositor," not "window manager," in anything user-facing** (`README.md`,
  crate descriptions, `--help` text). Wayland has no separate window-manager
  protocol the way X11 did — whatever composites client surfaces also arranges
  them, so there is no standalone Wayland window-manager process. Matches how
  niri/sway/Hyprland self-describe ("compositor" first, "tiling window
  manager" as the behavior description). Exception: `scoot-core`'s crate
  description really is just "window management state and layout" — it has no
  compositor concerns (no Wayland, no I/O), so that wording is accurate and
  should not change to match this rule.
- **Where a Wayland protocol standard exists (emerging or established),
  implement that rather than a bespoke alternative, unless there's a concrete
  reason it doesn't fit.** E.g. `ext-workspace-v1` over the older one-off wlr
  workspace protocols. This is also why `scoot-ipc`'s protocol is scoped to
  what genuinely needs to be scoot-specific (agent-driven input injection,
  screenshots, window/action queries) rather than reinventing something a
  standard protocol already covers for external tools like bars — see the
  protocol inventory in `docs/backlog/protocols/`.

## Engineering standards

Write clean, decoupled, idiomatic Rust: well-organized modules, no very large
files. Add and run tests as each piece lands, not batched at the end.

**Priority order when things trade off:** speed/efficiency, bug-free,
well-architected, loosely-coupled code first — beauty (visual polish,
decorations, theming) second. If a visual feature would cost meaningful
performance, correctness risk, or coupling, the engineering bar wins; polish
waits or gets a cheaper implementation.

**This is a compositor people depend on to get their work done — it must be
snappy and reliable, always.** A concrete bar every change is held to:
- Avoid heap allocation on any hot or per-event/per-frame path (input
  dispatch, the render loop, IPC dispatch) — reuse and pool (persistent
  buffers built once, updated in place), as the render buffers already do.
- Treat a plausible crash/hang/panic path with the same severity as data
  loss: a compositor crash takes every client's unsaved state with it.
- Consider edge cases explicitly (zero/one window, output-edge clamping,
  malformed input, hardware/session failures, events at their real maximum
  rate), not just the happy path.
- Benchmark before/after whenever a change touches a hot path, asked for or
  not.
- **Never defer a user-facing harm to a later stage.** (User, 2026-09-19,
  on PR #130.) If a change can disconnect a client, lose their work, or
  wedge their session, "documented and scheduled for stage N" is not a fix
  — staging is for scope, never for harm. Land it behind a flag that cannot
  reach the harm, or fix it, or do not merge.
  **The corollary is the half that takes discipline: establish whether the
  harm is real before you either defer it *or* block on it.** The same PR
  is the worked example. A failing test named
  `an_import_through_create_immed_is_not_a_client_kill` looked like proof of
  a reachable client kill, and the coordinating session said so. Review then
  probed the actual EGL device and found the advertised formats were a
  *subset* of what the renderer could import, that the refusal was the test
  allocator's `udmabuf` provenance rather than any format, and that nothing
  on that machine could produce a GBM dmabuf to reach the path at all — so
  the fix everyone had scheduled would not have fixed those tests. An
  alarming test name is a reason to go and measure, not a verdict. Blocking
  on a phantom costs real work; deferring a real one costs a user.

**Per-feature cycle**, repeated until stable before starting the next thing:
build → test → bug bash → optimize → benchmark → independent review → commit
→ PR → merge (the review gate below replaces a per-PR approval ask). Not
"make it work, polish later" — a feature is done only once it has passed the
full cycle. Split modules before they sprawl (tests in their own file), and
run `nextest`, `clippy -D warnings` and `fmt --check` after each step.
Beyond "does it work": actively look for bugs, not just the happy path;
measure performance with real before/after numbers. **Update `README.md` in
the same PR a feature lands, not later** — and more than the Status/Running
sections: every new/changed config option, default keybinding, CLI flag, or
IPC request/action a user or integrating agent would need to know. This
project let the config file and keybinding reference go stale across several
merged PRs before anyone noticed; `scoot-implementer` and `scoot-reviewer`
both call this out explicitly so it doesn't recur.

**Independent review is mandatory, and it's a real gate, not a formality.**
Use the `scoot-reviewer` subagent (`.claude/agents/scoot-reviewer.md`) — or,
if it doesn't exist in this checkout, an equivalently-instructed pass —
before any merge. The bar is stellar, principal-level-engineer code. A
subagent's "tests pass" self-report is a claim to independently re-derive,
not a verdict to relay: re-run build/test/clippy/fmt yourself against real
hardware where relevant, and trace what any new or changed field/flag means
everywhere it is read or written, not just at its introduction site. Worked
example (see `docs/roadmap/05b-vt-switch-eperm.md`): a fix gated `change_vt`
on `Tty::active`, which passed every test and read clean — but `active` meant
two different things at its two write sites (DRM master lost vs. session
paused), and conflating them silently broke the project's one hardware
recovery path. That class of bug is invisible to "compiles and passes tests"
and only shows from reading what a field means across every site touching it.

**Merge authority is delegated, standing on the review gate — not a per-PR
ask.** (User, 2026-09-12, after several PRs of check-ins: "Yes. For the last
time. You get to be the final gate after reviewing the review agent's
diagnosis." This supersedes any earlier "ask every time" framing.) The
coordinating session is the final gate: once `scoot-reviewer` (or an
equivalently rigorous pass) has reported back and the session has actually
read and weighed its diagnosis — not just seen "no blocking findings" and
moved on — merge without asking again. `gh pr merge` is unblocked mechanically
by the `Bash(gh pr merge:*)` rule in `.claude/settings.json` (the auto-mode
classifier otherwise denies it, reason "Merge Without Review"). The judgment
that still matters every time: did the review actually happen and come back
clean (or were its findings addressed and re-verified, not just noted)? A
blocking finding, an unclean test run, or a review that couldn't verify
something important (and said so) means don't merge, and explain why — the
gate is real, it's just no longer a chat round-trip when it passes.

**Self-modification note:** Claude Code refuses to let an agent write to its
own `.claude/` config (settings, agent definitions) in normal auto-mode, even
on direct chat instruction — a deliberate guardrail (the protected-path check
runs *before* `permissions.allow`, so no settings rule can lift it). Two real
paths around it, neither a workaround of its intent: (1) the user runs the
command themselves; (2) the session runs in `--dangerously-skip-permissions`
(bypass) mode, which the Claude Code docs intend only for isolated/disposable
environments — treat write access gained that way as scoped to the task at
hand, not a standing license, and keep the same care around destructive or
hard-to-reverse actions (the safety net isn't there to catch a mistake). If
asking the user to paste a multi-line heredoc themselves: this failed
non-obviously once when the user was on a phone — newline-dense text (like a
YAML frontmatter block) got silently corrupted (lines merged, then whole
spans lost) in the mobile input path. Prefer very short single-line commands
when the user is likely on mobile, and ask them to verify the result (e.g.
`cat -et` or a line count) rather than assuming a paste landed intact.

## Verification and evidence (avoiding redundant hardware work)

The standard verification set for any compositor change: `cargo nextest run
--workspace`, `cargo clippy -p scoot --all-targets -- -D warnings`, `cargo
fmt --check -p scoot`, and `scripts/smoke-test.sh` (backend-agnostic
IPC-driven end-to-end test; set `MODE=--nested` to run under a host
compositor, and real `--tty` hardware needs the binary launched there — see
the script's header).

**nextest is the runner to lean on** (user, 2026-09-19). It is the stricter
of the two and it guards the more important property — see below. `cargo
test -p scoot` is no longer a required per-change step; it stays the
fallback for a machine without nextest, and it is what CI runs
(`.github/workflows/ci.yml`, which says in as many words that it is not a
duplicate of the nextest step) so the coverage nextest structurally cannot
provide is not simply dropped.

Why nextest is the one to keep: it runs each test in its own process, which
is how a test that only passed because it shared
`smithay::utils::SERIAL_COUNTER` (process-global, and several suites say so
in as many words) with its neighbours gets caught.

**The doctest reason this used to give was wrong, and is corrected here
(2026-09-19).** It said `cargo test` was kept because only it runs
doctests. This workspace has **zero** doctests — `cargo test --doc` reports
`0 passed` for `scoot-core` and `scoot-ipc`, and for `scoot` it cannot run
any at all (`error: no library targets found`, it being a binary crate). So
that justification was vacuous.

The real reason to run both is that they catch **opposite** failure modes,
and neither tool can cover the other's: nextest isolates each test in its
own process, so it catches a test that only passed by *sharing* process
state — and structurally cannot see cross-test interference, having removed
it. `cargo test` runs them concurrently in one process, so it catches a test
that *breaks* when sharing, and cannot see the coupling nextest catches.
There is no flag on either tool that bridges this.

That asymmetry is why nextest is the one to lean on: coupling can mask a
*product* bug (a test passing that should not), while interference is
usually a *test-quality* bug.

**What leaning on nextest gives up, and how you would notice.** With every
test in its own process, a test that breaks when it shares one is invisible
locally. The tell is `cargo test` failing while `nextest` passes — which is
what CI should run `cargo test` for, and why a contributor reporting
exactly that shape is reporting a real bug rather than a flake. The standing deeper fix
is to remove the root cause: tests that compare serials relatively instead
of asserting absolute values would not care which runner ran them, and then
one runner would genuinely suffice.

nextest is installed on the dev VM (`vm/configuration.nix`); a machine
without it still has the `cargo test` baseline, which needs no extra tool.

The first four are cheap to re-run in full every time, no exceptions —
`cargo`'s incremental cache means re-running them after a successful build
costs almost nothing, and this is exactly where a fabricated or merely-wrong
"it works" self-report gets caught cheaply. Independent review never skips
these on the theory that the implementer already ran them.

Real hardware bug-bash and benchmarking (VT-switch cycles, jiffies-delta
sampling, IPC screenshot capture) is the genuinely expensive part — real
wall-clock time, SSH round trips, state `cargo` can't cache. Treat it the way
a task runner treats a cache: the implementer's report is the cache entry,
keyed by the exact commit/tree state it was captured against, valid only
unmodified.
- **The implementer records, not narrates**: the exact commands run, the exact
  commit SHA (or "uncommitted, working tree as of \<describe\>"), and raw
  output/artifacts (real screenshot paths, raw sampled numbers) — not a
  paraphrase. A claim with no reproducible evidence is treated as unverified.
- **The reviewer checks the cache key first**: if the code changed at all
  since that SHA (including a review-driven fix), the recorded evidence for
  what it touched is stale and must be redone.
- **If the key still matches, spot-check rather than fully redo**: re-run at
  least one representative scenario live (ideally the one most central to the
  diff's risk) to rule out fabrication, and audit the rest for methodological
  soundness (did they wait for idle before sampling? is the screenshot's
  content consistent with the claim?) rather than repeating every scenario.

## Local scratch/handoff state

`HANDOFF.md` at the repo root is gitignored — the convention for transient,
session-specific state (open PRs awaiting merge, which VMs are up, what's in
flight) that goes stale within hours and isn't meant to be shared or
reviewed. It's distinct from this file (durable, shared, git-tracked
doctrine) and from a user's own Claude memory (durable, user-specific,
cross-project). Keep it current as you work and keep it short — anything
durable belongs in `CLAUDE.md` or `ROADMAP.md`/`docs/`, not duplicated here.

## Process notes

- Three agent roles live in `.claude/agents/`: `scoot-orchestrator.md` (the
  coordinating session's role — delegate, gate on review, merge, repeat),
  `scoot-implementer.md` (implements one item per invocation, full cycle,
  never merges), `scoot-reviewer.md` (independent review gate, never edits or
  merges). Read the relevant one before acting in that role. These were
  renamed by the user in a separate, user-run pass, because `.claude/` is a
  protected path no agent may write — chat approval cannot lift that check,
  which is the point of it. **The local checkout directory is still
  `code/flexwm`** and must stay so: the dev VM's running 9p share points at
  that exact path.
- Delegate implementation to a **fresh, non-fork subagent**
  (`scoot-implementer`, or general-purpose if it doesn't fit) to keep the
  coordinating session's context lean. A `subagent_type: fork` inherits and
  re-sends the entire growing transcript every time it's spawned — fine
  occasionally, but it compounds across a long session; reserve fork for cases
  that genuinely need the parent's full context. The coordinating session is
  the orchestrator and the actual final gate — not a pass-through for
  implementer or reviewer self-reports.
- An agent that returns near-instantly with zero tool calls and just restates
  the task has not done the work — resume it with an explicit instruction to
  use its tools; don't accept the report.
- Both dev VMs (linux-builder on `:31022`, the scoot dev VM on
  `ssh -p 2222 dev@localhost`) may already be running — check (`nc -z localhost
  31022` / `nc -z localhost 2222`) before starting either. See `vm/README.md`
  for setup, boot, and troubleshooting, including the `NIX_SSL_CERT_FILE` and
  tty-suspend/disk-lock gotchas.
- Don't restart, kill, or otherwise manage either VM process from an agent
  session by default — if one is down, say so and ask rather than assuming
  ownership of its lifecycle. (Exception: if the user asks directly, be
  explicit that doing so makes the VM a child of that session's process tree.)
- **Never run branch-mutating git commands (`checkout`, `commit`, etc.) in the
  shared main checkout (`/Users/steveyackey/code/flexwm`) while a background
  implementer might still be active there.** Implementers are told to work
  directly in that checkout, not a worktree (see `scoot-implementer.md` — the
  dev VM's 9p mount points at that exact directory, unreachable from a
  worktree elsewhere without a manual copy-over). A background agent's own
  `git checkout -b` can silently switch the branch a concurrent orchestrator
  action lands on — this happened once: an orchestrator-side wording fix
  landed on the implementer's in-progress branch instead of the intended one,
  and `gh pr merge --delete-branch`'s local cleanup also touches whatever
  branch is checked out. No work was lost (recovered via `git reflog` and
  cherry-picked), but it was avoidable. If the orchestrator must touch the
  repo while an implementer may be active (fixing a nit, updating `ROADMAP.md`
  or `docs/`, merging a PR), do it from a throwaway `git worktree add
  /tmp/<name> <branch>` and remove it after pushing. Skip the worktree only
  once no implementer is active.
  **"Read-only agent" does not exempt the agent's git commands, and the
  orchestrator is the one who usually forgets this** (2026-09-21, PR #210).
  `scoot-reviewer` never edits, commits or merges — and that is exactly why
  it reads as safe to tell it "check out `<commit>^` if it helps verify the
  before/after claim", which is what the dispatch prompt did. `git checkout`
  is branch-mutating whoever runs it. The reflog of that session shows what
  followed: `HEAD` moved **three** times, not once —
  `22:42:09 asahi-tier-bench → asahi-tier-bench` (harmless),
  then `23:01:32 asahi-tier-bench → main`, then
  `23:02:08` back and off again. The 23:01:32 move is the one that matters:
  the reviewer took the shared checkout off the PR branch onto `main` as
  *cleanup*, believing `main` was the state it had found, and it sat there
  until the orchestrator's own `23:04:52` checkout. Nothing was lost only
  because no commit happened in that window — timing, not safety. Two
  lessons, and the second is the less obvious one: ask a reviewer to inspect
  a historical commit in its **own** `git worktree`, or hand it the artifact
  (a diff, a built binary, a store path) instead of a commit to check out;
  and note that an agent *restoring* what it believes was the original branch
  is its own hazard, because its belief about the original is a snapshot from
  whenever it started.
  (An earlier version of this entry said the checkout "came back a no-op …
  and nothing was lost", which was written from the one no-op move and missed
  the other three. Review caught it in the reflog. A near-miss recorded
  mildly teaches the mild lesson.)
- **The same collision can happen between two implementers, not just the
  orchestrator and one — this also happened once.** Two `scoot-implementer`
  agents were dispatched close together, reasoned safe because their file sets
  didn't overlap (one touched `session_lock.rs`/`headless.rs`/`shell.rs`, the
  other `tty/mod.rs`/`cli.rs`) — but file-level non-overlap doesn't prevent
  branch-level collision: both still needed `git checkout -b <branch>` in the
  same shared checkout. The second checkout silently moved the first's `HEAD`
  out from under it mid-session; the first caught it via `git reflog` (an
  unexpected `checkout: moving from <its-branch> to <the-other-branch>`) and
  recovered by finishing in a throwaway worktree, shipping to the dev VM by
  `tar` over ssh instead of the 9p mount (which then reflected the other
  branch). No work was lost, but the implementer had to notice and adapt
  mid-task rather than the orchestrator preventing it. The same overlap also
  caused a dev-VM **hardware** collision on the same pair: a `--tty` VT-bound
  seat can be held by only one process, and a reviewer's `--gpu` verification
  failed with a *different* errno (`EPERM`, seat busy) than the one under test
  because another agent's benchmark was running `--tty` at that moment —
  caught only because the errno didn't match the expected mechanism, fixed by
  re-running once the other agent's VM work finished. **When dispatching more
  than one implementer (or an implementer and a reviewer) concurrently, tell
  at least one explicitly to work in its own `git worktree`** (with the same
  ship-to-VM-over-ssh fallback if it needs dev-VM hardware and the 9p mount is
  on a different branch) **and to check for other active agents before
  claiming the `--tty` seat or trusting a benchmark number** — disjoint file
  sets do not make concurrent dispatch safe.
