---
title: "Later: accept context-bearing path strings in programs.scoot.looks"
status: "resolved"
area: "resolved"
priority: null
blocked: null
resolved: "2026-11-07"
---

# Later: accept context-bearing path strings in programs.scoot.looks

Filed 2026-11-06, found in review of the
[`look-packs-flake`](../resolved/look-packs-flake-done.md) PR. Serves
**daily-drive**: `"${inputs.foo}/looks"` is the first thing a Nix user
writes.

## The gap

`asDirPath` in `nix/look-catalog.nix` turns an absolute string into a path
with `/. + item`. A string carrying store-path context (any
`"${inputs.foo}/looks/a"` or `inputs.foo + "/looks/a"`) fails there with
"a string that refers to a store path cannot be appended to a path". The
same error already hit single look entries (`toEntry`) before packs, so
packs inherit it. Only context-free strings (`toString ./dir`) work.

Repro: `programs.scoot.looks = [ "${inputs.foo}/looks/a" ];` fails
evaluation with that message.

## What to do

Possible fix: `/. + builtins.unsafeDiscardStringContext item` in both
`asDirPath` and `toEntry`. The risk is the dropped context: the source
then is not a build input, which is fine here only because the catalog
reads it at evaluation and copies it by path; check `catalogDir` and the
render derivations still depend on the source. Pin it with a test using a
string with context (`"${pkgs.hello}/..."`-style fixture). Until then the
docs say entries are path values or `inputs.<name>.lookPacks.<pack>`.

## Not in this ticket

Widening the type beyond strings and paths.

## Resolution (2026-11-07)

Not `unsafeDiscardStringContext`. `asDirPath` keeps a string that carries
store-path context as that string (context-free absolute strings still
become `/. + item`), and `toEntry` now goes through `asDirPath`. The
context is what makes the source a build input; discarding it would turn
an unbuilt derivation output into a pure-evaluation "access to absolute
path forbidden" error and drop the source from the derivations' inputs.
`readFile`, `readDir` and `pathExists` read such a string directly, and a
derivation output that is not built yet is built during evaluation
(import from derivation), which is the cost of that case. Pinned in
`nix/tests.nix` (`ctxPack`), documented in the Look packs section of the
looks page.
