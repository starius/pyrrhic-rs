//! Value-based Syzygy probing over owned, immutable table generations.
#![forbid(unsafe_code)]

use crate::{
    engine_adapter::EngineAdapter,
    table_lookup::{material_key, probe_table_value, Generation, TableProbeValue},
    table_moves::{generate_captures, generate_moves, MoveList},
    table_position::{
        apply_move, is_capture, is_en_passant, is_pawn_move, side_to_move_is_in_check,
        ValidatedPosition,
    },
    tbprobe::{pyrrhic_move_from, pyrrhic_move_promotes, pyrrhic_move_to, PyrrhicMove},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProbeError;

#[derive(Clone, Copy)]
struct WdlValue {
    score: i32,
    zeroing: bool,
}

pub(crate) struct RootValue {
    pub(crate) code: u32,
    pub(crate) moves: [u32; 256],
    pub(crate) len: usize,
}

const WDL_TO_DTZ: [i32; 5] = [-1, -101, 0, 101, 1];

fn table_wdl(owner: &Generation, pos: &ValidatedPosition) -> Result<i32, ProbeError> {
    match probe_table_value(owner, pos, 0, false) {
        Ok(TableProbeValue::Value(value)) => Ok(value),
        _ => Err(ProbeError),
    }
}

fn table_dtz(
    owner: &Generation,
    pos: &ValidatedPosition,
    wdl: i32,
) -> Result<Option<i32>, ProbeError> {
    match probe_table_value(owner, pos, wdl, true) {
        Ok(TableProbeValue::Value(value)) => Ok(Some(value)),
        Ok(TableProbeValue::WrongSide) => Ok(None),
        _ => Err(ProbeError),
    }
}

fn successor<E: EngineAdapter>(
    pos: &ValidatedPosition,
    candidate: PyrrhicMove,
) -> Result<Option<ValidatedPosition>, ProbeError> {
    apply_move::<E>(pos, candidate).map_err(|_| ProbeError)
}

fn legal_moves<E: EngineAdapter>(pos: &ValidatedPosition) -> Result<MoveList<256>, ProbeError> {
    let mut legal = MoveList::new();
    for &candidate in generate_moves::<E>(pos).map_err(|_| ProbeError)?.as_slice() {
        if successor::<E>(pos, candidate)?.is_some() {
            legal.push(candidate).map_err(|_| ProbeError)?;
        }
    }
    Ok(legal)
}

fn is_mate<E: EngineAdapter>(pos: &ValidatedPosition) -> Result<bool, ProbeError> {
    if !side_to_move_is_in_check::<E>(pos).map_err(|_| ProbeError)? {
        return Ok(false);
    }
    for &candidate in generate_moves::<E>(pos).map_err(|_| ProbeError)?.as_slice() {
        if successor::<E>(pos, candidate)?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn probe_ab<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
    mut alpha: i32,
    beta: i32,
) -> Result<i32, ProbeError> {
    if pos.ep != 0 {
        return Err(ProbeError);
    }
    for &candidate in generate_captures::<E>(pos)
        .map_err(|_| ProbeError)?
        .as_slice()
    {
        if is_capture(pos, candidate) {
            if let Some(next) = successor::<E>(pos, candidate)? {
                let value = -probe_ab::<E>(owner, &next, -beta, -alpha)?;
                if value > alpha {
                    if value >= beta {
                        return Ok(value);
                    }
                    alpha = value;
                }
            }
        }
    }
    Ok(alpha.max(table_wdl(owner, pos)?))
}

fn probe_wdl<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
) -> Result<WdlValue, ProbeError> {
    let mut best_capture = -3;
    let mut best_ep = -3;
    for &candidate in generate_captures::<E>(pos)
        .map_err(|_| ProbeError)?
        .as_slice()
    {
        if is_capture(pos, candidate) {
            if let Some(next) = successor::<E>(pos, candidate)? {
                let value = -probe_ab::<E>(owner, &next, -2, -best_capture)?;
                if value > best_capture {
                    if value == 2 {
                        return Ok(WdlValue {
                            score: 2,
                            zeroing: true,
                        });
                    }
                    if is_en_passant(pos, candidate) {
                        best_ep = best_ep.max(value);
                    } else {
                        best_capture = value;
                    }
                }
            }
        }
    }
    let table = table_wdl(owner, pos)?;
    if best_ep > best_capture {
        if best_ep > table {
            return Ok(WdlValue {
                score: best_ep,
                zeroing: true,
            });
        }
        best_capture = best_ep;
    }
    if best_capture >= table {
        return Ok(WdlValue {
            score: best_capture,
            zeroing: best_capture > 0,
        });
    }
    if best_ep > -3 && table == 0 {
        let mut has_non_ep_legal = false;
        for &candidate in generate_moves::<E>(pos).map_err(|_| ProbeError)?.as_slice() {
            if !is_en_passant(pos, candidate) && successor::<E>(pos, candidate)?.is_some() {
                has_non_ep_legal = true;
                break;
            }
        }
        if !has_non_ep_legal && !side_to_move_is_in_check::<E>(pos).map_err(|_| ProbeError)? {
            return Ok(WdlValue {
                score: best_ep,
                zeroing: true,
            });
        }
    }
    Ok(WdlValue {
        score: table,
        zeroing: false,
    })
}

fn wdl_to_dtz(wdl: i32) -> Result<i32, ProbeError> {
    let index = usize::try_from(wdl + 2).map_err(|_| ProbeError)?;
    WDL_TO_DTZ.get(index).copied().ok_or(ProbeError)
}

fn probe_dtz_inner<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
    depth: u16,
) -> Result<i32, ProbeError> {
    // A wrong-side DTZ entry is resolved by probing successor positions.
    // Reject a corrupt table or adapter that would otherwise recurse forever.
    if depth > 256 {
        return Err(ProbeError);
    }
    let wdl = probe_wdl::<E>(owner, pos)?;
    if wdl.score == 0 {
        return Ok(0);
    }
    let base = wdl_to_dtz(wdl.score)?;
    if wdl.zeroing {
        return Ok(base);
    }
    let mut positive_moves = None;
    if wdl.score > 0 {
        let moves = legal_moves::<E>(pos)?;
        for &candidate in moves.as_slice() {
            if is_pawn_move(pos, candidate) && !is_capture(pos, candidate) {
                if let Some(next) = successor::<E>(pos, candidate)? {
                    if -probe_wdl::<E>(owner, &next)?.score == wdl.score {
                        return Ok(base);
                    }
                }
            }
        }
        positive_moves = Some(moves);
    }
    if let Some(dtz) = table_dtz(owner, pos, wdl.score)? {
        return Ok(base + if wdl.score > 0 { dtz } else { -dtz });
    }
    let mut best = if wdl.score > 0 { i32::MAX } else { base };
    let moves = match positive_moves {
        Some(moves) => moves,
        None => generate_moves::<E>(pos).map_err(|_| ProbeError)?,
    };
    for &candidate in moves.as_slice() {
        if !is_capture(pos, candidate) && !is_pawn_move(pos, candidate) {
            if let Some(next) = successor::<E>(pos, candidate)? {
                let value = -probe_dtz_inner::<E>(owner, &next, depth + 1)?;
                if value == 1 && is_mate::<E>(&next)? {
                    best = 1;
                } else if wdl.score > 0 {
                    if value > 0 {
                        best = best.min(value + 1);
                    }
                } else {
                    best = best.min(value - 1);
                }
            }
        }
    }
    Ok(best)
}

pub(crate) fn probe_wdl_public<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
) -> Result<u32, ProbeError> {
    let score = probe_wdl::<E>(owner, pos)?.score;
    u32::try_from(score + 2).map_err(|_| ProbeError)
}

pub(crate) fn probe_dtz_public<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
) -> Result<i32, ProbeError> {
    probe_dtz_inner::<E>(owner, pos, 0)
}

fn dtz_to_wdl(clock: u8, dtz: i32) -> u32 {
    let clock = i32::from(clock);
    let score = if dtz > 0 {
        if dtz + clock <= 100 {
            2
        } else {
            1
        }
    } else if dtz < 0 {
        if -dtz + clock <= 100 {
            -2
        } else {
            -1
        }
    } else {
        0
    };
    (score + 2) as u32
}

fn pack_move(pos: &ValidatedPosition, candidate: PyrrhicMove, dtz: i32) -> u32 {
    dtz_to_wdl(pos.rule50, dtz)
        | (pyrrhic_move_from(candidate) << 10)
        | (pyrrhic_move_to(candidate) << 4)
        | (pyrrhic_move_promotes(candidate) << 16)
        | (u32::from(is_en_passant(pos, candidate)) << 19)
        | (dtz.unsigned_abs() << 20)
}

pub(crate) fn probe_root_public<E: EngineAdapter>(
    owner: &Generation,
    pos: &ValidatedPosition,
) -> Result<RootValue, ProbeError> {
    let dtz = probe_dtz_inner::<E>(owner, pos, 0)?;
    let moves = generate_moves::<E>(pos).map_err(|_| ProbeError)?;
    let mut scores = [i16::MAX; 256];
    let mut packed = [0u32; 256];
    let mut packed_len = 0;
    let mut draw_count = 0u64;
    for (index, &candidate) in moves.as_slice().iter().enumerate() {
        let Some(next) = successor::<E>(pos, candidate)? else {
            continue;
        };
        let value = if dtz > 0 && is_mate::<E>(&next)? {
            1
        } else if next.rule50 != 0 {
            let mut value = -probe_dtz_inner::<E>(owner, &next, 0)?;
            if value > 0 {
                value += 1;
            } else if value < 0 {
                value -= 1;
            }
            value
        } else {
            wdl_to_dtz(-probe_wdl::<E>(owner, &next)?.score)?
        };
        if value == 0 {
            draw_count += 1;
        }
        scores[index] = value as i16;
        packed[packed_len] = pack_move(pos, candidate, value);
        packed_len += 1;
    }
    let selected = if dtz != 0 {
        let mut best = if dtz > 0 { i16::MAX } else { 0 };
        let mut chosen = None;
        for (&candidate, &score) in moves.as_slice().iter().zip(scores.iter()) {
            if score != i16::MAX
                && ((dtz > 0 && score > 0 && score < best) || (dtz < 0 && score < best))
            {
                best = score;
                chosen = Some(candidate);
            }
        }
        chosen
    } else if draw_count > 0 {
        let mut draw_index = material_key(pos, !pos.turn) % draw_count;
        let mut chosen = None;
        for (&candidate, &score) in moves.as_slice().iter().zip(scores.iter()) {
            if score == 0 {
                if draw_index == 0 {
                    chosen = Some(candidate);
                    break;
                }
                draw_index -= 1;
            }
        }
        chosen
    } else {
        None
    };
    let code = match selected {
        Some(candidate) => pack_move(pos, candidate, dtz),
        None if dtz < 0 => 4,
        None if dtz == 0 => 2,
        None => return Err(ProbeError),
    };
    Ok(RootValue {
        code,
        moves: packed,
        len: packed_len,
    })
}
