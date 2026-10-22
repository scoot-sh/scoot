#!/usr/bin/env python3
"""Tests for the release tooling: scripts/release-resolve,
scripts/workspace-version and scripts/release-notes.
Run: python3 scripts/test_release_resolve.py

Each resolve test builds a throwaway git repo (fixture manifests, tags,
branches) and drives the real `scripts/release-resolve` CLI against it
with `--root`, so the parse, manifest-match, on-main and existing-release
paths are the ones that ship. The GitHub check runs against a stub `gh`
placed on PATH, never the network. Nothing touches the real repo.
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
CRATES = ("scoot", "scootctl", "scootbg", "scootbar")
WORKSPACE_VERSION = Path(__file__).resolve().parent / "workspace-version"
RELEASE_NOTES = Path(__file__).resolve().parent / "release-notes"

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
    """A fixture repo: a workspace version, four inheriting crates, a main
    branch, optional tags."""

    def __init__(self, version="0.2.0"):
        self.tmp = Path(tempfile.mkdtemp(prefix="release-resolve-test-"))
        self.root = self.tmp / "repo"
        self.root.mkdir()
        sh("git", "init", "-q", "-b", "main", cwd=self.root)
        sh("git", "config", "user.name", "t", cwd=self.root)
        sh("git", "config", "user.email", "t@t", cwd=self.root)
        sh("git", "config", "tag.gpgsign", "false", cwd=self.root)
        sh("git", "config", "commit.gpgsign", "false", cwd=self.root)
        # An origin/main to resolve against, like a CI checkout has.
        sh("git", "remote", "add", "origin", self.tmp / "upstream.git",
           cwd=self.root)
        self.set_workspace_version(version)
        for package in CRATES:
            self.set_manifest(package)
        self.commit("chore: init")
        sh("git", "update-ref", "refs/remotes/origin/main", "main",
           cwd=self.root)

    def set_workspace_version(self, version):
        (self.root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["crates/*"]\n\n'
            f'[workspace.package]\nversion = "{version}"\nedition = "2024"\n')
        sh("git", "add", "Cargo.toml", cwd=self.root)

    def set_manifest(self, package, version=None):
        d = self.root / "crates" / package
        d.mkdir(parents=True, exist_ok=True)
        line = (f'version = "{version}"' if version
                else "version.workspace = true")
        (d / "Cargo.toml").write_text(
            f'[package]\nname = "{package}"\n{line}\n'
            f'\n[dependencies]\nserde = {{ version = "1" }}\n')
        sh("git", "add", str(d / "Cargo.toml"), cwd=self.root)

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
# A published release with no assets yet (the --jq output: isDraft, then
# one asset name per line).
GH_PRESENT_BARE = "#!/bin/sh\necho false\nexit 0\n"


def gh_present(*names, draft=False):
    # Simulates `gh release view --json isDraft,assets --jq '.isDraft,
    # .assets[].name'`: the draft flag, then one asset name per line, like
    # the real --jq output.
    body = "\n".join([f'echo {"true" if draft else "false"}']
                      + [f'echo "{n}"' for n in names])
    return f"#!/bin/sh\n{body}\nexit 0\n"


GH_BROKEN = "#!/bin/sh\necho 'Bad credentials' >&2\nexit 1\n"


class ResolveTests(unittest.TestCase):
    def test_tag_resolves_every_binary(self):
        w = World()
        w.commit("feat(scoot): a thing", "a.txt")
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("version=0.2.0", r.stdout)
        self.assertIn("binaries=scoot scootbg scootbar scootlaunch scootnotify scoot-globalshortcuts-portal", r.stdout)

    def test_a_bar_only_change_still_releases_every_binary(self):
        # One version: a release is the whole suite, whatever moved.
        w = World()
        w.commit("feat(scootbar): a thing", "b.txt")
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", "--json", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(r.stdout)["binaries"],
                         ["scoot", "scootbg", "scootbar", "scootlaunch", "scootnotify", "scoot-globalshortcuts-portal"])

    def test_json_output_names_expected_assets(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", "--json", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        got = json.loads(r.stdout)
        self.assertEqual(got["tag"], "v0.2.0")
        self.assertEqual(got["version"], "0.2.0")
        for binary in ("scoot", "scootbg", "scootbar", "scootlaunch", "scootnotify", "scoot-globalshortcuts-portal"):
            for arch in ("x86_64-linux", "aarch64-linux"):
                self.assertIn(f"{binary}-0.2.0-{arch}", got["assets"])
        self.assertIn("SHA256SUMS", got["assets"])
        self.assertIn("sbom-scoot-0.2.0.cyclonedx.json", got["assets"])
        # Named by the version: the RPM specs' Source0 downloads this name.
        self.assertIn("scoot-0.2.0-vendored.tar.gz", got["assets"])
        self.assertTrue(any(n.endswith("-vendored.tar.gz")
                            for n in got["assets"]),
                        got["assets"])
        # 6 binaries x 2 arches, the tarball, the SBOM, the checksums.
        self.assertEqual(len(got["assets"]), 15, got["assets"])

    def test_bad_tag_shape_refused(self):
        w = World()
        for bad in ("0.2.0", "v0.2", "v0.2.0-rc1", "nightly", "vv0.2.0"):
            r = w.resolve(bad, gh_stub=GH_ABSENT)
            self.assertEqual(r.returncode, 2, bad)
            self.assertIn("not vX.Y.Z", r.stderr, bad)

    def test_old_per_package_tags_refused(self):
        # scoot-v*/scootbar-v*/scootctl-v* belonged to the retired
        # per-package scheme: refused by shape, before any lookup.
        w = World()
        for old in ("scoot-v0.2.0", "scootbar-v0.2.0", "scootctl-v0.2.0"):
            w.tag(old)
            r = w.resolve(old, gh_stub=GH_ABSENT)
            self.assertEqual(r.returncode, 2, old)
            self.assertIn("no per-package tags", r.stderr, old)

    def test_missing_tag_refused(self):
        w = World()
        r = w.resolve("v9.9.9", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("no such tag", r.stderr)

    def test_manifest_mismatch_refused(self):
        w = World()
        w.set_workspace_version("0.3.0")
        w.commit("chore: release v0.3.0")
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("workspace version", r.stderr)
        self.assertIn("0.3.0", r.stderr)

    def test_a_crate_on_its_own_version_refused(self):
        # The successor of the old lockstep gate: one crate pinning a
        # version of its own would ship a binary that disagrees with the tag.
        w = World()
        w.set_manifest("scootbar", "0.2.0")
        w.commit("chore(scootbar): pin its own version")
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("crates/scootbar/Cargo.toml", r.stderr)

    def test_the_tagged_commit_is_read_not_the_work_tree(self):
        # A dispatch checks out main, not the tag: the gate must judge the
        # tree the build will check out.
        w = World()
        w.tag("v0.2.0")
        w.set_workspace_version("0.3.0")
        w.commit("chore: release v0.3.0")
        w.push_main()
        self.assertIn('"0.3.0"', (w.root / "Cargo.toml").read_text())
        r = w.resolve("v0.2.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 0, r.stderr)
        r = w.resolve("v0.3.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("no such tag", r.stderr)

    def test_tag_not_on_main_refused(self):
        w = World()
        sh("git", "checkout", "-qb", "side", cwd=w.root)
        w.commit("feat(scootbar): off-main work", "side.txt")
        w.tag("v0.2.0", ref="side")
        sh("git", "checkout", "-q", "main", cwd=w.root)
        r = w.resolve("v0.2.0", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("not on main", r.stderr)

    def test_existing_complete_release_refused(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        probe = w.resolve("v0.2.0", "--json", gh_stub=GH_PRESENT_BARE)
        assets = json.loads(probe.stdout)["assets"]
        r = w.resolve("v0.2.0", gh_stub=gh_present(*assets))
        self.assertEqual(r.returncode, 2)
        self.assertIn("already published", r.stderr)

    def test_complete_release_is_done_with_complete_ok(self):
        # The workflow's mode: a maintainer's tag push and release-plz's
        # dispatch can both reach one tag; the second finds it complete
        # and stops green instead of publishing again.
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        probe = w.resolve("v0.2.0", "--json", gh_stub=GH_PRESENT_BARE)
        assets = json.loads(probe.stdout)["assets"]
        self.assertFalse(json.loads(probe.stdout)["complete"])
        r = w.resolve("v0.2.0", "--json", "--complete-ok",
                      gh_stub=gh_present(*assets))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue(json.loads(r.stdout)["complete"])
        self.assertIn("nothing to do", r.stderr)

    def test_complete_ok_still_resumes_an_incomplete_release(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", "--json", "--complete-ok",
                      gh_stub=gh_present("SHA256SUMS"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertFalse(json.loads(r.stdout)["complete"])
        self.assertIn("incomplete release resumes", r.stderr)

    def test_a_draft_holding_every_asset_resumes(self):
        # The publish step un-drafts last: a run that died just before it
        # left every asset in a draft. That must resume (and un-draft),
        # never count as published, in either mode.
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        probe = w.resolve("v0.2.0", "--json", gh_stub=GH_PRESENT_BARE)
        assets = json.loads(probe.stdout)["assets"]
        for extra in ((), ("--complete-ok",)):
            r = w.resolve("v0.2.0", "--json", *extra,
                          gh_stub=gh_present(*assets, draft=True))
            self.assertEqual(r.returncode, 0, (extra, r.stderr))
            self.assertFalse(json.loads(r.stdout)["complete"])
            self.assertIn("still a draft", r.stderr)

    def test_complete_ok_keeps_every_other_gate(self):
        w = World()
        w.set_workspace_version("0.3.0")
        w.commit("chore: release v0.3.0")
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", "--complete-ok", gh_stub=GH_ABSENT)
        self.assertEqual(r.returncode, 2)
        self.assertIn("workspace version", r.stderr)

    def test_existing_incomplete_release_resumes(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", gh_stub=GH_PRESENT_BARE)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("incomplete release resumes", r.stdout + r.stderr)

    def test_gh_failure_fails_closed(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", gh_stub=GH_BROKEN)
        self.assertEqual(r.returncode, 2)
        self.assertIn("could not verify", r.stderr)

    def test_no_github_skips_the_release_check(self):
        w = World()
        w.tag("v0.2.0")
        w.push_main()
        r = w.resolve("v0.2.0", "--no-github")
        self.assertEqual(r.returncode, 0, r.stderr)


class WorkspaceVersionTests(unittest.TestCase):
    def run_it(self, root, *args):
        return sh(sys.executable, str(WORKSPACE_VERSION), "--root",
                  str(root), *args, cwd=root, check=False)

    def test_prints_the_workspace_version(self):
        w = World("0.2.0")
        r = self.run_it(w.root)
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "0.2.0"), r.stderr)

    def test_rolling_is_the_next_minor_of_the_same_major(self):
        # Above every vX.Y.* tag of the current workspace version, and
        # (from 0.2.0) above the 0.1.<count> rolling versions FlakeHub
        # published before this scheme.
        for version, rolling in (("0.2.0", "0.3"), ("0.2.7", "0.3"),
                                 ("0.9.0", "0.10"), ("1.0.0", "1.1"),
                                 ("1.4.9", "1.5")):
            w = World(version)
            r = self.run_it(w.root, "--rolling")
            self.assertEqual((r.returncode, r.stdout.strip()), (0, rolling),
                             (version, r.stderr))

    def test_check_passes_when_every_crate_inherits(self):
        w = World()
        r = self.run_it(w.root, "--check")
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_check_names_a_crate_with_its_own_version(self):
        w = World()
        w.set_manifest("scootbg", "0.1.0")
        r = self.run_it(w.root, "--check")
        self.assertEqual(r.returncode, 1)
        self.assertIn("crates/scootbg/Cargo.toml", r.stderr)
        self.assertNotIn("crates/scoot/Cargo.toml", r.stderr)

    def test_a_version_elsewhere_in_a_crate_manifest_is_not_its_own(self):
        # `version = "1"` under [dependencies] is a dependency, not the
        # crate's version: the fixture manifests carry one.
        w = World()
        self.assertIn('version = "1"',
                      (w.root / "crates/scoot/Cargo.toml").read_text())
        r = self.run_it(w.root, "--check")
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_a_non_semver_workspace_version_is_an_error(self):
        w = World("0.2")
        r = self.run_it(w.root)
        self.assertEqual(r.returncode, 1)
        self.assertIn("not X.Y.Z", r.stderr)

    def test_a_missing_workspace_package_table_is_an_error(self):
        w = World()
        (w.root / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        r = self.run_it(w.root)
        self.assertEqual(r.returncode, 1)
        self.assertIn("[workspace.package]", r.stderr)

    def test_the_real_repository_is_one_version(self):
        root = Path(__file__).resolve().parent.parent
        r = self.run_it(root, "--check")
        self.assertEqual(r.returncode, 0, r.stderr)


CHANGELOG = """# Changelog

Intro text.

## Unreleased

- hand-written note for the next release

## [0.2.1](https://github.com/scoot-sh/scoot/compare/v0.2.0...v0.2.1) - 2026-10-20

### Fixed

- *(scootbar)* a fix

## [0.2.0] - 2026-10-14

- the first release
"""


class ReleaseNotesTests(unittest.TestCase):
    def notes(self, version, text=CHANGELOG):
        tmp = Path(tempfile.mkdtemp(prefix="release-notes-test-"))
        (tmp / "CHANGELOG.md").write_text(text)
        return sh(sys.executable, str(RELEASE_NOTES), version, "--changelog",
                  str(tmp / "CHANGELOG.md"), cwd=tmp, check=False)

    def test_a_linked_dated_section(self):
        r = self.notes("0.2.1")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "### Fixed\n\n- *(scootbar)* a fix")

    def test_a_plain_dated_section_and_a_leading_v(self):
        r = self.notes("v0.2.0")
        self.assertEqual(r.stdout.strip(), "- the first release")

    def test_a_version_is_not_matched_as_a_prefix(self):
        r = self.notes("0.2.1", CHANGELOG.replace("[0.2.1]", "[0.2.10]"))
        self.assertIn("no CHANGELOG.md section", r.stderr)

    def test_a_missing_section_points_at_the_changelog(self):
        r = self.notes("9.9.9")
        self.assertEqual(r.returncode, 0)
        self.assertIn("no CHANGELOG.md section for 9.9.9", r.stderr)
        self.assertIn("blob/v9.9.9/CHANGELOG.md", r.stdout)


class SyncFuzzLocksTests(unittest.TestCase):
    SCRIPT = Path(__file__).resolve().parent / "sync-fuzz-locks"

    def test_workspace_crates_follow_the_version_and_others_stay(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.1"\n')
            crate = root / "crates" / "scootbg-mem"
            (crate).mkdir(parents=True)
            (crate / "Cargo.toml").write_text('[package]\nname = "scootbg-mem"\n')
            fuzz = root / "crates" / "scootbg" / "fuzz"
            fuzz.mkdir(parents=True)
            lock = fuzz / "Cargo.lock"
            lock.write_text('[[package]]\nname = "scootbg-mem"\nversion = "0.2.0"\n\n'
                            '[[package]]\nname = "serde"\nversion = "0.2.0"\n\n'
                            '[[package]]\nname = "scootbg-mem"\nversion = "9.0.0"\n'
                            'source = "registry+https://github.com/rust-lang/crates.io-index"\n')
            r = subprocess.run([str(self.SCRIPT), "--root", d], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn('name = "scootbg-mem"\nversion = "0.2.1"', lock.read_text())
            self.assertIn('name = "serde"\nversion = "0.2.0"', lock.read_text())
            # A registry crate sharing a workspace crate's name is not rewritten.
            self.assertIn('name = "scootbg-mem"\nversion = "9.0.0"', lock.read_text())


if __name__ == "__main__":
    unittest.main()
