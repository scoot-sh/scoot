"""Tests for the harness's own logic: the trace parser, the gate rule, the
table. Run with ``python3 -m unittest discover -s scripts/scootbg-bench``
(no compositor, daemon or nix needed)."""

import json
import os
import tempfile
import time
import unittest

import commits
import report


class ParseLine(unittest.TestCase):
    def test_wayland_rs_commit_and_attach(self):
        now = time.time_ns() // 1000
        truncated = now % (1 << 32)
        line = f"[{truncated // 1000:7}.{truncated % 1000:03}][rs] <- wl_surface@10.commit, ()"
        t, surface, request, args = commits.parse_line(line, now)
        self.assertEqual((t, surface, request, args), (now, 10, "commit", ""))
        line = f"[{truncated // 1000:7}.{truncated % 1000:03}][rs] <- wl_surface@10.attach, (12, 0, 0)"
        _, _, request, args = commits.parse_line(line, now + 5)
        self.assertEqual(request, "attach")
        self.assertFalse(commits.attach_is_null(args))
        self.assertTrue(commits.attach_is_null("0, 0, 0"))

    def test_wayland_rs_time_unwraps_across_the_32_bit_boundary(self):
        # A reference just after a wrap, an event just before it.
        span = 1 << 32
        reference = 10 * span + 1000
        event = reference - 3000
        truncated = event % span
        line = f"[{truncated // 1000:7}.{truncated % 1000:03}][rs] <- wl_surface@3.commit, ()"
        self.assertEqual(commits.parse_line(line, reference)[0], event)

    def test_libwayland_request_and_null_attach(self):
        now_s = time.time()
        lt = time.localtime(now_s)
        line = f"[{lt.tm_hour:02}:{lt.tm_min:02}:{lt.tm_sec:02}.123456] wl_surface#11.attach(nil, 0, 0)"
        t, surface, request, args = commits.parse_line(line, int(now_s * 1e6))
        self.assertEqual((surface, request), (11, "attach"))
        self.assertTrue(commits.attach_is_null(args))
        self.assertLess(abs(t - int(now_s * 1e6)), 2_000_000)

    def test_libwayland_events_and_other_interfaces_are_ignored(self):
        now = time.time_ns() // 1000
        self.assertIsNone(commits.parse_line("[21:58:30.696157]  -> wl_surface#11.enter(wl_output#9)", now))
        self.assertIsNone(commits.parse_line("[ 204195.263][rs] -> wl_surface@10.enter(wl_output@9[0])", now))
        self.assertIsNone(commits.parse_line("[21:58:30.696157] wl_callback#5.done(1)", now))
        self.assertIsNone(commits.parse_line("garbage", now))


class TraceCommits(unittest.TestCase):
    def test_only_commits_after_a_buffer_count_and_destroy_forgets(self):
        now = time.time_ns() // 1000
        tr = now % (1 << 32)

        def rs(request, args=""):
            return f"[{tr // 1000:7}.{tr % 1000:03}][rs] <- wl_surface@7.{request}, ({args})\n"

        with tempfile.NamedTemporaryFile("w", delete=False) as f:
            f.write(rs("commit"))  # the initial, bufferless commit
            f.write(rs("attach", "20, 0, 0"))
            f.write(rs("commit"))  # a buffer commit
            f.write(rs("commit"))  # no new attach: not one
            f.write(rs("attach", "20, 0, 0"))
            f.write(rs("destroy"))
            f.write(rs("commit"))  # the id reused: its attach was forgotten
            f.write(rs("attach", "0, 0, 0"))
            f.write(rs("commit"))  # a null attach: not one
            f.write("[ 1.000][rs] <- wl_surface@7.comm")  # a partial line
            path = f.name
        try:
            trace = commits.Trace(path)
            trace.poll()
            self.assertEqual(len(trace.commits), 1)
            with open(path, "a") as f:
                f.write("it, ()\n")
            trace.poll()  # the finished line is a bufferless commit
            self.assertEqual(len(trace.commits), 1)
        finally:
            os.unlink(path)


class Gate(unittest.TestCase):
    def test_beaten_only_beyond_five_percent_and_the_spread(self):
        ref = [100, 101, 102]
        self.assertEqual(report.verdict(ref, [96, 96, 96])[0], "tie")  # inside 5%
        self.assertEqual(report.verdict(ref, [90, 90, 90])[0], "beaten")
        self.assertEqual(report.verdict(ref, [110, 111, 112])[0], "win")
        # A wide spread widens the margin past 5%.
        self.assertEqual(report.verdict([100, 80, 120], [85, 85, 85])[0], "tie")

    def test_zeroes_tie(self):
        self.assertEqual(report.verdict([0, 0, 0], [0, 0, 0])[0], "tie")


class Table(unittest.TestCase):
    def test_na_did_not_run_informational_and_losses(self):
        meta = {
            "daemons": {
                "scootbg": {"ran": True, "static": {"size_bytes": 100, "disk_bytes": 100}},
                "scootbg-bilinear": {"ran": True, "informational": True},
                "rival": {"ran": True, "static": {"size_bytes": 50, "disk_bytes": 200}},
                "broken": {"ran": False},
            },
            "supports": {
                "scootbg": {"set:image": True, "set:color": True},
                "scootbg-bilinear": {"set:image": True},
                "rival": {"set:image": True, "set:color": False},
                "broken": {},
            },
        }
        runs = []
        for rnd, (s, b, r) in enumerate([(400, 300, 350), (410, 305, 352), (405, 310, 351)]):
            for name, v in (("scootbg", s), ("scootbg-bilinear", b), ("rival", r)):
                runs.append({"ok": True, "daemon": name, "row": "set", "variant": "image",
                             "geometry": "1x3840x2160", "latency_ms": v, "round": rnd})
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "meta.json"), "w") as f:
                json.dump(meta, f)
            with open(os.path.join(d, "runs.jsonl"), "w") as f:
                for r in runs:
                    f.write(json.dumps(r) + "\n")
            text = report.render(d)
        latency = next(line for line in text.splitlines() if line.startswith("| Set: latency to the JPEG"))
        cells = [c.strip() for c in latency.split("|")[2:-1]]
        self.assertEqual(cells[0], "405 [400–410]")
        self.assertNotIn("beats", cells[1])  # informational: never a verdict
        self.assertIn("beats scootbg", cells[2])
        self.assertEqual(cells[3], "did not run")
        color = next(line for line in text.splitlines() if line.startswith("| Set: latency to a color"))
        self.assertIn("n/a", color.split("|")[4])  # the rival cannot: n/a, not a win
        size = next(line for line in text.splitlines() if line.startswith("| Size"))
        self.assertIn("beats scootbg", size)
        self.assertIn("LOSS: Set: latency to the JPEG: rival", text)


if __name__ == "__main__":
    unittest.main()
