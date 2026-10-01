"""Tests for the order file tool's own logic: turning a symbol into a glob,
reading a qemu log, writing and reading the file. No qemu, scoot or binary
needed:

    python3 -m unittest discover -s scripts/scootbar-orderfile -p 'test_*.py'
"""

import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import orderfile  # noqa: E402

# A cargo build and the Nix build of the same source, as `nm` showed them
# (shortened): every crate's disambiguator differs, and so do the numbers of
# the back-references that follow one.
CARGO = "_RNvMs3_NtCscl3upvMoHOv_5alloc7raw_vecINtB5_6RawVecNtNtCsfGIORv1QPSQ_14wayland_client7globals6GlobalE8grow_oneCsfBkxij1jiAh_8scootbar"
NIX = "_RNvMs3_NtCscl3upvMoHOv_5alloc7raw_vecINtB5_6RawVecNtNtCs7l2zTPqwvAy_14wayland_client7globals6GlobalE8grow_oneCsfQhpWztiEt2_8scootbar"


class Glob(unittest.TestCase):
    def test_a_pattern_matches_its_own_symbol(self):
        for sym in (CARGO, NIX, "main", "_start", "__aarch64_ldadd4_relax"):
            self.assertTrue(orderfile.matches(orderfile.glob(sym), sym), sym)

    def test_one_pattern_matches_both_builds_of_a_function(self):
        for sym in (CARGO, NIX):
            glob = orderfile.glob(sym)
            self.assertTrue(orderfile.matches(glob, CARGO), glob)
            self.assertTrue(orderfile.matches(glob, NIX), glob)

    def test_the_hashes_and_back_references_are_what_is_wildcarded(self):
        glob = orderfile.glob(CARGO)
        self.assertNotIn("sfBkxij1jiAh", glob)
        self.assertNotIn("sfGIORv1QPSQ", glob)
        self.assertNotIn("B5_", glob)
        # the names stay: they are what tells one function from another
        for name in ("5alloc7raw_vec", "14wayland_client", "7globals6Global", "8grow_one", "8scootbar"):
            self.assertIn(name, glob)

    def test_another_function_does_not_match(self):
        other = CARGO.replace("8grow_one", "10finish_gro")
        self.assertFalse(orderfile.matches(orderfile.glob(CARGO), other))

    def test_a_local_symbols_number_is_not_part_of_it(self):
        glob = orderfile.glob("_RNvXsZ_NtCscl3upvMoHOv_5alloc6stringNtB5_6StringNtNtCskOwsldysgAL_4core3fmt5Write9write_str.1843")
        self.assertTrue(orderfile.matches(glob, glob.replace(".*", ".77")))
        self.assertTrue(orderfile.matches(glob, glob.replace(".*", ".llvm.5")))

    def test_a_short_symbol_stays_exact_rather_than_matching_the_world(self):
        # `_RNvC*_1a1b` would be nearly all wildcard
        self.assertEqual(orderfile.glob("_RNvCs1a_1a1b"), "_RNvCs1a_1a1b")

    def test_a_plain_c_symbol_is_its_own_pattern(self):
        self.assertEqual(orderfile.glob("call_weak_fn"), "call_weak_fn")


class Log(unittest.TestCase):
    LOG = "\n".join(
        [
            "IN: _start", "0x1000:  ", "OBJD-T: aa", "",
            "IN: memcpy", "0x2000:  ", "",  # libc: not the binary's
            "IN: first", "0x3000:  ", "",
            "IN: second", "0x4000:  ", "",
            "IN: first", "0x3000:  ", "",  # again: counted once
            "IN: third", "0x5000:  ", "",
        ]
    )

    def write(self, text):
        f = tempfile.NamedTemporaryFile("w", suffix=".log", delete=False)
        self.addCleanup(os.unlink, f.name)
        f.write(text)
        f.close()
        return f.name

    def test_functions_in_the_order_they_first_ran_and_only_the_binarys_own(self):
        path = self.write(self.LOG)
        running, ending = orderfile.tb_functions(path, {"_start", "first", "second", "third"})
        self.assertEqual(running, ["_start", "first", "second", "third"])
        self.assertEqual(ending, [])

    def test_what_ran_after_the_cut_is_kept_apart(self):
        path = self.write(self.LOG)
        cut = self.LOG.index("IN: third")
        running, ending = orderfile.tb_functions(path, {"_start", "first", "second", "third"}, cut)
        self.assertEqual(running, ["_start", "first", "second"])
        self.assertEqual(ending, ["third"])


class File(unittest.TestCase):
    def test_what_is_written_reads_back_in_order(self):
        patterns = ["_RNvC*_8scootbar4main", "call_weak_fn", "_RNvNtC*_8scootbar3cli4text"]
        text = orderfile.render(patterns, ["a header", "", "two lines"])
        self.assertEqual(orderfile.parse(text), patterns)

    def test_it_is_a_linker_script_for_the_text_section(self):
        text = orderfile.render(["f"], ["h"])
        self.assertIn(".text : {", text)
        self.assertTrue(text.rstrip().endswith("}"))
        for prefix in orderfile.PREFIXES:
            self.assertIn(f"*({prefix}f)", text)

    def test_the_sections_no_name_reaches_come_first(self):
        text = orderfile.render(["f"], ["h"])
        for section in orderfile.OBJECT_SECTIONS:
            self.assertLess(text.index(section), text.index("*(.text.f)"))

    def test_the_shipped_file_has_the_shape_the_linker_needs(self):
        path = os.path.join(HERE, "..", "..", "crates", "scootbar", "orderfile", "hot-text.ld")
        with open(path) as f:
            text = f.read()
        patterns = orderfile.parse(text)
        self.assertGreater(len(patterns), 100)
        self.assertEqual(len(patterns), len(set(patterns)), "a pattern is listed twice")
        self.assertEqual(text.count("{"), 1)
        self.assertEqual(text.count("}"), 1)
        for line in text.splitlines():
            if not line.startswith(("/*", " *", " */")):
                self.assertRegex(line, r"^(\.text : \{|\}|  \*[A-Za-z0-9_*:.-]*\.o\(\.text \.text\.\*\)|  \*\(\.text\.[^\s()]+\))$")


class Header(unittest.TestCase):
    def test_the_header_names_the_scenarios_and_counts_nothing(self):
        # Counts shift from run to run; a regeneration without a change of
        # code must not be a diff.
        a = orderfile.header_lines({"idle": (240, 240), "custom": (417, 120)})
        b = orderfile.header_lines({"idle": (239, 239), "custom": (400, 99)})
        self.assertEqual(a, b)
        self.assertIn("  idle", a)
        self.assertFalse(any(ch.isdigit() for line in a for ch in line))


if __name__ == "__main__":
    unittest.main()
