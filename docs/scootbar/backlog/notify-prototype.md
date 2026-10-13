---
title: "scootnotify go/no-go prototype: own the name, draw a card, measure against mako"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "scootui-extract-draw"
milestone: "M7"
---

# scootnotify prototype: the go/no-go gate

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#goals-non-goals-and-the-case-for-it).
Serves **daily-drive**; decides whether scootnotify is built or mako stays.

## What to do

A throwaway-quality binary on `scootui` and scootbar's D-Bus client: owns the name,
answers `Notify` with one card on an overlay surface, nothing else. Measure in the same
harness as lean mako (`docs/backlog/resolved/desktop-notifications-done.md`): idle RSS/PSS,
wakeups in 60 s, one card up. Render the real-client corpus (notify-send plain, markup,
`-i` name and path, `-h int:value:`, libnotify `image-data`, Firefox, Thunderbird, an
Electron app, a GTK4 app through the portal; one CJK, one Cyrillic body) beside mako.

## Acceptance

The gate in the design, recorded with the numbers: idle RSS at most half of mako's, 0
wakeups, a legible corpus judged by the maintainer from the screenshots, and every call
answered. A **no** resolves the epic as won't-do with this evidence.

## Evidence the review needs

The rows with the SHA and method, both sets of screenshots, the spike text-shaping result if
it has landed.

## Not in this ticket

Anything kept: the real server is `notify-server`.
