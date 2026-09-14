---
title: "A layer surface that commits but never attaches a buffer holds its exclusive zone"
status: "open"
area: "protocols"
priority: "low"
blocked: null
---

# A layer surface that commits but never attaches a buffer holds its exclusive zone

A layer surface that commits but never attaches a buffer holds its
exclusive zone (item 14, deliberate, documented in
`a_bar_reserves_its_zone_from_its_initial_commit_not_its_first_buffer`).
Smithay's `LayerMap::arrange` arranges every surface mapped into the map,
buffer or not, so the reservation starts at the initial (buffer-less)
commit the protocol requires. For a healthy bar that window is a frame or
two; for a client that commits and then hangs, the space stays reserved
until it disconnects. Fixing it means filtering on mapped-ness while
computing the zone, which today would mean reimplementing `arrange`
locally — worth doing only if a real client is seen to hit it, or if
upstream grows the distinction.
