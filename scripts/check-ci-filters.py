#!/usr/bin/env python3
"""Fail when ci.yml's classify job skips a job a crate change can affect.

Derives each job's true crate closure from the workspace dependency graph
(`cargo metadata --no-deps`, falling back to parsing Cargo.toml path deps
when cargo is unavailable) and checks that the classify `case` block fans
each crate out to all its dependents -- "a change to shared code fans out
to its dependents and nothing more" (CLAUDE.md). Fail open (unknown path
=> everything) stays; this script only checks that required jobs are set,
never that extra jobs are unset, so narrowing is a separate, proven step.

Usage: scripts/check-ci-filters.py [repo-root]
Exit 1 on an under-trigger (a dependent job the classifier does not run).
"""
import json
import re
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()

# --- workspace dependency graph -------------------------------------------
# Normal + dev deps (every job runs --all-targets, so tests count), split
# by platform: Linux jobs see cfg(linux) edges, the macos job does not.
def _load_fwd():
    try:
        import json as _json
        import subprocess as _sp
        meta = _json.loads(
            _sp.run(
                ["cargo", "metadata", "--no-deps", "--format-version", "1"],
                cwd=str(root), check=True, capture_output=True, text=True,
            ).stdout
        )
        linux, mac = {}, {}
        for pkg in meta["packages"]:
            name = pkg["name"]
            if not name.startswith("scoot"):
                continue
            lset, mset = set(), set()
            for dep in pkg["dependencies"]:
                d = dep["name"]
                if not d.startswith("scoot"):
                    continue
                t = dep.get("target")
                linux_only = isinstance(t, str) and "linux" in t
                lset.add(d)
                if not linux_only:
                    mset.add(d)
            linux[name] = lset
            mac[name] = mset
        return linux, mac
    except Exception:
        pass
    # No cargo on this box (the classify job has none): parse the manifests
    # directly. Equivalent for path edges; target tables are approximated
    # by ignoring cfg (over-approximates: safe, may only add runs).
    try:
        import tomllib
    except ImportError:
        import tomli as tomllib  # type: ignore
    linux = {}
    for manifest in sorted(root.glob("crates/*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text())
        name = data.get("package", {}).get("name")
        if not name or not name.startswith("scoot"):
            continue
        deps = set()
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for d in (data.get(section) or {}):
                if d.startswith("scoot"):
                    deps.add(d)
        for t in (data.get("target") or {}).values():
            if isinstance(t, dict):
                for section in ("dependencies", "dev-dependencies"):
                    for d in (t.get(section) or {}):
                        if d.startswith("scoot"):
                            deps.add(d)
        linux[name] = deps
    return linux, dict(linux)


fwd_linux, fwd_mac = _load_fwd()
# Default graph (Linux) used for dependents_of and Linux job closures.
fwd = fwd_linux

# dir name -> package name (usually identical; portal differs in dashes only
# by coincidence it matches too, but read it for real).
dir_to_pkg = {}
for manifest in sorted(root.glob("crates/*/Cargo.toml")):
    text = manifest.read_text()
    m = re.search(r'(?m)^name\s*=\s*"([^"]+)"', text)
    if m:
        dir_to_pkg[manifest.parent.name] = m.group(1)


def dependents_of(target):
    """All workspace crates depending on target, directly or transitively."""
    rev = set()
    # direct
    for c, ds in fwd.items():
        if target in ds:
            rev.add(c)
    # transitive: a crate depending on something that depends on target
    while True:
        added = {c for c, ds in fwd.items() if c != target and ds & rev} - rev
        if not added:
            break
        rev |= added
    return rev


# --- job closures: a job must run when the changed crate is in its closure --
# Primary crates per job (the ones the job builds/tests); the closure adds
# every crate the primary depends on. macos is client-only: it checks scoot,
# core, ipc, scootctl, look and rt (ci.yml's --exclude list), never dbus,
# bg-mem, ui, bar, bg, launch, notify or portal.
JOB_PRIMARY = {
    "scoot": {"scoot", "scootctl", "scoot-core"},
    "macos": {"scoot", "scootctl", "scoot-core", "scoot-ipc", "scoot-look", "scoot-rt"},
    "scootbg": {"scootbg"},
    "scootbar": {"scootbar"},
    "scootlaunch": {"scootlaunch"},
    "scootnotify": {"scootnotify"},
    "portal": {"scoot-globalshortcuts-portal"},
    "look": {"scoot-look"},
}


def closure(primary, graph):
    seen = set(primary)
    while True:
        added = set()
        for c in list(seen):
            added |= graph.get(c, set())
        added -= seen
        if not added:
            break
        seen |= added
    return seen


JOB_CLOSURE = {job: closure(p, fwd_linux) for job, p in JOB_PRIMARY.items()}
# macos sees the mac graph (no cfg(linux) edges): recompute just that one.
JOB_CLOSURE["macos"] = closure(JOB_PRIMARY["macos"], fwd_mac)


def required_jobs(changed_pkg):
    """Jobs that must run when workspace crate changed_pkg changes."""
    return sorted(j for j, cl in JOB_CLOSURE.items() if changed_pkg in cl)


# --- extract and simulate the classify case block --------------------------
ci_yml = (root / ".github/workflows/ci.yml").read_text()
m = re.search(
    r"while IFS= read -r path; do\n(.*?)\n\s*done <<<",
    ci_yml, re.DOTALL,
)
if not m:
    print("::error title=classify block not found::cannot locate the case block in ci.yml")
    sys.exit(2)
case_body = m.group(1)


def classify(paths):
    """Run the actual case block from ci.yml over sample paths (bash)."""
    script = (
        "scoot=false; macos=false; scootbg=false; scootbar=false; "
        "scootlaunch=false; scootnotify=false; portal=false; look=false; "
        "backlog=false; release_config=false; docs_site=false; session=false;\n"
    )
    for p in paths:
        # feed one path per loop iteration exactly as ci.yml does
        script += f'path={p!r}\n' + case_body + "\n"
    script += 'echo "scoot=$scoot macos=$macos scootbg=$scootbg scootbar=$scootbar scootlaunch=$scootlaunch scootnotify=$scootnotify portal=$portal look=$look backlog=$backlog release_config=$release_config docs_site=$docs_site session=$session"\n'
    out = subprocess.run(["bash", "-c", script], capture_output=True, text=True)
    if out.returncode != 0:
        print(out.stderr)
        sys.exit(2)
    # last line holds the final accumulated values
    last = [l for l in out.stdout.splitlines() if "=" in l][-1]
    got = dict(kv.split("=", 1) for kv in last.split())
    return got


errors = []
for dirname, pkg in sorted(dir_to_pkg.items()):
    for probe, label in (
        (f"crates/{dirname}/src/lib.rs", "code"),
        (f"crates/{dirname}/Cargo.toml", "manifest"),
    ):
        got = classify([probe])
        for job in required_jobs(pkg):
            if got.get(job) != "true":
                errors.append(
                    f"under-trigger: {probe} ({pkg}, {label}) needs {job}=true "
                    f"(closure {sorted(JOB_CLOSURE[job])}), classifier says {job}={got.get(job)}"
                )

if errors:
    print("::error title=ci filters under-trigger::a shared crate skips a dependent job")
    for e in errors:
        print(e)
    sys.exit(1)
print(f"ok: every crate fans out to its dependents ({len(dir_to_pkg)} crates checked)")
