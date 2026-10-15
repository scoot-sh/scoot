# Compositor protocol evidence (dev note, unpublished)

Reproducibility evidence moved out of the user-facing protocol pages.
Each section below is the full text formerly inline in
`site/src/content/docs/scoot/protocols.md`, kept verbatim so measurements,
versions and review history stay findable. The pages keep a short statement
plus a pointer here.

## Explicit-sync syncobj leak and verification (was protocols.md)

**A leak fixed by forking Smithay.** Upstream Smithay, at the revision scoot
pinned and on its `master`, never destroys the kernel handle an
`import_timeline` creates. So each import would leave one syncobj handle on
scoot's DRM file until scoot exits, about 80 bytes of kernel memory each,
together with any wait scoot abandoned on that syncobj when a surface was
destroyed mid-wait (about 200 bytes each). A hostile client could drive
that at wire speed (review measured about 24 MB/s), and no per-client bound
stops it, because every iteration ends with nothing live. scoot therefore
builds against a scoot-sh fork of Smithay that adds the one missing `Drop`.
Measured with it, both loops stay flat. Details are in
[`backlog/resolved/syncobj-handle-leak-done.md`](https://github.com/scoot-sh/scoot/tree/main/docs/backlog/resolved/syncobj-handle-leak-done.md).
Returning to upstream once it has the fix is
[`backlog/core/smithay-fork-repin.md`](https://github.com/scoot-sh/scoot/tree/main/docs/backlog/core/smithay-fork-repin.md).

**What has been verified.** On the dev VM's GPU tier (virtio-gpu), with a
test client that renders into card0 dumb buffers and signals its own syncobj
timelines standing in for a GPU: a held commit shown only after its signal,
also across a VT switch; a client that never signals stalling only itself,
while another client's frame pacing and IPC latency stay unchanged; the fds
of a client killed mid-wait all returned; both bounds disconnecting only
the offender; a fullscreen explicit client going direct on every frame, its
replaced buffers released when the replacement's flip completed; no change
in compositor CPU. On the dev VM no real explicit-sync client could be
run against it: Mesa's `vkcube` (lavapipe) and `es2gears_wayland` (llvmpipe)
both draw through `wl_shm` and never bind the global. On an Apple M2
([`Asahi.md`](https://github.com/scoot-sh/scoot/tree/main/Asahi.md)'s Test 7) the global is offered on the display
device itself (`apple,dcp` supports timelines), and Mesa's Vulkan driver
uses it: `vkcube` imported 40 timelines and set 827 acquire and 827
release points in about 14 s, with no stall or protocol error. `--nested` was not run live; the global
cannot appear there, since the only code that offers it runs in `--tty`'s
startup.

Fork entry: `dev/forks.md` (`scoot-sh/smithay`, `43f50eb2`, ~24 MB/s).
