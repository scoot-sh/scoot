#!/usr/bin/env python3
"""Tests for scripts/check-release-plz-crates, on throwaway git repos."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check-release-plz-crates")
BASE = '[workspace]\ngit_tag_name = "v{{ version }}"\n'


def entry(name):
    return f'\n[[package]]\nname = "{name}"\ngit_tag_name = "{name}-v{{{{ version }}}}"\n'


class Repo:
    def __init__(self, tmp):
        self.root = Path(tmp)
        self.git("init", "-q")

    def git(self, *args):
        subprocess.run(
            ["git", "-c", "user.email=t@t", "-c", "user.name=t", *args],
            cwd=self.root, check=True, capture_output=True,
        )

    def crate(self, name):
        path = self.root / "crates" / name / "Cargo.toml"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f'[package]\nname = "{name}"\n')

    def config(self, text):
        (self.root / "release-plz.toml").write_text(text)

    def commit(self, tag=None):
        self.git("add", "-A")
        self.git("commit", "-qm", "c", "--allow-empty")
        if tag:
            self.git("tag", tag)

    def run(self):
        out = subprocess.run(
            [sys.executable, str(SCRIPT), str(self.root)], capture_output=True, text=True
        )
        return out.returncode, out.stdout


class Check(unittest.TestCase):
    def repo(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        repo = Repo(tmp.name)
        repo.crate("old")
        repo.config(BASE)
        repo.commit("v1.0.0")
        return repo

    def test_a_new_crate_without_an_entry_fails_and_names_the_fix(self):
        repo = self.repo()
        repo.crate("fresh")
        repo.commit()
        code, out = repo.run()
        self.assertEqual(code, 1)
        self.assertIn('name = "fresh"', out)
        self.assertIn('git_tag_name = "fresh-v{{ version }}"', out)

    def test_a_new_crate_with_an_entry_passes(self):
        repo = self.repo()
        repo.crate("fresh")
        repo.config(BASE + entry("fresh"))
        repo.commit()
        self.assertEqual(repo.run()[0], 0)

    def test_an_entry_the_default_tag_name_does_not_count(self):
        repo = self.repo()
        repo.crate("fresh")
        repo.config(BASE + '\n[[package]]\nname = "fresh"\ngit_tag_name = "v{{ version }}"\n')
        repo.commit()
        self.assertEqual(repo.run()[0], 1)

    def test_an_entry_for_a_crate_that_does_not_exist_fails(self):
        repo = self.repo()
        repo.config(BASE + entry("ghost"))
        repo.commit()
        code, out = repo.run()
        self.assertEqual(code, 1)
        self.assertIn("ghost", out)

    def test_a_stale_entry_warns_and_passes(self):
        repo = self.repo()
        repo.config(BASE + entry("old"))
        repo.commit()
        code, out = repo.run()
        self.assertEqual(code, 0)
        self.assertIn("::warning", out)

    def test_no_tag_yet_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Repo(tmp)
            repo.crate("fresh")
            repo.config(BASE)
            repo.commit()
            self.assertEqual(repo.run()[0], 0)


if __name__ == "__main__":
    unittest.main()
