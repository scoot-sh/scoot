"""One daemon in one compositor session, and when its work is done.

"Done" (``settle``) means: every output's surface has had a buffer commit
since ``t0``, every client the harness ran has exited, and for ``quiet``
seconds there has been no further buffer commit and the daemon's process
tree has used under 2 ms of CPU. The time reported is the **last** buffer
commit, not the first: a daemon that commits a placeholder (black, or an
old frame) and then the wallpaper is timed to the wallpaper. Every run
also records the first commit and the count, so a placeholder shows.
"""

import os
import signal
import subprocess
import time

import procs

QUIET_S = 1.0
QUIET_CPU_NS = 2_000_000
TIMEOUT_S = 60.0


class Client:
    """A short-lived process run for the daemon (``scootbg set``, ``awww
    img``, ``wpaperctl``), reaped with ``wait4`` for its own rusage."""

    def __init__(self, popen):
        self.popen = popen
        self.status = None
        self.maxrss_kb = 0
        self.cpu_ns = 0

    def poll(self):
        if self.status is not None:
            return True
        pid, status, usage = os.wait4(self.popen.pid, os.WNOHANG)
        if pid == 0:
            return False
        self._reaped(status, usage)
        return True

    def wait_done(self, timeout=30.0):
        deadline = time.monotonic() + timeout
        while not self.poll():
            if time.monotonic() > deadline:
                self.popen.kill()
            time.sleep(0.002)
        return self

    def _reaped(self, status, usage):
        self.status = os.waitstatus_to_exitcode(status)
        self.popen.returncode = self.status
        self.maxrss_kb = usage.ru_maxrss
        self.cpu_ns = int((usage.ru_utime + usage.ru_stime) * 1e9)


class Run:
    def __init__(self, sess, daemon, tag, egl_env=None):
        self.sess = sess
        self.daemon = daemon
        self.group = procs.CpuGroup(tag)
        self.proc = None
        self.clients = []
        self.env = sess.client_env(daemon.extra_env(sess))
        if daemon.needs_egl and egl_env:
            self.env.update(egl_env)
        self.log_path = sess.path(f"{daemon.name}.log")

    def spawn_daemon(self, wall):
        argv = self.daemon.daemon_argv(self.sess, wall)
        log = open(self.log_path, "ab")
        self.proc = subprocess.Popen(
            self.group.wrap(argv),
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=log,
        )
        log.close()
        return self.proc

    def client(self, argv):
        log = open(self.log_path, "ab")
        popen = subprocess.Popen(
            self.group.wrap(argv),
            env=self.env,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=log,
        )
        log.close()
        c = Client(popen)
        self.clients.append(c)
        return c

    def wait_socket(self, timeout=10.0):
        from session import listening

        path = self.daemon.socket(self.sess)
        if path is None:
            return True
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if listening(path):
                return True
            if self.proc.poll() is not None:
                return False
            time.sleep(0.001)
        return False

    def start(self, wall):
        """Starts the daemon showing ``wall``: on its command line or config
        where it takes it there, else by its client the moment its socket
        accepts a connection (polled every 1 ms). Returns ``t0`` (µs)."""
        t0 = time.time_ns() // 1000
        takes_it = not self.daemon.start_clients(self.sess, wall)
        self.spawn_daemon(wall if takes_it else None)
        for argv in self.daemon.start_clients(self.sess, wall):
            if not self.wait_socket():
                raise RuntimeError(f"{self.daemon.name}: socket never came up\n{self.log_tail()}")
            self.client(argv)
        return t0

    def signal(self, sig):
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(sig)

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def pids(self):
        return self.group.pids()

    def log_tail(self, lines=8):
        try:
            with open(self.log_path, "rb") as f:
                text = f.read().decode("utf-8", "replace").splitlines()
        except FileNotFoundError:
            return ""
        return "\n".join(text[-lines:])

    def settle(self, t0_us, surfaces, quiet=QUIET_S, timeout=TIMEOUT_S):
        """Waits for the work started at ``t0_us`` to be on screen (see the
        module comment). Returns the commits' figures, or raises."""
        trace = self.sess.trace
        history = []  # (monotonic s, cpu ns)
        deadline = time.monotonic() + timeout
        while True:
            trace.poll()
            for c in self.clients:
                c.poll()
            commits = trace.since(t0_us)
            now = time.monotonic()
            cpu = self.group.usage_ns()
            history.append((now, cpu))
            while len(history) > 2 and history[1][0] <= now - quiet:
                history.pop(0)
            if len({s for _, s in commits}) >= surfaces and all(
                c.status is not None for c in self.clients
            ):
                last = max(t for t, _ in commits)
                wall_now = time.time_ns() // 1000
                calm = history[0][0] <= now - quiet and cpu - history[0][1] < QUIET_CPU_NS
                if wall_now - last >= quiet * 1e6 and calm:
                    return {
                        "latency_ms": (last - t0_us) / 1000,
                        "first_commit_ms": (min(t for t, _ in commits) - t0_us) / 1000,
                        "buffer_commits": len(commits),
                        "surfaces": len({s for _, s in commits}),
                    }
            if self.proc is not None and self.proc.poll() is not None:
                raise RuntimeError(
                    f"{self.daemon.name} exited ({self.proc.returncode}) before its "
                    f"wallpaper was up\n{self.log_tail()}"
                )
            bad = [c for c in self.clients if c.status not in (None, 0)]
            if bad:
                raise RuntimeError(
                    f"{self.daemon.name}: a client failed ({bad[0].status})\n{self.log_tail()}"
                )
            if now > deadline:
                raise RuntimeError(
                    f"{self.daemon.name}: not settled in {timeout} s "
                    f"({len(commits)} buffer commits on {len({s for _, s in commits})} "
                    f"surfaces, want {surfaces})\n{self.log_tail()}"
                )
            time.sleep(0.005)

    def close(self):
        """Stops the daemon (SIGTERM, then the group's SIGKILL) and removes
        the group."""
        if self.alive():
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
        for c in self.clients:
            if c.status is None:
                c.popen.kill()
                c.wait_done()
        self.group.close()
        if self.proc is not None:
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
