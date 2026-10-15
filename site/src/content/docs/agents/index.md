---
title: Agents
description: "Set up a machine for agent-driven use — headless sessions, IPC, screenshots, and the llms.txt sets."
---

The stack an agent needs: a headless session with no display, the IPC
socket to drive it, screenshots to see it, and this documentation as
plain Markdown.

## The loop

```sh
scoot --headless --outputs 2 -- foot &
scoot msg windows
scoot msg screenshot --out /tmp/shot.png
```

`--headless` renders into a framebuffer with no display;
`scoot msg` is the control client (it also builds on macOS, where `scoot`
is client-only, to drive a compositor in a VM). `windows`
answers every window's id, app id, title, output, workspace and
rectangle — what you click. `screenshot` answers the output's own
pixels. Then act: `scoot msg action focus-column left`, `scoot msg
pointer click X Y`, `scoot msg type "hello"`, `scoot msg key
super+Return`.

Read [Drive scoot over IPC](../msg/index.md) for the socket, then
the rules that keep automation honest:

- [Rules an agent needs](../msg/index.md#rules-an-agent-needs) — window focus vs keyboard focus, the `popup_grab` rule, physical-vs-logical conversion, pointer locks, `wait-idle` pacing.
- [Screenshots](../msg/screenshots.md) — capture reproducibly, the pointer, retry rules.
- [What the socket refuses](../msg/troubleshooting.md) — the ten bounds, paced captures, connection limits.
- [Events](../msg/events.md) — subscribe instead of polling.

`wait-idle` is the pacing primitive: block until nothing has redrawn
for `--quiet-ms` (default 200 ms) before screenshotting or typing.

## Machine-readable docs

Every page ships as plain Markdown: the three bundles
(`/llms.txt` index, `/llms-full.txt` everything, `/llms-small.txt`
compact) plus one twin per page at `/<slug>.md`. Per-app
sets (one per sidebar section) live under `/_llms-txt/`: the
`scoot msg / IPC` set is the grammar reference in one file.

`--help` prints the usage text, every request and every action, with
topics (`scoot msg --help`, `help <verb>`) and `--help --json` for the
machine-readable form — the [contract](../reference/cli.md) every
binary meets. Until the generated per-CLI pages land,
[Actions](../msg/actions.md) is the prose source.
