use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use crate::{
    engine_adapter::{Color, EngineAdapter, Piece},
    tbprobe::{self, tb_init, tb_probe_root, tb_probe_wdl, StateOwner},
};

/// Tablebase error type
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TBError {
    /// No tablebase files were found with the given search path
    BadPath,
    /// Tablebase initialization failed
    InitFailed,
    /// Probing the tablebases failed
    ProbeFailed,
}

/// Result of a Win-Draw-Loss (WDL) table probe
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum WdlProbeResult {
    /// The position is losing for the side to play
    Loss,
    /// The position is a forced loss for the side to play, but the 50-move rule
    /// makes this position a draw instead
    BlessedLoss,
    /// The position is drawn
    Draw,
    /// The position is a forced win for the side to play, but the 50-move rule
    /// makes this position a draw instead
    CursedWin,
    /// The position is winning for the side to play
    Win,
}

/// Result of a successful DTZ probe
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DtzResult {
    /// WDL value of the position
    pub wdl: WdlProbeResult,
    /// Start square of the suggested move
    pub from_square: u8,
    /// End square of the suggested move
    pub to_square: u8,
    /// Promotion of the suggested move. `[Piece::Pawn]` if there is no promotion
    pub promotion: Piece,
    /// Whether this move is an en passent capture
    pub ep: bool,
    /// Number of plies from this position to a zeroing move (pawn move or capture)
    pub dtz: u16,
}

/// DTZ value for a single position extracted from the tablebases
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DtzProbeValue {
    /// The position is a stalemate
    Stalemate,
    /// The position is a checkmate
    Checkmate,
    /// The DTZ probe failed
    Failed,
    /// The DTZ probe succeeded
    DtzResult(DtzResult),
}

/// Result of a Distance-To-Zero (DTZ) table probe
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DtzProbeResult {
    /// DTZ probe result for the root of the position
    pub root: DtzProbeValue,
    /// DTZ probe results for all moves from the root.
    pub moves: [DtzProbeValue; 256],
    /// The number of moves score in `moves`, the remaining entries will be `[DtzProbeValue::Failed]`
    pub num_moves: usize,
}

/// Handle to tablebase probing code.
///
/// ## Usage
/// This struct provides a safe wrapper around the unsafe Pyrrhic API. It can be
/// safely sent across threads and manages initialization and de-initialization of the tablebases.
#[derive(Clone)]
pub struct TableBases<E: EngineAdapter> {
    handle: Arc<StateOwner>,
    _engine: PhantomData<E>,
}

static ROOT_PROBE_MUTEX: Mutex<()> = Mutex::new(());

/// Reject bitboards that the translated encoder cannot safely represent.
/// The caller may pass an arbitrary position through this safe API.
#[inline]
#[allow(clippy::too_many_arguments)]
fn valid_probe_position<E: EngineAdapter>(
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    rooks: u64,
    bishops: u64,
    knights: u64,
    pawns: u64,
    ep: u32,
    turn: bool,
) -> bool {
    let occupied = white | black;
    if white & black != 0
        || !(2..=7).contains(&occupied.count_ones())
        || (kings & white).count_ones() != 1
        || (kings & black).count_ones() != 1
        || pawns & 0xff00_0000_0000_00ff != 0
    {
        return false;
    }

    let mut seen = 0;
    for pieces in [kings, queens, rooks, bishops, knights, pawns] {
        if seen & pieces != 0 {
            return false;
        }
        seen |= pieces;
    }
    if seen != occupied {
        return false;
    }

    // The pawnless king-pair index contains -1 for adjacent kings. Do not
    // pass such an index to the decoder, even when a material file exists.
    let white_king = (kings & white).trailing_zeros();
    let black_king = (kings & black).trailing_zeros();
    if (white_king % 8).abs_diff(black_king % 8) <= 1
        && (white_king / 8).abs_diff(black_king / 8) <= 1
    {
        return false;
    }

    // A legal chess position can have the side to move in check, but the
    // opposing king cannot already be attacked. Otherwise the translated
    // capture generator may remove that king and index a nonexistent square.
    let (attacker, target_king, reverse_pawn_color) = if turn {
        (white, black_king, Color::Black)
    } else {
        (black, white_king, Color::White)
    };
    let target_king = u64::from(target_king);
    if E::rook_attacks(target_king, occupied) & attacker & (rooks | queens) != 0
        || E::bishop_attacks(target_king, occupied) & attacker & (bishops | queens) != 0
        || E::knight_attacks(target_king) & attacker & knights != 0
        || E::pawn_attacks(reverse_pawn_color, target_king) & attacker & pawns != 0
    {
        return false;
    }

    if ep != 0 {
        let expected_rank = if turn { 5 } else { 2 };
        if ep >= 64 || ep / 8 != expected_rank || occupied & (1u64 << ep) != 0 {
            return false;
        }
        let captured = if turn { ep - 8 } else { ep + 8 };
        let opponent = if turn { black } else { white };
        if pawns & opponent & (1u64 << captured) == 0 {
            return false;
        }
    }
    true
}

impl<E: EngineAdapter> TableBases<E> {
    /// Initialize the tablebases
    /// * `path` - tablebase directories separated by ':' on Unix or ';' on Windows.
    ///
    /// ## Notes:
    /// Absolute paths with Windows drive letters are accepted.
    ///
    pub fn new<P: AsRef<str>>(path: P) -> Result<Self, TBError> {
        let handle = Arc::new(StateOwner::new());
        {
            let _scope = handle.enter();
            if !unsafe { tb_init(path.as_ref()) } {
                return Err(TBError::InitFailed);
            }
        }
        if handle.max_pieces() == 0 {
            return Err(TBError::BadPath);
        }
        Ok(Self {
            handle,
            _engine: PhantomData,
        })
    }

    /// Probe the Win-Draw-Loss (WDL) tables.
    #[allow(clippy::too_many_arguments)]
    pub fn probe_wdl(
        &self,
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        ep: u32,
        turn: bool,
    ) -> Result<WdlProbeResult, TBError> {
        if !valid_probe_position::<E>(
            white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
        ) {
            return Err(TBError::ProbeFailed);
        }
        let _scope = self.handle.enter();
        let result = unsafe {
            tb_probe_wdl::<E>(
                white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
            )
        };

        match result {
            0 => Ok(WdlProbeResult::Loss),
            1 => Ok(WdlProbeResult::BlessedLoss),
            2 => Ok(WdlProbeResult::Draw),
            3 => Ok(WdlProbeResult::CursedWin),
            4 => Ok(WdlProbeResult::Win),
            _ => Err(TBError::ProbeFailed),
        }
    }

    /// Probe signed distance to the next zeroing move from the side to move.
    /// This does not apply the halfmove clock.
    #[allow(clippy::too_many_arguments)]
    pub fn probe_dtz(
        &self,
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        ep: u32,
        turn: bool,
    ) -> Result<i32, TBError> {
        if !valid_probe_position::<E>(
            white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
        ) {
            return Err(TBError::ProbeFailed);
        }
        let _scope = self.handle.enter();
        unsafe {
            tbprobe::tb_probe_dtz::<E>(
                white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
            )
        }
        .ok_or(TBError::ProbeFailed)
    }

    /// Probe the Distance-To-Zero (DTZ) tables.
    ///
    /// ## Notes:
    /// The underlying `probe_root` function is not thread safe. Root probes
    /// are serialized so shared handles can still select tablebase moves.
    #[allow(clippy::too_many_arguments)]
    pub fn probe_root(
        &self,
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        rule50: u32,
        ep: u32,
        turn: bool,
    ) -> Result<DtzProbeResult, TBError> {
        if rule50 > u8::MAX as u32
            || !valid_probe_position::<E>(
                white, black, kings, queens, rooks, bishops, knights, pawns, ep, turn,
            )
        {
            return Err(TBError::ProbeFailed);
        }
        let _scope = self.handle.enter();
        let _guard = ROOT_PROBE_MUTEX.lock().map_err(|_| TBError::ProbeFailed)?;
        let mut results = [0u32; 256];
        let result = unsafe {
            tb_probe_root::<E>(
                white,
                black,
                kings,
                queens,
                rooks,
                bishops,
                knights,
                pawns,
                rule50,
                ep,
                turn,
                results.as_mut_ptr(),
            )
        };

        let result = extract_dtz_result(result);
        let mut dtz_data = DtzProbeResult {
            root: result,
            moves: [DtzProbeValue::Failed; 256],
            num_moves: 0,
        };
        match result {
            DtzProbeValue::Failed => return Err(TBError::ProbeFailed),
            DtzProbeValue::Stalemate | DtzProbeValue::Checkmate => Ok(dtz_data),
            DtzProbeValue::DtzResult(_) => {
                for value in results.map(extract_dtz_result) {
                    match value {
                        DtzProbeValue::Failed => break,
                        other => {
                            dtz_data.moves[dtz_data.num_moves] = other;
                            dtz_data.num_moves += 1;
                        }
                    }
                }
                Ok(dtz_data)
            }
        }
    }

    /// The maximum number of pieces (including kings) that the loaded tablebases can be probed with
    pub fn max_pieces(&self) -> u32 {
        self.handle.max_pieces()
    }

    /// Material names found in WDL files with plausible sizes, with DTZ
    /// availability from the same discovery pass. Headers load on first probe.
    pub fn materials(&self) -> Vec<(String, bool)> {
        self.handle.materials()
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (i32, i32) {
        self.handle.counts()
    }
}

fn extract_dtz_result(result: u32) -> DtzProbeValue {
    match result {
        0xFFFFFFFF => DtzProbeValue::Failed,
        2 => DtzProbeValue::Stalemate,
        4 => DtzProbeValue::Checkmate,
        other => {
            let wdl_result = other & 0xF;
            let to_square = (other & 0x3F0) >> 4;
            let from_square = (other & 0xFC00) >> 10;
            let promotion = (other & 0x70000) >> 16;
            let ep = (other & 0x80000) >> 19;
            let dtz = (other & 0xFFF00000) >> 20;

            DtzProbeValue::DtzResult(DtzResult {
                wdl: match wdl_result {
                    0 => WdlProbeResult::Loss,
                    1 => WdlProbeResult::BlessedLoss,
                    2 => WdlProbeResult::Draw,
                    3 => WdlProbeResult::CursedWin,
                    4 => WdlProbeResult::Win,
                    _ => unreachable!(),
                },
                from_square: from_square as u8,
                to_square: to_square as u8,
                promotion: match promotion {
                    1 => Piece::Queen,
                    2 => Piece::Rook,
                    3 => Piece::Bishop,
                    4 => Piece::Knight,
                    _ => Piece::Pawn,
                },
                ep: ep != 0,
                dtz: dtz as u16,
            })
        }
    }
}

#[cfg(test)]
mod position_tests {
    use super::valid_probe_position;
    use crate::engine_adapter::{Color, EngineAdapter};
    use cozy_chess::{
        get_bishop_moves, get_king_moves, get_knight_moves, get_pawn_attacks, get_rook_moves,
        BitBoard, Square,
    };

    #[derive(Clone)]
    struct TestAdapter;

    impl EngineAdapter for TestAdapter {
        fn pawn_attacks(color: Color, square: u64) -> u64 {
            get_pawn_attacks(
                Square::index(square as usize),
                if color == Color::White {
                    cozy_chess::Color::White
                } else {
                    cozy_chess::Color::Black
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

    #[test]
    fn reject_bitboards_that_the_encoder_cannot_represent() {
        let kings = (1u64 << 12) | (1u64 << 60);
        let pawns = (1u64 << 0) | (1u64 << 1);
        let white = (1u64 << 12) | (1u64 << 0);
        let black = (1u64 << 60) | (1u64 << 1);
        let valid = |white, black, kings, pawns, ep| {
            valid_probe_position::<TestAdapter>(white, black, kings, 0, 0, 0, 0, pawns, ep, true)
        };
        assert!(!valid(white, black, kings, pawns, 0));
        assert!(!valid(white | black, black, kings, pawns, 0));
        assert!(!valid(white, black, kings | pawns, pawns, 0));
        let valid_white = (1u64 << 12) | (1u64 << 36);
        let valid_black = (1u64 << 60) | (1u64 << 35);
        let valid_pawns = (1u64 << 36) | (1u64 << 35);
        assert!(valid(valid_white, valid_black, kings, valid_pawns, 43));
        let adjacent_kings = (1u64 << 12) | (1u64 << 20);
        assert!(!valid(
            valid_white,
            (1u64 << 20) | (1u64 << 35),
            adjacent_kings,
            valid_pawns,
            43
        ));
        assert!(!valid(valid_white, valid_black, kings, valid_pawns, 19));
        assert!(!valid(valid_white, valid_black, kings, valid_pawns, 64));
    }

    #[test]
    fn reject_attacked_opposing_king_but_accept_blocked_rays_and_check() {
        let bit = |square| 1u64 << square;
        let valid = |white, black, kings, queens, rooks, bishops, knights, pawns, turn| {
            valid_probe_position::<TestAdapter>(
                white, black, kings, queens, rooks, bishops, knights, pawns, 0, turn,
            )
        };

        let kings = bit(0) | bit(63); // White Ka1, Black Kh8.
        assert!(!valid(
            bit(0) | bit(7),
            bit(63),
            kings,
            0,
            bit(7),
            0,
            0,
            0,
            true
        ));
        assert!(!valid(
            bit(0) | bit(53),
            bit(63),
            kings,
            0,
            0,
            0,
            bit(53),
            0,
            true
        ));
        assert!(!valid(
            bit(0) | bit(54),
            bit(63),
            kings,
            0,
            0,
            0,
            0,
            bit(54),
            true
        ));
        assert!(valid(
            bit(0) | bit(7) | bit(31),
            bit(63),
            kings,
            0,
            bit(7),
            0,
            0,
            bit(31),
            true
        ));

        let bishop_kings = bit(1) | bit(63); // White Kb1 leaves a1 free.
        assert!(!valid(
            bit(1) | bit(0),
            bit(63),
            bishop_kings,
            0,
            0,
            bit(0),
            0,
            0,
            true
        ));
        assert!(valid(
            bit(1) | bit(0) | bit(18),
            bit(63),
            bishop_kings,
            0,
            0,
            bit(0),
            0,
            bit(18),
            true,
        ));

        // Black's b2 pawn attacks White's a1 king. Reverse pawn attacks are
        // needed when checking attacks from the target king square.
        assert!(!valid(
            bit(0),
            bit(63) | bit(9),
            kings,
            0,
            0,
            0,
            0,
            bit(9),
            false
        ));
        assert!(valid(
            bit(0),
            bit(63) | bit(9),
            kings,
            0,
            0,
            0,
            0,
            bit(9),
            true
        ));

        // A side to move in check is legal; the reverse orientation is not.
        let checked_kings = bit(4) | bit(58);
        let white = bit(4) | bit(2);
        let black = bit(58);
        assert!(valid(
            white,
            black,
            checked_kings,
            bit(2),
            0,
            0,
            0,
            0,
            false
        ));
        assert!(!valid(
            white,
            black,
            checked_kings,
            bit(2),
            0,
            0,
            0,
            0,
            true
        ));
    }
}
