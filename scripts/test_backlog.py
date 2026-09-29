#!/usr/bin/env python3
"""Tests for scripts/backlog's claims and readiness. Run: python3 scripts/test_backlog.py

Each test builds a throwaway git repo with a copy of the script, so nothing
touches the real backlog or the real claims file.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "backlog"
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t"]


def entry(slug, priority="low", blocked=None, milestone=None, area="scootbar", status="open"):
    b = "null" if blocked is None else json.dumps(blocked)
    m = f'milestone: "{milestone}"\n' if milestone else ""
    return (f'---\ntitle: "{slug}"\nstatus: "{status}"\narea: "{area}"\n'
            f'priority: "{priority}"\nblocked: {b}\n{m}---\n\n# {slug}\n')


class Repo:
    def __init__(self):
        self.root = Path(tempfile.mkdtemp(prefix="backlog-test-")).resolve()
        (self.root / "scripts").mkdir()
        shutil.copy(SCRIPT, self.root / "scripts" / "backlog")
        for d in ("docs/backlog/core", "docs/backlog/ipc", "docs/backlog/protocols",
                  "docs/backlog/testing", "docs/backlog/packaging", "docs/backlog/resolved",
                  "docs/scootbg/backlog/resolved", "docs/scootbar/backlog/resolved"):
            (self.root / d).mkdir(parents=True)
        for f in ("docs/backlog/README.md", "docs/scootbg/backlog/README.md",
                  "docs/scootbar/backlog/README.md", "ROADMAP.md"):
            (self.root / f).write_text("")
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        self.env = {**os.environ, "BACKLOG_AGENT": "tester"}
        self.env.pop("BACKLOG_CLAIMS_FILE", None)

    def add(self, slug, **kw):
        p = self.root / "docs/scootbar/backlog" / f"{slug}.md"
        p.write_text(entry(slug, **kw))
        return p

    def run(self, *args, cwd=None):
        return subprocess.run([sys.executable, str((cwd or self.root) / "scripts/backlog"), *args],
                              cwd=cwd or self.root, env=self.env, capture_output=True, text=True)

    def claim_json(self, *args):
        r = self.run("claim", *args, "--json")
        return r, (json.loads(r.stdout) if r.returncode == 0 else None)

    def commit_all(self):
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True)
        subprocess.run([*GIT, "commit", "-qm", "init"], cwd=self.root, check=True)

    def cleanup(self):
        shutil.rmtree(self.root, ignore_errors=True)


class ClaimTests(unittest.TestCase):
    def setUp(self):
        self.r = Repo()
        self.addCleanup(self.r.cleanup)

    def test_claim_records_uuid_agent_and_date(self):
        self.r.add("a")
        _, c = self.r.claim_json("a")
        self.assertEqual(len(c["uuid"]), 36)
        self.assertEqual(c["agent"], "tester")
        self.assertRegex(c["claimed"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        stored = json.loads((self.r.root / ".git/backlog-claims.json").read_text())
        self.assertIn("docs/scootbar/backlog/a.md", stored["claims"])

    def test_the_claims_file_is_inside_git_so_never_tracked(self):
        self.r.add("a")
        self.r.claim_json("a")
        self.r.run("claims")
        self.assertEqual(subprocess.run(["git", "status", "--porcelain", "--ignored"],
                                        cwd=self.r.root, capture_output=True, text=True).stdout
                         .count("backlog-claims"), 0)

    def test_second_claim_is_refused_with_exit_2(self):
        self.r.add("a")
        self.r.claim_json("a")
        r, _ = self.r.claim_json("a", "--agent", "other")
        self.assertEqual(r.returncode, 2)
        self.assertIn("claimed", r.stderr)

    def test_the_holder_can_renew_with_its_token_and_keeps_the_uuid(self):
        self.r.add("a")
        _, c = self.r.claim_json("a")
        r, c2 = self.r.claim_json("a", "--token", c["uuid"])
        self.assertEqual(r.returncode, 0)
        self.assertEqual(c2["uuid"], c["uuid"])

    def test_force_takes_a_live_claim(self):
        self.r.add("a")
        _, c = self.r.claim_json("a")
        _, c2 = self.r.claim_json("a", "--force", "--agent", "thief")
        self.assertNotEqual(c["uuid"], c2["uuid"])

    def test_a_stale_claim_can_be_taken_without_force(self):
        self.r.add("a")
        self.r.claim_json("a", "--ttl", "0")
        r, _ = self.r.claim_json("a", "--agent", "next")
        self.assertEqual(r.returncode, 0)

    def test_release_needs_the_token(self):
        self.r.add("a")
        _, c = self.r.claim_json("a")
        self.assertNotEqual(self.r.run("release", "a").returncode, 0)
        self.assertEqual(self.r.run("release", "a", "--token", c["uuid"]).returncode, 0)
        self.assertEqual(self.r.claim_json("a")[0].returncode, 0)

    def test_resolving_someone_elses_live_claim_is_refused_then_allowed_with_token(self):
        self.r.add("a")
        _, c = self.r.claim_json("a")
        self.assertNotEqual(self.r.run("resolve", "a").returncode, 0)
        r = self.r.run("resolve", "a", "--token", c["uuid"])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads((self.r.root / ".git/backlog-claims.json").read_text())["claims"], {})

    def test_claims_prune_drops_stale_and_orphaned(self):
        self.r.add("a")
        self.r.add("b")
        self.r.claim_json("a", "--ttl", "0")
        self.r.claim_json("b")
        (self.r.root / "docs/scootbar/backlog/b.md").unlink()
        self.r.run("claims", "--prune")
        self.assertEqual(json.loads((self.r.root / ".git/backlog-claims.json").read_text())["claims"], {})

    def test_corrupt_claims_file_is_refused_not_clobbered(self):
        self.r.add("a")
        f = self.r.root / ".git/backlog-claims.json"
        f.write_text("{not json")
        r = self.r.run("claim", "a")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(f.read_text(), "{not json")

    def test_worktrees_of_one_clone_share_claims(self):
        self.r.add("a")
        self.r.commit_all()
        wt = self.r.root.parent / (self.r.root.name + "-wt")
        subprocess.run([*GIT, "worktree", "add", "-q", str(wt), "-b", "other"], cwd=self.r.root, check=True)
        self.addCleanup(shutil.rmtree, wt, True)
        self.assertEqual(self.r.claim_json("a")[0].returncode, 0)
        seen = self.r.run("claim", "a", cwd=wt)
        self.assertEqual(seen.returncode, 2, seen.stderr)


class NextTests(unittest.TestCase):
    def setUp(self):
        self.r = Repo()
        self.addCleanup(self.r.cleanup)

    def test_next_prefers_milestone_then_priority_and_skips_blocked_and_claimed(self):
        self.r.add("m2-high", priority="high", milestone="M2")
        self.r.add("m1-low", priority="low", milestone="M1")
        self.r.add("m1-high-blocked", priority="high", milestone="M1", blocked="something")
        self.r.add("m1-high", priority="high", milestone="M1")
        order = []
        for _ in range(3):
            _, c = self.r.claim_json("--next")
            order.append(c["slug"])
        self.assertEqual(order, ["m1-high", "m1-low", "m2-high"])
        r, _ = self.r.claim_json("--next")
        self.assertEqual(r.returncode, 3)

    def test_next_honours_area_and_milestone_filters(self):
        self.r.add("a", milestone="M1")
        self.r.add("b", milestone="M2")
        _, c = self.r.claim_json("--next", "--milestone", "M2")
        self.assertEqual(c["slug"], "b")
        self.assertEqual(self.r.claim_json("--next", "--area", "core")[0].returncode, 3)

    def test_next_skips_research_unless_asked(self):
        self.r.add("r", priority="research", status="research")
        self.assertEqual(self.r.claim_json("--next")[0].returncode, 3)
        self.assertEqual(self.r.claim_json("--next", "--research")[0].returncode, 0)

    def test_a_swarm_claiming_at_once_never_gets_the_same_ticket(self):
        for i in range(12):
            self.r.add(f"t{i:02}")
        with ThreadPoolExecutor(12) as pool:
            got = list(pool.map(lambda i: self.r.claim_json("--next", "--agent", f"a{i}"), range(12)))
        self.assertTrue(all(r.returncode == 0 for r, _ in got), [r.stderr for r, _ in got])
        self.assertEqual(len({c["slug"] for _, c in got}), 12)
        self.assertEqual(len({c["uuid"] for _, c in got}), 12)
        r, _ = self.r.claim_json("--next")
        self.assertEqual(r.returncode, 3)

    def test_list_ready_hides_blocked_and_claimed(self):
        self.r.add("free")
        self.r.add("held")
        self.r.add("waits", blocked="free")
        self.r.claim_json("held")
        out = self.r.run("list", "--ready").stdout
        self.assertIn("free", out)
        self.assertNotIn("held", out)
        self.assertNotIn("waits", out)


class UnblockTests(unittest.TestCase):
    def setUp(self):
        self.r = Repo()
        self.addCleanup(self.r.cleanup)

    def blocked(self, slug):
        return self.r.run("show", slug, "--full").stdout.split("blocked:")[1].split("\n")[0].strip()

    def test_resolving_clears_a_blocker_that_named_only_it(self):
        self.r.add("dep")
        self.r.add("x", blocked="dep")
        self.r.run("resolve", "dep")
        self.assertEqual(self.blocked("x"), "null")

    def test_list_and_annotation_forms(self):
        self.r.add("dep")
        self.r.add("other")
        self.r.add("both", blocked="dep, other")
        self.r.add("anded", blocked="dep and other")
        self.r.add("noted", blocked="dep (with a note)")
        self.r.add("first", blocked="other and dep")
        self.r.run("resolve", "dep")
        self.assertEqual(json.loads(self.blocked("both")), "other")
        self.assertEqual(json.loads(self.blocked("anded")), "other")
        self.assertEqual(self.blocked("noted"), "null")
        self.assertEqual(json.loads(self.blocked("first")), "other")

    def test_prose_is_left_alone_and_partial_slug_names_do_not_match(self):
        self.r.add("dep")
        self.r.add("dep-two")
        self.r.add("needs", blocked="dep and a real-world thing")
        self.r.add("partial", blocked="dep-two")
        self.r.run("resolve", "dep")
        self.assertEqual(json.loads(self.blocked("needs")), "a real-world thing")
        self.assertEqual(json.loads(self.blocked("partial")), "dep-two")


if __name__ == "__main__":
    unittest.main()
