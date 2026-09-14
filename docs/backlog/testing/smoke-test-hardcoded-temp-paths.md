---
title: "`scripts/smoke-test.sh` hardcodes some of its temp paths, so two concurrent runs (e.g. two agents verifying different branches on the same VM at once) can collide (LOW, pre-existing)."
status: "open"
area: "testing"
priority: "low"
blocked: null
---

# `scripts/smoke-test.sh` hardcodes some of its temp paths, so two concurrent runs (e.g. two agents verifying different branches on the same VM at once) can collide (LOW, pre-existing).

`scripts/smoke-test.sh` hardcodes some of its temp paths, so two
concurrent runs (e.g. two agents verifying different branches on the same
VM at once) can collide (LOW, pre-existing). Found by `flexwm-reviewer`
reviewing item 16, whose own PR body overstated its run's isolation: the
`SOCKET`/`LOG` environment overrides the script does honor don't cover
every path it uses — `/tmp/flexwm-smoke-config.log`/`-config.sock` and
`-broken.*` are hardcoded (`scripts/smoke-test.sh` around lines 299-300
and 392-393) regardless of what `SOCKET`/`LOG` are set to. Low priority —
this session's own practice of using distinctly-named scratch scripts and
checking for other active agents before running concurrent hardware
verification has avoided hitting it so far — but worth closing so two
agents' hardware bug-bashes can't silently corrupt each other's evidence
if they ever do overlap. Fix direction: derive every temp path in the
script from the same overridable prefix, not just the two that happen to
have env vars today.
