#![forbid(unsafe_code)]
//! Checked Syzygy metadata parsing. Decoding uses the same parsed factors and
//! piece order as the legacy probe while the remaining decoder is converted.

use std::ops::Range;

use crate::storage::{Cursor, ParseError};

#[derive(Clone, Copy)]
pub(crate) struct Description {
    pub(crate) pieces: usize,
    pub(crate) primary_pawns: usize,
    pub(crate) secondary_pawns: usize,
    pub(crate) king_pair: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Encoding {
    pub(crate) pieces: [u8; 7],
    pub(crate) norm: [u8; 7],
    pub(crate) factor: [u64; 7],
    pub(crate) size: u64,
}

#[derive(Clone)]
pub(crate) enum PairHeader {
    Constant {
        flags: u8,
        value: u8,
    },
    Compressed {
        flags: u8,
        block_size: u8,
        index_bits: u8,
        real_blocks: u32,
        total_blocks: u32,
        min_len: u8,
        offsets: Box<[u16]>,
        bases: Box<[u64]>,
        symbol_lengths: Box<[u8]>,
        patterns: Box<[[u8; 3]]>,
        index_bytes: usize,
        size_bytes: usize,
        data_bytes: usize,
    },
}

impl PairHeader {
    pub(crate) fn flags(&self) -> u8 {
        match self {
            Self::Constant { flags, .. } | Self::Compressed { flags, .. } => *flags,
        }
    }

    pub(crate) fn section_sizes(&self) -> [usize; 3] {
        match self {
            Self::Constant { .. } => [0; 3],
            Self::Compressed {
                index_bytes,
                size_bytes,
                data_bytes,
                ..
            } => [*index_bytes, *size_bytes, *data_bytes],
        }
    }
}

#[derive(Clone)]
pub(crate) struct ParsedPair {
    pub(crate) header: PairHeader,
    pub(crate) index: Range<usize>,
    pub(crate) sizes: Range<usize>,
    pub(crate) data: Range<usize>,
}

pub(crate) struct ParsedTable {
    pub(crate) encodings: Vec<Encoding>,
    pub(crate) pairs: Vec<Option<ParsedPair>>,
    pub(crate) map_ranges: Vec<[Range<usize>; 4]>,
}

fn take_range(cursor: &mut Cursor<'_>, len: usize) -> Result<Range<usize>, ParseError> {
    let start = cursor.position();
    cursor.take(len)?;
    Ok(start..cursor.position())
}

pub(crate) fn parse_table(
    bytes: &[u8],
    magic: u32,
    is_wdl: bool,
    description: Description,
    pawn_factor_file: &[[u64; 4]; 6],
) -> Result<ParsedTable, ParseError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.read_u32()? != magic {
        return Err(ParseError::InvalidFormat);
    }
    let flags = cursor.read_u8()?;
    let split = is_wdl && flags & 1 != 0;
    let tables = if description.primary_pawns > 0 { 4 } else { 1 };
    let mut encodings = vec![
        Encoding {
            pieces: [0; 7],
            norm: [0; 7],
            factor: [0; 7],
            size: 0,
        };
        tables * (1 + usize::from(split))
    ];
    for table in 0..tables {
        let header_len = description.pieces + 1 + usize::from(description.secondary_pawns > 0);
        let header = cursor.take(header_len)?;
        encodings[table] = parse_encoding(header, 0, table, description, pawn_factor_file)?;
        if split {
            encodings[tables + table] =
                parse_encoding(header, 4, table, description, pawn_factor_file)?;
        }
    }
    cursor.align(2)?;

    let mut headers = vec![None; encodings.len()];
    for table in 0..tables {
        headers[table] = Some(parse_pair_header(
            &mut cursor,
            encodings[table].size,
            is_wdl,
        )?);
        if split {
            headers[tables + table] = Some(parse_pair_header(
                &mut cursor,
                encodings[tables + table].size,
                is_wdl,
            )?);
        }
    }

    let mut map_ranges = vec![std::array::from_fn(|_| 0..0); tables];
    if !is_wdl {
        for table in 0..tables {
            let pair_flags = headers[table]
                .as_ref()
                .ok_or(ParseError::InvalidFormat)?
                .flags();
            if pair_flags & 2 == 0 {
                continue;
            }
            let wide = pair_flags & 16 != 0;
            if wide {
                cursor.align(2)?;
            }
            for range in &mut map_ranges[table] {
                if wide {
                    let count = usize::from(cursor.read_u16()?);
                    *range = take_range(
                        &mut cursor,
                        count.checked_mul(2).ok_or(ParseError::Overflow)?,
                    )?;
                } else {
                    let count = usize::from(cursor.read_u8()?);
                    *range = take_range(&mut cursor, count)?;
                }
            }
        }
        cursor.align(2)?;
    }

    let mut pairs: Vec<Option<ParsedPair>> = vec![None; encodings.len()];
    for table in 0..tables {
        for side in 0..(1 + usize::from(split)) {
            let index = table + side * tables;
            let header = headers[index].as_ref().ok_or(ParseError::InvalidFormat)?;
            let start = cursor.position();
            let range = take_range(&mut cursor, header.section_sizes()[0])?;
            pairs[index] = Some(ParsedPair {
                header: header.clone(),
                index: range,
                sizes: start..start,
                data: start..start,
            });
        }
    }
    for table in 0..tables {
        for side in 0..(1 + usize::from(split)) {
            let index = table + side * tables;
            let pair = pairs[index].as_mut().ok_or(ParseError::InvalidFormat)?;
            pair.sizes = take_range(&mut cursor, pair.header.section_sizes()[1])?;
        }
    }
    for table in 0..tables {
        for side in 0..(1 + usize::from(split)) {
            let index = table + side * tables;
            cursor.align(64)?;
            let pair = pairs[index].as_mut().ok_or(ParseError::InvalidFormat)?;
            pair.data = take_range(&mut cursor, pair.header.section_sizes()[2])?;
        }
    }
    for pair in pairs.iter().flatten() {
        if let PairHeader::Compressed { total_blocks, .. } = &pair.header {
            let records = cursor.section(pair.index.start, pair.index.len())?;
            for record in records.as_chunks::<6>().0 {
                let block = u32::from_le_bytes(record[..4].try_into().expect("checked chunk"));
                if block >= *total_blocks {
                    return Err(ParseError::InvalidFormat);
                }
            }
        }
    }
    Ok(ParsedTable {
        encodings,
        pairs,
        map_ranges,
    })
}

fn symbol_children(pattern: [u8; 3]) -> (usize, usize) {
    (
        usize::from(pattern[0]) | (usize::from(pattern[1] & 0x0f) << 8),
        usize::from(pattern[2]) << 4 | usize::from(pattern[1] >> 4),
    )
}

fn symbol_lengths(patterns: &[[u8; 3]]) -> Result<Box<[u8]>, ParseError> {
    let mut state = vec![0u8; patterns.len()];
    let mut lengths = vec![0u8; patterns.len()];
    for root in 0..patterns.len() {
        let mut stack = vec![(root, false)];
        while let Some((symbol, returning)) = stack.pop() {
            if returning {
                let (left, right) = symbol_children(patterns[symbol]);
                let length = u16::from(lengths[left]) + u16::from(lengths[right]) + 1;
                lengths[symbol] = u8::try_from(length).map_err(|_| ParseError::InvalidFormat)?;
                state[symbol] = 2;
            } else if state[symbol] == 0 {
                let (left, right) = symbol_children(patterns[symbol]);
                if right == 0x0fff {
                    state[symbol] = 2;
                } else {
                    if left >= patterns.len() || right >= patterns.len() {
                        return Err(ParseError::InvalidFormat);
                    }
                    state[symbol] = 1;
                    stack.push((symbol, true));
                    stack.push((right, false));
                    stack.push((left, false));
                }
            } else if state[symbol] == 1 {
                return Err(ParseError::InvalidFormat);
            }
        }
    }
    Ok(lengths.into_boxed_slice())
}

pub(crate) fn parse_pair_header(
    cursor: &mut Cursor<'_>,
    table_size: u64,
    is_wdl: bool,
) -> Result<PairHeader, ParseError> {
    let flags = cursor.read_u8()?;
    if flags & 0x80 != 0 {
        let value = cursor.read_u8()?;
        return Ok(PairHeader::Constant {
            flags,
            value: if is_wdl { value } else { 0 },
        });
    }
    let block_size = cursor.read_u8()?;
    let index_bits = cursor.read_u8()?;
    let extra_blocks = u32::from(cursor.read_u8()?);
    let real_blocks = cursor.read_u32()?;
    let max_len = cursor.read_u8()?;
    let min_len = cursor.read_u8()?;
    if !(3..=30).contains(&block_size)
        || !(1..=63).contains(&index_bits)
        || real_blocks == 0
        || min_len == 0
        || max_len < min_len
        || max_len > 64
        || table_size == 0
    {
        return Err(ParseError::InvalidFormat);
    }
    let count = usize::from(max_len - min_len) + 1;
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(cursor.read_u16()?);
    }
    let num_symbols = usize::from(cursor.read_u16()?);
    if !(1..4096).contains(&num_symbols)
        || offsets
            .iter()
            .any(|&offset| usize::from(offset) > num_symbols)
    {
        return Err(ParseError::InvalidFormat);
    }
    let pattern_bytes = cursor.take(num_symbols.checked_mul(3).ok_or(ParseError::Overflow)?)?;
    let mut patterns = Vec::with_capacity(num_symbols);
    for chunk in pattern_bytes.as_chunks::<3>().0 {
        patterns.push([chunk[0], chunk[1], chunk[2]]);
    }
    if num_symbols & 1 != 0 {
        cursor.take(1)?;
    }
    let lengths = symbol_lengths(&patterns)?;

    let mut bases = vec![0u64; count];
    for i in (0..count.saturating_sub(1)).rev() {
        bases[i] = bases[i + 1]
            .checked_add(u64::from(offsets[i]))
            .and_then(|value| value.checked_sub(u64::from(offsets[i + 1])))
            .ok_or(ParseError::InvalidFormat)?
            / 2;
    }
    for (i, base) in bases.iter_mut().enumerate() {
        let shift = 64 - usize::from(min_len) - i;
        if *base > u64::MAX >> shift {
            return Err(ParseError::InvalidFormat);
        }
        *base <<= shift;
    }

    let total_blocks = real_blocks
        .checked_add(extra_blocks)
        .ok_or(ParseError::Overflow)?;
    let slots = table_size
        .checked_add((1u64 << index_bits) - 1)
        .ok_or(ParseError::Overflow)?
        >> index_bits;
    let index_bytes = usize::try_from(slots.checked_mul(6).ok_or(ParseError::Overflow)?)
        .map_err(|_| ParseError::Overflow)?;
    let size_bytes =
        usize::try_from(u64::from(total_blocks) * 2).map_err(|_| ParseError::Overflow)?;
    let data_bytes = usize::try_from(
        u64::from(real_blocks)
            .checked_shl(u32::from(block_size))
            .ok_or(ParseError::Overflow)?,
    )
    .map_err(|_| ParseError::Overflow)?;
    Ok(PairHeader::Compressed {
        flags,
        block_size,
        index_bits,
        real_blocks,
        total_blocks,
        min_len,
        offsets: offsets.into_boxed_slice(),
        bases: bases.into_boxed_slice(),
        symbol_lengths: lengths,
        patterns: patterns.into_boxed_slice(),
        index_bytes,
        size_bytes,
        data_bytes,
    })
}

fn choose(n: usize, k: usize) -> Result<u64, ParseError> {
    if k == 0 || k > n {
        return Err(ParseError::InvalidFormat);
    }
    let mut numerator = 1u128;
    let mut denominator = 1u128;
    for i in 0..k {
        numerator = numerator
            .checked_mul((n - i) as u128)
            .ok_or(ParseError::Overflow)?;
        denominator = denominator
            .checked_mul((i + 1) as u128)
            .ok_or(ParseError::Overflow)?;
    }
    u64::try_from(numerator / denominator).map_err(|_| ParseError::Overflow)
}

pub(crate) fn parse_encoding(
    header: &[u8],
    shift: u8,
    file: usize,
    description: Description,
    pawn_factor_file: &[[u64; 4]; 6],
) -> Result<Encoding, ParseError> {
    let n = description.pieces;
    let primary = if description.primary_pawns > 0 {
        description.primary_pawns
    } else if description.king_pair {
        2
    } else {
        3
    };
    let secondary = description.secondary_pawns;
    if !(3..=7).contains(&n)
        || primary == 0
        || primary + secondary > n
        || description.primary_pawns > 6
        || secondary > 6
        || file >= 4
        || !matches!(shift, 0 | 4)
        || header.len() != n + 1 + usize::from(secondary > 0)
    {
        return Err(ParseError::InvalidFormat);
    }

    let nibble = |byte: u8| (byte >> shift) & 0x0f;
    let order = nibble(header[0]) as usize;
    let order2 = (secondary > 0).then(|| nibble(header[1]) as usize);
    let mut pieces = [0; 7];
    let mut norm = [0; 7];
    let mut factor = [0; 7];
    for i in 0..n {
        pieces[i] = nibble(header[i + 1 + usize::from(secondary > 0)]);
        if !matches!(pieces[i], 1..=6 | 9..=14) {
            return Err(ParseError::InvalidFormat);
        }
    }
    if pieces[..n].iter().filter(|&&piece| piece == 6).count() != 1
        || pieces[..n].iter().filter(|&&piece| piece == 14).count() != 1
    {
        return Err(ParseError::InvalidFormat);
    }
    if description.primary_pawns > 0 {
        if !matches!(pieces[0], 1 | 9) || pieces[..primary].iter().any(|&piece| piece != pieces[0])
        {
            return Err(ParseError::InvalidFormat);
        }
        if secondary > 0
            && (pieces[primary] != (pieces[0] ^ 8)
                || pieces[primary..primary + secondary]
                    .iter()
                    .any(|&piece| piece != pieces[primary]))
        {
            return Err(ParseError::InvalidFormat);
        }
    }

    norm[0] = primary as u8;
    if secondary > 0 {
        norm[primary] = secondary as u8;
    }
    let mut groups = 1 + usize::from(secondary > 0);
    let mut k = primary + secondary;
    while k < n {
        let end = (k + 1..n).find(|&j| pieces[j] != pieces[k]).unwrap_or(n);
        norm[k] = (end - k) as u8;
        groups += 1;
        k = end;
    }
    if order >= groups || order2.is_some_and(|second| second >= groups || second == order) {
        return Err(ParseError::InvalidFormat);
    }

    let mut remaining = 64 - primary - secondary;
    let mut next_group = primary + secondary;
    let mut size = 1u64;
    for slot in 0..groups {
        let multiplier = if slot == order {
            factor[0] = size;
            if description.primary_pawns > 0 {
                pawn_factor_file[primary - 1][file]
            } else if description.king_pair {
                462
            } else {
                31_332
            }
        } else if order2 == Some(slot) {
            factor[primary] = size;
            choose(48 - primary, secondary)?
        } else {
            if next_group >= n {
                return Err(ParseError::InvalidFormat);
            }
            factor[next_group] = size;
            let count = norm[next_group] as usize;
            let combinations = choose(remaining, count)?;
            remaining -= count;
            next_group += count;
            combinations
        };
        size = size.checked_mul(multiplier).ok_or(ParseError::Overflow)?;
    }
    if next_group != n || size == 0 {
        return Err(ParseError::InvalidFormat);
    }
    Ok(Encoding {
        pieces,
        norm,
        factor,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_encoding, parse_pair_header, parse_table, Description, PairHeader};
    use crate::storage::{Cursor, ParseError};
    use crate::tbprobe::INDICES;

    fn material_description(name: &str) -> Description {
        let mut counts = [0usize; 16];
        let mut color = 0;
        for character in name.chars() {
            if character == 'v' {
                color = 8;
                continue;
            }
            let piece = match character {
                'P' => 1,
                'N' => 2,
                'B' => 3,
                'R' => 4,
                'Q' => 5,
                'K' => 6,
                _ => panic!("unexpected table name"),
            };
            counts[color + piece] += 1;
        }
        let white = counts[1];
        let black = counts[9];
        let (primary_pawns, secondary_pawns) = if black > 0 && (white == 0 || white > black) {
            (black, white)
        } else {
            (white, black)
        };
        Description {
            pieces: counts.iter().sum(),
            primary_pawns,
            secondary_pawns,
            king_pair: counts.iter().filter(|&&count| count == 1).count() == 2,
        }
    }

    #[test]
    fn pawnless_header_has_bounded_king_factor() {
        let factors = [[1; 4]; 6];
        let description = Description {
            pieces: 3,
            primary_pawns: 0,
            secondary_pawns: 0,
            king_pair: false,
        };
        let encoding = parse_encoding(&[0, 14, 6, 5], 0, 0, description, &factors).unwrap();
        assert_eq!(encoding.pieces[..3], [14, 6, 5]);
        assert_eq!(encoding.norm[0], 3);
        assert_eq!(encoding.factor[0], 1);
        assert_eq!(encoding.size, 31_332);
        assert!(matches!(
            parse_encoding(&[1, 14, 6, 5], 0, 0, description, &factors),
            Err(ParseError::InvalidFormat)
        ));
        assert!(matches!(
            parse_encoding(&[0, 14, 6, 7], 0, 0, description, &factors),
            Err(ParseError::InvalidFormat)
        ));
    }

    #[test]
    fn compressed_header_rejects_cycles_bad_symbols_and_truncation() {
        let mut header = vec![0, 6, 4, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0xf0, 0xff, 0];
        assert!(matches!(
            parse_pair_header(&mut Cursor::new(&header), 16, true),
            Ok(PairHeader::Compressed { .. })
        ));
        for end in 0..header.len() {
            assert!(parse_pair_header(&mut Cursor::new(&header[..end]), 16, true).is_err());
        }

        header[15] = 0;
        header[16] = 0;
        assert_eq!(
            parse_pair_header(&mut Cursor::new(&header), 16, true).err(),
            Some(ParseError::InvalidFormat)
        );
        header[14] = 1;
        assert_eq!(
            parse_pair_header(&mut Cursor::new(&header), 16, true).err(),
            Some(ParseError::InvalidFormat)
        );

        header[14..17].copy_from_slice(&[0, 0xf0, 0xff]);
        header[2] = 63;
        assert_eq!(
            parse_pair_header(&mut Cursor::new(&header), u64::MAX, true).err(),
            Some(ParseError::Overflow)
        );
    }

    #[test]
    fn constant_header_owns_its_value_without_index_sections() {
        let pair = parse_pair_header(&mut Cursor::new(&[0x80, 4]), 1, true).unwrap();
        assert_eq!(pair.section_sizes(), [0; 3]);
        assert!(matches!(
            pair,
            PairHeader::Constant {
                flags: 0x80,
                value: 4
            }
        ));
    }

    #[test]
    #[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
    fn ci_compact_tables_parse_both_king_queen_headers() {
        let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
        let description = Description {
            pieces: 3,
            primary_pawns: 0,
            secondary_pawns: 0,
            king_pair: false,
        };
        let factors = [[1; 4]; 6];
        for suffix in ["rtbw", "rtbz"] {
            let bytes =
                std::fs::read(std::path::Path::new(&path).join(format!("KQvK.{suffix}"))).unwrap();
            let header = &bytes[5..9];
            let shifts: &[u8] = if suffix == "rtbw" && bytes[4] & 1 != 0 {
                &[0, 4]
            } else {
                &[0]
            };
            for &shift in shifts {
                let encoding = parse_encoding(header, shift, 0, description, &factors).unwrap();
                assert_eq!(encoding.size, 31_332);
            }
            let mut cursor = Cursor::new(&bytes);
            cursor.take(9).unwrap();
            cursor.align(2).unwrap();
            let first = parse_pair_header(&mut cursor, 31_332, suffix == "rtbw").unwrap();
            if suffix == "rtbw" && bytes[4] & 1 != 0 {
                assert!(matches!(first, PairHeader::Constant { .. }));
                let second = parse_pair_header(&mut cursor, 31_332, true).unwrap();
                assert!(matches!(second, PairHeader::Compressed { .. }));
            }
            let magic = if suffix == "rtbw" {
                0x5d23_e871
            } else {
                0xa50c_66d7
            };
            let parsed =
                parse_table(&bytes, magic, suffix == "rtbw", description, &factors).unwrap();
            assert_eq!(parsed.encodings[0].size, 31_332);
            let tables = if description.primary_pawns > 0 { 4 } else { 1 };
            assert_eq!(
                parsed.encodings.len(),
                tables * (1 + usize::from(suffix == "rtbw" && bytes[4] & 1 != 0))
            );
            assert!(parsed.pairs.iter().all(Option::is_some));
        }
    }

    fn check_directory(path: &std::path::Path) -> usize {
        let mut parsed_count = 0;
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            let (is_wdl, magic) = match extension {
                "rtbw" => (true, 0x5d23_e871),
                "rtbz" => (false, 0xa50c_66d7),
                _ => continue,
            };
            let description = material_description(path.file_stem().unwrap().to_str().unwrap());
            let bytes = std::fs::read(&path).unwrap();
            let parsed = parse_table(
                &bytes,
                magic,
                is_wdl,
                description,
                &INDICES.pawn_factor_file,
            )
            .unwrap_or_else(|err| panic!("{}: {err:?}", path.display()));
            assert!(parsed.pairs.iter().all(Option::is_some));
            for pair in parsed.pairs.into_iter().flatten() {
                assert!(pair.index.end <= bytes.len());
                assert!(pair.sizes.end <= bytes.len());
                assert!(pair.data.end <= bytes.len());
            }
            parsed_count += 1;
        }
        parsed_count
    }

    #[test]
    #[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
    fn ci_compact_tables_parse_every_file_with_checked_offsets() {
        let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
        let parsed_count = check_directory(std::path::Path::new(&path));
        assert_eq!(parsed_count, 10);
    }

    #[test]
    #[ignore = "requires the complete SYZYGY_PATH 3-4-5 set"]
    fn complete_tables_parse_every_file_with_checked_offsets() {
        let path = std::env::var("SYZYGY_PATH").expect("SYZYGY_PATH is required");
        let parsed_count = check_directory(std::path::Path::new(&path));
        assert_eq!(parsed_count, 290);
    }
}
