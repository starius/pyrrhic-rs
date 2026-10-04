//! Immutable table discovery and synchronized, owned table loading.
#![forbid(unsafe_code)]

use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::OnceLock,
};

use crate::{
    storage::TableBytes,
    table_decoder::decode_pair,
    table_encoder::{encode_squares, fill_squares, leading_pawn},
    table_parser::{parse_table, Description, ParsedTable},
    tbprobe::{
        PyrrhicPosition, INDICES, PYRRHIC_PRIME_BBISHOP, PYRRHIC_PRIME_BKNIGHT,
        PYRRHIC_PRIME_BPAWN, PYRRHIC_PRIME_BQUEEN, PYRRHIC_PRIME_BROOK, PYRRHIC_PRIME_WBISHOP,
        PYRRHIC_PRIME_WKNIGHT, PYRRHIC_PRIME_WPAWN, PYRRHIC_PRIME_WQUEEN, PYRRHIC_PRIME_WROOK,
    },
};

const MIN_FILE_SIZE: u64 = 80;
const HASH_SIZE: usize = 4096;
const MAGIC: [u32; 2] = [0x5d23_e871, 0xa50c_66d7];
const SUFFIX: [&str; 2] = [".rtbw", ".rtbz"];
const WDL_TO_MAP: [usize; 5] = [1, 3, 0, 2, 0];
const PA_FLAGS: [u8; 5] = [8, 0, 0, 0, 4];
const PIECE_LETTERS: [char; 5] = ['Q', 'R', 'B', 'N', 'P'];
const MATERIAL_SHAPES: [(usize, usize); 11] = [
    (1, 0),
    (1, 1),
    (2, 0),
    (2, 1),
    (3, 0),
    (2, 2),
    (3, 1),
    (4, 0),
    (5, 0),
    (4, 1),
    (3, 2),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LookupError;

pub(crate) enum TableProbeValue {
    Value(i32),
    WrongSide,
}

#[derive(Clone, Copy)]
struct HashSlot {
    key: u64,
    entry: usize,
}

struct LoadedTable {
    backing: TableBytes,
    parsed: ParsedTable,
}

struct Entry {
    name: String,
    key: u64,
    symmetric: bool,
    description: Description,
    has_dtz: bool,
    loaded: [OnceLock<Result<LoadedTable, LookupError>>; 2],
}

impl Entry {
    fn file_name(&self, kind: usize) -> String {
        format!("{}{}", self.name, SUFFIX[kind])
    }
}

pub(crate) struct Generation {
    paths: Vec<PathBuf>,
    discovered: Vec<(String, bool)>,
    entries: Vec<Entry>,
    hash: [Option<HashSlot>; HASH_SIZE],
    max_cardinality: u32,
    num_dtz: u32,
}

impl Generation {
    pub(crate) fn new() -> Self {
        Self {
            paths: Vec::new(),
            discovered: Vec::new(),
            entries: Vec::new(),
            hash: [None; HASH_SIZE],
            max_cardinality: 0,
            num_dtz: 0,
        }
    }

    pub(crate) fn initialize(&mut self, path: &str) -> Result<(), LookupError> {
        if path.is_empty() || path == "<empty>" {
            return Ok(());
        }
        if path.contains('\0') {
            return Err(LookupError);
        }
        let separator = if cfg!(windows) { ';' } else { ':' };
        self.paths.extend(
            path.split(separator)
                .filter(|component| !component.is_empty())
                .map(PathBuf::from),
        );
        for (left_count, right_count) in MATERIAL_SHAPES {
            let left = combinations(left_count);
            let right = combinations(right_count);
            for left_side in &left {
                for right_side in &right {
                    if left_count == right_count && left_side > right_side {
                        continue;
                    }
                    let name = format!("K{}vK{}", letters(left_side), letters(right_side));
                    self.discover(&name)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn max_pieces(&self) -> u32 {
        self.max_cardinality
    }

    pub(crate) fn materials(&self) -> Vec<(String, bool)> {
        self.discovered.clone()
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (i32, i32) {
        (self.entries.len() as i32, self.num_dtz as i32)
    }

    fn open(&self, name: &str) -> Option<File> {
        self.paths
            .iter()
            .find_map(|path| OpenOptions::new().read(true).open(path.join(name)).ok())
    }

    fn available(&self, name: &str) -> bool {
        let Some(file) = self.open(name) else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        let size = metadata.len();
        if size < MIN_FILE_SIZE || size & 63 != 16 {
            eprintln!("Incomplete tablebase file {name}");
            println!("info string Incomplete tablebase file {name}");
            return false;
        }
        true
    }

    fn insert_hash(&mut self, key: u64, entry: usize) -> Result<(), LookupError> {
        let mut index = (key >> 52) as usize;
        for _ in 0..HASH_SIZE {
            if self.hash[index].is_none() {
                self.hash[index] = Some(HashSlot { key, entry });
                return Ok(());
            }
            index = (index + 1) & (HASH_SIZE - 1);
        }
        Err(LookupError)
    }

    fn lookup(&self, key: u64) -> Option<&Entry> {
        let mut index = (key >> 52) as usize;
        for _ in 0..HASH_SIZE {
            match self.hash[index] {
                Some(slot) if slot.key == key => return self.entries.get(slot.entry),
                None => return None,
                _ => index = (index + 1) & (HASH_SIZE - 1),
            }
        }
        None
    }

    fn discover(&mut self, name: &str) -> Result<(), LookupError> {
        if !self.available(&format!("{name}.rtbw")) {
            return Ok(());
        }
        let mut pieces = [0u8; 16];
        let mut color = 0usize;
        for letter in name.chars() {
            if letter == 'v' {
                color = 8;
                continue;
            }
            let kind = match letter {
                'P' => 1,
                'N' => 2,
                'B' => 3,
                'R' => 4,
                'Q' => 5,
                'K' => 6,
                _ => return Err(LookupError),
            };
            pieces[color + kind] += 1;
        }
        let key = piece_key(&pieces, false);
        let mirror = piece_key(&pieces, true);
        let white_pawns = usize::from(pieces[1]);
        let black_pawns = usize::from(pieces[9]);
        let has_pawns = white_pawns != 0 || black_pawns != 0;
        let (primary_pawns, secondary_pawns) =
            if black_pawns != 0 && (white_pawns == 0 || white_pawns > black_pawns) {
                (black_pawns, white_pawns)
            } else {
                (white_pawns, black_pawns)
            };
        let count = pieces
            .iter()
            .map(|&value| usize::from(value))
            .sum::<usize>();
        let description = Description {
            pieces: count,
            primary_pawns,
            secondary_pawns,
            king_pair: !has_pawns && pieces.iter().filter(|&&value| value == 1).count() == 2,
        };
        let has_dtz = self.available(&format!("{name}.rtbz"));
        let index = self.entries.len();
        self.entries.push(Entry {
            name: name.to_owned(),
            key,
            symmetric: key == mirror,
            description,
            has_dtz,
            loaded: std::array::from_fn(|_| OnceLock::new()),
        });
        self.insert_hash(key, index)?;
        if key != mirror {
            self.insert_hash(mirror, index)?;
        }
        self.discovered.push((name.to_owned(), has_dtz));
        self.max_cardinality = self.max_cardinality.max(count as u32);
        self.num_dtz += u32::from(has_dtz);
        Ok(())
    }

    fn load(&self, entry: &Entry, kind: usize) -> Result<LoadedTable, LookupError> {
        let name = entry.file_name(kind);
        let file = self.open(&name).ok_or(LookupError)?;
        let size = file.metadata().map_err(|_| LookupError)?.len();
        if size < MIN_FILE_SIZE || size & 63 != 16 {
            return Err(LookupError);
        }
        let backing = TableBytes::map(&file).map_err(|_| LookupError)?;
        if backing.as_slice().len() as u64 != size {
            return Err(LookupError);
        }
        let parsed = parse_table(
            backing.as_slice(),
            MAGIC[kind],
            kind == 0,
            entry.description,
            &INDICES.pawn_factor_file,
        )
        .map_err(|_| LookupError)?;
        Ok(LoadedTable { backing, parsed })
    }
}

fn letters(indices: &[usize]) -> String {
    indices.iter().map(|&index| PIECE_LETTERS[index]).collect()
}

fn combinations(count: usize) -> Vec<Vec<usize>> {
    fn append(out: &mut Vec<Vec<usize>>, current: &mut Vec<usize>, count: usize, first: usize) {
        if current.len() == count {
            out.push(current.clone());
            return;
        }
        for index in first..PIECE_LETTERS.len() {
            current.push(index);
            append(out, current, count, index);
            current.pop();
        }
    }
    let mut out = Vec::new();
    append(&mut out, &mut Vec::new(), count, 0);
    out
}

fn piece_key(pieces: &[u8; 16], mirror: bool) -> u64 {
    let primes = [
        0,
        PYRRHIC_PRIME_WPAWN,
        PYRRHIC_PRIME_WKNIGHT,
        PYRRHIC_PRIME_WBISHOP,
        PYRRHIC_PRIME_WROOK,
        PYRRHIC_PRIME_WQUEEN,
        0,
        0,
        0,
        PYRRHIC_PRIME_BPAWN,
        PYRRHIC_PRIME_BKNIGHT,
        PYRRHIC_PRIME_BBISHOP,
        PYRRHIC_PRIME_BROOK,
        PYRRHIC_PRIME_BQUEEN,
        0,
        0,
    ];
    primes
        .iter()
        .enumerate()
        .fold(0u64, |sum, (index, &prime)| {
            sum.wrapping_add(
                u64::from(pieces[index ^ if mirror { 8 } else { 0 }]).wrapping_mul(prime),
            )
        })
}

pub(crate) fn material_key(pos: &PyrrhicPosition, mirror: bool) -> u64 {
    let (white, black) = if mirror {
        (pos.black, pos.white)
    } else {
        (pos.white, pos.black)
    };
    u64::from((white & pos.queens).count_ones())
        .wrapping_mul(PYRRHIC_PRIME_WQUEEN)
        .wrapping_add(u64::from((white & pos.rooks).count_ones()).wrapping_mul(PYRRHIC_PRIME_WROOK))
        .wrapping_add(
            u64::from((white & pos.bishops).count_ones()).wrapping_mul(PYRRHIC_PRIME_WBISHOP),
        )
        .wrapping_add(
            u64::from((white & pos.knights).count_ones()).wrapping_mul(PYRRHIC_PRIME_WKNIGHT),
        )
        .wrapping_add(u64::from((white & pos.pawns).count_ones()).wrapping_mul(PYRRHIC_PRIME_WPAWN))
        .wrapping_add(
            u64::from((black & pos.queens).count_ones()).wrapping_mul(PYRRHIC_PRIME_BQUEEN),
        )
        .wrapping_add(u64::from((black & pos.rooks).count_ones()).wrapping_mul(PYRRHIC_PRIME_BROOK))
        .wrapping_add(
            u64::from((black & pos.bishops).count_ones()).wrapping_mul(PYRRHIC_PRIME_BBISHOP),
        )
        .wrapping_add(
            u64::from((black & pos.knights).count_ones()).wrapping_mul(PYRRHIC_PRIME_BKNIGHT),
        )
        .wrapping_add(u64::from((black & pos.pawns).count_ones()).wrapping_mul(PYRRHIC_PRIME_BPAWN))
}

pub(crate) fn probe_table_value(
    owner: &Generation,
    pos: &PyrrhicPosition,
    wdl: i32,
    dtz: bool,
) -> Result<TableProbeValue, LookupError> {
    let key = material_key(pos, false);
    if !dtz && key == 0 {
        return Ok(TableProbeValue::Value(0));
    }
    let entry = owner.lookup(key).ok_or(LookupError)?;
    let kind = usize::from(dtz);
    if dtz && !entry.has_dtz {
        return Err(LookupError);
    }
    let table = entry.loaded[kind]
        .get_or_init(|| owner.load(entry, kind))
        .as_ref()
        .map_err(|_| LookupError)?;
    let parsed = &table.parsed;
    let backing = table.backing.as_slice();
    let (flip, bside) = if entry.symmetric {
        (!pos.turn, false)
    } else {
        let flip = key != entry.key;
        (flip, pos.turn == flip)
    };
    let description = entry.description;
    let mut squares = [0u8; 7];
    let mut file = 0usize;
    let mut flags = 0u8;
    let pair_index;
    let index = if description.primary_pawns == 0 {
        pair_index = if dtz { 0 } else { usize::from(bside) };
        if dtz {
            let pair = parsed
                .pairs
                .first()
                .and_then(Option::as_ref)
                .ok_or(LookupError)?;
            flags = pair.header.flags();
            if flags & 1 != u8::from(bside) && !entry.symmetric {
                return Ok(TableProbeValue::WrongSide);
            }
        }
        let encoding = parsed.encodings.get(pair_index).ok_or(LookupError)?;
        let mut filled = 0;
        while filled < description.pieces {
            filled = fill_squares(pos, encoding, description, flip, 0, &mut squares, filled)
                .map_err(|_| LookupError)?;
        }
        encode_squares(&mut squares, encoding, description).map_err(|_| LookupError)?
    } else {
        let mirror = if flip { 0x38 } else { 0 };
        let first = parsed.encodings.first().ok_or(LookupError)?;
        let mut filled = fill_squares(pos, first, description, flip, mirror, &mut squares, 0)
            .map_err(|_| LookupError)?;
        file = leading_pawn(&mut squares, description.primary_pawns).map_err(|_| LookupError)?;
        pair_index = if dtz {
            file
        } else {
            file + 4 * usize::from(bside)
        };
        if dtz {
            let pair = parsed
                .pairs
                .get(file)
                .and_then(Option::as_ref)
                .ok_or(LookupError)?;
            flags = pair.header.flags();
            if flags & 1 != u8::from(bside) && !entry.symmetric {
                return Ok(TableProbeValue::WrongSide);
            }
        }
        let encoding = parsed.encodings.get(pair_index).ok_or(LookupError)?;
        while filled < description.pieces {
            filled = fill_squares(
                pos,
                encoding,
                description,
                flip,
                mirror,
                &mut squares,
                filled,
            )
            .map_err(|_| LookupError)?;
        }
        encode_squares(&mut squares, encoding, description).map_err(|_| LookupError)?
    };
    let pair = parsed
        .pairs
        .get(pair_index)
        .and_then(Option::as_ref)
        .ok_or(LookupError)?;
    if !parsed
        .encodings
        .get(pair_index)
        .is_some_and(|encoding| index < encoding.size)
    {
        return Err(LookupError);
    }
    let decoded = decode_pair(backing, pair, index).map_err(|_| LookupError)?;
    if !dtz {
        // Reject impossible leaves before capture search can discard their score.
        if decoded[0] > 4 {
            return Err(LookupError);
        }
        return Ok(TableProbeValue::Value(i32::from(decoded[0]) - 2));
    }
    let wdl_index = usize::try_from(wdl + 2).map_err(|_| LookupError)?;
    if wdl_index >= 5 {
        return Err(LookupError);
    }
    let mut value = i32::from(decoded[0]) + ((i32::from(decoded[1]) & 0xf) << 8);
    if flags & 2 != 0 {
        let range = parsed
            .map_ranges
            .get(file)
            .and_then(|ranges| ranges.get(WDL_TO_MAP[wdl_index]))
            .ok_or(LookupError)?;
        let map = backing.get(range.clone()).ok_or(LookupError)?;
        let value_index = usize::try_from(value).map_err(|_| LookupError)?;
        value = if flags & 16 == 0 {
            i32::from(*map.get(value_index).ok_or(LookupError)?)
        } else {
            let offset = value_index.checked_mul(2).ok_or(LookupError)?;
            let bytes = map.get(offset..offset + 2).ok_or(LookupError)?;
            i32::from(u16::from_le_bytes([bytes[0], bytes[1]]))
        };
    }
    if flags & PA_FLAGS[wdl_index] == 0 || wdl & 1 != 0 {
        value *= 2;
    }
    Ok(TableProbeValue::Value(value))
}
