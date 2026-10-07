#!/usr/bin/env python3
"""Tests for scripts/version. Run: python3 scripts/test_version.py

Each test builds a throwaway git repo (a fixture history with scoped
commits) and drives the real `scripts/version` CLI against it with
`--root`, so the parsing, bump, tag and check paths are the ones that
ship. Nothing touches the real repo.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "version"
SHIPPED = ("scoot", "scootctl", "scootbg", "scootbar")
INTERNAL = ("scoot-core", "scoot-ipc", "scootbg-mem")

GIT_ENV = {
    "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t",
    "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t",
}


def sh(*args, cwd):
    env = {**os.environ, **GIT_ENV}
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True,
                          text=True, env=env)


class World:
    """A fixture repo: minimal manifests plus a scripted history."""

    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="version-test-"))
        self.root = self.tmp / "repo"
        self.root.mkdir()
        sh("git", "init", "-q", cwd=self.root)
        sh("git", "config", "user.name", "t", cwd=self.root)
        sh("git", "config", "user.email", "t@t", cwd=self.root)
        (self.root / "Cargo.toml").write_text(
            '[workspace]\n[workspace.package]\nversion = "0.1.0"\n')
        for package in SHIPPED:
            self.set_manifest(package, 'version = "0.1.0"')
        for package in INTERNAL:
            (self.root / "crates" / package).mkdir(parents=True, exist_ok=True)
            (self.root / "crates" / package / "Cargo.toml").write_text(
                f'[package]\nname = "{package}"\nversion.workspace = true\n'
                'publish = false\n')

    def set_manifest(self, package, version_line):
        d = self.root / "crates" / package
        d.mkdir(parents=True, exist_ok=True)
        (d / "Cargo.toml").write_text(
            f'[package]\nname = "{package}"\n{version_line}\n')

    def commit(self, subject, body=None):
        # Every commit touches a file, so empty-message edge cases aside,
        # the history is real commits, not empty ones.
        f = self.root / "f.txt"
        with f.open("a") as fh:
            fh.write(subject + "\n")
        sh("git", "add", "-A", cwd=self.root)
        args = ["git", "commit", "-q", "-m", subject]
        if body is not None:
            args += ["-m", body]
        sh(*args, cwd=self.root)

    def tag(self, name):
        sh("git", "tag", name, cwd=self.root)

    def run(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), "--root",
                               str(self.root), *args],
                              capture_output=True, text=True)

    def plan(self, *args):
        r = self.run("plan", "--json", *args)
        assert r.returncode == 0, r.stderr
        return {p["package"]: p for p in json.loads(r.stdout)}

    def tearDown(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)


class TestBumps(unittest.TestCase):
    def setUp(self):
        self.w = World()

    def tearDown(self):
        self.w.tearDown()

    def test_feat_is_minor_fix_and_perf_are_patch(self):
        self.w.commit("feat(scoot): columns scroll")
        self.w.commit("fix(scootbar): clock ticks")
        self.w.commit("perf(scootbg): decode once")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "0.2.0")  # trio lockstep
        self.assertEqual(plan["scootctl"]["next"], "0.2.0")
        self.assertEqual(plan["scootbg"]["next"], "0.2.0")
        self.assertEqual(plan["scootbar"]["next"], "0.1.1")
        self.assertEqual(plan["scoot"]["bump"], "minor")
        self.assertEqual(plan["scootbar"]["bump"], "patch")

    def test_bang_and_footer_are_major_pre10_means_minor(self):
        self.w.commit("feat(scoot)!: drop the old socket")
        plan = self.w.plan()
        # Pre-1.0: a major bump lands as minor, never 1.0.0 undeclared.
        self.assertEqual(plan["scoot"]["next"], "0.2.0")
        self.assertEqual(plan["scoot"]["bump"], "major")
        self.w.commit("fix(scootbar): tick", body="BREAKING CHANGE: new clock shape")
        plan = self.w.plan()
        self.assertEqual(plan["scootbar"]["next"], "0.2.0")

    def test_a_body_that_only_quotes_the_footer_is_not_breaking(self):
        self.w.commit(
            "fix(scootbar): tick",
            body="This is not a BREAKING CHANGE: the clock shape is as before",
        )
        plan = self.w.plan()
        self.assertEqual(plan["scootbar"]["bump"], "patch")

    def test_post10_uses_normal_rules(self):
        for p in SHIPPED:
            self.w.set_manifest(p, 'version = "1.2.3"')
        self.w.commit("feat(scoot): columns scroll")
        self.w.commit("fix(scootbar): clock ticks")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "1.3.0")
        self.assertEqual(plan["scootbar"]["next"], "1.2.4")
        self.w.commit("fix(scootbg)!: drop a mode")
        plan = self.w.plan()
        self.assertEqual(plan["scootbg"]["next"], "2.0.0")
        self.assertEqual(plan["scoot"]["next"], "2.0.0")

    def test_unscoped_and_non_package_scopes_never_bump(self):
        self.w.commit("feat: no scope names no package")
        self.w.commit("fix: still no scope")
        self.w.commit("ci: rotate the runners")
        self.w.commit("docs(backlog): file the tray entry")
        self.w.commit("ci(nix): bump the pin")
        self.w.commit("docs(site): rewrite the install page")
        self.w.commit("chore(deps): bump serde")
        self.w.commit("docs(claude): reword the cycle")
        plan = self.w.plan()
        for p in SHIPPED:
            self.assertIsNone(plan[p]["bump"], p)
            self.assertIsNone(plan[p]["tag"], p)

    def test_housekeeping_types_with_a_scope_do_not_bump(self):
        self.w.commit("refactor(scoot): split the loop")
        self.w.commit("test(scootbar): pin the clock")
        self.w.commit("build(scootbg): thin the deps")
        self.w.commit("chore(scoot): rename a file")
        self.w.commit("ci(scoot): quiet a flake")
        plan = self.w.plan()
        for p in SHIPPED:
            self.assertIsNone(plan[p]["bump"], p)

    def test_merge_subjects_are_ignored_squash_titles_count(self):
        self.w.commit("Merge pull request #494 from scoot-sh/branch")
        self.w.commit("Merge branch 'main' into feat/x")
        plan = self.w.plan()
        for p in SHIPPED:
            self.assertIsNone(plan[p]["bump"], p)
        # A squash-merge lands the PR title as the subject: it parses.
        self.w.commit("feat(scoot): opt-in keybindings for virtual keyboards (#494)")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "0.2.0")

    def test_multi_scope_fans_out(self):
        self.w.commit("fix(scoot,scootbar): refuse a stale index")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "0.1.1")
        self.assertEqual(plan["scootbar"]["next"], "0.1.1")

    def test_trio_lockstep_bar_independent(self):
        self.w.commit("feat(scootbg): fetch from a link")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "0.2.0")
        self.assertEqual(plan["scootctl"]["next"], "0.2.0")
        self.assertEqual(plan["scootbg"]["next"], "0.2.0")
        self.assertIsNone(plan["scootbar"]["bump"])
        self.w.commit("feat(scootbar): tray module")
        plan = self.w.plan()
        self.assertEqual(plan["scootbar"]["next"], "0.2.0")
        # The bar's bump does not move the trio past its own feat.
        self.assertEqual(plan["scoot"]["next"], "0.2.0")

    def test_internal_lib_scopes_fan_out_like_ci(self):
        self.w.commit("fix(scoot-ipc): guess closer")
        plan = self.w.plan()
        # Every shipped package embeds scoot-ipc: all four move.
        for p in SHIPPED:
            self.assertEqual(plan[p]["next"], "0.1.1", p)
        w2 = World()
        self.addCleanup(w2.tearDown)
        w2.commit("fix(scoot-core): clamp an edge")
        plan = w2.plan()
        # Only scoot consumes scoot-core; the trio moves as one, the bar not at all.
        self.assertEqual(plan["scoot"]["next"], "0.1.1")
        self.assertIsNone(plan["scootbar"]["bump"])

    def test_unknown_scopes_and_types_are_ignored(self):
        self.w.commit("feat(scootnotify): a daemon that does not exist yet")
        self.w.commit("wip(scoot): not a type")
        plan = self.w.plan()
        for p in SHIPPED:
            self.assertIsNone(plan[p]["bump"], p)

    def test_no_prior_tag_reads_the_whole_history(self):
        self.w.commit("feat(scootbar): skeleton (#323)")
        self.w.commit("fix(scootbar): no hang without fonts")
        plan = self.w.plan()
        self.assertEqual(plan["scootbar"]["next"], "0.2.0")
        self.assertIsNone(plan["scootbar"]["last_tag"])

    def test_only_commits_after_the_tag_count(self):
        self.w.commit("feat(scoot): columns scroll")
        self.w.tag("scoot-v0.2.0")
        self.w.set_manifest("scoot", 'version = "0.2.0"')
        self.w.set_manifest("scootctl", 'version = "0.2.0"')
        self.w.set_manifest("scootbg", 'version = "0.2.0"')
        self.w.commit("docs(backlog): file something")
        plan = self.w.plan()
        self.assertIsNone(plan["scoot"]["bump"])
        self.assertEqual(plan["scoot"]["last_tag"], "scoot-v0.2.0")
        self.w.commit("fix(scoot): redraw a popup")
        plan = self.w.plan()
        self.assertEqual(plan["scoot"]["next"], "0.2.1")
        self.assertEqual(plan["scoot"]["tag"], "scoot-v0.2.1")

    def test_plan_prints_tags_and_creates_nothing(self):
        self.w.commit("feat(scootbar): tray module")
        r = self.w.run("plan")
        self.assertEqual(r.returncode, 0)
        self.assertIn("scootbar-v0.2.0", r.stdout)
        self.assertIn("dry run only", r.stdout)
        tags = subprocess.run(["git", "tag"], cwd=self.w.root,
                              capture_output=True, text=True).stdout
        self.assertEqual(tags.strip(), "")

    def test_changelog_lists_the_scoped_commits(self):
        self.w.commit("feat(scootbar): skeleton")
        self.w.commit("docs(backlog): unrelated")
        self.w.commit("fix(scootbar): no hang")
        r = self.w.run("changelog", "scootbar")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("## scootbar 0.2.0", r.stdout)
        self.assertIn("feat(scootbar): skeleton", r.stdout)
        self.assertIn("fix(scootbar): no hang", r.stdout)
        self.assertNotIn("docs(backlog)", r.stdout)


class TestCheck(unittest.TestCase):
    def setUp(self):
        self.w = World()

    def tearDown(self):
        self.w.tearDown()

    def test_clean_tree_passes(self):
        r = self.w.run("check")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_trio_drift_fails(self):
        self.w.set_manifest("scootbg", 'version = "0.2.0"')
        r = self.w.run("check")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("lockstep", r.stdout)

    def test_bar_may_drift_alone(self):
        self.w.set_manifest("scootbar", 'version = "0.5.0"')
        r = self.w.run("check")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_workspace_version_is_not_per_package(self):
        self.w.set_manifest("scootbar", "version.workspace = true")
        r = self.w.run("check")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("own `version`", r.stdout)

    def test_non_semver_fails(self):
        self.w.set_manifest("scoot", 'version = "0.1"')
        r = self.w.run("check")
        self.assertNotEqual(r.returncode, 0)

    def test_internal_lib_must_be_private(self):
        (self.w.root / "crates" / "scoot-ipc" / "Cargo.toml").write_text(
            '[package]\nname = "scoot-ipc"\nversion.workspace = true\n')
        r = self.w.run("check")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("publish = false", r.stdout)

    def test_tag_ahead_of_manifest_fails_until_allowed(self):
        self.w.commit("feat(scoot): columns scroll")
        self.w.tag("scoot-v0.9.9")
        r = self.w.run("check")
        self.assertNotEqual(r.returncode, 0)
        r = self.w.run("check", "--allow-ahead")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
