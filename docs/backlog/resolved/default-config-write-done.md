---
title: "`--print-default-config --write`: refuse-to-overwrite convenience — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `--print-default-config --write`: refuse-to-overwrite convenience — DONE

Requested as the follow-up `../resolved/default-config-command-done.md`
left open ("`--write` convenience that refuses to overwrite could follow if
wanted"). Resolved 2026-09-21 on branch `backlog/default-config-write`,
coordinator-directed, no gh issue.

Original shape (decided, kept intact): `scoot --print-default-config
--write` writes the emission to the default config location instead of
stdout — and refuses loudly (non-zero exit, naming the path) when anything
already exists there. No custom path argument: stdout composes for every
other destination (`> wherever`).

## What landed

- **Flag** (`cli.rs`): `Command::PrintDefaultConfig` carries `write: bool`;
  `--print-default-config` alone still emits to stdout, `--write` is its
  only trailing argument and anything else (`--wirte`, `extra`) is
  `Error::Unknown`, not a silent stdout emission. `--help` usage line is
  `scoot --print-default-config [--write]`.
- **Write path** (`compositor::config`): `write_default_config()` resolves
  through the now-`pub` `default_path` with the same two live env vars
  `load` reads (no second resolution to drift), creates parents, then
  `create_new` (`O_CREAT|O_EXCL`) with mode `0o600` and writes the
  `default_config_toml()` bytes — one function, not a second formatter.
  `main.rs` prints `wrote <path>` through the EPIPE-quiet contract;
  refusals are `Err`, which `main` reports as `scoot: {error}` on stderr
  with exit 1.
- **Nine tests**: existing-file refusal (untouched byte-for-byte, path
  named), byte-identity file-vs-stdout, missing parents created, `0o600`,
  symlink + dangling-symlink refusal (target untouched, link intact),
  8-thread double-invoke (exactly one winner, winner intact), unwritable
  parent (loud, names path), mid-write-failure cleanup seam, and
  emits-then-loads through `load`.
- **Docs**: `docs/configuration.md` usage line + emit paragraph (`--write`
  semantics, `0o600`, symlink refusal, no custom destination), README
  Configuring one-liner, `--help` line. No `scootctl` surface (out of
  scope, stands).

## Decisions

1. **Symlinks refuse as themselves.** On Linux `O_CREAT|O_EXCL` on a path
   naming a symlink fails `EEXIST` without traversal — verified live (exit
   1, `sentinel` target intact, link still a link) and pinned for both
   live and dangling links. No explicit `O_NOFOLLOW`/`symlink_metadata`
   pre-check: the refusal is the open itself, so there is no
   check-then-open race to close.
2. **Mid-write failure removes the partial file best-effort, then reports
   loud.** A torn config at the default location would read back as
   malformed on the next startup — worse than no file. The unlink is
   best-effort (on a truly full disk there may be nothing left to unlink
   with) and never shadows the write error. Pinned deterministically via a
   read-only-handle seam (`EBADF` standing in for `ENOSPC`); live ENOSPC
   was not simulated (needs a full filesystem or root, neither available —
   the dev-VM 9p mount is read-only from the VM side).
3. **No `fsync`.** `write_all` on an `fs::File` is syscalls, not userspace
   buffering; the OS-crash window between close and durable is out of
   scope for a one-shot config writer.
4. **Unresolvable home writes nowhere**, not to cwd (`NoBaseDir` naming
   both env vars); XDG-unset/empty falls back per `default_path`, the same
   function the loader uses.
5. **Success summary is `wrote <path>` on stdout** (EPIPE-quiet per the
   output contract); nothing is printed on the stdout path that changed.

## Evidence

Cheap set, all on the dev VM (`ssh -p 2222 dev@localhost`, tree at
`/mnt/scoot`, `CARGO_TARGET_DIR=/var/cargo-target`), branch
`backlog/default-config-write`:

- `cargo nextest run --workspace` — 1338 passed, 6 skipped.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check --all` — clean (both toolchains `rustfmt 1.9.0`).
- Load-bearing proof: with `create_new` temporarily neutered to `create`,
  exactly the three pins covering it fail (`write_refuses_an_existing...`,
  `write_refuses_a_symlink...`, `concurrent_double_invoke...` with 8
  winners instead of 1); restored afterwards. Pre-fix the trailing
  `--write` was silently ignored (the old parse arm took no arguments),
  so the flag is new behavior by construction.
- Live (`/var/cargo-target/debug/scoot`): fresh `--write` exits 0,
  prints `wrote <path>`, `mode=600`, `cmp` byte-identical to stdout;
  second invoke exits 1 naming the path with sha1 unchanged
  (`2731faa1...` both sides); symlink and read-only-parent refusals exit
  1 naming the path (target `sentinel` intact, link intact); concurrent
  pair leaves one `wrote` + one refusal with the winner `cmp`-intact;
  XDG-unset/empty fall back to `$HOME/.config`, neither-set refuses
  loud (exit 1); closed stdout+stderr gives exit 0 on success / 1 on
  refusal with no panic; `--wirte` and trailing junk exit 1 as unknown
  arguments; stdout `| head -c0` still exits 0.
- `scripts/smoke-test.sh` skipped: the diff adds a first-arg flag and a
  cold file writer and touches nothing the smoke test exercises
  (session startup, IPC, rendering); the new paths are covered above.
- Benchmark n/a: a cold one-shot CLI (one `open` + one `write_all` of
  ~90 lines); no hot or per-event path changed.

## Left out, with why

- Custom destination paths (stdout composes; its own ticket if asked).
- Overwriting/merging with an existing file (the ticket's refusal *is*
  the feature).
- `--write` on `scootctl` (compositor-binary home stands — the Keysym-
  dependent defaults live there; noted, not relitigated).
- Emission content itself (untouched; the anti-drift pins cover both
  transports through the one function).
