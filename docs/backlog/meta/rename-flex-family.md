---
title: "Rename `flexwm` to `flex` and split the CLI into `flexctl` — scheduled as the last item of the current backlog burn-down."
status: "open"
area: "meta"
priority: "low"
blocked: "everything else in the backlog burn-down"
---

# Rename `flexwm` to `flex` and split the CLI into `flexctl` — scheduled as the last item of the current backlog burn-down.

**Decided and scheduled, 2026-09-16** (user, via `/goal`, clarified in a
follow-up correction — "flexctl is part of the split"): do this rename
*last*, after every other open backlog item from this burn-down has landed —
not because it is low priority in the usual sense, but because it touches
nearly every file in the tree and every backlog item completed after it would
otherwise need its own diff rebased across it. Scope: **rename the compositor
binary/crates from `flexwm` to `flex`, and split today's `flexwm msg
...`/`type`/`key`/etc. CLI out of the compositor binary into a separate
`flexctl` binary/crate that talks the same Unix-socket protocol from the
outside** — i.e. both section 1 (the rename) and section 2 (`flexctl`) below
are in scope, not just the rename in isolation. Docs updated throughout.
**`flexbar` (section 3) stays out of scope** — a new project, not part of
this rename, undecided and unscheduled unless the user says otherwise.

---

*Original entry, 2026-09-13 — the mechanical rename details below (crates,
textual references, the naming collision check) and the `flexctl` split
design questions both still apply as scoped above; only `flexbar` (section 3)
is explicitly excluded.*

Rename the project from `flexwm` to `flex`, as part of a small family of
tools: `flex` (the compositor), `flexctl` (a CLI/IPC client), `flexbar` (a
companion status bar). User request, 2026-09-13 — not scoped or
scheduled yet, recorded here so it doesn't get lost. This is bigger than a
find-and-replace, in two separable ways:

**1. The rename itself.**

- **Crates.** All three workspace members are named for it:
  `crates/flexwm` (binary, package `flexwm`), `crates/flexwm-core`
  (platform-independent state/layout), `crates/flexwm-ipc` (the IPC
  protocol crate). Renaming the packages means every internal
  `flexwm-core = { path = ... }`/`flexwm-ipc = { path = ... }` dependency
  line, every `use flexwm_core::...`/`use flexwm_ipc::...` import, and the
  binary target name (`cargo build -p flexwm` → whatever the new package
  is called) all move together — a mechanical but wide-reaching change,
  not a one-line edit.
- **The GitHub repo** is `yackey-labs/flexwm`. A GitHub rename leaves a
  redirect from the old URL, but every local clone's `origin` remote
  still points at the old name until updated by hand (`git remote
  set-url`), and anything that hardcodes the URL (this repo's own
  `Cargo-Session`/attribution lines in past commits, any external bookmark
  or CI config) won't follow the redirect automatically. Coordinate the
  repo rename with updating local remotes in the same sitting, not as an
  afterthought.
- **Everything textual**: `README.md`, `ROADMAP.md` itself (including
  every historical entry that names `flexwm` — decide whether history
  gets rewritten or just new entries use the new name), `vm/README.md`,
  `vm/configuration.nix` (service/user names, paths), `HANDOFF.md`,
  `scripts/smoke-test.sh` and any other script that invokes the binary by
  name, and the `flexwm msg`/`flexwm --tty`/etc. CLI surface itself (which
  is also user-facing documentation, per this project's "compositor, not
  window manager"-style naming rules in `CLAUDE.md`).
- **Open question, not yet decided**: do the `.claude/agents/*.md` role
  files (`flexwm-implementer.md`, `flexwm-reviewer.md`,
  `flexwm-orchestrator.md`) and the subagent names they're invoked under
  rename too, for consistency? They're project tooling rather than
  product surface, so this could reasonably go either way — flag it for a
  decision when this is actually scoped, don't assume either answer here.
- **Worth a quick check before committing to `flex`**: whether that name
  collides with anything relevant (an existing crates.io crate, if this
  is ever meant to be published; an existing well-known `flex` CLI tool
  a user might have on `$PATH`, e.g. GNU flex the lexer generator, which
  is a real, extremely common collision to be aware of before locking in
  a bare `flex` binary name).

**2. `flexctl` implies splitting the CLI out of the compositor binary,
not just renaming it.** Today `flexwm msg ...` (and `type`/`key`/etc.) is
one `Command` variant of the single `flexwm` binary (`crates/flexwm/src/
cli.rs`) — the same executable that starts the compositor also sends it
IPC requests, distinguished by argv. A separate `flexctl` binary is a real
design decision, not a rename: does the compositor crate stop exporting a
CLI at all and become `flex --tty`/`flex --nested`/`flex --headless`
only, with everything under `msg` moving to a new crate/binary that talks
the same Unix-socket protocol from the outside? That would cleanly
separate "the compositor" from "a client of the compositor" (useful for
the computer-use goal specifically — an agent shells out to `flexctl`,
not to the compositor's own binary) but is a real crate-boundary change,
probably wanting its own design pass rather than riding along with a
find-and-replace rename.

**3. `flexbar` would be a new project**, not a rename of anything that
exists: a status bar built as a `wlr-layer-shell-unstable-v1` client
(item 14, PR #22), presumably filling the same niche as `waybar` but
purpose-built for this compositor. Worth doing at some point — flexwm/flex
has no bar of its own today, and every hardware screenshot/demo of the
layer-shell work so far uses `waybar`, a third-party dependency, to prove
the protocol works — but it is a whole new binary/crate with its own
scope, feature set and release cycle, not a line item inside the rename.
Should probably be scoped as its own separate roadmap item once the
rename (and the `flexctl` split, if that's the direction) land, rather
than being designed as a rider on this entry.
