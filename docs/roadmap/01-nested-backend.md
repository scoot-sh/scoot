---
item: "1"
title: "Nested backend"
status: "done"
area: "backend"
pr: null
commit: "494048b"
---

# Nested backend

~~Nested backend~~ — DONE, merged to `main` at `494048b`. Independent
review found 2 real low-probability bugs (resize-failure wedge,
skipped-frame-never-retried) + 1 unneeded `unsafe impl Send`; all fixed
and re-verified (`--headless`/`--nested` smoke test, 19 tests, clippy
clean) before merge.
