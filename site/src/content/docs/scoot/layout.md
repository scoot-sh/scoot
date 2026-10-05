---
title: Layout
description: "Gaps, column widths, and the default new-column width."
---

Widen the gaps, narrow the columns. `[layout]` is three fields, all
re-applied live by `scoot msg reload`:

| Field | Type | Default | Reload | Meaning |
|---|---|---|---|---|
| `gap` | integer (pixels) | `12` | live | Gap between columns, between windows stacked in a column, and at output edges. Clamped into `0..=10000`: negatives become `0`, anything above becomes `10000` — already wider than an 8K display's long edge, a guard against a typo, not a usable setting. A gap that large leaves no usable area, so windows end up 1x1. |
| `column_widths` | array of floats | `[1/3, 1/2, 2/3]` | live | Column widths as fractions of the output width, in the order `cycle-column-width` steps through and `set-column-width N` indexes into (0-based). Non-finite or non-positive entries are dropped; an empty list falls back to the built-in three. A shorter list clamps live columns onto the nearest surviving entry. |
| `default_column_width` | integer (unsigned) | `1` | live (new windows) | Index into `column_widths` for newly created columns (`1` selects `0.5`, half the output). Too large clamps to the last valid index; negative is a whole-file parse error, not a clamp. Live columns hold still. |

```toml
[layout]
gap = 8
column_widths = [0.25, 0.5, 0.75, 1.0]
default_column_width = 1
```

> **Symptom:** a gap wider than the screen, windows 1x1.
> You typed a huge `gap` (or a probe did). It clamps at 10000, which
> still leaves no usable area — set it back under a few dozen.
