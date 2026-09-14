---
title: "Output management (`wlr-output-management` or successor) for shell display/settings pages."
status: "open"
area: "protocols"
priority: "medium"
blocked: null
---

# Output management (`wlr-output-management` or successor) for shell display/settings pages.

Both shell probes hit this (DMS gap 4, Noctalia gap 4, 2026-09-14): the
shell's daemon initializes its output-management client and then finds
nothing to bind — DMS logs `Received empty outputs list`; Noctalia's
Settings → Display page renders with nothing behind it. `wl_output`
itself is well-formed, so this is specifically the management path:
mode/position/scale query and reconfiguration à la `wlr-randr`/`kanshi`.

Previously filed under
`docs/backlog/protocols/protocol-gaps-niche.md` as "moot while flexwm
has exactly one `Output` and no real multi-monitor support" — still
true for *reconfiguration*, but the probes show the *query* half now
has real clients (shell display pages), so a read-only advertisement
may be worth scheduling ahead of full multi-output support.

Per the standing rule, check for an `ext-` successor before reaching
for `wlr-output-management-unstable-v1`. Rough size: M–L (new protocol
surface; read-only advertisement first, reconfiguration later if ever
while single-output).
