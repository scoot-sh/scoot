---
title: "scootnotify go/no-go prototype, out of the workspace: own the name, draw a card, compare with mako"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
milestone: "M7"
---

# scootnotify prototype: the go/no-go gate

Filed 2026-10-12 from the [scootnotify design](../../../dev/design/scootnotify.md#goals-non-goals-and-the-case-for-it).
Serves **daily-drive**. It decides whether scootnotify is built or mako stays, **before** any
extraction is spent on it.

## What to do

A throwaway binary **outside the workspace** that `#[path]`-includes scootbar's drawing, text
and D-Bus sources, the pattern `crates/scootbar/fuzz/Cargo.toml:28-65` already uses. It owns
the name and answers `Notify` with one card on an overlay surface, and nothing else.

The memory row is close to certain to pass: scootbar idles at 3.6 to 4.3 MB, and stock mako
was measured at 9,424 kB RSS (#579 review). The rows that can fail are text quality and hangs.
Measure them in the same harness as mako:

- idle RSS/PSS, wakeups in 60 s, and with one card up;
- a real-client corpus rendered beside mako: notify-send plain, markup, `-i` name and path,
  `-h int:value:`; libnotify `image-data`; Firefox; Thunderbird; an Electron app; a GTK4 app
  through the portal; at least one CJK and one Cyrillic body, plus Arabic if the shaping spike
  has landed;
- a 5 MiB `Notify` answered, not hung.

## Acceptance

The design's gate, recorded here with the numbers:

- idle RSS at most half of mako's, and 0 wakeups;
- a legible corpus, judged by the maintainer from the screenshots;
- every call answered.

## Resolving this gate

The tickets it guards name it in prose ("GO verdict recorded in notify-prototype"), which `scripts/backlog resolve` does not strip ([why](../../../dev/design/shell-apps.md#how-the-gates-stay-closed)). Record the verdict here, then resolve. On **go**, a human deletes that clause from the guarded tickets. On **no**, a human resolves the epic and the guarded tickets as won't-do, citing this evidence.

## Evidence the review needs

The rows with the SHA and method, both sets of screenshots, and the prototype's source (out of
the workspace, deleted after).

## Not in this ticket

Anything kept: the real server is `notify-server`.
