use std::str::FromStr;

use crate::{
    engine_adapter::{Color, EngineAdapter},
    tablebases::{TableBases, WdlProbeResult},
    DtzProbeValue, TBError,
};
use cozy_chess::*;

fn full_syzygy_path() -> String {
    std::env::var("SYZYGY_PATH").expect("set SYZYGY_PATH for the full tablebase tests")
}
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
#[ignore = "requires SYZYGY_PATH with KPvKP and other full-set tables"]
fn test_probe_kpvk() {
    let tb = TableBases::<CozyChessAdapter>::new(full_syzygy_path()).unwrap();
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
fn independent_handles_can_load_the_same_path() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-independent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let first = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    let second = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(first.max_pieces(), 3);
    assert_eq!(second.max_pieces(), 3);
    drop(first);
    assert_eq!(second.max_pieces(), 3);
    drop(second);
    std::fs::remove_dir_all(dir).unwrap();
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
    let valid = std::env::temp_dir().join(format!("pyrrhic-valid-{}", std::process::id()));
    std::fs::create_dir_all(&valid).unwrap();
    std::fs::File::create(valid.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    assert!(TableBases::<CozyChessAdapter>::new(valid.to_str().unwrap()).is_ok());
    std::fs::remove_dir_all(valid).unwrap();
}

#[test]
fn wdl_only_directory_does_not_claim_dtz_coverage() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-wdl-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KQvK.rtbw")).unwrap();
    file.set_len(80).unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(tb.max_pieces(), 3);
    assert_eq!(tb.counts(), (1, 0));
    drop(tb);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn three_versus_two_material_is_discovered() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-three-v-two-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KRPvKR.rtbw")).unwrap();
    file.set_len(80).unwrap();

    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(tb.max_pieces(), 5);
    assert_eq!(tb.counts(), (1, 0));
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
#[ignore = "requires SYZYGY_PATH with KPvKP"]
fn test_multithread() {
    let pos = "8/7k/1p6/1P6/7K/8/8/8 w - - 0 1";
    let first_tb = TableBases::<CozyChessAdapter>::new(full_syzygy_path()).unwrap();
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
#[ignore = "requires SYZYGY_PATH with KQvK"]
fn root_probe_works_with_cloned_worker_handle() {
    let tb = TableBases::<CozyChessAdapter>::new(full_syzygy_path()).unwrap();
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

#[test]
fn replacement_preserves_existing_generation() {
    let base = std::env::temp_dir().join(format!("pyrrhic-reload-{}", std::process::id()));
    let small = base.join("small");
    let large = base.join("large");
    let empty = base.join("empty");
    std::fs::create_dir_all(&small).unwrap();
    std::fs::create_dir_all(&large).unwrap();
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::File::create(small.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    std::fs::File::create(large.join("KRPvKR.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(small.to_str().unwrap()).unwrap();
    let worker = tb.clone();
    let replacement = TableBases::<CozyChessAdapter>::new(large.to_str().unwrap()).unwrap();
    assert_eq!(worker.max_pieces(), 3);
    assert_eq!(replacement.max_pieces(), 5);
    assert!(matches!(
        TableBases::<CozyChessAdapter>::new(empty.to_str().unwrap()),
        Err(TBError::BadPath)
    ));
    assert_eq!(worker.max_pieces(), 3);
    drop(tb);
    assert_eq!(worker.max_pieces(), 3);
    drop(worker);
    drop(replacement);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn concurrent_clone_drops_release_generation() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::File::create(dir.join("KQvK.rtbw"))
        .unwrap()
        .set_len(80)
        .unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(16));
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let clone = tb.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                drop(clone);
            })
        })
        .collect();
    drop(tb);
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).is_ok());
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
fn invalid_pawn_placement_is_rejected_by_every_probe_entry_point() {
    let temporary = if std::env::var("SYZYGY_PATH").is_ok() {
        None
    } else {
        let dir = std::env::temp_dir().join(format!("pyrrhic-invalid-pos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for ext in ["rtbw", "rtbz"] {
            std::fs::File::create(dir.join(format!("KPvKP.{ext}")))
                .unwrap()
                .set_len(80)
                .unwrap();
        }
        Some(dir)
    };
    let path = std::env::var("SYZYGY_PATH")
        .unwrap_or_else(|_| temporary.as_ref().unwrap().to_str().unwrap().to_owned());
    let tb = TableBases::<CozyChessAdapter>::new(path).unwrap();

    // Both pawns are on the first rank. The secondary pawn would make the
    // translated encoder index BINOMIAL with a negative square offset.
    let white = (1u64 << 12) | (1u64 << 0);
    let black = (1u64 << 60) | (1u64 << 1);
    let kings = (1u64 << 12) | (1u64 << 60);
    let pawns = (1u64 << 0) | (1u64 << 1);
    assert_eq!(
        tb.probe_wdl(white, black, kings, 0, 0, 0, 0, pawns, 0, true),
        Err(TBError::ProbeFailed)
    );
    assert_eq!(
        tb.probe_dtz(white, black, kings, 0, 0, 0, 0, pawns, 0, true),
        Err(TBError::ProbeFailed)
    );
    assert_eq!(
        tb.probe_root(white, black, kings, 0, 0, 0, 0, pawns, 0, 0, true),
        Err(TBError::ProbeFailed)
    );
    drop(tb);
    if let Some(dir) = temporary {
        std::fs::remove_dir_all(dir).unwrap();
    }
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
    assert!(matches!(
        TableBases::<CozyChessAdapter>::new(format!("{}\0", dir.display())),
        Err(TBError::InitFailed)
    ));
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_failed_lazy_probes_leave_hash_and_other_generations_intact() {
    let dir = std::env::temp_dir().join(format!("pyrrhic-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = std::fs::File::create(dir.join("KQvK.rtbw")).unwrap();
    file.set_len(80).unwrap();
    let tb = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    let parallel = (0..8)
        .map(|_| {
            let tb = tb.clone();
            std::thread::spawn(move || {
                let board = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
                for _ in 0..50 {
                    assert_eq!(probe_wdl_for_board(&tb, &board), Err(TBError::ProbeFailed));
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in parallel {
        thread.join().unwrap();
    }
    assert_eq!(tb.max_pieces(), 3);
    let another = TableBases::<CozyChessAdapter>::new(dir.to_str().unwrap()).unwrap();
    assert_eq!(another.max_pieces(), 3);
    drop(another);
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
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
    assert_eq!(probe_wdl_for_board(&tb, &board), Err(TBError::ProbeFailed));
    drop(tb);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_probe_independent_generations() {
    let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let first = TableBases::<CozyChessAdapter>::new(&path).unwrap();
    let second = TableBases::<CozyChessAdapter>::new(&path).unwrap();
    assert_eq!(first.max_pieces(), 5);
    assert_eq!(first.counts(), (5, 5));
    let queen = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
    let split = Board::from_str("6rk/8/8/8/8/8/8/KNN5 w - - 0 1").unwrap();
    assert_eq!(probe_wdl_for_board(&first, &queen), Ok(WdlProbeResult::Win));
    assert_eq!(
        probe_wdl_for_board(&second, &split),
        Ok(WdlProbeResult::Draw)
    );
    drop(first);
    assert_eq!(
        probe_wdl_for_board(&second, &queen),
        Ok(WdlProbeResult::Win)
    );
    assert_eq!(
        second.probe_dtz(
            queen.colors(cozy_chess::Color::White).0,
            queen.colors(cozy_chess::Color::Black).0,
            queen.pieces(Piece::King).0,
            queen.pieces(Piece::Queen).0,
            queen.pieces(Piece::Rook).0,
            queen.pieces(Piece::Bishop).0,
            queen.pieces(Piece::Knight).0,
            queen.pieces(Piece::Pawn).0,
            0,
            true,
        ),
        Ok(13)
    );
}

#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_allow_concurrent_wdl_and_dtz_probes() {
    let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let tables = TableBases::<CozyChessAdapter>::new(path).unwrap();
    let board = Board::from_str("7k/8/8/8/8/8/8/1Q2K3 w - - 0 1").unwrap();
    let ready = std::sync::Barrier::new(3);
    std::thread::scope(|scope| {
        for worker in 0..3 {
            let tables = &tables;
            let board = &board;
            let ready = &ready;
            scope.spawn(move || {
                ready.wait();
                for _ in 0..500 {
                    if worker == 0 {
                        assert_eq!(
                            tables.probe_dtz(
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
                            ),
                            Ok(13)
                        );
                    } else {
                        assert_eq!(probe_wdl_for_board(tables, board), Ok(WdlProbeResult::Win));
                    }
                }
            });
        }
    });
}
