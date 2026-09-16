---
title: "Extract a shared test harness for the real-Wayland-client test files, then split the two largest by concern; adopt `cargo-nextest` alongside `cargo test` — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Extract a shared test harness for the real-Wayland-client test files, then split the two largest by concern; adopt `cargo-nextest` alongside `cargo test` — RESOLVED.

## Resolution (2026-09-16)

Done as specced, in seven commits — one per file migrated, then one per
split — so each step is reviewable on its own. Nothing was mocked and no
test was removed: every suite has exactly the test count it had before.

`crates/flexwm/src/compositor/test_support.rs` holds the shared half:
`Harness<S, A>` (the event loop, the `State`, the socket pairs, the client
threads, `run`/`run_on`/`send_step`/`wait_for_ack`/`wait_for`/
`run_expecting_disconnect`/`settle`/`tick`/`disconnect`, the framebuffer
readback, and one `Drop` that no longer joins while unwinding), the
client-side deadline-bounded `wait_for`, and the pixel helpers
(`pixel`/`contains`/`find_color`/`assert_pixel`).

Each suite keeps its own `Fixture` name and every call site through
`type Fixture = Harness<Step, Ack>;` plus an inherent impl — legal because
this is one crate and each suite's `Step` differs. `solid_buffer` and the
`Step`/`Ack` enums stayed put, as the entry asked.

### Line counts, at `c9eaa70` (before) and `189659a` (after)

Note that the entry's own numbers predate PR #44, which had already split
`layer_shell/tests.rs` into `tests/mod.rs` + `tests/popup.rs`.

```
                                  before    after harness    after split
session_lock/tests.rs               2270            2017      736 + 5 files
                                                                (138/222/241/296/431)
layer_shell/tests/mod.rs            2748            2524     1154 + 4 files
                                                                (116/157/392/741)
layer_shell/tests/popup.rs           683             683      683 (unchanged)
ext_workspace/tests.rs              1131            1007      1007
cursor/tests.rs                     1131            1041      1041
activation/tests.rs                 1108            1092      1092
                                   -----           -----
                                    9071            8364
compositor/test_support.rs             -             478
```

707 lines of duplicated harness removed for 478 lines of shared harness,
and no file over 1154 lines — that largest one being `layer_shell`'s
`TestClient` (nine protocols' `Dispatch` impls), its twenty-variant `Step`
and the client script that runs it, which is one cohesive thing and was
left alone.

`ext_workspace`, `cursor` and `activation` came in at 1007–1092 lines and
were **not** split: the entry said to measure first and not to split them
pre-emptively, and a split there would add navigation overhead for ~21–25
tests that already read as one subject.

### `cargo-nextest`

`pkgs.cargo-nextest` added to `vm/configuration.nix` and pushed with
`nixos-rebuild switch --target-host` (closure delta only, no image
rebuild). Guest disk before and after: `1.7G`/`1.6G` free of 16G, i.e. the
addition cost under 100 MiB and did not move the 90% figure.

`cargo nextest run --workspace` on the dev VM: **616 tests run, 616
passed, 1 skipped**, 20.2s. Nothing surfaced — no test in this workspace
turned out to depend on sharing `SERIAL_COUNTER` (or any other
process-global) with its neighbours, so there was no correctness fix in
disguise to make. Recorded because a *clean* run of that check is the
result, not the absence of one.

Documented in `CLAUDE.md`'s verification set and in `README.md`'s "To work
on the code", both as an addition to `cargo test` rather than a
replacement (nextest does not run doctests). `vm/README.md` and
`scripts/smoke-test.sh`'s header were checked: neither documents a test
invocation sequence, so neither went stale.

### Verification, at `189659a`, all on the dev VM (`cd /mnt/flexwm`)

```
cargo test --workspace         522 + 68 + 10 + 3 + 13 passed, 1 ignored, 0 failed
cargo nextest run --workspace  616 run, 616 passed, 1 skipped
cargo clippy -p flexwm --all-targets -- -D warnings   clean
cargo fmt --check -p flexwm                            clean
```

Per-suite test counts, `cargo test -p flexwm -- --list`, before → after:

```
session_lock::tests      40 -> 10 abandoned + 6 blanking + 6 input
                               + 10 lifecycle + 8 teardown   = 40
layer_shell::tests       38 -> 2 adversarial + 5 frames + 19 input
                               + 12 layout                   = 38
layer_shell::tests::popup 14 -> 14
ext_workspace::tests     21 -> 21
cursor::tests            25 -> 25
activation::tests        22 -> 22
```

### Behaviour deltas, stated rather than left to be found

All neutral-or-better, all a consequence of one harness replacing five:

- `Drop` gives each client thread a bounded, *dispatched* wait to finish
  and reports its error, and never joins a thread that has not finished
  (cursor's copy did, which could have hung the suite instead of failing
  it). It still refuses to join while unwinding.
- `layer_shell`'s client-side `wait_for_configure` was bounded by 50 round
  trips; it is now the shared deadline-bounded `wait_for`, for the reason
  `session_lock`'s copy already was one. No test consumed its `Err`. The
  one place that genuinely needs a bounded wait — the popup-configure
  probe, where "no configure" is an outcome to record — keeps its own
  explicit loop.
- `ext_workspace`'s `MapWindow` had a bare `for _ in 0..50 { roundtrip }`
  that fell through without acking if the configure was late; it now fails
  with a named error instead.
- `cursor`'s `run` now settles after each step, which its own did not, and
  its `wait_for` now diagnoses a dead client thread instead of timing out.
- `activation`'s `drive` traded an `AtomicBool` plus a side channel polled
  inside the dispatch loop for two sequential waits. Same ordering; a
  panicking client thread now reports its panic instead of a ten-second
  timeout blaming the compositor.

### Follow-ups, deliberately not done here

- `input_method/tests.rs` (758 lines) and the smaller `dispatch`/`shell`/
  `toplevel_icon` suites carry the same fixture shape. The entry scoped
  five files; these are a straightforward follow-on now that the harness
  exists.
- `solid_buffer` is still per-file. `session_lock`'s and `layer_shell`'s
  are identical modulo the memfd name, so unifying them is a one-line
  generic — left alone per this entry's own "extract what is actually
  identical, leave what only looks similar."

Original entry, left as written:

User question, 2026-09-16: "Why some 1600+ line files of tests?", followed
by a request to decide structure (a `tests/` folder vs. helpers vs.
colocated) and whether a faster test runner is warranted, informed by
idiomatic Rust and with "stellar coverage" as the constraint. This is that
decision, researched and ready to execute — not another survey.

## The five largest test files, and why they're actually that size

```
2563  compositor/layer_shell/tests.rs
2270  compositor/session_lock/tests.rs
1131  compositor/ext_workspace/tests.rs
1131  compositor/cursor/tests.rs
1078  compositor/activation/tests.rs
```

All five (plus `input_method/tests.rs` at 758) share one deliberate choice,
stated outright in `session_lock/tests.rs`'s own module doc: drive a *real*
`wayland-client` connection over a real `UnixStream` pair through a real
`State`, then assert on the wire (what the client was actually told) or on
real rendered pixels — never on which enum variant the render path picked.
That choice is correct and non-negotiable (see "What this is not" below);
it is also why these files are big: each real-client scenario needs a
`Connection`, an `EventQueue`, a registry roundtrip, a `TestClient` struct
implementing `Dispatch` for every protocol object it touches, and a
thread to run the client script on while the compositor's own event loop
is dispatched from the test's main thread. That is boilerplate no unit
test pays.

**The actual finding, from reading all five**: this boilerplate is not
merely large, it is *duplicated*, near byte-for-byte, three and four times
over:

- `run_client(stream: UnixStream, steps: Receiver<Step>, acks: Sender<Ack>) -> Result<(), String>`
  — this exact signature, same channel-driven "run one step, ack it, wait
  for the next" shape, appears independently in `session_lock/tests.rs`,
  `layer_shell/tests.rs`, `ext_workspace/tests.rs`, and `cursor/tests.rs`.
  Only the `Step` enum's variants and each step's handling body are
  genuinely file-specific.
- `wait_for<T>`/`wait_for_configure<T>` — a deadline-bounded (not
  roundtrip-count-bounded — `session_lock/tests.rs` explains why that
  distinction matters, around its render-loop timing) polling helper,
  reimplemented in at least `session_lock/tests.rs` and
  `layer_shell/tests.rs` with no file-specific logic in the body at all.
- `solid_buffer(shm, qh, width, height, color) -> wl_buffer::WlBuffer` — a
  memfd-backed `wl_shm` buffer helper, reimplemented in `session_lock`,
  `layer_shell`, and `cursor`.
- `pixel(pixels, x, y) -> [u8; 4]` and `contains`/`contains_color`/
  `assert_pixel`-style pixel assertions — reimplemented across
  `session_lock`, `layer_shell`, and `cursor`.
- The compositor-side setup — `EventLoop::try_new()` + `State::new(...)` +
  spawn the client thread + dispatch until it signals done or a deadline
  passes — is written inline, per test or per small cluster of tests,
  *within* each file (8 occurrences in `session_lock/tests.rs` alone, 4 in
  `layer_shell/tests.rs`, 3 each in `cursor/tests.rs` and
  `ext_workspace/tests.rs`, 2 in `activation/tests.rs`) rather than through
  one shared function anywhere.

In short: the project has independently converged on the same test
architecture four or five times without ever naming it, and each
reimplementation is a chance for the exact class of bug this repo's own
recent history already produced twice in `activation/tests.rs` during the
`xdg-activation-v1` serial-gate work — a synthetic test client whose
surface never got real focus, so a helper silently returned the *previous*
event's data instead of failing, and the test asserted something true by
accident. A shared, once-reviewed harness does not eliminate that risk, but
it means fixing it once fixes it everywhere, instead of requiring the same
review-cycle rediscovery in every file that copied the pattern.

## Why `tests/*.rs` (Cargo's separate-crate integration tests) is not an option here — confirmed, not assumed

`crates/flexwm` has no `[lib]` target and no `src/lib.rs` — only
`src/main.rs` (plus `cli.rs`, `msg.rs`). A top-level `tests/*.rs` file
compiles as its own crate that can only import a *library* target's public
API; a binary-only crate exposes nothing to import at all, regardless of
`pub`/`pub(crate)` visibility inside it. These tests reach deep into
crate-internal state on purpose — `State`'s fields, `pub(super)`/
`pub(crate)` items like `client_of`, `Recent`, `interaction_serials`,
`headless::Backend` — which is exactly the white-box access an external
`tests/` crate cannot get short of turning most of the compositor's
internals `pub` and adding a `lib.rs` whose only purpose would be
re-exporting test seams. That is a much larger, much worse change than the
one below: it would weaken the crate's actual encapsulation to serve test
organization, backwards from the goal. Ruled out.

(General Rust confirmation, not project-specific: `tests/` integration
tests are their own crate per file precisely so libraries can test their
*public* API as a consumer would — the standard shared-helper answer for
*that* directory is `tests/common/mod.rs`, which does not apply here
because there is no library surface for it to import in the first place.)

## Decision 1 — extract a shared harness module, keep tests colocated and white-box

Add `crates/flexwm/src/compositor/test_support.rs` (name to taste;
`test_support` matches this project's plain, literal naming elsewhere),
`#[cfg(test)] pub(crate) mod test_support;` declared once in
`compositor/mod.rs` alongside the other module declarations. Move into it,
generic over whatever varies per caller:

- `wait_for<T, C>(queue: &mut EventQueue<C>, client: &mut C, what: &str, ready: impl Fn(&C) -> Option<T>) -> Result<T, String>`
  — already has zero file-specific logic in any of its existing copies;
  purely mechanical to lift and genericize over the client type `C`.
- A `drive<C, R>(...)`-shaped function collapsing the repeated
  `EventLoop::try_new()` + `State::new(...)` + socket pair + spawn client
  thread + dispatch-until-signaled-done-or-deadline sequence into one call.
  This is the highest-value extraction, since it is the most-duplicated
  piece by raw occurrence count (8+4+3+3+2 = 20 inline copies across five
  files). Model its shape on `activation/tests.rs`'s existing `drive()` —
  it is already close to file-agnostic; read it first rather than
  designing from scratch. Parameterize over: the `Config`/`Keybindings`/
  `Appearance` used to build `State`, whether an output is added before the
  client connects, and the client-thread closure/script type.
- Small pixel helpers (`pixel`, `contains`, `assert_pixel` and friends) —
  near-identical across three files, low risk to unify.

Leave `solid_buffer` and each file's own `Step`/`Ack` enum definitions
where they are unless the extraction naturally covers them too: the enum
variants encode what is genuinely file-specific (what actions this
protocol's test script can perform), and forcing them into a shared type
would trade real duplication for a worse abstraction (a giant enum of every
protocol's steps, or a generic parameter nobody wants to name). Don't force
it — extract what is actually identical, leave what only looks similar.

**Then, and only then, re-measure and decide on further splitting.**
`session_lock/tests.rs` (2270 lines, ~40 tests covering clearly distinct
concerns by name — blanking/backdrop behavior, destroy-during-lock races,
input-while-locked) and `layer_shell/tests.rs` (2563 lines, ~39 tests) are
very likely still large enough after harness extraction to warrant
splitting into per-concern submodules, mirroring the pattern this repo
*already* uses elsewhere — `ext_workspace/diff/tests.rs`,
`cursor/shapes/tests.rs`, `ipc/{line,outbound,connection}/tests.rs` are all
existing precedent for "this module's tests split by sub-concern, not left
as one file". `ext_workspace/tests.rs`, `cursor/tests.rs`, and
`activation/tests.rs` (1078-1131 lines, 21-25 tests each) are borderline;
don't split them pre-emptively; measure post-extraction first, since the
harness alone may bring them under a size where a split just adds
navigation overhead for no benefit.

## What this is *not*, and must not become

Not a move away from real-client, wire-level, real-pixel testing toward
mocks or handler-level unit tests. That would be a regression in exactly
the coverage quality this project has repeatedly and explicitly chosen
(see `session_lock/tests.rs`'s own module doc, and the two rounds of
independent review during the `xdg-activation-v1` work that found bugs a
less rigorous test would have missed). The whole point of extracting the
harness is to make the *existing* standard of rigor cheaper to apply
consistently, not to lower the standard. Any implementer picking this up
who finds themselves reaching for a mock instead of threading through the
shared `drive()` helper has misread this entry.

## Decision 2 — adopt `cargo-nextest`, alongside `cargo test`, not instead of it

Confirmed via research (2026): nextest is mature, actively maintained, and
its core property — one process per test — is a genuine, on-topic win
here, not a generic "it's faster" pitch:

- Multiple test files already carry comments acknowledging that
  `smithay::utils::SERIAL_COUNTER` is process-global and "shared with every
  other test running in parallel" within one `cargo test` binary
  (`activation/tests.rs`, `interaction.rs`'s tests). Nextest's per-test
  process gives every test its own fresh counter, removing a whole class
  of cross-test coupling through global state — tests that happened to
  depend on that sharing would fail loudly under nextest, which the
  Rust community treats as "usually a correctness fix in disguise" rather
  than a nextest problem. Worth running once as a check for exactly that
  when this is picked up.
- No compatibility risk found for this project's style of test (real Unix
  sockets, threads, calloop event loops, target-cfg-gated Linux-only code):
  nextest builds test binaries through the ordinary `cargo` pipeline and
  only changes how the resulting binaries are *executed* (one per process
  instead of many in one process), so platform-gated dependencies and
  `#[cfg(target_os = "linux")]` code are unaffected.
- **Real limitation, confirmed**: nextest does not run doctests. `cargo
  test --doc` (or plain `cargo test`) is still needed for those. Moot
  today — `cargo test --workspace`'s own output already shows `Doc-tests
  flexwm_core: 0 tests` and `Doc-tests flexwm_ipc: 0 tests`, i.e. the
  workspace has none right now — but note it here so it isn't rediscovered
  as a surprise the day someone adds one.
- No CI exists in this repo (no `.github/` or equivalent found) to
  reconfigure, so this is a pure local/dev-VM improvement with no migration
  risk.

**Concretely**: add `pkgs.cargo-nextest` to `vm/configuration.nix`'s
`environment.systemPackages` (the dev VM is where the standard verification
set actually runs today per `CLAUDE.md`), and add `cargo nextest run
--workspace` to `CLAUDE.md`'s "Verification and evidence" section as an
addition to the standard set — run *alongside* `cargo test -p flexwm`
(which still covers doctests and remains the documented baseline that
needs no extra tool install), not as its replacement. Update
`scripts/smoke-test.sh`'s header comment or `vm/README.md` if either
documents the exact test invocation sequence, so the addition doesn't go
stale the way the config/keybinding reference once did (see `CLAUDE.md`'s
own note on that recurring failure mode).

## What it would take

1. Write `compositor/test_support.rs`, migrate `wait_for`/`drive`/pixel
   helpers into it from `session_lock/tests.rs` and `layer_shell/tests.rs`
   first (the two with the most duplication and the clearest existing
   `drive`-equivalent shape to generalize from), then `ext_workspace/tests.rs`
   and `cursor/tests.rs`, then reconcile `activation/tests.rs`'s own
   `drive()` to use the shared version instead of its bespoke one.
2. Re-run `wc -l` on all five files; decide the `session_lock`/`layer_shell`
   submodule split only after seeing the post-extraction sizes.
3. If splitting: mirror `ext_workspace/diff/`'s directory shape (a
   `tests/mod.rs` or numbered/named submodule files under a `tests/`
   subdirectory of the module, not a flat rename).
4. Add `cargo-nextest` to the dev VM, run the full suite under it once as a
   verification step in its own right (looking specifically for any test
   that only passed because of cross-test global-state leakage), fix
   anything that surfaces, then document it in `CLAUDE.md` and
   `vm/configuration.nix` per above.
5. Full cycle per `CLAUDE.md` as usual: `cargo test --workspace` (and
   `cargo nextest run --workspace` once added), `clippy -D warnings`,
   `fmt --check`, `scripts/smoke-test.sh` — this entry changes test
   organization and tooling, not compositor behavior, so no new
   README/config/keybinding documentation is owed beyond the `CLAUDE.md`
   verification-set update above.
