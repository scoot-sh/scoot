---
title: "A `[binds]` entry naming a capital letter parses, loads, and can never fire (LOW, pre-existing)."
status: "open"
area: "config"
priority: "low"
blocked: null
---

# A `[binds]` entry naming a capital letter parses, loads, and can never fire (LOW, pre-existing).

A `[binds]` entry naming a capital letter parses, loads, and can never
fire (LOW, pre-existing). `"A" = "close"` is accepted without a
warning, but `input.rs`'s `keysym_named` tries the name *exactly* first,
which for a single letter always succeeds and yields the distinct `A`
keysym — while `keybindings::match_key` is fed `handle.raw_syms().first()`,
the key's unshifted symbol, which is `a`. The two never meet. Found while
correcting this branch's documentation (the README claimed letters
"always resolve to their lowercase keysym", which is only true when the
exact lookup *fails*); reproduced on `--headless` in
`/home/dev/bindcase.sh` on the dev VM: with `"A" = "close"` the window
survives `flexwm msg key shift+a`, with `"shift+a" = "close"` it closes.
The README now says to write `"shift+a"`. Fixing it properly means
lowercasing a single-letter key name at *config parse* time only —
deliberately not in `keysym_named` itself, since `flexwm msg key A` must
keep refusing rather than silently becoming `a` (see the resolved entry
above) — plus a test per direction. Worth doing next time `config.rs` is
open; nothing silently misbehaves in the meantime, the bind simply does
nothing.
