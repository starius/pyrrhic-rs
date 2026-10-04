# Isolated Syzygy fuzz targets

This workspace is excluded from the root Cargo workspace and library package.
It fuzzes the MIT probe directly and has no GPL dependency.
The `fuzzing` feature exposes narrow parser/decoder hooks only to this tool.

Install `cargo-fuzz`, a compatible nightly Rust toolchain, and
`python-chess==1.999`. Fetch the
hash-pinned compact table set and generate local seed corpora without checking
table payloads into Git:

```bash
tables="$(mktemp -d)"
python3 tools/fetch_syzygy_ci.py --out-dir "$tables"
python3 tools/syzygy-fuzz/prepare_corpus.py --tables "$tables"
export SYZYGY_CI_PATH="$tables"
cargo fuzz run metadata --fuzz-dir tools/syzygy-fuzz -- -max_total_time=1800
cargo fuzz run decoder --fuzz-dir tools/syzygy-fuzz -- -max_total_time=1800
cargo fuzz run public_position --fuzz-dir tools/syzygy-fuzz -- -max_total_time=1800
```

Run one target at a time. Save the exact invocation, toolchain, corpus hashes,
stdout/stderr, and any minimized crash under an ignored result directory. The
decoder target starts with validated metadata from `KQvK.rtbz`, then mutates
its index, size, or compressed data sections. The metadata target selects
several pawn and pawnless layouts. The public target calls all three probe
entrypoints with arbitrary bitboards and compact table files.
