---
title: "`Cursor::element`'s fallback path allocates a one-element `Vec` every rendered frame (LOW)."
status: "open"
area: "rendering"
priority: "low"
blocked: null
---

# `Cursor::element`'s fallback path allocates a one-element `Vec` every rendered frame (LOW).

`Cursor::element`'s fallback path allocates a one-element `Vec` every
rendered frame (LOW). The old code returned `Option<...>` with no
allocation; now every `--tty` frame showing the default cursor (the
common case) allocates and frees a `Vec` to hold it. Not observable in
benchmarking (12+4 interleaved reps, no measurable difference), and
`render()` already builds a few per-frame `Vec`s this way, so it matches
local convention rather than breaking it — but a cheaper shape exists if
it ever matters: have `element()` append into a caller-owned
`&mut Vec<CursorElement<R>>` instead of returning a fresh one; a full fix
(a persistent element buffer on `Backend`) is a larger refactor than fits
here. The `Surface` path allocates regardless of this fix, since
Smithay's own `render_elements_from_surface_tree` returns a `Vec`.
