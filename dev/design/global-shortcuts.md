# Global shortcuts: design + stage 1 (compositor side)

Portal interface target: `org.freedesktop.portal.GlobalShortcuts` v2
(`CreateSession`, `BindShortcuts`, `ListShortcuts`, `ConfigureShortcuts`;
signals `Activated`/`Deactivated`/`ShortcutsChanged`). Backend interface:
`org.freedesktop.impl.portal.GlobalShortcuts` v2. No stable Wayland protocol
exists; `xx-hotkey-v1` is experimental (wayland-protocols MR 525) and
`hyprland-global-shortcuts-v1` is compositor-specific. scoot implements the
portal interface via a scoot-owned backend (stage 2); the compositor side
(stage 1, this PR) lives on scoot's existing IPC.

## Binding table

New module `crates/scoot/src/compositor/global_shortcuts.rs`, owned by
`State`, read by `input.rs`. `scoot-core` untouched (platform-independent).

- Key: `(app_id: String, shortcut_id: String)`. One entry per shortcut:
  `description`, `preferred_trigger: Option<String>` (raw hint, never parsed
  as authority), `bound: Option<(Modifiers, Keysym, String)>` (canonical
  combo string), `owner: u64` (IPC conn id that registered it).
- Caps (DoS bounds, refused with an ordinary IPC error, never a
  disconnect): at most 64 shortcuts per connection
  (`MAX_SHORTCUTS_PER_CONNECTION`) and 512 compositor-wide
  (`MAX_SHORTCUTS_TOTAL`); `preferred_trigger` at most 256 bytes,
  `description` at most 1024 bytes. Re-registering an existing id is an
  upsert and never counts against either count cap, but its new strings
  still face the length caps. The count caps plus the per-row string caps
  bound the `shortcuts` list reply (the fullest possible reply stays under
  the 1 MiB IPC line).
- Reverse index: `HashMap<(Modifiers, u32), (String, String)>` keyed on
  `(mods, keysym.raw())` (`Keysym` has no `Hash`; same keying as
  `config::apply_binds`). Hot path (`input::key_with`, per keypress) does
  one `match_key` miss on the compositor table, then one hash lookup with a
  stack-built key: no allocation. Emission (encode + subscriber fan-out)
  runs only on actual press/release, like the keyboard-layout emitter.
- Config source: `[global_shortcuts]` table, keys
  `"app-id:shortcut-id" = "combo"` (e.g.
  `"org.obs-studio:toggle-record" = "super+ctrl+r"`), value `""` or
  `"none"` clears. Parsed with the same `KeyCombo` + `keysym_named`
  resolution as `[binds]`; bad combos and collision groups are skipped
  loudly into `skipped_global_binds`, mirroring `skipped_binds`.

## Who owns assignment

The user does, via config file or `shortcut-bind` IPC. The app's
`preferred_trigger` is a hint only: stored, listed, never auto-bound.
Rationale: silent auto-binding lets any app squat a chord the user never
approved, and collides with the compositor's own binds. A fresh register
lands unbound unless `[global_shortcuts]` already names it; `BindShortcuts`
(stage 2 backend) maps to `shortcut-bind` with the same rule.

## Grab placement

`input::key_with`, `KeyState::Pressed`, after `keybindings.match_key`
misses, in one let-chain (`!is_empty && !locked && match &&
!shadowed`): with no shortcut registered the whole grab is one length
check, so the common session pays no hash at all; with registrations
one hash lookup on a stack-built key -- zero hot-path allocation either
way. Emission (encode + subscriber fan-out) runs only when a chord
actually fires, like the keyboard-layout emitter:

1. compositor bind hit -> intercept as today (compositor always wins);
2. else global registry hit and session unlocked -> insert keycode into
   `global_suppressed`, emit `ShortcutActivated`, `Intercept`;
3. else `Forward`.

Release: `cancel_bind_repeat`, then if keycode in `global_suppressed`
remove, emit `ShortcutDeactivated`, `Intercept`; else existing path.
(Synthetic `Request::Key` presses run this same path through `State::key`,
so an agent pressing a bound chord fires it exactly like a person --
consistent with compositor binds. Virtual keyboards run the same grab in
`virtual_filter`, but only with `[virtual_input] binds` on: the off path
stays one bool with no table lookup, mirroring the compositor-bind rule
and its trust boundary.)
Modifiers-only cannot bind (`KeyCombo` requires a key; `keysym_named`
refuses bare modifiers). Key repeat (`held_keys` re-press without
transition) never re-fires: only the first press emits `Activated`.
Locked session: global shortcuts do not fire (forwarded to lock client);
`ChangeVt` and `allow_when_locked` spawns keep their existing path.
Focus change between press and release: the suppressed set swallows the
release regardless of new focus (same invariant as `suppressed_keys`).

Press/release timestamps: `self.millis()` at intercept time, carried in
the event payloads (`timestamp_ms: u64`). Same clock as `InputTime`.

## Portal mapping (stage 2 contract, fixed here)

- `CreateSession` -> the registering IPC connection itself: stage 1 has
  no session object, so the connection id (`State::current_conn`) *is*
  the session. A re-register under the same `(app_id, shortcut_id)` moves
  that id to the newest connection; a disconnect drops every entry its
  connection owned.
- `BindShortcuts` -> `shortcut-register` (idempotent upsert by
  `(app_id, shortcut_id)`) + `shortcut-bind` for user-approved chords.
- `ListShortcuts` -> `shortcuts` list reply.
- `ConfigureShortcuts` -> out-of-band user edit of `[global_shortcuts]`
  + `reload`; stage 1 has no in-compositor configure UI.
- `Activated`/`Deactivated` -> subscribed `shortcut_activated` /
  `shortcut_deactivated` events. `ShortcutsChanged` -> emitted on
  register/unregister/(re)bind/reload so a subscribed backend re-lists;
  stage 1 carries it as a `shortcuts_changed` event on the same
  `global-shortcut` kind.

## Lifecycle

Entries are per-session, owned by the registering IPC connection id
(`State::current_conn`, set by `Connection::serve` before
`handle_request`; tests use `0`). `drop_subscriber` also drops that
conn's entries and emits `shortcuts_changed`; explicit
`shortcut-unregister` drops one. A dead registrant's chords go quiet on
disconnect with no stale suppressions (`global_suppressed` holds keycodes,
cleared on release; conn drop does not need keycode sweep because releases
still route through `key()`). A connection parked in `wait-idle` has
already left the event loop (`Connection::step` returns `Step::Close`),
so parking that connection also drops the shortcuts it registered -- while
its waiter fd lives on in `pending_idle`. In practice one request runs per
connection, so the pairing never matters; a client that registers and then
parks `wait-idle` on the same connection should expect its registrations
to go away with the park.

## Conflicts

Compositor binds win: a global chord equal to a live compositor combo stays
listed as `bound` but `conflicted: true` and never fires (re-check at fire
time, not just bind time, so a reload that adds a compositor bind
immediately shadows). VT-switch binds win the same way. Two registrations
sharing one chord collide the same way at the global level: the reverse
index holds one grab winner per chord, the loser stays listed but
`conflicted: true` and never fires (computed in `snapshot`, so a re-bind
that moves the winner flips both rows). No
`keyboard-shortcuts-inhibit` support exists in scoot today, so global
shortcuts ignore inhibit (documented; a future inhibit implementation
reuses this same ordering point).

## Security

Only the backend may register/receive in the stage-2 design; stage 1
exposes `shortcut-register/unregister/bind` to any same-UID IPC client
(the socket is `0600`, same-user-checked at accept). No sandboxing beyond
that in stage 1: documented as backend-primitives, not a user boundary.
An arbitrary IPC client can already inject keystrokes and screenshots, so
this grants nothing new. `shortcut-bind` can only bind chords to already
registered shortcuts, never run actions directly. `shortcut-bind` and
`shortcut-unregister` check that the id exists but not that the caller
owns it: any same-UID connection can bind or drop any registration. The
portal backend (stage 2) will restrict that to the owning session; until
then this is a same-user primitive, not a sandbox boundary.

## Edge cases

- Key repeat: swallowed (no re-emit).
- Modifiers-only: unrepresentable, refused at parse.
- Release after focus change: intercepted via suppressed set.
- Locked session: a press while locked never fires (forwarded to the lock
  client, no `Activated`). A release that pairs a press intercepted before
  the lock still closes the pair: it is intercepted and emits
  `Deactivated` for the press-time identity, even under lock (the release
  path is routed by what the press decided, never re-gated on the lock).
- VT switch mid-hold: the held key's release still routes through `key()`,
  so it is still intercepted and still emits `Deactivated` for the
  press-time identity; no second `Activated` fires for the hold.
- Registrant dies mid-hold: entry cleared, but a held key's release still
  closes the pair -- it is intercepted and emits `Deactivated` for the
  press-time identity (the suppressed set, never the live table), exactly
  like an unregister or rebind mid-hold.

## Crates / binaries / packaging

No new crate in stage 1 (registry lives in `scoot`; wire types in
`scoot-ipc`; CLI in `scootctl`). Stage 2 adds one binary crate
`scoot-portal-globalshortcuts` (name TBD `scoot-globalshortcuts-portal`):
`[[package]]` entry in `release-plz.toml` with its own `git_tag_name`, a
`crates/<name>` path filter in `ci.yml` classify, `.portal` file
(`UseIn=scoot`, `XDG_CURRENT_DESKTOP=scoot`) installed to
`/usr/share/xdg-desktop-portal/portals/`, D-Bus service
`org.freedesktop.impl.portal.GlobalShortcuts`, Nix module wiring,
deb/rpm/arch entries mirroring `scootnotify`'s unit + activation files.
