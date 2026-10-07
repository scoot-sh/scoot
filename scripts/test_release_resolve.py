#!/usr/bin/env python3
"""Tests for scripts/release-resolve. Run: python3 scripts/test_release_resolve.py

Each test builds a throwaway git repo (fixture manifests, tags, branches)
and drives the real `scripts/release-resolve` CLI against it with `--root`,
so the parse, manifest-match, on-main and existing-release paths are the
ones that ship. The GitHub check runs against a stub `gh` placed on PATH,
never the network. Nothing touches the real repo.
"""

import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "release-resolve"
TRIO = ("scoot", "scootctl", "scootbg")

GIT_ENV = {
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t",
}


def sh(*args, cwd, env=None, check=True):
    merged = {**os.environ, **GIT_ENV}
    if env:
        merged.update(env)
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True,
                          env=merged, check=check)


class World:
    """A fixture repo: four manifests, a main branch, optional tags."""

    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="release-resolve-test-"))
        self.root = self.tmp / "repo"
        self.root.mkdir()
        sh("git", "init", "-q", "-b", "main", cwd=self.root)
        sh("git", "config", "user.name", "t", cwd=self.root)
        sh("git", "config", "user.email", "t@t", cwd=self.root)
        # An origin/main to resolve against, like a CI checkout has.
        sh("git", "remote", "add", "origin", self.tmp / "upstream.git",
           cwd=self.root)
        for package in ("scoot", "scootctl", "scootbg", "scootbar"):
            self.set_manifest(package, "0.1.0")
        self.commit("chore: init")
        sh("git", "update-ref", "refs/remotes/origin/main", "main",
           cwd=self.root)

    def set_manifest(self, package, version):
        d = self.root / "crates" / package
        d.mkdir(parents=True, exist_ok=True)
        (d / "Cargo.toml").write_text(
            f'[package]\nname = "{package}"\nversion = "{version}"\n')

    def commit(self, subject, filename=None):
        if filename is not None:
            (self.root / filename).write_text(subject)
            sh("git", "add", filename, cwd=self.root)
        sh("git", "commit", "-q", "--allow-empty", "-m", subject,
           cwd=self.root)

    def tag(self, name, ref="main"):
        sh("git", "tag", name, ref, cwd=self.root)

    def push_main(self):
        """Move origin/main onto main, like a CI checkout of a main tag."""
        sh("git", "update-ref", "refs/remotes/origin/main", "main",
           cwd=self.root)

    def resolve(self, *args, gh_stub=None):
        env = {}
        if gh_stub is not None:
            bindir = self.tmp / "bin"
            bindir.mkdir(exist_ok=True)
            gh = bindir / "gh"
            gh.write_text(gh_stub)
            gh.chmod(gh.stat().st_mode | stat.S_IXUSR)
            env["PATH"] = f"{bindir}{os.pathsep}{os.environ['PATH']}"
        return sh(sys.executable, str(SCRIPT), *args,
                  "--root", str(self.root), cwd=self.root, env=env,
                  check=False)


GH_ABSENT = "#!/bin/sh\necho 'release not found' >&2\nexit 1\n"
GH_PRESENT_BARE = "#!/bin/sh\necho '{\"assets\": []}'\nexit 0\n"


def gh_present(*names):
    # Simulates `gh release view --json assets --jq '.assets[].name'`:
    # one asset name per line, like the real --jq output.
    body = "\n".join(f'echo "{n}"' for n in names)
    return f"#!/bin/sh\n{body}\nexit 0\n"


GH_BROKEN = "#!/bin/sh\necho 'Bad credentials' >&2\nexit 1\n"


class ResolveTests(unittest.TestCase):
    def test_trio_tag_resolves_the_trio(self):
        w = World()
        w.commit("feat(scoot): a thing", "a.txt")
        w.tag("scoot-v0.1.0")
        w.push_main()
        r = w.resolve("scoot-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("package=scoot", r.stdout)
        self.assertIn("binaries=scoot scootbg", r.stdout)

    def test_bar_tag_resolves_only_the_bar(self):
        w = World()
        w.commit("feat(scootbar): a thing", "b.txt")
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("binaries=scootbar", r.stdout)

    def test_json_output_names_expected_assets(self):
        w = World()
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", "--json", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        got = json.loads(r.stdout)
        self.assertEqual(got["package"], "scootbar")
        self.assertEqual(got["version"], "0.1.0")
        self.assertIn("scootbar-0.1.0-x86_64-linux", got["assets"])
        self.assertIn("scootbar-0.1.0-aarch64-linux", got["assets"])
        self.assertIn("SHA256SUMS", got["assets"])
        self.assertTrue(any(n.endswith("-vendored.tar.gz")
                            for n in got["assets"]),
                        got["assets"])
        self.assertTrue(any(n.endswith(".cyclonedx.json")
                            for n in got["assets"]),
                        got["assets"])

    def test_bad_tag_shape_refused(self):
        w = World()
        r = w.resolve("v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("not <package>-vX.Y.Z", r.stderr)

    def test_unknown_package_refused(self):
        w = World()
        r = w.resolve("scootnotify-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("unknown package", r.stderr)

    def test_missing_tag_refused(self):
        w = World()
        r = w.resolve("scootbar-v9.9.9", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("no such tag", r.stderr)

    def test_manifest_mismatch_refused(self):
        w = World()
        w.set_manifest("scootbar", "0.2.0")
        w.commit("chore(scootbar): release 0.2.0", "Cargo.toml")
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("manifest", r.stderr)

    def test_trio_lockstep_violation_refused(self):
        w = World()
        w.set_manifest("scootbg", "0.2.0")
        w.commit("chore(scootbg): bump alone", "Cargo.toml")
        w.tag("scoot-v0.1.0")
        w.push_main()
        r = w.resolve("scoot-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("lockstep", r.stderr)

    def test_tag_not_on_main_refused(self):
        w = World()
        sh("git", "checkout", "-qb", "side", cwd=w.root)
        w.commit("feat(scootbar): off-main work", "side.txt")
        w.tag("scootbar-v0.1.0", ref="side")
        sh("git", "checkout", "-q", "main", cwd=w.root)
        r = w.resolve("scootbar-v0.1.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("not on main", r.stderr)

    def test_existing_complete_release_refused(self):
        w = World()
        w.tag("scootbar-v0.1.0")
        w.push_main()
        probe = w.resolve("scootbar-v0.1.0", "--json",
                          gh_stub=GH_PRESENT_BARE)
        assets = json.loads(probe.stdout)["assets"]
        r = w.resolve("scootbar-v0.1.0", gh_stub=gh_present(*assets))
        self.assertEqual(r.returncode, 2)
        self.assertIn("already published", r.stderr)

    def test_existing_incomplete_release_resumes(self):
        w = World()
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", gh_stub=GH_PRESENT_BARE)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("incomplete release resumes", r.stdout + r.stderr)

    def test_gh_failure_fails_closed(self):
        w = World()
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", gh_stub=GH_BROKEN)
        self.assertEqual(r.returncode, 2)
        self.assertIn("could not verify", r.stderr)

    def test_no_github_skips_the_release_check(self):
        w = World()
        w.tag("scootbar-v0.1.0")
        w.push_main()
        r = w.resolve("scootbar-v0.1.0", "--no-github")
        self.assertEqual(r.returncode, 0, r.stderr)


if __name__ == "__main__":
    unittest.main()
