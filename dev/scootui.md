# scootui: the shared shell-app crate

`crates/scootui` is the library the shell apps share: pure drawing plus
the Wayland client scaffolding, behind Cargo features so a consumer pays
only for what it links. It holds no app logic: each app owns its state
machine and calls into scootui to draw, read input and serve its socket.

## Features and who enables what

| Feature | What it is | Enabled by |
|---|---|---|
| (always) | `paint` (canvas, spans), `color`, `theme`, `text`, `font`, `ellipsis` | everyone |
| `widgets` | The popup's pure half (layout, interact, paint, hover, wrap) | scootbar (`popup` feature) |
| `icons` | Path and cached icons; icon-theme lookup with `index.theme` inheritance | scootbar, scootnotify, scootlaunch |
| `png` | PNG image icons, decoded once and cached | scootnotify, scootlaunch, scootbar (`tray`, `icon-image`) |
| `tray` | Tray pixmaps through the shared resampler | scootbar (`tray`) |
| `entry` | Text entry (`entry/line.rs`), virtualized list and inline selector (`entry/list.rs`), focus chain (`entry/focus.rs`), and their paint (`Geometry`, `paint_entry`, `paint_list`, `paint_selector`). Pure state plus paint into the caller's canvas: no Wayland, no I/O. | scootlaunch |
| `surfaces` | On-demand surfaces: one overlay layer surface created when there is content and destroyed with its buffers unmapped when empty, plus a full-screen `xdg_toplevel` kind for hosts with no layer-shell. The pure lifecycle plus the Wayland glue and the named-memfd buffer pool. | scootnotify (stack), scootlaunch (panel) |
| `wayland` | The poll loop's Wayland primitives (flush, the gone check, the read after a poll) | scootbar, scootnotify; implied by `surfaces`, `keyboard` |
| `control` | The control socket's server half: accept, bounds (`MAX_CONNECTIONS`, `MAX_SUBSCRIBERS`, the write-stall deadline), line framing, socket paths. The socket name is the app's parameter. | scootbar, scootnotify |
| `keyboard` | The seat keyboard: keymap fd with a size cap, modifiers, key repeat on one timerfd, keysyms and UTF-8 into a caller-supplied buffer, Compose. Links libxkbcommon (see `dev/scootui-keyboard-link.md`). | scootlaunch |
| `test-support` | The headless-compositor harness the integration tests share (scoot and sway sessions, screenshot readers). Test only, never in a shipped binary. | all three apps' test targets |

SVG rasterization is not a scootui feature: scootlaunch owns it
(`resvg` with no default features, behind its own `svg` feature).

## Adding a consumer

Depend with `default-features = false` and name only the features the
app uses (the defaults are `widgets, icons`, which a non-bar app does
not want). A feature an app does not enable must not change its binary:
scootbar's `ldd` gate and the size budgets in the app docs enforce this.
New shared drawing goes behind a new feature, not into the always-on
set. Tests use `test-support` plus the headless `scoot` binary beside
the test binary (`SCOOTLAUNCH_REQUIRE_SCOOT=1` turns the skip into a
failure); screenshot fixtures that the docs reproduce live in the app's
test harness, generated in code, never checked in as binaries.
