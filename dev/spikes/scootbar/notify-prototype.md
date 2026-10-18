# notify-prototype: go/no-go measurement record

Measured 2026-10-18 for ticket `notify-prototype`
([docs](../../../../docs/scootbar/backlog/resolved/notify-prototype-done.md)),
from [scootnotify.md](../../design/scootnotify.md#goals-non-goals-and-the-case-for-it).
A throwaway binary **outside the workspace** that `#[path]`-includes
scootbar's drawing, text and D-Bus sources (the pattern
`crates/scootbar/fuzz/Cargo.toml` already uses). It owns
`org.freedesktop.Notifications` and draws one card per notification on an
overlay layer surface. The sources are deleted after measuring; this file
plus `notify-prototype/` (screenshots) is the record.

**Outcome: GO, on one condition.** Memory and text pass outright. The hang
row fails on unmodified scootbar D-Bus code and passes with the small
`LimitsExceeded` reply the design already prescribes for scootnotify
([shell-apps.md](../design/shell-apps.md#scoot-dbus-library-cratesscoot-dbus),
"a small addition"). `notify-server` must include that reply. Humans keep
the guarded tickets' prose clauses.

Workspace state for every number on this page:
`a2f7feacc` (worktree `spike/notify-prototype`; `crates/` identical to
`origin/main` at `a46ec7bb7`). Included copies verified byte-identical to
the worktree (`color.rs`, `paint.rs`, `text.rs`, `print.rs`,
`control/paths.rs`, `dbus/{conn,proto}.rs`,
`icon/{mod,cache,path,raster,sample,image}.rs`). Prototype source
`/tmp/w9p` (own `[workspace]`, release profile mirroring the workspace:
`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`,
`opt-level = 3`; `forbid(unsafe_code)`; features: `icon-image` on,
everything else off).

Harness (all rows dev-VM-only; the M2 was unavailable, no M2 rows):
aarch64 dev VM (`ssh -p 2222 dev@localhost`, rustc 1.97.1), headless
`scoot` 0.1.0 (`/var/cargo-target/release/scoot`, pixman, `--outputs 1`),
private `dbus-daemon` 1.16.2, fresh `XDG_RUNTIME_DIR` per run,
`CARGO_TARGET_DIR=/dev/shm/w9p-target` (removed after). Clients:
`notify-send` 0.8.8 and `mako` 1.11.0 from nixpkgs (stock, not lean),
plus a 200-line test sender in the prototype crate. Sampling follows the
[M6 method](../../../docs/scootbar/backlog/lightest.md): `/proc/PID`
after 14 s of settling and again 60 s later (`VmRSS`, `Pss`, voluntary
context switches, fds), one run per row. Fonts `~/w8-fonts` (DejaVu plus
Noto Sans SC/Arabic/Hebrew/Devanagari and Noto Emoji).

## Results

Gate: idle RSS at most half of mako's, 0 wakeups; a legible corpus judged
by the maintainer from side-by-side screenshots; every call answered.

| Row | notify-proto | mako 1.11.0 (same harness) |
|---|---|---|
| Idle RSS, no cards | **3,140 kB** (DejaVu only; 21,664 kB with all five fonts read into heap, see below) | 8,432 kB |
| Idle PSS | 1,746 kB | 5,102 kB |
| Idle wakeups / 60 s | **0** (9→9; full-font run 140→140) | **0** (55→55) |
| Threads / fds at idle | 1 / 6 | 1 / 8 |
| One card up, RSS | 3,220 kB (+76 over idle) | 16,056 kB (+7,624) |
| One card up, wakeups / 60 s | 1 (14→15, one unexplained wake) | 15 in ~48 s around one arrival |
| One card up, threads / fds | 1 / 7 | 2 / 8 |
| Stripped binary, aarch64 | 856,768 B | 70,840 B (dynamically linked) |
| ELF `.text` (`size -A`) | 608,328 B | not measured (own `.text` is small; cost is cairo/pango closure) |
| Links | libc, libgcc_s only (`ldd`: no libwayland, no C++ — wayland-client is pure Rust; the family keeps its libc-only property with no wrapper crate) | cairo, pango and the GTK stack (closure per design: 183.4 MiB lean) |
| `Notify` parse+store | 5–200 µs (stderr stamp; compositor time excluded) | not measured |

Idle RSS is 37% of mako's (gate: at most half). The design's other
budgets (≤3.0 MB idle, ≤1.6 MB binary, ≤15 ms startup) all hold on the
DejaVu run; startup-to-owned was not timed precisely.

**Fonts.** The 21.6 MB full-font row is a prototype artifact, not the
stack: the prototype reads each font file into a heap `Vec`, while the
real server memory-maps them exactly the way
[`font.rs`](../../../crates/scootbar/src/font.rs) already does
(`map_if_read_only`), so the bytes stay clean file pages shared with
every other process. Noto Sans SC alone is 17.7 MB of the 18.4 MB of
anonymous font pages. The fair row for the gate is the DejaVu one (same
stack, same harness, fonts equal); the full-font legibility runs still
prove the fallback chain works.

### Corpus

`notify-send` plain, markup, `-i` name, `-i` path, `-h int:value:`,
`image-data` (red 64x64 square, via the test sender), CJK, Cyrillic and
Arabic bodies, two action buttons, critical and low urgencies, a 3-card
stack, and `replaces_id` update — each beside mako in the same harness
(`notify-prototype/01-*.png` vs `m-*.png`). Not run: Firefox,
Thunderbird, an Electron app and a GTK4 portal app (no GUI apps launch
on the headless VM); theme-name icons (no hicolor theme resolution in
the prototype — a recorded limitation, not a gate row).

- Latin, Cyrillic and CJK draw legibly (CJK through the Noto fallback;
  mako shows tofu in this harness — it has no CJK fonts installed, a
  harness artifact, not a mako deficit).
- Markup strips to plain text and stays legible (the prototype
  advertises `body-markup` but does not render bold/italic; stated).
- PNG file icons and `image-data` render (triangle-downscaled through
  the real `icon/image.rs` + `sample.rs`); action buttons and the value
  bar render; `replaces_id` from a new connection swaps in place.
- **Arabic draws wrong** (logical order, isolated forms), exactly as the
  [text-shaping spike](scootbar/text-shaping.md) (NO-GO on size) said it
  would. Hebrew and Devanagari are the same by construction. v1 scoping
  per that verdict; the docs must say so.

### The hang row

A 5 MiB `Notify` (framed by hand — scootbar's own `Writer` cap refuses to
build one, as it should):

- **Unmodified code: the call hangs.** The message is skipped silently
  (`skipping a 5243140-byte message`), no reply is ever queued, and the
  client blocks until its own timeout (measured: sender still waiting at
  10 s). `notify-send` would sit the same way for its 25 s.
- **With the prescribed fix: answered in under a millisecond.** A
  12-line experiment-only patch to the included `conn.rs` copy answers
  `org.freedesktop.DBus.Error.LimitsExceeded` to an over-cap method
  call (parsed with the existing `Message::parse_header`, routed to the
  stamped sender). Measured: `huge error reply:
  org.freedesktop.DBus.Error.LimitsExceeded` on the next pump, daemon
  log shows the skip plus the reply. The design already requires this
  addition for `notify-server` (with a 4 MiB cap); the prototype proves
  the shape, it does not ship it.

Mako answers the same 5 MiB call with a return promptly (no hang), so
the bar the gate sets is reachable — by the patch, not by the skip.

`CloseNotification` on an unknown id gets the spec's empty D-Bus Error
(verified through `dbus-send`); every other method shape gets
`InvalidArgs`/`UnknownMethod`, never silence, never a crash.

## Bugs found in the prototype (all fixed in the throwaway)

Two would have bitten the real server, both cheap:

1. **Frame request after commit starves on a damage-driven compositor.**
   Requesting `wl_surface.frame` after attach+commit waits for a next
   presentation that never comes; cards never expired (7-minute-old
   cards on screen). Requesting the frame *before* the commit fixes it
   (presentation-started clock now starts; expiries fire on time).
2. **D-Bus milliseconds treated as seconds.** `expire_timeout` is
   already milliseconds; the prototype multiplied by 1000 once (cards
   lived 1000x too long). One-line fix.

Plus three tooling-grade findings, out of scope for the verdict: the
layer-shell rule that killed two mornings (a `set_size` 0 on an axis
without both anchors is `InvalidSize`, and it must be set before the
first commit — verified against the pinned Smithay source); the test
sender's missing flush before poll (queued a call then slept waiting
for its reply); and a dbus-daemon behavior note — short hand-framed
bodies under 4 bytes are refused with a hangup while byte-identical
structure with longer bodies routes (reproduced 5/5 vs 3/3; all real
clients unaffected; sender-side curiosity only, recorded here so
nobody re-runs it blind).

## Evidence kept

- `notify-prototype/01-plain.png` … `14-replace.png`: the corpus.
- `notify-prototype/m-plain.png`, `m-markup.png`, `m-value.png`,
  `m-cjk.png`, `m-arabic.png`: mako in the same harness.
- `notify-prototype/proto-onecard.png`, `proto-expiry-t2.png`,
  `proto-expiry-t18.png`: one card up; expiry before/after.
- Raw rows: the tables above are copied from `/proc` samples taken per
  the M6 method (14 s settle, 60 s window, one run per row); the 5 MiB
  transcripts (`NO_REPLY after 10s`, `LimitsExceeded`) are quoted above.
- Prototype source: throwaway crate deleted after measuring per the
  ticket (method, module list and both diffs — the 12-line
  `LimitsExceeded` patch and the two one-spot fixes — are described
  above; the `notify-server` ticket carries the real implementation).

No M2 rows (the M2 was unavailable to this session); every number above
is dev-VM-only.
