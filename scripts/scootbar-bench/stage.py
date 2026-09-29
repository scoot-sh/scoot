"""A compositor set up as M0's baselines had it: one 1920×1080 output, a
private session bus, fontconfig seeing DejaVu only, and two terminal
windows on two workspaces, so there is something to switch between.

The compositor itself is ``scripts/scootbg-bench/session.py``'s: headless
scoot or sway (pixman), each run with its own scratch directory as
``XDG_RUNTIME_DIR``, home, config, cache and state, and with
``WAYLAND_DEBUG=server`` so every buffer commit is timed where the
compositor received it (``commits.py``), whatever Wayland library the bar
uses.
"""

import json
import os
import signal
import subprocess
import time

import bars
from session import Session

PATIENCE = 20.0


def wait_until(predicate, timeout, step=0.02):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(step)
    return predicate()


def count_windows(reply, kind):
    """How many toplevel windows an IPC reply lists: scoot's ``windows``
    (a list, or an object holding one), or sway's ``get_tree`` (nodes with
    an ``app_id`` or X11 ``window``, at any depth)."""
    if kind == "scoot":
        if isinstance(reply, dict):
            reply = next((v for v in reply.values() if isinstance(v, list)), [])
        return len(reply) if isinstance(reply, list) else 0

    def walk(node):
        n = 1 if node.get("app_id") or node.get("window") else 0
        for child in node.get("nodes", []) + node.get("floating_nodes", []):
            n += walk(child)
        return n

    return walk(reply) if isinstance(reply, dict) else 0


class Stage:
    def __init__(self, compositor, bins, tools, keep=False):
        """``bins``: ``scoot`` and ``scootctl``, or ``sway`` and
        ``swaymsg``; ``tools``: ``foot``, ``dbus`` (the package) and
        ``fonts`` (a directory of font files)."""
        self.kind = compositor
        self.bins = bins
        self.tools = tools
        self.sess = Session(compositor, bins, 1920, 1080, 1, keep=keep)
        self.bus = None
        self.windows = []
        self.env = None

    def __enter__(self):
        try:
            return self.start()
        except BaseException:
            self.close()
            raise

    def __exit__(self, *exc):
        self.close()

    def start(self):
        self.sess.start()
        fonts = bars.fontconfig(self.sess.dir, self.tools["fonts"])
        bus = self.sess.path("bus")
        dbus = self.tools["dbus"]
        self.bus = subprocess.Popen(
            [os.path.join(dbus, "bin", "dbus-daemon"), "--nofork", "--nopidfile",
             "--config-file", os.path.join(dbus, "share", "dbus-1", "session.conf"),
             f"--address=unix:path={bus}"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        if not wait_until(lambda: os.path.exists(bus), PATIENCE):
            raise RuntimeError("the session bus did not come up")
        self.env = self.sess.client_env({
            "FONTCONFIG_FILE": fonts,
            "DBUS_SESSION_BUS_ADDRESS": f"unix:path={bus}",
            "LANG": "C.UTF-8",
        })
        self.window()
        self.switch(1)
        self.window()
        self.switch(0)
        return self

    def ipc(self, *args):
        if self.kind == "scoot":
            argv = [self.bins["scootctl"], *args]
            env = dict(os.environ, SCOOT_SOCKET=self.sess.ipc, XDG_RUNTIME_DIR=self.sess.dir)
        else:
            argv = [self.bins["swaymsg"], "-s", self.sess.ipc, *args]
            env = dict(os.environ)
        return subprocess.run(argv, env=env, capture_output=True, text=True, timeout=30)

    def switch_argv(self, index):
        """The command that shows workspace ``index`` (0-based), as M0 ran
        it: one short-lived client per switch."""
        if self.kind == "scoot":
            return [self.bins["scootctl"], "action", "focus-workspace-index", str(index)]
        return [self.bins["swaymsg"], "-s", self.sess.ipc, "workspace", "number", str(index + 1)]

    def switch(self, index):
        env = dict(os.environ, SCOOT_SOCKET=self.sess.ipc or "", XDG_RUNTIME_DIR=self.sess.dir)
        subprocess.run(self.switch_argv(index), env=env, capture_output=True, timeout=30, check=True)

    def window_count(self):
        r = self.ipc("windows") if self.kind == "scoot" else self.ipc("-t", "get_tree", "-r")
        try:
            return count_windows(json.loads(r.stdout), self.kind)
        except json.JSONDecodeError:
            return 0

    def window(self):
        before = self.window_count()
        log = open(self.sess.path(f"foot{len(self.windows)}.log"), "wb")
        self.windows.append(subprocess.Popen(
            [os.path.join(self.tools["foot"], "bin", "foot")],
            env=self.env, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
        ))
        log.close()
        if not wait_until(lambda: self.window_count() > before, PATIENCE):
            raise RuntimeError(f"foot did not map a window on {self.kind}")

    def close(self):
        for p in self.windows + ([self.bus] if self.bus else []):
            if p.poll() is None:
                p.send_signal(signal.SIGTERM)
                try:
                    p.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait()
        self.sess.close()
