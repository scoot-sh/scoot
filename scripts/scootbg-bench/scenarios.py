"""The measured scenarios. Each returns records (dicts) for ``runs.jsonl``.

Geometry: the timing and peak rows run on one 3840×2160 output, as
scootbg's earlier records did; the idle rows on 1× 1920×1080 and 2×
3840×2160, as the ticket asks.
"""

import os
import subprocess
import time

import procs
from daemons import COLOR, Wall
from runner import Run
from session import Session

UHD = (3840, 2160, 1)


class Context:
    """What every scenario needs: binaries, the compositor, images, EGL."""

    def __init__(self, compositor, bins, images, egl_env, magick=None, keep=False):
        self.compositor = compositor
        self.bins = bins
        self.images = images  # {"jpeg": path, "first": path}
        self.egl_env = egl_env
        self.magick = magick
        self.keep = keep

    def session(self, geometry):
        w, h, n = geometry
        return Session(self.compositor, self.bins, w, h, n, keep=self.keep)

    def wall(self, kind):
        return Wall("image", self.images["jpeg"]) if kind == "image" else Wall("color", COLOR)


def _base(ctx, daemon, row, variant, geometry, round_no):
    w, h, n = geometry
    return {
        "row": row,
        "variant": variant,
        "geometry": f"{n}x{w}x{h}",
        "daemon": daemon.name,
        "compositor": ctx.compositor,
        "round": round_no,
        "time": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "loadavg": os.getloadavg()[0],
    }


def _pixel(ctx, png, x, y):
    if not png or not ctx.magick:
        return None
    r = subprocess.run(
        [ctx.magick, png, "-format", f"%[fx:int(255*p{{{x},{y}}}.r)],"
         f"%[fx:int(255*p{{{x},{y}}}.g)],%[fx:int(255*p{{{x},{y}}}.b)]", "info:"],
        capture_output=True, text=True,
    )
    try:
        return [int(v) for v in r.stdout.strip().split(",")]
    except ValueError:
        return None


def verify_screen(ctx, sess, wall, outputs):
    """Checks every output shows the wallpaper: the color exactly (within
    1), or for the image a center pixel that is neither the compositor's
    background nor black. ``None`` when no screenshot tool is at hand."""
    verdicts = []
    for i in range(1, outputs + 1):
        png = sess.screenshot(i, sess.path(f"shot{i}.png"))
        px = _pixel(ctx, png, sess.width // 2, sess.height // 2)
        if px is None:
            return None
        if wall.kind == "color":
            want = [int(wall.value[j:j + 2], 16) for j in (1, 3, 5)]
            verdicts.append(all(abs(a - b) <= 1 for a, b in zip(px, want)))
        else:
            verdicts.append(px not in ([0, 0, 0], [20, 20, 25]) and len(set(px)) > 1)
        try:
            os.remove(png)
        except OSError:
            pass
    return all(verdicts)


def _peak_record(sampler, run):
    exact = {c.popen.pid: c.maxrss_kb for c in run.clients if c.status is not None}
    if run.alive():
        exact[run.proc.pid] = procs.status(run.proc.pid).get("VmHWM", 0)
    peak, per = procs.combined_peak(sampler, exact)
    daemon_hwm = per.get(run.proc.pid, 0) if run.proc else 0
    clients = sum(c.maxrss_kb for c in run.clients if c.status is not None)
    return {"peak_kb": peak, "daemon_hwm_kb": daemon_hwm, "clients_maxrss_kb": clients,
            "sampled_sum_kb": sampler.max_sum_kb}


def check(ctx, daemon):
    """Does the daemon put an image up on this compositor at all?"""
    geometry = (1920, 1080, 1)
    rec = _base(ctx, daemon, "check", "image", geometry, 0)
    with ctx.session(geometry) as sess:
        run = Run(sess, daemon, "check", ctx.egl_env)
        try:
            wall = ctx.wall("image")
            t0 = run.start(wall)
            rec.update(run.settle(t0, 1, timeout=30))
            rec["screen_ok"] = verify_screen(ctx, sess, wall, 1)
            rec["ok"] = rec["screen_ok"] is not False
            if not rec["ok"]:
                rec["error"] = "committed a buffer, but the screen does not show the image"
        except RuntimeError as e:
            rec["ok"] = False
            rec["error"] = str(e)
        finally:
            run.close()
    return [rec]


def startup(ctx, daemon, kind, round_no):
    """Daemon start to the wallpaper on screen, from nothing saved."""
    rec = _base(ctx, daemon, "startup", kind, UHD, round_no)
    if not daemon.supports("startup", kind):
        return []
    with ctx.session(UHD) as sess:
        run = Run(sess, daemon, "start", ctx.egl_env)
        sampler = procs.PeakSampler(run.group).start()
        try:
            wall = ctx.wall(kind)
            t0 = run.start(wall)
            rec.update(run.settle(t0, UHD[2]))
            rec["cpu_ms"] = run.group.usage_ns() / 1e6
            sampler.stop()
            rec.update(_peak_record(sampler, run))
            rec["threads"] = procs.status(run.proc.pid).get("Threads")
            rec["screen_ok"] = verify_screen(ctx, sess, wall, 1)
            rec["ok"] = rec["screen_ok"] is not False
        except RuntimeError as e:
            rec["ok"] = False
            rec["error"] = str(e)
        finally:
            sampler.stop()
            run.close()
        written = _base(ctx, daemon, "disk-written", f"after-start-{kind}", UHD, round_no)
        written.update(sess.disk_written())
        written["ok"] = True
    return [rec, written]


def live_set(ctx, daemon, round_no):
    """One live change to the JPEG, then one to a color, on a daemon
    already showing a small PNG (scaled to the output, so a full buffer is
    mapped while the JPEG decodes, as it would be in use)."""
    out = []
    if not (daemon.supports("set", "image") or daemon.supports("set", "color")):
        return out
    with ctx.session(UHD) as sess:
        run = Run(sess, daemon, "set", ctx.egl_env)
        try:
            first = Wall("image", ctx.images["first"])
            run.settle(run.start(first), UHD[2])
            run.clients = []  # the start-up client is not part of any step
            for kind in ("image", "color"):
                if not daemon.supports("set", kind):
                    continue
                rec = _base(ctx, daemon, "set", kind, UHD, round_no)
                wall = ctx.wall(kind)
                for pid in run.pids():
                    procs.reset_hwm(pid)
                sampler = procs.PeakSampler(run.group).start()
                cpu0 = run.group.usage_ns()
                t0 = time.time_ns() // 1000
                run.client(daemon.set_argv(sess, wall))
                try:
                    rec.update(run.settle(t0, UHD[2]))
                    rec["cpu_ms"] = (run.group.usage_ns() - cpu0) / 1e6
                    sampler.stop()
                    rec.update(_peak_record(sampler, run))
                    rec["screen_ok"] = verify_screen(ctx, sess, wall, 1)
                    rec["ok"] = rec["screen_ok"] is not False
                except RuntimeError as e:
                    rec["ok"] = False
                    rec["error"] = str(e)
                finally:
                    sampler.stop()
                out.append(rec)
                # Only this step's client counts toward the next step's peak.
                run.clients = [c for c in run.clients if c.status is None]
            rec_disk = _base(ctx, daemon, "disk-written", "after-set", UHD, round_no)
            rec_disk.update(sess.disk_written())
            rec_disk["ok"] = True
            out.append(rec_disk)
        except RuntimeError as e:
            rec = _base(ctx, daemon, "set", "setup", UHD, round_no)
            rec.update(ok=False, error=str(e))
            out.append(rec)
        finally:
            run.close()
    return out


def restore(ctx, daemon, kind, round_no):
    """Start-up showing what the previous run of the daemon was last set
    to: a first daemon is started, set, and stopped the way a user would
    (``scootbg kill``, ``awww kill``), then a second one is timed."""
    if not daemon.supports("restore", kind):
        return []
    rec = _base(ctx, daemon, "restore", kind, UHD, round_no)
    with ctx.session(UHD) as sess:
        prep = Run(sess, daemon, "prep", ctx.egl_env)
        run = None
        try:
            wall = ctx.wall(kind)
            prep.settle(prep.start(wall), UHD[2])
            daemon.stop(sess, prep)
            prep.proc.wait(timeout=10)
            prep.close()
            time.sleep(0.2)
            run = Run(sess, daemon, "restore", ctx.egl_env)
            t0 = time.time_ns() // 1000
            run.spawn_daemon(None)
            rec.update(run.settle(t0, UHD[2]))
            rec["cpu_ms"] = run.group.usage_ns() / 1e6
            rec["screen_ok"] = verify_screen(ctx, sess, wall, 1)
            rec["ok"] = rec["screen_ok"] is not False
        except (RuntimeError, subprocess.TimeoutExpired) as e:
            rec["ok"] = False
            rec["error"] = str(e)
        finally:
            prep.close()
            if run:
                run.close()
    return [rec]


def idle(ctx, daemons, geometry, kind, round_no, idle_s=60.0, window_s=60.0):
    """Memory ``idle_s`` after the wallpaper is up, then wakeups and CPU
    over ``window_s``. Every daemon of the batch runs at once, each with
    its own compositor: an idle process is unaffected by its neighbours,
    and all of them see the same shared-library sharing, which PSS
    depends on.

    **Where the floor lives.** A daemon's own RSS and PSS do not say what
    its wallpaper costs. swaybg, for one, unmaps its buffer right after
    committing it, so its RSS shows none of the pixels; on sway, the
    compositor maps that buffer's file without having read it yet, so no
    process's RSS or PSS shows them at all, although the pages exist. So
    the floor is defined by the compositor: the shared-memory files it
    maps at the idle sample that it did not map before the daemon started
    (read once every compositor of the batch is up, before any daemon
    starts), weighed by their allocated pages, resident or not
    (``procs.shm_mappings``). The daemon's RSS and PSS above the floor
    leave out only its mappings of those files; any other shared memory it
    keeps (an IPC buffer, a spare) stays in. ``total_pss_kb`` is the
    daemon's PSS above the floor, plus the floor, plus the growth of the
    compositor's anonymous PSS (a copy or texture it made of the pixels):
    the wallpaper's whole cost, each page counted once wherever it is
    mapped. The compositor's file-backed PSS is left out, since it only
    moves as other processes map the same libraries.
    """
    live = []
    out = []
    try:
        sessions = []
        for d in daemons:
            if not d.supports("idle", kind):
                continue
            sess = ctx.session(geometry)
            sessions.append((d, sess))
            sess.start()
        time.sleep(1.0)
        baseline = {d.name: procs.compositor_memory(sess.proc.pid) for d, sess in sessions}
        baseline_shm = {d.name: set(procs.shm_mappings(sess.proc.pid)) for d, sess in sessions}
        for d, sess in sessions:
            run = Run(sess, d, "idle", ctx.egl_env)
            live.append((d, sess, run, None))
            live[-1] = (d, sess, run, run.start(ctx.wall(kind)))
        up = {}
        for d, sess, run, t0 in live:
            try:
                up[d.name] = run.settle(t0, geometry[2], timeout=120)
            except RuntimeError as e:
                rec = _base(ctx, d, "idle", kind, geometry, round_no)
                rec.update(ok=False, error=str(e))
                out.append(rec)
        ready = [(d, s, r) for d, s, r, _ in live if d.name in up]
        # Idle for idle_s from the moment the last of them settled.
        time.sleep(idle_s)
        before = {}
        for d, sess, run in ready:
            rec = _base(ctx, d, "idle", kind, geometry, round_no)
            rec["up"] = up[d.name]
            pids = run.pids()
            mem = [procs.memory_at_rest(p) for p in pids]
            rec["processes"] = len(pids)
            for key in mem[0] if mem else []:
                rec[key] = sum(m[key] for m in mem)
            comp0 = baseline[d.name]
            comp1 = procs.compositor_memory(sess.proc.pid)
            rec["compositor_before"] = comp0
            rec["compositor_after"] = comp1
            floor = procs.floor_accounting(pids, sess.proc.pid, baseline_shm[d.name])
            rec.update(floor)
            rec["rss_above_floor_kb"] = rec.get("rss_kb", 0) - floor["daemon_floor_rss_kb"]
            rec["pss_above_floor_kb"] = rec.get("pss_kb", 0) - floor["daemon_floor_pss_kb"]
            rec["compositor_anon_growth_kb"] = comp1["pss_anon_kb"] - comp0["pss_anon_kb"]
            rec["total_pss_kb"] = (
                rec["pss_above_floor_kb"] + floor["floor_kb"] + rec["compositor_anon_growth_kb"]
            )
            rec["libraries"] = sorted({lib for p in pids for lib in procs.mapped_libraries(p)})
            before[d.name] = (
                rec,
                {p: procs.switches(p) for p in pids},
                run.group.usage_ns(),
            )
        time.sleep(window_s)
        for d, sess, run in ready:
            rec, sw0, cpu0 = before[d.name]
            total = 0
            threads_before = sum(len(v) for v in sw0.values())
            threads_after = 0
            for p in run.pids():
                sw1 = procs.switches(p)
                threads_after += len(sw1)
                for tid, n in sw1.items():
                    total += n - sw0.get(p, {}).get(tid, 0)
            rec["window_s"] = window_s
            rec["wakeups"] = total
            rec["window_cpu_ms"] = (run.group.usage_ns() - cpu0) / 1e6
            rec["threads_before"] = threads_before
            rec["threads_after"] = threads_after
            after = [procs.memory_at_rest(p) for p in run.pids()]
            rec["rss_after_window_kb"] = sum(m["rss_kb"] for m in after)
            rec["screen_ok"] = verify_screen(ctx, sess, ctx.wall(kind), geometry[2])
            rec["ok"] = rec["screen_ok"] is not False and run.alive()
            out.append(rec)
    finally:
        for d, sess, run, _ in live:
            run.close()
        for d, sess in sessions:
            sess.close()
    return out
