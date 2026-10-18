# Keyboard-stack spike: `kbvm` vs `xkbcommon`, measured

Measured 2026-10-17/18 for [scootui-keyboard-spike](../../../docs/scootbar/backlog/scootui-keyboard-spike.md).
Two throwaway binaries doing the ticket's own job — bind `wl_seat` v9+,
read the keymap scoot sends, turn key events into text and keysyms — one per
option, timed and sized on the dev VM and the Mac. The spike crates are
removed after; this file is the record (as with the
[D-Bus spike](scootbar/dbus-client.md)).

**Outcome: `xkbcommon` (the crate over libxkbcommon).** `kbvm` passes the
whole layout set including dead keys, but costs 918 KB of stripped binary
on the dev VM against the design's 300 KB bar, and needs two environment
variables set before it finds anything on NixOS. `xkbcommon` costs 328 KB,
needs no environment, fails loudly instead of decoding to nothing, and is
already in `Cargo.lock` through Smithay, so it adds no new version.

## What was run

- `spike-kbvm`: `kbvm` 0.2.0 with `compose`. Client pattern from kbvm's own
  docs: `Context` + `keymap_from_bytes` (the `wl_keyboard.keymap` fd) or
  `keymap_from_names` (RMLVO, for the layout sweep) -> `to_builder` ->
  `build_lookup_table`; effective mods/group straight from the compositor's
  `modifiers` event into `LookupTable::lookup`; `KeysymProps::char` for
  text; `ComposeTable` for dead keys.
- `spike-xkb`: `xkbcommon` 0.9.0 over libxkbcommon 1.13.2. The pattern
  scoot's own tests use: `Context` + `Keymap::new_from_string` (the fd) or
  `Keymap::new_from_names` (RMLVO) -> `State::new`; the `modifiers` event
  into `update_mask`; `key_get_one_sym` for the keysym, `key_get_utf8` for
  text; `compose::Table` + `compose::State` fed per press.
- `spike-base`: empty binary that sleeps (RSS/size baseline).
- `spike-wl`: `wayland-client` only (the glue every client pays).

Both backends share one Wayland harness, so the mechanics are identical:
seat v9 bind, fd read with a rewind ([below](#a-second-reader-gets-an-empty-keymap)),
a mapped `xdg_toplevel` for the typed-through leg (keys only reach the
focused client). Each bin carries `#![forbid(unsafe_code)]`.

Commands (dev VM; `XKB_CONFIG_ROOT` only for the kbvm RMLVO/compile legs):

```sh
./spike-kbvm vectors            # 27 probes across the ticket's layout set
./spike-xkb vectors             # same probes; outputs diffed after normalising the XK_ prefix
./spike-kbvm compile 30         # per-layout keymap time, mean of 30
./spike-xkb compile 30
./spike-kbvm compose            # dead_acute+e, dead_acute twice, under LANG=C and en_US.UTF-8
./spike-xkb compose
./spike-kbvm wayland 2          # live keymap fd under scoot --headless
./spike-kbvm focused 60         # mapped surface; scootctl type reaches it
```

Measured at spike branch `c96e7e341` (decode/lookup code identical in every
build of the spike's life; only the Wayland plumbing changed). Release
profile mirrors the workspace (`lto = "fat"`, `codegen-units = 1`,
`strip = true`, `opt-level = 3`, `panic = "abort"`). rustc 1.97.1 on the
dev VM, 1.98.1 on the Mac. xkeyboard-config 2.48 both boxes; libxkbcommon
1.13.2 both boxes (NixOS system copy on the VM, nix store on the Mac).
scoot is `/var/cargo-target/release/scoot` (`scoot 0.1.0, ipc protocol 4`,
built 2026-09-28), run `scoot --headless --socket w8-scoot` (pixman,
GPU-free) with `XKB_DEFAULT_*` per layout; typing via
`scootctl type`/`scootctl action focus-window-id`.

## Results

### Correctness: identical on all 27 probes

`diff` of the two `vectors` outputs (after `s/keysym=XK_/keysym=/`) is
empty, on both boxes. Anchors, all correct in both:

| Layout | Probe (evdev, shift, group) | Both decode |
|---|---|---|
| us | A key plain / +Shift; 1 key plain / +Shift; `'`; `=` +Shift | `a A 1 ! ' +` |
| us(intl) | `'` key | `dead_acute`, no text |
| de | Z key; Y key; 2 +Shift; `-` key | `y z " ß` (Z/Y swapped vs us) |
| fr | A; Q; W; 1 plain / +Shift | `q a z & 1` (AZERTY) |
| dvorak | S; D; A; E positions | `o e a .` |
| us,ru `grp:alt_shift_toggle` | A group 0 / group 1 / group 1 +Shift; 1 group 1 | `a ф Ф 1` |

Typed through `scoot msg type` into a focused spike surface, per live
session (scoot restarted with each `XKB_DEFAULT_*` row; listeners run
sequentially — see [below](#a-second-reader-gets-an-empty-keymap)):

| Session | Typed | kbvm decoded | xkb decoded |
|---|---|---|---|
| us | `AbC xyz 123!?` | identical, 19 keys | identical, 19 keys |
| us(intl) | `café` | `c a f (AltGr) é` | identical |
| de | `yYzZß?` | identical, incl. Shift+ß → `?` | identical |
| fr | `aqz&1` | identical, incl. unshifted `&`, shifted `1` | identical |
| dvorak | `oea.` | identical | identical |
| us,ru + toggle | `Aф` | `A` both; `ф` refused by scoot's own planner (`no key for ф in this layout`), same for both | same |

Group-1 decoding itself is covered by the vectors rows above
(`ф`/`Ф` correct in both). Not run: real keys on M2 hardware (none
available to this session); the scripted vectors plus the headless live
runs cover the same decode path.

### Compose

Both compose `dead_acute` + `e` → `é`, and `dead_acute` twice → `´`, under
`LANG=C` and `LANG=en_US.UTF-8`. Compose does not depend on the locale in
either stack; it depends on finding the Compose file:

| | libxkbcommon | kbvm |
|---|---|---|
| Zero env, NixOS dev VM | works (store paths compiled in) | table missing |
| Zero env, Mac | works (nix libxkbcommon's compiled-in `XLOCALEDIR`) | table missing |
| `XLOCALEDIR` (nix libX11 locale dir) or `XCOMPOSEFILE` set | works | works, same output |

A greeter that relies on kbvm compose must ship one of those variables
(the look/session would have to set it); libxkbcommon needs nothing.

### Keymap compile time (mean of 30, ms)

| Layout | kbvm, VM | xkbcommon, VM | kbvm, Mac | xkbcommon, Mac |
|---|---|---|---|---|
| us | 5.7 | 3.1 | 1.8 | 1.1 |
| us(intl) | 5.7 | 3.5 | 1.7 | 1.1 |
| de | 5.7 | 3.6 | 1.8 | 1.1 |
| fr | 4.9 | 3.3 | 1.7 | 1.1 |
| dvorak | 4.6 | 3.3 | 1.6 | 1.1 |
| us,ru + toggle | 5.8 | 3.5 | 1.8 | 1.2 |

Live keymap fds from scoot (35–38 KB of keymap text): xkbcommon 1.0–1.9 ms,
kbvm 2.4–4.2 ms on the VM. One-shot per seat either way; not decisive.

### Size and RSS (stripped release; VM aarch64, Mac arm64)

| | VM bytes | VM `.text` | Mac bytes | Mac `__text` |
|---|---|---|---|---|
| base (empty) | 332,456 | 215,656 | 302,640 | 206,464 |
| wayland-client only | 332,456 (+0) | 220,392 (+4,736) | 319,616 (+16,976) | 212,604 (+6,140) |
| kbvm | 1,250,016 (**+917,560**) | 845,736 (**+630,080**) | 1,115,248 (+812,608) | 832,320 (+625,856) |
| xkbcommon | 660,136 (+327,680) | 388,200 (+172,544) | 587,072 (+284,432) | 377,376 (+170,912) |

Plus the shared library: `libxkbcommon.so.0`, 426,832 bytes on the VM
(1.7 MB closure), 385,536 bytes `libxkbcommon.0.dylib` on the Mac — on a
scoot desktop that library is already resident inside scoot itself, so its
marginal closure there is zero. `spike-kbvm` links libc and libgcc_s only
(`ldd`: nothing else), keeping the shell family's libc-only property;
`spike-xkb` adds exactly `libxkbcommon.so.0`.

RSS holding one compiled us keymap + lookup/state, then idle (stable
across two samples 8–12 s apart):

| | VM VmRSS | Mac RSS |
|---|---|---|
| base | 1,820 kB | 1,472 kB |
| wayland-client only | 1,820 (+0) | 1,488 (+16) |
| kbvm | 4,772 (**+2,952**) | 6,848 (+5,376) |
| xkbcommon | 3,068 (+1,248) | 2,800 (+1,328) |

One thread and 3 fds in every binary on the VM. Resolve graph: 48 crates
total; kbvm alone pulls 22 (`arrayvec bstr cfg-if debug-fn equivalent
foldhash hashbrown indexmap isnt kbvm kbvm-proc linearize linearize-derive
log secure-execution serde_core serde_derive syn thiserror thiserror-impl
unicode-width version_check` beyond what the other backends share), all
MIT/Apache-2.0/Zlib/Unlicense/Unicode-3.0 — allowed by `deny.toml`.
`xkbcommon` the crate pulls only `libc memmap2 xkeysym`.

The design's bar was kbvm if it passes the layout set and costs under
300 KB. It passes everything and costs 918 KB stripped on the VM
(+630 KB `.text`), over three times the bar. Cost alone decides against
it before the environment variables do.

### `unsafe`, and the password-buffer question

Neither backend needs `unsafe` in the consumer: both spike bins carry
`#![forbid(unsafe_code)]` and exercise the full client pattern including
compose. All of `xkbcommon`'s `unsafe` stays inside the dependency.

Keysym to UTF-8, the row the ticket asked about:

| | Call | Allocates? | Safe in consumer? |
|---|---|---|---|
| kbvm lookup | `KeysymProps::char() -> Option<char>` | no (`char` by value; caller pushes into its own buffer) | yes |
| kbvm compose | `FeedResult::Composed { string: Option<&str>, .. }` | no (borrowed from the table) | yes |
| xkbcommon lookup | `key_get_utf8() -> String` | **yes** (heap `String` per press) | yes |
| xkbcommon lookup | `key_get_utf32() -> u32` | no (`char::from_u32`, push into caller buffer) | yes |
| xkbcommon compose | `compose::State::utf8() -> Option<String>` | **yes** (stack buffer inside, copied out to `String`) | yes |

So the widget's caller-supplied-buffer call does not exist safely in
`xkbcommon` 0.9.0: single-codepoint keys go through `key_get_utf32` with
no allocation, but composed text comes back as a `String`. If the
greeter's threat model needs composed-char passwords with no heap copies,
that is one small wrapper around C `xkb_state_key_get_utf8(state, key,
buf, size)` via the crate's `State::get_raw_ptr` — the only `unsafe` the
stack needs, and it lives in that wrapper, not in `scootui`. kbvm needs no
wrapper at all here, but that does not outweigh the size verdict.

### `memfd_secret` (scootgreet's locked page)

Dev VM kernel 6.18.50: available. Raw `memfd_create("spike",
MFD_SECRET)` returns a fd; `CONFIG_SECRETMEM=y`. (A first probe OR-ing
`MFD_EXEC` failed with `EINVAL`; without it, OK.) Not run: the M2 Asahi
kernel (no access from this session); macOS has no such API at all.

## Findings for the implementer

1. **A second reader gets an empty keymap** (filed as a core ticket with
   this record as evidence). Smithay's `KeymapFile` wraps one sealed
   memfd (`SealedFile::with_data` seeks to 0 once, at creation) and every
   client's `wl_keyboard.keymap` fd is that same description via
   `SCM_RIGHTS`, so the file offset is shared: the first read()-based
   client to read moves it to the end and every later client reads zero
   bytes. Proven both directions on a fresh server — kbvm first reads
   35,572 bytes and xkbcommon second reads 0, then after a restart
   xkbcommon first reads 35,572 and kbvm second reads 0. Real toolkits
   mmap the fd (offset-independent) and never notice; a read()-based
   launcher client must seek to 0 first (or mmap). The spike does the
   seek; without it the kbvm leg of this spike read empty twice.
2. **kbvm fails silently.** With `XKB_CONFIG_ROOT` unset on NixOS,
   `keymap_from_names` returns an *empty* keymap with exit 0 — every
   lookup is `NoSymbol`, no error anywhere. libxkbcommon prints
   `couldn't find file "rules/evdev"` and returns `None`. A kbvm consumer
   must treat "zero keys in the table" as a failed compile.
3. **The event loop must read.** `EventQueue::dispatch_pending` dispatches
   only what was already read; a client that loops on it never hears
   anything past its roundtrips (this spike's typed-through leg was deaf
   until it polled the connection fd and read when ready). Related:
   `get_keyboard` queued during a roundtrip's dispatch needs a later
   roundtrip (or flush) to reach the server at all.
4. **No surface, no keys.** Key and modifier events go only to the
   keyboard-focused client. A surface-less test client sees the keymap
   and repeat info but never a press — the spike maps a plain
   `xdg_toplevel` for the typed-through leg, the same focus any launcher
   holds while open.

## What landed

Nothing: spike code only, deleted in the same branch. The decision is
recorded here and in the [shell-apps design](../design/shell-apps.md#keyboard-and-text-entry):
use `xkbcommon`, no wrapper crate (consumer `unsafe`-free as proven);
`key_get_utf32` into the caller's buffer on secret fields; the
caller-buffer compose call is a follow-up only if the greeter's threat
model demands it.
