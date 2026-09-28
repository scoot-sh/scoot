"""How each daemon is started, changed live, stopped and weighed.

Each is run the way its own documentation says, with the settings that make
the work comparable, all recorded here:

- scootbg: ``daemon``, then ``set`` (``fill``, Lanczos3: the defaults).
- awww: ``awww-daemon``, then ``awww img -t none`` (``crop`` and Lanczos3,
  its defaults, so the same work as ``fill``) or ``awww clear RRGGBB``.
  Its restore is the daemon running ``awww img`` itself from ``PATH``.
- wpaperd: a config file naming the image, ``mode = "center"`` (its
  cover-and-crop mode), transitions off; live change by ``wpaperctl
  set-wallpaper``. No colors. Renders with OpenGL ES (Mesa's software
  EGL here).
- swaybg: ``-o '*' -i IMAGE -m fill`` or ``-c RRGGBB``. No live change, no
  restore.
- wbg: ``--stretch IMAGE`` (its only way to cover the output: without it
  it letterboxes). No colors, no live change, no restore.
- hyprpaper: a config file naming the image, ``fit_mode = cover``. Its live
  change is Hyprland-only (its IPC turns off without
  ``HYPRLAND_INSTANCE_SIGNATURE``); no colors.

nixpkgs wraps swaybg in a small C launcher that sets
``GDK_PIXBUF_MODULE_FILE``, which gdk-pixbuf needs to find its loaders, and
awww in bash scripts that only prepend procps to ``PATH``. swaybg runs
through its launcher (it ``exec``s, so the measured process is swaybg);
awww's unwrapped binaries run directly with the same ``PATH``, so no bash
start-up is charged to awww.
"""

import os
import re
import signal
import subprocess
import time

COLOR = "#1e1e2e"


class Wall:
    def __init__(self, kind, value):
        assert kind in ("image", "color")
        self.kind = kind
        self.value = value

    def __repr__(self):
        return f"{self.kind}:{os.path.basename(self.value)}"


class Daemon:
    name = ""
    informational = False  # a variant shown for context, outside the gate
    nix_attr = None
    needs_egl = False
    colors = True
    live_image = True
    live_color = True
    restore_image = True
    restore_color = True

    def __init__(self, store=None, bins=None):
        self.store = store  # the package's store path, when there is one
        self.bins = bins or {}

    # What to weigh for the Size row: the package's ELF executables
    # (nixpkgs' launchers left out).
    def elf_binaries(self):
        raise NotImplementedError

    def socket(self, sess):
        return None

    def extra_env(self, sess):
        return {}

    def daemon_argv(self, sess, wall):
        """The daemon's command line; ``wall`` is what it should show from
        the start when it takes that on its command line or config, else
        ``None`` for a daemon started empty (then ``start_clients``)."""
        raise NotImplementedError

    def start_clients(self, sess, wall):
        return []

    def set_argv(self, sess, wall):
        return None

    def stop(self, sess, run):
        """Stops the daemon the way a user would, so it saves what it saves."""
        run.signal(signal.SIGTERM)

    def supports(self, row, wall_kind):
        if wall_kind == "color" and not self.colors:
            return False
        if row == "set":
            return self.live_image if wall_kind == "image" else self.live_color
        if row == "restore":
            return self.restore_image if wall_kind == "image" else self.restore_color
        return True


class Scootbg(Daemon):
    name = "scootbg"

    def elf_binaries(self):
        return [self.bins["scootbg"]]

    def socket(self, sess):
        return sess.path(f"scootbg-{sess.display}.sock")

    def daemon_argv(self, sess, wall):
        return [self.bins["scootbg"], "daemon"]

    def start_clients(self, sess, wall):
        return [self.set_argv(sess, wall)] if wall else []

    def set_argv(self, sess, wall):
        return [self.bins["scootbg"], "set", wall.value]

    def stop(self, sess, run):
        # `kill` waits for a state write under way before the daemon exits.
        run.client([self.bins["scootbg"], "kill"]).wait_done()


class ScootbgBilinear(Scootbg):
    """scootbg with ``--filter bilinear``: the scaling work wbg does
    (pixman's ``PIXMAN_FILTER_BEST`` is bilinear), for the image rows only.
    Shown beside the others for context, never in the gate: the gate
    compares defaults."""

    name = "scootbg-bilinear"
    informational = True

    def set_argv(self, sess, wall):
        return [self.bins["scootbg"], "set", "--filter", "bilinear", wall.value]

    def supports(self, row, wall_kind):
        return wall_kind == "image" and row in ("startup", "set")


class Awww(Daemon):
    name = "awww"
    nix_attr = "awww"
    restore_color = False  # `clear` is not cached: a restart restores the last image

    def elf_binaries(self):
        return [
            os.path.join(self.store, "bin", ".awww-daemon-wrapped"),
            os.path.join(self.store, "bin", ".awww-wrapped"),
        ]

    def _wrapper_path_prefix(self):
        with open(os.path.join(self.store, "bin", "awww-daemon")) as f:
            m = re.search(r"^PATH='([^']*)'\$PATH", f.read(), re.M)
        return m.group(1) if m else ""

    def extra_env(self, sess):
        # The daemon restores by running `awww img ...` through `sh` from
        # PATH; point `awww` at the unwrapped client.
        shim = sess.path("bin")
        os.makedirs(shim, exist_ok=True)
        link = os.path.join(shim, "awww")
        if not os.path.lexists(link):
            os.symlink(os.path.join(self.store, "bin", ".awww-wrapped"), link)
        prefix = self._wrapper_path_prefix()
        return {"PATH": ":".join(p for p in (shim, prefix, os.environ.get("PATH", "")) if p)}

    def socket(self, sess):
        return sess.path(f"{sess.display}-awww-daemon.sock")

    def daemon_argv(self, sess, wall):
        return [os.path.join(self.store, "bin", ".awww-daemon-wrapped")]

    def start_clients(self, sess, wall):
        return [self.set_argv(sess, wall)] if wall else []

    def set_argv(self, sess, wall):
        client = os.path.join(self.store, "bin", ".awww-wrapped")
        if wall.kind == "image":
            return [client, "img", "--transition-type", "none", wall.value]
        return [client, "clear", wall.value.lstrip("#")]

    def stop(self, sess, run):
        run.client([os.path.join(self.store, "bin", ".awww-wrapped"), "kill"]).wait_done()


class Wpaperd(Daemon):
    name = "wpaperd"
    nix_attr = "wpaperd"
    needs_egl = True
    colors = False
    restore_image = False  # measured: a restart shows the config's image, not a set-wallpaper choice

    def elf_binaries(self):
        return [os.path.join(self.store, "bin", n) for n in ("wpaperd", "wpaperctl")]

    def socket(self, sess):
        return sess.path("wpaperd.sock")

    def daemon_argv(self, sess, wall):
        config = sess.path("wpaperd.toml")
        with open(config, "w") as f:
            f.write(
                "[default]\nmode = \"center\"\ntransition-time = 0\n"
                "initial-transition = false\n\n"
                f"[any]\npath = \"{wall.value}\"\n"
            )
        return [os.path.join(self.store, "bin", "wpaperd"), "-c", config]

    def set_argv(self, sess, wall):
        return [os.path.join(self.store, "bin", "wpaperctl"), "set-wallpaper", wall.value]


class Swaybg(Daemon):
    name = "swaybg"
    nix_attr = "swaybg"
    live_image = live_color = restore_image = restore_color = False

    def elf_binaries(self):
        return [os.path.join(self.store, "bin", ".swaybg-wrapped")]

    def daemon_argv(self, sess, wall):
        argv = [os.path.join(self.store, "bin", "swaybg"), "-o", "*"]
        if wall.kind == "image":
            return argv + ["-i", wall.value, "-m", "fill"]
        return argv + ["-c", wall.value.lstrip("#")]


class Wbg(Daemon):
    name = "wbg"
    nix_attr = "wbg"
    colors = False
    live_image = live_color = restore_image = restore_color = False

    def elf_binaries(self):
        return [os.path.join(self.store, "bin", "wbg")]

    def daemon_argv(self, sess, wall):
        return [os.path.join(self.store, "bin", "wbg"), "--stretch", wall.value]


class Hyprpaper(Daemon):
    name = "hyprpaper"
    nix_attr = "hyprpaper"
    needs_egl = True
    colors = False
    live_image = live_color = False  # IPC is Hyprland-only
    restore_image = restore_color = False

    def elf_binaries(self):
        return [os.path.join(self.store, "bin", "hyprpaper")]

    def daemon_argv(self, sess, wall):
        config = sess.path("hyprpaper.conf")
        with open(config, "w") as f:
            f.write(
                "splash = false\nwallpaper {\n    monitor =\n"
                f"    path = {wall.value}\n    fit_mode = cover\n}}\n"
            )
        return [os.path.join(self.store, "bin", "hyprpaper"), "-c", config]


COMPETITORS = [Awww, Hyprpaper, Swaybg, Wbg, Wpaperd]
ALL = [Scootbg, ScootbgBilinear] + COMPETITORS


def nix_resolve(attr, flake_dir):
    """(store path, version) of ``nixpkgs#attr`` at the flake's pinned
    nixpkgs, built (or fetched) in the sandbox."""
    common = ["--inputs-from", flake_dir]
    env = dict(os.environ)
    ca = "/root/.ccr/ca-bundle.crt"
    extra = []
    if os.path.exists(ca):
        env.setdefault("NIX_GIT_SSL_CAINFO", ca)
        extra = ["--option", "extra-sandbox-paths", ca]
    out = subprocess.run(
        ["nix", "build", "--no-link", "--print-out-paths", *common, f"nixpkgs#{attr}",
         "--option", "sandbox", "true", *extra],
        env=env, capture_output=True, text=True, check=True,
    ).stdout.strip().splitlines()[-1]
    version = subprocess.run(
        ["nix", "eval", "--raw", *common, f"nixpkgs#{attr}.version"],
        env=env, capture_output=True, text=True, check=True,
    ).stdout.strip()
    return out, version


def wait_until(predicate, timeout, step=0.001):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(step)
    return predicate()
