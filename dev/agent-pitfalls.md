# Known pitfalls (hit in this repo; every implementer and reviewer checks these first)
1. Events discarded next to a reply: any "pump until reply X" loop must park every other event (calls, signals), never drop them (scootnotify #690).
2. Defaults that must follow an option: when a value is chosen by a toggle (e.g. launcher daemon), every consumer's default follows it (NixOS menu defaults, #681).
3. Capabilities off in production but on in tests: struct Default vs the production constructor (pidfds, #685). Test the production path.
4. Unix socket paths <= 107 bytes under CI TMPDIR; short scratch prefixes.
5. Waits must fail fast on a dead child and name the daemon state in the failure message.
6. Idle wakeups: measure live (voluntary_ctxt_switches over 60 s) for anything with timers or sockets.
7. A red CI is a bug until `gh run view --log-failed` proves otherwise; never rerun-and-hope.
8. Breaking = any CLI, config schema or control-socket protocol change: mark `!` + BREAKING CHANGE footer.
9. New crate: release-plz [[package]] entry, ci.yml classify filter, changelog_include if user-visible.
10. Docs in the same PR (site page; README only if what scoot is changed).
Reviewers: start the CI watch the moment the PR head is known and review while CI runs; do not wait for green before reading the diff.
