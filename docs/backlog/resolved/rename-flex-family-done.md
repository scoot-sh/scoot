---
title: "Split the CLI out of the compositor binary into `scootctl` — both halves landed; closed 2026-09-20."
status: "resolved"
area: "meta"
priority: "low"
blocked: null
---

# Split the CLI out of the compositor binary into `scootctl` — CLOSED, both halves landed

Closed 2026-09-20 by coordinator verification (no code change): the rename
half landed 2026-09-18 (PR #128, `flexwm` → `scoot`, no compat fallback) and
the `scootctl` half landed 2026-09-20 (PR #157, new lib+bin crate, `scoot
msg` kept as a permanent alias, Darwin default is `scootctl` — see
[`scootctl-split-done.md`](./scootctl-split-done.md)). The `.claude/agents/`
role files were renamed by the user in a separate pass (zero residual
`flexwm` under `.claude/`). A status bar stays out of scope, as the entry
always said.

Remaining `flexwm` spellings in the tree are all deliberate, verified
2026-09-20: the recorded-evidence archives (`docs/backlog/resolved/`,
`docs/roadmap/`, `Asahi.md` run records — renaming those would falsify what
was run), `vm/README.md`'s pre-rename migration instructions (functional),
and the on-disk checkout path `/Users/steveyackey/code/flexwm` (the dev VM's
9p share points at that exact directory; it did not move).

*(Original filename kept in spirit — `rename-flex-family.md` — so existing
links keep resolving; `flex` is not the name that was chosen — see below.)*

## What landed, 2026-09-18: the rename, as `scoot`, not `flex`

`flex` was dropped before any code moved: crates.io already carries an
unrelated, active `flex` crate, and a bare `flex` binary collides with GNU
flex the lexer generator on every `$PATH` that has it — the collision check
this entry asked for, answered. The user chose **`scoot`**, which passed the
same availability check, and owns `scoot.sh`.

Done in one mechanical PR (#128), behavior-preserving except for the
user-visible names below, which changed with **no compatibility fallback** to
the old ones (`CLAUDE.md` rules out compat shims, and the break is meant to be
clean):

- crates `flexwm`/`flexwm-core`/`flexwm-ipc` → `scoot`/`scoot-core`/`scoot-ipc`
  (`git mv`, so history follows), binary `flexwm` → `scoot`
- IPC socket `flexwm.sock` → `scoot.sock`, `$FLEXWM_SOCKET` → `$SCOOT_SOCKET`
- config `$XDG_CONFIG_HOME/flexwm/config.toml` → `.../scoot/config.toml`,
  `~/.config/flexwm/` → `~/.config/scoot/`. There is no `/etc` config path
  and never was — the loader reads the XDG location only (`config.rs`), and
  the `/etc/...` string that looks like one is an arbitrary `--config`
  argument inside a `cli.rs` unit test.
- GitHub repo `yackey-labs/flexwm` → `scoot-sh/scoot` (transferred by the user
  before the PR; the local remote and every in-tree URL follow it)
- the dev VM's share moved `/mnt/flexwm` → `/mnt/scoot`, `$FLEXWM_SRC` →
  `$SCOOT_SRC`, state dir `~/.local/state/flexwm-vm/` → `scoot-vm/` (needs a
  VM rebuild plus the `mv` in `vm/README.md` to keep an existing disk)
- the names clients and operators see at runtime followed too: the `wl_seat`
  name, `wl_output`'s `make` (and the `xdg_output` description built from
  it), the `--nested` window's own title/app-id, the IPC refusal messages
  that name the compositor, the staging socket prefix in `$XDG_RUNTIME_DIR`
  (`.flexwm-` → `.scoot-`), the screenshot worker's thread name and the
  `memfd` names. A client that matched on any of those strings sees the new
  one, with no alias.

Two deliberate exceptions to "rename everything", both recorded here so a
later grep for `flexwm` does not read as unfinished work:

- **`docs/backlog/resolved/` and `docs/roadmap/` (the numbered files) keep the
  old name verbatim.** They are the recorded-evidence archive: renaming
  `/nix/store/…-flexwm-0.1.0.drv`, a typed-input string a pixel-diff was
  measured against, or a screenshot path would falsify a record of what was
  actually run. Both index READMEs say so.
- **`.claude/agents/flexwm-*.md` are untouched** — a protected path no agent
  may write. Their filenames *and* their contents (`/mnt/flexwm`,
  `cargo test -p flexwm`) are the user's own separate pass. That answers the
  open question this entry carried: the role files do rename, just not from
  inside an agent session.

## What remains: `scootctl` — LANDED

> **Resolved — see
> [`docs/backlog/resolved/scootctl-split-done.md`](../resolved/scootctl-split-done.md).**
> New `scootctl` lib+bin crate; `scoot msg` kept as a permanent alias
> parsing/running through it; Darwin default is now `scootctl`; no wire
> change. The design answers below all held as written. Follow-ups live in
> the resolved record (`--print-default-config`, a possible future `scoot
> msg` removal), not here.

Unchanged in substance from the original entry (below), with `flexctl` read as
`scootctl`: today `scoot msg …` (and `type`/`key`/etc.) is one `Command`
variant of the single `scoot` binary (`crates/scoot/src/cli.rs`) — the same
executable that starts the compositor also sends it IPC requests,
distinguished by argv. A separate `scootctl` binary is a real design decision,
not a rename: does the compositor crate stop exporting a CLI at all and become
`scoot --tty`/`--nested`/`--headless` only, with everything under `msg` moving
to a new crate/binary that talks the same Unix-socket protocol from the
outside? That would cleanly separate "the compositor" from "a client of the
compositor" (useful for the computer-use goal specifically — an agent shells
out to `scootctl`, not to the compositor's own binary), and it is a real
crate-boundary change wanting its own design pass, which is why it was split
out of the rename PR rather than riding along with it. Things it has to
answer:

- what `scoot msg` itself becomes — removed outright, or kept as an alias
- where the macOS story lands: today the compositor is `cfg`'d out on Darwin
  and the same package *is* the client (`flake.nix` leans on that, and
  `nix build` on a Mac produces a client-only `scoot`); with a separate
  `scootctl` crate that conditional could go away entirely
- which crate `crates/scoot/src/msg.rs` and the CLI parsing in `cli.rs` move
  to, and what `scoot-ipc` already exports that both would share

**A status bar (`scootbar`, `flexbar` in the original) is still out of
scope** — a whole new binary/crate with its own scope and release cycle, not a
line item here.

---

*Original entry, 2026-09-13, kept for the record. Section 1 (the rename) is
the work that landed above; section 2 is what remains; section 3 is excluded.
The `flexwm` spellings in it are the original text.*

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
