# Isolated GPL Syzygy reference

This executable is an independent correctness oracle for the maintained MIT
Pyrrhic backend. Its `Cargo.toml` and `Cargo.lock` form a separate workspace.
It is excluded from the root Cargo dependency graph and the library package.

The executable uses `shakmaty 0.30.0` and `shakmaty-syzygy 0.28.0`. Both pinned
crates declare `GPL-3.0-or-later`; this tool's package declares the same
license. Keep their notices with any independently distributed copy of the
tool. Do not copy implementation code from either crate into Pyrrhic.

Build with:

```sh
cargo build --locked --release \
  --manifest-path tools/syzygy-reference/Cargo.toml \
  --target-dir target/syzygy-reference
```

Run `target/syzygy-reference/release/pyrrhic-syzygy-reference --tables DIRECTORY`
and send one JSON request per input line. The protocol version is `1`. A
request has an `id`, legal standard-chess `fen`, an `operation` of `wdl`, `dtz`,
or `root`, and optional `required_files` with exact filenames. Every successful
request returns a five-valued `wdl` in `[-2, 2]`. DTZ requests also return a
signed `dtz` and an explicit `rounded` bit, plus the halfmove-aware `wdl50`.
Root requests also return terminal status, a selected move, every legal
move's child WDL/DTZ result, the reference policy's `optimal_moves`, and
`mate_moves` for immediate checkmates. The comparison harness checks the
tablebase outcome and reports reference policy differences separately when
moves preserve the same WDL and immediate DTZ. Probe or coverage errors have
a non-`ok` status; the comparison harness must fail on them.

`tools/syzygy_full_regressions.jsonl` contains direct-probe positions that
exposed a missed en passant right after double pawn pushes. Run it with the
complete pinned three-to-five-piece set; the compact CI set lacks the needed
successor materials.
