---
title: "Make every --help agent-friendly: complete, structured, example-led, machine-readable"
status: "open"
area: "ipc"
priority: "high"
blocked: null
---

# Make every --help agent-friendly: complete, structured, example-led, machine-readable

Filed 2026-10-04. The maintainer: "make sure our help commands are very agent
friendly." Serves **computer use** first (an agent learns scoot from `--help`
before anything else) and **daily-drive** (people read it too).

## The gap

Each binary hand-writes its help (`crates/scoot/src/cli.rs` `USAGE`,
`crates/scootctl/src/cli.rs` with `REQUESTS_HELP`/`ACTIONS_HELP`
single-sourced into `scoot --help`, `crates/scootbar/src/cli.rs` assembled
per module, `crates/scootbg/src/cli.rs` `USAGE`/`DAEMON_HELP`). Nobody has
audited them as an agent would use them: whether every command, flag, action,
config key and exit code is discoverable from the binary alone, whether there
are examples, and whether any of it is machine-readable.

## What to do

Audit all four (and `scoot msg`), then make them meet one contract, written
down in `docs/` and pinned by tests:

- **Complete and discoverable from the binary:** `X --help` and
  `X help [TOPIC]` / `X SUBCOMMAND --help` for every subcommand; topics for
  the action grammar, the request list, config keys, environment variables
  and exit codes. Nothing only in the docs.
- **Example-led:** each command shows one or two real invocations with the
  output shape, the first thing an agent copies.
- **Stable and plain:** to stdout, exit 0, no color or pager when not a TTY,
  wrapped under 100 columns, the same section order in every binary (usage,
  description, commands, options, examples, exit codes, see also with the
  docs site's URL and its `/llms.txt`).
- **Machine-readable:** `--help --json` (or `help --json`) emits the same
  content as JSON (commands, flags with types and defaults, actions with
  argument grammar, exit codes), versioned, from the same single source as
  the text so they cannot drift; a test diffs them.
- **Errors that teach:** a usage error names what was wrong, the nearest
  valid choice ("did you mean"), and the help topic to read, on stderr with
  a distinct exit code (`2`, as scootbar already does).
- **Single source:** keep or extend the existing single-sourcing (scootctl
  and scoot print the same grammar); no hand-copied lists.
- Measure: an agent with no docs, only the binaries, completes a short
  script of tasks (open a terminal, move it to workspace 2, screenshot one
  output, set a bar value, change the wallpaper) using only help output;
  record what it got wrong before and after.

## Not in this ticket

The docs site itself (`docs-site`); shell completions (a later ticket can
generate them from the JSON).
