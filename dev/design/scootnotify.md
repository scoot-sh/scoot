# scootnotify: design

Design pass, 2026-10-12. This covers what the notification daemon does and how it is
built. The parts it shares with scootlaunch and scootgreet (the `scootui` and
`scoot-dbus` crates, the theme tokens, Stylix, live look swap, CI, start
order and the consolidated decision list) are in [shell-apps.md](shell-apps.md) and
are not repeated here. Tickets: [the epic](../../docs/scootbar/backlog/scootnotify.md).

Spec references are to the Desktop Notifications Specification at
<https://specifications.freedesktop.org/notification/latest/>, which is
**version 1.3** (published 18 August 2024). 1.2 (28 October 2010) is still hosted at
`/notification/1.2/`. The only difference found between them is 1.3's `call.*`
categories. Both hosted renders already carry the `ActivationToken` signal.
Section numbers below are 1.3's.

## Contents

1. [Goals, non-goals and the case for it](#goals-non-goals-and-the-case-for-it)
2. [Spec conformance](#spec-conformance)
3. [Behavior on screen](#behavior-on-screen)
4. [Do not disturb, history, rules](#do-not-disturb-history-rules)
5. [Lock, idle, fullscreen, outputs](#lock-idle-fullscreen-outputs)
6. [The control interface, and what an agent gets](#the-control-interface-and-what-an-agent-gets)
7. [The bar](#the-bar)
8. [Architecture](#architecture)
9. [Security: untrusted content](#security-untrusted-content)
10. [Failure modes](#failure-modes)
11. [Name ownership and coexistence](#name-ownership-and-coexistence)
12. [Stylix and look integration](#stylix-and-look-integration)
13. [Testing and verification](#testing-and-verification)
14. [Docs and packaging](#docs-and-packaging)
15. [Tickets](#tickets)

## Goals, non-goals and the case for it

**Goals.** Own `org.freedesktop.Notifications` and show popups that match the look,
at a fraction of mako's cost and with nothing behind it but itself. Feed the bar
natively. Give agents a stable JSON view of notifications and a way to act on them.

**Non-goals (v1).** These are stated so their absence is a decision, not an oversight:

- Sound. The `sound` capability is not advertised and `sound-file`/`sound-name` are
  ignored. A rule's `exec` hook can play one. There is no audio dependency.
- `body-images` (`<img>` in the body), `icon-multi` (animation), `action-icons`.
- Inline reply. It is not in the spec at all: no capability, no signal (checked
  against 1.3).
- Clickable hyperlinks. `body-hyperlinks` is not advertised in v1, and `<a>` draws as
  underlined text ([why](#security-untrusted-content)).
- On-disk history ([shell-apps.md, Decisions needed](shell-apps.md#decisions-needed), item 8).
- Animation. Cards appear and disappear in one frame. A slide-in is a later,
  measured ticket, never default-on if it costs idle wakeups.
- A notification center panel. History is reached through `msg history` and
  `restore`, and the bar's tooltip shows the count. A panel is a later popup on
  `scootui` widgets.

**What we gain over the incumbent** (lean mako plus `scoot-notify-sync`;
`docs/backlog/resolved/desktop-notifications-done.md`):

- *Memory and processes.* mako is 9.3 MB RSS empty, and the feed is an 11.4 MB
  tree (`busctl --user monitor` piped through a shell loop calling `makoctl` and `jq`,
  `nix/modules/notifications-home.nix:58-198`). scootnotify targets about 3 MB in one process,
  with the feed built in ([budgets](#budgets)).
- *Closure.* Lean mako is 183.4 MiB (53.3 MiB marginal; cairo, pango, X client
  libraries). scootnotify links libc, libm and libgcc_s (the bar family's
  property) plus fonts the profile already carries.
- *Look consistency.* The same tokens, font file, corners and opacity as the bar.
  Today mako gets four color leaves and a critical ring, and its font is mako's default
  `monospace 10` resolved through fontconfig (no `font=` key is generated,
  `notifications-home.nix:206-235`).
- *Bar integration without polling or scripts*: DND and the unread count pushed on
  change, with the module id and icons unchanged.
- *Computer use.* A versioned JSON interface: list, history, invoke an
  action by key, dismiss, and a `subscribe` stream an agent can block on ("wait
  until a notification whose summary contains 'Download complete' appears").
  `makoctl list -j` has some of this, but no event stream and no schema we own.

**What we lose:** pango's shaping, bidi and color emoji (a spike decides how much of
that comes back; [shell-apps.md, Text](shell-apps.md#text-shaping-wrapping-emoji)),
mako's mature criteria syntax and its documentation, and years of real-world
testing against odd clients.

**Go/no-go gate (`notify-prototype`).** Build if the prototype shows all of:

1. idle RSS at most half of lean mako's in the same harness, and 0 wakeups in 60 s;
2. a real-client corpus renders legibly, judged by the maintainer from screenshots side by side with mako. The corpus is `notify-send` (plain, markup, `-i` name, `-i` path, `-h int:value:`), libnotify `image-data`, Firefox, Thunderbird, an Electron app, and a GTK4 app through the portal, with at least one CJK and one Cyrillic body;
3. no client hangs: every call is answered, including oversized ones.

Keep mako, as the paved-path contract allows at no cost, if any of these fails.

## Spec conformance

| Item | Spec | v1 | Detail |
|---|---|---|---|
| Bus name, object, interface | §2 "take the org.freedesktop.Notifications service on the session bus"; object `/org/freedesktop/Notifications` | yes | name ownership: [below](#name-ownership-and-coexistence) |
| D-Bus activation | §2 "can optionally be activated automatically" | yes | an activation file with `SystemdService=scootnotify.service` |
| `GetCapabilities` | §9.1.1 | yes | returns `body`, `body-markup`, `actions`, `icon-static`. Not `persistence`: expired popups emit `NotificationClosed`, and history is a local convenience that does not keep the id alive |
| `Notify` | §9.1.2 | yes | all eight arguments, bounded as in [Security](#security-untrusted-content) |
| `CloseNotification` | §9.1.3 "If the notification no longer exists, an empty D-Bus Error message is sent back" | yes | closes, emits `NotificationClosed(id, 3)`, and an unknown id gets an error with no body |
| `GetServerInformation` | §9.1.4 | yes | `("scootnotify", "scoot", <crate version>, "1.3")` |
| `NotificationClosed` | §9.2.1 reasons 1 to 4; "The ID … is invalidated *before* the signal is sent" | yes | 1 expired, 2 dismissed by the user (click, keybind, `msg dismiss`), 3 `CloseNotification`, 4 evicted by the live cap or the daemon shutting down cleanly |
| `ActionInvoked` | §9.2.2 | yes | on a click on an action button, or on the card body for `default`; `msg invoke` too |
| `ActivationToken` | §9.2.3 "can be emitted before a ActionInvoked signal" | yes, for pointer invocations | a token from `xdg_activation_v1` with the click's button serial and the card's surface, emitted before `ActionInvoked`. Not for `msg invoke`: scoot mints tokens only from a recent key or button serial delivered to the same client (`crates/scoot/src/compositor/activation.rs:180-238`), and an IPC-invoked action has none |
| `replaces_id` | §9.1.2 "must atomically … replace", returns the same id | yes | swapped in place in one commit (no flicker), expiry restarted; an unknown id gets a new notification with a fresh id (the spec is silent; this is what mako and dunst do). Cross-sender policy: [Security](#security-untrusted-content) |
| Ids | §9.1.2 "will not be reused unless a MAXINT number … Servers must make sure not to return zero" | yes | u32 counter, 0 skipped on wrap, live ids skipped; persisted in blocks to `$XDG_RUNTIME_DIR` so a crash restart does not reissue ids clients may still hold ([Failure modes](#failure-modes)) |
| `expire_timeout` | §9.1.2 "-1 … server's settings", "0, never expire", counted "since the display" | yes | -1 means the per-urgency default; 0 means never; a positive value is honored, clamped to 1 h. The clock starts at **first presentation** ([Lock, idle](#lock-idle-fullscreen-outputs)) |
| Urgency | §7 low/normal/critical; "Critical notifications should not automatically expire" | yes | defaults: low 5 s, normal 10 s, critical never (a rule can override; `expire_timeout > 0` on a critical one is honored, since the client asked) |
| Actions | §2 pairs of key and label; `default` key | yes | at most 16 pairs read, 4 shown as buttons; `default` is never a button, but the card body invokes it; `resident` keeps the card after an invoke, otherwise invoking closes it with reason 2 |
| Body markup | §4 `b i u a img`; others "should filter them out" | yes | `b` bold face if configured, else synthetic emboldening; `i` italic face if configured, else upright; `u` underline; `a` underlined accent text, not clickable in v1; `img` dropped, its `alt` kept as text; other tags stripped, their text kept; entities decoded ([Security](#security-untrusted-content)) |
| Icons and images | §5.1 one image: `image-data` > `image-path` > `app_icon` > `icon_data` | yes | one icon slot per card, chosen in that order, plus the deprecated `image_data` and `image_path` (=1.1). `app_icon` and `image-path` are a `file://` URI, an absolute path, or a theme name. PNG only in v1 |
| `image-data` format | §5.2 `(iiibiiay)`, bits 8, channels 3 or 4 by `has_alpha` | yes | validated, downscaled to the icon size at arrival, source dropped |
| Hints | §8 "ignore any hints they do not understand" | partial | `urgency`, `category`, `desktop-entry`, `image-data`, `image-path`, `transient`, `resident`, `suppress-sound` (a no-op), and the de-facto `value` (int 0 to 100, a progress bar; not in the spec, sent by volume and brightness scripts for mako and dunst) are read. `sound-file`, `sound-name`, `x`, `y`, `action-icons` are ignored. `x-*` vendor hints are skipped unread |
| `transient` | §8 "by-pass the server's persistence" | yes | never enters history |
| Categories | §6 | passthrough | used by rules, shown in JSON; no category-specific drawing |
| `desktop-entry` | §8 | yes | rule matching, JSON, and the app's icon when `app_icon` is empty (looked up through the entry's `Icon=` once the launcher's index exists; until then, the entry id is tried as an icon name) |

**Summary length**: §2 says the summary "should generally not be longer than 40
characters … server implementations should word wrap if necessary". The summary
wraps to at most 2 lines and then ellipsizes.

## Behavior on screen

**Card.** Icon (48 logical px, left), summary (bold), a line with
app name and age (`dim`), the body wrapped to at most 5 lines (configurable,
then an ellipsis), an optional progress bar from `value`, then a row of up to 4 action
buttons. A 2 px border colored by urgency, corner radius 8 by default, width
360 logical px. Every size is in logical pixels and drawn at device pixels
(fractional scale through `wp-fractional-scale-v1` + `wp-viewporter`, as the bar).

**Stack.** One `overlay` layer surface anchored to a corner (default top right,
12 px margin, `exclusive_zone = 0` so it sits beside the bar, the layer-shell spec's
own example: "A notification might set its exclusive zone to 0"), holding the whole
stack. It is sized to the stack, with an input region of the cards' union so the gaps click
through. Newest on top, at most 5 visible (configurable), and past that a "+N more" pill.
**The surface and its buffers exist only while there is a card to show.** They are
created on the first, and destroyed (buffers unmapped, glyph cache dropped) when the
last closes, so an idle daemon holds no surface and no pixels.

**Pointer.** A left click on the body invokes `default` if the notification has it,
otherwise dismisses. A click on a button invokes that action. A right click dismisses
(reason 2). A middle click dismisses all. Hover pauses that card's expiry. There is no
keyboard focus (`keyboard_interactivity: none`): keyboard actions go through scoot binds
running `scootnotify msg …` (the desktop profile's three binds, [below](#stylix-and-look-integration)).

**Grouping.** None on screen in v1. `replaces_id` is how a sender updates one card. JSON
and `msg history` group by app.

## Do not disturb, history, rules

**DND** is a mode: `off` or `on`. While on, new notifications skip the screen and go
straight to history and the unread count, except critical ones, which still show
(`dnd-allows-critical = true` by default). Expiry for those that skipped the screen
never starts, so they stay unread until seen. Turning DND off shows nothing
retroactively; `restore` does. The mode survives a reload but not a restart (no disk).
It is set by `msg dnd on|off|toggle`, by the bar's click, or by a scoot bind.

**History** is an in-memory ring: 64 entries by default, at most 256. It holds
app, summary, body (capped), urgency, category, time and an icon *name or path*,
never `image-data` pixels. An entry enters when its card closes for any reason,
or when it skips the screen. `transient` notifications never enter. The unread
count is entries not yet seen on screen. `restore` re-shows the newest history entry
as a new card with a new id. The old id was invalidated by its `NotificationClosed`,
as the spec requires.

**Rules** are `[[rule]]` tables in file order. A rule matches on `app-name`,
`desktop-entry`, `category` (exact or a `class.` prefix), `urgency`, and
`summary-contains` (plain substring; no regex engine). Every matching rule applies,
and later keys win. Actions: `urgency`, `timeout`, `hide` (straight to history),
`history = false`, `output`, `dnd-bypass`, and `exec`, an argv run with the
notification's fields in `SCOOTNOTIFY_*` environment variables. `exec` is the
sound hook and the "do something on a notification" hook. It is spawned with the
bar's `exec` discipline: argv not shell, process group, pidfd reaping, at most 4 running
(`docs/scootbar/backlog/robustness-and-limits.md`, exec bounds). At most 32 rules.

## Lock, idle, fullscreen, outputs

**Lock.** scoot never draws a layer surface while locked: the frame is built from
the lock surfaces alone (`crates/scoot/src/compositor/session_lock.rs:51-58`,
`render/elements.rs:359-376`), and ordinary clients "get no frame callbacks while
locked". So content is never shown over the lock by construction. The daemon needs to know
the lock state only for expiry, and it does not need IPC for that. **A card's
expiry clock starts when the compositor first presents it**: on the
`wl_surface.frame` callback for the commit that first contains it. This is the
spec's own wording ("since the display of the notification"). A notification that arrives
while locked is held with its clock stopped and expires normally after unlock.
mako's measured behavior was the same ("queued popup held, shown on unlock",
`desktop-notifications-done.md`), but there it was accidental and here it is the rule.
It is also compositor-neutral: no scoot IPC (`lock_changed` exists, `msg/events.md`, but is
not needed). Frame callbacks cost one wakeup per presented frame, and only while a
card is newly shown, never at idle.

**Idle.** With `pause-when-idle = 60` (seconds; 0 disables), the daemon binds
`ext_idle_notifier_v1` v2 `get_input_idle_notification`. That one ignores inhibitors
by design (`site/src/content/docs/scoot/protocols.md`, idle section), so a playing
video does not count as presence. While idled, every expiry clock pauses, and on
`resumed` they continue. Someone who walked away comes back to their notifications.
IPC-injected input counts as activity in scoot (`crates/scoot/src/compositor/idle.rs:22-26`),
so an agent driving the VM keeps notifications flowing normally.

**Fullscreen.** `overlay` stays above fullscreen windows in scoot
(`crates/scoot/src/compositor/layer_shell.rs:100-133`, `ABOVE_FULLSCREEN`), so
cards show over a fullscreen video or game by default, as mako on `layer=overlay`
does today. `fullscreen = "hold"` queues non-critical notifications while a fullscreen
window covers the target output and releases them when it leaves.
Detection binds `zwlr_foreign_toplevel_manager_v1` (v2+ for the fullscreen state)
**only at the moment a card would be placed**, reads the burst of handles, and
`stop`s. It never holds the manager at idle: a held manager would wake the daemon
on every terminal's title change. scoot counts these binds live and releases one on `stop`
(`crates/scoot/src/compositor/bind_budget.rs:141,194-201`, at most 8 held per client), so a
bind-and-stop per placement never exhausts the budget. Caveat: fullscreen coverage is per
output (`crates/scoot-core/src/world/fullscreen.rs:41-49`), while `activated` is the single
global focus, so a fullscreen window on an unfocused output is detected by `fullscreen` plus
`output_enter`, not by `activated`.

**Outputs.** `output = "focused"` (default), `"compositor"`, or an output name.
`focused` uses the same on-demand foreign-toplevel snapshot: the
output of the `activated` toplevel, else the compositor's choice (a null
`wl_output`, which scoot resolves to its primary output,
`crates/scoot/src/compositor/layer_shell.rs:228-239`). On hotplug the stack moves to the new
target if its output left, and nothing is lost. With zero outputs, cards wait and their clocks
stay stopped (they were never presented).

## The control interface, and what an agent gets

A control socket at `$XDG_RUNTIME_DIR/scootnotify-<WAYLAND_DISPLAY basename>.sock`
with a `.lock` beside it, line-framed JSON with a protocol version. It reuses
scootbar's framing, claim and bounds through `scootui` `control`: 16 connections with the oldest
non-subscriber evicted, 4 subscribers, 64 KiB request lines, and a write-stall
deadline (`crates/scootbar/src/control/mod.rs:57-65`).

```
scootnotify msg list [--json]            # live notifications
scootnotify msg history [--json]
scootnotify msg dismiss [ID | --all]     # reason 2
scootnotify msg invoke ID [ACTION]       # ACTION defaults to "default"
scootnotify msg restore                  # newest history entry back on screen
scootnotify msg dnd on|off|toggle|status
scootnotify msg reload                   # re-theme in place (shell-apps.md, Live look swap)
scootnotify msg subscribe                # notify / close / invoke / dnd events, one JSON per line
scootnotify msg wait [--app A] [--summary-contains S] [--timeout SECS]
scootnotify msg version
```

`wait` is client-side: it subscribes, reads `list` for the baseline, and exits 0 on the first match,
or 1 on timeout. It costs the daemon nothing more than a subscriber. This is the
computer-use primitive: an agent that started a download can block on its
completion without screenshots and OCR.

The JSON object (versioned, documented on the site) is `id`, `app_name`,
`desktop_entry`, `summary`, `body` (plain text, markup stripped),
`urgency` (`low|normal|critical`), `category`, `actions` (`[{key, label}]`),
`icon` (`{kind: name|path|data, value}`; pixels never), `created`, `shown`
(both CLOCK_REALTIME ms; `shown` null until presented), `expires` (ms or null) and
`output`.

## The bar

The bar's `notifications` module is a `push` module today, fed by
`scoot-notify-sync` (`notifications-home.nix:58-198`, bar half `nix/modules/scootbar.nix:426-438`).
scootnotify replaces the script and keeps the contract, so the swap stays
daemon-only as `desktop-notifications-done.md` promised:

- On every change of DND or of the unread count, and only then, scootnotify connects to
  the bar's control socket (`$XDG_RUNTIME_DIR/scootbar-<display>.sock`) and sends
  `set notifications <payload>`. The payload is the script's exact shape:
  - DND: `{text:"DND n"|"DND", class:"muted", tooltip, icon}`;
  - unread: `{text:"n", class:"urgent"|"normal", tooltip, icon}`, where `urgent`
    means at least one unread critical;
  - idle: `{text:""}`.
  
  Icons come from `programs.scoot.desktop.notifications.bar.icons` (`desktop.nix:492-529`).
- A bar that is not running: the send fails, and the next change retries. When the bar
  starts later, it shows its `placeholder` until the next change. A one-shot re-send
  on the bar's socket appearing is done with an inotify watch on `$XDG_RUNTIME_DIR` that exists
  **only while a send is owed**. A module that is not placed answers an error, and the
  daemon logs guidance once per run, as the script did.
- The click toggles DND through `scootnotify msg dnd toggle` (the bar's
  `on-click` default in `scootbar.nix` changes from `makoctl mode -t do-not-disturb`
  when the daemon is scootnotify).

The rejected alternative is the bar subscribing to scootnotify. It would put a
notifications client in the bar and break the "daemon is the only half that changes"
contract (`scootbar.nix:141-145`).

## Architecture

`crates/scootnotify`, a binary, `#![forbid(unsafe_code)]`, single thread, one
`poll(2)` loop.

```
src/main.rs, cli.rs            daemon / msg / --check / --help
src/config.rs (+tests)         notify.toml, deny_unknown_fields, 64 KiB cap
src/model/                     pure state, no I/O (the scoot-core discipline):
  store.rs                     live notifications, ids, replace, caps, history ring, unread
  expiry.rs                    deadlines from first presentation, pause and resume, the earliest deadline
  rules.rs                     matching and applying
  dnd.rs
src/bus/                       org.freedesktop.Notifications over scoot-dbus
  server.rs                    method routing, name ownership, Introspect, Peer
  notify.rs (+fuzz)            Notify body to a bounded Incoming
  hints.rs, image.rs           image-data validation and downscale
src/markup.rs (+fuzz)          body markup to bounded styled spans
src/view/                      cards to pixels (scootui text, paint, icons)
  layout.rs                    stack geometry, wrap, input region
  render.rs                    per-card damage
src/daemon/                    the poll loop: Wayland, bus, control, timer
src/feed.rs                    the bar push client
```

**Dependencies.** `scootui` (`widgets`, `icons` + `png`, `wayland`, `control`),
`scoot-dbus`, `scootbg-mem`, `wayland-client` 0.31, `wayland-protocols`
0.32 (`staging` for `xdg-activation` and `ext-idle-notify`),
`wayland-protocols-wlr` 0.3 (layer shell, foreign toplevel), `rustix`, `serde`,
`serde_json` and `toml`. These are the bar's own set. Shaping (`rustybuzz`) is optional,
pending the spike. **No new external crate is needed for v1.**

**Event loop sources:** the Wayland fd; the bus fd (or the link's inotify while there is
no bus); the control listener and up to 16 connections; **one** `timerfd` for the
earliest expiry deadline across all cards (re-armed on change, disarmed when nothing
is counting); the idle notification object (on the Wayland fd); the inotify watch on
`$XDG_RUNTIME_DIR` only while a bar send is owed; and pidfds of rule `exec` children.
At idle (no cards, no owed send, no children) the set is the Wayland fd, the bus fd and
the listener, and nothing wakes.

**Rendering.** On arrival a notification is laid out once: markup to spans, spans
shaped (or mapped char by char) into a positioned-glyph list, and the body wrapped.
The result is stored with the notification in fixed-capacity buffers (the 5-line cap
bounds it). A repaint blits those positions, with no layout and no allocation, held by
a `count_allocations == 0` test like the bar's. Damage is per card. A card's arrival
damages the stack below it, since the cards shift. The double buffer is the bar's (`SLOTS = 2`,
never writing a buffer the compositor holds).

### Budgets

Targets, to be ratified by the prototype's measurement. They are not promises until then:

| Row | Target | Incumbent |
|---|---|---|
| Idle RSS, no cards | ≤ 3.0 MB | mako 9.3 MB + feed 11.4 MB |
| Idle PSS | ≤ 1.6 MB | not measured |
| Idle wakeups | 0 in 60 s | 0 (mako) |
| Threads / fds at idle | 1 / ≤ 8 | — |
| One card on screen, scale 1 | ≤ +0.6 MB (two 360×120×4 B buffers are 0.35 MB) | 16.5 MB total for mako with one up |
| `Notify` to commit | ≤ 3 ms on the M2 (debug log stamp; compositor time excluded) | not measured |
| Startup to name owned | ≤ 15 ms | not measured |
| Stripped binary, aarch64 | ≤ 1.6 MB | — |
| Closure | glibc, libgcc_s + the font | 183.4 MiB |

They are measured with `scripts/scootbar-bench`'s runner, generalized, by the M6 method
(`docs/scootbar/backlog/lightest.md`), against lean mako, dunst and swaync in the same harness.
The rows and rules follow the bar's ratchet. Each milestone's table is published in the
epic entry, and kept rather than overwritten, until the app gets its own backlog.

## Security: untrusted content

Any same-user process can call `Notify`. scoot's trust model already puts a
same-uid process inside the boundary (`site/src/content/docs/scoot/protocols.md:12-17`),
so the goal is not isolation. It is that **no content crashes, hangs, floods or
misleads**:

- **Message size.** The per-connection cap is raised to 4 MiB (scootbar's stays at 1 MiB),
  which fits a 1000×1000 RGBA `image-data`. A larger call gets `LimitsExceeded`, never
  silence ([shell-apps.md, scoot-dbus](shell-apps.md#scoot-dbus-library-cratesscoot-dbus)).
- **Field caps** (cut at a char boundary, never refused): `app_name` 128 B,
  `summary` 1 KiB, `body` 8 KiB, an action key 64 B, an action label 128 B, at most 16 action
  pairs read, at most 64 hints read (the rest skipped unread), `category` and
  `desktop-entry` 128 B. Control characters other than `\n` and `\t` are dropped at parse.
- **Markup** is parsed by a bounded tokenizer into at most 64 spans: allowed tags,
  stripped tags, `&amp; &lt; &gt; &quot; &apos;` and numeric `&#N;`/`&#xN;` entities
  (invalid code points become U+FFFD). Unknown entities stay literal. An unclosed or
  malformed tag ends markup parsing, and the rest is plain text. It is never an error, and
  nothing is ever interpreted beyond the five tags. The spec gives no entity rule
  beyond "XML-based"; this choice is documented.
- **Links.** `<a href>` is not clickable in v1. A body link from an arbitrary
  sender that opens a URL on click is an easy phishing surface, and the spec says
  "Hyperlinks cannot function as a replacement for actions". If added later, it
  will be http, https and mailto only, with the URL shown on hover, and opened through the
  launcher's spawn path.
- **`image-data`**: width and height 1 to 2048, `bits_per_sample == 8`, `channels`
  3 or 4 and agreeing with `has_alpha`, `rowstride >= width * channels`, and
  `rowstride * (height - 1) + width * channels <= data.len()` with checked arithmetic.
  Anything else means no image. It is downscaled at arrival to the icon size
  in device pixels (at most 128) through `scootui`'s triangle filter, and the source is dropped.
- **Paths** (`image-path`, `app_icon`): an absolute path, or `file://` with
  percent-decoding. Every other scheme means no image. The file is opened `O_NONBLOCK`,
  must be a regular file of at most 8 MiB, and must be PNG. This is the tray's icon discipline
  (`crates/scootbar/src/modules/tray/theme.rs:38-58`), so a FIFO or `/dev/zero` cannot
  hang or balloon the daemon. Theme names follow the tray's name rules (at most 128 B, no `/`,
  NUL or leading dot).
- **Rate limits.** Per sender (unique bus name): a token bucket of 20 per
  second, burst 40. Past it, `Notify` still returns an id (the method must answer),
  but the notification goes to history unshown. One summary card per 10 s says
  "APP sent N notifications". Globally: at most 128 live notifications. At the cap the
  oldest non-critical one closes with reason 4. A flood therefore costs bounded memory and at
  most one redraw per frame.
- **`replaces_id` across senders.** It is honored when the sender is the same unique name **or**
  the same `app_name` as the original. `notify-send -r ID` runs a new connection
  every time, so it must work (volume and brightness OSD scripts depend on it), and the
  same-uid trust model makes a stricter rule theater. `CloseNotification` takes
  the same rule. A mismatch is treated as a fresh notification, or for close, as an
  unknown id.
- **Fuzz targets**: the `Notify` body reader (valid D-Bus shapes mutated in
  nesting, counts and lengths, as the D-Bus spike's wire finding 4 says), the markup
  parser, and the `image-data` validator. Each has a committed corpus, replayed on stable in
  `cargo test` and run in CI's fuzz job.

## Failure modes

| Failure | Behavior |
|---|---|
| Name already owned (mako, dunst, swaync) | Refuse, naming the owner. Never take over ([below](#name-ownership-and-coexistence)). Exit status 3, which the unit's `RestartPreventExitStatus=3` turns into a stopped unit rather than a restart loop |
| Session bus absent at start, or lost | The `link` lifecycle waits on the socket's directory and redials, re-requesting the name and the match rules. On-screen cards stay, and new ones arrive when the bus is back |
| Compositor gone (Wayland hang-up) | Exit 1. The unit is `PartOf` the session target and goes with the session. A compositor restart restarts it |
| No outputs | Cards wait with their clocks stopped; nothing is lost |
| A crash | `panic = "abort"` (the workspace profile). `Restart=on-failure`, `RestartSec=1`. During the gap, `Notify` calls D-Bus-activate the unit, and the bus holds them until the name is owned. **What was on screen and in history is lost**, said plainly in the docs: the cost of not writing notifications to disk. The id counter is reserved in blocks of 1024 in `$XDG_RUNTIME_DIR/scootnotify-<display>.ids`, so a restart continues past every id it might have issued and a client's stale `replaces_id` cannot hit a stranger's notification |
| Bad config at start | Exit 2 with the error (`--check` shows the same). Under the unit it restarts and fails again. The Nix module runs `--check` at build time, as scootbar's tests do, so that path needs a hand-edited file |
| Bad config on reload | Keep the running config, and answer the error to `msg reload` |
| A font that will not load | At start, fall back down the chain to the well-known list (the bar's `font::text`, `crates/scootbar/src/font.rs:166`). On reload, keep the running font |
| Bar socket absent | Retry on the next change ([The bar](#the-bar)) |

## Name ownership and coexistence

- `RequestName("org.freedesktop.Notifications", DO_NOT_QUEUE)`, without
  `ALLOW_REPLACEMENT` (so no one can take the name from scootnotify) and without
  `REPLACE_EXISTING` (so it never takes the name from anyone). The flag values are already
  in the client (`crates/scootbar/src/dbus/proto.rs:99-112`). This is the same request
  pattern the tray's watcher uses (`modules/tray/watcher.rs:1486-1505`).
- If the reply is not `PRIMARY_OWNER`: `GetNameOwner`, then
  `GetConnectionUnixProcessID`, then `/proc/PID/comm`, then print
  `org.freedesktop.Notifications is owned by mako (pid 1234); scootnotify will not take it over. Stop that daemon first (systemctl --user stop mako) and remove its D-Bus activation file, or it will come back on the next notification.`
  and exit 3.
- **The activation race.** Each daemon ships a `share/dbus-1/services/org.freedesktop.Notifications.service`.
  With two on the bus's search path, the bus activates one of them on the next
  `Notify`, and which one is not something to rely on (search-path order between
  `XDG_DATA_HOME` and `XDG_DATA_DIRS` is daemon behavior this plan did not verify).
  The desktop profile therefore never installs both: with `daemon = "scootnotify"` the mako
  package is not in `home.packages`, and with `"mako"` scootnotify's is not. The
  install docs for the non-Nix path say to remove or mask the other file.

## Stylix and look integration

The shared rules (tokens, precedence, fonts, opacity, live reload) are in
[shell-apps.md](shell-apps.md#theme-tokens-stylix-and-looks). Per element:

| Element | Token | Stylix | Look |
|---|---|---|---|
| Card fill | `background` with alpha | `base00` × `opacity.popups` | `barColors.background`, opaque |
| Summary, body | `foreground` | `base05` | `barColors.foreground` |
| App name, age, "+N more" | `dim` | `base03` | `barColors.dim` |
| Border, normal | `border` | `base0D` | `appearance.focus_ring_active_color` (mako's today) |
| Border, low | `dim` | `base03` | `barColors.dim` |
| Border, critical | `urgent` | `base08` | `barColors.urgent` (mako's critical ring today) |
| Progress bar | `selection` | `base02` (Stylix's mako `progress-color`) | `barColors.accent` (mako's today) |
| Action button outline / hovered fill | `accent` / `hover` | `base0D` / `base0D` | as the bar |
| Link text | `accent`, underlined | `base0D` | `barColors.accent` |
| Font | — | `fonts.sansSerif`, `sizes.popups` (pt → px) | `fonts.ui` |

**Nix.** `programs.scootnotify` with `enable`, `package`, `features`,
`settings` (TOML), `stylix.enable` and `systemd.enable`. On Home Manager:
`xdg.configFile."scoot/notify.toml"`; a user unit with `Type=dbus`,
`BusName=org.freedesktop.Notifications`, `ExecStart=scootnotify daemon`,
`ExecReload=scootnotify msg reload`, `X-Reload-Triggers` on the config file
(not restart; [shell-apps.md](shell-apps.md#live-look-swap)), `Restart=on-failure`,
`RestartPreventExitStatus=3`, `ExecCondition` on `WAYLAND_DISPLAY` (mako's unit has
the same, `notifications-home.nix:318-353`), and `PartOf`/`After`/`WantedBy` the session target
by scootbar's rule (`scoot-session.target` under the profile). The package's D-Bus
service file is on the user's data path. NixOS: `/etc/scoot/notify.toml`,
`services.dbus.packages`, and the same user unit.

**The profile switch.** `programs.scoot.desktop.notifications.daemon` widens to
`[ "mako" "scootnotify" ]`. With `"scootnotify"`:

- `programs.scootnotify.enable = mkDefault true`, with the look rungs gated on
  `theme.targets.notifications.enable`;
- mako and `scoot-notify-sync` are not installed;
- the three binds (`desktop.nix:101-115`, `keys-home.nix:337-339`) run
  `scootnotify msg dismiss`, `msg dnd toggle` and `msg restore`;
- the bar's `notifications` push table keeps its id, icons and placement, and its
  `on-click` becomes `scootnotify msg dnd toggle`;
- `desktop.notifications.settings` is mako's flat `key=value` and does not carry
  over. An eval warning names `programs.scootnotify.settings` when it is non-empty with
  the new daemon. This is the one option that does not survive the swap, and the docs say so.

**Tests** (in `nix/scootbar-tests.nix`'s style, a new `nix/scootnotify-tests.nix`):

- no Stylix gives the plain defaults;
- Stylix gives exactly the table above, `font-size` 13 from 10 pt popups, and a font file;
- each Stylix leaf yields to a user value, looped over every token;
- the look rung applies and Stylix beats it per leaf;
- `opacity.popups` 0.9 becomes the documented alpha;
- the rendered TOML round-trips;
- the real binary's `--check` accepts every rendered file and refuses a typo'd key.

The Stylix stub grows to declare `stylix.fonts.sizes.popups` and
`stylix.opacity.popups`. **What the stub cannot vouch for**, as scootbar's header
says: a rename upstream passes the stub and misses on a real Stylix. The real-Stylix
script (`scripts/scootbar-stylix-test.sh`, not in CI) gains a scootnotify block.

## Testing and verification

- **Unit**: the model (ids, replace, expiry from presentation, pause and resume, caps, rate
  limits, rules, DND, history), the markup parser, the image validator, and the payload builder.
  Every one is pure.
- **Fuzz**: Notify body, markup, image-data ([Security](#security-untrusted-content)).
- **Bus tests against a real `dbus-daemon`** (scootbar's `dbus/daemon_tests.rs` pattern):
  name ownership and refusal naming the owner, every method, both signals with the right reasons,
  `LimitsExceeded` for a 5 MiB call, an id never reused across a restart, and
  the D-Bus activation file activating the unit. Driven by an independent
  marshaller (`jeepney` or `busctl`, as the M6 benches did), and by real `notify-send`.
- **Headless scoot harness**: send through `notify-send`; screenshot through scoot IPC;
  assert pixels (seven-segment test font); click through `scoot msg pointer`;
  assert `ActionInvoked` and `ActivationToken` order; lock with the
  harness's `tests/lockclient`; assert nothing drawn and the clock not started; unlock and
  assert it shows and then expires; hotplug an output (`--outputs 2` and `output-power`).
- **Goldens**: low, normal and critical cards, with an icon, with actions, with progress, the
  stack with "+N more", per look (five) and with a Stylix palette.
- **Ratchet**: the [budgets](#budgets), against lean mako, dunst and swaync, with a flood
  row (10,000 `Notify` in 10 s from one sender: CPU, RSS before and after, wakeups) as
  the bar's runaway rows do.
- **M2 real login**: above a fullscreen window, nothing over the lock, the DND toggle from
  the bar, an action that focuses a running single-instance app through
  `ActivationToken` (Firefox or foot `--server`), and a multi-day soak for RSS and fds.

## Docs and packaging

- Site `/scootnotify/` (per `site/OUTLINE.md`'s Coming apps): overview with the Nix table
  (option, type, default, reload column, as `/scootbar/`), configure (every key: type,
  default, example, reload), rules, `msg` reference with the JSON schema, theming, and
  troubleshooting with `> **Symptom:**` boxes ("owned by mako", "never expires",
  "nothing over fullscreen", "no icon", "Arabic text draws wrong" if shaping is off).
  Sidebar group and `customSets` entry in `site/astro.config.mjs`.
- `desktop/index.md` Notifications section: the daemon choice and the swap.
  `start/compose.mdx`: drop "(coming)". `agents/`: the `wait` primitive.
- Packages: deb, rpm and Arch (with the unit, the activation file and an example config);
  `install.sh --notify`; flake `packages.scootnotify`, `homeModules`, `nixosModules`.
- CI: the classify arm and job in the PR that creates the crate (`notify-server`).

## Tickets

Each is one implementer's PR. Each states which goal it serves, and each needs the
full per-feature cycle with `scoot-reviewer`. Full text, acceptance and evidence are in the
entries.

| Slug | Serves | Blocked by |
|---|---|---|
| [`scootnotify`](../../docs/scootbar/backlog/scootnotify.md) (epic) | daily-drive, computer use | its children |
| [`notify-prototype`](../../docs/scootbar/backlog/notify-prototype.md): the go/no-go gate | daily-drive | `scootui-extract-draw` |
| [`notify-server`](../../docs/scootbar/backlog/notify-server.md): the D-Bus server and model, headless | both | `notify-prototype`, `scoot-dbus-extract` |
| [`notify-popups`](../../docs/scootbar/backlog/notify-popups.md): cards, stack, lock and idle rules, outputs | daily-drive | `notify-server`, `scootui-extract-wayland` |
| [`notify-control`](../../docs/scootbar/backlog/notify-control.md): `msg`, JSON, subscribe, wait, DND, history, rules, fullscreen hold | computer use, daily-drive | `notify-popups` |
| [`notify-bar-feed`](../../docs/scootbar/backlog/notify-bar-feed.md): native push to the bar | daily-drive | `notify-control` |
| [`notify-nix`](../../docs/scootbar/backlog/notify-nix.md): modules, Stylix, look, profile switch (opt-in) | daily-drive | `notify-bar-feed` |
| [`notify-packaging`](../../docs/scootbar/backlog/notify-packaging.md): deb, rpm, Arch, install.sh | daily-drive | `notify-nix` |
| [`notify-default-flip`](../../docs/scootbar/backlog/notify-default-flip.md): make it the profile default | daily-drive | `notify-packaging` |
