#!/usr/bin/env python3
"""Tests for scripts/bump-downstream's tag resolve.
Run: python3 scripts/test_bump_downstream.py

The resolve must query the exact ref plus its peel, never a prefix glob:
with both v0.6.2 and v0.6.20 present, resolving v0.6.2 must return v0.6.2's
own peeled commit, not v0.6.20's. The fixture is a local bare repo, so the
check works offline.
"""

import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "bump-downstream"

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


def load():
    from importlib.machinery import SourceFileLoader
    loader = SourceFileLoader("bump_downstream", str(SCRIPT))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


class World:
    """A repo with colliding tags: v0.6.2 and v0.6.20 on different commits."""

    def __init__(self, light62=False):
        self.tmp = Path(tempfile.mkdtemp(prefix="bump-downstream-test-"))
        self.work = self.tmp / "work"
        self.work.mkdir()
        sh("git", "init", "-q", "-b", "main", cwd=self.work)
        sh("git", "config", "user.name", "t", cwd=self.work)
        sh("git", "config", "user.email", "t@t", cwd=self.work)
        sh("git", "config", "tag.gpgsign", "false", cwd=self.work)
        sh("git", "config", "commit.gpgsign", "false", cwd=self.work)
        (self.work / "f").write_text("one\n")
        sh("git", "add", "f", cwd=self.work)
        sh("git", "commit", "-qm", "one", cwd=self.work)
        if light62:
            sh("git", "tag", "v0.6.2", cwd=self.work)
        else:
            sh("git", "tag", "-a", "v0.6.2", "-m", "v0.6.2", cwd=self.work)
        (self.work / "f").write_text("two\n")
        sh("git", "add", "f", cwd=self.work)
        sh("git", "commit", "-qm", "two", cwd=self.work)
        sh("git", "tag", "-a", "v0.6.20", "-m", "v0.6.20", cwd=self.work)
        self.bare = self.tmp / "upstream.git"
        sh("git", "clone", "-q", "--bare", str(self.work), str(self.bare),
           cwd=self.tmp)
        self.want_62 = sh("git", "rev-parse", "v0.6.2^{}", cwd=self.work).stdout.strip()
        self.want_620 = sh("git", "rev-parse", "v0.6.20^{}", cwd=self.work).stdout.strip()
        assert self.want_62 != self.want_620


class TestScootRevForTag(unittest.TestCase):
    def test_exact_ref_wins_over_prefix_collision(self):
        w = World()
        mod = load()
        got = mod.scoot_rev_for_tag("v0.6.2", repo=str(w.bare))
        self.assertEqual(got, w.want_62)
        self.assertNotEqual(got, w.want_620)

    def test_lightweight_tag_beside_annotated_collider(self):
        # The shape the old `refs/tags/<tag>*` glob got wrong: a lightweight
        # v0.6.2 has no `^{}` line, so "first peeled ref in the listing" was
        # v0.6.20's. Fails on the glob implementation.
        w = World(light62=True)
        out = sh("git", "ls-remote", str(w.bare), "refs/tags/v0.6.2*",
                 cwd=w.tmp).stdout
        self.assertNotIn("refs/tags/v0.6.2^{}", out)
        self.assertIn("refs/tags/v0.6.20^{}", out)
        mod = load()
        got = mod.scoot_rev_for_tag("v0.6.2", repo=str(w.bare))
        self.assertEqual(got, w.want_62)
        self.assertNotEqual(got, w.want_620)

    def test_lightweight_tag_without_peel(self):
        w = World()
        sh("git", "tag", "v0.6.3", cwd=w.work)
        sh("git", "push", "-q", str(w.bare), "v0.6.3", cwd=w.work)
        want = sh("git", "rev-parse", "v0.6.3^{}", cwd=w.work).stdout.strip()
        mod = load()
        self.assertEqual(mod.scoot_rev_for_tag("v0.6.3", repo=str(w.bare)), want)

    def test_missing_tag_refuses(self):
        w = World()
        mod = load()
        with self.assertRaises(SystemExit):
            mod.scoot_rev_for_tag("v9.9.9", repo=str(w.bare))


if __name__ == "__main__":
    unittest.main()
