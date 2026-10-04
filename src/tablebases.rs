use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use crate::{
    engine_adapter::{EngineAdapter, Piece},
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
