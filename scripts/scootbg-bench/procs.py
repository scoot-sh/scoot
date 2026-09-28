"""What a daemon costs, read from the kernel.

CPU is a cgroup's: every process of one daemon run (the daemon, the
clients the harness runs for it, and anything the daemon spawns itself,
such as awww-daemon's ``sh -c 'awww img ...'`` on restore) joins one
accounting group before it execs, and the group's counter keeps the time
of processes that have already exited: nanoseconds on cgroup v1
(``cpuacct.usage``), microseconds on v2 (``cpu.stat`` ``usage_usec``),
against ``/proc``'s 10 ms ticks. A process joins through a ``/bin/sh``
wrapper that writes its own pid to ``cgroup.procs`` and then ``exec``s the
command (``CpuGroup.wrap``), so the pid the harness holds is the command's,
and nothing runs between ``fork`` and ``exec`` in the harness's own
(threaded) process: ``preexec_fn`` is unsafe with threads, and the peak
sampler is one.

Memory is per process, from ``/proc``: ``VmHWM`` (reset with
``clear_refs``) for peaks, ``smaps_rollup`` and ``status`` at rest.
"""

import os
import sys
import threading
import time

PAGE = os.sysconf("SC_PAGE_SIZE")


class CpuGroup:
    """A cpuacct (cgroup v1) or cpu (cgroup v2) accounting group."""

    _serial = 0

    def __init__(self, tag):
        CpuGroup._serial += 1
        name = f"sbb-{os.getpid()}-{CpuGroup._serial}-{tag}"
        v1 = "/sys/fs/cgroup/cpuacct"
        v2 = "/sys/fs/cgroup"
        if os.path.isfile(os.path.join(v1, "cpuacct.usage")) or os.path.isdir(v1):
            self.path = os.path.join(v1, name)
            self.v2 = False
        elif os.path.isfile(os.path.join(v2, "cgroup.controllers")):
            self.path = os.path.join(v2, name)
            self.v2 = True
        else:
            raise RuntimeError("no cgroup cpu accounting (cgroup v1 cpuacct or v2) to use")
        os.mkdir(self.path)
        self._procs = os.path.join(self.path, "cgroup.procs")

    def wrap(self, argv):
        """``argv`` run so that it joins the group before it starts: the
        shell writes its pid (which ``exec`` keeps) and execs the command.
        The shell's few hundred µs after joining are charged to the group,
        the same for every daemon."""
        return ["/bin/sh", "-c", 'echo $$ > "$0" && exec "$@"', self._procs, *argv]

    def usage_ns(self):
        if self.v2:
            with open(os.path.join(self.path, "cpu.stat")) as f:
                for line in f:
                    key, value = line.split()
                    if key == "usage_usec":
                        return int(value) * 1000
            return 0
        with open(os.path.join(self.path, "cpuacct.usage")) as f:
            return int(f.read())

    def pids(self):
        try:
            with open(self._procs) as f:
                return [int(x) for x in f.read().split()]
        except FileNotFoundError:
            return []

    def close(self):
        """Kills whatever is left in the group and removes it; says so on
        stderr, and returns False, if it cannot be removed (a leaked
        group would keep counting nothing, but it is litter)."""
        for _ in range(100):
            left = self.pids()
            if not left:
                break
            for pid in left:
                try:
                    os.kill(pid, 9)
                except ProcessLookupError:
                    pass
            time.sleep(0.02)
        try:
            os.rmdir(self.path)
        except FileNotFoundError:
            return True
        except OSError as e:
            print(f"bench: cannot remove cgroup {self.path}: {e}", file=sys.stderr)
            return False
        return True


def status(pid):
    """``/proc/PID/status`` fields that carry a number (kB for sizes)."""
    out = {}
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                key, _, rest = line.partition(":")
                parts = rest.split()
                if parts and parts[0].isdigit():
                    out[key] = int(parts[0])
    except (FileNotFoundError, ProcessLookupError):
        pass
    return out


def smaps_rollup(pid):
    out = {}
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                key, _, rest = line.partition(":")
                parts = rest.split()
                if len(parts) == 2 and parts[1] == "kB":
                    out[key] = int(parts[0])
    except (FileNotFoundError, ProcessLookupError):
        pass
    return out


def rss_kb(pid):
    try:
        with open(f"/proc/{pid}/statm") as f:
            return int(f.read().split()[1]) * PAGE // 1024
    except (FileNotFoundError, ProcessLookupError, IndexError):
        return 0


def reset_hwm(pid):
    """Resets ``VmHWM`` to the current RSS (``clear_refs`` 5)."""
    try:
        with open(f"/proc/{pid}/clear_refs", "w") as f:
            f.write("5")
    except (FileNotFoundError, ProcessLookupError):
        pass


def switches(pid):
    """Context switches of every thread of ``pid``, by thread id."""
    out = {}
    try:
        tids = os.listdir(f"/proc/{pid}/task")
    except (FileNotFoundError, ProcessLookupError):
        return out
    for tid in tids:
        st = status(f"{pid}/task/{tid}")
        if st:
            out[int(tid)] = st.get("voluntary_ctxt_switches", 0) + st.get(
                "nonvoluntary_ctxt_switches", 0
            )
    return out


def fd_count(pid):
    try:
        return len(os.listdir(f"/proc/{pid}/fd"))
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return 0


def mapped_libraries(pid):
    """Paths of the shared objects ``pid`` has mapped (dlopened ones too)."""
    libs = set()
    try:
        with open(f"/proc/{pid}/maps") as f:
            for line in f:
                parts = line.split(None, 5)
                if len(parts) == 6:
                    path = parts[5].strip()
                    if ".so" in os.path.basename(path) and path.startswith("/"):
                        libs.add(path)
    except (FileNotFoundError, ProcessLookupError):
        pass
    return sorted(libs)


def memory_at_rest(pid):
    """What a process holds, in kB: RSS and PSS with and without its shared
    memory (the output buffers: ``wl_shm`` pools are memfds)."""
    st = status(pid)
    sm = smaps_rollup(pid)
    rss = sm.get("Rss", st.get("VmRSS", 0))
    pss = sm.get("Pss", 0)
    shmem = st.get("RssShmem", 0)
    pss_shmem = sm.get("Pss_Shmem", 0)
    return {
        "rss_kb": rss,
        "pss_kb": pss,
        "rss_shmem_kb": shmem,
        "pss_shmem_kb": pss_shmem,
        "rss_above_floor_kb": rss - shmem,
        "pss_above_floor_kb": pss - pss_shmem,
        "rss_anon_kb": st.get("RssAnon", 0),
        "rss_file_kb": st.get("RssFile", 0),
        "threads": st.get("Threads", 0),
        "fds": fd_count(pid),
    }


class PeakSampler:
    """Samples the processes of a group while a daemon works: the summed
    RSS every ``interval`` s, and the summed PSS every ``pss_every``-th
    sample (``smaps_rollup`` walks the page tables, so it is read less
    often). PSS is the figure the gate uses: awww decodes in its client and
    hands the pixels over in shared memory, and a sum of RSS counts those
    pages once per process that maps them, where PSS counts them once in
    all. A sample can miss a spike shorter than its interval; one process's
    ``VmHWM`` is exact but is RSS, so it is reported beside, not mixed in."""

    def __init__(self, group, interval=0.005, pss_every=2):
        self.group = group
        self.interval = interval
        self.pss_every = pss_every
        self.max_sum_kb = 0
        self.max_pss_kb = 0
        self.hwm_kb = {}  # pid -> last VmHWM seen
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()
        return self

    def _run(self):
        n = 0
        while not self._stop.is_set():
            total = pss = 0
            pids = self.group.pids()
            for pid in pids:
                total += rss_kb(pid)
                hwm = status(pid).get("VmHWM")
                if hwm:
                    self.hwm_kb[pid] = max(self.hwm_kb.get(pid, 0), hwm)
            if n % self.pss_every == 0:
                for pid in pids:
                    pss += smaps_rollup(pid).get("Pss", 0)
                self.max_pss_kb = max(self.max_pss_kb, pss)
            n += 1
            self.max_sum_kb = max(self.max_sum_kb, total)
            self._stop.wait(self.interval)

    def stop(self):
        self._stop.set()
        if self._thread.is_alive():
            self._thread.join()


def combined_peak(sampler, exact_hwm):
    """The peak of a run: the larger of the sampled summed RSS and any one
    process's ``VmHWM`` (``exact_hwm``: pid -> kB, read after the fact where
    possible, e.g. ``wait4``'s ``ru_maxrss`` for clients)."""
    per_process = dict(sampler.hwm_kb)
    for pid, kb in exact_hwm.items():
        per_process[pid] = max(per_process.get(pid, 0), kb)
    single = max(per_process.values(), default=0)
    return max(sampler.max_sum_kb, single), per_process


def compositor_memory(pid):
    """The compositor's side of a wallpaper, in kB (see
    ``scenarios.idle``)."""
    sm = smaps_rollup(pid)
    st = status(pid)
    return {
        "rss_kb": sm.get("Rss", 0),
        "pss_kb": sm.get("Pss", 0),
        "pss_anon_kb": sm.get("Pss_Anon", 0),
        "pss_shmem_kb": sm.get("Pss_Shmem", 0),
        "pss_file_kb": sm.get("Pss_File", 0),
        "rss_anon_kb": st.get("RssAnon", 0),
        "rss_shmem_kb": st.get("RssShmem", 0),
    }


def _is_shm(path):
    return path.startswith("/memfd:") or path.startswith("/dev/shm/") or path.startswith("/SYSV")


def shm_mappings(pid):
    """Every shared-memory file ``pid`` maps (memfds, ``/dev/shm``), by
    file: ``{(dev, inode): {"name", "rss_kb", "pss_kb", "allocated_bytes"}}``.
    ``allocated_bytes`` is the file's allocated pages (``st_blocks``, read
    through ``/proc/PID/map_files``), resident in this process or not: a
    compositor can map a client's buffer without having read it yet, so
    the pages exist while no RSS shows them. ``None`` when the file cannot
    be opened (that needs root)."""
    out = {}
    cur = None
    try:
        with open(f"/proc/{pid}/smaps") as f:
            for line in f:
                parts = line.split()
                if not parts:
                    continue
                if "-" in parts[0] and ":" not in parts[0] and len(parts) >= 5:
                    path = parts[5] if len(parts) > 5 else ""
                    cur = None
                    if _is_shm(path):
                        key = (parts[3], int(parts[4]))
                        cur = out.setdefault(
                            key, {"name": path, "rss_kb": 0, "pss_kb": 0, "allocated_bytes": None}
                        )
                        if cur["allocated_bytes"] is None:
                            try:
                                st = os.stat(f"/proc/{pid}/map_files/{parts[0]}")
                                cur["allocated_bytes"] = st.st_blocks * 512
                            except OSError:
                                pass
                elif cur is not None and parts[0] in ("Rss:", "Pss:"):
                    cur["rss_kb" if parts[0] == "Rss:" else "pss_kb"] += int(parts[1])
    except (FileNotFoundError, ProcessLookupError):
        pass
    return out


def floor_accounting(daemon_pids, compositor_pid, compositor_baseline_keys):
    """The idle rows' floor, above-floor and total figures (see
    ``scenarios.idle``). kB throughout."""
    comp = shm_mappings(compositor_pid)
    floor_keys = set(comp) - set(compositor_baseline_keys)
    daemon_floor_rss = daemon_floor_pss = 0
    daemon_other_shm = {}
    for pid in daemon_pids:
        for key, m in shm_mappings(pid).items():
            if key in floor_keys:
                daemon_floor_rss += m["rss_kb"]
                daemon_floor_pss += m["pss_kb"]
            else:
                daemon_other_shm[f"{key[0]}:{key[1]}"] = m
    floor_bytes = 0
    for key in floor_keys:
        allocated = comp[key]["allocated_bytes"]
        floor_bytes += allocated if allocated is not None else comp[key]["rss_kb"] * 1024
    return {
        "floor_kb": floor_bytes // 1024,
        "floor_files": [comp[k]["name"] for k in sorted(floor_keys)],
        "daemon_floor_rss_kb": daemon_floor_rss,
        "daemon_floor_pss_kb": daemon_floor_pss,
        "daemon_other_shm": daemon_other_shm,
    }
