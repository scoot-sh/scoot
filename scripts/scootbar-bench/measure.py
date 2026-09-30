"""The measured rows, for one bar on one stage (``stage.py``). Each returns
records (dicts) for ``runs.jsonl``.

- **Startup**: from just before ``exec`` to the bar's first buffer commit,
  as the compositor received it (``WAYLAND_DEBUG=server``): the first
  frame, whatever it shows. Each start waits until no client has committed
  a buffer for half a second (``wait_quiet``), so the first commit after
  it is the bar's: the windows redraw when shown and when a bar's zone
  comes or goes, but only after the bar's own commit. M0 timed the same
  event from the bar's own ``WAYLAND_DEBUG`` output instead, which costs
  the bar the printing; this costs the compositor the printing, which
  every bar waits on alike. So the absolute times are this harness's own:
  compare them within a run, or with another run of this script, not with
  M0's table.
- **Idle**: a fresh start, its first frame awaited, then a fixed settle
  (idle is waited for, not detected, as M0 did), then a window. **Wakeups
  are voluntary context switches**, summed over every thread of every
  process of the bar (the ratified target counts these: two a minute with
  a clock); involuntary ones are kept beside them. **CPU** is the bar's
  cgroup over the window (``procs.CpuGroup``: every process it runs, exited
  ones included). **Memory** is read at the window's end: RSS, PSS
  (``smaps_rollup``), heap (``RssAnon``), and peak (``VmHWM``, the whole
  run, startup included).
- **Switching**: straight after the idle window, ``switches`` workspace
  switches at ``hz`` a second between the two windows' workspaces, the same
  counters. A bar that shows no workspaces still gets the row: it says
  what the compositor's churn costs a bystander.
"""

import os
import signal
import subprocess
import time

import machine
import procs

START_TIMEOUT = 30.0
# How long no client may commit before a start is timed, and how long to
# wait for that.
QUIET_S = 0.5
QUIET_TIMEOUT = 30.0


def voluntary(pid):
    """(voluntary, involuntary) context switches summed over every thread
    of ``pid``."""
    vol = invol = 0
    try:
        tids = os.listdir(f"/proc/{pid}/task")
    except (FileNotFoundError, ProcessLookupError):
        return 0, 0
    for tid in tids:
        st = procs.status(f"{pid}/task/{tid}")
        vol += st.get("voluntary_ctxt_switches", 0)
        invol += st.get("nonvoluntary_ctxt_switches", 0)
    return vol, invol


def sample(group):
    """What the bar's processes have done so far: context switches per
    process and the group's CPU."""
    return {pid: voluntary(pid) for pid in group.pids()}, group.usage_ns()


def delta(before, after):
    """(voluntary, involuntary, cpu ms) between two samples. A process that
    exited in between is not counted (its CPU is: the group keeps it)."""
    (sw0, cpu0), (sw1, cpu1) = before, after
    vol = sum(v - sw0.get(pid, (0, 0))[0] for pid, (v, _) in sw1.items())
    invol = sum(i - sw0.get(pid, (0, 0))[1] for pid, (_, i) in sw1.items())
    return vol, invol, (cpu1 - cpu0) / 1e6


class BarRun:
    """One start of a bar, in its own accounting group."""

    def __init__(self, stage, bar, argv, tag):
        self.stage = stage
        self.bar = bar
        self.argv = argv
        self.group = procs.CpuGroup(tag)
        self.proc = None
        self.log = stage.sess.path(f"{bar.name}-{tag}.log")

    def start(self):
        """Starts the bar; returns ``t0`` in µs, taken just before."""
        log = open(self.log, "ab")
        t0 = time.time_ns() // 1000
        self.proc = subprocess.Popen(
            self.group.wrap(self.argv), env=self.stage.env,
            stdin=subprocess.DEVNULL, stdout=log, stderr=log,
        )
        log.close()
        return t0

    def first_frame(self, t0):
        """The first buffer commit at or after ``t0`` (µs), or raises."""
        trace = self.stage.sess.trace
        deadline = time.monotonic() + START_TIMEOUT
        while time.monotonic() < deadline:
            trace.poll()
            commits = trace.since(t0)
            if commits:
                return min(t for t, _ in commits)
            if self.proc.poll() is not None:
                raise RuntimeError(f"{self.bar.name} exited ({self.proc.returncode}) "
                                   f"before its first frame\n{self.tail()}")
            time.sleep(0.001)
        raise RuntimeError(f"{self.bar.name}: no frame in {START_TIMEOUT} s\n{self.tail()}")

    def tail(self, lines=8):
        try:
            with open(self.log, "rb") as f:
                return "\n".join(f.read().decode("utf-8", "replace").splitlines()[-lines:])
        except FileNotFoundError:
            return ""

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def close(self):
        if self.alive():
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
        self.group.close()
        if self.proc is not None:
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass


def base(stage, bar, row, round_no):
    return {
        "compositor": stage.kind,
        "bar": bar.name,
        "row": row,
        "round": round_no,
        "time": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "loadavg": os.getloadavg()[0],
        "hw_start": machine.state(),
    }


def finish(rec):
    """Stamps the machine's state at the end of a record (``machine.py``),
    so a run that throttled or lost mains power shows it."""
    rec["hw_end"] = machine.state()
    return rec


def wait_quiet(trace, quiet_s=QUIET_S, timeout=QUIET_TIMEOUT):
    """Waits until no client has committed a buffer for ``quiet_s``, so the
    first commit after a start is the bar's. The windows commit when they
    are shown (a workspace switch) and when a bar's zone comes or goes;
    once settled they draw nothing. Raises if they never settle."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        trace.poll()
        now_us = time.time_ns() // 1000
        last = max((t for t, _ in trace.commits), default=0)
        if now_us - last >= quiet_s * 1e6:
            return
        time.sleep(0.02)
    raise RuntimeError(f"the compositor's clients never went quiet for {quiet_s} s")


def startup(stage, bar, argv, rounds):
    out = []
    for r in range(1, rounds + 1):
        rec = base(stage, bar, "startup", r)
        run = None
        try:
            wait_quiet(stage.sess.trace)
            run = BarRun(stage, bar, argv, f"start{r}")
            t0 = run.start()
            rec["latency_ms"] = (run.first_frame(t0) - t0) / 1000
            rec["ok"] = True
        except (RuntimeError, OSError) as e:
            rec.update(ok=False, error=str(e))
        finally:
            if run is not None:
                run.close()
        out.append(finish(rec))
    return out


PART_W = 300
BAR_H = 26
OUTPUT_W = 1920


def screen_ok(stage, magick, rgb, parts=()):
    """``(ok, missing)`` from one screenshot of the output. ``ok``: the
    bar's own color is at the top, at the middle of its width where no
    module is placed (the workspaces are on the left and the clock on the
    right; a pixel at the corner could be a workspace's). ``missing``: the
    parts of ``parts`` (``left``, ``center``, ``right``) whose 300-pixel
    span of the bar shows only one color, so their modules drew nothing
    (Waybar's ``ext/workspaces`` with no workspace protocol would do that).
    Both are ``None`` when there is no screenshot or no ImageMagick to
    read one."""
    if not magick:
        return None, None
    png = stage.sess.screenshot(1, stage.sess.path("shot.png"))
    if not png:
        return None, None
    r = subprocess.run(
        [magick, png, "-format", "%[fx:int(255*p{960,4}.r)],%[fx:int(255*p{960,4}.g)],"
         "%[fx:int(255*p{960,4}.b)]", "info:"],
        capture_output=True, text=True,
    )
    try:
        got = [int(v) for v in r.stdout.strip().split(",")]
    except ValueError:
        return None, None
    missing = []
    x0 = {"left": 0, "center": (OUTPUT_W - PART_W) // 2, "right": OUTPUT_W - PART_W}
    for part in parts:
        r = subprocess.run(
            [magick, png, "-crop", f"{PART_W}x{BAR_H}+{x0[part]}+0", "+repage",
             "-format", "%k", "info:"],
            capture_output=True, text=True,
        )
        try:
            if int(r.stdout.strip()) < 2:
                missing.append(part)
        except ValueError:
            return None, None
    return all(abs(a - b) <= 1 for a, b in zip(got, rgb)), missing


def idle(stage, bar, argv, settle_s, window_s, switches, hz, magick=None, rgb=(0x1e, 0x1e, 0x2e),
         parts=()):
    rec = base(stage, bar, "idle", 1)
    run = BarRun(stage, bar, argv, "idle")
    try:
        wait_quiet(stage.sess.trace)
        t0 = run.start()
        rec["first_frame_ms"] = (run.first_frame(t0) - t0) / 1000
        time.sleep(settle_s)
        before = sample(run.group)
        t_start = time.monotonic()
        time.sleep(window_s)
        after = sample(run.group)
        elapsed = time.monotonic() - t_start
        vol, invol, cpu = delta(before, after)
        pids = run.group.pids()
        if not run.alive() or not pids:
            raise RuntimeError(f"{bar.name} exited while idle\n{run.tail()}")
        mem = [procs.memory_at_rest(p) for p in pids]
        rec.update(
            processes=len(pids),
            window_s=elapsed,
            wakeups=vol,
            involuntary=invol,
            wakeups_per_min=vol / elapsed * 60,
            idle_cpu_ms=cpu,
            rss_kb=sum(m["rss_kb"] for m in mem),
            pss_kb=sum(m["pss_kb"] for m in mem),
            rss_anon_kb=sum(m["rss_anon_kb"] for m in mem),
            hwm_kb=sum(procs.status(p).get("VmHWM", 0) for p in pids),
            threads=sum(m["threads"] for m in mem),
            fds=sum(m["fds"] for m in mem),
        )
        rec["screen_ok"], missing = screen_ok(stage, magick, rgb, parts)
        rec["parts_missing"] = missing
        if switches:
            before = sample(run.group)
            t_start = time.monotonic()
            for i in range(switches):
                stage.switch((i + 1) % 2)
                time.sleep(max(0.0, t_start + (i + 1) / hz - time.monotonic()))
            after = sample(run.group)
            vol, invol, cpu = delta(before, after)
            rec.update(
                switches=switches,
                switch_window_s=time.monotonic() - t_start,
                switch_wakeups=vol,
                switch_cpu_ms=cpu,
                rss_after_switching_kb=sum(procs.rss_kb(p) for p in run.group.pids()),
            )
            stage.switch(0)
        rec["ok"] = run.alive() and rec["screen_ok"] is not False and not missing
        if rec["screen_ok"] is False:
            rec["error"] = "the bar's color is not at the top of the output"
        elif missing:
            rec["error"] = f"nothing drawn in the bar's {', '.join(missing)} part"
    except (RuntimeError, OSError, subprocess.SubprocessError) as e:
        rec.update(ok=False, error=str(e))
    finally:
        run.close()
    return [finish(rec)]
