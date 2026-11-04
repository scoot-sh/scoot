---
title: "Robustness and resource limits: hostile input, failing children, a dying compositor"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "ongoing"
---

# Robustness and resource limits

Filed 2026-09-29. Serves **daily-drive**. The release profile is
`panic = "abort"`, so a panic kills the bar and its content. `CLAUDE.md`'s
bar applies: a plausible crash or hang is treated like data loss. Modeled on
what scootbg and scoot bound (`docs/scootbg/README.md`, `docs/ipc.md#resource-bounds`).

## Bounds to build in from the first PR that adds the surface

- **Control socket** (*built*: 16 connections with the oldest
  non-subscriber evicted to admit one more (#339); 4 subscribers with the
  5th refused with a reason (#367); 64 KiB request lines refused with a
  reason (#339); the EMFILE spare fd plus a 1 s listener rest instead of
  a spin (#339); the write-stall deadline landed in #493, 30 s
  for a request peer that stops reading, refreshed on any delivery):
  connection cap with a refusal that says why; line and message size caps;
  a write-stall deadline that drops a peer that stopped reading; `EMFILE`
  on accept sheds instead of spinning the loop.
- **`exec` modules** (*built*, with
  [exec-push-button-modules](resolved/exec-push-button-modules-done.md): at most
  8 placed, lines past 4096 bytes dropped whole, one 4 KiB read per 16 ms frame
  while output keeps coming, restarts 1 s to 60 s, a pidfd to reap, the process
  group killed with the module, a fuzz target for the payload, and a flooding
  child against a running bar with the RSS and fd count checked in
  `tests/exec.rs`): a cap on the number of children and on line length (drop
  the excess with a warning); update rate coalesced to the next frame; restart
  with exponential backoff; children reaped (no zombies), never inheriting the
  bar's fds (close-on-exec everywhere, verified as scoot's fd audit did); a
  child that never prints is fine, one that floods cannot grow memory.
- **Config and JSON** (*built*: the file is capped at 64 KiB, read with
  `.take(MAX_FILE + 1)` from a regular file opened non-blocking (#339,
  #513); update payloads are capped at 4096 bytes and 8 levels deep
  (#366); every parse failure is a named error, never a panic, and hot
  reload keeps the running config on failure (#339).
- **Text**: bounded glyph cache and bounded string lengths per module.
  *Built with the clock
  ([module-api-and-clock](resolved/module-api-and-clock-done.md)):* the
  glyph cache holds at most 512 glyphs and 4 MiB of coverage and is dropped
  and refilled past either; a glyph whose bounds would need a rasterizer
  buffer past 4 megapixels (a hostile font) is not drawn; a module's view
  text and tooltip are cut at 256 bytes; control characters are never
  drawn. **The font mapping is decided there**: a font is mapped only when
  it is owned by root, writable by no one and on a read-only mount (a
  read-only mount alone is not enough, as review showed), and read into
  the heap everywhere else, at most 64 MiB, so a `cp` over the font under a
  running bar cannot `SIGBUS` it.
  What is left here is a title stream fuzzed through the cache
  ([icons-and-fonts](resolved/icons-and-fonts-done.md) owns that test).
- **Time zone file**: the TZif reader caps the file (64 KiB in the spike),
  checks every count and index, and falls back to the last transition or to
  UTC rather than failing ([M0 §3](resolved/dependencies-done.md#3-time-zone)).
  *Built with the clock:* the cap is enforced while reading (at most 64 KiB
  + 1 bytes, from a regular file only, opened without blocking, so
  `TZ=:/dev/zero` or a FIFO cannot hang or balloon it), the instant is
  clamped before any arithmetic, and a `cargo fuzz` target covers the
  reader and the POSIX rule parser.
- **Module count and layout** (*built*: at most 32 modules placed (#324);
  a layout wider than the output clips deliberately, not by overflow:
  `layout.rs` uses saturating arithmetic on every client-controlled size
  (#324)).

## Where other bars actually break

From the research (2026-09-29; the sources are GitHub issues, and the report
notes it could not reach Reddit or Hacker News), the failures that recur are
**not** one-frame costs but lifecycle ones. Each gets a named test:

- **Suspend/resume and DPMS wake**: infinite loops and 100% CPU after resume,
  segfaults on wake (Waybar #4393, #1019). Test: suspend cycle with a dead fd in
  the poll set; assert idle CPU returns to zero.
- **Output hotplug**: crashes on HDMI disconnect, modules that stop updating
  after hotplug (Waybar #2808, #4823). Test: repeated add/remove with the module
  set running; assert every module still updates.
- **Memory growth over days**: leaks with only two modules enabled (Waybar #5186),
  runaway growth in a privacy module (#3981). Test: a multi-day soak in the
  [ratchet](lightest.md), and no per-event allocation on the hot path.
- **Busy loops from polled scripts** (Waybar #5303, #4987): see the streaming
  rule in [exec-push-button-modules](resolved/exec-push-button-modules-done.md).
- **Tray failures** (the largest single group): see [tray](resolved/tray-done.md).
- **Zero outputs** and a compositor restart mid-session.

## Failure of the world around it

- **Compositor gone**: the Wayland connection breaks; exit promptly and cleanly.
- **Registry changes** (*built*: the `ext-workspace` and toplevel managers'
  `finished` drops staged state and destroys the handles, with tests
  (#334, #374); an output or global removed mid-frame is handled by the
  output set; a layer surface `closed` by the compositor is rebuilt once
  the output is still there, then given up on for that output's life
  (#323); a `wl_output.name` past 128 bytes is ignored (#539).
- **Allocation failure** (*built*, audited 2026-11-03, see
  [Allocation-failure audit](#allocation-failure-audit-2026-11-03)): know where
  the bar can abort on refused memory (the scaler in scootbg is the precedent,
  [`scaler-oom-abort`](../../scootbg/backlog/resolved/scaler-oom-abort-done.md)); avoid large
  allocations sized by external input. Every block sized by a file, the bus or
  the compositor is now fallible or bounded, each pinned by a test that aborts
  without it; what is left is the libraries' own working memory, listed in the
  audit.
- **Restart policy** (*built*: the NixOS and home-manager units set
  `Restart=on-failure` with `RestartSec=2`, bound to
  `scoot-session.target` (#454); scoot itself does not supervise clients).
  The docs half of
  [nix-modules-and-stylix](resolved/nix-modules-and-stylix-done.md) is
  built: "When the bar exits" in the scootbar index page says what the
  unit does and what happens without it, and troubleshooting has the
  symptom. The unit behavior is pinned by `scripts/scootbar-unit-test.sh`
  (S5 and the retry cases) and the exit 1 on compositor loss by
  `tests/compositor_gone.rs`.

## Verification

Fuzz (see [testing-and-ci](resolved/testing-and-ci-done.md)), an fd storm and a flooding
`exec` child against a running bar with the RSS and fd count measured before
and after, and a kill of the compositor mid-frame.

## Done when

Each bound above has a test that fails without it, and none of the storms moves
RSS or fd count beyond a stated margin.

## Audit 2026-10-08

The ticket is claimed live by `rob2` (claimed 2026-10-07, still within TTL
when checked; the claim was left alone, not forced). The external-input
vectors the brief named were audited against the code instead of re-bounded,
and all are already capped, each with a test that fails without it:

- **D-Bus**: messages past 1 MiB are skipped whole (`MAX_MESSAGE`,
  `frame_header`), the spec's own 128 MiB ends the connection (`MAX_WIRE`);
  nesting past 32 refused (`MAX_DEPTH`); names/paths/signatures at the
  spec's 255/1024/255 (`MAX_NAME`, `MAX_PATH`, `MAX_SIGNATURE`); strings are
  borrowed and cut where they are used (tray titles at 128 bytes, MPRIS
  fields at 120, view text at 256). All in #388 and its follow-ups.
- **Tray pixmaps and menus**: 64 pixmaps of at most 256 px a side
  (`MAX_PIXMAPS`, `MAX_PIXMAP_SIDE`), 64 menu items at 8 levels
  (`MAX_MENU_ITEMS`, `MAX_MENU_DEPTH`), 128 properties (`MAX_PROPERTIES`),
  32 tray items (`MAX_ITEMS`), all refused whole with the last state kept.
- **Config sizes**: 64 KiB file cap above; icon files 8 MiB with 1 Mpx and
  16 MiB decode budgets; fonts 64 MiB.
- **Protocol vectors**: 32 configured outputs (policy; live `wl_output` globals follow the compositor and are not counted), 8 ext-workspace groups with 32
  workspaces (#334), 64 toplevels with 64-byte app ids (#374), 128-byte
  output names (#539).

Two walks deliberately stop at the message cap plus a consumer cap rather
than a walk cap, and both say so in their module docs with tests: MPRIS
artists (walks the list, keeps 16) and BlueZ managed objects (bounded by
the 1 MiB message, held to 8 adapters and 64 devices). No new bound was
invented on top of those decisions.

Genuinely open: the suspend/resume DPMS test, the multi-day soak and the
allocation-failure audit. This entry
stays `open` (`ongoing`): it is the checklist, not a task to resolve.

## Lifecycle tests 2026-11-03

Three of the "genuinely open" items turned out to be built and pinned
already, or to need only a small addition; no bug was found.

- **Hotplug storm**: already covered on a headless sway, the one compositor
  here that adds and removes outputs at runtime (`tests/hotplug.rs`:
  `a_hotplug_storm_leaves_one_bar_per_output_and_no_leak` and
  `modules_still_update_after_a_hotplug_storm`, which checks the clock
  still ticks on the survivors and the newcomer, with the fd count equal
  and one buffer set per output after ten plug/unplug pairs). Run 2026-11-03
  on the dev VM, all 10 tests in `hotplug.rs` pass.
- **Zero outputs**: `zero_outputs_at_start_waits_without_waking` and
  `a_bar_comes_and_goes_with_its_output` (down to none and back) cover the
  wake count and recovery. New, `tests/compositor_gone.rs`:
  `a_bar_with_no_outputs_exits_when_the_compositor_goes`.
- **Compositor gone**: `bar.rs` had the plain case. New, in
  `tests/compositor_gone.rs`: the compositor `SIGKILL`ed with ten outputs
  just created and then all unplugged and an `exec` child running
  (`the_compositor_going_mid_storm_ends_the_bar_and_its_command`). The bar
  exits 1 (so `Restart=on-failure` restarts it) inside 5 s, says "lost the
  connection to the compositor", and the child is gone. Both fail without
  the behavior: `return Ok(())` for the exit gives status 0 and fails the
  status assertion; `continue` in its place (a spin on the hang-up) fails
  with "scootbar did not exit within 20s".
- **Compositor restart**: a restart is the exit above plus a fresh start,
  which every test already does against a new compositor; there is no
  reconnect logic to test, by design. What is not covered: scoot's own
  `--headless` has no runtime output add/remove, so these run on sway
  (skipped without it; `SCOOTBAR_REQUIRE_SWAY` makes that a failure).

## Allocation-failure audit 2026-11-03

Question: which allocations in `scootbar` and `scootui` are sized by something
outside the process (a file, a D-Bus message, a config value, the compositor's
scale or size), and can end the bar when the allocator refuses (an address-space
limit such as `LimitAS=`/`ulimit -v`, or strict overcommit), or overflow? The
scootbg precedent is the scaler, which is the library's own infallible `Vec`;
here the infallible allocations were mostly ours.

**One real bug, reachable only through a bogus compositor.** The scale was
never bounded. `Preferred::scale` passed any positive `wl_output.scale`,
`preferred_buffer_scale` (an `int`) or `preferred_scale` (a `uint` in 120ths) to
`Scale`, and every device length is `logical x scale`. The popup path builds its
corner and shadow tables from `render::device(popup_radius, scale)` cut back
only to half the popup's size, and the popup's size is the *logical* clamp
(`MAX_SIDE`, 8192) times the same scale, before any buffer exists to refuse it:
`Round::new` -> `Corners::new(radius)` is `radius x radius` bytes in an
infallible `Vec`, and `Shadow::build_corner` the same plus the blur. With
`wl_output.scale = 100000` and any `popup-radius` (8, say; the default of 0
follows `bar.radius`, which is 0 too, and builds no table), that is a
radius of 800,000 and a 640 GB request: `memory allocation of 640000000000
bytes failed`, `SIGABRT`, the moment a popup opens. The bar's own tables are
guarded by the fallible `ShmBuffer` created first in `Canvas::draw`, but the
restyle path (`daemon/surfaces.rs`, `scene.update`) has no buffer in front of
it, so it is exposed to a hostile configure size as well. Real compositors do
not send this (scoot caps an output at 4.0, `output_scale::MAX_SCALE`; sway's
and wlroots' limits were not checked and are not needed), so the exposure is a
broken or hostile compositor, which `wl_output.name` (#539) is already treated
as. Fixed at the one choke point: `Preferred::scale` cuts the result to
`density::MAX_SCALE` (16, 1920 in 120ths), pinned by
`density::tests::a_bogus_compositor_scale_is_cut_back_before_it_sizes_anything`.
The worst table that leaves is a 512 px radius at 16x, 8192 px a side, 64 MB,
which is a configured radius of the maximum at a scale nobody has.

**Made fallible** (each has a test in `crates/scootui/src/refused_alloc.rs` or
`crates/scootbar/src/modules/tray/theme/refused.rs` that runs in a child
process whose allocator refuses blocks past a size, `scootui::refusal`, and
that ends in `memory allocation of N bytes failed` / `SIGABRT` with the fix
reverted; the sizes below are the aborts seen):

| Site | Sized by | Max | Now |
|---|---|---|---|
| `font::read_capped` | the font file | 64 MiB (41,943,041 B in the test) | `FileError::NoMemory` |
| `icon::image::load` read, `decode` output and pixels | the PNG file | 8 + 16 + 4 MiB (4,194,304 B) | `Error::NoMemory`, `zeroed_bytes` |
| `tray::theme::read_from_file`, `decode`, downscale | an item's `IconThemePath` PNG | same (1,048,576 B, 4,194,304 B) | icon left out |
| `icon::tray::TrayIcon::take` | a bus pixmap | 1 MiB (1,048,576 B) | no icon |
| `icon::sample::scale_into` row buffer | an icon's height | 8 MiB (8,388,608 B) | transparent icon |
| `icon::cache` arena, `raster` buffer | the size drawn | 256 KiB, 1 MiB (262,400 B, 1,052,672 B) | no icon / blank mask, retried next frame |
| `text` glyph arena | a glyph's bounds | 1 MiB (293,970 B) | glyph not drawn, retried |

The tray-theme row is the most reachable of these: an item chooses its
`IconThemePath` (a same-user peer), so the PNG's bytes are the peer's, and the
decode runs on the bus turn.

**Checked and fine**

- D-Bus: an over-cap message is discarded as it streams (`apply_discard` after
  every 8 KiB read, at most `DISCARD_BUDGET_PER_PUMP` a turn), never staged, so
  a 128 MiB declared length costs no memory; reading stops at the cap plus a fixed
  watermark (`read_watermark`). `read_pixmaps` checks `width x height x 4`
  against the array length with sides <= 256, so no product overflows or
  allocates from a declared size; entries are borrowed slices.
- Config: the file is 64 KiB (`read_bytes`, `take(MAX_FILE + 1)`); every
  `Vec::with_capacity(items.len())` in `config/` is over an already parsed
  array of that file. `bar.radius`/`popup-radius` are <= `MAX_RADIUS` (512),
  height <= `MAX_HEIGHT` (1024).
- Control socket line buffer: grows to `max + 1` (64 KiB) by `reserve_exact`.
- Keymap: read `take(MAX_KEYMAP_BYTES + 1)` (1 MiB).
- `ShmBuffer`/`Canvas::draw`/popup buffers: `Geometry::xrgb8888` checks the
  `i32` bound, `memfd_create`/`ftruncate`/`mmap` are fallible, and the error is
  `DrawError::Shm`. `Scale::buffer`, `scaled_length` and `device` are checked or
  saturating.
- `Corners`/`Shadow` tables with the scale cut: bounded by `radius` <= 512 x 16.
- Module count (32), view text (256 B), exec lines (4096 B), tray items,
  menus, the glyph cache (512 glyphs, 4 MiB) and icon cache (4 MiB): constants.
- `clock/zone.rs` TZif read: 64 KiB cap.
- `Layer` (`scootui::paint`, its `begin` is an infallible `resize` up to
  `MAX_LAYER_PIXELS`, 8 MB) is scootnotify's, not scootbar's, and is left.
- `snapshots.rs` is test-only.

**Not fixable here, left as it is**

- The `png` crate's decoder working memory and `ab_glyph`'s per-glyph `f32`
  rasterizer buffer (up to 16 MiB for one glyph at the `MAX_RASTER` bound) are
  the libraries' own infallible allocations, as the scaler's weight tables were.
  A limit tighter than they need still ends the bar. Both are bounded by
  constants here (decoder: the 16 MiB budget and header checks; glyph:
  `raster_size`). Said in `troubleshooting.md`.
- `modules/volume/mod.rs::cookie` reads `~/.config/pulse/cookie` with
  `std::fs::read`, unbounded. It is the user's own file (not external input),
  so it is not an allocation hazard worth a change here, but a FIFO there
  would block the open; not touched, noted.
- Strict overcommit remains racy where a limit is probabilistic: a refusal is
  honored when it comes, but memory taken by another process after a check is
  not a case a check can close.

How the tests refuse memory: a real `RLIMIT_AS` was tried first and abandoned,
because glibc serves a request from address space it already holds (a spawned
thread's 64 MiB arena reserve, the roughly 2 MiB the first `brk` takes) before
asking the kernel, so a limit of "now plus a little" refused a 40 MiB block
only sometimes and a 1 MiB block never (seen with `strace`: an `mmap` refused
with `ENOMEM` and the `Vec` allocated anyway). The test child's
`scootbg_mem::CountingAlloc` answers null for every block past a size instead,
which is what the system allocator does when it is out, deterministically.

Still open from this entry: the suspend/resume test and multi-day soak, and
the restart-policy docs half.
