use cozy_chess::{
    get_bishop_moves, get_king_moves, get_knight_moves, get_pawn_attacks, get_rook_moves, BitBoard,
    Color as ChessColor, Square,
};
use pyrrhic_rs::{Color, DtzProbeValue, EngineAdapter, TBError, TableBases, WdlProbeResult};

#[derive(Clone)]
struct Adapter;

impl EngineAdapter for Adapter {
    fn pawn_attacks(color: Color, square: u64) -> u64 {
        let color = if color == Color::White {
            ChessColor::White
        } else {
            ChessColor::Black
        };
        get_pawn_attacks(Square::index(square as usize), color).0
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

#[derive(Clone, Copy)]
struct Position {
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    turn: bool,
}

const fn bit(square: u32) -> u64 {
    1u64 << square
}

impl Position {
    // 7k/8/8/8/8/8/8/1Q2K3 w - - 0 1
    const WITNESS: Self = Self {
        white: bit(1) | bit(4),
        black: bit(63),
        kings: bit(4) | bit(63),
        queens: bit(1),
        turn: true,
    };

    // 2k5/8/8/8/8/8/8/2Q1K3 w - - 0 1
    const ATTACKED_KING: Self = Self {
        white: bit(2) | bit(4),
        black: bit(58),
        kings: bit(4) | bit(58),
        queens: bit(2),
        turn: true,
    };

    // 7k/6Q1/8/8/8/8/8/K7 w - - 0 1: the old capture path removed h8.
    const CAPTURABLE_KING: Self = Self {
        white: bit(0) | bit(54),
        black: bit(63),
        kings: bit(0) | bit(63),
        queens: bit(54),
        turn: true,
    };
}

#[derive(Clone, Copy, Debug)]
enum Probe {
    Wdl,
    Dtz,
    Root,
}

fn probe(tb: &TableBases<Adapter>, pos: Position, kind: Probe) -> Result<i32, TBError> {
    match kind {
        Probe::Wdl => tb
            .probe_wdl(
                pos.white, pos.black, pos.kings, pos.queens, 0, 0, 0, 0, 0, pos.turn,
            )
            .map(|value| match value {
                WdlProbeResult::Win => 1,
                WdlProbeResult::Loss => -1,
                _ => 0,
            }),
        Probe::Dtz => tb.probe_dtz(
            pos.white, pos.black, pos.kings, pos.queens, 0, 0, 0, 0, 0, pos.turn,
        ),
        Probe::Root => tb
            .probe_root(
                pos.white, pos.black, pos.kings, pos.queens, 0, 0, 0, 0, 0, 0, pos.turn,
            )
            .map(|result| match result.root {
                DtzProbeValue::DtzResult(root) if root.wdl == WdlProbeResult::Win => 1,
                DtzProbeValue::DtzResult(root) if root.wdl == WdlProbeResult::Loss => -1,
                _ => 0,
            }),
    }
}

// This checks public first-load and generation behavior, not a root move that
// the TSV fixture schema can express.
#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_cache_dtz_failure_without_poisoning_wdl() {
    let source = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let destination = std::env::temp_dir().join(format!(
        "pyrrhic-load-isolation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&destination).unwrap();
    for suffix in ["rtbw", "rtbz"] {
        let name = format!("KQvK.{suffix}");
        std::fs::copy(
            std::path::Path::new(&source).join(&name),
            destination.join(name),
        )
        .unwrap();
    }

    let tables = TableBases::<Adapter>::new(destination.to_str().unwrap()).unwrap();
    std::fs::remove_file(destination.join("KQvK.rtbz")).unwrap();
    std::fs::write(destination.join("KQvK.rtbz"), [0; 80]).unwrap();
    assert_eq!(probe(&tables, Position::WITNESS, Probe::Wdl), Ok(1));
    assert_eq!(
        probe(&tables, Position::WITNESS, Probe::Dtz),
        Err(TBError::ProbeFailed)
    );
    assert_eq!(probe(&tables, Position::WITNESS, Probe::Wdl), Ok(1));

    std::fs::copy(
        std::path::Path::new(&source).join("KQvK.rtbz"),
        destination.join("KQvK.rtbz"),
    )
    .unwrap();
    assert_eq!(
        probe(&tables, Position::WITNESS, Probe::Dtz),
        Err(TBError::ProbeFailed)
    );
    let replacement = TableBases::<Adapter>::new(destination.to_str().unwrap()).unwrap();
    assert_eq!(probe(&replacement, Position::WITNESS, Probe::Dtz), Ok(13));
    drop(replacement);
    drop(tables);
    std::fs::remove_dir_all(destination).unwrap();
}

// This tests direct API failure and cache isolation, which cannot be expressed
// as a TSV assertion about Ember's chosen root move.
#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_reject_attacked_opposing_king_without_poisoning_tables() {
    let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    for kind in [Probe::Wdl, Probe::Dtz, Probe::Root] {
        for warm in [false, true] {
            let tb = TableBases::<Adapter>::new(&path).unwrap();
            if warm {
                assert_eq!(probe(&tb, Position::WITNESS, Probe::Wdl), Ok(1));
                assert_eq!(probe(&tb, Position::WITNESS, Probe::Dtz), Ok(13));
            }
            for invalid in [Position::ATTACKED_KING, Position::CAPTURABLE_KING] {
                assert_eq!(probe(&tb, invalid, kind), Err(TBError::ProbeFailed));
                let expected = match kind {
                    Probe::Wdl | Probe::Root => 1,
                    Probe::Dtz => 13,
                };
                assert_eq!(probe(&tb, Position::WITNESS, kind), Ok(expected));
            }
        }
    }
}

// The side to move may be in check; the API must still probe it normally.
#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_accept_legal_side_to_move_in_check() {
    let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let tb = TableBases::<Adapter>::new(path).unwrap();
    let checked = Position {
        turn: false,
        ..Position::ATTACKED_KING
    };
    assert_eq!(probe(&tb, checked, Probe::Wdl), Ok(-1));
    assert!(probe(&tb, checked, Probe::Dtz).unwrap() < 0);
    assert_eq!(probe(&tb, checked, Probe::Root), Ok(-1));
}

// This checks the public packed move array and successor clock at the API's
// u8 limit. Ember's TSV move fixtures cannot set a 254/255 halfmove clock.
#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_root_array_saturates_at_maximum_clock() {
    let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let tb = TableBases::<Adapter>::new(path).unwrap();
    let pos = Position::WITNESS;
    let root_at = |clock| {
        tb.probe_root(
            pos.white, pos.black, pos.kings, pos.queens, 0, 0, 0, 0, clock, 0, pos.turn,
        )
        .unwrap()
    };
    let at_254 = root_at(254);
    let at_255 = root_at(255);
    assert!(matches!(
        at_255.root,
        DtzProbeValue::DtzResult(value) if value.wdl == WdlProbeResult::CursedWin
    ));
    assert!(at_255.num_moves > 0);
    assert!(at_255.moves[..at_255.num_moves]
        .iter()
        .all(|value| matches!(value, DtzProbeValue::DtzResult(_))));
    assert!(at_255.moves[at_255.num_moves..]
        .iter()
        .all(|value| matches!(value, DtzProbeValue::Failed)));
    assert_eq!(at_254.root, at_255.root);
    assert_eq!(at_254.num_moves, at_255.num_moves);
    assert_eq!(at_254.moves, at_255.moves);
}

// A malformed successor WDL leaf must fail the public probe even if capture
// search would otherwise discard its out-of-range score. This direct probe
// contract cannot be expressed as a TSV move fixture.
#[test]
#[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
fn ci_compact_tables_reject_invalid_successor_wdl() {
    let source = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
    let destination = std::env::temp_dir().join(format!(
        "pyrrhic-invalid-wdl-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&destination).unwrap();
    std::fs::copy(
        std::path::Path::new(&source).join("KRvKP.rtbw"),
        destination.join("KRvKP.rtbw"),
    )
    .unwrap();

    // A complete 80-byte constant KRvK WDL table with an invalid leaf 5.
    let mut corrupt = [0u8; 80];
    corrupt[..4].copy_from_slice(&0x5d23_e871_u32.to_le_bytes());
    corrupt[4] = 1;
    corrupt[5..9].copy_from_slice(&[0, 0x6e, 0xe6, 0x44]);
    corrupt[10..14].copy_from_slice(&[0x80, 4, 0x80, 5]);
    std::fs::write(destination.join("KRvK.rtbw"), corrupt).unwrap();

    let tables = TableBases::<Adapter>::new(destination.to_str().unwrap()).unwrap();
    // 7k/1p6/8/8/8/8/1R6/K7 w - - 0 1: Rxb7 reaches corrupt KRvK.
    assert_eq!(
        tables.probe_wdl(
            bit(0) | bit(9),
            bit(49) | bit(63),
            bit(0) | bit(63),
            0,
            bit(9),
            0,
            0,
            bit(49),
            0,
            true,
        ),
        Err(TBError::ProbeFailed)
    );
    drop(tables);
    std::fs::remove_dir_all(destination).unwrap();
}
