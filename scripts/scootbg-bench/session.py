"""A headless compositor for one measurement, with its trace.

scoot: ``scoot --headless --width W --height H --outputs N``.
sway: wlroots' headless backend with the pixman renderer and N outputs,
each given the mode, as ``crates/scootbg/tests/common`` starts it.

Both run with ``WAYLAND_DEBUG=server`` and their stderr in ``trace.log``
(see ``commits.py``). Each session has its own short scratch directory as
``XDG_RUNTIME_DIR`` and as the daemons' home, config, cache and state
directories, so no run sees another's (or the user's) state, and what a
daemon writes to disk can be counted afterwards.
"""

import os
import shutil
import signal
import subprocess
import tempfile
import time

from commits import Trace

PATIENCE = 20.0


def _is_socket(path):
    try:
        import stat

        return stat.S_ISSOCK(os.stat(path).st_mode)
    except FileNotFoundError:
        return False


_SO_ACCEPTCON = 0x10000


def _listening_by_diag(path):
    """Whether the socket file at ``path`` is a listening unix socket, asked
    of the kernel's ``sock_diag`` (``UNIX_DIAG``): it reports each listener's
    filesystem inode and device (``UDIAG_SHOW_VFS``), which is how the
    renamed socket file is found. ``/proc/net/unix`` cannot do this: its
    inode column is the socket's own number, not the file's, and its name
    column is the name the socket was *bound* at."""
    import socket
    import struct

    info = os.stat(path)
    major, minor = os.major(info.st_dev), os.minor(info.st_dev)
    dev = (minor & 0xFF) | (major << 8) | ((minor & ~0xFF) << 12)  # new_encode_dev
    af_unix, tcp_listen, show_vfs, unix_diag_vfs = 1, 10, 0x2, 1  # UDIAG_SHOW_VFS, UNIX_DIAG_VFS
    request = struct.pack("=BBHIIIII", af_unix, 0, 0, 1 << tcp_listen, 0, show_vfs, 0xFFFFFFFF, 0xFFFFFFFF)
    header = struct.pack("=IHHII", 16 + len(request), 20, 0x301, 1, 0)  # SOCK_DIAG_BY_FAMILY, REQUEST|DUMP
    nl = socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, 4)  # NETLINK_SOCK_DIAG
    try:
        nl.settimeout(2.0)
        nl.send(header + request)
        while True:
            data = nl.recv(65536)
            at = 0
            while at + 16 <= len(data):
                length, kind = struct.unpack_from("=IH", data, at)
                if kind == 3 or length < 16:  # NLMSG_DONE
                    return False
                if kind == 2:  # NLMSG_ERROR
                    raise OSError("sock_diag refused")
                end = at + length
                cursor = at + 16 + 16  # past nlmsghdr and unix_diag_msg
                while cursor + 4 <= end:
                    alen, atype = struct.unpack_from("=HH", data, cursor)
                    if alen < 4:
                        break
                    if atype == unix_diag_vfs and alen >= 12:
                        vino, vdev = struct.unpack_from("=II", data, cursor + 4)
                        if vino == info.st_ino and vdev == dev:
                            return True
                    cursor += (alen + 3) & ~3
                at += (length + 3) & ~3
    finally:
        nl.close()


def listening(path):
    """True once the socket at ``path`` is listening, learned without
    connecting: a probe that connects and hangs up kills ``awww-daemon``
    (SIGPIPE), and would be a request the daemon has to serve. scootbg binds
    in a staging directory and renames the socket onto ``path``, so the
    listener is matched by the file's device and inode through
    ``sock_diag``; if the kernel has no ``UNIX_DIAG``, ``/proc/net/unix`` is
    read by name, which still finds a daemon that binds at ``path`` itself
    (awww, wpaperd)."""
    try:
        import stat

        if not stat.S_ISSOCK(os.stat(path).st_mode):
            return False
    except OSError:
        return False
    try:
        return _listening_by_diag(path)
    except OSError:
        pass
    try:
        with open("/proc/net/unix") as f:
            next(f)
            for line in f:
                parts = line.split()
                if len(parts) >= 8 and parts[7] == path and int(parts[3], 16) & _SO_ACCEPTCON:
                    return True
    except (OSError, ValueError):
        pass
    return False


class Session:
    def __init__(self, compositor, bins, width, height, outputs, keep=False):
        self.kind = compositor
        self.bins = bins
        self.width = width
        self.height = height
        self.outputs = outputs
        self.keep = keep
        # Short: Unix socket paths inside must fit 107 bytes.
        self.dir = tempfile.mkdtemp(prefix="sbb", dir="/tmp")
        os.chmod(self.dir, 0o700)
        for sub in ("home", "config", "cache", "state"):
            os.mkdir(os.path.join(self.dir, sub))
        self.trace_path = os.path.join(self.dir, "trace.log")
        self.proc = None
        self.exe = None
        self.display = None
        self.ipc = None
        self.trace = Trace(self.trace_path)

    # -- lifecycle ---------------------------------------------------------

    def start(self):
        env = dict(os.environ)
        for var in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "SWAYSOCK", "DISPLAY", "SCOOT_SOCKET"):
            env.pop(var, None)
        env["XDG_RUNTIME_DIR"] = self.dir
        env["WAYLAND_DEBUG"] = "server"
        log = open(self.trace_path, "wb")
        if self.kind == "scoot":
            config = os.path.join(self.dir, "scoot.toml")
            open(config, "w").close()
            self.ipc = os.path.join(self.dir, "s.sock")
            argv = [
                self.bins["scoot"],
                "--headless",
                "--width",
                str(self.width),
                "--height",
                str(self.height),
                "--outputs",
                str(self.outputs),
                "--socket",
                self.ipc,
                "--config",
                config,
            ]
        elif self.kind == "sway":
            config = os.path.join(self.dir, "sway.conf")
            with open(config, "w") as f:
                f.write("swaybg_command -\nxwayland disable\n")
                for i in range(1, self.outputs + 1):
                    f.write(f"output HEADLESS-{i} mode {self.width}x{self.height}\n")
            env.update(
                {
                    "WLR_BACKENDS": "headless",
                    "WLR_RENDERER": "pixman",
                    "WLR_LIBINPUT_NO_DEVICES": "1",
                    "WLR_HEADLESS_OUTPUTS": str(self.outputs),
                    # nixpkgs' wrapper starts sway under dbus-run-session
                    # without one; nothing needs to listen.
                    "DBUS_SESSION_BUS_ADDRESS": f"unix:path={self.dir}/no-bus",
                }
            )
            argv = [self.bins["sway"], "-c", config]
        else:
            raise ValueError(f"unknown compositor {self.kind}")
        self.proc = subprocess.Popen(
            argv,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=log,
            start_new_session=True,
        )
        log.close()
        deadline = time.monotonic() + PATIENCE
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"{self.kind} exited during start-up:\n{self.log_tail()}")
            names = os.listdir(self.dir)
            wayland = [n for n in names if n.startswith("wayland-") and not n.endswith(".lock")]
            if self.kind == "sway":
                ipc = [n for n in names if n.startswith("sway-ipc.")]
                self.ipc = os.path.join(self.dir, ipc[0]) if ipc else None
            if wayland and self.ipc and _is_socket(self.ipc):
                self.display = sorted(wayland)[0]
                # What actually runs, past any wrapper that execs it.
                self.exe = os.path.realpath(f"/proc/{self.proc.pid}/exe")
                return self
            time.sleep(0.02)
        raise RuntimeError(f"{self.kind} did not come up in {PATIENCE} s:\n{self.log_tail()}")

    def close(self):
        if self.proc is not None and self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
                self.proc.wait()
        if not self.keep:
            shutil.rmtree(self.dir, ignore_errors=True)

    def __enter__(self):
        return self.start()

    def __exit__(self, *exc):
        self.close()

    # -- for daemons -------------------------------------------------------

    def path(self, *parts):
        return os.path.join(self.dir, *parts)

    def client_env(self, extra=None):
        env = dict(os.environ)
        for var in ("WAYLAND_SOCKET", "WAYLAND_DEBUG", "SWAYSOCK", "DISPLAY"):
            env.pop(var, None)
        env.update(
            {
                "XDG_RUNTIME_DIR": self.dir,
                "WAYLAND_DISPLAY": self.display,
                "HOME": self.path("home"),
                "XDG_CONFIG_HOME": self.path("config"),
                "XDG_CACHE_HOME": self.path("cache"),
                "XDG_STATE_HOME": self.path("state"),
            }
        )
        if extra:
            env.update(extra)
        return env

    def log_tail(self, lines=15):
        try:
            with open(self.trace_path, "rb") as f:
                text = f.read().decode("utf-8", "replace").splitlines()
        except FileNotFoundError:
            return ""
        noise = ("] <- ", "] -> ", "[rs]")
        kept = [line for line in text if not any(n in line for n in noise) and "#" not in line[:40]]
        return "\n".join(kept[-lines:])

    def screenshot(self, output_index, out):
        """A PNG of one output (1-based), or ``None`` if it cannot be taken."""
        if self.kind == "scoot":
            argv = [self.bins["scoot"], "msg", "screenshot", "--output", str(output_index)]
            argv += ["--out", out, "--no-cursor"]
            env = dict(os.environ, SCOOT_SOCKET=self.ipc, XDG_RUNTIME_DIR=self.dir)
        else:
            grim = self.bins.get("grim")
            if not grim:
                return None
            argv = [grim, "-o", f"HEADLESS-{output_index}", out]
            env = self.client_env()
        r = subprocess.run(argv, env=env, capture_output=True, timeout=30)
        return out if r.returncode == 0 and os.path.exists(out) else None

    def disk_written(self):
        """Bytes under the session's home, config, cache and state
        directories, split into Mesa's shader cache (written by a GL
        daemon's driver) and everything else, with the file list."""
        total = mesa = 0
        files = []
        for sub in ("home", "cache", "state", "config"):
            root = self.path(sub)
            for dirpath, _dirs, names in os.walk(root):
                for name in names:
                    p = os.path.join(dirpath, name)
                    try:
                        size = os.lstat(p).st_size
                    except FileNotFoundError:
                        continue
                    rel = os.path.relpath(p, self.dir)
                    if "mesa_shader_cache" in rel:
                        mesa += size
                    else:
                        total += size
                        files.append([rel, size])
        return {"bytes": total, "mesa_shader_cache_bytes": mesa, "files": files}
