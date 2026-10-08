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


def fake_source(versions):
    d = Path(tempfile.mkdtemp(prefix="deb-test-"))
    for crate, version in versions.items():
        cdir = d / "crates" / crate
        cdir.mkdir(parents=True)
        (cdir / "Cargo.toml").write_text(f'[package]\nname = "{crate}"\nversion = "{version}"\n')
    return d


class ControlTest(unittest.TestCase):
    def test_versions_read_the_manifests(self):
        p = run_meta("versions", "--source", str(REPO))
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("scoot 0.1.0", p.stdout)
        self.assertIn("scootbg 0.1.0", p.stdout)
        self.assertIn("scootbar 0.1.0", p.stdout)

    def test_scoot_control_pins_its_scootbg(self):
        src = fake_source({"scoot": "0.2.0", "scootbg": "0.2.0", "scootbar": "0.3.0"})
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

    def test_every_control_names_compositor_not_window_manager(self):
        for package in ("scoot", "scootbg", "scootbar"):
            p = run_meta(
                "control", "--package", package, "--source", str(REPO),
                "--arch", "amd64", "--depends", "libc6",
                "--scootbg-version", "0.1.0-1",
            )
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertNotIn("window manager", p.stdout.lower())
            self.assertNotIn("colour", p.stdout.lower())


class ShlibdepsTest(unittest.TestCase):
    # A realistic bookworm shlibdeps line for the compositor; the gate
    # passes it through verbatim (derived, not remembered).
    SCOOT_SHLIBS = (
        "shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 3.0), "
        "libinput10 (>= 1.9.0), libpixman-1-0 (>= 0.30.0), "
        "libseat1 (>= 0.4.0), libudev1 (>= 183), libxkbcommon0 (>= 0.8.0)"
    )
    BAR_SHLIBS = "shlibs:Depends=libc6 (>= 2.36), libgcc-s1 (>= 3.0)"

    def test_compositor_shlibdeps_passes_verbatim(self):
        p = run_meta("check-shlibdeps", "--package", "scoot", self.SCOOT_SHLIBS)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("libseat1 (>= 0.4.0)", p.stdout)

    def test_bar_shlibdeps_passes(self):
        p = run_meta("check-shlibdeps", "--package", "scootbar", self.BAR_SHLIBS)
        self.assertEqual(p.returncode, 0, p.stderr)

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
