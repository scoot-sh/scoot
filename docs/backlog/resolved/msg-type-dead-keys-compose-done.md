---
title: "`flexwm msg type` still can't produce a character that needs a dead key, a compose sequence, or a layout the session isn't currently on (LOW). — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `flexwm msg type` still can't produce a character that needs a dead key, a compose sequence, or a layout the session isn't currently on (LOW). — RESOLVED (PR #121)

## What it said

`flexwm msg type` covered every character on the active layout at some
shift level, and refused the rest loudly (`no key for \`X\` in this layout`,
or the locks-or-latches message). Three things were out of reach: dead keys
and compose sequences (`é` on plain `us` is `Compose`, `'`, `e`), characters
on an inactive layout group (deliberately unswitched), and levels only a
locking/latching modifier reaches (deliberately unpressed). The ASCII cost
was concrete: `de` and `es` refused `^` and `` ` ``, `pt`/`se`/`no`/`dk`
refused `~` as well -- and `~` bites in practice (paths, globs, regexes).
Prescription for the first: an `xkb::Compose` table plus a second resolution
path when the single-key lookup fails.

## Resolution

**Fixed for two-key dead-led sequences; the other two boundaries stand, and
three-key `Multi_key` sequences are a stated non-goal.** New module
`compositor/input/compose.rs` (plus `compose/tests.rs`), wired as a fallback
in `State::type_text`:

- On the first character the direct path cannot type, the session-locale
  compose table is loaded (`LC_ALL`, then `LC_CTYPE`, then `LANG`, else `C`
  -- the order `setlocale` consults, the same source toolkits read; `C`
  compiles to the default table, measured) and every dead key the *active*
  layout carries is fed through it followed by every keysym on that layout,
  recording pairs that compose to a single character. Built at most once per
  request and never on the direct path, so plain text pays nothing.
- Per character, the first recorded pair whose halves *both* plan in the
  active layout (through the existing `modifiers::plan`, sharing the
  request's probe cache) is pressed as the two keypresses a person would
  type, each half with its own level's modifiers. Both halves resolve before
  either is pressed, so one character is all-or-nothing; the string as a
  whole stays prefix-typed, per the existing contract.
- No Rust-heap allocation: the map and both scratch lists are fixed-size
  stack arrays (512 sequences, 32 deads, 1024 syms; `de` measures 378
  composed pairs from 13 deads over 623 syms). A cap reached truncates with
  a `debug_assert` -- a dropped sequence refuses loudly, never mistypes.
  (Two bounded library-inherent exceptions, documented on the module: naming
  a keysym allocates a short `String`, and the table itself lives on the C
  heap. Both happen at most once per request, only on one that already hit
  an untypable character.)

What now types, all proven end to end against a real client below: `é` on
`de` (`dead_acute` + `e`, four key events), `^` and `` ` `` on `de`
(dead-plus-space), `~` on `se` -- whose session-table-first pair is the
*doubled* press (`dead_tilde` twice, the table's other standard spelling),
with the key sitting at level 2, so eight events -- and, found while
verifying rather than predicted, `ø` on `de`, which resolves and
round-trips byte-exact. All 95 printable ASCII characters now type on all
fourteen swept Latin layouts (unit sweep below; session-table spot probes
for `en_US.UTF-8` and `C`).

What stays refused, each pinned by a test: a character with no dead-led
sequence on the active layout (`é` on plain `us`, which carries no dead keys
and no Compose key -- zero keys sent); a character only on an inactive group
(`ü` with `us,de` in group 0 -- the scan collects deads and plans halves in
the active group only, and a map built for another layout degrades to `None`
rather than typing its keys); levels only a locking/latching modifier
reaches (untouched path, pre-existing pin). Three-key `Multi_key`-led
sequences are not driven even where the layout has a Compose key: searching
them multiplies the scan by the keysym count again, for sessions that opted
into a key most layouts lack. Keybinding interception warns per half, as
before.

**Overlap checked:** the ticket's AltGr note (`@` on `de` needs a modifier
`key` has no name for) is unchanged -- `type` already covered it directly,
and this changes nothing about `key`. `modifiers::plan` itself has zero
hunks in this diff; the direct path is untouched.

## Tests

End to end (`input/tests.rs`, real client, whose decode now runs the same
compose machine a toolkit does -- fed from a minimal inline table so plain
text takes the byte-identical old path, proven by the unchanged suite):

- `a_dead_key_sequence_types_the_composed_character` (`é` on `de` →
  `é`, 4 keys), `a_dead_ascii_character_types_via_its_dead_key` (`~` on
  `se` → `~`, 8 keys), `direct_and_composed_characters_mix_in_one_string`
  (`café` on `de` → `café`, 10 keys) -- all three fail-first (below).
- `a_character_with_no_sequence_is_still_refused_loudly` (`é` on `us`:
  error names it, zero keys) and `a_character_only_on_an_inactive_group_is_refused`
  (`ü` on `us,de` group 0: error names it, zero keys) -- pins, passing
  before and after.

Unit (`compose/tests.rs`, hermetic inline table, real keymaps): dead+base
resolution and replayed keysyms on `de`; empty map on `us`; dead+space gaps
on `se`/`de`; every recorded pair replays to its character; unlisted `α`
resolves to nothing; a `de` map plans to nothing on `us` (stale-map
safety); group-0-only sequences on `de(neo),de` plus `ü` unreachable while
group 0 is `us`; and every dead-ASCII gap from the fourteen-layout sweep
resolving through the fallback.

## Fail-first record

Dev VM, pre-fix tree (branch at `01e0d8d` + tests, implementation absent):
`cargo test -p flexwm --bin flexwm input::` → 34 passed, 3 failed:

- `a_dead_key_sequence_types_the_composed_character`: `the text is typable:
  "no key for \`é\` in this layout"`
- `direct_and_composed_characters_mix_in_one_string`: `the text is typable:
  "no key for \`é\` in this layout"`
- `a_dead_ascii_character_types_via_dead_key_plus_space`: `the text is
  typable: "no key for \`~\` in this layout"`

Post-fix: 106 passed, 0 failed (same filter); full `cargo test -p flexwm`
942 passed; `cargo nextest run --workspace` 1047 passed; `cargo clippy -p
flexwm --all-targets -- -D warnings` clean; `cargo fmt --check -p flexwm`
clean; `scripts/smoke-test.sh` (prefixed) 17 ok, no BUG.

Two findings on the way, both in the tests, not the implementation: the
session table prefers the doubled `dead_tilde` press for `~` on `se`
(measured in-source-order), and `dead_tilde` sits at level 2 there -- hence
8 keys, not 4. The first `~` run also caught the test client missing the
doubled spelling, which a real toolkit decodes; the client's inline table
now carries both spellings per dead key.

## Verified live

Dev VM, debug binary, `XKB_DEFAULT_LAYOUT=de`, `--headless`, real `foot`
(all `msg` exits 0 unless noted):

- `msg type 'café Üß'` then `printf ok-café-ü-ß > file` round-tripped
  byte-exact out of the shell: `od -c` →
  `o k - c a f 303 251 - 303 274 - 303 237` (`é` via the sequence, `ü`/`ß`
  direct).
- `msg type 'ø'` exited 0 and `printf ø > file` read back `c3 b8` --
  the unpredicted `ø` resolves and decodes exactly.
- Screenshot `/tmp/compose-live.png` (77,423 bytes) visually verified:
  foot shows `live-café-6084` and the `ok-café-ü-ß` prompt line.

## Benchmark

Dev VM, release `--headless`, `de` layout, `time flexwm msg type`, 7 reps
(cap-ticket method; its published baselines ~2µs/char plain,
~4.3µs/char shifted):

- Plain 16,384 chars: 0.04s each rep (~2.4µs/char) -- matches: no
  regression on the direct path.
- Shifted 16,384: 0.08--0.09s (~5.2µs/char) -- same ballpark: no
  regression.
- Composed 4,096 `é`: 0.08--0.09s (~21µs/char) -- new capability, ~4x
  shifted per character (four key events plus the failed direct scan, the
  map lookup and two plans). Scaled to the cap, 16,384 all-composed
  characters cost ~0.34s of event-loop time against the 75ms plain-text
  budget. Accepted and stated: that input is 16k accented characters in one
  request (agents split there anyway), and real sizes cost milliseconds --
  a few hundred `é` is ~4--6ms.
- Single `é` (includes the one-time table build plus scan): ~0.01s wall
  against ~0.00s for `e`; unit-measured debug build/scan 2.4ms + 0.5ms.

## Stated limits (unverified, not papered over)

- Session tables beyond `en_US.UTF-8` and `C` are unprobed: the all-ASCII
  claim rests on a table carrying the dead-plus-space pairs, true everywhere
  measured. A locale lacking one refuses that character exactly as before.
- Live proof is `--headless` only. The path is backend-agnostic by
  construction (same IPC `type_text`, same seat keymap the shifted
  characters already take through every backend), with the headless
  harness plus smoke test as the regression net.
- `Multi_key` three-key sequences: decided, not built (see above).
