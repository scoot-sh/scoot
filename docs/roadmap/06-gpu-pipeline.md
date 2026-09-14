---
item: "6"
title: "Real GPU rendering pipeline"
status: "planned"
area: "backend"
pr: null
commit: null
---

# Real GPU rendering pipeline

A real GPU rendering pipeline, added at the end after everything above is
stable — an actual goal, not just a "don't foreclose it" constraint.
Likely a GLES/Vulkan Smithay renderer as an alternative to pixman,
selected per-backend (e.g. tty backend prefers GPU when a real DRM/GBM
device supports it, headless/nested/webtop keep pixman) rather than
replacing CPU rendering outright, since GPU-free operation for
webtop/no-GPU boxes stays a hard requirement. This is exactly why the
render-target/presentation split from item 1 onward matters: it's the
seam a GPU renderer slots into.
