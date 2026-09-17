---
title: "A `[binds]` entry naming a capital letter parses, loads, and can never fire (LOW, pre-existing). — RESOLVED"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# A `[binds]` entry naming a capital letter parses, loads, and can never fire (LOW, pre-existing). — RESOLVED

## What it said

A `[binds]` entry naming a capital letter parses, loads, and can never
fire (LOW, pre-existing). `"A" = "close"` is accepted without a warning,
but `input.rs`'s `keysym_named` tries the name *exactly* first, which for a
single letter always succeeds and yields the distinct `A` keysym — while
`keybindings::match_key` is fed `handle.raw_syms().first()`, the key's
unshifted symbol, which is `a`. The two never meet. Fixing it properly
means lowercasing a single-letter key name at *config parse* time only —
deliberately not in `keysym_named` itself, since `flexwm msg key A` must
keep refusing rather than silently becoming `a` (see the resolved entry
above) — plus a test per direction.

## Resolution

**The fold already existed, broader than the ticket asked — and the ticket
is what narrowed it.** `parse_combo` (`config.rs`) has folded the key name
with `to_ascii_lowercase()` since the config file was introduced, so a lone
`"A"` already bound `a` on `main`, while the README still documented the
bind as dead. What landed here: the fold scoped to **single ASCII letters
only**, a warning when it fires, boundary tests, the README correction,
and a live end-to-end proof.

**Scope: single ASCII letters, decided explicitly.** The old whole-name
fold was observably wrong beyond single letters: multi-character names can
be cased pairs of *distinct* keysyms, and folding silently redirects them —
measured, `parse_combo("OE")` resolved to `XK_oe` (œ) instead of `XK_OE`
(Œ) before this change. Now only a one-byte ASCII alphabetic name folds;
`"F1"`, `"Return"`/`"RETURN"` resolve exactly as written (through
`keysym_named`'s existing case-insensitive fallback, pinned by the
pre-existing multi-character test), digits/symbols pass through untouched,
and non-ASCII passes through unfolded — keysym case semantics outside ASCII
are murky (e.g. Turkish dotted/dotless I), so an unknown non-ASCII name
stays an unknown key, skipped with `apply_binds`' usual warning, rather
than a folded guess at another keysym. Narrowing to single-letter-only
rather than keeping the shipped whole-name fold is the ticket's specified
fix, and the `OE` measurement makes it a correctness call, not taste.

**Warn, not silent, with reasoning.** After the fold `"A"` *does*
something — the unshifted `a` key, emphatically not `shift+a` — and a bare
`a` bind intercepts every unmodified press of that key, so silently
reinterpreting a capital is its own surprise (the ticket's original
complaint was "accepted without a warning", then doing nothing; doing the
possibly-unmeant thing deserves the same loudness). `parse_combo` now
`WARN`s naming the bind and the `shift+x` spelling, following the module's
existing pattern (`into_scale` warns when it clamps; `apply_binds` warns
per skipped bind). A lone lowercase letter warns about nothing: nothing was
changed. The pre-existing silent-fold precedent (a lone `"Super+H"`)
argued the other way and lost on the bare-letter impact above.

**Review fix (pre-merge): the warn was factually wrong for
modifier-qualified capitals.** `"shift+A"` folded and warned "this bind
means plain `a`, not `shift+a`" — but that bind *does* mean `shift+a`, so
a fully-correct config was scolded at every startup. The warn is now gated
on ambiguity: only a changed letter with no Shift named warns
(`fold_letter` returns the decision alongside the name, pinned by
`a_capital_with_shift_named_binds_shift_quietly`, which also asserts
`"shift+A"` fires on shift+a). With Shift named the chord is shift+a
either way; lone lowercase, digits, multi-character and non-ASCII names
resolve quietly as before.

**`keysym_named` untouched — `msg key A` still refuses.** The refusal is
the contract the dead-keys entry's "Adjacent" section records (review
measured `key exclam` typing `1` as the same bug and fixed it by refusing):
`press` holds only the modifiers its caller names, so a keysym above the
unmodified level has no honest answer. Pinned by a dedicated
`msg_key_capital_letter_still_refused` test alongside the parametrized
refusal test that already covered `"A"`.

**Overlap checked: `msg-key-modifier-resolution.md` does not overlap.**
That entry is about `resolve_combo`'s hard-coded `_L` modifier keysyms on
exotic layouts; this change touches only the config-parse path
(`parse_combo`), not `resolve_combo`, `press`, or `type_text`. Explicitly
deferred with this pointer; that entry stays open.

**Tests** (`config.rs`, all fail-first where behavior is concerned):
`a_lone_capital_letter_bind_fires_on_the_unshifted_key` (fails with the
fold neutered to exact lookup: `match_key` returns `None`, the ticket's
never-fires symptom, reproduced); `only_a_single_ascii_letter_is_case_folded`
(`"A"`→`a`, `"a"`/`"Z"`, `"1"` untouched, `"F1"`/`"Return"`/`"RETURN"`
stable, `"OE"` stays Œ — this one fails on the pre-change whole-name fold
with `XK_oe`, the overreach measurement above — `"É"` stays an unknown
key); `shift_plus_lowercase_still_requires_shift` (unshifted `a` doesn't
fire it, shifted does); `msg_key_capital_letter_still_refused` (input
tests). Pre-existing collision/multi-character tests confirm no
regression. No hot path touched (parse time, once per startup), so no
benchmark — stated, not skipped.

**Verified live** (dev VM, `--headless`, uncommitted tree as of this PR's
head): config `"A" = "close"` + injected bare `a` closed the window
(`msg windows` length 1 → 0); `msg key A` refused with exit 1 and the
unmodified-level message; the startup log carries the new warning naming
`bind="A"`. `scripts/smoke-test.sh` gains a permanent
`run_capital_bind_test` section (warn present in the log + `msg key a`
closes the window) and passes end to end.
