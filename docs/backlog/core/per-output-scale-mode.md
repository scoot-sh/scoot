---
title: "Per-output scale/mode configuration surface (after milestone 19)"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# Per-output scale/mode configuration surface (after milestone 19)

Left open deliberately through milestone 19 phases A–F (see
`../roadmap/19-multi-output.md`, "Explicitly not in this milestone"):
every output beyond the first inherits the session's single `--width` /
`--height` / `[output] scale`, and `wlr-output-management` `apply`/`test`
stay refused as documented. This entry tracks what a real surface would
need, once there is hardware to test it against — do not build it blind.

## Questions to settle first

1. **Where does per-output geometry live?** `--width`/`--height`/`--mode`
   today describe one output. Options: indexed config sections
   (`[output.2]`?), `--outputs` as a list, or runtime-only via a
   (still-refused) `apply` path. The config-is-state principle
   (`startup-programs-and-autostart.md`) constrains the shape.
2. **Scale per output?** `[output] scale` is session-wide and clients
   learn it at bind time (see `config-reload-done.md` for why it doesn't
   reload). Per-output scale multiplies the bind-time problem by N —
   needs the bind path re-examined, not just a new key.
3. **`apply`/`test`?** Refusal stands as documented
   (`output-management-reconfiguration-done.md`) until a configuration
   exists that could ask for something real. This surface is what would
   make it real — the two land together or not at all.

## Ground rules (from the milestone)

- No building without two-connector hardware to verify against (the same
  gate as phase E). `--headless --outputs N` can carry the harness shape
  but not the modesetting truth.
- Single-output behavior stays byte-identical; additive or nothing.
