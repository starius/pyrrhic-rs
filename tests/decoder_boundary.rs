use cozy_chess::{
    get_bishop_moves, get_king_moves, get_knight_moves, get_pawn_attacks, get_rook_moves, BitBoard,
    Color as ChessColor, Square,
};
use pyrrhic_rs::{Color, DtzProbeValue, EngineAdapter, TableBases};

#[derive(Clone)]
struct Adapter;

impl EngineAdapter for Adapter {
    fn pawn_attacks(color: Color, square: u64) -> u64 {
        get_pawn_attacks(
            Square::index(square as usize),
            if color == Color::White {
                ChessColor::White
            } else {
                ChessColor::Black
            },
        )
        .0
    }

    fn knight_attacks(square: u64) -> u64 {
        get_knight_moves(Square::index(square as usize)).0
    }

    fn bishop_attacks(square: u64, occupied: u64) -> u64 {
        get_bishop_moves(Square::index(square as usize), BitBoard(occupied)).0
    }

    fn rook_attacks(square: u64, occupied: u64) -> u64 {
        get_rook_moves(Square::index(square as usize), BitBoard(occupied)).0
    }

    fn queen_attacks(square: u64, occupied: u64) -> u64 {
        Self::bishop_attacks(square, occupied) | Self::rook_attacks(square, occupied)
    }

    fn king_attacks(square: u64) -> u64 {
        get_king_moves(Square::index(square as usize)).0
    }
}

// A KQvKP WDL block ends immediately before the decoder's speculative word
// refill. The direct signed-DTZ and root contracts cannot be asserted in a
// TSV root-choice fixture alone.
#[test]
#[ignore = "requires the complete SYZYGY_PATH 3-4-5 set"]
fn complete_tables_decode_the_last_block_without_crossing_sections() {
    let path = std::env::var("SYZYGY_PATH").expect("SYZYGY_PATH is required");
    let tables = TableBases::<Adapter>::new(path).unwrap();

    // k7/5p2/5b2/8/8/K7/8/3Q4 b - - 1 1
    assert_eq!(
        tables.probe_dtz(
            (1 << 16) | (1 << 3),
            (1 << 56) | (1 << 45) | (1 << 53),
            (1 << 16) | (1 << 56),
            1 << 3,
            0,
            1 << 45,
            0,
            1 << 53,
            0,
            false,
        ),
        Ok(-6)
    );

    // k7/5p2/5b2/8/3Q4/K7/8/8 w - - 0 1
    let root = tables
        .probe_root(
            (1 << 16) | (1 << 27),
            (1 << 56) | (1 << 45) | (1 << 53),
            (1 << 16) | (1 << 56),
            1 << 27,
            0,
            1 << 45,
            0,
            1 << 53,
            0,
            0,
            true,
        )
        .unwrap();
    assert!(matches!(
        root.root,
        DtzProbeValue::DtzResult(value) if value.from_square == 27 && value.to_square == 45
    ));
}
