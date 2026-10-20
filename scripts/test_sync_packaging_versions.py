#!/usr/bin/env python3
"""Tests for scripts/sync-packaging-versions.
Run: python3 scripts/test_sync_packaging_versions.py

Each test builds a throwaway root (a workspace Cargo.toml, one release
plus one -git PKGBUILD with .SRCINFO, one RPM spec) and drives the real
script with `--root`, so the edits under test are the ones that ship.
Nothing touches the real repo.
"""

import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "sync-packaging-versions"
DATE = "Tue Oct 20 2026"

PKGBUILD = """pkgname=@PKG@
pkgver=OLDVER
pkgrel=1
@DEPENDS@
source=("$pkgname-$pkgver.tar.gz::https://github.com/scoot-sh/scoot/archive/refs/tags/v$pkgver.tar.gz")
"""

SRCINFO = """pkgbase = @PKG@
\tpkgver = OLDVER
\tpkgrel = 1
@DEP@
\tsource = @PKG@-OLDVER.tar.gz::https://github.com/scoot-sh/scoot/archive/refs/tags/vOLDVER.tar.gz

pkgname = @PKG@
"""

GIT_PKGBUILD = """pkgname=@PKG@-git
pkgver=OLDVER.r0.g0000000
pkgrel=1
"""

GIT_SRCINFO = """pkgbase = @PKG@-git
\tpkgver = OLDVER.r0.g0000000
\tpkgrel = 1

pkgname = @PKG@-git
"""

SPEC = """Name:           scoot
Version:        OLDVER
Release:        1%{?dist}
Requires:       scootbg = %{version}-%{release}

%changelog
* Wed Oct 14 2026 scoot-sh <packages@scoot.sh> - OLDVER-1
- Built from the vOLDVER release tag.
"""


class World:
    """A fixture root at one workspace version with stale packaging."""

    def __init__(self, workspace="0.3.1", packaged="0.2.1"):
        self.tmp = tempfile.TemporaryDirectory(prefix="sync-packaging-")
        self.root = Path(self.tmp.name)
        (self.root / "Cargo.toml").write_text(
            '[workspace.package]\nversion = "%s"\n' % workspace)
        arch = self.root / "packaging" / "arch"
        for pkg in ("scoot", "scootbg", "scootbar", "scootlaunch"):
            depends = ("depends=('glibc' \"scootbg=$pkgver\")"
                       if pkg == "scoot" else "depends=('glibc')")
            dep = ("\tdepends = scootbg=OLDVER\n" if pkg == "scoot" else "")
            d = arch / pkg
            d.mkdir(parents=True)
            (d / "PKGBUILD").write_text(
                PKGBUILD.replace("@PKG@", pkg)
                .replace("@DEPENDS@", depends)
                .replace("OLDVER", packaged))
            (d / ".SRCINFO").write_text(
                SRCINFO.replace("@PKG@", pkg)
                .replace("@DEP@\n", dep)
                .replace("OLDVER", packaged))
            d = arch / f"{pkg}-git"
            d.mkdir()
            (d / "PKGBUILD").write_text(
                GIT_PKGBUILD.replace("@PKG@", pkg)
                .replace("OLDVER", packaged))
            (d / ".SRCINFO").write_text(
                GIT_SRCINFO.replace("@PKG@", pkg)
                .replace("OLDVER", packaged))
        rpm = self.root / "packaging" / "rpm"
        rpm.mkdir(parents=True)
        for spec in ("scoot", "scootbg", "scootbar", "scootlaunch"):
            (rpm / f"{spec}.spec").write_text(
                SPEC.replace("OLDVER", packaged))

    def run(self, *args):
        return subprocess.run([str(SCRIPT), "--root", str(self.root),
                               "--date", DATE, *args],
                              capture_output=True, text=True)

    def close(self):
        self.tmp.cleanup()


class SyncPackagingVersionsTests(unittest.TestCase):
    def setUp(self):
        self.w = World()

    def tearDown(self):
        self.w.close()

    def test_release_pkgbuild_srcinfo_and_spec_follow_the_version(self):
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        pkgbuild = (self.w.root / "packaging" / "arch" / "scoot"
                    / "PKGBUILD").read_text()
        self.assertIn("\npkgver=0.3.1\n", pkgbuild)
        srcinfo = (self.w.root / "packaging" / "arch" / "scoot"
                   / ".SRCINFO").read_text()
        self.assertIn("\tpkgver = 0.3.1\n", srcinfo)
        self.assertIn("\tdepends = scootbg=0.3.1\n", srcinfo)
        self.assertIn("\tsource = scoot-0.3.1.tar.gz::"
                      "https://github.com/scoot-sh/scoot"
                      "/archive/refs/tags/v0.3.1.tar.gz\n", srcinfo)
        self.assertNotIn("0.2.1", srcinfo)
        spec = (self.w.root / "packaging" / "rpm" / "scoot.spec"
                ).read_text()
        self.assertIn("\nVersion:        0.3.1\n", spec)
        self.assertIn(f"* {DATE} scoot-sh <packages@scoot.sh> - 0.3.1-1\n"
                      "- Built from the v0.3.1 release tag.\n", spec)
        # The previous entry survives byte-identical (the splice only
        # prepends; a regression here ate the old entry's first line).
        self.assertIn("* Wed Oct 14 2026 scoot-sh <packages@scoot.sh>"
                      " - 0.2.1-1\n"
                      "- Built from the v0.2.1 release tag.\n", spec)

    def test_git_pkgbuilds_track_the_static_base(self):
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        pkgbuild = (self.w.root / "packaging" / "arch" / "scoot-git"
                    / "PKGBUILD").read_text()
        self.assertIn("\npkgver=0.3.1.r0.g0000000\n", pkgbuild)
        srcinfo = (self.w.root / "packaging" / "arch" / "scoot-git"
                   / ".SRCINFO").read_text()
        self.assertIn("\tpkgver = 0.3.1.r0.g0000000\n", srcinfo)

    def test_second_run_changes_nothing(self):
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        before = {p: p.read_bytes() for p in self.w.root.rglob("*")
                  if p.is_file() and p.name != "Cargo.toml"}
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("nothing to do", r.stdout)
        for p, content in before.items():
            self.assertEqual(p.read_bytes(), content, p)
        spec = (self.w.root / "packaging" / "rpm" / "scoot.spec"
                ).read_text()
        self.assertEqual(spec.count("- 0.3.1-1"), 1)

    def test_version_already_current_is_a_no_op(self):
        w = World(workspace="0.2.1", packaged="0.2.1")
        try:
            r = w.run()
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("nothing to do", r.stdout)
        finally:
            w.close()

    def test_release_is_reset_only_when_the_version_moves(self):
        spec = self.w.root / "packaging" / "rpm" / "scoot.spec"
        text = spec.read_text().replace("Release:        1%{?dist}",
                                        "Release:        3%{?dist}")
        spec.write_text(text)
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("\nRelease:        1%{?dist}\n", spec.read_text())
        # A second run leaves the reset Release alone.
        r = self.w.run()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("\nRelease:        1%{?dist}\n", spec.read_text())


if __name__ == "__main__":
    unittest.main()
