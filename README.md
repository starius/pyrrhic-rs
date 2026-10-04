# Pyrrhic-rs

`pyrrhic-rs` is a Rust library for probing Syzygy endgame tablebases during a
chess-engine search. This fork retains the original MIT attribution
and the Pyrrhic table format and probing policy.

It is based on [Algorhythm's pyrrhic-rs](https://github.com/Algorhythm-sxv/pyrrhic-rs).
The original Rust and Pyrrhic notices are in `LICENSE` and `LICENSE-PYRRHIC`.

## Usage

The parser, decoder, encoder, move generator, recursive probe, and generation
ownership use safe Rust. The only unsafe operation in this crate maps a table
file through `memmap2`; each mapping stays owned by its loaded generation.
The mapped files must remain unchanged and untruncated for the lifetime of
every generation using them. Read-only mappings do not protect against
changes made by another process.

`TableBases` checks input bitboards before probing and publishes complete WDL
and DTZ loads independently. Clone handles retain the same immutable discovery
generation. A newly constructed handle discovers and loads its own generation.

As `pyrrhic-rs` is designed to be used within an existing engine, the user must implement the `EngineAdapter` trait on a type for the probing code to be able to use the engine's own move generation code. Afterwards, `TableBases::new()` can be called using this type as a parameter.

### Example using `cozy_chess`:

```rust,no_run
use cozy_chess::*;
use pyrrhic_rs::EngineAdapter;

#[derive(Clone)]
struct CozyChessAdapter;

impl EngineAdapter for CozyChessAdapter {
    fn pawn_attacks(color: pyrrhic_rs::Color, sq: u64) -> u64 {
        let attacks = get_pawn_attacks(
            Square::index(sq as usize),
            if color == pyrrhic_rs::Color::Black {
                Color::Black
            } else {
                Color::White
            },
        );
        attacks.0
    }
    fn knight_attacks(sq: u64) -> u64 {
        get_knight_moves(Square::index(sq as usize)).0
    }
    fn bishop_attacks(sq: u64, occ: u64) -> u64 {
        get_bishop_moves(Square::index(sq as usize), BitBoard(occ)).0
    }
    fn rook_attacks(sq: u64, occ: u64) -> u64 {
        get_rook_moves(Square::index(sq as usize), BitBoard(occ)).0
    }
    fn king_attacks(sq: u64) -> u64 {
        get_king_moves(Square::index(sq as usize)).0
    }
    fn queen_attacks(sq: u64, occ: u64) -> u64 {
        (get_bishop_moves(Square::index(sq as usize), BitBoard(occ))
            | get_rook_moves(Square::index(sq as usize), BitBoard(occ)))
        .0
    }
}

fn main() {
    let _tb = pyrrhic_rs::TableBases::<CozyChessAdapter>::new("./syzygy").unwrap();
}
```

## Copyright
`pyrrhic-rs` was initially transliterated from the original [Pyrrhic](https://github.com/AndyGrant/Pyrrhic) library in C, and is therefore subject to the following copyrights:

- [Fathom](https://github.com/basil00/Fathom) © 2015 basil, all rights reserved
- Modifications Copyright © 2016-2019 by Jon Dart
- Modifications Copyright © 2020-2024 by Andrew Grant

## Acknowledgments
- Ronald "Syzygy" de Man, creator of the Syzygy tablebases
- [C2Rust](https://github.com/immunant/c2rust), used for the original translation
  of the C implementation into Rust

## Validation

With Rust and Cargo installed, run `cargo fmt --all --check`,
`cargo clippy --locked --all-targets --all-features -- -D warnings`, and
`cargo test --locked --all-features`. Tests marked `ignored` need Syzygy table
files; their required material and environment variables are stated beside
each test. Run `cargo package --locked --list` to inspect the distributable
library archive.

For the compact real-table suite, install the pinned Python dependency with
`python3 -m pip install -r tools/requirements.txt`, then run:

```sh
tables="$(mktemp -d)"
python3 tools/fetch_syzygy_ci.py --out-dir "$tables"
export SYZYGY_CI_PATH="$tables"
cargo test --locked --all-features ci_compact_tables_ \
  -- --ignored --test-threads=1
cargo test --locked --manifest-path tools/syzygy-reference/Cargo.toml \
  ci_reference_preserves_signed_distance_and_rounding \
  -- --ignored --test-threads=1
cargo build --locked --example syzygy_probe
cargo build --locked --manifest-path tools/syzygy-reference/Cargo.toml
python3 tools/compare_syzygy.py \
  --reference tools/syzygy-reference/target/debug/pyrrhic-syzygy-reference \
  --candidate target/debug/examples/syzygy_probe \
  --tables "$tables" --manifest nix/syzygy-3-4-5.json \
  --cases tools/syzygy_compact_cases.jsonl \
  --output-dir "$(mktemp -d)/comparison"
python3 -m unittest discover -s tools/tests -p 'test_*.py'
```

The compact set has ten files and totals 774,944 bytes. The independent
reference executable uses GPL crates in its separate Cargo workspace. It is
for testing only and is excluded from the library package. Full 3–5 piece
regressions in `tools/syzygy_full_regressions.jsonl` require the complete
table set and should be run when that set is already available.
