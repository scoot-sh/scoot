---
title: "scoot --help/JSON config section list omits [virtual_input]"
status: "open"
area: "core"
priority: "low"
blocked: null
---

# scoot --help/JSON config section list omits [virtual_input]

Filed 2026-10-14. Serves **daily-drive** (help is the first thing a user or agent reads; an incomplete section list hides a real restart-only gate).

## The gap

`crates/scoot/src/cli.rs:45-46` (`CONFIG_BODY`) and `cli.rs:355-368`
(`config_sections` JSON) list eleven sections and omit `[virtual_input]`,
but the compositor implements `[virtual_input] enabled|binds`
(`crates/scoot/src/compositor/config.rs:326-336,471,543-556,658-667`)
and emits it in `--print-default-config`. Found during the docs1 slice
review (docs listed twelve tables; help lists eleven). Evidence:
`CONFIG_BODY` has no `virtual_input`; `config_sections` JSON has no
`"[virtual_input]"`.

## What to do

Add `[virtual_input]` to both lists (text + JSON render from the same
table per `docs/cli-help.md`, so one edit plus the text/JSON diff test).
Pin with the existing help-contract test. Edge case: none, display-only.

## Not in this ticket

Docs-side wording (fixed in `docs/scoot-slice-cleanup`); no behavior change.
