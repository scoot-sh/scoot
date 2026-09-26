---
title: "Choosing dependencies (and checking licences)"
status: "research"
area: "scootbg"
priority: "research"
blocked: null
---

# Choosing dependencies (and checking licences)

Decide before the first code PR, and record the choice and why in the
crate's `Cargo.toml` comments. The tiebreaker is always
[weight](lightest.md): binary size, idle memory and compile time, measured.

- **Wayland client:** `smithay-client-toolkit` (MIT) versus plain
  `wayland-client` plus `wayland-protocols(-wlr)`. SCTK saves the output
  and layer-shell boilerplate; plain is smaller. Measure binary size and
  compile time.
- **Decoding:** the `image` crate (MIT/Apache-2.0) with only the needed
  format features, versus `zune-*` decoders directly.
- **Scaling:** `fast_image_resize` (MIT/Apache-2.0, SIMD) versus `image`'s
  own resize. Benchmark a 6000×4000 JPEG to 3840×2160.
- **CLI:** match whatever `scootctl` already uses, unless a lighter parser
  saves real bytes in a binary this small.
- **No async runtime.** A `poll` loop is enough for one Wayland fd and one
  socket.

Every licence must be MIT-compatible. Nothing from awww/swww (GPL-3.0),
`wpaperd` or other GPL daemons.
