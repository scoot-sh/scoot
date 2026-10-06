# Agent-friendly help: the contract every binary meets

An agent learns scoot from `--help` before anything else, so every binary's
help meets one contract, pinned by tests. People read it too.

## The contract

- **Complete from the binary:** `X --help` and `X help [TOPIC]`
  (`X SUBCOMMAND --help` where a binary has subcommands with their own
  parsers: `scootbar daemon --help`, `scootbg set --help`). Topics cover the
  command list, the action grammar, config keys, environment variables and
  exit codes. Nothing load-bearing lives only in the docs.
- **Example-led:** each command shows one or two real invocations with the
  output shape, the first thing an agent copies.
- **Stable and plain:** to stdout, exit 0, no color and no pager, wrapped
  under 100 columns, the same section order in every binary: usage,
  description, commands, options, examples, exit codes, environment
  (`XDG_*` and friends where the binary reads them), see also (with the
  docs URL and its live `/llms.txt`, `https://www.scoot.sh/llms.txt`).
- **Machine-readable:** `--help --json` (or `help --json`) emits the same
  content as JSON, versioned with `schema_version` (currently `1`; a script
  should refuse what it does not know). Text and JSON render from the same
  tables, and a test diffs them: every tabulated name appears in both.
- **Errors that teach:** a usage error names what was wrong, the nearest
  valid choice ("did you mean"), and the help topic to read, on stderr with
  exit code `2`. Garbage that is close to nothing stays a bare refusal.
- **Single source:** the request/action grammar, the flag lists and the
  module registry each have one owner; help renders them, never a second
  copy. The typo guesser and the docs URL live once in `scoot-ipc`, which
  every binary renders from, so a domain move is one edit. No new
  dependencies for any of it beyond that (hand-rolled parsers, `serde_json`
  and `scoot-ipc`, all already in the tree).

## Topics and JSON per binary

| Binary | `help` topics | JSON |
| --- | --- | --- |
| `scoot msg` | `requests`, `actions`, `exit-codes`, `environment`; `help <verb>` prints one verb's row | requests (syntax, description, example, reply shape), actions (name, args, description), exit codes, environment |
| `scoot` | `config` plus the client's topics via `scoot msg help ...` (`scoot msg` is the only client) | backends (flags with what each takes and its default), the client document embedded, config sections |
| `scootbar` | `daemon`, `msg` | commands, daemon flags (types and defaults), msg commands, the modules in this build with their actions, exit codes, environment |
| `scootbg` | one per command (`daemon`, `set`, `clear`, `query`, `version`, `kill`, `apply-config`) | commands, `set`'s modes and filters, exit codes, environment |

One spelling note: a client verb's own row is `scoot msg help <verb>`,
not `scoot msg <verb> --help` -- `--help` after a verb would be ambiguous
with what the verb itself takes (`scoot msg type --help` types the text
`--help`; `action spawn --help` runs the command
`--help`). Bare `help` works wherever `--help` does (`scootbar msg help`,
`scootbg set help`).

## Exit codes

`0` for success (help and `--version` count, including into a closed pipe),
`1` when the request ran and failed (no daemon, a refused request, a failed
write, the compositor going away), `2` for a usage error (an unknown command,
flag or value).

## Environment

`scoot msg` reads `SCOOT_SOCKET`, else `$XDG_RUNTIME_DIR/scoot.sock` (as does `scoot` itself)
and need `XDG_RUNTIME_DIR` to exist; `scoot` also reads `XDG_CONFIG_HOME`
for its config file and the caller's `WAYLAND_DISPLAY` for `--nested`.
`scootbar` reads `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`
(`scootbar-DISPLAY.sock`) plus `XDG_CONFIG_HOME` for `bar.toml`; `scootbg`
reads `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` (`scootbg-NAME.sock`) plus
`XDG_STATE_HOME` for its profiles.
