---
title: CLI reference
description: "Every scootnotify command and flag that ships today: daemon, --check, --config, and the notify.toml keys."
---

Every `scootnotify` command and flag that ships today, and how it
behaves at the edges. What scootnotify is for is in
[Overview](./index.md). Later tickets add `msg` (list, dismiss,
invoke, history, DND, subscribe, wait), popups and rules; this page
grows with them.

## Commands

```sh
scootnotify daemon                          # own the name and serve until killed
scootnotify daemon --check                  # validate the config, exit 0 or 1
scootnotify daemon --config PATH            # read PATH instead of the default
scootnotify --help                          # print help
scootnotify --version                       # print the version
```

Only `daemon` serves yet; anything else is a loud error (exit 2,
naming the command). `--help` and `--version` win wherever they
stand. Exit statuses: 0 done, 1 failed, 2 misused, 3 the name is
already owned (the unit stops instead of restart-looping).

A taken name refuses with the owner named:

```sh
scootnotify daemon
# org.freedesktop.Notifications is owned by mako (pid 1234);
# scootnotify will not take it over. Stop that daemon first and
# remove its D-Bus activation file, or it will come back on the
# next notification.
# exit 3
```

## Configure

`$XDG_CONFIG_HOME/scoot/notify.toml` beside the bar's (`--config`
overrides it, `--check` validates it). Every key is optional; a
missing file is the built-in defaults.

```toml
timeout_low_ms = 5000        # low-urgency expiry for expire_timeout = -1
timeout_normal_ms = 10000    # normal-urgency expiry for expire_timeout = -1
timeout_max_ms = 3600000     # a positive expire_timeout is honored up to this
max_live = 128               # live cards past this evict oldest-first
rate_per_sec = 20            # per-sender refill, notifications per second
rate_burst = 40              # per-sender burst before cards count unshown
```

Unknown keys are refused (a typo that silently did nothing would be
worse than an error), and out-of-range numbers name their key. The
file is capped at 64 KiB. A bad file at daemon start runs on the
built-in defaults and reports the error; `--check` exits 1 on it.

```sh
scootnotify daemon --check --config ./notify.toml
# ./notify.toml: valid
```

## Troubleshooting

> **Symptom:** the daemon exits 3 saying the name is owned.

Another notification daemon (mako, dunst, swaync) holds
`org.freedesktop.Notifications`, or its D-Bus activation file brings
it back on the next notification. Stop it first
(`systemctl --user stop mako`) and remove or mask its activation
file; scootnotify never takes the name from anyone.

> **Symptom:** notifications never expire.

A critical urgency never expires unless the sender passes a positive
`expire_timeout`, and `expire_timeout = 0` means never. Check the
sender's urgency and timeout before the config: the defaults only
apply to `expire_timeout = -1` (low 5 s, normal 10 s).

> **Symptom:** an app's cards stop appearing under load.

The sender tripped its rate limit (20 per second, burst 40): its
`Notify` calls still get ids, but the cards count unshown instead of
showing. A `replaces_id` update never trips it. Wait a second and
retry, or pace the sender.

> **Symptom:** no icon on the card.

`image-data` must be 8-bit, 3 or 4 channels agreeing with its alpha
flag, sides 1 to 2048, with pixels covering every row — anything
else means no image. `image-path` and `app_icon` take a `file://`
URI, an absolute path, or a theme name; every other scheme means no
image.
