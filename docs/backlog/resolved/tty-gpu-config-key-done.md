---
title: "A config-file key for `--tty`'s DRM device, so `--gpu` doesn't have to be retyped on every launch. — RESOLVED."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A config-file key for `--tty`'s DRM device — RESOLVED.

## The entry as filed

`docs/backlog/tty/tty-gpu-config-key.md` (MEDIUM, blocked on user's Asahi
hardware confirming `--gpu`):

> A config-file key for `--tty`'s DRM device, so `--gpu` doesn't have to be
> retyped on every launch. Suggested by the review of PR #27, 2026-09-13;
> deliberately *not* in that PR. On hardware where the automatic search picks
> wrong (the Apple Silicon case item 17 exists for), `--gpu PATH` is the fix,
> and a fix you must remember to type is not the daily-drive form of one — a
> display manager, a `.desktop` entry or a shell alias each have to carry it
> separately. Shape: a new `[tty]` section (the config file has
> `[layout]`/`[appearance]`/`[binds]` today, none of which fits a backend
> device path) with a `gpu = "/dev/dri/card1"` key, and `--gpu` overriding it
> the way an explicit flag should. Small: `config.rs` already has the
> parse-and-warn-per-key machinery, and `tty::init` already takes the path as
> an `Option<&Path>` argument, so nothing below `compositor::run` changes.
> Gate on the user confirming `--gpu` actually fixes their Asahi machine
> first — if it doesn't, the right shape of the persistent setting may not
> be a device path at all, and this would be a config key shipped for a
> workaround that didn't work.

## Resolution (2026-09-18)

The key is implemented, tested, and verified live on the dev VM. It was
deliberately **not** held for the Asahi hardware confirmation the ticket
gated on: the gate conflated two separable things — the config key (parse,
validate, plumb, select; fully buildable and testable without split-GPU
hardware) and the hardware confirmation (does `--gpu` fix the Asahi
machine, which needs the user's own hardware, not the dev VM). The first
is done here; the second stays open as a stated residual below, with its
revisit condition. If the Asahi probe ever shows a device path is the
wrong shape of persistent setting, the key is small enough to reshape —
but there is no evidence for that today, and holding a daily-drive
usability key for it kept every multi-GPU user retyping the flag.

### What changed

- `config.rs`: new `[tty]` section (`TtyConfig { gpu: Option<PathBuf> }`,
  `deny_unknown_fields` like every other table) and `LoadedConfig::gpu`.
  An empty `gpu = ""` parses to `Some("")`, not `None` — the loader must
  not silently swallow what the user wrote (that would be fail-open).
  The module doc records the one deliberate exception to its
  never-block-startup rule: a set-but-unusable `[tty] gpu` behaves like
  `--gpu` (exactly that device, startup error when it cannot be driven),
  because falling back to the automatic pick would silently drive a device
  the config explicitly ruled out.
- `tty/gpu.rs`: new `ExplicitGpu::{Flag, Config}` (the path plus where the
  name came from) and `resolve(flag, config)` — `--gpu` wins over
  `[tty] gpu`, neither means the automatic search, and an
  explicitly-set-but-empty path in *either* source is a hard startup error
  (`EmptyGpuPath`, naming the surface that set it) rather than something
  the session is asked to open. `candidates` and `unusable_device_error`
  take `Option<ExplicitGpu>` instead of `Option<&Path>`; the explicit
  failure wording names the actual source (`` --gpu PATH `` vs
  `` [tty] gpu = "PATH" ``), so a config-file user is never told to
  re-check a flag they never passed.
- `tty/mod.rs`: `init` takes `Option<ExplicitGpu>` (re-exported
  crate-local); nothing below `compositor::run` changed otherwise, as the
  ticket predicted.
- `compositor/mod.rs`: `run` resolves once (`tty::resolve`) so the `--tty`
  init and the not-`--tty` warnings read the same answer; a config-file
  `gpu` on `--headless`/`--nested` warns exactly like `--gpu` does.
- `cli.rs`: the `--gpu` doc records that the flag wins over `[tty] gpu`.
- `README.md`: the `--tty` device section documents the key and the
  precedence; the Configuration section counts five tables, carries the
  `[tty] gpu` exception in its failure semantics, gains a `### [tty]`
  reference row, and the example `config.toml` shows the commented key.

Fallback rules, stated once: no explicit source → primary first, then
every other seat device until one works (unchanged); any explicit source
→ exactly that device, no fallback, failure is a startup error (the
`--gpu` shape, now shared). `--gpu` beats `[tty] gpu` silently when both
are set — the universal flag-wins convention, documented, not warned.

One adjacent behavior change, kept deliberately: an empty `--gpu ""`
used to reach the session and be refused there; `resolve` now refuses it
up front with a clearer message. It was never a working configuration —
only the refusal text changed, not the fail-closed shape.

### Tests

`config.rs`: `[tty] gpu` round-trips into `LoadedConfig::gpu`; a missing
table/key means no explicit device; a typo'd key and a wrong-typed value
fall back whole-file (the module's rule, pinned for the new table);
an empty value is preserved for `resolve` to refuse.

`gpu.rs`: the config source replaces the search like the flag does;
`resolve` prefers the flag, uses the config alone, returns `None` for
neither, and refuses empty from either source (with the refusal naming
the right surface); the config-sourced startup error names `[tty] gpu`
and never mentions `--gpu`.

Fail-first: the tests were written against `ExplicitGpu`, `resolve` and
`TtyConfig` before any of them existed — the first dev-VM run failed to
compile with 26 errors naming exactly those three items. Implemented
green after.

### Live evidence (dev VM, QEMU/virtio-gpu, single GPU — selection is trivially the default here)

Against the uncommitted working tree of `feat/tty-gpu-config-key`
(`cargo build -p flexwm` on the 9p mount; seat verified free via `pgrep`
before each run):

- Default `--tty`: `drm: driving this device path=/dev/dri/card0
  connector=Virtual-1 width=1280 height=720`, screenshot 1280x720
  (19665 bytes, md5 `09a91ae837c2118f5b0ed6dac8bab4b6`), `msg action quit`
  clean, zero teardown/permission/panic lines (PR #111 stays quiet).
- `[tty] gpu = "/dev/dri/card0"`: drives card0, screenshot **byte-identical**
  (`09a91ae8…` both) — the key does not disturb the default path.
- `[tty] gpu = "/dev/dri/card9"`: exit 1, no socket, no fallback —
  `flexwm: the device named by \`[tty] gpu = "/dev/dri/card9"\` could not
  be opened through the session (…No such file or directory…)`.
- `[tty] gpu = ""`: exit 1, no socket —
  `` flexwm: `[tty] gpu` is set but empty; name a DRM device (…) or remove
  the key``.
- `--gpu /dev/dri/card0` over a config naming card9: drives card0 —
  the flag wins live, not just in unit tests.
- `--headless` with the key set: warns `[tty] gpu names the DRM device
  for --tty; ignoring it on this backend`, starts and quits clean.

Full set on the same tree: `cargo test -p flexwm` (922 passed, 0 failed,
+ 3 doctests), `cargo nextest run --workspace` (1027 passed, 1 skipped),
`cargo clippy -p flexwm --all-targets -- -D warnings` clean,
`cargo fmt --check -p flexwm` clean, `scripts/smoke-test.sh`
(`SMOKE_PREFIX=/tmp/smoke-gpu`, headless) exit 0 with 17 `ok` lines.

### Benchmark

No hot-path benchmark: `resolve` runs once at startup and `describe`
allocates one `String` per failed explicit device on the startup-failure
path only. Nothing on any per-frame, per-event, or IPC-dispatch path
changed or grew an allocation.

### The Asahi residual — CLOSED (2026-09-18)

The hardware confirmation this entry deferred is **done, and the answer is
that the automatic search already works on Apple Silicon.** `--gpu` is a
convenience on this machine, not a requirement, and the `[tty] gpu` key
needs no reshaping — the "device path is the wrong shape" risk did not
materialize.

Measured on the reporter's Apple M2 (`apple,t8112`, j413), NixOS aarch64,
per [`Asahi.md`](../../../Asahi.md) Test 2. The split is exactly the one the
key was designed for:

| node | driver | role |
| --- | --- | --- |
| `/dev/dri/card1` | `asahi` | render / AGX, **no KMS** |
| `/dev/dri/card2` | `apple-drm` | display controller, owns `eDP-1` |
| `/dev/dri/renderD128` | `asahi` | render node |

**The evidence is stronger than the probe this entry asked for: it is the
user's live, daily-driven session, not a test run.** flexwm runs as their
desktop via greetd (`flexwm --tty -- noctalia`, pid 1700, seat0/vc1) with
**no `--gpu` flag and no `[tty] gpu` key set** — and its only open DRM fd is
`/dev/dri/card2`. So the fallback probed the `asahi` render node, rejected
it, and landed on the `apple-drm` display controller unattended.
`flexwm msg outputs` against that live session reports `eDP-1`, 1280×800
logical at scale 2.0.

Independent corroboration from the same boot: the greeter (niri) hit the
exact failure this fallback routes around —

```
niri::backend::tty: error adding primary node device, display-only devices
may not work: DRM access error: Error loading resource handles on device
`Some("/dev/dri/card1")` (Operation not supported (os error 95))
```

— the `os error 95` signature this entry predicted, on the very device
flexwm rejected, from a compositor whose primary-node heuristic lacks the
fallback.

**Re-confirmed on `f688ac9` itself.** The first measurement was taken against
an older build that happened to be what the session was running. The machine
was then rebuilt onto `f688ac9` and logged back in, and the new session
(different pid, same story) again opens exactly one DRM fd — `/dev/dri/card2`
— with no `--gpu` and no `[tty] gpu`, on `eDP-1`. Worth having, because that
delta included PR #122, which touches `tty/mod.rs`: the additions are tablet
event arms and two `has_capability(TabletTool)` guards, and this confirms
they left device selection alone.

**Not captured, and why:** the literal `drm: driving this device` line. The
session's stderr goes to `/dev/tty1` uncaptured, and retrieving it would mean
restarting the user's desktop — which was also hosting the session doing the
measuring, and which only one process can hold the `--tty` seat for. The
open-fd evidence answers the same question without that cost. Anyone wanting
the log line can add a redirect to the greetd session command.

**What stays untested, and why it no longer blocks:** the `[tty] gpu` key and
`--gpu` flag themselves on *this* hardware. They did not need to run, because
the mispick they exist to work around does not occur here — which is the
finding. Their behavior remains unit-test proven plus dev-VM verified above.
If some future Apple Silicon topology *does* mispick, this is the machine to
retest on.

### Not verified live (stated plainly)

- A lock, capture, or hotplug held across a config-named device choice;
  the legacy (non-atomic) probe of a named device (dev VM is atomic).
- Two agents racing the `--tty` seat while one of them uses the key —
  the seat-takes-one-client refusal is unchanged and source-agnostic.

No keybinding, CLI flag, or IPC surface comes with any of this: a `[tty]`
table the file never names behaves exactly as before (no explicit
device), and `--gpu`'s wire-visible behavior is unchanged apart from the
clearer empty-path refusal above.
