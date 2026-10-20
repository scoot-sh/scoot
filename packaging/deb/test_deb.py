#!/usr/bin/env python3
"""Tests for deb-meta.py: control rendering and the shlibdeps allow-list.

Run: python3 packaging/deb/test_deb.py (also a step in deb.yml).
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
META = HERE / "deb-meta.py"
REPO = HERE.parent.parent


def run_meta(*args):
    p = subprocess.run(
        [sys.executable, str(META), *args],
        capture_output=True, text=True, check=False,
    )
    return p


def fake_source(version):
    """A source tree with the one workspace version every crate inherits."""
    d = Path(tempfile.mkdtemp(prefix="deb-test-"))
    (d / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/*"]\n\n'
        f'[workspace.package]\nversion = "{version}"\nedition = "2024"\n\n'
        '[workspace.dependencies]\nserde = { version = "1" }\n')
    for crate in ("scoot", "scootbg", "scootbar", "scootlaunch"):
        cdir = d / "crates" / crate
        cdir.mkdir(parents=True)
        (cdir / "Cargo.toml").write_text(
            f'[package]\nname = "{crate}"\nversion.workspace = true\n')
    return d


class ControlTest(unittest.TestCase):
    def test_versions_read_the_workspace_version(self):
        p = run_meta("versions", "--source", str(fake_source("0.4.2")))
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(p.stdout, "scoot 0.4.2\nscootbar 0.4.2\nscootbg 0.4.2\nscootlaunch 0.4.2\n")

    def test_versions_of_this_tree_agree(self):
        p = run_meta("versions", "--source", str(REPO))
        self.assertEqual(p.returncode, 0, p.stderr)
        versions = {line.split()[1] for line in p.stdout.splitlines()}
        self.assertEqual(len(versions), 1, p.stdout)

    def test_a_tree_without_a_workspace_version_is_refused(self):
        d = fake_source("0.2.0")
        (d / "Cargo.toml").write_text('[workspace]\nmembers = ["crates/*"]\n')
        p = run_meta("versions", "--source", str(d))
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("[workspace.package] version", p.stderr)

    def test_scoot_control_pins_its_scootbg(self):
        src = fake_source("0.2.0")
        p = run_meta(
            "control", "--package", "scoot", "--source", str(src),
            "--arch", "amd64", "--depends", "libc6 (>= 2.36)",
            "--scootbg-version", "0.2.0-1",
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        for line in (
            "Package: scoot",
            "Version: 0.2.0-1",
            "Architecture: amd64",
            "Depends: libc6 (>= 2.36), scootbg (= 0.2.0-1)",
        ):
            self.assertIn(line, p.stdout)

    def test_a_snapshot_suffix_rides_on_the_upstream_version(self):
        src = fake_source("0.2.0")
        p = run_meta(
            "control", "--package", "scoot", "--source", str(src),
            "--arch", "amd64", "--depends", "libc6",
            "--upstream-suffix", "+git20261013.615608c",
            "--scootbg-version", "0.2.0+git20261013.615608c-1",
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("Version: 0.2.0+git20261013.615608c-1", p.stdout)
        self.assertIn("scootbg (= 0.2.0+git20261013.615608c-1)", p.stdout)

    def test_a_malformed_snapshot_suffix_is_refused(self):
        src = fake_source("0.2.0")
        for bad in ("git20261013.615608c", "+git2026.615608c", "+git20261013.XYZ", "-dirty"):
            p = run_meta(
                "control", "--package", "scootbar", "--source", str(src),
                "--arch", "amd64", "--depends", "libc6", "--upstream-suffix", bad,
            )
            self.assertNotEqual(p.returncode, 0, bad)

    def test_scoot_needs_its_scootbg_version(self):
        # Rendering scoot without the coupled version is a loud refusal,
        # not a control with a silently missing dependency.
        p = run_meta(
            "control", "--package", "scoot", "--source", str(REPO),
            "--arch", "amd64", "--depends", "libc6",
        )
        self.assertNotEqual(p.returncode, 0)

    def test_bar_control_suggests_a_font(self):
        p = run_meta(
            "control", "--package", "scootbar", "--source", str(REPO),
            "--arch", "arm64", "--depends", "libc6",
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("Architecture: arm64", p.stdout)
        self.assertIn("Suggests: fonts-dejavu-core", p.stdout)

    def test_launch_control_suggests_a_font(self):
        # The panel refuses to open with no font (unlike the bar, which
        # draws a plain one), but the headless commands need none, so
        # the font stays a suggestion, not a dependency.
        p = run_meta(
            "control", "--package", "scootlaunch", "--source", str(REPO),
            "--arch", "amd64", "--depends", "libc6",
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("Package: scootlaunch", p.stdout)
        self.assertIn("Suggests: fonts-dejavu-core", p.stdout)
        self.assertIn("Depends: libc6", p.stdout)
        self.assertNotIn("fonts-dejavu-core", [
            line for line in p.stdout.splitlines()
            if line.startswith("Depends:")][0])

    def test_every_control_names_compositor_not_window_manager(self):
        for package in ("scoot", "scootbg", "scootbar", "scootlaunch"):
            p = run_meta(
                "control", "--package", package, "--source", str(REPO),
                "--arch", "amd64", "--depends", "libc6",
                "--scootbg-version", "0.1.0-1",
            )
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertNotIn("window manager", p.stdout.lower())
            self.assertNotIn("colour", p.stdout.lower())

    def test_no_source_only_fields_in_a_binary_control(self):
        # Standards-Version lives in debian/control, not in an
        # installed binary control: bookworm lintian flags it as
        # `unknown-field` there.
        p = run_meta(
            "control", "--package", "scoot", "--source", str(REPO),
            "--arch", "amd64", "--depends", "libc6",
            "--scootbg-version", "0.1.0-1",
        )
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertNotIn("Standards-Version", p.stdout)


class ShlibdepsTest(unittest.TestCase):
    # A realistic bookworm shlibdeps line for the compositor; the gate
    # passes it through verbatim (derived, not remembered).
    SCOOT_SHLIBS = (
        "shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 3.0), "
        "libinput10 (>= 1.9.0), libpixman-1-0 (>= 0.30.0), "
        "libseat1 (>= 0.4.0), libudev1 (>= 183), libxkbcommon0 (>= 0.8.0)"
    )
    BAR_SHLIBS = "shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 3.0)"
    LAUNCH_SHLIBS = (
        "shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 3.0), "
        "libxkbcommon0 (>= 0.8.0)"
    )

    def test_compositor_shlibdeps_passes_verbatim(self):
        p = run_meta("check-shlibdeps", "--package", "scoot", self.SCOOT_SHLIBS)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("libseat1 (>= 0.4.0)", p.stdout)

    def test_bar_shlibdeps_passes(self):
        p = run_meta("check-shlibdeps", "--package", "scootbar", self.BAR_SHLIBS)
        self.assertEqual(p.returncode, 0, p.stderr)

    def test_launch_shlibdeps_passes_with_only_the_keyboard_library(self):
        # The panel links libxkbcommon past libc and nothing else: the
        # keyboard library passes, a compositor library fails.
        p = run_meta("check-shlibdeps", "--package", "scootlaunch", self.LAUNCH_SHLIBS)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("libxkbcommon0 (>= 0.8.0)", p.stdout)

    def test_launch_must_not_link_the_compositor_stack(self):
        p = run_meta(
            "check-shlibdeps", "--package", "scootlaunch",
            self.LAUNCH_SHLIBS + ", libinput10 (>= 1.9.0)",
        )
        self.assertNotEqual(p.returncode, 0)

    def test_a_new_library_is_a_loud_refusal(self):
        # A dependency outside the ldd-derived table (a renamed package,
        # a new link) fails the build instead of shipping a wrong
        # Depends: the fixer updates ALLOW and the README table, from
        # ldd -- not memory.
        p = run_meta(
            "check-shlibdeps", "--package", "scoot",
            self.SCOOT_SHLIBS + ", libsurprise99 (>= 1.0)",
        )
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("libsurprise99", p.stderr)

    def test_bar_must_not_link_the_gpu_stack(self):
        # The libc-only contract for the bar and the wallpaper daemon:
        # a compositor library leaking into either fails here.
        for package in ("scootbar", "scootbg"):
            p = run_meta(
                "check-shlibdeps", "--package", package,
                self.BAR_SHLIBS + ", libinput10 (>= 1.9.0)",
            )
            self.assertNotEqual(p.returncode, 0)

    def test_garbage_shlibdeps_is_a_loud_refusal(self):
        p = run_meta("check-shlibdeps", "--package", "scoot", "nothing useful here")
        self.assertNotEqual(p.returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
