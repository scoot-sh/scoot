---
title: "`State::listen` panics (aborts the process) if `$XDG_RUNTIME_DIR` is unset, instead of a clean startup error (LOW) \u2014 DONE as item 12(a)"
status: "resolved"
area: "resolved"
priority: null
blocked: null
---

# `State::listen` panics (aborts the process) if `$XDG_RUNTIME_DIR` is unset, instead of a clean startup error (LOW) — DONE as item 12(a)

~~`State::listen` panics (aborts the process) if `$XDG_RUNTIME_DIR` is
unset, instead of a clean startup error (LOW)~~ — DONE as item 12(a), with
the before/after release-binary evidence (core dump vs one-line error)
recorded there. Original diagnosis, left as written: found incidentally by
`flexwm-reviewer` while reviewing PR #14, unrelated to that PR and left
untouched by it. `crates/flexwm/src/compositor/state.rs:196`:
`ListeningSocketSource::new_auto().expect("a free wayland socket")` — the
actual failure in this case is Smithay's `RuntimeDirNotSet`, which
`.expect()` turns into a panic and a core dump rather than a message
telling the operator what's actually wrong. Fix: match on the error and
print a clear startup error instead of panicking, same shape as this
project's other clean-startup-error paths (e.g. `ipc::init`'s "no socket
path" error).
