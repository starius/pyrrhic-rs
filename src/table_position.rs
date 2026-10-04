#![forbid(unsafe_code)]
//! Value-based tablebase position transitions and king safety checks.

use crate::{
    engine_adapter::{Color, EngineAdapter},
    tbprobe::{
        pyrrhic_move_from, pyrrhic_move_promotes, pyrrhic_move_to, PyrrhicMove, PyrrhicPosition,
    },
};

/// A position admitted by the public bitboard validation or produced by a
/// checked move. Recursive probing passes this value instead of revalidating
/// every bitboard at each node.
#[derive(Clone, Copy)]
pub(crate) struct ValidatedPosition(PyrrhicPosition);

impl ValidatedPosition {
    pub(crate) fn from_public_checked(position: PyrrhicPosition) -> Self {
        Self(position)
    }
}

impl std::ops::Deref for ValidatedPosition {
    type Target = PyrrhicPosition;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PositionError;

fn bit(square: u32) -> Result<u64, PositionError> {
    1u64.checked_shl(square).ok_or(PositionError)
}

fn moved(bits: u64, from: u64, to: u64) -> u64 {
    (bits & !(from | to)) | if bits & from != 0 { to } else { 0 }
}

pub(crate) fn is_pawn_move(position: &PyrrhicPosition, candidate: PyrrhicMove) -> bool {
    let us = if position.turn {
        position.white
    } else {
        position.black
    };
    let from = pyrrhic_move_from(candidate);
    us & position.pawns & (1u64 << from) != 0
}

pub(crate) fn is_en_passant(position: &PyrrhicPosition, candidate: PyrrhicMove) -> bool {
    position.ep != 0
        && pyrrhic_move_to(candidate) == u32::from(position.ep)
        && is_pawn_move(position, candidate)
}

pub(crate) fn is_capture(position: &PyrrhicPosition, candidate: PyrrhicMove) -> bool {
    let them = if position.turn {
        position.black
    } else {
        position.white
    };
    them & (1u64 << pyrrhic_move_to(candidate)) != 0 || is_en_passant(position, candidate)
}

fn king_square(bits: u64) -> Result<u64, PositionError> {
    if bits.count_ones() != 1 {
        return Err(PositionError);
    }
    Ok(u64::from(bits.trailing_zeros()))
}

pub(crate) fn moved_side_is_legal<E: EngineAdapter>(
    position: &PyrrhicPosition,
) -> Result<bool, PositionError> {
    let moved_side = if position.turn {
        position.black
    } else {
        position.white
    };
    let opponent = if position.turn {
        position.white
    } else {
        position.black
    };
    let square = king_square(position.kings & moved_side)?;
    let occupied = moved_side | opponent;
    let pawn_direction = if position.turn {
        Color::Black
    } else {
        Color::White
    };
    Ok(E::king_attacks(square) & position.kings & opponent == 0
        && E::rook_attacks(square, occupied) & (position.rooks | position.queens) & opponent == 0
        && E::bishop_attacks(square, occupied) & (position.bishops | position.queens) & opponent
            == 0
        && E::knight_attacks(square) & position.knights & opponent == 0
        && E::pawn_attacks(pawn_direction, square) & position.pawns & opponent == 0)
}

pub(crate) fn side_to_move_is_in_check<E: EngineAdapter>(
    position: &PyrrhicPosition,
) -> Result<bool, PositionError> {
    let us = if position.turn {
        position.white
    } else {
        position.black
    };
    let them = if position.turn {
        position.black
    } else {
        position.white
    };
    let square = king_square(position.kings & us)?;
    let occupied = us | them;
    let pawn_direction = if position.turn {
        Color::White
    } else {
        Color::Black
    };
    Ok(
        E::rook_attacks(square, occupied) & (position.rooks | position.queens) & them != 0
            || E::bishop_attacks(square, occupied) & (position.bishops | position.queens) & them
                != 0
            || E::knight_attacks(square) & position.knights & them != 0
            || E::pawn_attacks(pawn_direction, square) & position.pawns & them != 0,
    )
}

fn ep_after_double_push<E: EngineAdapter>(
    from: u32,
    to: u32,
    white_moved: bool,
    opposing_pawns: u64,
) -> u8 {
    if from ^ to != 16 {
        return 0;
    }
    let ep = if white_moved {
        from.checked_add(8)
    } else {
        from.checked_sub(8)
    };
    let Some(ep) = ep.filter(|&square| square < 64) else {
        return 0;
    };
    let mover = if white_moved {
        Color::White
    } else {
        Color::Black
    };
    if E::pawn_attacks(mover, u64::from(ep)) & opposing_pawns != 0 {
        ep as u8
    } else {
        0
    }
}

pub(crate) fn apply_move<E: EngineAdapter>(
    position: &ValidatedPosition,
    candidate: PyrrhicMove,
) -> Result<Option<ValidatedPosition>, PositionError> {
    let from = pyrrhic_move_from(candidate);
    let to = pyrrhic_move_to(candidate);
    let from_bit = bit(from)?;
    let to_bit = bit(to)?;
    let us = if position.turn {
        position.white
    } else {
        position.black
    };
    let them = if position.turn {
        position.black
    } else {
        position.white
    };
    if from == to || us & from_bit == 0 || us & to_bit != 0 || them & position.kings & to_bit != 0 {
        return Err(PositionError);
    }
    let pawn = position.pawns & from_bit != 0;
    let promotion = pyrrhic_move_promotes(candidate);
    if promotion != 0
        && (!pawn || !matches!(promotion, 1..=4) || to / 8 != if position.turn { 7 } else { 0 })
    {
        return Err(PositionError);
    }
    if pawn && promotion == 0 && to / 8 == if position.turn { 7 } else { 0 } {
        return Err(PositionError);
    }
    let ep_capture = pawn && position.ep != 0 && to == u32::from(position.ep);
    let ep_captured_bit = if ep_capture {
        if them & to_bit != 0 {
            return Err(PositionError);
        }
        let captured = if position.turn {
            to.checked_sub(8)
        } else {
            to.checked_add(8)
        }
        .filter(|&square| square < 64)
        .ok_or(PositionError)?;
        let captured_bit = bit(captured)?;
        if them & position.pawns & captured_bit == 0 {
            return Err(PositionError);
        }
        captured_bit
    } else {
        0
    };

    let mut next = PyrrhicPosition {
        turn: !position.turn,
        white: moved(position.white, from_bit, to_bit),
        black: moved(position.black, from_bit, to_bit),
        kings: moved(position.kings, from_bit, to_bit),
        queens: moved(position.queens, from_bit, to_bit),
        rooks: moved(position.rooks, from_bit, to_bit),
        bishops: moved(position.bishops, from_bit, to_bit),
        knights: moved(position.knights, from_bit, to_bit),
        pawns: moved(position.pawns, from_bit, to_bit),
        rule50: position.rule50,
        ep: 0,
    };
    if promotion != 0 {
        next.pawns &= !to_bit;
        match promotion {
            1 => next.queens |= to_bit,
            2 => next.rooks |= to_bit,
            3 => next.bishops |= to_bit,
            4 => next.knights |= to_bit,
            _ => return Err(PositionError),
        }
        next.rule50 = 0;
    } else if pawn {
        next.rule50 = 0;
        next.ep = ep_after_double_push::<E>(from, to, position.turn, them & position.pawns);
        if ep_capture {
            next.white &= !ep_captured_bit;
            next.black &= !ep_captured_bit;
            next.pawns &= !ep_captured_bit;
        }
    } else if them & to_bit != 0 {
        next.rule50 = 0;
    } else {
        next.rule50 = position.rule50.wrapping_add(1);
    }
    moved_side_is_legal::<E>(&next).map(|legal| legal.then_some(ValidatedPosition(next)))
}

#[cfg(test)]
mod tests {
    use super::{apply_move, PositionError, ValidatedPosition};
    use crate::{
        engine_adapter::{Color, EngineAdapter},
        tbprobe::{pyrrhic_make_move, PyrrhicPosition},
    };

    #[derive(Clone)]
    struct Cozy;

    impl EngineAdapter for Cozy {
        fn pawn_attacks(color: Color, square: u64) -> u64 {
            cozy_chess::get_pawn_attacks(
                cozy_chess::Square::index(square as usize),
                if color == Color::White {
                    cozy_chess::Color::White
                } else {
                    cozy_chess::Color::Black
                },
            )
            .0
        }
        fn knight_attacks(square: u64) -> u64 {
            cozy_chess::get_knight_moves(cozy_chess::Square::index(square as usize)).0
        }
        fn bishop_attacks(square: u64, occupied: u64) -> u64 {
            cozy_chess::get_bishop_moves(
                cozy_chess::Square::index(square as usize),
                cozy_chess::BitBoard(occupied),
            )
            .0
        }
        fn rook_attacks(square: u64, occupied: u64) -> u64 {
            cozy_chess::get_rook_moves(
                cozy_chess::Square::index(square as usize),
                cozy_chess::BitBoard(occupied),
            )
            .0
        }
        fn queen_attacks(square: u64, occupied: u64) -> u64 {
            Self::rook_attacks(square, occupied) | Self::bishop_attacks(square, occupied)
        }
        fn king_attacks(square: u64) -> u64 {
            cozy_chess::get_king_moves(cozy_chess::Square::index(square as usize)).0
        }
    }

    #[test]
    fn double_push_and_en_passant_preserve_the_piece_population() {
        // This observes the private EP transition and piece masks, which a
        // root-move TSV fixture does not expose.
        let position = ValidatedPosition::from_public_checked(PyrrhicPosition {
            white: (1 << 4) | (1 << 12),
            black: (1 << 60) | (1 << 27),
            kings: (1 << 4) | (1 << 60),
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: (1 << 12) | (1 << 27),
            rule50: 19,
            ep: 0,
            turn: true,
        });
        let pushed = apply_move::<Cozy>(&position, pyrrhic_make_move(0, 12, 28))
            .unwrap()
            .unwrap();
        assert_eq!(pushed.ep, 20);
        assert_eq!(pushed.rule50, 0);
        let captured = apply_move::<Cozy>(&pushed, pyrrhic_make_move(0, 27, 20))
            .unwrap()
            .unwrap();
        assert_eq!(captured.pawns.count_ones(), 1);
        assert_eq!(captured.white & (1 << 28), 0);
        assert_eq!(captured.black & (1 << 20), 1 << 20);
    }

    #[test]
    fn king_capture_and_missing_mover_are_rejected() {
        let position = ValidatedPosition::from_public_checked(PyrrhicPosition {
            white: (1 << 4) | (1 << 46),
            black: 1 << 63,
            kings: (1 << 4) | (1 << 63),
            queens: 1 << 46,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: true,
        });
        assert!(matches!(
            apply_move::<Cozy>(&position, pyrrhic_make_move(0, 46, 63)),
            Err(PositionError)
        ));
        assert!(matches!(
            apply_move::<Cozy>(&position, pyrrhic_make_move(0, 30, 31)),
            Err(PositionError)
        ));
    }

    #[test]
    fn promotion_requires_a_pawn_and_an_explicit_piece() {
        // This checks the private move encoding contract, not a root choice.
        let position = ValidatedPosition::from_public_checked(PyrrhicPosition {
            white: (1 << 4) | (1 << 48),
            black: 1 << 60,
            kings: (1 << 4) | (1 << 60),
            queens: 0,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 1 << 48,
            rule50: 0,
            ep: 0,
            turn: true,
        });
        assert!(matches!(
            apply_move::<Cozy>(&position, pyrrhic_make_move(0, 48, 56)),
            Err(PositionError)
        ));
        assert!(matches!(
            apply_move::<Cozy>(&position, pyrrhic_make_move(5, 48, 56)),
            Err(PositionError)
        ));
        assert!(apply_move::<Cozy>(&position, pyrrhic_make_move(1, 48, 56))
            .unwrap()
            .is_some());
    }
}
