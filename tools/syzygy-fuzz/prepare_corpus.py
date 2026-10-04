#!/usr/bin/env python3
"""Generate local Syzygy fuzz seeds from the pinned compact table set."""

import argparse
from pathlib import Path

import chess


MATERIALS = ("KQvK", "KPvK", "KRvKP", "KNNvKR")
POSITIONS = (
    "7k/8/8/8/8/8/8/1Q2K3 w - - 0 1",
    "2k5/8/8/8/8/8/8/2Q1K3 b - - 0 1",
    "7k/6Q1/8/8/8/8/8/K7 w - - 0 1",
    "2k5/8/8/8/8/8/8/2Q1K3 w - - 0 1",
    "7k/8/8/8/8/8/4P3/K7 w - - 0 1",
    "k7/5p2/5b2/8/8/K7/8/3Q4 b - - 1 1",
    "k7/5p2/5b2/8/3Q4/K7/8/8 w - - 0 1",
    "8/8/8/3pP3/8/8/8/K6k w - d6 0 1",
)


def position_bytes(fen):
    board = chess.Board(fen)
    white = int(board.occupied_co[chess.WHITE])
    black = int(board.occupied_co[chess.BLACK])
    masks = (white, black) + tuple(
        int(board.pieces(piece, chess.WHITE) | board.pieces(piece, chess.BLACK))
        for piece in (
            chess.KING,
            chess.QUEEN,
            chess.ROOK,
            chess.BISHOP,
            chess.KNIGHT,
            chess.PAWN,
        )
    )
    ep = board.ep_square or 0
    return (
        b"".join(mask.to_bytes(8, "little") for mask in masks)
        + ep.to_bytes(4, "little")
        + bytes([int(board.turn)])
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tables", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent / "corpus"
    for target in ("metadata", "decoder", "public_position"):
        (root / target).mkdir(parents=True, exist_ok=True)

    for material_index, material in enumerate(MATERIALS):
        for type_index, suffix in enumerate(("rtbw", "rtbz")):
            data = (args.tables / f"{material}.{suffix}").read_bytes()
            selector = bytes([material_index * 2 + type_index])
            name = f"{material}-{suffix}"
            (root / "metadata" / name).write_bytes(selector + data)
            for size in (*range(0, min(96, len(data))), len(data) // 2, len(data) - 1):
                (root / "metadata" / f"{name}-prefix-{size}").write_bytes(
                    selector + data[:size]
                )

    for section in range(3):
        for index in (0, 1, 31331, 65535):
            for offset in (0, 255, 256, 511, 65535):
                payload = (
                    bytes([section])
                    + index.to_bytes(8, "little")
                    + offset.to_bytes(8, "little")
                )
                (root / "decoder" / f"section-{section}-index-{index}-offset-{offset}").write_bytes(
                    payload + bytes(64)
                )
    for number, fen in enumerate(POSITIONS):
        (root / "public_position" / f"position-{number}").write_bytes(
            position_bytes(fen)
        )
    print(f"prepared seeds in {root}")


if __name__ == "__main__":
    main()
