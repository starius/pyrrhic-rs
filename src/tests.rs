use std::str::FromStr;

use crate::{
    engine_adapter::{Color, EngineAdapter},
    tablebases::{TableBases, WdlProbeResult},
    DtzProbeValue, TBError,
};
use cozy_chess::*;

const SYZYGY_PATH: &str = env!("SYZYGY_PATH");
#[derive(Copy, Clone)]
struct CozyChessAdapter;

impl EngineAdapter for CozyChessAdapter {
    fn pawn_attacks(color: Color, sq: u64) -> u64 {
        let attacks = get_pawn_attacks(
            Square::index(sq as usize),
            if color == Color::Black {
                cozy_chess::Color::Black
            } else {
                cozy_chess::Color::White
            },
        );
        attacks.0
    }
    fn knight_attacks(sq: u64) -> u64 {
        get_knight_moves(Square::index(sq as usize)).0
    }
    fn bishop_attacks(sq: u64, occ: u64) -> u64 {
        get_bishop_moves(Square::index(sq as usize), BitBoard(occ)).0
    }
    fn rook_attacks(sq: u64, occ: u64) -> u64 {
        get_rook_moves(Square::index(sq as usize), BitBoard(occ)).0
    }
    fn king_attacks(sq: u64) -> u64 {
        get_king_moves(Square::index(sq as usize)).0
    }
    fn queen_attacks(sq: u64, occ: u64) -> u64 {
        (get_bishop_moves(Square::index(sq as usize), BitBoard(occ))
            | get_rook_moves(Square::index(sq as usize), BitBoard(occ)))
        .0
    }
}

#[test]
fn test_probe_kpvk() {
    let tb = loop {
        let test = TableBases::<CozyChessAdapter>::new(SYZYGY_PATH);
        if let Ok(tb) = test {
            break tb;
        }
    };
    let test_pos_wins = [
        ("6k1/8/8/3P4/4K3/8/8/8 w - - 0 1", 1),
        ("8/7k/1p6/1P6/7K/8/8/8 w - - 0 1", 21),
    ];
    let test_pos_draw = "6k1/8/8/3P4/4K3/8/8/8 b - - 0 1";

    for (win_pos, dtz_expected) in test_pos_wins {
        let test_board_win = Board::from_str(win_pos).unwrap();

        let wdl_win = tb.probe_wdl(
            test_board_win.colors(cozy_chess::Color::White).0,
            test_board_win.colors(cozy_chess::Color::Black).0,
            test_board_win.pieces(Piece::King).0,
            test_board_win.pieces(Piece::Queen).0,
            test_board_win.pieces(Piece::Rook).0,
            test_board_win.pieces(Piece::Bishop).0,
            test_board_win.pieces(Piece::Knight).0,
            test_board_win.pieces(Piece::Pawn).0,
            0, // no ep square
            test_board_win.side_to_move() == cozy_chess::Color::White,
        );
        assert_eq!(wdl_win, Ok(WdlProbeResult::Win));
        let direct_dtz = tb.probe_dtz(
            test_board_win.colors(cozy_chess::Color::White).0,
            test_board_win.colors(cozy_chess::Color::Black).0,
            test_board_win.pieces(Piece::King).0,
            test_board_win.pieces(Piece::Queen).0,
            test_board_win.pieces(Piece::Rook).0,
            test_board_win.pieces(Piece::Bishop).0,
            test_board_win.pieces(Piece::Knight).0,
            test_board_win.pieces(Piece::Pawn).0,
            0,
            true,
        );
        assert_eq!(direct_dtz, Ok(dtz_expected));
        let dtz_result = tb.probe_root(
            test_board_win.colors(cozy_chess::Color::White).0,
            test_board_win.colors(cozy_chess::Color::Black).0,
            test_board_win.pieces(Piece::King).0,
            test_board_win.pieces(Piece::Queen).0,
            test_board_win.pieces(Piece::Rook).0,
            test_board_win.pieces(Piece::Bishop).0,
            test_board_win.pieces(Piece::Knight).0,
            test_board_win.pieces(Piece::Pawn).0,
            0,
            0,
            test_board_win.side_to_move() == cozy_chess::Color::White,
        );

        assert!(match dtz_result.unwrap().root {
            DtzProbeValue::DtzResult(result) => i32::from(result.dtz) == dtz_expected,
            _ => false,
        })
    }
    let test_board_draw = Board::from_str(test_pos_draw).unwrap();
    let wdl_draw = tb.probe_wdl(
        test_board_draw.colors(cozy_chess::Color::White).0,
        test_board_draw.colors(cozy_chess::Color::Black).0,
        test_board_draw.pieces(Piece::King).0,
        test_board_draw.pieces(Piece::Queen).0,
        test_board_draw.pieces(Piece::Rook).0,
        test_board_draw.pieces(Piece::Bishop).0,
        test_board_draw.pieces(Piece::Knight).0,
        test_board_draw.pieces(Piece::Pawn).0,
        0, // no ep square
        test_board_draw.side_to_move() == cozy_chess::Color::White,
    );

    assert!(wdl_draw == Ok(WdlProbeResult::Draw));
}

#[test]
fn test_double_init() {
    let first_tb = loop {
        let test = TableBases::<CozyChessAdapter>::new(SYZYGY_PATH);
        if let Ok(tb) = test {
            break tb;
        }
    };
    let second_tb = TableBases::<CozyChessAdapter>::new(SYZYGY_PATH);

    assert!(matches!(second_tb, Err(TBError::AlreadyInitialized)));
    std::hint::black_box(first_tb);
}

#[test]
fn bad_path_does_not_block_later_initialization() {
    let empty = std::env::temp_dir().join(format!("pyrrhic-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let error = match TableBases::<CozyChessAdapter>::new(empty.to_str().unwrap()) {
        Ok(tb) => panic!(
            "empty directory unexpectedly loaded {} pieces",
            tb.max_pieces()
        ),
        Err(error) => error,
    };
    assert_eq!(error, TBError::BadPath);
    std::fs::remove_dir(&empty).unwrap();
    assert!(TableBases::<CozyChessAdapter>::new(SYZYGY_PATH).is_ok());
}

#[test]
fn wdl_only_directory_does_not_claim_dtz_coverage() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-wdl-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KQvK.rtbw")).unwrap();
    file.set_len(80).unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(tb.max_pieces(), 3);
    assert_eq!(unsafe { crate::tbprobe::TB_NUM_WDL }, 1);
    assert_eq!(unsafe { crate::tbprobe::TB_NUM_DTZ }, 0);
    drop(tb);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn three_versus_two_material_is_discovered() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-three-v-two-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KRPvKR.rtbw")).unwrap();
    file.set_len(80).unwrap();

    let tb = loop {
        match TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()) {
            Ok(tb) => break tb,
            Err(TBError::AlreadyInitialized) => std::thread::yield_now(),
            Err(error) => panic!("five-piece table was not discovered: {error:?}"),
        }
    };
    assert_eq!(tb.max_pieces(), 5);
    assert_eq!(unsafe { crate::tbprobe::TB_NUM_WDL }, 1);
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(windows)]
#[test]
fn absolute_windows_drive_path_loads() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-drive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KQvK.rtbw")).unwrap();
    file.set_len(80).unwrap();
    let path = dir.to_str().unwrap();
    assert!(path.contains(':'));
    let tb = TableBases::<CozyChessAdapter>::new(path).unwrap();
    assert_eq!(tb.max_pieces(), 3);
    drop(tb);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn unix_path_list_keeps_colon_separator() {
    let first = std::env::temp_dir().join(format!("pyrrhic-path-a-{}", std::process::id()));
    let second = std::env::temp_dir().join(format!("pyrrhic-path-b-{}", std::process::id()));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::File::create(second.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let path = format!("{}:{}", first.display(), second.display());
    let tb = TableBases::<CozyChessAdapter>::new(&path).unwrap();
    assert_eq!(tb.max_pieces(), 3);
    drop(tb);
    std::fs::remove_dir_all(first).unwrap();
    std::fs::remove_dir_all(second).unwrap();
}

#[test]
fn path_components_are_bounded_and_embedded_nul_fails() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-path-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let separator = if cfg!(windows) { ';' } else { ':' };
    let path = format!("{separator}{}{separator}{separator}", dir.display());
    let tb = TableBases::<CozyChessAdapter>::new(&path).unwrap();
    assert_eq!(tb.max_pieces(), 3);
    drop(tb);
    assert!(matches!(
        TableBases::<CozyChessAdapter>::new(format!("{}\0", dir.display())),
        Err(TBError::InitFailed)
    ));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_failed_lazy_probes_do_not_mutate_material_hash() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let tb = tb.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let board = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
                barrier.wait();
                for _ in 0..50 {
                    let result = tb.probe_wdl(
                        board.colors(cozy_chess::Color::White).0,
                        board.colors(cozy_chess::Color::Black).0,
                        board.pieces(Piece::King).0,
                        board.pieces(Piece::Queen).0,
                        board.pieces(Piece::Rook).0,
                        board.pieces(Piece::Bishop).0,
                        board.pieces(Piece::Knight).0,
                        board.pieces(Piece::Pawn).0,
                        0,
                        true,
                    );
                    assert_eq!(result, Err(TBError::ProbeFailed));
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(tb.max_pieces(), 3);
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

fn probe_wdl_for_board(
    tb: &TableBases<CozyChessAdapter>,
    board: &Board,
) -> Result<WdlProbeResult, TBError> {
    tb.probe_wdl(
        board.colors(cozy_chess::Color::White).0,
        board.colors(cozy_chess::Color::Black).0,
        board.pieces(Piece::King).0,
        board.pieces(Piece::Queen).0,
        board.pieces(Piece::Rook).0,
        board.pieces(Piece::Bishop).0,
        board.pieces(Piece::Knight).0,
        board.pieces(Piece::Pawn).0,
        0,
        board.side_to_move() == cozy_chess::Color::White,
    )
}

#[test]
fn short_header_after_discovery_returns_probe_failure() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-short-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KQvK.rtbw")).unwrap();
    file.set_len(80).unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    file.set_len(4).unwrap();
    let board = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
    assert_eq!(probe_wdl_for_board(&tb, &board), Err(TBError::ProbeFailed));
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_file_after_discovery_returns_probe_failure() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-missing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("KQvK.rtbw");
    std::fs::File::create(&file).unwrap().set_len(80).unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    std::fs::remove_file(file).unwrap();
    let board = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
    assert_eq!(probe_wdl_for_board(&tb, &board), Err(TBError::ProbeFailed));
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn test_multithread() {
    let pos = "8/7k/1p6/1P6/7K/8/8/8 w - - 0 1";
    let first_tb = loop {
        let test = TableBases::<CozyChessAdapter>::new(SYZYGY_PATH);
        if let Ok(tb) = test {
            break tb;
        }
    };
    let second_tb = first_tb.clone();

    let worker = std::thread::spawn(move || {
        let board = Board::from_fen(pos, false).unwrap();
        for _ in 0..1000 {
            std::hint::black_box({
                let _ = second_tb.probe_wdl(
                    board.colors(cozy_chess::Color::White).0,
                    board.colors(cozy_chess::Color::Black).0,
                    board.pieces(Piece::King).0,
                    board.pieces(Piece::Queen).0,
                    board.pieces(Piece::Rook).0,
                    board.pieces(Piece::Bishop).0,
                    board.pieces(Piece::Knight).0,
                    board.pieces(Piece::Pawn).0,
                    0,
                    board.side_to_move() == cozy_chess::Color::White,
                );
            });
        }
    });
    let board = Board::from_fen(pos, false).unwrap();
    for _ in 0..10000 {
        std::hint::black_box({
            let _ = first_tb.probe_wdl(
                board.colors(cozy_chess::Color::White).0,
                board.colors(cozy_chess::Color::Black).0,
                board.pieces(Piece::King).0,
                board.pieces(Piece::Queen).0,
                board.pieces(Piece::Rook).0,
                board.pieces(Piece::Bishop).0,
                board.pieces(Piece::Knight).0,
                board.pieces(Piece::Pawn).0,
                0,
                board.side_to_move() == cozy_chess::Color::White,
            );
        });
    }
    worker.join().unwrap();
}

#[test]
fn root_probe_works_with_cloned_worker_handle() {
    let tb = TableBases::<CozyChessAdapter>::new(SYZYGY_PATH).unwrap();
    let worker = tb.clone();
    let board = Board::from_str("8/7k/8/8/8/8/8/Q3K3 w - - 0 1").unwrap();
    let result = tb.probe_root(
        board.colors(cozy_chess::Color::White).0,
        board.colors(cozy_chess::Color::Black).0,
        board.pieces(Piece::King).0,
        board.pieces(Piece::Queen).0,
        board.pieces(Piece::Rook).0,
        board.pieces(Piece::Bishop).0,
        board.pieces(Piece::Knight).0,
        board.pieces(Piece::Pawn).0,
        0,
        0,
        true,
    );
    assert!(matches!(result.unwrap().root, DtzProbeValue::DtzResult(_)));
    std::hint::black_box(worker);
}
