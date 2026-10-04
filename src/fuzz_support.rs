#![forbid(unsafe_code)]
//! Narrow access to private parser and decoder paths for the isolated fuzz
//! workspace. This module is absent from default and release builds.

use std::{ops::Range, path::Path, sync::OnceLock};

use crate::{
    table_decoder::decode_pair,
    table_parser::{parse_table, Description, ParsedPair},
    tbprobe::INDICES,
};

const MATERIALS: [(&str, Description); 4] = [
    (
        "KQvK",
        Description {
            pieces: 3,
            primary_pawns: 0,
            secondary_pawns: 0,
            king_pair: false,
        },
    ),
    (
        "KPvK",
        Description {
            pieces: 3,
            primary_pawns: 1,
            secondary_pawns: 0,
            king_pair: false,
        },
    ),
    (
        "KRvKP",
        Description {
            pieces: 4,
            primary_pawns: 1,
            secondary_pawns: 0,
            king_pair: false,
        },
    ),
    (
        "KNNvKR",
        Description {
            pieces: 5,
            primary_pawns: 0,
            secondary_pawns: 0,
            king_pair: false,
        },
    ),
];

/// Parse arbitrary bytes under one of the compact table layouts. The first
/// byte selects material and WDL/DTZ; the rest is the proposed table file.
pub fn parse_metadata(input: &[u8]) {
    let Some((&selector, bytes)) = input.split_first() else {
        return;
    };
    let (_, description) = MATERIALS[usize::from(selector >> 1) % MATERIALS.len()];
    let is_wdl = selector & 1 == 0;
    let magic = if is_wdl { 0x5d23_e871 } else { 0xa50c_66d7 };
    let _ = parse_table(bytes, magic, is_wdl, description, &INDICES.pawn_factor_file);
}

struct DecoderSeed {
    bytes: Vec<u8>,
    pair: ParsedPair,
    positions: u64,
}

static DECODER_SEED: OnceLock<DecoderSeed> = OnceLock::new();

fn decoder_seed() -> &'static DecoderSeed {
    DECODER_SEED.get_or_init(|| {
        let directory = std::env::var("SYZYGY_CI_PATH")
            .expect("SYZYGY_CI_PATH must point to the compact table set");
        let bytes = std::fs::read(Path::new(&directory).join("KQvK.rtbz"))
            .expect("compact KQvK.rtbz is required");
        let parsed = parse_table(
            &bytes,
            0xa50c_66d7,
            false,
            MATERIALS[0].1,
            &INDICES.pawn_factor_file,
        )
        .expect("compact KQvK.rtbz must parse");
        DecoderSeed {
            bytes,
            pair: parsed.pairs[0].clone().expect("first DTZ pair is present"),
            positions: parsed.encodings[0].size,
        }
    })
}

/// Decode with metadata from an actual validated compact table. Mutate one
/// of its index, size, or compressed-data sections for each input. Eight bytes
/// select an offset across the entire section.
fn mutation_span(input: &[u8], section: &Range<usize>) -> Option<(usize, usize)> {
    if input.len() < 17 || section.is_empty() {
        return None;
    }
    let selector = u64::from_le_bytes(input[9..17].try_into().ok()?);
    let section_len = u64::try_from(section.len()).ok()?;
    let offset = section.start + usize::try_from(selector % section_len).ok()?;
    let count = (section.end - offset).min(input.len() - 17).min(256);
    Some((offset, count))
}

pub fn decode_mutated(input: &[u8]) {
    if input.len() < 17 {
        return;
    }
    let seed = decoder_seed();
    let mut bytes = seed.bytes.clone();
    let index =
        u64::from_le_bytes(input[1..9].try_into().expect("checked length")) % seed.positions;
    let section = match input[0] % 3 {
        0 => &seed.pair.index,
        1 => &seed.pair.sizes,
        _ => &seed.pair.data,
    };
    if let Some((offset, count)) = mutation_span(input, section) {
        for (old, change) in bytes[offset..offset + count]
            .iter_mut()
            .zip(&input[17..17 + count])
        {
            *old ^= *change;
        }
    }
    let _ = decode_pair(&bytes, &seed.pair, index);
}

/// Ensure seed files exercise each selected metadata layout.
pub fn check_compact_seeds(directory: &Path) {
    for (material, description) in MATERIALS {
        for (extension, magic, is_wdl) in
            [("rtbw", 0x5d23_e871, true), ("rtbz", 0xa50c_66d7, false)]
        {
            let bytes = std::fs::read(directory.join(format!("{material}.{extension}")))
                .expect("compact seed file is required");
            parse_table(
                &bytes,
                magic,
                is_wdl,
                description,
                &INDICES.pawn_factor_file,
            )
            .expect("compact seed must parse");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_compact_seeds, mutation_span};

    #[test]
    fn decoder_mutations_reach_late_offsets_and_stay_in_section() {
        let mut input = vec![0; 17 + 256];
        input[9..17].copy_from_slice(&700u64.to_le_bytes());
        assert_eq!(mutation_span(&input, &(100..1000)), Some((800, 200)));
        input[9..17].copy_from_slice(&895u64.to_le_bytes());
        assert_eq!(mutation_span(&input, &(100..1000)), Some((995, 5)));
    }

    #[test]
    #[ignore = "run with SYZYGY_CI_PATH pointing to the compact Nix tablebase set"]
    fn ci_compact_tables_cover_fuzz_metadata_layouts() {
        let path = std::env::var("SYZYGY_CI_PATH").expect("SYZYGY_CI_PATH is required");
        check_compact_seeds(std::path::Path::new(&path));
    }
}
