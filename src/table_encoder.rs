#![forbid(unsafe_code)]
//! Bounded position-to-table indexing. The input describes one validated
//! tablebase position and uses fixed stack storage throughout a warm probe.

use crate::{
    table_parser::{Description, Encoding},
    tbprobe::{
        PyrrhicPosition, DIAG, FILE_TO_FILE, FLAP, FLIP_DIAG, INDICES, KK_IDX, LOWER, OFF_DIAG,
        PAWN_TWIST, TRIANGLE,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EncodeError;

fn piece_bitboard(position: &PyrrhicPosition, piece: u8, flip: bool) -> Result<u64, EncodeError> {
    let white = (piece & 8 == 0) != flip;
    let side = if white {
        position.white
    } else {
        position.black
    };
    let typed = match piece & 7 {
        1 => position.pawns,
        2 => position.knights,
        3 => position.bishops,
        4 => position.rooks,
        5 => position.queens,
        6 => position.kings,
        _ => return Err(EncodeError),
    };
    Ok(side & typed)
}

pub(crate) fn fill_squares(
    position: &PyrrhicPosition,
    encoding: &Encoding,
    description: Description,
    flip: bool,
    mirror: u8,
    squares: &mut [u8; 7],
    start: usize,
) -> Result<usize, EncodeError> {
    if start >= description.pieces || description.pieces > squares.len() {
        return Err(EncodeError);
    }
    let mut pieces = piece_bitboard(position, encoding.pieces[start], flip)?;
    if pieces == 0 || pieces.count_ones() as usize > description.pieces - start {
        return Err(EncodeError);
    }
    let mut next = start;
    while pieces != 0 {
        let square = pieces.trailing_zeros() as u8;
        pieces &= pieces - 1;
        squares[next] = square ^ mirror;
        next += 1;
    }
    Ok(next)
}

pub(crate) fn leading_pawn(squares: &mut [u8; 7], primary: usize) -> Result<usize, EncodeError> {
    if primary == 0 || primary > squares.len() {
        return Err(EncodeError);
    }
    for index in 1..primary {
        if FLAP[usize::from(squares[0])] > FLAP[usize::from(squares[index])] {
            squares.swap(0, index);
        }
    }
    Ok(usize::from(FILE_TO_FILE[usize::from(squares[0] & 7)]))
}

fn add_factor(index: u64, value: u64, factor: u64) -> Result<u64, EncodeError> {
    index
        .checked_add(value.checked_mul(factor).ok_or(EncodeError)?)
        .ok_or(EncodeError)
}

fn choose_index(row: usize, column: usize) -> Result<u64, EncodeError> {
    if row >= INDICES.binomial.len() || column >= 64 {
        return Err(EncodeError);
    }
    Ok(INDICES.binomial[row][column])
}

pub(crate) fn encode_squares(
    squares: &mut [u8; 7],
    encoding: &Encoding,
    description: Description,
) -> Result<u64, EncodeError> {
    let n = description.pieces;
    if !(3..=7).contains(&n) {
        return Err(EncodeError);
    }
    let mut occupied = 0u64;
    for &square in &squares[..n] {
        if square >= 64 || occupied & (1u64 << square) != 0 {
            return Err(EncodeError);
        }
        occupied |= 1u64 << square;
    }
    if squares[0] & 4 != 0 {
        for square in &mut squares[..n] {
            *square ^= 7;
        }
    }

    let mut group_start;
    let mut index;
    if description.primary_pawns == 0 {
        if squares[0] & 32 != 0 {
            for square in &mut squares[..n] {
                *square ^= 0x38;
            }
        }
        let special = if description.king_pair { 2 } else { 3 };
        for first in 0..n {
            let direction = OFF_DIAG[usize::from(squares[first])];
            if direction != 0 {
                if direction > 0 && first < special {
                    for square in &mut squares[..n] {
                        *square = FLIP_DIAG[usize::from(*square)];
                    }
                }
                break;
            }
        }
        if description.king_pair {
            let king_index =
                KK_IDX[usize::from(TRIANGLE[usize::from(squares[0])])][usize::from(squares[1])];
            index = u64::try_from(king_index).map_err(|_| EncodeError)?;
            group_start = 2;
        } else {
            let p0 = i32::from(squares[0]);
            let p1 = i32::from(squares[1]);
            let p2 = i32::from(squares[2]);
            let first_skip = i32::from(p1 > p0);
            let second_skip = i32::from(p2 > p0) + i32::from(p2 > p1);
            let value = if OFF_DIAG[p0 as usize] != 0 {
                i32::from(TRIANGLE[p0 as usize]) * 63 * 62 + (p1 - first_skip) * 62 + p2
                    - second_skip
            } else if OFF_DIAG[p1 as usize] != 0 {
                6 * 63 * 62
                    + i32::from(DIAG[p0 as usize]) * 28 * 62
                    + i32::from(LOWER[p1 as usize]) * 62
                    + p2
                    - second_skip
            } else if OFF_DIAG[p2 as usize] != 0 {
                6 * 63 * 62
                    + 4 * 28 * 62
                    + i32::from(DIAG[p0 as usize]) * 7 * 28
                    + (i32::from(DIAG[p1 as usize]) - first_skip) * 28
                    + i32::from(LOWER[p2 as usize])
            } else {
                6 * 63 * 62
                    + 4 * 28 * 62
                    + 4 * 7 * 28
                    + i32::from(DIAG[p0 as usize]) * 7 * 6
                    + (i32::from(DIAG[p1 as usize]) - first_skip) * 6
                    + i32::from(DIAG[p2 as usize])
                    - second_skip
            };
            index = u64::try_from(value).map_err(|_| EncodeError)?;
            group_start = 3;
        }
        index = index.checked_mul(encoding.factor[0]).ok_or(EncodeError)?;
    } else {
        let primary = description.primary_pawns;
        if primary == 0 || primary > 6 || primary > n {
            return Err(EncodeError);
        }
        squares[1..primary]
            .sort_unstable_by_key(|&square| std::cmp::Reverse(PAWN_TWIST[usize::from(square)]));
        group_start = primary;
        index = INDICES.pawn_idx[primary - 1][usize::from(FLAP[usize::from(squares[0])])];
        for i in 1..primary {
            index = index
                .checked_add(choose_index(
                    primary - i,
                    usize::from(PAWN_TWIST[usize::from(squares[i])]),
                )?)
                .ok_or(EncodeError)?;
        }
        index = index.checked_mul(encoding.factor[0]).ok_or(EncodeError)?;
        if description.secondary_pawns > 0 {
            let end = primary
                .checked_add(description.secondary_pawns)
                .filter(|&end| end <= n)
                .ok_or(EncodeError)?;
            squares[primary..end].sort_unstable();
            let mut value = 0u64;
            for i in primary..end {
                let square = i32::from(squares[i]);
                let skips = squares[..primary]
                    .iter()
                    .filter(|&&previous| square > i32::from(previous))
                    .count() as i32;
                let column = usize::try_from(square - skips - 8).map_err(|_| EncodeError)?;
                value = value
                    .checked_add(choose_index(i - primary + 1, column)?)
                    .ok_or(EncodeError)?;
            }
            index = add_factor(index, value, encoding.factor[primary])?;
            group_start = end;
        }
    }

    while group_start < n {
        let count = usize::from(encoding.norm[group_start]);
        let end = group_start
            .checked_add(count)
            .filter(|&end| count > 0 && end <= n)
            .ok_or(EncodeError)?;
        squares[group_start..end].sort_unstable();
        let mut value = 0u64;
        for i in group_start..end {
            let square = i32::from(squares[i]);
            let skips = squares[..group_start]
                .iter()
                .filter(|&&previous| square > i32::from(previous))
                .count() as i32;
            let column = usize::try_from(square - skips).map_err(|_| EncodeError)?;
            value = value
                .checked_add(choose_index(i - group_start + 1, column)?)
                .ok_or(EncodeError)?;
        }
        index = add_factor(index, value, encoding.factor[group_start])?;
        group_start = end;
    }
    if index >= encoding.size {
        return Err(EncodeError);
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::{encode_squares, fill_squares, leading_pawn, EncodeError};
    use crate::{table_parser::Description, tbprobe::PyrrhicPosition};

    #[test]
    fn invalid_piece_population_and_duplicate_squares_fail_without_writes() {
        let description = Description {
            pieces: 3,
            primary_pawns: 0,
            secondary_pawns: 0,
            king_pair: false,
        };
        let encoding = crate::table_parser::Encoding {
            pieces: [14, 6, 5, 0, 0, 0, 0],
            norm: [3, 0, 0, 0, 0, 0, 0],
            factor: [1, 0, 0, 0, 0, 0, 0],
            size: 31_332,
        };
        let position = PyrrhicPosition {
            white: 1 << 4,
            black: 1 << 63,
            kings: (1 << 4) | (1 << 63),
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: true,
        };
        let mut squares = [0; 7];
        assert_eq!(
            fill_squares(&position, &encoding, description, false, 0, &mut squares, 2),
            Err(EncodeError)
        );
        squares[..3].copy_from_slice(&[63, 4, 4]);
        assert_eq!(
            encode_squares(&mut squares, &encoding, description),
            Err(EncodeError)
        );
        assert_eq!(leading_pawn(&mut squares, 0), Err(EncodeError));
    }
}
