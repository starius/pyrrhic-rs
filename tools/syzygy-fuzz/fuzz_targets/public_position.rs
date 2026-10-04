#![no_main]

use std::sync::OnceLock;

use cozy_chess::{
    get_bishop_moves, get_king_moves, get_knight_moves, get_pawn_attacks, get_rook_moves, BitBoard,
    Color as ChessColor, Square,
};
use libfuzzer_sys::fuzz_target;
use pyrrhic_rs::{Color, EngineAdapter, TableBases};

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

static TABLES: OnceLock<TableBases<Adapter>> = OnceLock::new();

fuzz_target!(|input: &[u8]| {
    if input.len() < 69 {
        return;
    }
    let directory = std::env::var("SYZYGY_CI_PATH")
        .expect("SYZYGY_CI_PATH must point to the compact table set");
    let tables =
        TABLES.get_or_init(|| TableBases::new(directory).expect("compact tables required"));
    let mut masks = [0u64; 8];
    for (number, mask) in masks.iter_mut().enumerate() {
        let start = number * 8;
        *mask = u64::from_le_bytes(input[start..start + 8].try_into().expect("checked length"));
    }
    let ep = u32::from_le_bytes(input[64..68].try_into().expect("checked length"));
    let turn = input[68] & 1 != 0;
    let [white, black, kings, queens, rooks, bishops, knights, pawns] = masks;
    let _ = tables.probe_wdl(
        white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
    );
    let _ = tables.probe_dtz(
        white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
    );
    let _ = tables.probe_root(
        white, black, kings, queens, rooks, bishops, knights, pawns, 0, ep, turn,
    );
});
