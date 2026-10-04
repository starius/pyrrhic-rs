import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

import chess


TOOLS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS_DIR))

from generate_syzygy_cases import (  # noqa: E402
    Writer,
    add_special_cases,
    is_reachable_probe_position,
    main,
    material_index,
    sample_material,
    stem_for_board,
)


class GenerateSyzygyCasesTests(unittest.TestCase):
    def test_rejects_nonpositive_three_piece_limit_before_creating_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "focused.jsonl"
            for limit in (0, -1):
                arguments = [
                    "--output", str(output), "--only-material", "KRvKP",
                    "--placements-per-orientation", "1", "--walks-per-material", "0",
                    "--three-piece-limit", str(limit),
                ]
                with contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as raised:
                        main(arguments)
                self.assertEqual(raised.exception.code, 2)
                self.assertFalse(output.exists())
                self.assertFalse(output.with_suffix(".jsonl.invocation.json").exists())

    def test_only_material_cli_accepts_a_manifest_stem(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "focused.jsonl"
            arguments = [
                "--output", str(output), "--only-material", "KRvKP",
                "--placements-per-orientation", "1", "--walks-per-material", "0",
            ]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(main(arguments), 0)
            cases = [json.loads(line) for line in output.read_text().splitlines()]
            self.assertEqual(len(cases), 4)
            self.assertTrue(all(case["id"].startswith("KRvKP:") for case in cases))

    def test_rejects_two_knight_checks_accepted_by_python_chess(self):
        board = chess.Board("8/8/4N2B/1N6/3k4/5K2/8/8 b - - 0 1")
        self.assertTrue(board.is_valid())
        self.assertFalse(is_reachable_probe_position(board))

    def test_seeded_samples_cover_orientations_and_successors(self):
        index = material_index(TOOLS_DIR.parent / "nix/syzygy-3-4-5.json")
        self.assertEqual(len(index), 145)

        def generate():
            stream = io.StringIO()
            writer = Writer(stream)
            sample_material(writer, index, "KRvKP", 0x53595A59, 3, 8)
            return [json.loads(line) for line in stream.getvalue().splitlines()]

        first = generate()
        self.assertEqual(first, generate())
        self.assertEqual(len({item["id"] for item in first}), len(first))
        placements = [item for item in first if ":walk:" not in item["id"]]
        self.assertEqual(len(placements), 4 * 3)
        self.assertEqual(len([item for item in first if ":walk:" in item["id"]]), 8)
        for item in first:
            board = chess.Board(item["fen"])
            self.assertTrue(board.is_valid())
            self.assertEqual(len(item["required_files"]), 2)
            self.assertEqual(
                item["required_files"][0],
                f"{stem_for_board(board, index)}.rtbw",
            )

    def test_special_cases_reach_promotions_ep_and_terminal_states(self):
        index = material_index(TOOLS_DIR.parent / "nix/syzygy-3-4-5.json")
        stream = io.StringIO()
        add_special_cases(Writer(stream), index)
        cases = {item["id"]: item for item in map(json.loads, stream.getvalue().splitlines())}
        promotion = chess.Board(cases["special:promotion:root"]["fen"])
        self.assertEqual(
            {move.promotion for move in promotion.legal_moves if move.promotion},
            {chess.QUEEN, chess.ROOK, chess.BISHOP, chess.KNIGHT},
        )
        ep = chess.Board(cases["special:en-passant:root"]["fen"])
        self.assertTrue(any(ep.is_en_passant(move) for move in ep.legal_moves))
        self.assertTrue(chess.Board(cases["special:mate:root"]["fen"]).is_checkmate())
        self.assertTrue(chess.Board(cases["special:stalemate:root"]["fen"]).is_stalemate())
        for label in ("zeroing-capture", "promotion"):
            for halfmoves in (99, 100):
                boundary = cases[f"special:{label}:clock-{halfmoves}:root"]
                self.assertEqual(chess.Board(boundary["fen"]).halfmove_clock, halfmoves)
        for halfmoves in (0, 1, 49, 87, 89, 98, 99, 100):
            dtz = cases[f"special:clock-{halfmoves}:dtz"]
            root = cases[f"special:clock-{halfmoves}:root"]
            self.assertEqual(dtz["fen"], root["fen"])
            self.assertEqual(root["operation"], "root")
            loss = cases[f"special:clock-loss-{halfmoves}:root"]
            self.assertEqual(chess.Board(loss["fen"]).halfmove_clock, halfmoves)
            self.assertFalse(chess.Board(loss["fen"]).turn)


if __name__ == "__main__":
    unittest.main()
