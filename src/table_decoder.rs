#![forbid(unsafe_code)]
//! Safe, allocation-free decoding of one validated Syzygy pair value.

use crate::table_parser::{PairHeader, ParsedPair};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DecodeError;

fn le_u16(bytes: &[u8], offset: usize) -> Result<u16, DecodeError> {
    let end = offset.checked_add(2).ok_or(DecodeError)?;
    let value: [u8; 2] = bytes
        .get(offset..end)
        .ok_or(DecodeError)?
        .try_into()
        .unwrap();
    Ok(u16::from_le_bytes(value))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32, DecodeError> {
    let end = offset.checked_add(4).ok_or(DecodeError)?;
    let value: [u8; 4] = bytes
        .get(offset..end)
        .ok_or(DecodeError)?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(value))
}

fn padded_block_word(bytes: &[u8], offset: usize, block_end: usize) -> Result<u32, DecodeError> {
    if offset > block_end || block_end > bytes.len() {
        return Err(DecodeError);
    }
    let available = block_end.saturating_sub(offset).min(4);
    let mut word = [0u8; 4];
    word[..available].copy_from_slice(bytes.get(offset..offset + available).ok_or(DecodeError)?);
    Ok(u32::from_be_bytes(word))
}

fn be_u64(bytes: &[u8], offset: usize) -> Result<u64, DecodeError> {
    let end = offset.checked_add(8).ok_or(DecodeError)?;
    let value: [u8; 8] = bytes
        .get(offset..end)
        .ok_or(DecodeError)?
        .try_into()
        .unwrap();
    Ok(u64::from_be_bytes(value))
}

fn children(pattern: [u8; 3]) -> (usize, usize) {
    (
        usize::from(pattern[0]) | (usize::from(pattern[1] & 0x0f) << 8),
        usize::from(pattern[2]) << 4 | usize::from(pattern[1] >> 4),
    )
}

pub(crate) fn decode_pair(
    bytes: &[u8],
    pair: &ParsedPair,
    index: u64,
) -> Result<[u8; 2], DecodeError> {
    let PairHeader::Compressed {
        block_size,
        index_bits,
        real_blocks,
        total_blocks,
        min_len,
        offsets,
        bases,
        symbol_lengths,
        patterns,
        ..
    } = &pair.header
    else {
        let PairHeader::Constant { value, .. } = pair.header else {
            unreachable!();
        };
        return Ok([value, 0]);
    };
    if !(1..=63).contains(index_bits)
        || !(3..=30).contains(block_size)
        || *min_len == 0
        || offsets.len() != bases.len()
        || offsets.is_empty()
        || usize::from(*min_len) + offsets.len() - 1 > 64
        || symbol_lengths.len() != patterns.len()
    {
        return Err(DecodeError);
    }

    let index_bytes = bytes.get(pair.index.clone()).ok_or(DecodeError)?;
    let size_bytes = bytes.get(pair.sizes.clone()).ok_or(DecodeError)?;
    let payload = bytes.get(pair.data.clone()).ok_or(DecodeError)?;
    let main_index = usize::try_from(index >> index_bits).map_err(|_| DecodeError)?;
    let record_offset = main_index.checked_mul(6).ok_or(DecodeError)?;
    let block = usize::try_from(le_u32(index_bytes, record_offset)?).map_err(|_| DecodeError)?;
    let literal_offset = i64::from(le_u16(
        index_bytes,
        record_offset.checked_add(4).ok_or(DecodeError)?,
    )?);
    let mask = (1u64 << index_bits) - 1;
    let centered = i64::try_from(index & mask).map_err(|_| DecodeError)?
        - i64::try_from(1u64 << (index_bits - 1)).map_err(|_| DecodeError)?;
    let mut literal = centered + literal_offset;
    let mut block = block;
    let block_size_at = |number: usize| -> Result<i64, DecodeError> {
        if number >= *total_blocks as usize {
            return Err(DecodeError);
        }
        Ok(i64::from(le_u16(
            size_bytes,
            number.checked_mul(2).ok_or(DecodeError)?,
        )?))
    };
    let mut steps = 0usize;
    let last_literal = loop {
        let last = block_size_at(block)?;
        if literal < 0 {
            block = block.checked_sub(1).ok_or(DecodeError)?;
            literal += block_size_at(block)? + 1;
        } else if literal > last {
            literal -= last + 1;
            block = block.checked_add(1).ok_or(DecodeError)?;
        } else {
            break last;
        }
        steps += 1;
        if steps > *total_blocks as usize {
            return Err(DecodeError);
        }
    };
    if block >= *real_blocks as usize {
        return Err(DecodeError);
    }
    let block_offset = block
        .checked_mul(
            1usize
                .checked_shl(u32::from(*block_size))
                .ok_or(DecodeError)?,
        )
        .ok_or(DecodeError)?;
    let block_end = block_offset
        .checked_add(
            1usize
                .checked_shl(u32::from(*block_size))
                .ok_or(DecodeError)?,
        )
        .ok_or(DecodeError)?;
    if block_end > payload.len() {
        return Err(DecodeError);
    }
    let mut code = be_u64(payload, block_offset)?;
    let mut stream_offset = block_offset.checked_add(8).ok_or(DecodeError)?;
    let mut bit_count = 0u32;
    let min_len = usize::from(*min_len);
    let mut symbol = None;
    for _ in 0..=last_literal {
        let mut length_index = 0;
        while code < *bases.get(length_index).ok_or(DecodeError)? {
            length_index += 1;
        }
        let length = min_len + length_index;
        let base = *bases.get(length_index).ok_or(DecodeError)?;
        let first = usize::from(*offsets.get(length_index).ok_or(DecodeError)?);
        let code_index =
            usize::try_from((code - base) >> (64 - length)).map_err(|_| DecodeError)?;
        let selected = first.checked_add(code_index).ok_or(DecodeError)?;
        let span = i64::from(*symbol_lengths.get(selected).ok_or(DecodeError)?) + 1;
        if literal < span {
            symbol = Some(selected);
            break;
        }
        literal -= span;
        if length == 64 {
            return Err(DecodeError);
        }
        code <<= length;
        bit_count += length as u32;
        while bit_count >= 32 {
            bit_count -= 32;
            // The last symbol can trigger one speculative refill at the end
            // of a compressed block. Its low bits are not consumed; fill
            // those bits with zero instead of reading the next file section.
            code |= u64::from(padded_block_word(payload, stream_offset, block_end)?)
                .checked_shl(bit_count)
                .ok_or(DecodeError)?;
            stream_offset = stream_offset.checked_add(4).ok_or(DecodeError)?;
        }
    }
    let mut symbol = symbol.ok_or(DecodeError)?;
    for _ in 0..=usize::from(symbol_lengths[symbol]) {
        if symbol_lengths[symbol] == 0 {
            if literal != 0 {
                return Err(DecodeError);
            }
            let pattern = *patterns.get(symbol).ok_or(DecodeError)?;
            return Ok([pattern[0], pattern[1]]);
        }
        let pattern = *patterns.get(symbol).ok_or(DecodeError)?;
        let (left, right) = children(pattern);
        let left_span = i64::from(*symbol_lengths.get(left).ok_or(DecodeError)?) + 1;
        if literal < left_span {
            symbol = left;
        } else {
            literal -= left_span;
            symbol = right;
        }
        if symbol >= patterns.len() {
            return Err(DecodeError);
        }
    }
    Err(DecodeError)
}

#[cfg(test)]
mod tests {
    use super::{decode_pair, padded_block_word, DecodeError};
    use crate::table_parser::{PairHeader, ParsedPair};

    #[test]
    fn constant_and_compressed_pairs_decode_without_raw_memory_access() {
        let constant = ParsedPair {
            header: PairHeader::Constant {
                flags: 0x80,
                value: 4,
            },
            index: 0..0,
            sizes: 0..0,
            data: 0..0,
        };
        assert_eq!(decode_pair(&[], &constant, 0), Ok([4, 0]));

        let compressed = ParsedPair {
            header: PairHeader::Compressed {
                flags: 0,
                block_size: 6,
                index_bits: 4,
                real_blocks: 1,
                total_blocks: 1,
                min_len: 1,
                offsets: Box::new([0]),
                bases: Box::new([0]),
                symbol_lengths: Box::new([0]),
                patterns: Box::new([[3, 0xf0, 0xff]]),
                index_bytes: 6,
                size_bytes: 2,
                data_bytes: 64,
            },
            index: 0..6,
            sizes: 6..8,
            data: 8..72,
        };
        let mut bytes = [0u8; 72];
        bytes[4] = 8;
        assert_eq!(decode_pair(&bytes, &compressed, 0), Ok([3, 0xf0]));
        assert_eq!(decode_pair(&bytes[..71], &compressed, 0), Err(DecodeError));
        bytes[0] = 1;
        assert_eq!(decode_pair(&bytes, &compressed, 0), Err(DecodeError));
        bytes[0] = 0;
        for position in 0..bytes.len() {
            for bit in 0..8 {
                let mut altered = bytes;
                altered[position] ^= 1 << bit;
                let _ = decode_pair(&altered, &compressed, 0);
            }
        }
    }

    #[test]
    fn speculative_refill_at_block_end_uses_zero_padding() {
        // A valid last-block symbol can request a refill whose bits are not
        // consumed. The private reader must stay inside that block.
        let bytes = [0xaa, 0xbb, 0xcc, 0xdd, 0x11, 0x22, 0x33, 0x44];
        assert_eq!(padded_block_word(&bytes, 2, 4), Ok(0xccdd0000));
        assert_eq!(padded_block_word(&bytes, 4, 4), Ok(0));
        assert_eq!(padded_block_word(&bytes, 5, 4), Err(DecodeError));
    }
}
