#!/usr/bin/env python3
"""Tests for scripts/backlog's claims and readiness. Run: python3 scripts/test_backlog.py

Each test builds a bare "origin" with a main branch and separate clones of it,
each with its own copy of the script, so "another agent on another machine" is
a real second clone. Nothing touches the real backlog or its claims.
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
CLAIM_A = "docs/backlog/claims/scootbar__backlog__a.json"


def entry(slug, priority="low", blocked=None, milestone=None, area="scootbar", status="open"):
    b = "null" if blocked is None else json.dumps(blocked)
    m = f'milestone: "{milestone}"\n' if milestone else ""
    return (f'---\ntitle: "{slug}"\nstatus: "{status}"\narea: "{area}"\n'
            f'priority: "{priority}"\nblocked: {b}\n{m}---\n\n# {slug}\n')


def sh(*args, cwd, check=True, env=None):
    return subprocess.run(args, cwd=cwd, check=check, capture_output=True, text=True, env=env)


class Clone:
    def __init__(self, world, name):
        self.root = world.tmp / name
        sh("git", "clone", "-q", str(world.origin), str(self.root), cwd=world.tmp)
        sh("git", "config", "user.name", name, cwd=self.root)
        sh("git", "config", "user.email", f"{name}@t", cwd=self.root)
        self.env = {**os.environ, "BACKLOG_AGENT": name}
        for k in ("BACKLOG_CLAIMS_FILE", "BACKLOG_REMOTE", "BACKLOG_CLAIM_BRANCH"):
            self.env.pop(k, None)

    def git(self, *args, check=True):
        return sh("git", *args, cwd=self.root, check=check)

    def add(self, slug, **kw):
        """Put an entry on main: write, commit, push."""
        self.git("pull", "--rebase", "-q", "origin", "main")
        (self.root / "docs/scootbar/backlog" / f"{slug}.md").write_text(entry(slug, **kw))
        self.git("add", "-A")
        self.git("commit", "-qm", f"add {slug}")
        self.git("push", "-q", "origin", "HEAD:main")

    def run(self, *args, env=None):
        return subprocess.run([sys.executable, str(self.root / "scripts/backlog"), *args],
                              cwd=self.root, env={**self.env, **(env or {})},
                              capture_output=True, text=True)

    def claim(self, *args, env=None):
        r = self.run("claim", *args, "--json", env=env)
        return r, (json.loads(r.stdout) if r.returncode == 0 else None)

    def head(self):
        return self.git("rev-parse", "HEAD").stdout.strip()

    def local(self):
        p = self.root / ".git/backlog-claims.json"
        return json.loads(p.read_text())["claims"] if p.exists() else {}


class World:
    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="backlog-test-")).resolve()
        self.origin = self.tmp / "origin.git"
        sh("git", "init", "-q", "--bare", "--initial-branch=main", str(self.origin), cwd=self.tmp)
        seed = self.tmp / "seed"
        sh("git", "init", "-q", "--initial-branch=main", str(seed), cwd=self.tmp)
        sh("git", "config", "user.name", "seed", cwd=seed)
        sh("git", "config", "user.email", "seed@t", cwd=seed)
        (seed / "scripts").mkdir()
        shutil.copy(SCRIPT, seed / "scripts/backlog")
        for d in ("docs/backlog/core", "docs/backlog/ipc", "docs/backlog/protocols",
                  "docs/backlog/testing", "docs/backlog/packaging", "docs/backlog/resolved",
                  "docs/scootbg/backlog/resolved", "docs/scootbar/backlog/resolved"):
            (seed / d).mkdir(parents=True)
            (seed / d / ".gitkeep").write_text("")
        for f in ("docs/backlog/README.md", "docs/scootbg/backlog/README.md",
                  "docs/scootbar/backlog/README.md", "ROADMAP.md"):
            (seed / f).write_text("")
        sh("git", "add", "-A", cwd=seed)
        sh("git", "commit", "-qm", "seed", cwd=seed)
        sh("git", "remote", "add", "origin", str(self.origin), cwd=seed)
        sh("git", "push", "-q", "origin", "main", cwd=seed)

    def clone(self, name):
        return Clone(self, name)

    def on_origin(self, path):
        r = sh("git", "show", f"main:{path}", cwd=self.origin, check=False)
        return r.stdout if r.returncode == 0 else None

    def cleanup(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


class Base(unittest.TestCase):
    def setUp(self):
        self.w = World()
        self.addCleanup(self.w.cleanup)
        self.a = self.w.clone("alice")


class ClaimTests(Base):
    def test_a_claim_is_a_commit_on_main_and_touches_nothing_local(self):
        self.a.add("a")
        head, branch = self.a.head(), self.a.git("branch", "--show-current").stdout
        r, c = self.a.claim("a")
        self.assertEqual(r.returncode, 0, r.stderr)
        stored = json.loads(self.w.on_origin(CLAIM_A))
        self.assertEqual(stored["uuid"], c["uuid"])
        self.assertEqual(stored["agent"], "alice")
        self.assertRegex(stored["claimed"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        log = sh("git", "log", "-1", "--format=%s", "main", cwd=self.w.origin).stdout
        self.assertIn("claim a (alice)", log)
        self.assertIn("[skip ci]", log)
        self.assertEqual(self.a.head(), head)  # the checked-out branch did not move
        self.assertEqual(self.a.git("branch", "--show-current").stdout, branch)
        self.assertEqual(self.a.git("status", "--porcelain").stdout, "")  # working tree clean
        self.assertEqual(self.a.local()["docs/scootbar/backlog/a.md"]["uuid"], c["uuid"])

    def test_the_local_record_is_inside_git_so_never_tracked(self):
        self.a.add("a")
        self.a.claim("a")
        self.assertEqual(self.a.git("status", "--porcelain", "--ignored").stdout.count("backlog-claims"), 0)
        self.assertNotIn("backlog-claims", self.a.git("ls-files").stdout)

    def test_another_clone_is_refused_without_ever_pulling(self):
        self.a.add("a")
        self.a.claim("a")
        b = self.w.clone("bob")  # cloned after: its checkout has the claim, but stale ones must not matter
        b.git("reset", "-q", "--hard", "HEAD~1")  # a checkout from before the claim
        self.assertFalse((b.root / CLAIM_A).exists())
        r, _ = b.claim("a")
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("alice", r.stderr)

    def test_an_entry_not_on_main_cannot_be_claimed(self):
        (self.a.root / "docs/scootbar/backlog/local-only.md").write_text(entry("local-only"))
        r, _ = self.a.claim("local-only")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("push it to main first", r.stderr)

    def test_the_holder_renews_with_its_recorded_uuid_and_keeps_it(self):
        self.a.add("a")
        _, c = self.a.claim("a")
        r = self.a.run("renew", "a")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(json.loads(self.w.on_origin(CLAIM_A))["uuid"], c["uuid"])

    def test_force_takes_a_live_claim_and_a_stale_claim_needs_no_force(self):
        self.a.add("a")
        _, c = self.a.claim("a")
        b = self.w.clone("bob")
        self.assertEqual(b.claim("a", "--force")[0].returncode, 0)
        self.assertNotEqual(json.loads(self.w.on_origin(CLAIM_A))["uuid"], c["uuid"])
        self.a.add("s")
        self.a.claim("s", "--ttl", "0")
        self.assertEqual(b.claim("s")[0].returncode, 0)

    def test_release_needs_the_token_then_removes_the_claim_from_main(self):
        self.a.add("a")
        _, c = self.a.claim("a")
        b = self.w.clone("bob")
        self.assertEqual(b.run("release", "a").returncode, 2)
        self.assertIsNotNone(self.w.on_origin(CLAIM_A))
        self.assertEqual(b.run("release", "a", "--token", c["uuid"]).returncode, 0)
        self.assertIsNone(self.w.on_origin(CLAIM_A))
        self.assertEqual(b.claim("a")[0].returncode, 0)

    def test_resolve_refuses_someone_elses_live_claim_but_takes_the_token(self):
        self.a.add("a")
        _, c = self.a.claim("a")
        b = self.w.clone("bob")
        self.assertNotEqual(b.run("resolve", "a").returncode, 0)
        self.assertEqual(b.run("resolve", "a", "--token", c["uuid"]).returncode, 0)
        self.assertEqual(self.a.run("resolve", "a").returncode, 0)  # its own uuid is on file

    def test_a_claim_on_a_resolved_entry_is_an_orphan_and_prune_removes_it(self):
        self.a.add("a")
        self.a.claim("a")
        self.a.git("pull", "--rebase", "-q", "origin", "main")
        self.a.run("resolve", "a")
        self.a.git("add", "-A")
        self.a.git("commit", "-qm", "resolve a")
        self.a.git("push", "-q", "origin", "HEAD:main")
        self.assertIn("orphan", self.a.run("claims").stdout)
        self.assertEqual(self.a.run("claims", "--prune").returncode, 0)
        self.assertIsNone(self.w.on_origin(CLAIM_A))

    def test_the_next_claim_tidies_dead_claims_in_the_same_commit(self):
        self.a.add("old")
        self.a.claim("old", "--ttl", "0")
        self.a.add("new")
        self.a.claim("new")
        self.assertIsNone(self.w.on_origin("docs/backlog/claims/scootbar__backlog__old.json"))
        self.assertIsNotNone(self.w.on_origin("docs/backlog/claims/scootbar__backlog__new.json"))

    def test_an_unreachable_remote_is_exit_4_not_a_local_claim(self):
        self.a.add("a")
        r, _ = self.a.claim("a", env={"BACKLOG_REMOTE": "nowhere"})
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertEqual(self.a.local(), {})

    def test_a_protected_main_is_a_clear_error(self):
        self.a.add("a")
        hook = self.w.origin / "hooks/pre-receive"
        hook.write_text("#!/bin/sh\necho 'GH006: protected branch' >&2\nexit 1\n")
        hook.chmod(0o755)
        r, _ = self.a.claim("a")
        self.assertEqual(r.returncode, 4)
        self.assertIn("push access", r.stderr)
        self.assertEqual(self.a.local(), {})

    def test_a_corrupt_local_record_is_refused_not_clobbered(self):
        self.a.add("a")
        f = self.a.root / ".git/backlog-claims.json"
        f.write_text("{not json")
        r, _ = self.a.claim("a")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(f.read_text(), "{not json")

    def test_two_worktrees_of_one_clone_both_see_main(self):
        self.a.add("a")
        wt = self.w.tmp / "wt"
        self.a.git("worktree", "add", "-q", str(wt), "-b", "other")
        self.a.claim("a")
        r = subprocess.run([sys.executable, str(wt / "scripts/backlog"), "claim", "a"],
                           cwd=wt, env={**self.a.env, "BACKLOG_AGENT": "wt"},
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2, r.stderr)


class NextTests(Base):
    def test_next_prefers_milestone_then_priority_and_skips_blocked_and_claimed(self):
        self.a.add("m2-high", priority="high", milestone="M2")
        self.a.add("m1-low", priority="low", milestone="M1")
        self.a.add("m1-high-blocked", priority="high", milestone="M1", blocked="something")
        self.a.add("m1-high", priority="high", milestone="M1")
        order = [self.a.claim("--next")[1]["slug"] for _ in range(3)]
        self.assertEqual(order, ["m1-high", "m1-low", "m2-high"])
        self.assertEqual(self.a.claim("--next")[0].returncode, 3)

    def test_next_from_another_clone_skips_what_main_says_is_claimed(self):
        self.a.add("first", priority="high")
        self.a.add("second", priority="low")
        self.a.claim("--next")
        b = self.w.clone("bob")
        b.git("reset", "-q", "--hard", "HEAD~1")  # stale checkout: it must still see main
        self.assertEqual(b.claim("--next")[1]["slug"], "second")

    def test_next_reads_readiness_from_main_not_the_local_checkout(self):
        self.a.add("dep")
        self.a.add("waits", blocked="dep")
        b = self.w.clone("bob")
        self.a.git("pull", "--rebase", "-q", "origin", "main")
        self.a.run("resolve", "dep")
        self.a.git("add", "-A")
        self.a.git("commit", "-qm", "resolve dep")
        self.a.git("push", "-q", "origin", "HEAD:main")
        # bob's checkout still shows `waits` blocked and `dep` open; main does not
        self.assertEqual(b.claim("--next")[1]["slug"], "waits")

    def test_next_honours_filters_and_skips_research_unless_asked(self):
        self.a.add("a", milestone="M1")
        self.a.add("b", milestone="M2")
        self.a.add("r", priority="research", status="research")
        self.assertEqual(self.a.claim("--next", "--milestone", "M2")[1]["slug"], "b")
        self.assertEqual(self.a.claim("--next", "--area", "core")[0].returncode, 3)
        self.a.claim("--next")
        self.assertEqual(self.a.claim("--next")[0].returncode, 3)
        self.assertEqual(self.a.claim("--next", "--research")[1]["slug"], "r")

    def test_a_swarm_of_separate_clones_never_gets_the_same_ticket(self):
        for i in range(8):
            self.a.add(f"t{i}")
        clones = [self.w.clone(f"agent{i}") for i in range(8)]
        with ThreadPoolExecutor(8) as pool:
            got = list(pool.map(lambda c: c.claim("--next"), clones))
        self.assertTrue(all(r.returncode == 0 for r, _ in got), [r.stderr for r, _ in got])
        self.assertEqual(len({c["slug"] for _, c in got}), 8)
        self.assertEqual(len({c["uuid"] for _, c in got}), 8)
        self.assertEqual(self.w.clone("late").claim("--next")[0].returncode, 3)
        # eight tickets, eight claim files on main
        files = sh("git", "ls-tree", "--name-only", "main", "docs/backlog/claims/",
                   cwd=self.w.origin).stdout.split()
        self.assertEqual(len(files), 8)

    def test_a_swarm_racing_for_one_ticket_has_exactly_one_winner(self):
        self.a.add("only")
        clones = [self.w.clone(f"agent{i}") for i in range(6)]
        with ThreadPoolExecutor(6) as pool:
            got = list(pool.map(lambda c: c.claim("only"), clones))
        codes = sorted(r.returncode for r, _ in got)
        self.assertEqual(codes, [0, 2, 2, 2, 2, 2], [r.stderr for r, _ in got])

    def test_list_ready_hides_blocked_and_claimed_after_a_fetch(self):
        self.a.add("free")
        self.a.add("held")
        self.a.add("waits", blocked="free")
        b = self.w.clone("bob")
        b.claim("held")
        out = self.a.run("list", "--ready", "--fetch").stdout
        self.assertIn("free", out)
        self.assertNotIn("held", out)
        self.assertNotIn("waits", out)


class UnblockTests(Base):
    def blocked(self, slug):
        return self.a.run("show", slug, "--full").stdout.split("blocked:")[1].split("\n")[0].strip()

    def setUp(self):
        super().setUp()
        self.a.git("pull", "-q", "origin", "main")

    def local_entry(self, slug, **kw):
        (self.a.root / "docs/scootbar/backlog" / f"{slug}.md").write_text(entry(slug, **kw))

    def test_resolving_clears_a_blocker_that_named_only_it(self):
        self.local_entry("dep")
        self.local_entry("x", blocked="dep")
        self.a.run("resolve", "dep")
        self.assertEqual(self.blocked("x"), "null")

    def test_list_and_annotation_forms(self):
        for s, b in (("dep", None), ("other", None), ("both", "dep, other"),
                     ("anded", "dep and other"), ("noted", "dep (with a note)"),
                     ("first", "other and dep")):
            self.local_entry(s, blocked=b)
        self.a.run("resolve", "dep")
        self.assertEqual(json.loads(self.blocked("both")), "other")
        self.assertEqual(json.loads(self.blocked("anded")), "other")
        self.assertEqual(self.blocked("noted"), "null")
        self.assertEqual(json.loads(self.blocked("first")), "other")

    def test_prose_is_left_alone_and_partial_slug_names_do_not_match(self):
        for s, b in (("dep", None), ("dep-two", None), ("needs", "dep and a real-world thing"),
                     ("partial", "dep-two")):
            self.local_entry(s, blocked=b)
        self.a.run("resolve", "dep")
        self.assertEqual(json.loads(self.blocked("needs")), "a real-world thing")
        self.assertEqual(json.loads(self.blocked("partial")), "dep-two")


if __name__ == "__main__":
    unittest.main()
