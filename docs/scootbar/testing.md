# Testing scootbar

How scootbar is tested and how to run it. The harnesses, fuzz targets,
feature matrix and benchmark script grow with
[testing-and-ci](backlog/testing-and-ci.md); this page says what exists.

## Running it

From the repository root, in the dev shell, with `scoot` built beside
scootbar (`cargo build -p scoot -p scootbar`: the end-to-end tests start
it) and sway from the pinned nixpkgs:

```sh
export SCOOTBAR_TEST_SWAY=$(nix build --inputs-from . nixpkgs#sway --no-link --print-out-paths)/bin/sway
SCOOTBAR_REQUIRE_SCOOT=1 SCOOTBAR_REQUIRE_SWAY=1 \
  devenv shell -- cargo nextest run -p scootbar
devenv shell -- cargo test -p scootbar
devenv shell -- cargo clippy -p scootbar --all-targets -- -D warnings
devenv shell -- cargo fmt --check -p scootbar
```

| Variable | Effect |
|---|---|
| `SCOOTBAR_TEST_SCOOT` | the `scoot` binary to run; default, the one beside the test's `scootbar` |
| `SCOOTBAR_REQUIRE_SCOOT` | no `scoot` is a failure, not a skip, and so is no `foot` on `PATH` for the window-placement check (CI sets it) |
| `SCOOTBAR_TEST_SWAY` | the `sway` binary to run; default, `sway` on `PATH` |
| `SCOOTBAR_REQUIRE_SWAY` | no `sway` is a failure, not a skip (CI sets it) |
| `SCOOTBAR_DEBUG_NO_VIEWPORTER` | read by a **debug** `scootbar` only (compiled out of release builds): leaves `wp_viewporter` unbound, as on a compositor without it, so the integer-scale fallback is tested on compositors that have one |

## What covers what

- **Unit tests**, in a `tests.rs` beside each module: flags and their
  bounds (`cli`, `bar`), colors, scale arithmetic at every step from 0.5 to
  4 against the compositor's rounding (`density`), every ordering of an
  output's events through the pure model, closes and retries included
  (`outputs`), the double buffer's choice of slot (`daemon/canvas`), and
  the fill (`paint`).
- **On a headless scoot** (`tests/bar.rs`): a bar on each of two outputs
  reserving its height (scoot's `outputs` usable area) and in its color on
  a screenshot; margins, with the zone including the anchored edge's
  margin, the surface exactly the bar, and a real window (`foot`) placed
  beside it; a bottom bar; device-exact buffers at 1.5, 1.25 and 2 with
  the scale changed live by a config reload (checked on the protocol
  trace, since a solid color scaled down looks the same), and the
  integer-scale fallback without a viewporter; zero wakeups
  while idle; exit status 1 when the compositor is killed.
- **On a headless sway** (`tests/hotplug.rs`): outputs plugged and
  unplugged at runtime, down to none and back; starting with zero outputs;
  a storm of back-to-back plugs and unplugs with the fd count and the
  mapped buffers checked for leaks; the margin rule again.

## Not covered

- **Real hardware**: every run above is headless and pixman.
- **A compositor that closes a bar surface**: the retry-once-then-give-up
  path is unit-tested on the model only; neither compositor here closes
  a bar that is still on a live output.
