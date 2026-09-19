---
title: "`smoke-test.sh` defaults to a shared target dir, which has now handed three agents someone else's binary"
status: "open"
area: "testing"
priority: "medium"
blocked: null
---

# `smoke-test.sh` defaults to a shared target dir, which has now handed three agents someone else's binary

`scripts/smoke-test.sh:33`:

```sh
SCOOT=${SCOOT:-/var/cargo-target/debug/scoot}
```

`/var/cargo-target` is the dev VM's *shared* target directory. When two
agents work concurrently — which is now normal — whichever built last owns
that path, so the script silently tests a binary from a different branch.

**This happened three times in one session, to three different agents**,
and each caught it a different way: one by a live check showing one output
where two were expected, one by `strings … | grep -c "ignoring a host
configure"` returning 0, one by a reviewer's smoke run passing against the
wrong tree. Two of those were near-misses that would have produced a
confident wrong verdict — a green smoke run against code you did not build
is worse than a red one.

The failure is silent by construction: the path exists, the binary runs, and
nothing in the output says which tree it came from.

## Shape

Options, roughly in order of preference:

- **Default to the invoking tree** (`$CARGO_TARGET_DIR/debug/scoot`, else
  `./target/debug/scoot`) rather than an absolute machine-specific path.
- **Print the binary's path and mtime** in the header the script already
  emits, so a wrong one is visible in the log rather than inferred later.
- At minimum, **fail loudly if `SCOOT` is unset and the default does not
  exist**, instead of proceeding.

Cheap, and it removes a whole class of wrong verdict rather than one
instance of it.
