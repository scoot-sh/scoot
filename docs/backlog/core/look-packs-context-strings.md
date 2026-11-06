---
title: "Later: accept context-bearing path strings in programs.scoot.looks"
status: "open"
area: "core"
priority: "low"
blocked: null
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
