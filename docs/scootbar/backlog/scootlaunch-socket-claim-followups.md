---
title: "scootlaunch socket claim follow-ups from the #766 review"
status: "open"
area: "scootbar"
priority: "medium"
blocked: null
---

# scootlaunch socket claim follow-ups from the #766 review

Filed 2026-10-27. Serves **daily-drive** (a second press on the launcher bind must never silently do nothing, hang, or leave the toggle name missing; the published socket and its lock file are the toggle contract).

Each item below was re-verified against the code on `origin/main` at `51e30fef` (the #766 merge) before filing; line refs are `crates/scootlaunch/src/socket.rs` unless named otherwise.

## The gap

- **F1 (same-uid lock-holder wedge):** `claim()` maps a held lock to a silent yield: `flock` `WOULDBLOCK` returns `Ok(Open::Signalled)` (`socket.rs:167-175`), so the second press exits 0 with no panel and no error. The review proved it live: an attacker `flock -x` on the real lock name with no panel makes the victim return `Signalled` and publish nothing. Cross-uid is stopped by the `0700` runtime dir; a same-uid attacker already has equivalent primitives (squat the name, send `close`, fill the backlog), so this is a documented-limitation class, not a regression.
- **F2 (blocking fast-path connect, pre-existing):** the fast path `UnixStream::connect` in `open()` (`socket.rs:141`) is blocking, so a `SIGSTOP`ped panel with a full backlog hangs the toggling press — the exact hazard the slow-path probe's comment names (`socket.rs:199-202`). The slow path `answers()` is nonblocking (`socket_with` + `NONBLOCK`, `socket.rs:203-219`). Pre-existing behavior; only the slow path was fixed.
- **F3 (unlink-then-rename gap):** `clear_stale` unlinks the stale socket (`remove_file`, `socket.rs:246`) and `bind_staged` renames later (`rename`, `socket.rs:314`), leaving a missing-name window. A direct rename-over-stale would be atomic with no gap. Only a same-uid/old-version racer could exploit the gap.
- **F4 (staging not fd-pinned):** `bind_staged` builds the staging dir with `DirBuilder::mode(0700)` + `set_permissions` + `rename` (`socket.rs:277-320`) with no `O_DIRECTORY|O_NOFOLLOW` recheck, no `/proc/self/fd` bind, no `fchmod` pin — despite the module docs saying "as scoot's IPC listener does" (`socket.rs:20-23`, `137-139`). Contrast `crates/scoot/src/compositor/ipc/listener.rs:147-211`, which re-opens staging with `O_DIRECTORY|O_NOFOLLOW`, `fchmod`s through the fd, and binds under `/proc/self/fd/<n>`. Same-uid swap / pathological umask (e.g. `777`) only.
- **F5 (lock open lacks `O_NOFOLLOW`):** the lock `OpenOptions` (`socket.rs:159-166`: `read+write, create, truncate(false), mode 0600`) sets no `O_NOFOLLOW`, so a symlinked lock path is followed (verified in the review: holder listens, victim content intact). Safe in practice (no unlink/truncate through it), but asymmetric with the care elsewhere.
- **F6 (unreachable `SUN_PATH_MAX` message):** `bind_staged`'s length check (`socket.rs:261-267`) is unreachable via `claim()`: overlong paths already fail in `answers()` at `SocketAddrUnix::new` (`socket.rs:213`, `ENAMETOOLONG`, observed in the review). Still a loud refusal; the friendly message is dead.
- **F7 (site names no lock file):** `site/src/content/docs/scootlaunch/panel.md`'s toggle paragraph ("no pid file, no abstract name, just a socket in `$XDG_RUNTIME_DIR` with the peer's uid checked") names no lock file, now one file short of the truth. The toggle contract itself is unchanged.
- **F8 (untidy resolved ticket):** `docs/scootbar/backlog/resolved/launch-toggle-race-and-socket-mode-done.md` keeps empty template sections (`The gap`, `What to do`, `Not in this ticket`), the `Found reviewing #674` draft paragraph, and a `## Docs (same PR)` instruction block. `scripts/backlog check` passes; still untidy for the archive.
- **F9 (stale `Claim._lock` comment):** the field comment (`socket.rs:362-364`: "`None` is never used") describes an `Option`, but the field is a plain `File`. Stale comment.

## What to do

- F1: brief wait-and-reprobe before yielding, or a visible error for the wedge; pin with a test that holds the lock with no panel and asserts the second press's behavior.
- F2: nonblocking fast-path connect or a bounded timeout; pin with a stopped listener + full backlog (the press must not hang).
- F3: rename-without-unlink over the stale socket; pin the no-gap shape (name always resolves during handover).
- F4: align staging with `compositor/ipc/listener.rs` (`O_DIRECTORY|O_NOFOLLOW` recheck, `fchmod`, `/proc/self/fd` bind) or soften the "as scoot's IPC listener does" doc claim; pin the chosen shape.
- F5: add `O_NOFOLLOW` to the lock open; pin symlink-at-lock-path behavior.
- F6: reorder the length check ahead of the probe or drop the dead arm; pin the overlong-path message.
- F7: one line in `panel.md` naming `scootlaunch-<display>.lock` beside the socket.
- F8: tidy the archive file (fill or drop the empty sections, remove the draft paragraph and the instruction block) as part of the follow-up change.
- F9: fix the `_lock` comment to describe the plain-`File`-held-for-life shape.

## Not in this ticket

The #766 fix itself (atomic lock claim + `0600`-before-publish staging rename, landed as `51e30fef`); the shared `scootui::control::Claim` bind-then-chmod window and its `scootbg` duplicate (`control-socket-mode-window.md`); hot-path or idle behavior (unchanged: lock fd never polled, staging runs once at startup).
