#!/usr/bin/env python3
"""Generate a deterministic legal corpus for the full 3–5 piece Syzygy set."""

import argparse
import hashlib
import json
import random
import sys
from collections import Counter
from itertools import combinations, product
from pathlib import Path

import chess


SEED = 0x53595A59
THREE_PIECE = ("KQvK", "KRvK", "KBvK", "KNvK", "KPvK")


def material_key(white, black):
    side = lambda symbols: tuple(sorted(symbols))
    return tuple(sorted((side(white), side(black))))


def material_index(manifest):
    entries = json.loads(manifest.read_text(encoding="utf-8"))
    names = {entry["name"] for entry in entries}
    stems = sorted(name[:-5] for name in names if name.endswith(".rtbw"))
    if len(stems) != 145 or any(f"{stem}.rtbz" not in names for stem in stems):
        raise ValueError("expected the complete 145-pair 3–5 piece manifest")
    index = {}
    for stem in stems:
        white, black = stem.split("v")
        key = material_key(white, black)
        if key in index:
            raise ValueError(f"ambiguous material {stem}")
        index[key] = stem
    return index


def stem_for_board(board, index):
    white = [piece.symbol().upper() for piece in board.piece_map().values() if piece.color]
    black = [piece.symbol().upper() for piece in board.piece_map().values() if not piece.color]
    if len(white) + len(black) == 2:
        return None
    return index.get(material_key(white, black))


def initial_board(stem, swap_colors, turn, squares):
    left, right = stem.split("v")
    labels = [(symbol, not swap_colors) for symbol in left] + [
        (symbol, swap_colors) for symbol in right
    ]
    board = chess.Board(None)
    board.turn = turn
    for (symbol, white), square in zip(labels, squares):
        board.set_piece_at(square, chess.Piece.from_symbol(symbol if white else symbol.lower()))
    board.castling_rights = 0
    board.ep_square = None
    board.halfmove_clock = 0
    board.fullmove_number = 1
    return board


def is_reachable_probe_position(board):
    """Match the oracle's stricter check validation for synthetic placements.

    A legal last move can expose one sliding check while giving one direct
    check, but it cannot create two simultaneous pawn/knight checks. The
    python-chess static validator accepts that impossible arrangement.
    """
    if not board.is_valid():
        return False
    steppers = board.pawns | board.knights | board.kings
    return (board.checkers_mask() & steppers).bit_count() <= 1


def seeded_random(seed, label):
    digest = hashlib.sha256(f"{seed}:{label}".encode()).digest()
    return random.Random(int.from_bytes(digest[:8], "big"))


def required_files(stem):
    return [] if stem is None else [f"{stem}.rtbw", f"{stem}.rtbz"]


class Writer:
    def __init__(self, stream):
        self.stream = stream
        self.counts = Counter()
        self.seen_ids = set()

    def emit(self, case_id, board, operation, stem):
        if case_id in self.seen_ids:
            raise ValueError(f"duplicate generated id {case_id}")
        if not is_reachable_probe_position(board):
            raise ValueError(f"invalid generated board {case_id}: {board.fen()}")
        self.seen_ids.add(case_id)
        request = {
            "version": 1,
            "id": case_id,
            "fen": board.fen(en_passant="fen"),
            "operation": operation,
            "required_files": required_files(stem),
        }
        self.stream.write(json.dumps(request, sort_keys=True, separators=(",", ":")) + "\n")
        self.counts[operation] += 1
        self.counts[stem or "KvK"] += 1


def generate_three_piece(writer, index, *, limit=None):
    for stem in (*THREE_PIECE, None):
        orientations = (False, True) if stem else (False,)
        for flipped, turn in product(orientations, (chess.WHITE, chess.BLACK)):
            count = 0
            if stem:
                positions = product(range(64), repeat=3)
            else:
                positions = combinations(range(64), 2)
            for squares in positions:
                if len(set(squares)) != len(squares):
                    continue
                if stem:
                    board = initial_board(stem, flipped, turn, squares)
                else:
                    board = initial_board("KvK", False, turn, squares)
                if not is_reachable_probe_position(board):
                    continue
                # The two ordered king placements are both needed for KvK.
                if stem is None:
                    swapped = initial_board("KvK", False, turn, squares[::-1])
                    for candidate in (board, swapped):
                        writer.emit(f"KvK:{int(turn)}:{count}:dtz", candidate, "dtz", None)
                        count += 1
                    continue
                actual = stem_for_board(board, index)
                if actual != stem:
                    raise ValueError(f"unexpected material for {stem}: {actual}")
                ident = f"{stem}:{int(flipped)}:{int(turn)}:{count}"
                writer.emit(f"{ident}:dtz", board, "dtz", stem)
                if count % 128 == 0:
                    writer.emit(f"{ident}:root", board, "root", stem)
                count += 1
                if limit is not None and count >= limit:
                    break
            if count == 0:
                raise ValueError(f"no legal positions for {stem} {flipped} {turn}")


def sample_material(writer, index, stem, seed, per_orientation, walks):
    selected = []
    for flipped, turn in product((False, True), (chess.WHITE, chess.BLACK)):
        rng = seeded_random(seed, f"placement:{stem}:{int(flipped)}:{int(turn)}")
        seen = set()
        wanted = per_orientation
        attempts = 0
        while len(seen) < wanted:
            attempts += 1
            if attempts > wanted * 100:
                raise ValueError(f"insufficient legal placements for {stem}")
            board = initial_board(
                stem, flipped, turn, rng.sample(range(64), len(stem.replace("v", "")))
            )
            if not is_reachable_probe_position(board):
                continue
            fen = board.fen(en_passant="fen")
            if fen in seen:
                continue
            seen.add(fen)
            selected.append(board)
            ident = f"{stem}:{int(flipped)}:{int(turn)}:{len(seen)}"
            writer.emit(f"{ident}:dtz", board, "dtz", stem)
            if len(seen) % 16 == 0:
                writer.emit(f"{ident}:root", board, "root", stem)
        if len(seen) != wanted:
            raise ValueError(f"incomplete placement bucket {stem}")

    rng = seeded_random(seed, f"walk:{stem}")
    board = selected[rng.randrange(len(selected))].copy(stack=False)
    unique = set()
    attempts = 0
    while len(unique) < walks:
        attempts += 1
        if attempts > walks * 100:
            raise ValueError(f"insufficient successor positions for {stem}")
        legal = list(board.legal_moves)
        if not legal:
            board = selected[rng.randrange(len(selected))].copy(stack=False)
            continue
        board.push(rng.choice(legal))
        if board.occupied.bit_count() > 5:
            raise ValueError("walk increased material beyond five")
        current_stem = stem_for_board(board, index)
        if board.occupied.bit_count() > 2 and current_stem is None:
            raise ValueError(f"missing successor material after {stem}")
        fen = board.fen(en_passant="fen")
        if fen in unique:
            if attempts % 16 == 0:
                board = selected[rng.randrange(len(selected))].copy(stack=False)
            continue
        unique.add(fen)
        ident = f"{stem}:walk:{len(unique)}"
        writer.emit(f"{ident}:dtz", board, "dtz", current_stem)
        if len(unique) % 16 == 0:
            writer.emit(f"{ident}:root", board, "root", current_stem)
        if attempts % 32 == 0:
            board = selected[rng.randrange(len(selected))].copy(stack=False)


def add_special_cases(writer, index):
    positions = {
        "zeroing-capture": "5k2/R7/8/8/5K2/p7/8/8 w - - 0 62",
        "promotion": "8/P7/8/8/8/8/4K3/7k w - - 0 1",
        "en-passant": "8/8/8/3pP3/8/8/4K3/7k w - d6 0 1",
        "mate": "7k/6Q1/5K2/8/8/8/8/8 b - - 0 1",
        "stalemate": "7k/5Q2/6K1/8/8/8/8/8 b - - 0 1",
    }
    for label, fen in positions.items():
        board = chess.Board(fen)
        if not board.is_valid():
            raise ValueError(f"invalid special position {label}")
        stem = stem_for_board(board, index)
        writer.emit(f"special:{label}:root", board, "root", stem)
        if label in ("zeroing-capture", "promotion"):
            for halfmoves in (99, 100):
                boundary = board.copy(stack=False)
                boundary.halfmove_clock = halfmoves
                writer.emit(
                    f"special:{label}:clock-{halfmoves}:root", boundary, "root", stem
                )
    queen = chess.Board("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1")
    losing_queen = queen.copy(stack=False)
    losing_queen.turn = chess.BLACK
    # Keep generated clocks within the standard fifty-move rule boundary.
    for halfmoves in (0, 1, 49, 87, 89, 98, 99, 100):
        board = queen.copy(stack=False)
        board.halfmove_clock = halfmoves
        writer.emit(f"special:clock-{halfmoves}:dtz", board, "dtz", "KQvK")
        writer.emit(f"special:clock-{halfmoves}:root", board, "root", "KQvK")
        losing_board = losing_queen.copy(stack=False)
        losing_board.halfmove_clock = halfmoves
        writer.emit(f"special:clock-loss-{halfmoves}:root", losing_board, "root", "KQvK")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    default_manifest = Path(__file__).resolve().parent.parent / "nix/syzygy-3-4-5.json"
    parser.add_argument("--manifest", type=Path, default=default_manifest)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=lambda value: int(value, 0), default=SEED)
    parser.add_argument("--placements-per-orientation", type=int, default=512)
    parser.add_argument("--walks-per-material", type=int, default=256)
    parser.add_argument("--three-piece-limit", type=int)
    parser.add_argument("--only-material")
    args = parser.parse_args(argv)
    if args.three_piece_limit is not None and args.three_piece_limit <= 0:
        parser.error("--three-piece-limit must be positive")
    if args.output.exists() or args.placements_per_orientation <= 0 or args.walks_per_material < 0:
        parser.error("output must be new and counts must be valid")
    index = material_index(args.manifest)
    if args.only_material and args.only_material not in index.values():
        parser.error("unknown material")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    invocation = {"seed": args.seed, "manifest": str(args.manifest),
                  "manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
                  "placements_per_orientation": args.placements_per_orientation,
                  "walks_per_material": args.walks_per_material,
                  "three_piece_limit": args.three_piece_limit,
                  "only_material": args.only_material}
    args.output.with_suffix(args.output.suffix + ".invocation.json").write_text(
        json.dumps(invocation, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    temporary = args.output.with_name(args.output.name + f".tmp.{__import__('os').getpid()}")
    with temporary.open("w", encoding="utf-8") as stream:
        writer = Writer(stream)
        if args.only_material:
            sample_material(writer, index, args.only_material, args.seed,
                            args.placements_per_orientation, args.walks_per_material)
        else:
            generate_three_piece(writer, index, limit=args.three_piece_limit)
            for stem in sorted(index.values()):
                if stem not in THREE_PIECE:
                    sample_material(writer, index, stem, args.seed,
                                    args.placements_per_orientation, args.walks_per_material)
            add_special_cases(writer, index)
    temporary.replace(args.output)
    print(json.dumps({"output": str(args.output), "cases": sum(
        writer.counts[operation] for operation in ("wdl", "dtz", "root")),
        "operations": {operation: writer.counts[operation]
                       for operation in ("wdl", "dtz", "root")}}, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
