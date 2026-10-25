# Lock screen blur: why pre-blurred, not screenshots

2026-10-24. The default lock shows the look's wallpaper blurred. The
blur is baked at build time (ImageMagick over the shipped wallpaper,
`nix/modules/idle-home.nix` `lockWallpaper`) and the locker displays
it with `-i`, with an unlock banner composed over it
(`--effect-compose`). This note records why the seemingly obvious
alternative -- a live screenshot blurred at lock time
(`swaylock-effects --screenshots --effect-blur`) -- cannot work here,
so nobody re-derives it.

scoot deliberately publishes no `wlr-screencopy-unstable-v1` (see
`crates/scoot/src/compositor/screencopy.rs`: it speaks
`ext-image-copy-capture` instead). The effects fork captures through
the wlr manager only; against scoot the capture fails and the fork
silently falls back to its `color` -- which is white by default. A
screenshots-based lock would therefore show a flat frame under scoot
on every machine while appearing to work under compositors that do
publish wlr-screencopy. The pre-blurred image behaves identically
everywhere, costs nothing at lock time (the lock comes up at once on
the CPU renderer too), and degrades to the configured `color` when the
image cannot load -- never failing open.

Trade-off, tracked not hidden: the built blur is per generation, so a
wallpaper applied live (`scootbg set`, `scoot look apply`) keeps the
built blur until the next rebuild, and per-output wallpapers share the
one image. See docs/backlog/core/lock-screen-per-output-blur.md for
the runtime follow-up (scootbg maintaining a blurred copy per output).
