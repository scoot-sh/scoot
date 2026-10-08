---
title: "Desktop: an input-method engine slot (fcitx5-class) for CJK and compose"
status: "open"
area: "packaging"
priority: "low"
blocked: null
---

# Desktop: an input-method engine slot (fcitx5-class) for CJK and compose

Filed 2026-10-04, child of `desktop-paved-path` (from PR #430's review).
Serves **daily-drive** for users who type CJK or rely on an IME.

## The gap

The compositor implements `text-input-v3` and `input-method-v2`
(`site/src/content/docs/scoot/protocols.md`, table and "Input methods"
section), but nothing on the
paved path installs or starts an engine, so the protocols have no client.

## What to do

An off-by-default `desktop.inputMethod` slot: pick the lightest engine that
speaks `input-method-v2` (fcitx5 vs others, say why), start it as a user unit
ordered after the session target, set the toolkit variables only where a
toolkit still needs them, and theme its candidate window from the look.

## Acceptance

Eval pins; on the M2, type a CJK string into a GTK app, a Qt app and foot,
and show the candidate popup placed at the cursor.

## Not in this ticket

On by default (most users do not need it); XIM for X clients beyond what
XWayland already carries.
