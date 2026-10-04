import contextlib
import copy
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


TOOLS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS_DIR))

from compare_syzygy import (  # noqa: E402
    compare, compare_packed_root, main, packed_move_distance,
    packed_wdl, run_probe,
)


class CompareSyzygyTests(unittest.TestCase):
    @staticmethod
    def packed_root_witness():
        reference = {
            "root_status": "moves", "selected_move": "b1b7",
            "dtz": {"value": 13}, "mate_moves": [],
            "moves": [
                {"uci": "b1b7", "child_wdl": 2,
                 "child_dtz": {"value": -12}, "zeroing": False, "ep": False},
                {"uci": "b1g6", "child_wdl": 0,
                 "child_dtz": {"value": 0}, "zeroing": False, "ep": False},
            ],
        }
        candidate = {
            "selected_move": "b1b7",
            "packed_root": {"uci": "b1b7", "wdl": 2, "dtz": 13, "ep": False},
            "packed_moves": [
                {"uci": "b1b7", "wdl": 2, "dtz": 13, "ep": False},
                {"uci": "b1g6", "wdl": 0, "dtz": 0, "ep": False},
            ],
            "packed_num_moves": 2,
            "packed_unused_failed": True,
        }
        return reference, candidate

    def test_packed_root_rejects_corrupt_count_array_and_each_field(self):
        reference, valid = self.packed_root_witness()
        self.assertEqual(compare_packed_root(reference, valid, 0), [])
        mutations = [
            ("packed_num_moves", 1, "packed_move_count"),
            ("packed_unused_failed", False, "packed_unused_entries"),
        ]
        for key, bad, expected in mutations:
            candidate = copy.deepcopy(valid)
            candidate[key] = bad
            self.assertIn(expected, compare_packed_root(reference, candidate, 0))
        for field, bad in (("uci", "b1a2"), ("wdl", 1),
                           ("dtz", 12), ("ep", True)):
            candidate = copy.deepcopy(valid)
            candidate["packed_root"][field] = bad
            self.assertIn(f"packed_root_{field}",
                          compare_packed_root(reference, candidate, 0))
            candidate = copy.deepcopy(valid)
            candidate["packed_moves"][0][field] = bad
            errors = compare_packed_root(reference, candidate, 0)
            if field == "uci":
                self.assertIn("packed_move_set", errors)
            else:
                self.assertIn(f"packed_move:b1b7_{field}", errors)
        for bad, expected in (([], "packed_move_array_length"),
                              ([valid["packed_moves"][0]] * 2, "packed_duplicate:b1b7")):
            candidate = copy.deepcopy(valid)
            candidate["packed_moves"] = bad
            self.assertIn(expected, compare_packed_root(reference, candidate, 0))

    def test_packed_root_terminal_and_fifty_move_edges(self):
        reference, candidate = self.packed_root_witness()
        reference.update(root_status="stalemate", dtz={"value": 0},
                         selected_move=None, moves=[])
        candidate.update(selected_move=None, packed_root=None, packed_moves=[],
                         packed_num_moves=0)
        self.assertEqual(compare_packed_root(reference, candidate, 0), [])
        candidate["packed_root"] = {"uci": "b1b7", "wdl": 2,
                                    "dtz": 13, "ep": False}
        self.assertIn("packed_terminal_root",
                      compare_packed_root(reference, candidate, 0))
        self.assertEqual(packed_wdl(98, 1), 2)
        self.assertEqual(packed_wdl(99, 1), 2)
        self.assertEqual(packed_wdl(100, 1), 1)
        self.assertEqual(packed_wdl(254, -1), -1)
        self.assertEqual(packed_wdl(255, 0), 0)

    def test_packed_move_distance_accounts_for_mate_zeroing_and_quiet_ply(self):
        quiet = {"uci": "b1b7", "child_wdl": 2,
                 "child_dtz": {"value": -12}, "zeroing": False}
        self.assertEqual(packed_move_distance(quiet, 13, set()), 13)
        self.assertEqual(packed_move_distance(quiet, 13, {"b1b7"}), 1)
        self.assertEqual(packed_move_distance({**quiet, "zeroing": True}, 13, set()), 1)
        self.assertEqual(packed_move_distance({**quiet, "child_wdl": -1,
                                               "zeroing": True}, -101, set()), -101)
        self.assertEqual(packed_move_distance({**quiet, "child_dtz": {"value": 12}},
                                              -13, set()), -13)

    def test_winning_zeroing_moves_tie_at_dtz_one(self):
        reference = {
            "wdl": 2,
            "dtz": {"value": 1},
            "root_status": "moves",
            "selected_move": "e5e6",
            "optimal_moves": ["e5e6"],
            "mate_moves": [],
            "moves": [
                {"uci": "e5e6", "child_wdl": 2,
                 "child_dtz": {"value": -1}, "zeroing": True},
                {"uci": "e5d6", "child_wdl": 2,
                 "child_dtz": {"value": -2}, "zeroing": True},
                {"uci": "e2e3", "child_wdl": 2,
                 "child_dtz": {"value": -1}, "zeroing": False},
            ],
        }
        candidate = copy.deepcopy(reference)
        candidate["selected_move"] = "e5d6"
        self.assertEqual(compare(reference, candidate, "root"), [])
        candidate["selected_move"] = "e2e3"
        self.assertIn("selected_move_not_optimal", compare(reference, candidate, "root"))

    def test_draw_moves_and_nonzeroing_mates_preserve_the_root_outcome(self):
        reference = {
            "wdl": 0, "dtz": {"value": 0}, "root_status": "moves",
            "selected_move": "a1a2", "optimal_moves": ["a1a2"],
            "mate_moves": [],
            "moves": [
                {"uci": "a1a2", "child_wdl": 0,
                 "child_dtz": {"value": 0}, "zeroing": False},
                {"uci": "a1b1", "child_wdl": 0,
                 "child_dtz": {"value": 0}, "zeroing": True},
            ],
        }
        candidate = copy.deepcopy(reference)
        candidate["selected_move"] = "a1b1"
        self.assertEqual(compare(reference, candidate, "root"), [])

        reference["wdl"] = 2
        reference["dtz"]["value"] = 1
        reference["mate_moves"] = ["a1b1"]
        for move in reference["moves"]:
            move["child_wdl"] = 2
            move["zeroing"] = False
        candidate = copy.deepcopy(reference)
        candidate["selected_move"] = "a1b1"
        self.assertEqual(compare(reference, candidate, "root"), [])
        reference["mate_moves"] = []
        self.assertIn("selected_move_not_optimal", compare(reference, candidate, "root"))

    def test_immediate_losing_resets_tie_at_root_dtz_minus_one(self):
        reference = {
            "wdl": -2, "dtz": {"value": -1}, "root_status": "moves",
            "selected_move": "a7g7", "optimal_moves": ["a7g7"],
            "mate_moves": [],
            "moves": [
                {"uci": "a7g7", "child_wdl": -2,
                 "child_dtz": {"value": 35}, "zeroing": True},
                {"uci": "h7g7", "child_wdl": -2,
                 "child_dtz": {"value": 1}, "zeroing": True},
                {"uci": "h7h8", "child_wdl": -2,
                 "child_dtz": {"value": 1}, "zeroing": False},
            ],
        }
        candidate = copy.deepcopy(reference)
        candidate["selected_move"] = "h7g7"
        self.assertEqual(compare(reference, candidate, "root"), [])
        candidate["selected_move"] = "h7h8"
        self.assertIn("selected_move_not_optimal", compare(reference, candidate, "root"))

    def test_catches_sign_move_set_and_selected_outcome_errors(self):
        reference = {
            "wdl": 2,
            "dtz": {"value": 13, "rounded": True},
            "root_status": "moves",
            "selected_move": "b1b7",
            "optimal_moves": ["b1b7"],
            "mate_moves": [],
            "moves": [
                {"uci": "b1b7", "child_wdl": 2,
                 "child_dtz": {"value": -12}, "zeroing": False},
                {"uci": "b1g6", "child_wdl": 0,
                 "child_dtz": {"value": 0}, "zeroing": False},
                {"uci": "b1a2", "child_wdl": 2,
                 "child_dtz": {"value": -16}, "zeroing": False},
            ],
        }
        self.assertEqual(compare(reference, copy.deepcopy(reference), "root"), [])
        candidate = copy.deepcopy(reference)
        candidate["dtz"]["value"] = -13
        candidate["moves"][0]["child_wdl"] = 0
        candidate["selected_move"] = "b1g6"
        errors = compare(reference, candidate, "root")
        self.assertIn("dtz", errors)
        self.assertIn("child_wdl:b1b7", errors)
        self.assertIn("selected_move_wdl", errors)
        candidate = copy.deepcopy(reference)
        candidate["selected_move"] = "b1a2"
        self.assertIn("selected_move_not_optimal", compare(reference, candidate, "root"))
        candidate = copy.deepcopy(reference)
        candidate["moves"].pop()
        self.assertIn("move_set", compare(reference, candidate, "root"))

    def test_duplicate_cases_fail_before_creating_durable_results(self):
        request = {
            "version": 1,
            "id": "duplicate",
            "fen": "7k/8/8/8/8/8/8/1Q2K3 w - - 0 1",
            "operation": "wdl",
            "required_files": ["KQvK.rtbw"],
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            cases = directory / "cases.jsonl"
            cases.write_text((json.dumps(request) + "\n") * 2, encoding="utf-8")
            tables = directory / "tables"
            tables.mkdir()
            output = directory / "result"
            args = [
                "--reference", sys.executable, "--candidate", sys.executable,
                "--tables", str(tables), "--cases", str(cases),
                "--output-dir", str(output),
            ]
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args), 1)
            self.assertFalse(output.exists())

    @unittest.skipIf(os.name == "nt", "test executable uses a Unix shebang")
    def test_short_successful_transcript_is_rejected_and_preserved(self):
        request = {
            "version": 1,
            "id": "one",
            "fen": "7k/8/8/8/8/8/8/1Q2K3 w - - 0 1",
            "operation": "wdl",
            "required_files": [],
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            executable = directory / "short-reply"
            executable.write_text("#!/usr/bin/env python3\nimport sys\nsys.stderr.write('ended early')\n",
                                  encoding="utf-8")
            executable.chmod(0o755)
            batch = directory / "batch"
            batch.mkdir()
            args = SimpleNamespace(tables=directory, timeout=5)
            with self.assertRaisesRegex(RuntimeError, "returned 0 of 1 replies"):
                run_probe("short", executable, args, [request], batch)
            self.assertTrue((batch / "short.invocation.json").is_file())
            self.assertTrue((batch / "requests.jsonl").is_file())
            self.assertEqual((batch / "short.stdout.jsonl").read_bytes(), b"")
            self.assertEqual((batch / "short.stderr.log").read_text(), "ended early")

    @unittest.skipIf(os.name == "nt", "test executable uses a Unix shebang")
    def test_resume_rejects_changed_invocation_tables_and_transcripts(self):
        request = {
            "version": 1,
            "id": "resume-one",
            "fen": "7k/8/8/8/8/8/8/1Q2K3 w - - 0 1",
            "operation": "wdl",
            "required_files": ["KQvK.rtbw"],
        }
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            cases = directory / "cases.jsonl"
            cases.write_text(json.dumps(request) + "\n", encoding="utf-8")
            tables = directory / "tables"
            tables.mkdir()
            table = tables / "KQvK.rtbw"
            table.write_bytes(b"table-one")
            successor = tables / "KRvK.rtbw"
            successor.write_bytes(b"successor-one")
            executable = directory / "probe"
            executable.write_text(
                "#!/usr/bin/env python3\n"
                "import json, os, sys\n"
                "with open(os.environ['SYZYGY_TEST_COUNT'], 'a') as count:\n"
                "    count.write('run\\n')\n"
                "for line in sys.stdin:\n"
                "    request = json.loads(line)\n"
                "    print(json.dumps({'version': 1, 'id': request['id'], "
                "'status': 'ok', 'wdl': 0}))\n",
                encoding="utf-8",
            )
            executable.chmod(0o755)
            count = directory / "count"
            output = directory / "results"
            args = [
                "--reference", str(executable), "--candidate", str(executable),
                "--tables", str(tables), "--cases", str(cases),
                "--output-dir", str(output),
            ]
            with mock.patch.dict(os.environ, {"SYZYGY_TEST_COUNT": str(count)}):
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(main(args), 0)
                    self.assertEqual(main(args + ["--resume"]), 0)
            self.assertEqual(count.read_text().splitlines(), ["run", "run"])
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args + ["--resume", "--batch-size", "2"]), 1)
            self.assertEqual(count.read_text().splitlines(), ["run", "run"])
            table.write_bytes(b"table-two")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args + ["--resume"]), 1)
            self.assertEqual(count.read_text().splitlines(), ["run", "run"])
            table.write_bytes(b"table-one")
            successor.write_bytes(b"successor-two")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args + ["--resume"]), 1)
            self.assertEqual(count.read_text().splitlines(), ["run", "run"])
            successor.write_bytes(b"successor-one")
            transcript = output / "batch-000000" / "candidate.stdout.jsonl"
            transcript.write_bytes(transcript.read_bytes() + b" ")
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(main(args + ["--resume"]), 1)
            self.assertEqual(count.read_text().splitlines(), ["run", "run"])


if __name__ == "__main__":
    unittest.main()
