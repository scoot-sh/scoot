# Benchmarks

Measured resource usage: the method, the raw numbers, and what each number
can and cannot show. Every row says what it measured. The caveats sit next
to the results because some rows are not like-for-like, and each place where
they aren't is marked.

- [Whole-desktop idle cost on the Asahi M2: scoot vs niri vs Hyprland vs GNOME vs KDE Plasma (2026-10-05)](#whole-desktop-idle-cost-on-the-asahi-m2-scoot-vs-niri-vs-hyprland-vs-gnome-vs-kde-plasma-2026-10-05)
- [scoot vs niri, nested on the dev VM (2026-09-24)](#scoot-vs-niri-nested-on-the-dev-vm-2026-09-24)
- [scoot vs niri on a real GPU, nested and `--tty` (2026-09-25)](#scoot-vs-niri-on-a-real-gpu-nested-and---tty-2026-09-25)
- scoot's own tiers on real hardware (dumb buffers + pixman vs GPU scanout
  on an Apple M2) are in [`Asahi.md`](../Asahi.md) Test 4, summarised in
  [backends](https://www.scoot.sh/scoot/backends.md#which-renderer-draws-the-frames).

## Whole-desktop idle cost on the Asahi M2: scoot vs niri vs Hyprland vs GNOME vs KDE Plasma (2026-10-05)

How much does each desktop cost at rest, set up the way its own project
recommends — and where should scoot get cheaper? The headline: scoot's
compositor is the lightest of the five (63.6 MB PSS against niri's 83,
Hyprland's 128, KWin's 126 and gnome-shell's ~160), its bar is 7–9x
lighter than waybar and 58x lighter than plasmashell — but its whole
*session* burns the most CPU (0.60% of a core against niri's 0.05%,
KDE's 0.17%, Hyprland's 0.40% and GNOME's 0.53%), and essentially all of
it is scoot's own session plumbing: the launcher's 1-second poll plus
the user-manager round trips it causes (35 of 36 ticks). Fixing that
one ticket takes the session to ~0.03% — lightest of all. The ranked
leads are at the end; each is filed in the backlog and linked here.
*Correction (2026-10-05): the lean scoot cohort below showed no
wallpaper — its image file was unreadable to the test user
(`Permission denied`), so the compositor's background showed. With the
wallpaper up the session is ~12 MB higher (~209 MB; compositor
~75.6 MB) — still the lightest (leads 2–3). No number below is
rewritten; each is labeled with its cohort.*

### What ran

| | |
|---|---|
| Machine | Apple MacBook Air M2 under Asahi Linux (NixOS 26.11pre-git, kernel 7.1.13, Mesa 26.2.2 `asahi`), eDP-1 + DP-1 connected as found, panel at 100% (509/509) for every round, volume 100%, Wi-Fi as is |
| scoot | 0.1.0, origin/main at `5f96802` (a `path:` input of a throwaway wrapper flake, so the tree is pinned by construction), desktop profile `programs.scoot.desktop.enable = true`, `look = "moonrise"` (idle/lock, notifications, keymap, bar, wallpaper; clipboard slot off — PR #443 is unmerged and the slot is inert) |
| niri | 26.04 (`programs.niri.enable`) + its wiki's [Important software](https://github.com/YaLTeR/niri/wiki/Important-software) (mako, xdg-desktop-portal-gtk + xdg-desktop-portal-gnome + gnome-keyring, an auth agent, xwayland-satellite) and [Getting Started](https://github.com/YaLTeR/niri/wiki/Getting-Started) defaults (waybar 0.15.0 spawned, animations on, alacritty/fuzzel/swaylock defaults noted, foot used for the measurement) |
| Hyprland | 0.56.2 (`programs.hyprland.enable`, uwsm off — plain `start-hyprland`, the upstream default) + its wiki's [Must have](https://wiki.hypr.land/Useful-Utilities/Must-have/) (mako, PipeWire, xdg-desktop-portal-hyprland, hyprpolkitagent 0.1.3, Qt 5+6 Wayland, fonts) + the wiki's [status-bar example](https://wiki.hypr.land/Useful-Utilities/Status-bars/) (waybar) + hyprpaper/hypridle/hyprlock |
| GNOME | `services.desktopManager.gnome.enable` defaults (gnome-session 50.1, gnome-shell + mutter 50.4) |
| KDE Plasma | `services.desktopManager.plasma6.enable` defaults (plasma-workspace 6.7.5, kwin 6.7.5) |

Matched everywhere: moonrise wallpaper
(`docs/assets/wallpapers/moonrise.png`) in fill mode, foot 1.28.0 with
`DroidSansM Nerd Font Propo:size=11` showing an idle shell, bar content
workspaces + clock + network + volume + battery (scootbar; waybar with
`niri/workspaces` / `hyprland/workspaces`; GNOME/KDE stock shells, whose
content is recorded below), DP-1 at scale 1.0, notification daemons
running with nothing queued, PipeWire + WirePlumber + rtkit for all five
(the Hyprland Must-have lists PipeWire; GNOME/KDE need it; the scootbar
volume module speaks the PulseAudio protocol). Idle policy matched
where possible: dim 2 min, lock 4 min, screens off 5 min (scoot profile
defaults, mirrored in a swayidle script for niri/Hyprland; GNOME/KDE
run their own managers at defaults).

Deliberate deviations, each with its reason:

- eDP-1 scale is 1.5 everywhere except Hyprland (1.6) and GNOME (1.667):
  Hyprland 0.56.2 accepts the `1.5` rule (keyword returns ok) and keeps
  rendering at 1.6; GNOME offers no 1.5 without fractional scaling, and
  enabling `scale-monitor-framebuffer` plus a `monitors.xml` with 1.5
  did not stick across two re-logins. Both are recorded, not forced —
  at idle (no redraws) scale changes buffer sizes only.
- scoot's bar is the profile's bar minus the moonrise example's extras
  (no window title, no `load`/`cpu` exec pollers, no launcher buttons):
  matched content, and exec pollers would add wakeups with no
  counterpart elsewhere.
- niri/Hyprland run waybar with the matched config instead of waybar's
  own default config (whose sway modules are inert under niri).
- niri/Hyprland lock with plain swaylock (the profile themes its own;
  the locker never fires inside any sample window, so theming is out of
  the idle-cost picture).
- KDE's wallpaper tool refused the in-tree path, so it runs a copy in
  `~/Pictures` (same bytes, `preserveAspectCrop` = fill).
- The profile's user units (`scoot-idle`, `scootbar`, the notification
  feed, the audio inhibitor) start in *every* session via
  `graphical-session.target`, so they were stopped post-login everywhere
  but scoot. mako is the exception: the profile's mako doubles as the
  niri/Hyprland notification daemon (same binary, themed config —
  noted); in GNOME/KDE it was stopped so their own daemons own the bus.
- niri spawns its own xwayland-satellite, so the config's duplicate was
  removed (killed post-login); hyprpolkitagent ships no `bin/` and has
  no NixOS module, so it was started from its store `libexec` path.
- A `systemd-inhibit` sleep/idle lock (block) ran for the whole
  campaign: an auto-suspend on an unattended box would strand it with
  nobody to wake it. One sleeping process, outside every sample.
- Settle is 170 s, not 180 s: the 240 s lock timeout would end a
  180+60 s window exactly at the mark, so 170 + 60 = 230 s keeps
  margin. No locker fired in any of the 15 clean rounds (verified in
  the process lists).

### Method (fixed before measuring; what broke is recorded too)

One temporary system (`~/fx/cmpde-cerval`, a wrapper flake around the
maintainer's `~/nixos-config` with the same inputs and copied lock, the
scoot input overridden to the pinned origin/main tree; never committed)
provides all five `.desktop` sessions. greetd + ReGreet stays the login
manager throughout (no GDM or SDDM; verified `display-manager` is still
greetd after the switch). The `scoot-test` user logs in through greetd
IPC (the `~/fx/greet-login/login.py` pattern, with the session name
patched per desktop) only when no `scoot-test`/`steve` session holds
seat0; other agents may use the seat, so every login checks first. Home
state was wiped to the same baseline before each desktop's first round
(all non-symlink files under a targeted list — caches, dconf,
desktop-state dirs — while Home Manager symlinks and nix state stay);
the whole home was snapshotted beforehand and restored after.

Per desktop, 3 rounds: log in, open one foot, settle 170 s, sample
60 s. The sampler (`sample.sh`: per-process utime+stime from
`/proc/<pid>/stat`, voluntary/involuntary switches from `status`,
PSS/RSS from `smaps_rollup`, plus appeared/exited processes) runs as
root so every process reads. Settle flatness was polled per 30 s in the
first campaign (scoot ~16, niri 1–4, Hyprland 0–3, GNOME ~16, KDE 4–7
ticks per 30 s for uid 1001); the second campaign sleeps the same
170 s. Reported: median and range. Mapping-class splits (`smaps.sh`:
heap/stack/shm/drm/file-so/...) were taken for each desktop's
compositor and companions. Power: `power_now` averaged over 120 s
(`~/fx/pwr.sh`) per desktop with screens at 509, valid only while
discharging.

What broke, and what was done about it:

- `sample.sh` read utime/stime with `$12`/`$13` — in shell that is
  `${1}2`, so every CPU number in the first campaign is void (all
  zeros). Memory, wakeups and PSS/RSS from that campaign are valid
  (separate reads). Fixed to `${12}`/`${13}`, verified against
  `$14+$15`, and the whole CPU series re-measured (the `*-c*`
  rounds); memory uses both campaigns. The per-role tables give the median of the three
  `*-c*` rounds; where the six scoot sessions split into a lean and a fat cohort (lead 2), the
  compositor row says so. *Correction (2026-10-05): lean turned out to
   be no wallpaper at all (the image file was unreadable to the test
   user); fat is the wallpaper up, ~12 MB higher — session ~209 MB,
   compositor ~75.6 MB.*
- `loginctl terminate-session` does not reap everything: sudo-launched
  foots always linger, and KDE's kwin+plasmashell survived two
  logouts, so KDE rounds 2–3 ran on round 1's compositor (discarded,
  redone). Every other round was audited process-by-process (`lstart`
  in each round's `ps0`): clean. From the redo on, every round ends
  with `pkill -9 -u scoot-test` and a zero-process check.
- First login: niri's config with single-line `output … { scale …
  }` blocks is invalid KDL — niri fell back to its default config
  (wrong scale, wrong spawns). Rewrote multi-line, proved with `niri
  validate`, re-measured. That invalid session's numbers are excluded.
- GNOME needed a longer tail than the method's settle: background
  activity persists ~15–20 min post-login (a profile mako unit
  crash-looping on gnome-shell's bus name added ~15 ticks/30 s of
  churn until stopped — an artifact of the shared test user, excluded
  from every sample by the endpoint-diff). A 300 s validation sample
  reads zero ticks anywhere, corroborating the settled median.
- Screenshots: `grim` works under niri/Hyprland, fails under GNOME
  (no wlr-screencopy) and KDE; GNOME's Shell screenshot API refused in
  the unattended session (recorded, skipped); KDE's came via
  `spectacle -b`. scoot's came through its own IPC.

### Results

Whole-session totals (uid 1001 incl. the user manager, one idle foot,
and every daemon the session runs): median of 3 rounds, range in
brackets. CPU is ticks/60 s of `_SC_CLK_TCK` = 100 (36 ticks = 0.60% of
a core); wakeups are voluntary context switches per 60 s.

| desktop | CPU ticks | % of a core | wakeups | PSS | RSS |
|---|---|---|---|---|---|
| scoot | 36 [35–37] | 0.60 | 470 [458–477] | 197 MB [197–203] (lean: no wallpaper; ~209 with it up, leads 2–3) | 331 MB [331–359] |
| niri | 3 [3–6] | 0.05 | 184 [179–185] | 443 MB (range not recorded) | 938 MB [937–938] |
| Hyprland | 24 [21–25] | 0.40 | 1173 [1170–1205] | 385 MB [384–385] | 693 MB [692–693] |
| GNOME | 32 [23–33] | 0.53 | 295 [190–320] | 657 MB [656–662] | 1563 MB [1562–1569] |
| KDE Plasma | 10 [9–11] | 0.17 | 213 [154–384] | 764 MB [760–772] | 1752 MB [1748–1785] |

Per-role medians (ticks / wakeups per 60 s / PSS; one idle foot everywhere):

| role | scoot | niri | Hyprland | GNOME | KDE |
|---|---|---|---|---|---|
| compositor | 1 / 18 / 63.6 MB (lean cohort = no wallpaper; 75.6–75.9 in the fat one with it up, lead 2) | 2 / 85 / 83 MB | 1 / 235 / 128.5 MB | 27 / 58–97 / ~160 MB | 1 / 18–60 / 126–132 MB |
| bar / shell | 0 / 39 / 4.6 MB (scootbar) | 1–2 / 66 / 34.4 MB (waybar) | 1 / 66 / 42 MB (waybar) | in-shell | 6–7 / 93–135 / 266–271 MB (plasmashell) |
| wallpaper | 0 / 0 / 2.1–14.7 MB (scootbg, bimodal: lean showed no wallpaper, fat holds the output-sized floor — see leads) | 0 / 0 / 2.0 MB (swaybg) | 0–1 / 20 / 40.5 MB (hyprpaper) | in-shell | in-shell |
| notifications | 0 / 0 / 3.7 MB (mako) | 0 / 0 / 7.4 MB (mako) | 0 / 0 / 3.2 MB (mako) | in-shell | in-shell |
| idle / lock | 0 / 0 / ~1 MB (swayidle) | 0 / 0 / ~1 MB | 0 / 0 / ~1 MB | 2–3 / 44–104 / 2.4 MB (gsd-power) | 1 / 2 / 19 MB (powerdevil) |
| portals | on-demand (0 at sample) | 0 / ~5 / 52 MB (always on) | on-demand (0 at sample) | 0 / 3 / 44 MB | 1 / 7 / 44 MB |
| polkit agent | — (profile has none yet) | 0 / 0 / 26 MB | 0 / 0 / 27 MB | in-shell | 0 / 0–1 / 14 MB |
| audio (PipeWire etc.) | 0 / ~0 / ~44 MB | 0 / ~0 / ~42 MB | 0 / ~0 / ~40 MB | 0 / ~0 / ~40 MB | 0 / ~0 / ~43 MB |
| terminal (foot+bash) | 0 / ~0 / 26–29 MB | 0 / ~0 / 19 MB | 0 / 45 / 25–26 MB | 0 / ~0 / 14 MB | 0 / ~0 / 20–21 MB |
| XWayland | — (off by default) | 0 / 0 / 46 MB | 0 / 0 / 41 MB | 0 / 0 / 77 MB | 0 / 0 / 40 MB |
| session plumbing | 35 / ~410 / 46–51 MB | 0 / ~27 / 110 MB | 22 / ~810 / 32 MB | 0–5 / 126–259 / 262 MB | 1 / 34–180 / 103–110 MB |

Reading it role by role:

- **Compositor.** scoot is the lightest resident (63.6 MB lean — no
  wallpaper, see leads — and ~75.6 MB with it up, still ahead of niri;
  its `smaps`
  split: heap 32 kB, anon 8.2 MB, shm 12–24 MB, file `.so` 16–32 MB —
  with two rows to verify, see leads), then niri (83 MB), KWin
  (126–132 MB), Hyprland (128–129 MB) and gnome-shell (~160 MB, plus
  its calendar server). At idle the compositor's CPU is ~0 everywhere
  except gnome-shell (~25 ticks). scoot apparently draws no frames at idle (compositor 1 tick and ~15–18 wakes per 60 s; no frame
  counter was sampled) and wakes ~15 times a minute; niri wakes ~85 (about half its session's
  total); Hyprland wakes ~235.
- **Bar / shell.** scootbar (4.6 MB, ~39 wakes) against waybar (34
  MB under niri, 42 MB under Hyprland, 64–68 wakes — same ~1/s clock
  tick, GTK tax on memory) and plasmashell (266–271 MB, ~100+ wakes:
  the shell is the desktop). GNOME's top bar lives in the shell
  process. scootbar is 7–9x lighter than waybar and 58x lighter than
  plasmashell, with slightly fewer wakeups than waybar.
- **Wallpaper.** scootbg-lean (2.1 MB) showed the compositor's
  background, not the wallpaper — it matches swaybg (2.0 MB) only in
  the sense that nothing was drawn;
  scootbg-fat (14.7 MB) does not, and neither matches hyprpaper
  (40.5 MB — the single heaviest wallpaper daemon here). See leads.
- **Idle policy.** swayidle costs ~1 MB and nothing anywhere. The
  desktops' own managers cost more resident (powerdevil 19 MB,
  gsd-power 2.4 MB with 2–3 ticks) for dim/blank/suspend logic scoot
  does in one daemon.
- **Portals.** scoot's and Hyprland's start on demand (nothing resident
  at the sample); niri's recommended stack keeps 52 MB warm
  (portal-gnome 28, main 11, gtk 8) plus an ibus stack near 60 MB that
  nothing else measured pulls in. GNOME/KDE carry 44 MB each.
- **Session plumbing.** This is where scoot *loses*: 35 of its 36
  ticks and ~400 of its 470 wakeups are `scoot-session`'s 1 s poll
  (4–5 ticks, ~200 wakes) plus the user-manager round trips it causes
  (~30 ticks, ~200 wakes) — the already-filed
  [idle-poll ticket](backlog/resolved/session-launcher-idle-poll-done.md), now
  quantified: the fix takes the whole session from 0.60% of a core to
  ~0.03%. Hyprland's plumbing is second noisiest (dbus-broker alone
  wakes ~600 times a minute — something chats constantly on its bus;
  its user manager burns 17–18 ticks), while niri's, GNOME's and KDE's
  managers sleep through the window.
- **What each desktop gives you for its cost.** The totals above buy
  different things. GNOME carries evolution-data-server (~106 MB:
  calendar, address book, alarms), online accounts, ibus, its
  keyring and a 77 MB XWayland stack. KDE carries Discover's update
  checker, baloo, kactivitymanagerd, accessibility, the wallet and a
  40 MB XWayland. niri's recommended stack carries the 52 MB portals,
  the 60 MB ibus stack and a 46 MB XWayland pair. Hyprland carries Qt,
  hyprpaper and its portal backend. scoot carries PipeWire (~40 MB, a
  campaign-level choice shared by all five) and a 15 MB push-notification
  distributor nothing uses (see leads).

Power (screens at 509, 120 s average, valid only while discharging):
scoot **9.73 W**, niri **9.67 W**. The battery sits at the 80% charge
limit, so the method's prescribed drop to 75 bought exactly 5 points
of discharge (80 → 75) before the limiter held the pack again and
`power_now` went invalid — Hyprland/GNOME/KDE power has no number for
that reason, not for lack of trying. The two numbers say the panel and
backlight dominate: two different desktops agree within 0.06 W, i.e. no measurable difference
between them (one 120 s average each; 0.6% apart is below what this method resolves). Charge
state was restored exactly afterwards (threshold 80, timer restarted,
`scoot-charge sync`).

### Optimization leads for scoot

Biggest expected win first. Each is filed; the first confirms and
quantifies the ticket this benchmark was built to check.

1. **Kill the 1 s session poll** ([idle-poll ticket](backlog/resolved/session-launcher-idle-poll-done.md),
   already open): 35 of 36 ticks and ~400 of 470 wakeups per minute.
   Nothing else in the session is within an order of magnitude. The
   fix takes scoot from the most CPU-hungry session here (0.60%) to
   the least (~0.03%, compositor 1 tick + foot 0).
2. **scootbg's bimodal idle image** ([image-retention](scootbg/backlog/resolved/image-retention-done.md)):
   14.7 vs 2.1 MB across identical sessions, and the compositor shows
   the same ~12 MB campaign delta (75.6 vs 63.6 MB). *Correction
   (2026-10-05): traced to two artifacts, not a leak — the fat cohort's
   12 MB "anon" is the two output-sized `wl_shm` pools misfiled by the
   mapping classifier (see below), and the lean cohort never drew the
   wallpaper at all (its image file was unreadable: `Permission denied`,
   so the compositor's background showed). No saving to take; the ticket
   pins the no-retained-copy behavior with a test instead.*
3. **Verify the 17 MB `[stack]`** ([compositor-stack-pss](backlog/resolved/compositor-stack-pss-done.md)):
   the mapping split's largest scoot-owned row after file-backed Mesa.
   If real touched stack, shrink the deep path (up to ~17 MB); if an
   artifact, fix the classifier both this page and the ticket rely on.
   (Hyprland shows the same 17 MB stack row — compare notes, not code.)
   *Correction (2026-10-05): artifact confirmed — the same classifier
   off-by-one credited a client shm buffer's PSS to `[stack]` (live
   `[stack]` is 128 kB); all three fat rounds hold the same ~29.5 MB
   shm. No compositor change.*
4. **Mask the Push portal out** ([kunifiedpush-sessions](backlog/packaging/kunifiedpush-sessions.md)):
   `kunifiedpush-distributor` at ~15 MB in every session for push
   notifications nothing uses — bigger than mako, the bar and the idle
   daemon combined.
5. **Wake the bar only when its text changes** ([second-wakeups](scootbar/backlog/resolved/second-wakeups-done.md), resolved 2026-10-05 in #458):
   ~40 wakes/min. The guess at the time (the clock ticking every second)
   was wrong: the clock is minute-aligned and wakes twice a minute. The
   cause was the network module redrawing on every dBm wobble of its 10 s
   signal re-read; #458 redraws only when the shown level changes.

Where scoot is already lightest, plainly: compositor PSS (63.6 MB lean with no wallpaper, 75.9 in the fat cohort with it up,
against 83/128/126/~160), bar (4.6 MB against 34/42/270), wallpaper at
its leanest (2.1 MB, matching swaybg — *correction (2026-10-05): that
cohort showed the compositor's background, not the wallpaper; with the
wallpaper up scootbg holds its output-sized floor, ~12 MB PSS here,
against swaybg's 2.0 MB, which drops client buffers after upload — a
possible future lever, kept deliberately for now: see
[image-retention](scootbg/backlog/resolved/image-retention-done.md)*),
portals on demand (0 resident
against niri's 52 and 44 each for GNOME/KDE), and no XWayland tax. The notification daemon is
the same small mako everywhere it runs (3–4 MB; the 7.4 MB niri reading is its themed config).

### What these numbers do not show

- Foot's PSS (12–28 MB) is not comparable across desktops: window
  geometry was not controlled, so its shm buffers differ. The
  compositor-side numbers are largely unaffected (second-order, through the client shm buffers the
  compositor holds: a few MB against tens-of-MB gaps).
- The endpoint-diff sampler misses processes that live entirely inside
  the 60 s window (a crash-looping mako unit's children, one-shot
  migrators). The 30 s settle sums bound that blind spot; nothing in
  them contradicts the medians above.
- GNOME (1.667) and Hyprland (1.6) did not take the 1.5 eDP scale (see
  deviations). At idle with no redraws that changes buffer sizes only.
- GNOME's settle never visibly flattens the way the others do
  (+10–16 ticks/30 s for minutes): first-run/background activity with
  a long tail. The 60 s medians are corroborated by a 300 s validation
  sample (zero ticks anywhere) and three extra 60 s probes.
- Screenshots: scoot/niri/Hyprland/KDE at rest are in the evidence
  (`ev/*-edp.png`); GNOME refused both paths (grim unsupported, Shell
  API denied unattended).
- Totals include what each desktop gives you (settings apps, indexers,
  online accounts, animations) — bytes alone would punish the fuller
  desktops for features scoot does not have.
- Evidence: per-process TSVs, `smaps` splits and screenshots under
  `~/fx/cmpde-cerval/ev/` on the Asahi box; the wrapper flake, sampler
  scripts and login helper beside them. Nothing was committed there.

## scoot vs niri on a real GPU, nested and `--tty` (2026-09-25)

The same comparison on an Apple M2 under Asahi Linux (Mesa 26.2.2's
`asahi` driver). Here both compositors' GLES paths render on a real GPU,
and both can own the real panel over `--tty`. The full method, every
check and each raw cell are in [`Asahi.md`](../Asahi.md) Test 9. This is
the summary. The build is `main` at `e1dce6f`, built on the machine; the
niri is nixpkgs' 26.04. CPU is the compositor's process total
(`/proc/PID/stat`), in ms.

### Summary

- **With a real GPU, scoot's GPU tier used the least total CPU for
  relayout and pointer motion on the panel.** On the real 2560x1600 panel at scale 1.5, `--tty --renderer
  gles` spent 2.7% of a core on a relayout storm against niri's 4.6–4.9%.
  Under continuous pointer motion it spent 9.2–9.6% against niri's
  22–23.5%. Both compositors draw the pointer here, and neither has a
  cursor plane to use. **These are totals, not like-for-like work.** No
  frame counts exist on `--tty`, so per-frame cost there is unknown. The
  pointer rows ran at an uncounted event rate. Nested, niri draws about
  three frames per relayout action to scoot's one. Nested, the
  `gpu-scanout` build hands its frames to the host as dma-bufs. It spent
  250 ms on 200 relayout actions against niri's 580, and 820 ms animating
  against niri's 1290 (no DIAG frame count for that build). It was not
  lowest everywhere: nested pointer 100 ms against pixman's 90, and
  nested shot-grim 40 against the read-back tier's 30. On the panel,
  `grim` tied niri (anim on) in round 1.
- **pixman stops being the cheap option.** Nested at 1600x1000 it costs
  7.1 ms per animated frame against niri's 2.4 and delivers 41 frames/s
  against 54. On the panel it takes 44% of a core for relayout and 32–35%
  for pointer motion. It stays the right default without a GPU (see the
  dev VM section below), and it is the wrong choice with one.
- **niri's renderer is about 6–15x cheaper per frame than on llvmpipe**
  (0.94 ms per relayout frame, 2.35 ms per animated frame). scoot-gles on
  the read-back path now matches it per animated frame (2.47 ms), while
  niri still draws about three frames per relayout action to scoot's one.
- **Idle:** neither scoot tier woke even once in 20 s, nested or on the
  panel. niri woke 65–74 times on the panel. **Memory with three
  terminals:** nested, 44 MB (pixman), 81–87 MB (GLES) and 110 MB (niri).
  On the panel, 94 MB, 102 MB and 127 MB.
- **Screenshots** through each compositor's own IPC cost 20–25 ms of CPU
  per 2560x1600 capture on scoot-gpu, 27–32 ms on pixman and 41–44 ms on
  niri. Through `grim`, all are within 12–15 ms. Neither GLES scoot nor
  niri grew by a frame per capture.

### Nested (cage with GLES as the host, 1600x1000, three rounds, medians)

| scene | scoot-pixman | scoot-gles (read-back) | scoot-gles (dma-buf, `gpu-scanout`) | niri, anim off | niri, anim on |
|---|---|---|---|---|---|
| idle 20 s | 0 | 0 | 0 | 10 | 0 |
| pointer, 1200 events | 90 | 100 | 100 | 750 | 760 |
| relayout, 200 actions | 1760 | 510 | 250 | 580 | 630 |
| shot-ipc, 10 | 100 | 80 | 80 | 160 | 160 |
| shot-grim, 10 | 50 | 30 | 40 | 40 | 40 |
| animate 10 s | 2940 | 1270 | 820 | 1290 | 1270 |
| ms per relayout frame (DIAG) | 8.78 | 2.53 | not run | 0.94 | 1.01 |
| ms per animated frame (DIAG) | 7.10 (41/s) | 2.47 (51/s) | not run | 2.38 (54/s) | 2.35 (54/s) |

As on the VM, nested scoot presents no frames for pointer motion because
the host draws the pointer, while niri redraws (628 frames, 1.2 ms each).

### `--tty` on the real panel (2560x1600, scale 1.5 on both, two rounds)

Each cell is round 1, then round 2. `t9b.sh` ran its scenes back to back, with no idle wait between them.

| scene | scoot-pixman | scoot-gpu | niri, anim off | niri, anim on |
|---|---|---|---|---|
| idle 20 s | 0, 0 | 0, 0 | 0, 10 | 0, 0 |
| relayout 10 s | 4390, 4350 | 270, 270 | 470, 460 | 490, 470 |
| pointer 10 s (ydotool) | 3470, 3160 | 960, 920 | 2210, 2220 | 2210, 2350 |
| shot-ipc, 10 | 270, 320 | 200, 250 | 420, 440 | 410, 430 |
| shot-grim, 10 | 150, 150 | 120, 120 | 140, 130 | 120, 130 |

The pointer row is CPU per second at whatever rate ydotool's
one-fork-per-event loop reached. scoot-pixman took about a third fewer
wakeups there, which suggests it served fewer events. **Not measured:**
input-to-present latency, and frame counts on `--tty`.

## scoot vs niri, nested on the dev VM (2026-09-24)

scoot's layout comes from [niri](https://github.com/niri-wm/niri), which
makes niri the obvious reference point. The question here is how much
running each one costs on the same workload, on the one machine where both
could run. That machine is the dev VM, and it can host only half of the
answer. niri renders only through GLES, and it refuses a software renderer
on a real `--tty` session. The VM's GPU has no 3D, so niri could run there
only nested, and every GLES row (niri's and scoot's `--renderer gles`)
rasterises on llvmpipe, a software renderer. The real-GPU half, `--tty`
included, was run on an Apple M2 on 2026-09-25: see
[the next section](#scoot-vs-niri-on-a-real-gpu-nested-and---tty-2026-09-25).
Nothing in this section is a claim about real hardware.

niri is a mature, much fuller compositor: animations, an overview,
screencasting through PipeWire, a hotkey overlay, rich per-window rules and a
lot more that scoot does not have. Some of the footprint below pays for
those features. This page compares costs on a narrow workload, not the value
each compositor gives you.

### Summary

- **Without a GPU,** niri's only option is software GLES, and scoot's
  default pixman renderer is far cheaper than that. Here, scoot spent 1.9–2.5
  ms of CPU per presented frame against niri's 14–16 ms. At rest with three
  terminals it used 49 MB RSS against niri's 185 MB, committed its first
  frame to the host 27 ms after starting against niri's 113 ms, and idled
  with no wakeups at all against niri's ~3 a second (which is already
  negligible).
- **On the same GL stack (scoot `--renderer gles` against niri, both on
  llvmpipe), niri is the more efficient renderer.** A frame cost 14.2 ms in
  niri against 20.5 ms in scoot during relayout, and 14.8 ms against 18.4 ms
  with a client animating. niri also delivered more frames to that client (54
  a second against scoot-gles's 43 and scoot-pixman's 50). scoot-gles used
  about 20% less memory at rest (149 MB RSS against 185 MB). However, it
  **grew by one frame of memory per screenshot while nothing was redrawing**,
  which is a bug this benchmark found. It is
  [fixed](backlog/resolved/gles-capture-leaks-a-frame-per-shot-done.md) as
  of PR #238, after these numbers were taken. niri stayed bounded under the
  same captures.
- **Part of scoot's lower totals comes from doing less work, not doing
  the same work more cheaply.** Nested, scoot draws no pointer of its own
  (the host draws the host's pointer), while niri composites its pointer
  into every frame. While the pointer moved, niri presented 56.5 frames a
  second at 16.2 ms of CPU each, in line with the 14–15 ms its relayout and
  animation frames cost, so that row is niri re-rendering its output rather
  than handling input. During a relayout storm niri presented 59.3 frames a
  second, which is the host's 60 Hz cap: that is about three per action,
  but it may simply be every frame the host offered. scoot presented one
  per action.
- **Screenshots:** through each compositor's own IPC, a scoot capture took
  11 ms of wall time and 9 ms of compositor CPU, against niri's 34 ms and
  44 ms. niri's path does more: it also puts the image on the clipboard and
  tries to show a desktop notification, and it writes a PNG a third the
  size. **Through `grim`, niri was faster** (43 ms against 56 ms), although
  scoot spent less CPU on the capture (4 ms against 30–32 ms). scoot holds
  a `grim` capture until its next frame tick by design, which probably
  explains the gap, but that was not measured.

### What ran

| | |
|---|---|
| Machine | the dev VM (`vm/`): NixOS aarch64 under QEMU on an Apple-silicon Mac, 4 vCPUs, 3.9 GB RAM, kernel 6.18.50, virtio-gpu with no 3D |
| scoot | `main` at `fe41921`, `cargo build --release -p scoot -p scootctl` under the repo's own release profile (fat LTO, `codegen-units = 1`), built on the VM with `CARGO_BUILD_JOBS=1`, peak RSS 1.66 GB. Binary sha256 `e6e30f71…dbc65`, 5,854,232 bytes. |
| niri | 26.04 from nixpkgs (`/nix/store/ww71z668r7kprqxwncl8xhsyjg6sxgr7-niri-26.04`), built by nixpkgs from niri's own release profile (thin LTO, `overflow-checks = true`, line-table debuginfo; see the caveats) |
| Host | cage 0.3.1 (wlroots), headless backend, **pixman** renderer, run with `-d` (see below), one 1600x1000 output at scale 1 and 60 Hz |
| Clients | foot 1.28.0 with an empty config, grim 1.5.0 |
| GLES | Mesa 26.2.2 llvmpipe (LLVM 21.1.8). scoot-gles's log names it. niri's logs from these runs do not: niri's default log filter hides Smithay's "GL Renderer" line. llvmpipe is inferred for niri because the VM has no other GL driver (its EGL probe of virtio-gpu fails, and a later niri run with the renderer logged named llvmpipe). The harness now logs it for niri too. |

The four variants alternated in a rotating order for three rounds (ABCD,
BCDA, CDAB), each in a fresh session:

- **scoot-pixman**: `scoot --nested --width 1600 --height 1000`, built-in
  defaults (no config file).
- **scoot-gles**: the same with `--renderer gles`.
- **niri-off**: niri with
  [`scripts/niri-ab/niri-anim-off.kdl`](../scripts/niri-ab/niri-anim-off.kdl),
  written for this benchmark to match scoot's defaults as far as niri
  allows. It sets 12 px gaps, half-width new columns, a 3 px ring in scoot's
  two colors around every window (niri's `border`; its `focus-ring`, which
  rings only the focused window, is off), no shadows, `prefer-no-csd`, and
  `animations { off; }`.
- **niri-on**: the same file with only the `animations` line removed, so
  niri runs its default animations.

Each session: start, three `foot`s, then six measured scenes, each after
waiting for the compositor to go idle (under 2 ms of CPU in each of two
consecutive half-seconds):

| Scene | What happens |
|---|---|
| idle | 20 s with nothing happening |
| pointer | 1200 absolute pointer motions at 120 Hz for 10 s, alternating between two points in two different windows. They are injected into the **host** by one persistent `zwlr_virtual_pointer_v1` device ([`scripts/niri-ab/vptr`](../scripts/niri-ab/vptr)), so both compositors get the same `wl_pointer` events from the same source. |
| relayout | 200 layout actions at 20 a second, cycling focus left, left, right, right, then move column left and right, through each compositor's own IPC client |
| shot-ipc | 10 captures through each compositor's own screenshot path, 200 ms apart, pointer omitted |
| shot-grim | 10 captures by `grim` against the nested session, 200 ms apart |
| animate | a fourth `foot` prints a line about every 16 ms for 10 s |

A second, separate pass of the same script (`DIAG=1`) ran with the host
logging its protocol traffic. It counts how many frames each compositor
presented in each scene and times the first frame. That log slows the host
down, so none of the CPU numbers come from that pass.

### Results

Every cell is the median of three rounds, with the range across rounds in
brackets. CPU is the compositor process's on-CPU time, recorded two ways.
"cpu ms" sums `/proc/PID/task/*/schedstat` over the threads alive at both
ends of the scene, which gives ns precision but misses any thread that
started and exited inside it. "process ms" is `/proc/PID/stat`'s
utime+stime, the process total, in 10 ms ticks, and it keeps exited
threads' time. The two agree within a tick in every row except niri's own
IPC screenshots, where niri encodes each PNG on a short-lived thread and
"cpu ms" misses 19–23% of the total (marked † below; trust "process ms"
there).
"Wakeups" counts how many times any surviving thread was put on a CPU, so it
undercounts in the same place. None of these figures include the clients,
the IPC client processes or the host. "Frames" are the compositor's own
commits to the host, from the DIAG pass.

#### CPU per scene

| scene | variant | cpu ms | process ms | cpu % of a core | wakeups/s | µs per event | frames/s (DIAG) |
|---|---|---|---|---|---|---|---|
| idle (20 s) | scoot-pixman | 0.0 [0.0–0.0] | 0 | 0.0 | 0.0 [0.0–0.0] | – | 0 |
| | scoot-gles | 0.0 [0.0–0.0] | 0 | 0.0 | 0.0 [0.0–0.0] | – | 0 |
| | niri-off | 6.9 [6.9–6.9] | 10 [10–10] | 0.0 | 3.1 [3.1–3.1] | – | 0 |
| | niri-on | 6.8 [6.6–6.9] | 10 [10–20] | 0.0 | 3.1 [3.1–3.1] | – | 0 |
| pointer (1200 events) | scoot-pixman | 82.8 [82.2–84.3] | 80 [70–90] | 0.8 | 228 [228–228] | 69 [68–70] | 0 |
| | scoot-gles | 93.8 [91.9–96.8] | 100 [100–100] | 0.9 | 229 [228–229] | 78 [77–81] | 0 |
| | niri-off | 9625 [9577–9628] | 9630 [9580–9630] | 91.5 | 1586 [1584–1594] | 8021 [7980–8023] | 56.5 |
| | niri-on | 9664 [9568–9736] | 9660 [9560–9740] | 91.9 | 1589 [1588–1591] | 8053 [7973–8114] | 56.7 |
| relayout (200 actions) | scoot-pixman | 375 [370–379] | 380 [360–380] | 3.7 | 95 [94–95] | 1875 [1850–1895] | 20.0 |
| | scoot-gles | 4119 [4104–4213] | 4110 [4100–4220] | 41.1 | 522 [521–522] | 20595 [20521–21065] | 20.1 |
| | niri-off | 8454 [8407–8496] | 8450 [8400–8490] | 84.2 | 1633 [1615–1640] | 42484 [42248–42694] | 59.3 |
| | niri-on | 8520 [8380–8562] | 8520 [8380–8570] | 85.0 | 1949 [1947–1955] | 43025 [42322–43032] | 59.3 |
| shot-ipc (10) | scoot-pixman | 83.3 [82.1–86.5] | 90 [90–90] | 3.7 | 22 [22–23] | 8332 [8207–8645] | 0 |
| | scoot-gles | 59.8 [58.3–59.9] | 60 [50–60] | 2.7 | 18 [18–18] | 5977 [5835–5987] | 0 |
| | niri-off | 352 [342–360] † | **440 [420–450]** | 17.9 (process) | 133 [130–136] † | **44000** (process) | 0 |
| | niri-on | 349 [347–361] † | **440 [440–450]** | 17.9 (process) | 133 [128–134] † | **44000** (process) | 0 |
| shot-grim (10) | scoot-pixman | 41.5 [40.5–41.9] | 40 [30–40] | 1.6 | 26 [26–26] | 4146 [4050–4193] | 0 |
| | scoot-gles | 47.1 [44.7–48.6] | 50 [40–50] | 1.8 | 26 [26–26] | 4708 [4471–4860] | 0 |
| | niri-off | 314 [311–314] | 320 [310–320] | 12.4 | 112 [105–115] | 31389 [31121–31428] | 0 |
| | niri-on | 299 [296–315] | 300 [290–310] | 11.9 | 110 [108–113] | 29911 [29578–31464] | 0 |
| animate (10 s) | scoot-pixman | 1226 [1215–1263] | 1230 [1210–1270] | 12.2 | 151 [151–151] | – | 49.8 |
| | scoot-gles | 7892 [7753–7893] | 7880 [7750–7890] | 78.8 | 1010 [1002–1021] | – | 43.0 |
| | niri-off | 8020 [7935–8051] | 8020 [7920–8060] | 80.1 | 1688 [1686–1693] | – | 54.1 |
| | niri-on | 8030 [8029–8038] | 8030 [8020–8040] | 80.2 | 1684 [1681–1690] | – | 54.1 |

† **Understated.** niri 26.04 encodes and writes each IPC screenshot on a
thread it spawns for the job, which has exited by the time the scene is
sampled, so "cpu ms" and "wakeups" leave that thread's work out. The
process total is 23–27% higher in all six niri sessions (niri-off
342/352/360 ms by threads against 420/440/450 ms by process; niri-on
361/347/349 against 450/440/440). Use "process ms". Wakeups have no process
total, so the true count is higher by an unknown amount. Every scoot row,
and every other niri row, agrees with its process total within a tick.

#### CPU per presented frame

Main-pass CPU median divided by DIAG-pass frame median. Because the two
come from separate passes, this is a ratio of medians, not a per-round
figure.

| scene | scoot-pixman | scoot-gles | niri-off | niri-on |
|---|---|---|---|---|
| pointer | no frames | no frames | 16.20 ms (594 frames) | 16.21 ms (596) |
| relayout | 1.87 ms (201 frames) | 20.49 ms (201) | 14.16 ms (597) | 14.34 ms (594) |
| animate | 2.46 ms (499 frames) | 18.35 ms (430) | 14.82 ms (541) | 14.81 ms (542) |

#### Memory

`/proc/PID/smaps_rollup`, in MB (kB / 1000; the raw kB values are in the
evidence). Pss is split three ways: anonymous, file-backed, and shared
memory (Pss − anon − file, derived per round). For the scoot-pixman row
that last part is the largest: the clients' `wl_shm` buffers, which the
compositor maps. Medians of three rounds; a range is shown wherever the
three rounds were more than 0.2 MB apart.

| when | variant | Rss | Pss | Pss anon | Pss file | Pss shmem | threads |
|---|---|---|---|---|---|---|---|
| empty session | scoot-pixman | 29.1 | 26.0 | 13.9 | 5.9 | 6.3 | 1 |
| | scoot-gles | 122.5 | 118.9 | 39.9 | 72.7 | 6.3 | 10 |
| | niri-off | 165.5 | 142.9 | 54.7 | 75.7 | 12.5 | 16 |
| | niri-on | 165.5 | 142.9 | 54.7 | 75.7 | 12.5 | 16 |
| three `foot`s | scoot-pixman | 48.8 | 38.6 | 14.0 | 5.8 | 18.8 | 1 |
| | scoot-gles | 148.8 | 140.9 | 52.8 | 72.3 | 15.8 | 10 |
| | niri-off | 185.3 | 155.5 | 63.7 [63.6–63.8] | 74.4 | 17.4 | 16 |
| | niri-on | 185.6 | 155.8 | 64.1 | 74.3 | 17.4 | 16 |
| after 20 screenshots | scoot-pixman | 61.7 | 48.3 | 20.4 | 5.9 | 22.0 | 3 |
| | scoot-gles | **283.6** | 274.0 | **184.2** | 72.4 | 17.4 | 12 |
| | niri-off | 217.9 [205.4–218.0] | 205.9 [193.5–206.1] | 89.6 [77.2–89.8] | 92.6 | 23.7 | 17 |
| | niri-on | 212.9 | 201.0 | 84.7 | 92.6 | 23.7 | 17 |
| end (after animate) | scoot-pixman | 68.0 | 51.4 | 20.4 | 5.8 | 25.2 | 3 |
| | scoot-gles | 289.8 | 277.0 | 184.2 | 72.3 | 20.6 | 12 |
| | niri-off | 208.6 [208.6–214.7] | 193.4 [193.2–199.4] | 74.1 [74.0–80.2] | 92.4 | 26.8 | 17 |
| | niri-on | 219.2 | 203.9 | 84.7 | 92.3 | 26.8 | 17 |

At rest, about 72–76 MB of both scoot-gles's and niri's Pss is
file-backed Mesa and LLVM. Compare those two when you want a like-for-like
GL stack; pixman against niri is the comparison for a machine without a
GPU. The scoot-gles jump after screenshots is the leak described above. A
probe on its own shows it linear and unbounded: 6,250 kB (one 1600x1000
frame) per capture, reaching 885 MB after 120 captures of a still screen.
Drawing frames afterwards did not bring RSS back down (compare the "end"
row), though it did stop further captures from growing it.

**These scoot-gles RSS numbers predate the fix** (PR #238, after `fe41921`).
With it, 120 captures of a still screen leave scoot-gles flat after at most
one frame, and the rows after a screenshot scene would no longer carry the
jump. The [resolved record](backlog/resolved/gles-capture-leaks-a-frame-per-shot-done.md)
has the before/after numbers. It also explains the release behaviour: a
drawn frame did free the queued buffers, and glibc kept its high-water
mark. The rest of this table, and every CPU number, is unaffected.

#### Startup

Milliseconds from `exec` of the compositor.

| variant | IPC answering | first frame committed to the host (DIAG) |
|---|---|---|
| scoot-pixman | 15.6 [6.7–17.4] | 27.1 [24.8–27.2] |
| scoot-gles | 41.7 [41.1–47.2] | 62.8 [62.4–63.2] |
| niri-off | 112.7 [111.8–114.6] | 112.9 [111.5–114.7] |
| niri-on | 114.6 [113.7–123.9] | 111.6 [109.7–112.8] |

#### Screenshot latency

Wall milliseconds per capture, across all 30 captures of each kind, from
the start of the client command to a complete PNG on disk.

| variant | own IPC | PNG bytes | grim | PNG bytes |
|---|---|---|---|---|
| scoot-pixman | 11.3 [7.0–13.0] | 40,537 | 55.9 [50.1–58.0] | 10,240 |
| scoot-gles | 9.4 [8.7–10.4] | 40,537 | 56.1 [48.2–59.6] | 10,239 |
| niri-off | 34.3 [28.7–38.2] | 11,774 | 43.0 [35.4–44.6] | 10,225 |
| niri-on | 33.9 [29.8–38.1] | 11,775 | 42.6 [31.4–44.7] | 10,225 |

#### Binaries

| | scoot | niri |
|---|---|---|
| size | 5.85 MB (stripped by the release profile) | 35.4 MB as shipped by nixpkgs (not stripped); 23.4 MB stripped |
| shared libraries (`ldd`) | 22 | 59 |

scoot's list is libinput, libseat, libudev, libxkbcommon, pixman and their
dependencies. libEGL and libGLESv2 are loaded at runtime only under
`--renderer gles`, so `ldd` does not show them; the Pss-file column above
does. niri also links EGL, GBM, PipeWire, pango/cairo, fontconfig/freetype
and the X11 client libraries. Its runtime closure in the Nix store is 738 MB.

### What these numbers do not show

- **This is a VM and llvmpipe.** Every GLES number, niri's and
  scoot-gles's alike, is software rasterisation on 4 vCPUs. On a real GPU,
  per-frame GLES costs change completely. That half is Asahi.md Test 9.
- **Nested, not `--tty`.** Neither compositor's KMS path, cursor plane or
  vblank pacing is exercised. Frames are paced by the host's 60 Hz frame
  callbacks.
- **The pointer row compares different work.** Nested, niri composites its
  own pointer into every frame (56.5 frames a second while it moves). scoot
  `--nested` draws no pointer at all; the host shows its own. Each of those
  niri frames cost 16.2 ms of CPU, in line with the 14–15 ms of its relayout
  and animation frames, so 9.6 s against 83 ms is niri re-rendering its
  output 57 times a second against scoot only dispatching input. It does not
  mean niri's input handling costs 100x. On a real session both draw a
  pointer.
- **Relayout goes through two different IPC clients**, `scootctl action
  focus-column left` against `niri msg action focus-column-left`. Their own
  CPU is not counted, but they differ: in a separate probe, one `niri msg
  action` invocation took 16.3 ms end to end (about one frame) against
  0.5 ms for `scootctl action`. Both kept the 20-a-second
  pace (niri 198–199 of 200, scoot 200). niri presented 59.3 frames a second
  during the storm, which is the host's 60 Hz cap: that is about three per
  action, but it may just be niri drawing every frame the host offered while
  anything was changing. scoot presented one per action. The per-action
  column carries that difference. The per-frame table normalises for it,
  but a cap-bound frame count is not the same thing as a count of the frames
  the workload needed.
- **The screenshot paths are not the same work.** niri's also puts the
  image on the clipboard and tries to show a desktop notification over
  D-Bus. There is no notification daemon on the VM, so each attempt failed
  (`ServiceUnknown` in niri's log), but it still cost time. niri also
  answers before the file is written, and compresses harder: its PNG is
  11.8 kB against scoot's 40.5 kB. niri's latency is therefore timed to a
  complete PNG on disk through a shell polling loop (`tail`, `grep` and
  `sleep 0.001` per check). That loop is coarser than the 1 ms sleep
  suggests, and it adds to niri's numbers. scoot's are timed to
  `scootctl`'s exit, after it has written the file. For `grim`, scoot speaks
  ext-image-copy-capture and niri wlr-screencopy. scoot deliberately holds
  each capture until its next frame tick (see the module doc of
  `crates/scoot/src/compositor/screencopy.rs`). That is the likely cause of
  its extra `grim` latency, but it was not measured.
- **The two builds use different release profiles.** scoot's is fat LTO
  with `codegen-units = 1`. niri's (its `Cargo.toml` at v26.04) is thin LTO
  with `overflow-checks = true` and line-table debuginfo, which trades some
  speed for safety and backtraces. Some of niri's per-frame CPU may be that
  profile rather than its design; this benchmark cannot separate the two.
- **The configs are as close as niri allows, not identical.** niri's ring
  is its `border`, which is a different implementation from scoot's ring:
  niri's takes its width out of the layout, while scoot draws its ring in the
  gap. Window sizes therefore differ by a few pixels. niri also runs
  machinery that scoot lacks, such as its config-file watcher (see the
  artifact below).
- **The `gpu-scanout` build was not measured separately, because nested it
  changes nothing here.** Its dma-buf presenter needs the host's
  `zwp_linux_dmabuf_v1` at version 4 (`crates/scoot/src/compositor/nested/gpu.rs`,
  `try_negotiate`). cage on pixman offers no `linux-dmabuf` (its globals are
  in the evidence), so that build logs `presenting to the host by read-back`
  and presents exactly as the default build does. Asahi.md Test 9 Part A
  gives the host GLES so that the path comes up.
- **Input-to-present latency was not measured.** Nested, there is no signal
  both compositors expose for "this input reached the screen". scoot draws
  no nested pointer, and the host's presentation feedback times the host's
  frames rather than the nested compositor's. It remains an open question
  for real hardware.
- **scoot presented fewer frames than niri for the animating client** (49.8
  a second against 54.1). The client prints about one line per 16 ms.
  scoot-pixman is far from CPU-bound there (12% of a core), so the cause is
  pacing rather than cost. Since resolved as a measured don't-build
  ([record](backlog/resolved/nested-frame-rate-vs-client-done.md)):
  scoot paces `--nested` off its own 16 ms timer rather than host frame
  callbacks, and the done-gated client draws fewer frames on the longer
  loop — loop-paced, nothing cheap and safe to fix.
- **A harness artifact was caught and fixed.** A first full run had niri-off
  reading its config from the VM's 9p mount of this checkout. niri watches
  its config file, and on 9p that cost it about 55 extra wakeups a second at
  idle (1153 against 62 over 20 s in a direct probe). niri-on already read
  a local copy. Both configs are now copied to local disk, and every number
  above comes from the re-run. The first run is kept in the evidence, marked
  superseded.

### Reproducing

On a machine with `cage`, `wlr-randr`, `foot` and `grim` on `PATH`:

```sh
cargo build --release -p scoot -p scootctl
cargo build --release --manifest-path scripts/niri-ab/vptr/Cargo.toml --target-dir /tmp/vptr
NIRI=$(nix build --no-link --print-out-paths nixpkgs#niri)/bin/niri
common="SCOOT=$PWD/target/release/scoot SCOOTCTL=$PWD/target/release/scootctl NIRI=$NIRI VPTR=/tmp/vptr/release/nab-vptr"
env $common OUT=/tmp/niri-ab scripts/niri-ab-bench.sh
env $common OUT=/tmp/niri-ab-diag DIAG=1 scripts/niri-ab-bench.sh
scripts/niri-ab/summarize.sh /tmp/niri-ab /tmp/niri-ab-diag
```

About 16 minutes per pass at the defaults. Keep `OUT` on a local disk. The
script copies niri's config there for the reason given in the last caveat
above.

The evidence for the run above is on the dev VM under `~/evidence/niri-ab/`:
- `run-main/` and `run-diag/`: every raw TSV, and each session's compositor
  and host logs (the DIAG host logs gzipped);
- `summary.md`: the tables above, before rounding;
- `versions.tsv` in each run directory;
- `scripts-used/`: the exact scripts, with their sha256.
  `bench-script-after-runs.diff` holds the changes made to the benchmark
  script up to the PR's first review: a per-session watchdog, `timeout` on
  the calls that spawn clients and count windows, and each session
  directory recreated from scratch (a re-run into the same `OUT` used to
  read the previous run's pid). The review then added one functional change,
  visible in git history: niri now runs with
  `RUST_LOG=niri=debug,smithay::backend::renderer::gles=info` (niri's own
  default filter plus Smithay's renderer at info), so that its log names its
  GL renderer. That adds a handful of startup lines. Neither the measured
  calls nor the startup poll changed. The pointer helper has changed
  only by a clippy fix since (`events % 2 == 0` became
  `events.is_multiple_of(2)`);
- `static-facts.txt`: sizes and `ldd`;
- `cage-host-wayland-info.txt`: the host's globals;
- the `probe-*.txt` files behind the leak, the IPC client costs and the 9p
  artifact;
- `verify-sample-sh.txt`: `sample.sh`'s `session` mode run from inside
  both nested compositors. It was produced by a version of `sample.sh`
  between the one in `scripts-used/` (older) and the committed one, and
  that exact version was not recorded. `verify-sample-sh-proc-cpu.txt` is
  the committed version's `proc_cpu_ms` column (the process total) against
  niri's screenshot thread: 140 ms against 88 ms by surviving threads;
- `superseded-9p-config/`: the discarded first run.
