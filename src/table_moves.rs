//! Bounded pseudo-legal tablebase move generation. Move order matches the
//! original probe because root tie selection depends on it.

use crate::{
    engine_adapter::{Color, EngineAdapter},
    tbprobe::{
        pyrrhic_make_move, PyrrhicMove, PyrrhicPosition, PYRRHIC_PROMOSQS, PYRRHIC_PROMOTES_BISHOP,
        PYRRHIC_PROMOTES_KNIGHT, PYRRHIC_PROMOTES_NONE, PYRRHIC_PROMOTES_QUEEN,
        PYRRHIC_PROMOTES_ROOK,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MoveError;

#[derive(Debug)]
pub(crate) struct MoveList<const N: usize> {
    moves: [PyrrhicMove; N],
    len: usize,
}

impl<const N: usize> MoveList<N> {
    pub(crate) fn new() -> Self {
        Self {
            moves: [0; N],
            len: 0,
        }
    }

    pub(crate) fn as_slice(&self) -> &[PyrrhicMove] {
        &self.moves[..self.len]
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn push(&mut self, value: PyrrhicMove) -> Result<(), MoveError> {
        let Some(destination) = self.moves.get_mut(self.len) else {
            return Err(MoveError);
        };
        *destination = value;
        self.len += 1;
        Ok(())
    }

    fn add(&mut self, promotes: bool, from: u32, to: u32) -> Result<(), MoveError> {
        if from >= 64 || to >= 64 {
            return Err(MoveError);
        }
        if promotes {
            for promotion in [
                PYRRHIC_PROMOTES_QUEEN,
                PYRRHIC_PROMOTES_KNIGHT,
                PYRRHIC_PROMOTES_ROOK,
                PYRRHIC_PROMOTES_BISHOP,
            ] {
                self.push(pyrrhic_make_move(promotion, from, to))?;
            }
        } else {
            self.push(pyrrhic_make_move(PYRRHIC_PROMOTES_NONE, from, to))?;
        }
        Ok(())
    }
}

fn pop_square(bits: &mut u64) -> Option<u32> {
    if *bits == 0 {
        return None;
    }
    let square = bits.trailing_zeros();
    *bits &= *bits - 1;
    Some(square)
}

fn add_piece_attacks<const N: usize>(
    mut sources: u64,
    targets: u64,
    occupied: u64,
    attack: impl Fn(u64, u64) -> u64,
    list: &mut MoveList<N>,
) -> Result<(), MoveError> {
    while let Some(from) = pop_square(&mut sources) {
        let mut destinations = attack(u64::from(from), occupied) & targets;
        while let Some(to) = pop_square(&mut destinations) {
            list.add(false, from, to)?;
        }
    }
    Ok(())
}

fn add_nonpawn_attacks<E: EngineAdapter, const N: usize>(
    position: &PyrrhicPosition,
    targets: u64,
    list: &mut MoveList<N>,
) -> Result<(), MoveError> {
    let us = if position.turn {
        position.white
    } else {
        position.black
    };
    let occupied = position.white | position.black;
    add_piece_attacks(
        us & position.kings,
        targets,
        occupied,
        |square, _| E::king_attacks(square),
        list,
    )?;
    add_piece_attacks(
        us & (position.rooks | position.queens),
        targets,
        occupied,
        E::rook_attacks,
        list,
    )?;
    add_piece_attacks(
        us & (position.bishops | position.queens),
        targets,
        occupied,
        E::bishop_attacks,
        list,
    )?;
    add_piece_attacks(
        us & position.knights,
        targets,
        occupied,
        |square, _| E::knight_attacks(square),
        list,
    )?;
    Ok(())
}

fn add_pawn_moves<E: EngineAdapter, const N: usize>(
    position: &PyrrhicPosition,
    captures_only: bool,
    list: &mut MoveList<N>,
) -> Result<(), MoveError> {
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
    let occupied = us | them;
    let enemy_without_king = them & !position.kings;
    let color = if position.turn {
        Color::White
    } else {
        Color::Black
    };
    let forward = if position.turn { 8 } else { -8 };
    if position.ep >= 64 {
        return Err(MoveError);
    }
    let mut pawns = us & position.pawns;
    while let Some(from) = pop_square(&mut pawns) {
        let attacks = E::pawn_attacks(color, u64::from(from));
        if position.ep != 0 && attacks & (1u64 << position.ep) != 0 {
            list.add(false, from, u32::from(position.ep))?;
        }
        if !captures_only {
            let one = (from as i32).checked_add(forward).ok_or(MoveError)?;
            let one = u32::try_from(one)
                .ok()
                .filter(|&square| square < 64)
                .ok_or(MoveError)?;
            if occupied & (1u64 << one) == 0 {
                list.add(PYRRHIC_PROMOSQS & (1u64 << one) != 0, from, one)?;
                let starting_rank = if position.turn { 1 } else { 6 };
                if from / 8 == starting_rank {
                    let two = (from as i32)
                        .checked_add(2 * forward)
                        .and_then(|square| u32::try_from(square).ok())
                        .filter(|&square| square < 64)
                        .ok_or(MoveError)?;
                    if occupied & (1u64 << two) == 0 {
                        list.add(false, from, two)?;
                    }
                }
            }
        }
        let mut targets = attacks & enemy_without_king;
        while let Some(to) = pop_square(&mut targets) {
            list.add(PYRRHIC_PROMOSQS & (1u64 << to) != 0, from, to)?;
        }
    }
    Ok(())
}

pub(crate) fn generate_captures<E: EngineAdapter>(
    position: &PyrrhicPosition,
) -> Result<MoveList<64>, MoveError> {
    let them = if position.turn {
        position.black
    } else {
        position.white
    };
    let mut list = MoveList::new();
    add_nonpawn_attacks::<E, 64>(position, them & !position.kings, &mut list)?;
    add_pawn_moves::<E, 64>(position, true, &mut list)?;
    Ok(list)
}

pub(crate) fn generate_moves<E: EngineAdapter>(
    position: &PyrrhicPosition,
) -> Result<MoveList<256>, MoveError> {
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
    let mut list = MoveList::new();
    add_nonpawn_attacks::<E, 256>(position, !us & !(them & position.kings), &mut list)?;
    add_pawn_moves::<E, 256>(position, false, &mut list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::{generate_captures, generate_moves, MoveError, MoveList};
    use crate::{
        engine_adapter::{Color, EngineAdapter},
        tbprobe::{pyrrhic_move_to, PyrrhicPosition},
    };

    #[derive(Clone)]
    struct Everywhere;

    impl EngineAdapter for Everywhere {
        fn pawn_attacks(_: Color, _: u64) -> u64 {
            0
        }
        fn knight_attacks(_: u64) -> u64 {
            u64::MAX
        }
        fn bishop_attacks(_: u64, _: u64) -> u64 {
            u64::MAX
        }
        fn rook_attacks(_: u64, _: u64) -> u64 {
            u64::MAX
        }
        fn queen_attacks(_: u64, _: u64) -> u64 {
            u64::MAX
        }
        fn king_attacks(_: u64) -> u64 {
            u64::MAX
        }
    }

    fn position(queens: u64) -> PyrrhicPosition {
        PyrrhicPosition {
            white: queens | 1,
            black: 1 << 63,
            kings: (1 << 63) | 1,
            queens,
            rooks: 0,
            bishops: 0,
            knights: 0,
            pawns: 0,
            rule50: 0,
            ep: 0,
            turn: true,
        }
    }

    #[test]
    fn a_callback_cannot_add_king_captures_or_overrun_the_move_buffer() {
        // This checks the private move-buffer contract for unusual adapter
        // masks, which a TSV root-move fixture cannot express.
        let simple = position(1 << 1);
        assert!(generate_captures::<Everywhere>(&simple)
            .unwrap()
            .as_slice()
            .is_empty());
        assert!(generate_moves::<Everywhere>(&simple)
            .unwrap()
            .as_slice()
            .iter()
            .all(|&value| pyrrhic_move_to(value) != 63));

        let crowded = position(0b1111110);
        assert!(matches!(
            generate_moves::<Everywhere>(&crowded),
            Err(MoveError)
        ));
        let mut short: MoveList<1> = MoveList::new();
        short.push(1).unwrap();
        assert_eq!(short.push(2), Err(MoveError));
    }
}
