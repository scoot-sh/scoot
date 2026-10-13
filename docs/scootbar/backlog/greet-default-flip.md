---
title: "scootgreet: make it the default greeter when `programs.scoot.greeter.enable` is on"
status: "open"
area: "scootbar"
priority: "low"
blocked: "greet-packaging-iso"
milestone: "M7"
---

# scootgreet: flip the default

Filed 2026-10-12. Serves **daily-drive**. The
[shared criteria](../../../dev/design/shell-apps.md#nix-modules-and-the-desktop-profile-switch) apply, plus the
scoot-iso QEMU boot.

**It must not regress screen-reader users at first boot.** ReGreet stays the default whenever
the system enables a screen reader. This ticket finds and pins the exact NixOS option. Needs the
maintainer's ruling.
