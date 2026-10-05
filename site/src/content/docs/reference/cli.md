---
title: Generated CLI pages
description: "Reserved slot — complete CLI references generated from --help --json."
---

Reserved. When the agent-friendly `--help` work lands (`--help
--json` on every binary), this section gains one generated page per
CLI — `scoot`, `scootctl`, `scootbar`, `scootbg` — rebuilt by the site
build from the binaries' own output, so no flag can rot. Until then:

- `scoot` flags live in [scoot overview](../scoot/index.md#launch-it-three-ways); requests and actions in [scootctl](../scootctl/actions.md).
- `scootbar` flags and `msg` live in [scootbar CLI reference](../scootbar/cli.md).
- `scootbg` commands live in [scootbg CLI reference](../scootbg/cli.md).
- `--help` on any binary prints the current text today: no display needed.
