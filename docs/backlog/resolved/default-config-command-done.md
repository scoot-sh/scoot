---
title: "No way to emit a default config file — DONE."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# No way to emit a default config file — DONE

Requested 2026-09-19. Resolved 2026-09-20 as decided, with the ticket's
shape intact: `scoot --print-default-config > ~/.config/scoot/config.toml`
emits to stdout (never a path, so it cannot clobber), generated from the
live defaults, in the commented-example format modeled on
`docs/configuration.md`, with the parse-back test closing the loop.

## What landed

- **`scoot --print-default-config`** (new `cli::Command` variant, first-arg
  flag like `--help`): prints `config::default_config_toml()` through
  `scootctl::output::write_str`, so it inherits the client output contract
  -- a closed stdout (`| head -c0`) is a quiet exit 0, pinned live, not a
  panic. `--help` names it on its own usage line.
- **The emitter** (`config::default_config_toml`, ~150 lines): every current
  table/field, including `[autostart]`, from `Config::default()`,
  `Appearance::default()`, `Keybindings::default()`, scale 1.0, no gpu, no
  renderer backend, no autostart entries. Live section headers, every key
  present and commented with its default as the value -- so the file as-is
  parses back to *all* defaults exactly, not just `Config` (stronger than
  the ticket's loop-closing test asked).
- **Five tests**: the round-trip pin, the every-section pin, the
  byte-identical pin, the commented-keys + every-default-bind-loads-back
  pin, and the translucent-alpha spelling pin; plus two CLI tests (parses to
  its own command, `--help` names it).
- **Docs**: `docs/configuration.md` usage block + "The config file"
  emit pointer, README Configuring one-liner, `--help` USAGE line.

## Decisions

1. **Home: the compositor binary** (coordinator decision 1, confirmed in
   the code): the defaults live in the Linux-gated config module
   (Keysym-dependent), so the emitter lives there too. On a machine with no
   `scoot` binary (macOS, where only `scootctl` builds) the flag parses and
   answers a clean error pointing at `docs/configuration.md` instead --
   verified live on the Mac checkout (exit 1, no panic).
2. **No `--write`** (coordinator decision 3): stdout composes and cannot
   clobber; left out as the ticket marked it "could follow".
3. **No reload annotations**: the ticket allowed them only if trivial, and
   per-field applied/refused markers are not trivial -- the emission's
   header carries one reload line and `docs/configuration.md` owns the
   detail.
4. **Determinism is structural, no sorting**: `[binds]` iterates the
   default table's hardcoded `Vec` order (`Keybindings::iter`, new), and no
   `HashMap` appears anywhere in the emission path -- so two emissions are
   byte-identical by construction (pinned by test and proven live with
   `cmp`). The loader's `HashMap` collision rule is untouched.
5. **Colors use `.round()`, matching the compositor's own channel
   conversion** (`Color`'s pixel value), not truncation. Consequence,
   recorded deliberately: the emitted `background_color` is `#14141a`
   while `docs/configuration.md`'s pixel-sampled value is `#141419` -- one
   LSB in one channel, invisible, and the header comment discloses the
   approximation. The other eight channels agree with the sampled values.
6. **Modifier order is canonical `super+shift+ctrl+alt`** (e.g.
   `super+shift+ctrl+j`); parse-insignificant, and every emitted combo is
   pinned to resolve back through `parse_combo`.
7. **`ChangeVt` emits as a comment, never silently dropped**: the defaults
   hold none (VT binds are session-layered), but the match arm is total, so
   a future default needs no second change.

## Evidence

Cheap set, all on the dev VM (`ssh -p 2222 dev@localhost`, tree at
`/mnt/scoot`, `CARGO_TARGET_DIR=/var/cargo-target`), branch
`backlog/default-config-command`:

- `cargo nextest run --workspace` — 1209 passed, 4 skipped.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --check --all` — clean.
- Live: `/var/cargo-target/debug/scoot --print-default-config` saved twice,
  `cmp` byte-identical (90 lines); `| head -c0` exits 0; `--headless
  --config` with the emitted file boots with a silent log (killed by
  `timeout`, exit 124, zero config warnings).
- Load-bearing proof: a temporary mutation emitting `gap` live with the
  wrong value fails exactly the two pins that cover it (`...parses back...`
  with "the [layout] emission drifted", `...commented...` with "a live
  (uncommented) key line"), while determinism and section pins stay green;
  reverted afterwards.
- `scripts/smoke-test.sh` skipped: the diff adds a first-arg flag and a
  stdout emitter and touches nothing the smoke test exercises (session
  startup, IPC, rendering); the new paths are covered above.
- Benchmark n/a: a cold one-shot CLI (~90 lines of string formatting); no
  hot or per-event path changed (`Keybindings::iter` is emission-only;
  `match_key`/`insert` untouched).

## Left out, with why

- `--write` (ticket's own "could follow"; stdout is the safer default).
- Home-manager schema agreement: the flake ticket
  (`docs/backlog/packaging/flake-consumer-and-home-manager.md`) owns that
  question -- this emission is the format a module schema must agree with.
