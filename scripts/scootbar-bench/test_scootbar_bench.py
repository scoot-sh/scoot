"""Tests for the harness's own logic: the bars' configs, counting windows,
the switch arithmetic, the tables and both gates. No compositor, bar or
nix needed:

    python3 -m unittest discover -s scripts/scootbar-bench -p 'test_*.py'
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.join(HERE, "..", "scootbg-bench"))

import bars  # noqa: E402
import machine  # noqa: E402
import measure  # noqa: E402
import stage  # noqa: E402
import tables  # noqa: E402


class Configs(unittest.TestCase):
    def test_every_bar_shows_the_clock_in_the_same_look(self):
        with tempfile.TemporaryDirectory() as d:
            argv = bars.Scootbar(binary="/x/scootbar").write_config(d, "/f/DejaVuSans.ttf", "clock", "scoot")
            flags = dict(zip(argv[2::2], argv[3::2]))
            self.assertEqual(argv[:2], ["/x/scootbar", "daemon"])
            self.assertEqual(flags["--clock-format"], bars.FORMAT)
            self.assertEqual(flags["--right"], "clock")
            self.assertEqual(flags["--height"], str(bars.HEIGHT))
            self.assertEqual(flags["--font-size"], str(bars.FONT_PX))
            self.assertEqual(flags["--background"], "#" + bars.BACKGROUND)

            argv = bars.Yambar(store="/s/yambar").write_config(d, None, "clock", "scoot")
            self.assertEqual(argv[:2], ["/s/yambar/bin/yambar", "-c"])
            with open(argv[2]) as f:
                text = f.read()
            # The format split where yambar wants it: date, then time.
            self.assertIn('date-format: "%a %d %b"', text)
            self.assertIn('time-format: "%H:%M"', text)
            self.assertIn(f"height: {bars.HEIGHT}", text)
            self.assertIn(f"background: {bars.BACKGROUND}ff", text)

            argv = bars.Waybar(store="/s/waybar").write_config(d, None, "clock", "scoot")
            self.assertEqual(argv[0], "/s/waybar/bin/waybar")
            with open(argv[argv.index("-c") + 1]) as f:
                config = json.load(f)
            self.assertEqual(config["modules-right"], ["clock"])
            self.assertEqual(config["clock"]["format"], "{:" + bars.FORMAT + "}")
            self.assertEqual(config["clock"]["interval"], 60)
            self.assertNotIn("modules-left", config)
            with open(argv[argv.index("-s") + 1]) as f:
                self.assertIn(f"#{bars.BACKGROUND}", f.read())

    def test_the_clock_scopes_scootbar_command_line_is_m1s(self):
        # The like-for-like baseline ran M1's scootbar with exactly these
        # flags; a part the scope leaves empty is not given at all.
        with tempfile.TemporaryDirectory() as d:
            argv = bars.Scootbar(binary="/x/scootbar").write_config(d, "/f.ttf", "clock", "sway")
        self.assertEqual(argv[argv.index("--right") + 1], "clock")
        self.assertNotIn("--left", argv)
        self.assertNotIn("--center", argv)

    def test_the_workspaces_scope_places_workspaces_left_and_the_clock_right(self):
        with tempfile.TemporaryDirectory() as d:
            argv = bars.Scootbar(binary="/x/scootbar").write_config(
                d, "/f.ttf", "clock-workspaces", "scoot")
        self.assertEqual(argv[argv.index("--left") + 1], "workspaces")
        self.assertEqual(argv[argv.index("--right") + 1], "clock")
        self.assertNotIn("--center", argv)

    def test_waybar_uses_each_compositors_workspaces_module(self):
        for compositor, module in (("scoot", "ext/workspaces"), ("sway", "sway/workspaces")):
            with tempfile.TemporaryDirectory() as d:
                argv = bars.Waybar(store="/s/waybar").write_config(
                    d, None, "clock-workspaces", compositor)
                with open(argv[argv.index("-c") + 1]) as f:
                    config = json.load(f)
            self.assertEqual(config["modules-left"], [module])
            self.assertEqual(config["modules-right"], ["clock"])
            self.assertNotIn("modules-center", config)
            self.assertEqual(config[module], {"format": "{name}"})

    def test_yambar_shows_workspaces_on_sway_only_and_says_why_not_on_scoot(self):
        y = bars.Yambar(store="/s/yambar")
        self.assertIsNone(y.cannot_show("clock", "scoot"))
        self.assertIsNone(y.cannot_show("clock-workspaces", "sway"))
        why = y.cannot_show("clock-workspaces", "scoot")
        self.assertIn("ext-workspace-v1", why)
        with tempfile.TemporaryDirectory() as d:
            argv = y.write_config(d, None, "clock-workspaces", "sway")
            with open(argv[2]) as f:
                text = f.read()
            with self.assertRaises(AssertionError):
                y.write_config(d, None, "clock-workspaces", "scoot")
        self.assertIn("  left:\n    - i3:", text)
        self.assertIn("  right:\n    - clock:", text)
        # The clock alone has no left part.
        with tempfile.TemporaryDirectory() as d:
            argv = y.write_config(d, None, "clock", "scoot")
            with open(argv[2]) as f:
                self.assertNotIn("left:", f.read())

    def test_the_other_bars_can_show_every_scope_on_both_compositors(self):
        for cls in (bars.Scootbar, bars.Waybar):
            for scope in bars.SCOPES:
                for compositor in ("scoot", "sway"):
                    self.assertIsNone(cls(store="/s", binary="/b").cannot_show(scope, compositor))

    def test_an_unknown_scope_is_refused(self):
        with tempfile.TemporaryDirectory() as d, self.assertRaises(KeyError):
            bars.Waybar(store="/s").write_config(d, None, "battery", "scoot")

    def test_the_fontconfig_file_sees_one_directory(self):
        with tempfile.TemporaryDirectory() as d:
            with open(bars.fontconfig(d, "/fonts/here")) as f:
                text = f.read()
            self.assertIn("<dir>/fonts/here</dir>", text)
            self.assertEqual(text.count("<dir>"), 1)
            self.assertTrue(os.path.isdir(os.path.join(d, "fc-cache")))


class Machine(unittest.TestCase):
    def test_a_reading_has_what_the_kernel_offers_and_summarises(self):
        state = machine.state()
        self.assertEqual(set(state), {"cpus", "temps_c", "power", "loadavg"})
        self.assertIsInstance(machine.summary(state), str)
        # A synthetic capped, hot reading.
        state = {"cpus": {"cpu0": {"governor": "g", "cur_khz": 2000000,
                                   "policy_max_khz": 1000000, "hw_max_khz": 2000000}},
                 "temps_c": {"a/b": 80.0}, "power": {}, "loadavg": [0, 0, 0]}
        self.assertEqual(machine.summary(state), "max cur 2000 MHz, cap YES, hottest a/b 80.0 C")


class Executables(unittest.TestCase):
    def test_a_binary_wrapper_is_not_weighed_but_what_it_runs_is(self):
        with tempfile.TemporaryDirectory() as d:
            for name, head in (("waybar", b"\x7fELF"), (".waybar-wrapped", b"\x7fELF"),
                               ("helper", b"#!/bin/sh"), ("tool", b"\x7fELF")):
                with open(os.path.join(d, name), "wb") as f:
                    f.write(head + b"rest")
            got = [os.path.basename(p) for p in bars.real_executables(d)]
            self.assertEqual(got, [".waybar-wrapped", "tool"])


class Windows(unittest.TestCase):
    def test_scoot_and_sway_replies(self):
        self.assertEqual(stage.count_windows([{"id": 1}, {"id": 2}], "scoot"), 2)
        self.assertEqual(stage.count_windows({"windows": [{"id": 1}]}, "scoot"), 1)
        self.assertEqual(stage.count_windows({"error": "x"}, "scoot"), 0)
        tree = {"nodes": [{"nodes": [
            {"nodes": [{"app_id": "foot", "nodes": []}], "floating_nodes": []},
            {"nodes": [], "floating_nodes": [{"app_id": None, "window": 7, "nodes": []}]},
        ]}]}
        self.assertEqual(stage.count_windows(tree, "sway"), 2)
        self.assertEqual(stage.count_windows([], "sway"), 0)


class Quiet(unittest.TestCase):
    """A start is timed only once no window has drawn for a while: the
    smoke run's first version timed a terminal's redraw as a bar's first
    frame (1.4 ms)."""

    class Trace:
        def __init__(self, commits):
            self.commits = commits

        def poll(self):
            pass

    def test_a_recent_commit_holds_the_start_back(self):
        import time

        now = time.time_ns() // 1000
        with self.assertRaisesRegex(RuntimeError, "never went quiet"):
            measure.wait_quiet(self.Trace([(now, 3)]), quiet_s=5, timeout=0.1)
        measure.wait_quiet(self.Trace([(now - 6_000_000, 3)]), quiet_s=5, timeout=0.1)
        measure.wait_quiet(self.Trace([]), quiet_s=5, timeout=0.1)


class Switches(unittest.TestCase):
    def test_deltas_are_per_thread_and_never_negative_when_threads_exit(self):
        before = ({(10, "10"): (5, 1), (10, "11"): (7, 0), (11, "11"): (9, 0)}, 1_000_000)
        after = ({(10, "10"): (8, 1), (12, "12"): (2, 2)}, 4_500_000)
        # (10, 10): +3 voluntary; (12, 12) is new: all of its own; the
        # threads that exited are not counted, not subtracted.
        self.assertEqual(measure.delta(before, after), (5, 2, 3.5))
        # A pool whose threads were all replaced still reads zero or more.
        gone = ({(10, "10"): (100, 4)}, 1_000_000)
        fresh = ({(10, "77"): (3, 0)}, 1_000_000)
        self.assertEqual(measure.delta(gone, fresh), (3, 0, 0.0))


def write(d, meta, runs):
    with open(os.path.join(d, "meta.json"), "w") as f:
        json.dump(meta, f)
    with open(os.path.join(d, "runs.jsonl"), "w") as f:
        for r in runs:
            f.write(json.dumps(r) + "\n")


def results(scootbar_rss_kb, rival_rss_kb, scootbar_size=100):
    meta = {
        "compositors": ["scoot"],
        "compositor_versions": {"scoot": "scoot 0.1.0"},
        "bars": {
            "scootbar": {"ran": {"scoot": True},
                         "static": {"size_bytes": scootbar_size, "binaries_bytes": 90}},
            "rival": {"ran": {"scoot": True}, "static": {"size_bytes": 500, "binaries_bytes": 50}},
            "broken": {"ran": {"scoot": False}},
        },
    }
    runs = []
    for bar, rss in (("scootbar", scootbar_rss_kb), ("rival", rival_rss_kb)):
        for r, latency in enumerate((10, 11, 12)):
            runs.append({"compositor": "scoot", "bar": bar, "row": "startup", "round": r,
                         "ok": True, "latency_ms": latency})
        runs.append({"compositor": "scoot", "bar": bar, "row": "idle", "round": 1, "ok": True,
                     "rss_kb": rss, "wakeups_per_min": 2.0, "threads": 1})
    runs.append({"compositor": "scoot", "bar": "broken", "row": "idle", "round": 1,
                 "ok": False, "error": "it fell over\nand more"})
    return meta, runs


class Gates(unittest.TestCase):
    def test_a_competitor_ahead_is_a_loss_and_a_bare_binary_is_not_gated(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, *results(scootbar_rss_kb=4096, rival_rss_kb=1024))
            text = tables.render(d)
        rss = next(line for line in text.splitlines() if line.startswith("| Idle RSS"))
        self.assertIn("beats scootbar", rss)
        self.assertIn("LOSS on scoot: Idle RSS: rival", text)
        self.assertIn("1 loss(es)", text)
        # Smaller bare executable, but that row is not gated.
        bare = next(line for line in text.splitlines() if line.startswith("| Bare executable"))
        self.assertIn("not gated", bare)
        self.assertNotIn("beats", bare)
        self.assertIn("did not run", rss)
        self.assertIn("it fell over", text)
        self.assertIn("- tie: Startup to first frame: rival", text)

    def test_scootbar_ahead_passes(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, *results(scootbar_rss_kb=1024, rival_rss_kb=4096))
            self.assertIn("0 loss(es)", tables.render(d))

    def test_the_gate_counts_a_loss_and_a_pass_is_zero(self):
        with tempfile.TemporaryDirectory() as d:
            write(d, *results(scootbar_rss_kb=4096, rival_rss_kb=1024))
            text, failed = tables.gate(d)
            self.assertEqual(failed, 1, text)
            write(d, *results(scootbar_rss_kb=1024, rival_rss_kb=4096))
            text, failed = tables.gate(d)
            self.assertEqual(failed, 0, text)
            self.assertNotIn("Not judged", text)

    def test_a_gated_row_scootbar_has_no_value_for_fails_the_gate(self):
        meta, runs = results(scootbar_rss_kb=1024, rival_rss_kb=4096)
        for r in runs:
            if r["bar"] == "scootbar" and r["row"] == "idle":
                r.update(ok=False, error="no first frame")
        with tempfile.TemporaryDirectory() as d:
            write(d, meta, runs)
            text, failed = tables.gate(d)
        # Idle RSS and idle wakeups: the rival has both, scootbar neither.
        # Threads is not gated, and the broken bar did not run at all.
        self.assertEqual(failed, 2, text)
        self.assertIn("0 loss(es)", text)
        self.assertIn("- NO SCOOTBAR VALUE on scoot: Idle RSS: rival has one", text)
        self.assertNotIn("Threads: rival", text)

    def test_report_exits_1_when_the_gate_fails(self):
        bench = os.path.join(HERE, "bench.py")
        with tempfile.TemporaryDirectory() as d:
            write(d, *results(scootbar_rss_kb=4096, rival_rss_kb=1024))
            lost = subprocess.run([sys.executable, bench, "report", d],
                                  capture_output=True, text=True)
            write(d, *results(scootbar_rss_kb=1024, rival_rss_kb=4096))
            passed = subprocess.run([sys.executable, bench, "report", d],
                                    capture_output=True, text=True)
        self.assertEqual(lost.returncode, 1, lost.stderr)
        self.assertIn("LOSS on scoot: Idle RSS: rival", lost.stdout)
        self.assertEqual(passed.returncode, 0, passed.stderr)
        self.assertIn("0 loss(es)", passed.stdout)

    def test_a_bar_that_cannot_show_the_scope_is_named_and_not_passed(self):
        meta, runs = results(scootbar_rss_kb=1024, rival_rss_kb=4096)
        meta["bars"]["rival"]["cannot_show"] = {"scoot": "no ext-workspace-v1 module"}
        meta["bars"]["rival"]["ran"] = {"scoot": False}
        runs = [r for r in runs if r["bar"] != "rival"]
        with tempfile.TemporaryDirectory() as d:
            write(d, meta, runs)
            text, failed = tables.gate(d)
        self.assertIn("cannot show this scope", text)
        self.assertIn("- NOT COMPARED on scoot: rival: no ext-workspace-v1 module", text)
        self.assertEqual(failed, 1, text)
        self.assertIn("0 loss(es)", text)

    def test_the_machines_readings_are_summarised_beside_the_numbers(self):
        def reading(cur, cap, ac):
            return {"cpus": {"cpu0": {"governor": "schedutil", "cur_khz": cur,
                                      "policy_max_khz": cap, "hw_max_khz": 2424000}},
                    "temps_c": {"chip/NAND": 31.5}, "power": {"ac": {"type": "Mains", "online": ac}},
                    "loadavg": [0.1, 0.1, 0.1]}

        meta, runs = results(1024, 4096)
        runs[3]["hw_start"] = reading(600000, 2424000, "1")
        runs[3]["hw_end"] = reading(2000000, 1800000, "0")
        with tempfile.TemporaryDirectory() as d:
            write(d, meta, runs)
            text, _ = tables.gate(d)
        line = next(x for x in text.splitlines() if x.startswith("Machine, 2 readings"))
        self.assertIn("schedutil", line)
        self.assertIn("below the hardware maximum in 1", line)
        self.assertIn("600 to 2000 MHz", line)
        self.assertIn("offline in 1", line)
        # M1's runs carry none: no line, no crash.
        meta, runs = results(1024, 4096)
        with tempfile.TemporaryDirectory() as d:
            write(d, meta, runs)
            self.assertNotIn("Machine,", tables.gate(d)[0])

    def test_compare_finds_a_regression_and_ignores_noise(self):
        with tempfile.TemporaryDirectory() as base, tempfile.TemporaryDirectory() as now:
            write(base, *results(1024, 4096))
            write(now, *results(1030, 4096))  # inside 5%
            text, regressions = tables.compare(now, base)
            self.assertEqual(regressions, 0, text)
            write(now, *results(2048, 4096, scootbar_size=300))
            text, regressions = tables.compare(now, base)
            self.assertEqual(regressions, 2, text)  # RSS and Size
            self.assertIn("| scoot | Idle RSS | 1.0 | 2.0 | **REGRESSED** |", text)


if __name__ == "__main__":
    unittest.main()
