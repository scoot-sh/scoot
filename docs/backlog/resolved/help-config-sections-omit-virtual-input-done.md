---
title: "scoot --help/JSON config section list omits [virtual_input]"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-10-28"
---

# scoot --help/JSON config section list omits [virtual_input]

Filed 2026-10-14. Serves **daily-drive** (help is the first thing a user or agent reads; an incomplete section list hides a real restart-only gate).

## The gap

`crates/scoot/src/cli.rs` (`CONFIG_BODY`) and `config_sections` JSON
list eleven sections and omit `[virtual_input]`, but the compositor
implements `[virtual_input] enabled|binds`
(`crates/scoot/src/compositor/config.rs`) and emits it in
`--print-default-config` (docs list twelve tables; help lists eleven).
Evidence: `CONFIG_BODY` has no `virtual_input`; `config_sections` JSON
has no `"[virtual_input]"`.

## What to do

Add `[virtual_input]` to both lists (text + JSON render from the same
table per `docs/cli-help.md`, so one edit plus the text/JSON diff test).
Pin with the existing help-contract test. Edge case: none, display-only.

## Resolution (2026-10-28)

`CONFIG_SECTIONS` in `crates/scoot/src/cli.rs` is now the one table the
`scoot help config` text and the JSON `config_sections` render from. It lists
fifteen entries: `[virtual_input]` was missing, and so was `[animation]`.
The ticket's "twelve tables" was the default-config emission (eleven
tables, no `[[outputs]]`, `[wallpaper]`, `[global_shortcuts]` or
`[[window_rule]]`); the file reads fifteen spellings (fourteen tables plus
`[[window_rule]]`, matching the site's "Fourteen optional tables"). The
help text also gained one sentence naming the restart-only keys
(`[tty] gpu`, `[renderer] backend`, `[xwayland] enabled`,
`[virtual_input] enabled|binds`). Pinned by tests that compare the table
to `FileConfig`'s fields and to the tables `--print-default-config` emits.
