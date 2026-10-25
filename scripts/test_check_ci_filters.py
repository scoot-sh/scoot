#!/usr/bin/env python3
"""Table-driven test for ci.yml's classify case block: filters cannot rot.

Runs the actual `case` block extracted from .github/workflows/ci.yml over a
broad path sample and asserts the expected job set per path. The expectations
encode the dependency closures (see scripts/check-ci-filters.py for the
derivation): a change to shared code fans out to its dependents and nothing
more; unknown paths fail open (everything true); docs-only paths run
nothing.

Run: python3 scripts/test_check_ci_filters.py [repo-root]
"""
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve() \
    if __name__ != "__main__" or len(sys.argv) <= 2 else Path(sys.argv[1]).resolve()


def repo_root():
    # argv[1] is an optional repo root for both runner and unittest paths.
    for arg in sys.argv[1:]:
        p = Path(arg)
        if p.is_dir() and (p / ".github/workflows/ci.yml").exists():
            return p.resolve()
    return Path(__file__).resolve().parents[1]


ROOT = repo_root()

CI_YML = (ROOT / ".github/workflows/ci.yml").read_text()
M = re.search(r"while IFS= read -r path; do\n(.*?)\n\s*done <<<", CI_YML, re.DOTALL)
assert M, "classify case block not found"
CASE_BODY = M.group(1)

JOBS = ["scoot", "macos", "scootbg", "scootbar", "scootlaunch",
        "scootnotify", "portal", "look", "backlog", "release_config",
        "docs_site", "session"]


def classify(paths):
    script = (
        "scoot=false; macos=false; scootbg=false; scootbar=false; "
        "scootlaunch=false; scootnotify=false; portal=false; look=false; "
        "backlog=false; release_config=false; docs_site=false; session=false;\n"
    )
    for p in paths:
        script += f'path={p!r}\n' + CASE_BODY + "\n"
    script += 'echo "scoot=$scoot macos=$macos scootbg=$scootbg scootbar=$scootbar scootlaunch=$scootlaunch scootnotify=$scootnotify portal=$portal look=$look backlog=$backlog release_config=$release_config docs_site=$docs_site session=$session"\n'
    out = subprocess.run(["bash", "-c", script], capture_output=True, text=True)
    assert out.returncode == 0, out.stderr
    last = [l for l in out.stdout.splitlines() if "=" in l][-1]
    return dict(kv.split("=", 1) for kv in last.split())


# Path -> the jobs that must be true (subset assertion: extra true is
# fail-open and allowed, missing true is an under-trigger bug). `all` means
# every job true; `none` means every job false.
CASES = {
    # compositor side
    "crates/scoot/src/compositor/mod.rs": {"scoot", "macos"},
    "crates/scoot-core/src/lib.rs": {"scoot", "macos", "scootbg", "scootbar", "portal"},
    "crates/scoot-ipc/src/socket.rs": {"scoot", "macos", "scootbg", "scootbar", "portal"},
    "crates/scootctl/src/main.rs": {"scoot", "macos"},
    "crates/scoot-look/src/schema.rs": {"scoot", "macos", "scootbar", "scootlaunch", "scootnotify", "look"},
    "crates/scoot-rt/src/lib.rs": {"scoot", "macos", "scootbar", "scootlaunch", "scootnotify", "portal"},
    "crates/scoot-dbus/src/conn.rs": {"scoot", "scootbar", "scootlaunch", "scootnotify", "portal"},
    "crates/scootui/src/canvas.rs": {"scootbar", "scootlaunch", "scootnotify"},
    "crates/scootbg-mem/src/lib.rs": {"scoot", "scootbg", "scootbar", "scootlaunch", "scootnotify", "portal"},
    # leaf apps (plus the integration jobs that run on either side)
    "crates/scootbg/src/main.rs": {"scootbg"},
    "crates/scootbar/src/main.rs": {"scootbar"},
    "crates/scootlaunch/src/main.rs": {"scootlaunch"},
    "crates/scootnotify/src/main.rs": {"scootnotify"},
    "crates/scoot-globalshortcuts-portal/src/main.rs": {"portal"},
    "crates/scootbar/Cargo.toml": {"scoot", "scootbar"},
    "crates/scootui/Cargo.toml": {"scootbar", "scootlaunch", "scootnotify"},
    # shared roots and infra: fail open
    "Cargo.toml": {"scoot", "macos", "scootbg", "scootbar", "scootlaunch",
                   "scootnotify", "portal", "look"},
    "Cargo.lock": {"scoot", "macos", "scootbg", "scootbar", "scootlaunch",
                   "scootnotify", "portal", "look"},
    "flake.nix": {"scoot", "macos", "scootbg", "scootbar", "scootlaunch",
                  "scootnotify", "portal", "look"},
    ".github/workflows/ci.yml": {"scoot", "macos", "scootbg", "scootbar", "scootlaunch",
                                 "scootnotify", "portal", "look"},
    # owned elsewhere: nothing here
    "README.md": set(),
    "docs/scootbar/README.md": set(),
    "dev/ci.md": set(),
    "site/src/content/docs/scoot/index.mdx": {"docs_site"},
    "packaging/arch/PKGBUILD": set(),
    "packaging/deb/control": set(),
    "packaging/rpm/scoot.spec": set(),
    "scripts/backlog": {"backlog"},
    "resources/scoot-session": {"session"},
}


class ClassifyTest(unittest.TestCase):
    def test_each_sample_fans_out(self):
        for path, must in CASES.items():
            with self.subTest(path=path):
                got = classify([path])
                true_jobs = {j for j in JOBS if got.get(j) == "true"}
                if not must:
                    self.assertEqual(true_jobs, set(), f"{path} should run nothing, runs {true_jobs}")
                else:
                    missing = set(must) - true_jobs
                    self.assertEqual(missing, set(), f"{path} missing {missing} (runs {true_jobs})")


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
