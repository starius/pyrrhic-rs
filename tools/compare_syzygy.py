#!/usr/bin/env python3
"""Compare independent Syzygy probe processes over an explicit JSONL corpus."""

import argparse
import base64
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_atomic(path, data):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)


def requests_from(path):
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                raise ValueError(f"blank case at line {line_number}")
            request = json.loads(line)
            if set(request) != {"version", "id", "fen", "operation", "required_files"}:
                raise ValueError(f"unexpected request schema at line {line_number}")
            if request["version"] != 1 or request["operation"] not in ("wdl", "dtz", "root"):
                raise ValueError(f"invalid request at line {line_number}")
            if not isinstance(request["id"], str) or not request["id"]:
                raise ValueError(f"missing id at line {line_number}")
            if not isinstance(request["fen"], str) or not request["fen"]:
                raise ValueError(f"missing FEN at line {line_number}")
            files = request["required_files"]
            if not isinstance(files, list) or any(
                not isinstance(name, str)
                or not name.endswith((".rtbw", ".rtbz"))
                or name.startswith(".")
                or "/" in name
                or "\\" in name
                for name in files
            ):
                raise ValueError(f"invalid required_files at line {line_number}")
            yield request


def preflight(args):
    args.candidate = args.candidate.resolve(strict=True)
    args.reference = args.reference.resolve(strict=True)
    if args.baseline:
        args.baseline = args.baseline.resolve(strict=True)
    args.cases = args.cases.resolve(strict=True)
    args.tables = args.tables.resolve(strict=True)
    if not args.tables.is_dir() or not args.cases.is_file():
        raise ValueError("table directory and case file are required")
    for executable in (args.candidate, args.reference, args.baseline):
        if executable and not executable.is_file():
            raise ValueError(f"missing probe executable: {executable}")
    if args.batch_size <= 0 or args.timeout <= 0:
        raise ValueError("batch size and timeout must be positive")

    expected = {}
    if args.manifest:
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        expected = {entry["name"]: entry for entry in manifest}
        if len(expected) != len(manifest):
            raise ValueError("duplicate filename in table manifest")
    required = set()
    ids = set()
    count = 0
    by_operation = Counter()
    for request in requests_from(args.cases):
        if request["id"] in ids:
            raise ValueError(f"duplicate case id {request['id']}")
        ids.add(request["id"])
        required.update(request["required_files"])
        by_operation[request["operation"]] += 1
        count += 1
    if count == 0:
        raise ValueError("case file is empty")
    for name in sorted(required):
        file = args.tables / name
        if not file.is_file():
            raise ValueError(f"required table missing: {file}")
        if expected and name not in expected:
            raise ValueError(f"table manifest mismatch: {name}")
    # A probe can read successor tables that are absent from required_files.
    # Bind a resumed comparison to the entire table directory it used.
    table_sha256 = {}
    for file in sorted(args.tables.iterdir()):
        if not file.is_file() or file.suffix not in (".rtbw", ".rtbz"):
            continue
        name = file.name
        digest = sha256(file)
        table_sha256[name] = digest
        if name in expected:
            if file.stat().st_size != expected[name]["bytes"]:
                raise ValueError(f"table manifest mismatch: {name}")
            sri = "sha256-" + base64.b64encode(bytes.fromhex(digest)).decode("ascii")
            if sri != expected[name]["hash"]:
                raise ValueError(f"table hash mismatch: {name}")
    return {
        "case_count": count,
        "operations": dict(by_operation),
        "required_files": sorted(required),
        "table_sha256": table_sha256,
        "case_sha256": sha256(args.cases),
        "binary_sha256": {
            name: sha256(binary)
            for name, binary in (
                ("candidate", args.candidate),
                ("reference", args.reference),
                ("baseline", args.baseline),
            )
            if binary
        },
        "table_directory": str(args.tables),
        "manifest_sha256": sha256(args.manifest) if args.manifest else None,
    }


def batches(iterable, size):
    batch = []
    for item in iterable:
        batch.append(item)
        if len(batch) == size:
            yield batch
            batch = []
    if batch:
        yield batch


def run_probe(name, executable, args, batch, batch_dir):
    command = [str(executable), "--tables", str(args.tables)]
    input_bytes = b"".join(
        (json.dumps(request, sort_keys=True, separators=(",", ":")) + "\n").encode()
        for request in batch
    )
    request_path = batch_dir / "requests.jsonl"
    if request_path.exists():
        if request_path.read_bytes() != input_bytes:
            raise RuntimeError(f"changed requests in {batch_dir.name}")
    else:
        write_atomic(request_path, input_bytes)
    invocation = {
        "command": command,
        "name": name,
        "requests_sha256": hashlib.sha256(input_bytes).hexdigest(),
        "timeout_seconds": args.timeout,
    }
    invocation_path = batch_dir / f"{name}.invocation.json"
    if invocation_path.exists():
        if json.loads(invocation_path.read_text()) != invocation:
            raise RuntimeError(f"changed {name} invocation in {batch_dir.name}")
    else:
        write_atomic(
            invocation_path,
            (json.dumps(invocation, indent=2, sort_keys=True) + "\n").encode(),
        )
    stdout_path = batch_dir / f"{name}.stdout.jsonl"
    stderr_path = batch_dir / f"{name}.stderr.log"
    status_path = batch_dir / f"{name}.status.json"
    reused = getattr(args, "resume", False) and stdout_path.exists() and stderr_path.exists()
    if reused:
        if not status_path.is_file():
            raise RuntimeError(f"unverified {name} transcript in {batch_dir.name}")
        stdout = stdout_path.read_bytes()
        status = json.loads(status_path.read_text())
        if status != {
            "exit_code": 0,
            "stdout_sha256": hashlib.sha256(stdout).hexdigest(),
            "stderr_sha256": sha256(stderr_path),
            "reply_count": len(batch),
        }:
            raise RuntimeError(f"changed {name} transcript in {batch_dir.name}")
    else:
        if stdout_path.exists() or stderr_path.exists():
            raise RuntimeError(f"incomplete {name} transcript in {batch_dir.name}")
        try:
            finished = subprocess.run(
                command,
                input=input_bytes,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=args.timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            write_atomic(stdout_path, exc.stdout or b"")
            write_atomic(stderr_path, exc.stderr or b"")
            raise RuntimeError(f"{name} timed out in {batch_dir.name}") from exc
        write_atomic(stdout_path, finished.stdout)
        write_atomic(stderr_path, finished.stderr)
        if finished.returncode:
            raise RuntimeError(f"{name} exited {finished.returncode} in {batch_dir.name}")
        stdout = finished.stdout
    lines = stdout.splitlines()
    if len(lines) != len(batch):
        raise RuntimeError(f"{name} returned {len(lines)} of {len(batch)} replies")
    responses = []
    for request, line in zip(batch, lines):
        reply = json.loads(line)
        if reply.get("version") != 1 or reply.get("id") != request["id"]:
            raise RuntimeError(f"{name} returned an out-of-order or invalid reply")
        if reply.get("status") != "ok":
            raise RuntimeError(f"{name} failed {request['id']}: {reply.get('error')}")
        if not isinstance(reply.get("wdl"), int) or reply["wdl"] not in range(-2, 3):
            raise RuntimeError(f"{name} omitted valid WDL for {request['id']}")
        if request["operation"] != "wdl":
            dtz = reply.get("dtz")
            if not isinstance(dtz, dict) or not isinstance(dtz.get("value"), int):
                raise RuntimeError(f"{name} omitted signed DTZ for {request['id']}")
        if request["operation"] == "root":
            if reply.get("root_status") not in ("moves", "checkmate", "stalemate"):
                raise RuntimeError(f"{name} omitted root status for {request['id']}")
            if not isinstance(reply.get("moves"), list):
                raise RuntimeError(f"{name} omitted root moves for {request['id']}")
            if name == "reference" and not isinstance(reply.get("mate_moves"), list):
                raise RuntimeError(f"reference omitted mate moves for {request['id']}")
        responses.append(reply)
    if not reused:
        write_atomic(
            status_path,
            (json.dumps({
                "exit_code": 0,
                "stdout_sha256": hashlib.sha256(stdout).hexdigest(),
                "stderr_sha256": sha256(stderr_path),
                "reply_count": len(batch),
            }, indent=2, sort_keys=True) + "\n").encode(),
        )
    return responses


def move_map(reply):
    moves = reply["moves"]
    result = {move["uci"]: move for move in moves}
    if len(result) != len(moves):
        raise ValueError("duplicate root move")
    return result


def valid_move_subset(values, moves):
    return (
        isinstance(values, list)
        and all(isinstance(value, str) for value in values)
        and len(set(values)) == len(values)
        and set(values) <= set(moves)
    )


def equivalent_root_choice(reference, selected, moves):
    """Name a root-policy difference with the same WDL and immediate DTZ."""
    move = moves.get(selected)
    if move is None:
        return None
    if reference["wdl"] == 0 and move["child_wdl"] == 0:
        return "draw_move"
    if reference["wdl"] == 2 and reference["dtz"]["value"] == 1:
        if move["child_wdl"] == 2 and (
            move["zeroing"] or selected in reference["mate_moves"]
        ):
            return "immediate_win"
    if reference["wdl"] == -2 and reference["dtz"]["value"] == -1:
        if move["child_wdl"] == -2 and move["zeroing"]:
            return "immediate_loss"
    return None


def packed_wdl(clock, dtz):
    if dtz == 0:
        return 0
    decisive = clock + abs(dtz) <= 100
    return (2 if decisive else 1) * (1 if dtz > 0 else -1)


def packed_move_distance(move, root_dtz, mate_moves):
    if root_dtz > 0 and move["uci"] in mate_moves:
        return 1
    if move["zeroing"]:
        return {-2: -1, -1: -101, 0: 0, 1: 101, 2: 1}[move["child_wdl"]]
    child_dtz = move["child_dtz"]["value"]
    distance = -child_dtz
    return distance + (1 if distance > 0 else -1 if distance < 0 else 0)


def packed_fields(expected, actual, label):
    errors = []
    if not isinstance(actual, dict):
        return [f"{label}_missing"]
    for field, value in expected.items():
        present = actual.get(field)
        if type(present) is not type(value) or present != value:
            errors.append(f"{label}_{field}")
    return errors


def compare_packed_root(reference, candidate, clock):
    """Check the actual public root array against independent reference probes."""
    errors = []
    try:
        legal = move_map(reference)
        root_dtz = reference["dtz"]["value"]
        mate_moves = set(reference["mate_moves"])
    except (KeyError, TypeError, ValueError):
        return ["reference_packed_inputs"]
    packed = candidate.get("packed_moves")
    count = candidate.get("packed_num_moves")
    if type(count) is not int or count != len(legal):
        errors.append("packed_move_count")
    if not isinstance(packed, list) or len(packed) != len(legal):
        errors.append("packed_move_array_length")
    if candidate.get("packed_unused_failed") is not True:
        errors.append("packed_unused_entries")
    if not isinstance(packed, list):
        return errors
    encoded = {}
    for item in packed:
        if not isinstance(item, dict) or not isinstance(item.get("uci"), str):
            errors.append("packed_move_encoding")
            continue
        uci = item["uci"]
        if uci in encoded:
            errors.append(f"packed_duplicate:{uci}")
        encoded[uci] = item
    if set(encoded) != set(legal):
        errors.append("packed_move_set")
    for uci in sorted(set(encoded) & set(legal)):
        move = legal[uci]
        try:
            distance = packed_move_distance(move, root_dtz, mate_moves)
            expected = {
                "uci": uci,
                "wdl": packed_wdl(clock, distance),
                "dtz": abs(distance),
                "ep": move["ep"],
            }
        except (KeyError, TypeError, ValueError):
            errors.append(f"reference_packed_move:{uci}")
            continue
        errors.extend(packed_fields(expected, encoded[uci], f"packed_move:{uci}"))
    selected = candidate.get("selected_move")
    root = candidate.get("packed_root")
    if reference["root_status"] == "moves":
        if selected not in legal:
            errors.append("packed_root_selected_move")
        else:
            expected = {
                "uci": selected,
                "wdl": packed_wdl(clock, root_dtz),
                "dtz": abs(root_dtz),
                "ep": legal[selected]["ep"],
            }
            errors.extend(packed_fields(expected, root, "packed_root"))
    elif root is not None:
        errors.append("packed_terminal_root")
    return errors


def compare(reference, candidate, operation, *, baseline=None):
    errors = []
    if reference["wdl"] != candidate["wdl"]:
        errors.append("wdl")
    if operation != "wdl" and reference["dtz"]["value"] != candidate["dtz"]["value"]:
        errors.append("dtz")
    if operation == "root":
        if reference["root_status"] != candidate["root_status"]:
            errors.append("root_status")
        try:
            reference_moves = move_map(reference)
            candidate_moves = move_map(candidate)
        except (KeyError, ValueError):
            errors.append("duplicate_or_missing_moves")
        else:
            if set(reference_moves) != set(candidate_moves):
                errors.append("move_set")
            for uci in sorted(reference_moves.keys() & candidate_moves.keys()):
                left, right = reference_moves[uci], candidate_moves[uci]
                if left["child_wdl"] != right["child_wdl"]:
                    errors.append(f"child_wdl:{uci}")
                if left["child_dtz"]["value"] != right["child_dtz"]["value"]:
                    errors.append(f"child_dtz:{uci}")
                if left["zeroing"] != right["zeroing"]:
                    errors.append(f"zeroing:{uci}")
            selected = candidate.get("selected_move")
            if reference_moves and selected not in reference_moves:
                errors.append("selected_move_illegal")
            elif reference_moves and reference_moves[selected]["child_wdl"] != max(
                move["child_wdl"] for move in reference_moves.values()
            ):
                errors.append("selected_move_wdl")
            elif not reference_moves and selected is not None:
                errors.append("terminal_selected_move")
            optimal = reference.get("optimal_moves")
            if not valid_move_subset(optimal, reference_moves):
                errors.append("reference_optimal_set")
            mate_moves = reference.get("mate_moves")
            if not valid_move_subset(mate_moves, reference_moves):
                errors.append("reference_mate_set")
            elif (
                valid_move_subset(optimal, reference_moves)
                and selected not in optimal
                and reference_moves
                and not equivalent_root_choice(reference, selected, reference_moves)
            ):
                errors.append("selected_move_not_optimal")
    if baseline is not None:
        if candidate["wdl"] != baseline["wdl"]:
            errors.append("baseline_wdl")
        if operation != "wdl" and candidate["dtz"]["value"] != baseline["dtz"]["value"]:
            errors.append("baseline_dtz")
        if operation == "root" and candidate["selected_move"] != baseline["selected_move"]:
            errors.append("baseline_selected_move")
    return errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--tables", type=Path, required=True)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--batch-size", type=int, default=4096)
    parser.add_argument("--timeout", type=float, default=1800)
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args(argv)
    try:
        evidence = preflight(args)
        recorded_invocation = {**evidence, "batch_size": args.batch_size,
                               "timeout": args.timeout}
        if args.resume:
            prior = json.loads((args.output_dir / "invocation.json").read_text())
            if prior != recorded_invocation:
                raise RuntimeError("resume invocation differs from the recorded run")
        else:
            args.output_dir.mkdir(parents=True, exist_ok=False)
            write_atomic(
                args.output_dir / "invocation.json",
                (json.dumps(recorded_invocation, indent=2, sort_keys=True) + "\n").encode(),
            )
        mismatches = []
        counts = Counter()
        equivalent_choices = Counter()
        for index, batch in enumerate(batches(requests_from(args.cases), args.batch_size)):
            batch_dir = args.output_dir / f"batch-{index:06d}"
            batch_dir.mkdir(exist_ok=args.resume)
            reference = run_probe("reference", args.reference, args, batch, batch_dir)
            candidate = run_probe("candidate", args.candidate, args, batch, batch_dir)
            baseline = (
                run_probe("baseline", args.baseline, args, batch, batch_dir)
                if args.baseline else [None] * len(batch)
            )
            for request, left, right, old in zip(batch, reference, candidate, baseline):
                counts[request["operation"]] += 1
                errors = compare(left, right, request["operation"], baseline=old)
                if request["operation"] == "root":
                    errors.extend(compare_packed_root(
                        left, right, int(request["fen"].split()[4])
                    ))
                if request["operation"] == "root" and not errors:
                    reference_moves = move_map(left)
                    selected = right.get("selected_move")
                    if selected not in left["optimal_moves"]:
                        reason = equivalent_root_choice(left, selected, reference_moves)
                        if reason:
                            equivalent_choices[reason] += 1
                if errors:
                    mismatches.append({"id": request["id"], "errors": errors,
                                       "reference": left, "candidate": right,
                                       "baseline": old})
        write_atomic(
            args.output_dir / "mismatches.jsonl",
            b"".join((json.dumps(item, sort_keys=True) + "\n").encode()
                     for item in mismatches),
        )
        summary = {"compared": sum(counts.values()), "operations": dict(counts),
                   "mismatches": len(mismatches), "evidence": evidence,
                   "outcome_equivalent_root_choices": dict(equivalent_choices)}
        if summary["compared"] != evidence["case_count"]:
            raise RuntimeError("comparison count differs from preflight")
        write_atomic(args.output_dir / "summary.json",
                     (json.dumps(summary, indent=2, sort_keys=True) + "\n").encode())
        print(json.dumps({key: summary[key] for key in ("compared", "mismatches")}))
        return 2 if mismatches else 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as exc:
        print(f"syzygy comparison failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
