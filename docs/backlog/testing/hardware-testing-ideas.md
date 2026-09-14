---
title: "Enhanced testing ideas for the hardware/DRM-dependent gap (research, 2026-09-12) \u2014 evaluate, none committed to yet."
status: "research"
area: "testing"
priority: "research"
blocked: null
---

# Enhanced testing ideas for the hardware/DRM-dependent gap (research, 2026-09-12) — evaluate, none committed to yet.

Enhanced testing ideas for the hardware/DRM-dependent gap (research,
2026-09-12) — evaluate, none committed to yet. This project's `--tty`
backend (real DRM/libseat/libinput) has essentially zero unit-test
coverage by necessity — `Tty` holds real kernel handles a test process
can't construct — so its correctness today rests entirely on code-reading
plus manual hardware bug-bash on the dev VM before every merge. Researched
how other projects close a version of this same gap:
- VKMS (`vkms.ko`), a real in-kernel virtual DRM/KMS driver purpose-built
  for this. Gives a genuine `/dev/dri/cardN` with an emulated CRTC/
  connector/plane, no physical GPU needed. Mesa's own CI combines VKMS +
  llvmpipe (software rendering) + a real Wayland compositor for headless
  DRM-path testing; Collabora published a concrete recipe for testing
  Weston's actual DRM backend this way using `virtme` (a QEMU wrapper
  that boots a custom kernel sharing the host's rootfs, lighter than a
  full VM image). Their own stated caveat: "VKMS does not substitute a
  real graphics card yet," device enumeration order isn't deterministic,
  and they still test on real hardware too — a complement to hardware
  bug-bash, not a replacement for it. This project's dev VM is already a
  real (if virtual, via virtio-gpu) DRM device for manual/agent-driven
  testing; VKMS would be the piece that makes some slice of that
  *automated and CI-runnable* instead of always requiring a live VM
  session. Non-trivial setup cost (custom kernel config, `virtme`,
  figuring out what's actually exercisable without display output).
- Property-based testing for `flexwm-core` specifically (the cheapest,
  most directly actionable idea here, no new infrastructure needed).
  `flexwm-core` is already pure and I/O-free by design (no Wayland, no
  I/O — see this file's vision note), exactly the shape property testing
  wants: generate random sequences of window-management actions (via the
  `proptest` crate) and assert invariants hold (no window ever gets a
  negative width, focus always points at a window that still exists, a
  workspace's columns stay internally consistent after any action
  sequence) instead of hand-writing every case. Confirmed niri — the
  project this one is explicitly modeled on — does exactly this for its
  own layout logic (per its `CONTRIBUTING.md`: "for new layout actions,
  we add randomized tests"), alongside "client-server tests" for Wayland
  protocol edge cases — the same two-tier split (pure unit/property tests
  + live client-server integration tests) this project has already
  organically converged on via `dispatch/tests.rs`/`ipc/tests.rs`.
- WLCS (Wayland Conformance Test Suite), a shared protocol-level
  black-box suite any compositor can plug into via a small adapter —
  Smithay's own reference compositor does this (`wlcs_anvil`). Tests real
  client-visible protocol behavior without touching internals; a bigger
  lift to adopt than the other two, but conceptually the same idea as
  this project's own "drive it over IPC/the protocol and check what
  happens" tests, standardized and shared across compositors.
- libinput's own approach for input hardware (`litest`): builds
  virtual devices via the kernel's `uinput` driver to exercise real
  input-handling code without physical hardware — this project already
  does something in the same spirit (a real `/dev/uinput`-injected
  keystroke was part of item 3's original hardware verification).
Rough priority if picked up: property-based tests for `flexwm-core` first
(cheap, zero new infrastructure, closes a real gap immediately);
VKMS-in-CI second (bigger payoff — real automated DRM-path testing
instead of always needing a live VM session — but real setup cost); WLCS
as a longer-term, larger investment worth knowing exists.
