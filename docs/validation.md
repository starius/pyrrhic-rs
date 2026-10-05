# Full differential validation

The compact CI corpus keeps downloads below 1 MB. A separate full run uses
the complete three-to-five-piece set: 145 WDL/DTZ pairs, 290 files, and
983957920 bytes. Supply a directory containing those files through
`SYZYGY_FULL_PATH`. Their names, sizes, and SHA-256 hashes are pinned in
`nix/syzygy-3-4-5.json`; the comparator checks the supplied files against it.
Keep the directory unchanged while any probe process may use it.

The generator enumerates legal three-piece placements and samples four- and
five-piece positions using a fixed seed, placements in both color
orientations, and legal walks. It adds explicit en-passant, promotion,
zeroing, and halfmove-clock cases. This covers the complete material set;
four- and five-piece position coverage is sampled rather than exhaustive.

## Run the current fork

The commands below select implementation and tools revision
`61bb83b7c46de492198b1565f45129e48f240c48`. Later documentation commits do not
change that implementation. Run from the fork's root in Bash, after checking
out that revision in a separate checkout. Record a different revision
explicitly when evaluating a library change.

```bash
set -euo pipefail
: "${SYZYGY_FULL_PATH:?Set this to the complete 3-5 piece table directory}"
validation_dir="$(mktemp -d)"
git rev-parse HEAD > "$validation_dir/source-revision.txt"
git status --porcelain=v1 > "$validation_dir/source-status.txt"
rustc -Vv > "$validation_dir/rustc.txt"
cargo -V > "$validation_dir/cargo.txt"
python3 -V > "$validation_dir/python.txt"

python3 -m venv "$validation_dir/venv"
"$validation_dir/venv/bin/python" -m pip install -r tools/requirements.txt

cargo build --locked --release --example syzygy_probe
cargo build --locked --release \
  --manifest-path tools/syzygy-reference/Cargo.toml

"$validation_dir/venv/bin/python" tools/generate_syzygy_cases.py \
  --manifest nix/syzygy-3-4-5.json \
  --output "$validation_dir/cases.jsonl" \
  --seed 1398364761 \
  --placements-per-orientation 512 --walks-per-material 256 \
  | tee "$validation_dir/generation.log"

set +e
"$validation_dir/venv/bin/python" tools/compare_syzygy.py \
  --candidate target/release/examples/syzygy_probe \
  --reference tools/syzygy-reference/target/release/pyrrhic-syzygy-reference \
  --tables "$SYZYGY_FULL_PATH" \
  --manifest nix/syzygy-3-4-5.json \
  --cases "$validation_dir/cases.jsonl" \
  --output-dir "$validation_dir/comparison" \
  --batch-size 50000 --timeout 1800 \
  > "$validation_dir/comparison.log" 2>&1
comparison_status=$?
set -e
printf '%s\n' "$comparison_status" > "$validation_dir/comparison.exit-code"
cat "$validation_dir/comparison.log"
test "$comparison_status" -eq 0
printf 'Evidence: %s\n' "$validation_dir"
```

The reference executable uses GPL crates in its independent workspace. It
is a separate test process and does not enter the library dependency graph
or package. Each DTZ request also compares WDL. Root comparisons check
distances, legal move translation, and outcome equivalence when the engines
choose different equally valid moves.

Allow enough time and storage for millions of requests and their raw
transcripts. The corpus and its generation invocation stay beside the
comparison. `comparison/invocation.json` records corpus, manifest, table,
and executable hashes, batch size, and timeout. Each `batch-*` directory
holds requests, per-process invocations, stdout, stderr, and status records
with exit code, reply count, and transcript hashes. `summary.json` is written
only after all batches are compared; require zero mismatches and a compared
count equal to the recorded corpus size.

If interrupted, retain the partial directory. Re-run the same comparison
command with `--resume` and the original output directory only when using
the same inputs and binaries. The comparator checks the stored hashes,
invocations, successful exits, and complete transcripts before reusing a
batch. Changed inputs require a new output directory.

## Historical development evidence

On 2026-10-03, the vendored implementation used during
[Ember PR 56](https://github.com/ExxDreamerCode/Ember/pull/56) passed
4,271,533 requests: 4,220,960 DTZ requests and 50,573 root requests, with zero
mismatches under the comparator's outcome-equivalence rules. The run also
compared the earlier corrected backend. It did not run the Ember adapter
for every request, and it predates the standalone fork's published revision.

The retained candidate source snapshot is
`/root/ember-syzygy-safe-20261003/perf-prefix12` on the development host
accessed as `ssh large`. The complete run lives at
`/root/ember-syzygy-safe-20261003/full-diff-prefix12`, including its
`invocation.json`, `summary.json`, and all 86 raw batches. The invoking script
is `/root/ember-syzygy-safe-20261003/run-full-diff-prefix12.sh`; its adjacent
status file records exit code 0. These private development artifacts are
retained for audit, rather than committed to the library repository.

The original input corpus is `full-corpus.jsonl` in the same development
directory. Its generation used seed `1398364761`, 512 placements per
orientation, and 256 walks per material. The run used batches of 50000 and
a timeout of 1800 seconds per process. Its recorded SHA-256 identities are:

| Input | SHA-256 |
| --- | --- |
| Corpus | `314d422c04247c47e96c684068203a50c409d3c73d139a1f7e50e56ea8f1bd9e` |
| Table manifest | `7ae2d9397a98bfbd05149bb078ed619eb146357d0da38946b83cbb8e31116c61` |
| Candidate executable | `a976ace823cc763c5253047788cd847377d1bc0e43ca2bc7bf1b51ec36704325` |
| Earlier backend executable | `6961782fbcce3b602cd78f7a8adebb117598542e6672aa6b24db390744308247` |
| Independent reference executable | `1c24c538cc75f1e99c33c2cca68466842e052782635edaa6a1f91f0d874caca8` |

The historical generator is retained at
`/root/ember-syzygy-safe-20261003/perf-prefix12/tools/generate_syzygy_cases.py`
with SHA-256
`328df8c8b83f64136b09c060a96a61b6b995868f167ae0de9225aa6cd7b706b5`.
The current fork adds halfmove-boundary root requests, so generating a
corpus with its tools creates a new dataset and comparison result. Keep
that new evidence separate from the historical run; use the retained
corpus and verify its hash when replaying the original input.
