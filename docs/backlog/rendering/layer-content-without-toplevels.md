---
title: "[nested] top-layer exclusive-zone content missing from frames with no toplevels mapped"
status: "open"
area: "rendering"
priority: "high"
blocked: null
---

# [nested] top-layer exclusive-zone content missing from frames with no toplevels mapped

Filed as gh issue #183 (live field report, same session as #182 — read
the issue for the full screenshot matrix and the reporter's trace; this
entry tracks it).

scoot `4f8707c` `--nested`, 430x744, noctalia-shell 4.7.7 bar with 30px
exclusive zone. Screenshot matrix (`scoot msg screenshot`): no toplevels
→ wallpaper only, bar absent; ghostty mapped → bar rendered on top;
ghostty closed → bar absent again. `scoot msg outputs` holds
`usable={y:30,h:714}` throughout — the zone-owning surface stays
registered while contributing no content.

Reporter's trace (verify, don't assume): `gather_elements` collects
`ABOVE_WINDOWS` layers unconditionally (`render/elements.rs`), and
`layer_elements` iterates every mapped surface — no window-count gate in
the render path found. The client draws fine the moment any toplevel
exists. Suspect layer-map/state side: the zone-owning surface is present
for exclusion but skipped (or bufferless) in composition exactly when the
window stack is empty.

Notes: kept separate from #182 deliberately (clicks dead even when the
bar IS rendered), though both smell like layer handling keyed off window
existence — confirm or refute the shared root. Note phase B (PR #186)
touched layer maps/callbacks since the report rev (`4f8707c` predates
multi-output A–D): re-verify against current `main` first — it may have
moved. Fail-first with mapped layer surface + zero windows + pixel
census; live `--nested` proof.
