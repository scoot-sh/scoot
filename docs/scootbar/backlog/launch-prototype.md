---
title: "scootlaunch go/no-go prototype, out of the workspace: typing, latency, activation vs fuzzel"
status: "open"
area: "scootbar"
priority: "medium"
blocked: "maintainer go-ahead on the shell-apps design (dev/design/shell-apps.md, Decisions needed 1)"
milestone: "M7"
---

# scootlaunch prototype: the go/no-go gate

Filed 2026-10-12 in the #579 fix round. The gate moved out of `launch-ui` to run before any
extraction. Serves **daily-drive**.

## What to do

A throwaway binary **outside the workspace**, `#[path]`-including scootbar's drawing and text
sources (`crates/scootbar/fuzz/Cargo.toml:28-65`'s pattern) and the keyboard spike's pick. It
shows an overlay panel with exclusive keyboard focus over a real desktop-entry list, plus a
mouse-pick launch.

Measure against fuzzel 1.14.1 in the same harness on the M2. The #579 review measured fuzzel
open at 18,944 kB RSS; the first frame is 50 to 57 ms (`desktop-launcher-done.md`).

## Acceptance

All of the following, recorded here:

- first frame warm and cold, open RSS, and keystroke-to-commit at 300 and 3,000 entries, all
  at or under fuzzel's;
- a query typed correctly on AZERTY and with `us(intl)` dead keys;
- a mouse-picked entry re-activating an already-running single-instance app. That is the case
  scoot's source documents fuzzel failing (`crates/scoot/src/compositor/activation.rs:81-99`).

## Resolving this gate

The tickets it guards name it in prose ("GO verdict recorded in launch-prototype"), which `scripts/backlog resolve` does not strip ([why](../../../dev/design/shell-apps.md#how-the-gates-stay-closed)). Record the verdict here, then resolve. On **go**, a human deletes that clause from the guarded tickets. On **no**, a human resolves the epic and the guarded tickets as won't-do, citing this evidence.

## Evidence the review needs

The rows beside fuzzel with the SHA, the typing transcript, and the activation screenshot.
