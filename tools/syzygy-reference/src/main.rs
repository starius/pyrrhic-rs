use std::{
    io::{self, BufRead as _, BufWriter, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use shakmaty::{fen::Fen, uci::UciMove, CastlingMode, Chess, Position as _};
use shakmaty_syzygy::{Dtz, MaybeRounded, Tablebase};

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
    rounded: bool,
}

impl From<MaybeRounded<Dtz>> for Distance {
    fn from(value: MaybeRounded<Dtz>) -> Self {
        match value {
            MaybeRounded::Precise(Dtz(value)) => Self {
                value,
                rounded: false,
            },
            MaybeRounded::Rounded(Dtz(value)) => Self {
                value,
                rounded: true,
            },
        }
    }
}

#[derive(Serialize)]
struct MoveResult {
    uci: String,
    child_wdl: i32,
    child_dtz: Distance,
    zeroing: bool,
    ep: bool,
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
    optimal_moves: Option<Vec<String>>,
    mate_moves: Option<Vec<String>>,
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
            optimal_moves: None,
            mate_moves: None,
            moves: None,
        }
    }

    fn fail(&mut self, status: &'static str, message: impl Into<String>) {
        self.status = status;
        self.error = Some(message.into());
    }
}

fn respond(req: Request, path: &Path, tables: &Tablebase<Chess>) -> Response {
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
    let fen = match req.fen.parse::<Fen>() {
        Ok(fen) => fen,
        Err(err) => {
            response.fail("invalid_position", err.to_string());
            return response;
        }
    };
    let position: Chess = match fen.into_position(CastlingMode::Standard) {
        Ok(position) => position,
        Err(err) => {
            response.fail("invalid_position", err.to_string());
            return response;
        }
    };
    let wdl = match tables.probe_wdl_after_zeroing(&position) {
        Ok(wdl) => wdl,
        Err(err) => {
            response.fail("probe_error", err.to_string());
            return response;
        }
    };
    response.wdl = Some(wdl as i32);
    if matches!(req.operation, Operation::Wdl) {
        return response;
    }
    match tables.probe_dtz(&position) {
        Ok(dtz) => response.dtz = Some(dtz.into()),
        Err(err) => {
            response.fail("probe_error", err.to_string());
            return response;
        }
    }
    match tables.probe_wdl(&position) {
        Ok(wdl50) => response.wdl50 = Some(format!("{wdl50:?}")),
        Err(err) => {
            response.fail("probe_error", err.to_string());
            return response;
        }
    }
    if matches!(req.operation, Operation::Dtz) {
        return response;
    }

    let legal = position.legal_moves();
    response.root_status = Some(if position.is_checkmate() {
        "checkmate"
    } else if position.is_stalemate() {
        "stalemate"
    } else {
        "moves"
    });
    let selected = match tables.best_move(&position) {
        Ok(selected) => selected,
        Err(err) => {
            response.fail("probe_error", err.to_string());
            return response;
        }
    };
    response.selected_move = selected.map(|(mv, _)| UciMove::from_standard(mv).to_string());

    let mut moves = Vec::with_capacity(legal.len());
    let mut rankings = Vec::with_capacity(legal.len());
    let mut mate_moves = Vec::new();
    for mv in legal {
        let mut child = position.clone();
        child.play_unchecked(mv);
        let child_wdl = match tables.probe_wdl_after_zeroing(&child) {
            Ok(wdl) => -(wdl as i32),
            Err(err) => {
                response.fail(
                    "probe_error",
                    format!("{}: {err}", UciMove::from_standard(mv)),
                );
                return response;
            }
        };
        let child_dtz = match tables.probe_dtz(&child) {
            Ok(dtz) => Distance::from(dtz),
            Err(err) => {
                response.fail(
                    "probe_error",
                    format!("{}: {err}", UciMove::from_standard(mv)),
                );
                return response;
            }
        };
        let uci = UciMove::from_standard(mv).to_string();
        let zeroing = mv.is_zeroing();
        // This ranking follows the GPL reference's best_move policy. Keep it
        // in this isolated reference process, never in the MIT backend.
        let child_checkmate = child.is_checkmate();
        let immediate_loss = child_dtz.value == -1 && child_checkmate;
        if child_checkmate {
            mate_moves.push(uci.clone());
        }
        rankings.push((
            -child_wdl,
            !immediate_loss,
            zeroing ^ (child_dtz.value < 0),
            -child_dtz.value,
            uci.clone(),
        ));
        moves.push(MoveResult {
            uci,
            child_wdl,
            child_dtz,
            zeroing,
            ep: matches!(mv, shakmaty::Move::EnPassant { .. }),
        });
    }
    let best = rankings
        .iter()
        .map(|entry| (&entry.0, &entry.1, &entry.2, &entry.3))
        .min();
    let optimal_moves = rankings
        .iter()
        .filter(|entry| Some((&entry.0, &entry.1, &entry.2, &entry.3)) == best)
        .map(|entry| entry.4.clone())
        .collect::<Vec<_>>();
    if response
        .selected_move
        .as_ref()
        .is_some_and(|selected| !optimal_moves.contains(selected))
    {
        response.fail(
            "oracle_inconsistent",
            "selected move is outside the optimal set",
        );
        return response;
    }
    response.optimal_moves = Some(optimal_moves);
    response.mate_moves = Some(mate_moves);
    response.moves = Some(moves);
    response
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--tables")) {
        return Err("usage: pyrrhic-syzygy-reference --tables DIRECTORY".into());
    }
    let path: PathBuf = args.next().ok_or("missing table directory")?.into();
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut tables = Tablebase::<Chess>::new();
    let count = tables.add_directory(&path)?;
    if count == 0 {
        return Err("no Syzygy tables found".into());
    }
    eprintln!(
        "reference loaded {count} table files from {}",
        path.display()
    );

    let stdin = io::stdin().lock();
    let mut stdout = BufWriter::new(io::stdout().lock());
    for line in stdin.lines() {
        let line = line?;
        let req: Request = serde_json::from_str(&line)?;
        let response = respond(req, &path, &tables);
        serde_json::to_writer(&mut stdout, &response)?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("syzygy reference: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
    fn ci_reference_preserves_signed_distance_and_rounding() {
        let path: PathBuf = std::env::var_os("SYZYGY_CI_PATH")
            .expect("SYZYGY_CI_PATH is required")
            .into();
        let mut tables = Tablebase::<Chess>::new();
        assert_eq!(tables.add_directory(&path).unwrap(), 10);
        let req = Request {
            version: 1,
            id: "queen".into(),
            fen: "7k/8/8/8/8/8/8/1Q2K3 w - - 0 1".into(),
            operation: Operation::Dtz,
            required_files: vec!["KQvK.rtbw".into(), "KQvK.rtbz".into()],
        };
        let result = respond(req, &path, &tables);
        assert_eq!(result.status, "ok");
        assert_eq!(result.wdl, Some(2));
        let distance = result.dtz.unwrap();
        assert_eq!(distance.value, 13);
        assert!(distance.rounded);
        assert_eq!(result.wdl50.as_deref(), Some("Win"));
    }
}
