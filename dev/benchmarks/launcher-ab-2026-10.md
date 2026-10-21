# Launcher A/B bench: scootlaunch vs fuzzel (M2, 2026-10-20, re-run 2026-10-21)

Release binaries only: `scoot 0.4.1` / `scootlaunch 0.4.1` (aarch64,
`gh release download v0.4.1`, SHA256SUMS-checked) against the installed
`fuzzel 1.14.1` on the same M2, same headless scoot session, same
DejaVu Sans face at a comparable pixel size for both. Scripts:
`scripts/launcher-ab-bench/` (bench.sh, gen-entries.sh, stats.awk).
Raw per-iteration columns live in
`dev/benchmarks/launcher-ab-2026-10-data/` (30 lines each, committed
with the bench); every table below is timeouts/n/min/median/p95/max/mean
over N=30, no outliers discarded. Reproduce any cell with e.g.
`awk -f scripts/launcher-ab-bench/stats.awk dev/benchmarks/launcher-ab-2026-10-data/launch-fuzzel-1000-warm.txt`
(query files hold `total<TAB>update` per line: `cut -f1` for totals,
`cut -f2` for updates).

## Machine state

- Apple M2 (Asahi), NixOS `7.1.13`, aarch64, 8 cores, idle
  (`/proc/loadavg` 0.00 0.05 0.03 at run start), Nix 2.34.8.
- Compositor per run: the released `scoot` binary,
  `--headless --width 800 --height 600`, cpu renderer, `--config /dev/null`.
- `scoot 0.4.1 (ipc protocol 12)`, `scootlaunch 0.4.1`,
  `fuzzel version: 1.14.1 +cairo +png +svg(resvg) -assertions`.
- Binary SHAs (pristine downloads, SHA256SUMS-checked):
  `scoot-0.4.1-aarch64-linux`
  `a7adb11c4ac22dcbc963bb80ac6f24feb3e7616edc8daae0948be1aa9572ba59`,
  `scootlaunch-0.4.1-aarch64-linux`
  `5fc1cd17396b8663b04d26fb398c1374047816c3d68dedc4ccc44f682a32ae0e`.
  (An earlier revision of this doc pinned different SHAs
  (`2d8f5c…`, `04ea9f…`) that match nothing in the release's
  SHA256SUMS; where they came from is unknown, and they were wrong.
  The v0.4.1 release was published once, 2026-10-20T18:57:11Z; the
  values above verify against its SHA256SUMS.)
- Font for both: DejaVu Sans. scootlaunch draws its 13.0 px default
  (look.rs `DEFAULT_FONT_SIZE_PX`) from the DejaVu Sans file passed via
  `--font`
  (`...-home-manager-path/share/fonts/truetype/DejaVuSans.ttf`);
  fuzzel gets `--font "DejaVu Sans:size=10"`, the same face at
  10 pt ≈ 13.3 px at 96 DPI. Both launchers also run with
  `XDG_CONFIG_HOME` pointed at an empty private dir, so neither reads
  operator config (the M2's real `fuzzel/fuzzel.ini` sets
  `DroidSansM Nerd Font:size=11`, which the first run accidentally
  used for fuzzel; this re-run fixes that).
- Synthetic entries: `gen-entries.sh` (deterministic, realistic
  names/generic-names/keywords, no `Icon=` lines, `Exec=/bin/true`,
  no `TryExec`), counts 100 / 1000 / 5000, each launcher run with
  `XDG_DATA_HOME` empty and `XDG_DATA_DIRS` set to only that dir.
  Indexed counts (`--list`): 100, 1000, **4096** — scootlaunch caps at
  `MAX_ENTRIES` 4096 (further files skipped with a note); fuzzel parses
  all 5000.

## How it was timed (and the floor)

`scoot msg windows` lists no layer-shell surfaces, so no cheap poll
exists: detection is screenshot-diff polling (`scoot msg screenshot
--out`, `cmp` against the launcher-less base frame). Method floor,
measured first with nothing open (N=30 screenshot round trips):

- floor: timeouts=0 n=30 min=16 median=36 p95=39 max=39 mean=30 (ms)

Every launch number below therefore includes one screenshot round
trip (~16–39 ms). Processes are killed by exact captured PID, never by
pattern. "Cold" for scootlaunch = fresh index cache per iteration
(page cache could not be dropped without root: stated, not worked
around); every fuzzel launch re-parses desktop files, so it has no
cold/warm split. Query = one fixed 5-char string `term5`, typed via a
single `scoot msg type`; the update clock starts at the type call and
stops at the first frame differing from the pre-keystroke reference
(0.5 s settle before typing; totals include that settle plus launch).
Controls: with no typing both panels' frames are byte-stable over 2 s
(no caret-blink race); caret phase is inside the reference anyway.

NixOS cannot exec the generic release binaries directly (no `/lib64`
loader, stub-ld only), so the bench ran patchelf'd *copies* (originals
untouched, SHAs above): interpreter set to nixpkgs glibc 2.42-84,
rpath at the nixpkgs providers of the needed libs
(scootlaunch: libxkbcommon; scoot: + libinput/seatd/systemd-minimal-libs/pixman).
Same code, only loader paths differ.

## M1: launch to first frame (ms, exec to mapped first buffer)

| entries | scootlaunch cold (no index) | scootlaunch warm (indexed) | fuzzel (parses each launch) |
|---|---|---|---|
| 100 | 0/30/34/**59**/77/77/60 | 0/30/19/**59**/76/76/59 | 0/30/32/**55**/125/146/69 |
| 1000 | 0/30/41/**57**/61/73/56 | 0/30/38/**58**/76/76/59 | 0/30/50/**74**/115/130/79 |
| 5000 | 0/30/82/**102**/140/142/107 | 0/30/62/**101**/144/145/105 | 0/30/158/**184**/215/245/185 |

Cells read timeouts/n/min/median/p95/max/mean. scootlaunch's index
build is lost in the noise (cold ≈ warm at every size). At 100
entries the two are tied inside the method floor (fuzzel's median is
4 ms ahead, its p95 50 ms behind); at 1000 the gap is 16–17 ms,
likewise below the 36 ms floor median. The launch gap that stands
above the floor is at 5000 (median 184 vs 101–102).

## M2: launch + fixed query (ms; total = exec to updated frame, update = keystroke to updated frame)

scootlaunch cold / warm, then fuzzel; cells again
timeouts/n/min/median/p95/max/mean.

| entries | sl cold total | sl cold update | sl warm total | sl warm update | fz total | fz update |
|---|---|---|---|---|---|---|
| 100 | 0/30/590/**606**/625/626/607 | 0/30/22/**25**/42/43/26 | 0/30/573/**603**/630/647/601 | 0/30/20/**25**/40/41/26 | 0/30/595/**633**/682/695/635 | 0/30/19/**26**/62/68/29 |
| 1000 | 0/30/589/**606**/628/630/605 | 0/30/19/**25**/27/27/25 | 0/30/592/**606**/623/623/606 | 0/30/19/**25**/30/41/25 | 0/30/601/**654**/701/718/654 | 0/30/27/**47**/66/80/49 |
| 5000 | 0/30/614/**649**/679/687/650 | 0/30/24/**26**/60/61/28 | 0/30/614/**650**/665/685/646 | 0/30/25/**26**/29/43/26 | 0/30/731/**758**/839/840/767 | 0/30/31/**53**/69/75/53 |

Totals carry the 500 ms settle plus launch (compare M1 for the launch
share). The update column is the ranking/redraw path:
scootlaunch is flat at 25–26 ms median at every size and index state;
fuzzel climbs 26 → 47 → 53 ms median with entry count. No-index vs
indexed is indistinguishable for scootlaunch here. The update medians
(25–26 ms) sit below the 36 ms floor median, so the absolute
"keystroke-to-frame 25 ms" value is not literally true — the floor is
not a simple additive bias on the update path; only the ranking
(scootlaunch flat, fuzzel climbing) is valid.

## M3: memory while open (kB, `/proc/PID/smaps_rollup`, 2 s sampling)

| entries | scootlaunch peak RSS/PSS | scootlaunch steady | fuzzel peak RSS/PSS | fuzzel steady |
|---|---|---|---|---|
| 100 | 5408 / 3004 | same | 16784 / 10015 | same |
| 1000 | 6688 / 4284 | same | 18208 / 11423 | same |
| 5000 | 11120 / 8742 | same | 24784 / 17999 | same |

Peak == steady in every cell (no load-time spike survives the 2 s
window). scootlaunch holds roughly a third of fuzzel's PSS at every
size (2.9–3.0/4.3/8.7 vs 10.0/11.4/18.0 MB); both grow ~linearly with
entries (~1.1 kB/entry PSS for scootlaunch past 1000, ~1.6 kB for
fuzzel).

Sizes on disk: `scootlaunch-0.4.1-aarch64-linux` 2,101,936 bytes
(release with the default `svg` feature; the panel doc's 2,167,544 is
the local stripped build, same feature set), `fuzzel` (nixpkgs
1.14.1) 371,560 bytes with a ~160 MB nix closure
(`nix path-info --closure-size`: 160,116,360 bytes — cairo/pango
stack). scootlaunch needs `libxkbcommon.so.0` (+ libc); scoot headless
needs libinput/libseat/libudev/libpixman/libxkbcommon. The release
`scoot` binary is 9,995,696 bytes.

## M4: idle cost (launcher open, idle, 30 s)

| launcher | cpu ticks (utime+stime) | voluntary ctxt switches |
|---|---|---|
| scootlaunch (100 entries) | 0 | 0 |
| fuzzel (100 entries) | 0 | 0 |

Neither wakes while idle. Resident cost when closed: no `scootlaunch`
process and no `fuzzel` process remains after close (verified by exact
name after every cell) — neither runs a daemon; one process per open,
as documented for scootlaunch.

## M5: feature/behavior parity (docs + tried on the M2)

Ranking quality on this box's 13 real entries, top-3 per query
(scootlaunch via `--query`, fuzzel typed in app mode, read off
screenshots), re-run on the corrected setup (sandboxed config, DejaVu
Sans both sides): identical to the first run, both always put the
obvious winner first; order past first differs (frecency/prefix
weights differ).

| query | scootlaunch top-3 | fuzzel top-3 |
|---|---|---|
| fire | Firefox | Firefox |
| term | Alacritty, Foot Server, Foot | Foot Server, Alacritty, Foot Client, Foot |
| foot | Foot Server, Foot, Foot Client | Foot, Foot Client, Foot Server |
| chrome | Google Chrome | Google Chrome |
| ghost | Ghostty, Google Chrome | Ghostty |
| alac | Alacritty | Alacritty |
| mpv | mpv Media Player | mpv Media Player |
| nvim | Neovim wrapper | Neovim wrapper |
| set | Qt6 Settings, Foot Server, Foot | Qt6 Settings |
| map | mpv Media Player, Neovim wrapper, Ghostty | Neovim wrapper, mpv Media Player |

Both always put the obvious winner first; order past first differs
(frecency/prefix weights differ). `music`/`pdf`/`calc`/`mail` match
nothing on this box in either (no such apps installed).

| area | scootlaunch (tried) | fuzzel (tried) |
|---|---|---|
| app search | index + frecency; empty query = frecency then name | same class, frecency too |
| icons | real theme icons where present (Firefox/Chrome/foot/ghostty/mpv resolved; rest letter tiles) | real icons too (Firefox/foot/Alacritty/Ghostty rendered) |
| theming | look layer (`<state>/look/launch.toml`), per-invocation `rrggbbaa` flags | `fuzzel.ini` + same seven roles as CLI flags via repo wrapper |
| dmenu/stdin | `--dmenu`, fuzzel's flag meanings; empty query shows stdin order; `--index/--placeholder/--password/--prompt-only` verified by docs, pick path tried | same surface; `--dmenu0`, `--select/--select-index` extras scootlaunch refuses loudly (exit 2) |
| run/PATH mode | `--run` (alias `--list-executables-in-path`): panel over PATH executables only (tried, lists `accessdb…`) | `--list-executables-in-path` *adds* PATH executables to the app list (tried: apps plus `accessdb`, `NetworkManager`) — different semantics behind a near-identical flag |
| calculator/modes | none (panel/dmenu/run) | none (drun/dmenu) |
| terminal apps | `Terminal=true` prefixes default terminal (`foot`); `--run-id` on a scratch `Terminal=true` entry returned ok with pid+scope (the terminal itself needs a Wayland connection, absent headless) | man documents a terminal-launch command for `Terminal=true` (not live-tried) |
| keyboard | type/Up/Down/Enter/Escape/Ctrl+Enter tried; second instance toggles (first exits 1, second does not stay open) | same flows tried; dmenu Enter picks; app-mode arrows work |
| mouse | row click picks in dmenu (first run: clicked beta, printed `beta`; re-probe via `scoot msg pointer` could not reproduce — cursor placement under headless IPC maps ~60 px off, so no effective click landed; keyboard pick re-tried: typed beta, printed `beta`) | hover/click had no effect under this scoot build (re-verified on the corrected setup: highlight stays on the first row through move and click, launcher stays open; cursor moves) — not isolated whether scoot layer-shell pointer focus or fuzzel; keyboard flows unaffected |
| exit paths | pick 0, dismiss 1 (panel and dmenu, re-tried); empty+`--no-run-if-empty` 1 without opening (tried) | pick 0 (tried); dmenu Escape **2** (re-tried), app-mode Escape **0** (re-tried); empty+`--no-run-if-empty` 1 (tried) |
| agent surface | `--list/--query --json`, `--run-id` (tried) | none |

Fair notes for fuzzel: smaller binary (372 kB vs 2.1 MB), no 4096-entry
cap, dmenu extras (`--dmenu0`, `--select`), driven entirely by
keybindings users already have. Behind for scootlaunch: dmenu exit
codes differ (dismiss 1 vs 2 — scripts keying on fuzzel's 2 break),
`--run` semantics differ behind a confusingly similar flag, mouse
unverified for fuzzel here, no fuzzel.ini compat (by design — set the
face in `launch.toml`).

Re-run scope: ranking (all 10 probes both sides), exit codes,
scootlaunch `--run` (panel shot re-taken, PATH executables only),
fuzzel icons (theme icons visible in the re-run screenshots) and the
fuzzel mouse no-op were re-tried on the corrected setup with the same
results; the remaining rows are carried over from the first run
(same binaries, same box).

## Caveats

- Timing floor is the screenshot round trip (median 36, p95 39); M1
  medians include it. Relative gaps stand; absolute "time to photons"
  is lower by roughly the floor.
- At 100 and 1000 entries the launch gap sits below the floor
  (100: medians 59 vs 55; 1000: 57–58 vs 74 — both gaps under the
  36 ms floor median and its 16–39 spread), so those cells only show
  a tie, not a lead either way. The launch conclusion rests on 5000
  entries (gap ~82 ms), the query-update trend, and memory.
- "Cold" = no index, not cold page cache (no root to drop it; stated
  per the brief). One single first-ever panel open took ~15 s to first
  frame; every later cold-cache open measured 35–150 ms. The outlier
  happened once, was not reproduced across 90+ cold opens, and its
  cause was not isolated — treat flipping criteria accordingly.
- 800×600 snapshot at cpu rendering; absolute ms scale with output
  size and renderer, relative ordering should not.
- M2 totals include a fixed 0.5 s settle; the update column is the
  comparable figure. Query is one string (`term5`); ranking cost varies
  with query selectivity.
- Synthetic entries carry no icons: icon decode cost is excluded for
  both (real-data screenshots show both resolve theme icons).
- fuzzel measured is the profile's 1.14.1 (nixpkgs build); the bench
  did not `nix shell` a second copy.
- Mouse/fuzzel finding is one box, one build, keyboard-first scope —
  reported, not root-caused.

## Recommendation

The data supports flipping the default to scootlaunch, with
conditions. scootlaunch is tied inside the floor at 100–1000 entries
and clearly faster at 5000 (101–102 vs 184 ms median
launch-to-first-frame), keystroke-to-frame flat at ~25 ms while
fuzzel climbs to 53 ms at 5000, PSS roughly a third
(3.0/4.3/8.7 vs 10.0/11.4/18.0 MB), zero idle cost and zero resident
cost on both sides. Ranking agrees on the winner for all 10 probes.
Conditions before the flip: (1) decide the dmenu dismiss-code break
(scootlaunch 1 vs fuzzel 2) and the `--run`-vs-additive flag mismatch
— both are script-visible incompatibilities, not just looks;
(2) keep fuzzel one line away (`daemon = "fuzzel"`) until the
first-launch outlier above is either reproduced-and-fixed or ages out;
(3) re-run the mouse row against a newer scoot before claiming parity
there. Nothing in the numbers argues for staying on fuzzel by default.
