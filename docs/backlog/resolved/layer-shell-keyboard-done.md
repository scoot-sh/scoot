---
title: "Layer-shell keyboard interactivity (`keyboard_interactivity`) \u2014 DONE, folded back into item 14"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Layer-shell keyboard interactivity (`keyboard_interactivity`) — DONE, folded back into item 14

~~Layer-shell keyboard interactivity (`keyboard_interactivity`)~~ — DONE,
folded back into item 14 after review pushed back on shipping the gap.
The model this entry laid out (exclusive on top/overlay takes focus while
mapped and front-most wins; `on_demand` anywhere and `exclusive` on
bottom/background are click-to-focus; `none` never) is what was built,
unchanged. Two things the entry did not anticipate: keyboard focus is
*derived* from the layer map on every refresh rather than stored as an
override `apply()` respects, which removed most of the teardown surface it
worried about; and "while it is mapped" needed to mean "has a buffer"
(`LayerSurfaceCachedState::last_acked`), not "is in the layer map", or a
surface that draws nothing could hold every keystroke. See item 14.
