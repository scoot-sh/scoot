# Shipped milestones

One file per numbered roadmap item, in the order it was worked. Each carries
YAML frontmatter (`item`, `title`, `status`, `area`, `pr`, `commit`) so they
can be filtered without parsing prose:

```sh
rg -l 'status: "planned"' docs/roadmap        # what is not done yet
rg -l 'area: "protocols"' docs/roadmap        # everything protocol-shaped
```

The prose inside each file is the original `ROADMAP.md` entry, moved
verbatim — review findings, raw evidence, exact commands and all. `ROADMAP.md`
at the repo root is the index; this directory is the detail.

All of it predates the rename to `scoot` (2026-09-18) and keeps the old name,
`flexwm`, verbatim for the same reason it is verbatim in every other respect:
the nix store paths, typed input strings, screenshot paths and commands here
are a record of what was actually run. Read `flexwm` as `scoot`.

| Item | Title | Status | PR |
| ---- | ----- | ------ | -- |
| [1](./01-nested-backend.md) | Nested backend | done | — |
| [2](./02-keybindings-layer.md) | Keybindings layer | done | #4 |
| [3](./03-tty-drm-backend.md) | Real tty/DRM backend | done | #5 |
| [4a](./04a-config-file.md) | Config file | done | #6 |
| [4b](./04b-decorations.md) | Window decorations | done | #7 |
| [5](./05-cursor-rendering.md) | Cursor rendering for `--tty` | done | #8 |
| [5a](./05a-merge-authority.md) | Merge-authority history | done | — |
| [5b](./05b-vt-switch-eperm.md) | VT-switch-back `EPERM` | done | #9 |
| [6](./06-gpu-pipeline.md) | Real GPU rendering pipeline | **planned** | — |
| [7](./07-shm-resize-crash.md) | `wl_shm_pool.resize(0)` crash-DoS | done | #12 |
| [8](./08-client-cursor-surface.md) | Client-supplied cursor images | done | #13 |
| [9](./09-ipc-hardening.md) | IPC hardening | done | #14 |
| [10](./10-ipc-connection-loop.md) | Non-blocking IPC connection loop | done | #15 |
| [11](./11-keystroke-flush.md) | Flush a real keystroke immediately | done | #17 |
| [12](./12-four-bounds.md) | Four missing bounds | done | #18 |
| [13](./13-cursor-size-color.md) | Configurable fallback cursor | done | — |
| [14](./14-layer-shell.md) | `wlr-layer-shell-unstable-v1` | done | #22 |
| [15](./15-ext-workspace.md) | `ext-workspace-v1` | done | #24 |
| [16](./16-nix-packaging.md) | `nix build` / `nix run` | done | #26 |
| [17](./17-drm-device-selection.md) | DRM device-selection fallback + `--gpu` | done | #27 |
| [18](./18-session-lock.md) | `ext-session-lock-v1` | done | #25 |

Item 6 is the only remaining *ordered* milestone; everything else current
lives in [`docs/backlog/`](../backlog/).
