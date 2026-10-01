#!/usr/bin/env python3
"""The hot-text order file for scootbar (docs/scootbar/README.md, "Usage
optimization"): which functions a running bar executes, listed so the linker
puts them next to each other.

    scripts/scootbar-orderfile/orderfile.py gen --scootbar B --scoot S --scootctl C --font F.ttf \\
        [--foot-bin F --fonts-dir D] [--qemu Q] [--out crates/scootbar/orderfile/hot-text.ld]
    scripts/scootbar-orderfile/orderfile.py check --binary B [--orderfile FILE] \\
        [--profile --scoot S --scootctl C --font F.ttf]

**Why.** The kernel maps a file-backed page on a read fault *and the 64 KiB
around it* (`fault_around_bytes`). rustc lays functions out in source order
inside each crate, so the 230-odd functions a bar runs between its start and
its first frame sit in nearly every 64 KiB of a 1.1 MB `.text`, and the
kernel maps all of it: 1152 kB resident of a 1392 kB mapping, for 257 KB of
code that runs. The same functions in a row take five windows, not fourteen.

**How.** `gen` runs scootbar under `qemu-user` with `-d in_asm`, which logs
each translation block once, the first time it executes, with the name of
the function it is in: that is the set of functions run (and the order they
first ran), taken from the shipped binary, with no instrumentation and no
change of the code. Several scenarios are run and merged, the bench's idle
bar first, so what an idle bar runs sits at the very front and what a bar
with workspaces, a push module, the pointer and `msg` runs follows it.

The names are Rust v0-mangled symbols, which carry a hash for every crate
(`Cs<hash>_8scootbar`) and `B<n>_` back-references whose numbers move with
those hashes' lengths, and **those differ between a `cargo build` and the Nix
package** (measured: 182 of 235 hot names do not exist in the Nix build). So
each symbol is turned into a glob with those two kinds of token replaced by
`*` (`glob()`); a pattern stays a superset of its own symbol, so a
substitution can only match more, never less. `check` says how many sections
each pattern matches in a real build.

The output is a GNU ld `--section-ordering-file` (binutils 2.43 and later):
a mini linker script that places the listed input sections first in `.text`.
`crates/scootbar/build.rs` hands it to the linker when the linker takes it.

Stdlib only; needs `nm` (binutils), `qemu-<arch>` (the machine's own, user
mode), a headless `scoot` and `scootctl`, and for the workspaces scenario a
`foot` and a directory holding DejaVu (`--foot-bin`, `--fonts-dir`).
"""

import argparse
import fnmatch
import json
import os
import platform
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_OUT = os.path.join(HERE, "..", "..", "crates", "scootbar", "orderfile", "hot-text.ld")

# The prefixes LLVM gives a function's section: `.text.<sym>` for most,
# `.text.unlikely.<sym>` for `#[cold]` ones (the grow paths of `Vec`, which
# do run), `.text.startup.<sym>` for static constructors.
PREFIXES = (".text.", ".text.unlikely.", ".text.startup.")

# Input sections that no function name reaches, named by the object file
# (and archive member) they come from: `_start` and `call_weak_fn` are in
# the C start-up objects' plain `.text`, and the outline atomics every
# `Arc` and `Mutex` calls (`__aarch64_ldadd4_relax` and the like) are one
# `.text` each in `libcompiler_builtins`'s members (`*lse_*.o`; where the
# architecture has none this matches nothing).
OBJECT_SECTIONS = (
    "*crt1.o(.text .text.*)",
    "*Scrt1.o(.text .text.*)",
    "*crti.o(.text .text.*)",
    "*compiler_builtins*:*lse_*.o(.text .text.*)",
)

# Functions that run on some runs and not on others, by what a run happens
# to do: a `Vec` grows when it is full, so *which* `grow_one` runs depends on
# how many glyphs the clock's text has or events arrived together. They are
# small (about a hundred bytes each), so all of a family goes in, found or
# not in the profile.
ALWAYS = (
    "*5alloc7raw_vec*8grow_one*",
    "*5alloc7raw_vec*11RawVecInner11finish_grow*",
    "*5alloc7raw_vec*do_reserve_and_handle*",
    "*8smallvec*",
)

# A pattern with fewer literal characters than this over-matches: keep the
# symbol exact instead.
MIN_LITERAL = 16

FONT_PX = "14"


# ---------------------------------------------------------------- patterns

# LLVM numbers a local symbol it had to rename (`write_str.1843`, or
# `.llvm.1843`): the number is the build's, not the function's.
RENAMED = re.compile(r"\.(llvm\.)?\d+$")


def glob(sym):
    """The section-name glob for a symbol: a v0-mangled one has its crate
    disambiguators (`Cs<base62>_`) and back-references (`B<base62>_`)
    replaced by `*`, which are the parts that change between builds of the
    same source. Anything else (`main`, `_start`, a libgcc routine) is its
    own pattern. The result always matches `sym`."""
    out = RENAMED.sub(".*", sym)
    if not sym.startswith("_R"):
        return out
    out = re.sub(r"Cs[0-9A-Za-z]+_", "C*_", out)
    out = re.sub(r"B[0-9A-Za-z]*_", "*", out)
    out = re.sub(r"\*+", "*", out)
    if len(out.replace("*", "")) < MIN_LITERAL:
        return sym
    return out


def matches(pattern, name):
    return fnmatch.fnmatchcase(name, pattern)


def render(patterns, header):
    """The order file: a mini linker script, the patterns in order."""
    lines = ["/*"] + [f" * {line}" if line else " *" for line in header] + [" */", ".text : {"]
    lines += [f"  {section}" for section in OBJECT_SECTIONS]
    for p in patterns:
        for prefix in PREFIXES:
            lines.append(f"  *({prefix}{p})")
    lines.append("}")
    return "\n".join(lines) + "\n"


def parse(text):
    """The patterns of an order file, in order, as `render` wrote them
    (the first prefix's line of each)."""
    out = []
    for line in text.splitlines():
        m = re.fullmatch(r"\s*\*\(\.text\.(.+)\)\s*", line)
        if m and not m.group(1).startswith(("unlikely.", "startup.")):
            out.append(m.group(1))
    return out


# --------------------------------------------------------------- the binary

def nm_symbols(binary):
    """`[(address, size, name)]` of the text symbols of an unstripped
    binary, by address."""
    out = subprocess.run(["nm", "-S", "-n", "--defined-only", binary], capture_output=True, text=True, check=True).stdout
    syms = []
    for line in out.splitlines():
        t = line.split()
        if len(t) == 4 and t[2] in "tTwW":
            syms.append((int(t[0], 16), int(t[1], 16), t[3]))
    return syms


# ------------------------------------------------------------ the profile

def tb_functions(log_path, known, cut=None):
    """The functions of `known` (a set of symbol names) that a qemu
    `-d in_asm` log shows translated, in the order they first ran: those
    before byte `cut` of the log, then those after it. (`cut` is where the
    log stood when the scenario began to end the bar, so what only a
    shutdown runs is kept apart from what a running bar does.)"""
    seen, running, ending = set(), [], []
    pos = 0
    with open(log_path, "rb") as f:
        for raw in f:
            pos += len(raw)
            if raw.startswith(b"IN: "):
                name = raw[4:].decode(errors="replace").strip()
                if name in known and name not in seen:
                    seen.add(name)
                    (running if cut is None or pos <= cut else ending).append(name)
    return running, ending


def wait_until(predicate, timeout, step=0.05):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(step)
    return predicate()


class Session:
    """A headless scoot in a scratch directory, and the environment a client
    runs in there."""

    def __init__(self, scoot, scootctl):
        self.scootctl = scootctl
        self.dir = tempfile.mkdtemp(prefix="sbo", dir="/tmp")
        os.chmod(self.dir, 0o700)
        for sub in ("home", "config", "cache", "state"):
            os.mkdir(os.path.join(self.dir, sub))
        self.ipc = os.path.join(self.dir, "s.sock")
        config = os.path.join(self.dir, "scoot.toml")
        open(config, "w").close()
        env = self.base_env()
        self.proc = subprocess.Popen(
            [scoot, "--headless", "--width", "1920", "--height", "1080", "--outputs", "1",
             "--socket", self.ipc, "--config", config],
            env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        if not wait_until(lambda: self.display() is not None and os.path.exists(self.ipc), 20):
            self.close()
            raise RuntimeError("scoot did not come up")

    def display(self):
        names = [n for n in os.listdir(self.dir) if n.startswith("wayland-") and not n.endswith(".lock")]
        return sorted(names)[0] if names else None

    def base_env(self):
        env = dict(os.environ)
        for var in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "SWAYSOCK", "DISPLAY", "WAYLAND_DEBUG"):
            env.pop(var, None)
        env.update(
            XDG_RUNTIME_DIR=self.dir,
            HOME=os.path.join(self.dir, "home"),
            XDG_CONFIG_HOME=os.path.join(self.dir, "config"),
            XDG_CACHE_HOME=os.path.join(self.dir, "cache"),
            XDG_STATE_HOME=os.path.join(self.dir, "state"),
            SCOOT_SOCKET=self.ipc,
        )
        return env

    def client_env(self):
        env = self.base_env()
        env["WAYLAND_DISPLAY"] = self.display()
        return env

    def ctl(self, *args):
        return subprocess.run([self.scootctl, *args], env=self.client_env(), capture_output=True, text=True, timeout=30)

    def close(self):
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
        shutil.rmtree(self.dir, ignore_errors=True)


def fonts_conf(directory, fonts_dir):
    path = os.path.join(directory, "fonts.conf")
    cache = os.path.join(directory, "fc-cache")
    os.makedirs(cache, exist_ok=True)
    with open(path, "w") as f:
        f.write(
            '<?xml version="1.0"?>\n<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">\n<fontconfig>\n'
            f"  <dir>{fonts_dir}</dir>\n  <cachedir>{cache}</cachedir>\n"
            "  <alias><family>monospace</family><prefer><family>DejaVu Sans Mono</family></prefer></alias>\n"
            "</fontconfig>\n"
        )
    return path


class Scenario:
    """One bar run under qemu, driven by `drive(session, bar_msg)`, ended by
    `msg kill` so that qemu flushes its log. `run` returns the log's path and
    leaves in `cut` the log's size when the ending began."""

    def __init__(self, args, name, flags, config=None, windows=False, drive=None, seconds=20):
        self.args, self.name, self.flags, self.config = args, name, flags, config
        self.windows, self.drive, self.seconds = windows, drive, seconds
        self.cut = None

    def run(self, workdir):
        a = self.args
        log = os.path.join(workdir, f"tb-{self.name}.log")
        sess = Session(a.scoot, a.scootctl)
        foot, bar = None, None
        try:
            env = sess.client_env()
            argv = [a.qemu, "-d", "in_asm", "-D", log, a.scootbar, "daemon",
                    "--font", a.font, "--font-size", FONT_PX, "--height", "26",
                    "--background", "#1e1e2e", "--foreground", "#cdd6f4", *self.flags]
            if self.config:
                path = os.path.join(sess.dir, "bar.toml")
                with open(path, "w") as f:
                    f.write(self.config)
                argv += ["--config", path]
            bar = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

            def msg(*words):
                return subprocess.run([a.scootbar, "msg", *words], env=env, capture_output=True, text=True, timeout=60)

            # qemu is slow to start; wait for the control socket and the first frame
            if not wait_until(lambda: any("scootbar" in n and n.endswith(".sock") for n in os.listdir(sess.dir)), 90):
                raise RuntimeError(f"{self.name}: no control socket: {bar.stderr.read().decode(errors='replace')[-400:]}")
            time.sleep(3)
            if self.windows:
                fenv = dict(env)
                fenv["FONTCONFIG_FILE"] = fonts_conf(sess.dir, a.fonts_dir)
                foot = []
                for n in range(2):
                    foot.append(subprocess.Popen([a.foot_bin], env=fenv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
                    wait_until(lambda: len(json.loads(sess.ctl("windows").stdout or "[]")) > n, 20)
                    if n == 0:
                        sess.ctl("action", "move-window-to-workspace-index", "1")
                sess.ctl("action", "focus-workspace-index", "0")
                time.sleep(1)
            if self.drive:
                self.drive(sess, msg)
            deadline = time.monotonic() + self.seconds
            while time.monotonic() < deadline and bar.poll() is None:
                time.sleep(0.5)
            self.cut = os.path.getsize(log)
            msg("kill")
            try:
                bar.wait(timeout=30)
            except subprocess.TimeoutExpired:
                bar.send_signal(signal.SIGTERM)
                bar.wait(timeout=10)
        finally:
            for p in [*(foot or []), bar]:
                if p is not None and p.poll() is None:
                    p.kill()
            sess.close()
        return log


def drive_workspaces(sess, msg):
    """Switches between the two workspaces, moves over, clicks and scrolls
    the bar, and asks it questions over its control socket."""
    for i in range(6):
        sess.ctl("action", "focus-workspace-index", str(i % 2))
        time.sleep(0.4)
    for x, y in ((1850, 12), (1700, 12), (30, 12), (60, 12), (30, 12)):
        sess.ctl("pointer", "move", str(x), str(y))
        time.sleep(0.4)
    sess.ctl("pointer", "click", "60", "12", "left")
    time.sleep(0.4)
    sess.ctl("pointer", "scroll", "0", "1")
    time.sleep(0.4)
    sess.ctl("pointer", "move", "900", "600")
    for words in (("query",), ("layout",), ("version",)):
        msg(*words)
        time.sleep(0.2)


def drive_custom(sess, msg):
    for text in ("build ok", "3 new mails", "", "build ok again"):
        msg("set", "status", json.dumps(text) if text else "null")
        time.sleep(0.4)
    msg("query")
    msg("query", "status")
    msg("layout")
    sess.ctl("pointer", "move", "1850", "12")
    time.sleep(0.4)
    sess.ctl("pointer", "click", "1850", "12", "left")
    time.sleep(0.4)
    msg("reload")
    time.sleep(1)
    msg("hide")
    time.sleep(0.5)
    msg("show")
    time.sleep(1)


CUSTOM_CONFIG = """left = ["workspaces", "launcher"]
right = ["status", "clock"]

[button.launcher]
text = "Apps"

[push.status]
placeholder = "idle"
"""


def scenarios(args):
    out = [
        # The bench's idle row: the clock alone, across a minute boundary.
        Scenario(args, "idle-clock", ["--right", "clock", "--clock-format", "%a %d %b %H:%M"], seconds=75),
        # The same bar with more to draw: which `Vec` has to grow, and when,
        # depends on how many glyphs the text has, so a bar showing another
        # date would run functions the first run did not.
        Scenario(args, "idle-clock-long", ["--right", "clock", "--clock-format", "%A %d %B %Y %H:%M"], seconds=30),
    ]
    if args.foot_bin and args.fonts_dir:
        out.append(Scenario(args, "workspaces", ["--left", "workspaces", "--right", "clock"],
                            windows=True, drive=drive_workspaces, seconds=5))
        out.append(Scenario(args, "custom", [], config=CUSTOM_CONFIG, windows=True, drive=drive_custom, seconds=3))
    else:
        print("note: no --foot-bin/--fonts-dir: the workspaces and custom-module scenarios are skipped", file=sys.stderr)
    return out


def gen(args):
    syms = nm_symbols(args.scootbar)
    known = {n for _, _, n in syms}
    order, ending, listed, per = [], [], set(), {}
    workdir = tempfile.mkdtemp(prefix="sbo-tb", dir="/tmp")
    try:
        for sc in scenarios(args):
            print(f"scenario {sc.name} ...", file=sys.stderr)
            log = sc.run(workdir)
            running, after = tb_functions(log, known, sc.cut)
            fresh = [f for f in running if f not in listed]
            listed.update(fresh)
            order += fresh
            ending += after
            per[sc.name] = (len(running), len(fresh))
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
    # What only an ending bar runs goes last, and only if no scenario ran it
    # while the bar was up.
    tail = [f for f in dict.fromkeys(ending) if f not in listed]
    order += tail
    sizes = {n: s for _, s, n in syms}
    patterns, seen = list(ALWAYS), set(ALWAYS)
    for name in order:
        g = glob(name)
        if g not in seen:
            seen.add(g)
            patterns.append(g)
    total = sum(sizes.get(n, 0) for n in order)
    header = [
        "scootbar's hot text: the functions a running bar executes, in the order they first run,",
        "as globs (crate hashes and back-references wildcarded). Generated by",
        "scripts/scootbar-orderfile/orderfile.py gen; see its docstring. Do not edit.",
        "",
        f"{len(order)} functions, {total} bytes; {len(tail)} of them (last) run only when the bar ends.",
        "Scenarios, in order (functions run while the bar was up, and of those new to the list):",
    ] + [f"  {k}: {run}, {new}" for k, (run, new) in per.items()]
    with open(args.out, "w") as f:
        f.write(render(patterns, header))
    print(f"wrote {args.out}: {len(patterns)} patterns, {total} bytes of code", file=sys.stderr)
    return 0


# ------------------------------------------------------------------ check

def windows_of(syms, size=65536):
    """How many `size`-byte windows of the address space the symbols touch."""
    return len({w for a, n, _ in syms for w in range(a // size, (a + max(n, 1) - 1) // size + 1)})


def union_regex(patterns):
    return re.compile("|".join(fnmatch.translate(p) for p in patterns))


def check(args):
    """How the order file lands in a build.

    Statically: how many patterns match a symbol of the binary, and how much
    of what they match sits in one stretch. With `--profile`, also what the
    bar *runs* (the bench's idle scenario, under qemu): the functions that
    ran and no pattern matches, and how many 64 KiB windows the ones that ran
    are spread over, which is the number that decides the memory.

    Exit 1 when under `--min-coverage` of the patterns match anything, or
    (with `--profile`) more than `--max-unlisted` of the executed bytes are
    in no pattern."""
    syms = nm_symbols(args.binary)
    with open(args.orderfile) as f:
        patterns = parse(f.read())
    hit, matched = 0, {}
    for p in patterns:
        found = [s for s in syms if matches(p, s[2])]
        hit += bool(found)
        matched.update((s[2], s) for s in found)
    cover = hit / len(patterns) if patterns else 0.0
    ms = sorted(matched.values())
    runs, run = [], []
    for s in ms:
        if run and s[0] - (run[-1][0] + run[-1][1]) > 65536:
            runs.append(run)
            run = []
        run.append(s)
    if run:
        runs.append(run)
    main_run = max(runs, key=lambda r: sum(s[1] for s in r), default=[])
    in_main = sum(s[1] for s in main_run)
    total = sum(s[1] for s in ms)
    print(f"patterns: {len(patterns)}; matching a symbol of the binary: {hit} ({cover:.0%})")
    print(f"matched symbols: {len(ms)}, {total} bytes; {windows_of(ms)} 64 KiB windows; "
          f"{in_main} bytes ({in_main / total if total else 0:.0%}) in the main stretch, "
          f"{windows_of(main_run)} windows")
    ok = cover >= args.min_coverage and total > 0 and in_main >= 0.9 * total
    if args.profile:
        ok &= profile_check(args, syms, union_regex(patterns))
    print("ok" if ok else "FAIL")
    return 0 if ok else 1


def profile_check(args, syms, listed):
    known = {n for _, _, n in syms}
    sc = Scenario(args, "idle-check", ["--right", "clock", "--clock-format", "%a %d %b %H:%M"], seconds=args.seconds)
    workdir = tempfile.mkdtemp(prefix="sbo-chk", dir="/tmp")
    try:
        log = sc.run(workdir)
        ran, _ = tb_functions(log, known, sc.cut)
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
    by_name = {n: (a, s, n) for a, s, n in syms}
    rows = [by_name[n] for n in ran]
    unlisted = [r for r in rows if not listed.fullmatch(r[2])]
    total = sum(r[1] for r in rows)
    bad = sum(r[1] for r in unlisted)
    print(f"ran while up: {len(rows)} functions, {total} bytes, spread over {windows_of(rows)} 64 KiB windows; "
          f"in no pattern: {len(unlisted)} functions, {bad} bytes")
    for a, s, n in sorted(unlisted):
        print(f"  not listed: {a:#x} {s:6} {n[:110]}")
    return bad <= args.max_unlisted * total


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("gen", help="profile the bar and write the order file")
    g.add_argument("--scootbar", required=True, help="an unstripped release scootbar (CARGO_PROFILE_RELEASE_STRIP=false)")
    g.add_argument("--scoot", required=True)
    g.add_argument("--scootctl", required=True)
    default_qemu = shutil.which(f"qemu-{platform.machine()}") or f"qemu-{platform.machine()}"
    g.add_argument("--qemu", default=default_qemu)
    g.add_argument("--font", required=True, help="a TTF file for --font")
    g.add_argument("--foot-bin", help="foot, for the workspaces scenarios")
    g.add_argument("--fonts-dir", help="a directory holding DejaVu, for foot")
    g.add_argument("--out", default=DEFAULT_OUT)
    c = sub.add_parser("check", help="how an order file lands in a build")
    c.add_argument("--binary", required=True, help="an unstripped scootbar built with the order file")
    c.add_argument("--orderfile", default=DEFAULT_OUT)
    c.add_argument("--min-coverage", type=float, default=0.9)
    c.add_argument("--profile", action="store_true", help="also run the idle scenario and say what ran that no pattern lists")
    c.add_argument("--max-unlisted", type=float, default=0.05, help="with --profile: the share of executed bytes allowed in no pattern")
    c.add_argument("--seconds", type=int, default=25)
    c.add_argument("--scoot")
    c.add_argument("--scootctl")
    c.add_argument("--qemu", default=default_qemu)
    c.add_argument("--font")
    args = p.parse_args(argv)
    if args.cmd == "check" and args.profile:
        for need in ("scoot", "scootctl", "font"):
            if not getattr(args, need):
                p.error(f"--profile needs --{need}")
        args.scootbar = args.binary
        args.foot_bin = args.fonts_dir = None
    return gen(args) if args.cmd == "gen" else check(args)


if __name__ == "__main__":
    sys.exit(main())
