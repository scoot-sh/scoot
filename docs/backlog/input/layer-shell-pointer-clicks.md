---
title: "[nested] pointer clicks never reach layer-shell surfaces; toplevels fine"
status: "open"
area: "input"
priority: "high"
blocked: null
---

# [nested] pointer clicks never reach layer-shell surfaces; toplevels fine

Filed as gh issue #182 (live field report, Selkies webtop deployment —
read the issue for the full repro, deployment, and the reporter's
code-level suspicion; this entry tracks it, it does not replace it).

scoot `4f8707c` `--nested` (pixman) in pixelflux's compositor, single
output 430x744, noctalia-shell 4.7.7 bar (30px exclusive zone) + launcher
overlay. `scoot msg pointer click 30 15` on the bar's launcher icon:
nothing (screenshot-verified), repeated at other bar points and right
button — all no-ops. Toplevel clicks work (ghostty CSD responds).
Spawning the launcher over IPC works, so the shell client is healthy —
only pointer delivery to layers is dead.

Reporter's trace (verify, don't assume): `Request::Click` is
move-then-press-release (`ipc.rs`), so focus should establish before the
button; `surface_under` checks `ABOVE_WINDOWS` layers first
(`state.rs`); `pointer_button` delivers to `pointer.current_focus`
(`input.rs`); button codes map (`nested_dispatch.rs`), same path works
for toplevels. Suspect: the layer branch of hit test / focus handoff
(`layer_shell.rs` `layer_hit` and surroundings) — motion over the bar
never establishes layer focus, so the button lands wherever focus was.

Notes for whoever takes it: reproduce over the control socket (no client
needed for the probe — `msg pointer click` + screenshot); companion
issue #183 (same session, bar content missing with no toplevels) smells
related (layer state keyed off window existence) but was kept separate
deliberately — confirm or refute the shared root rather than assuming.
Fail-first with a harness layer client + injected click; live proof on
`--nested` (and `--headless` if layers exist there).
