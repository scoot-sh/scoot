---
title: "Open question: does `--tty` over SSH on the dev VM actually hold real DRM master, or is it running in \"unprivileged mode\" the whole time? \u2014 RESOLVED 2026-09-13 (investigation + docs fix, PR #20). It holds real DRM master. The warning is a red herring, it is not SSH-specific, and no flexwm code change is warranted."
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# Open question: does `--tty` over SSH on the dev VM actually hold real DRM master, or is it running in "unprivileged mode" the whole time? — RESOLVED 2026-09-13 (investigation + docs fix, PR #20). It holds real DRM master. The warning is a red herring, it is not SSH-specific, and no flexwm code change is warranted.

~~Open question: does `--tty` over SSH on the dev VM actually hold real
DRM master, or is it running in "unprivileged mode" the whole time?~~ —
RESOLVED 2026-09-13 (investigation + docs fix, PR #20). It holds real DRM
master. The warning is a red herring, it is not SSH-specific, and no
flexwm code change is warranted. Raised by `flexwm-reviewer` while
reviewing item 13: every `--tty` run over SSH logs Smithay's own `Unable to
become drm master, assuming unprivileged mode` at device-open time, in
apparent tension with `vm/README.md`. Answer, from primary sources plus
live measurement on the dev VM at `0678765`:

**Mechanism.** "Unprivileged mode" is Smithay's name for "this process may
not call `SET_MASTER` itself", not "this process is not master". Master
goes to whichever open file is *first* to open the device while nothing
else already holds it, plus seatd's own explicit `DRM_IOCTL_SET_MASTER`
call on that file right after (`seatd/seat.c`) — root is not what grants
master (`drm_master_open`, `drivers/gpu/drm/drm_auth.c`, hands it to the
first opener regardless of uid, and root can't take it from an existing
holder either: `drm_setmaster_ioctl` returns `EBUSY`); root is what lets
seatd open the device node and manage VTs at all. Since seatd is normally
the only thing that ever opens the GPU node on this VM, in practice it *is*
first, and passes that already-master fd over its socket to flexwm, which
inherits it (master is a property of the open file, not the process) — but
that is a fact about this VM's setup, not something "opened by seatd"
guarantees in general, and the caveat below is exactly why. flexwm's own
`SET_MASTER` inside Smithay's `DrmDeviceFd::new` is then refused with
`EACCES` because kernel 6.18's `drm_master_check_perm`
(`drivers/gpu/drm/drm_auth.c`) requires `was_master && file->pid ==
current->tgid` (or `CAP_SYS_ADMIN`), and `drm_file_update_pid`
(`drm_file.c`) deliberately never re-owns a file that was master — so the
fd's recorded owner stays seatd forever. Smithay's resulting `privileged =
false` is the *correct* state for the libseat path: it is what stops
`DrmDevice::pause`/`activate` (`device/mod.rs:417`/`431`) from issuing
`SET_MASTER`/`DROP_MASTER` themselves, which seatd already does as root on
every VT switch. **The identical warning also fires when master genuinely
isn't held**: if something else already has it when seatd opens the device,
seatd's own `SET_MASTER` gets `EBUSY` too, only logs it, and hands the fd
over anyway — that case fails loudly at modeset instead of at open, which is
exactly why the evidence below checks the actual kernel state rather than
trusting the log line alone.

**Evidence** (commands and raw output in PR #20's description). While an
SSH-started `--tty` runs: `/sys/kernel/debug/dri/0/clients` shows exactly one
client, `seatd 460 ... master y`; a root `drmSetMaster` probe gets `EBUSY`
(root cannot take master, i.e. someone holds it) and `drmIsMaster` reads 0;
`/sys/kernel/debug/dri/0/state` shows `crtc-0 enable=1 active=1`, mode
`1600x1000`, with the plane's `fb=42` *allocated by flexwm* — real scanout,
not the pixman intermediate. A `chvt 2`/`chvt 1` cycle moves all of it in
lockstep: `master n` + plane back to `[fbcon]`'s fb + probe acquires master
freely while paused, then `master y` + plane back to flexwm's fb + `EBUSY`
again after the switch back. The `EACCES`-despite-master condition was also
reproduced in isolation with no seatd involved at all (a process opens
card0, `drmIsMaster=1`; its forked child, same open file, different tgid,
also reads `drmIsMaster=1` but gets `EACCES` from `drmSetMaster`) — which is
what makes "the warning does not mean what it looks like" a fact rather than
an inference. The same warning appears verbatim when started from a real VT
(`openvt -c 3 -s`, tty3 as controlling terminal and foreground VT) with
master equally held, so it was never about SSH.

**Why SSH works at all — `vm/README.md`'s stated reason was wrong, its
conclusion was right.** It credited "logind's PAM stack registers those with
a real seat/session too." It doesn't: `loginctl` reports `Seat=`, `VTNr=0`,
`Remote=yes` for an SSH session, and `LIBSEAT_BACKEND=logind flexwm --tty`
over SSH fails immediately with `Failed to open session: No data available`.
What actually happens is that libseat uses its **seatd** backend, and
seatd's `seat0` is VT-bound: `seat_add_client` assigns every client
`seat->cur_vt`, the VT in the foreground at connect time, regardless of how
the process was started. Confirmed by the session number in seatd's own log
tracking the foreground VT: `Added client 1 to seat0` over SSH with tty1
foreground, `Added client 3 to seat0` for the `openvt -c 3 -s` run. Fixed in
`vm/README.md` (mechanism, a "is it really DRM master" troubleshooting entry
with the two debugfs checks, and the two consequences of VT binding: one
libseat client at a time on a VT-bound seat, and `XDG_RUNTIME_DIR` must
exist) and in `vm/configuration.nix`'s `services.seatd.enable` comment,
which carried the same wrong claim.

**What this means for prior hardware claims: they stand, and now have a
mechanism behind them.** Items 3 and 5b are the ones that specifically
depended on *holding* master (real scanout; VT-switch pause/reactivate
semantics), and the cycle above re-demonstrates both directly. The
historical runs are covered too, without re-running them: seatd logs
`Could not make device fd drm master: ...` whenever its own root-side
`drm_set_master` fails, and
`journalctl -u seatd | grep -c 'Could not make device fd drm master'`
returns **0** across this VM's entire persistent journal — 2072 seatd lines
and 323 `Opened client` events, reaching back to its first boot at
2026-09-11 15:16:57 — while those same runs logged `drm: modeset (full
commit)` with no error, and `DRM_IOCTL_MODE_ATOMIC` is `DRM_MASTER`-gated by
`drm_ioctl.c`, so the kernel would have returned `EACCES` had master not
been held. One limit, stated rather than papered over: nothing here is a
*photograph* of the QEMU window — host `screencapture` is blocked by macOS
Screen Recording permission in this environment ("could not create image
from display"). The scanout evidence is the kernel's own atomic state plus
the master-gating of the ioctl that set it, which is strictly more specific
than a photo, but if a future claim wants a visual, look at the window.
