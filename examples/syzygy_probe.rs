use std::{
    io::{self, BufRead as _, BufWriter, Write as _},
    path::{Path, PathBuf},
    str::FromStr as _,
};

use cozy_chess::{
    get_bishop_moves, get_king_moves, get_knight_moves, get_pawn_attacks, get_rook_moves, BitBoard,
    Board, Color as ChessColor, Piece as ChessPiece, Rank, Square,
};
use pyrrhic_rs::{
    Color, DtzProbeValue, EngineAdapter, Piece as ProbePiece, TableBases, WdlProbeResult,
};
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    id: String,
    fen: String,
    operation: Operation,
    #[serde(default)]
    required_files: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Wdl,
    Dtz,
    Root,
}

#[derive(Serialize)]
struct Distance {
    value: i32,
    rounded: Option<bool>,
}

#[derive(Serialize)]
struct MoveResult {
    uci: String,
    child_wdl: i32,
    child_dtz: Distance,
    zeroing: bool,
}

#[derive(Serialize)]
struct Response {
    version: u8,
    id: String,
    status: &'static str,
    error: Option<String>,
    wdl: Option<i32>,
    dtz: Option<Distance>,
    wdl50: Option<String>,
    root_status: Option<&'static str>,
    selected_move: Option<String>,
    moves: Option<Vec<MoveResult>>,
}

impl Response {
    fn new(id: String) -> Self {
        Self {
            version: 1,
            id,
            status: "ok",
            error: None,
            wdl: None,
            dtz: None,
            wdl50: None,
            root_status: None,
            selected_move: None,
            moves: None,
        }
    }

    fn fail(&mut self, status: &'static str, error: impl Into<String>) {
        self.status = status;
        self.error = Some(error.into());
    }
}

fn wdl_number(wdl: WdlProbeResult) -> i32 {
    match wdl {
        WdlProbeResult::Loss => -2,
        WdlProbeResult::BlessedLoss => -1,
        WdlProbeResult::Draw => 0,
        WdlProbeResult::CursedWin => 1,
        WdlProbeResult::Win => 2,
    }
}

fn ep_square(board: &Board) -> u32 {
    board
        .en_passant()
        .map(|file| {
            let rank = if board.side_to_move() == ChessColor::White {
                Rank::Sixth
            } else {
                Rank::Third
            };
            Square::new(file, rank) as u32
        })
        .unwrap_or(0)
}

fn probe_wdl(tb: &TableBases<Adapter>, board: &Board) -> Result<WdlProbeResult, String> {
    tb.probe_wdl(
        board.colors(ChessColor::White).0,
        board.colors(ChessColor::Black).0,
        board.pieces(ChessPiece::King).0,
        board.pieces(ChessPiece::Queen).0,
        board.pieces(ChessPiece::Rook).0,
        board.pieces(ChessPiece::Bishop).0,
        board.pieces(ChessPiece::Knight).0,
        board.pieces(ChessPiece::Pawn).0,
        ep_square(board),
        board.side_to_move() == ChessColor::White,
    )
    .map_err(|err| format!("WDL: {err:?}"))
}

fn probe_dtz(tb: &TableBases<Adapter>, board: &Board) -> Result<i32, String> {
    tb.probe_dtz(
        board.colors(ChessColor::White).0,
        board.colors(ChessColor::Black).0,
        board.pieces(ChessPiece::King).0,
        board.pieces(ChessPiece::Queen).0,
        board.pieces(ChessPiece::Rook).0,
        board.pieces(ChessPiece::Bishop).0,
        board.pieces(ChessPiece::Knight).0,
        board.pieces(ChessPiece::Pawn).0,
        ep_square(board),
        board.side_to_move() == ChessColor::White,
    )
    .map_err(|err| format!("DTZ: {err:?}"))
}

fn root_move_uci(from: u8, to: u8, promotion: ProbePiece) -> String {
    let mut uci = format!(
        "{}{}",
        Square::index(from as usize),
        Square::index(to as usize)
    );
    let suffix = match promotion {
        ProbePiece::Queen => "q",
        ProbePiece::Rook => "r",
        ProbePiece::Bishop => "b",
        ProbePiece::Knight => "n",
        ProbePiece::Pawn | ProbePiece::King => "",
    };
    uci.push_str(suffix);
    uci
}

fn respond(req: Request, path: &Path, tb: &TableBases<Adapter>) -> Response {
    let mut response = Response::new(req.id);
    if req.version != 1 {
        response.fail("invalid_request", "unsupported protocol version");
        return response;
    }
    for filename in &req.required_files {
        if filename.contains('/') || filename.contains('\\') || filename.starts_with('.') {
            response.fail("invalid_request", "invalid required filename");
            return response;
        }
        if !path.join(filename).is_file() {
            response.fail("missing_input_table", filename.clone());
            return response;
        }
    }
    let board = match Board::from_str(&req.fen) {
        Ok(board) => board,
        Err(err) => {
            response.fail("invalid_position", err.to_string());
            return response;
        }
    };
    match probe_wdl(tb, &board) {
        Ok(wdl) => response.wdl = Some(wdl_number(wdl)),
        Err(err) => {
            response.fail("probe_error", err);
            return response;
        }
    }
    if matches!(req.operation, Operation::Wdl) {
        return response;
    }
    match probe_dtz(tb, &board) {
        Ok(value) => {
            response.dtz = Some(Distance {
                value,
                rounded: None,
            });
        }
        Err(err) => {
            response.fail("probe_error", err);
            return response;
        }
    }
    if matches!(req.operation, Operation::Dtz) {
        return response;
    }

    let mut legal = Vec::new();
    board.generate_moves(|group| {
        legal.extend(group);
        false
    });
    let root = match tb.probe_root(
        board.colors(ChessColor::White).0,
        board.colors(ChessColor::Black).0,
        board.pieces(ChessPiece::King).0,
        board.pieces(ChessPiece::Queen).0,
        board.pieces(ChessPiece::Rook).0,
        board.pieces(ChessPiece::Bishop).0,
        board.pieces(ChessPiece::Knight).0,
        board.pieces(ChessPiece::Pawn).0,
        board.halfmove_clock().into(),
        ep_square(&board),
        board.side_to_move() == ChessColor::White,
    ) {
        Ok(root) => root,
        Err(err) => {
            response.fail("probe_error", format!("root: {err:?}"));
            return response;
        }
    };
    response.root_status = Some(match root.root {
        DtzProbeValue::Checkmate => "checkmate",
        DtzProbeValue::Stalemate => "stalemate",
        DtzProbeValue::DtzResult(best) => {
            let selected = root_move_uci(best.from_square, best.to_square, best.promotion);
            if !legal.iter().any(|mv| mv.to_string() == selected) {
                response.fail("invalid_root", format!("selected illegal move {selected}"));
                return response;
            }
            response.selected_move = Some(selected);
            "moves"
        }
        DtzProbeValue::Failed => {
            response.fail("probe_error", "root returned a failed value");
            return response;
        }
    });
    let mut moves = Vec::with_capacity(legal.len());
    for mv in legal {
        let mut child = board.clone();
        child.play_unchecked(mv);
        let child_wdl = match probe_wdl(tb, &child) {
            Ok(value) => -wdl_number(value),
            Err(err) => {
                response.fail("probe_error", format!("{mv}: {err}"));
                return response;
            }
        };
        let child_dtz = match probe_dtz(tb, &child) {
            Ok(value) => value,
            Err(err) => {
                response.fail("probe_error", format!("{mv}: {err}"));
                return response;
            }
        };
        moves.push(MoveResult {
            uci: mv.to_string(),
            child_wdl,
            child_dtz: Distance {
                value: child_dtz,
                rounded: None,
            },
            zeroing: child.halfmove_clock() == 0,
        });
    }
    response.moves = Some(moves);
    response
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--tables")) {
        return Err("usage: syzygy_probe --tables DIRECTORY".into());
    }
    let path: PathBuf = args.next().ok_or("missing table directory")?.into();
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let tb = TableBases::<Adapter>::new(path.to_str().ok_or("non-UTF8 table directory")?)
        .map_err(|err| format!("table discovery: {err:?}"))?;
    eprintln!("candidate discovered {}-piece tables", tb.max_pieces());

    let stdin = io::stdin().lock();
    let mut stdout = BufWriter::new(io::stdout().lock());
    for line in stdin.lines() {
        let line = line?;
        let req: Request = serde_json::from_str(&line)?;
        let response = respond(req, &path, &tb);
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("syzygy candidate: {err}");
        std::process::exit(1);
    }
}
