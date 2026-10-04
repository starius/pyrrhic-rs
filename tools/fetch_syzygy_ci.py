#!/usr/bin/env python3
"""Fetch the 774,944-byte, hash-pinned Syzygy CI set for local PGO training."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import tempfile
from urllib.request import urlopen


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "nix" / "syzygy-3-4-5.json"
BASE_URL = "https://tablebase.lichess.ovh/tables/standard"
MATERIALS = ("KQvK", "KPvK", "KRvK", "KRvKP", "KNNvKR")
FILENAMES = {f"{material}.{extension}" for material in MATERIALS
             for extension in ("rtbw", "rtbz")}
TOTAL_BYTES = 774_944


def entries_from_manifest(path: Path, require_pinned_size: bool = True) -> list[dict]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    entries = [item for item in raw if item["name"] in FILENAMES]
    if {item["name"] for item in entries} != FILENAMES or len(entries) != len(FILENAMES):
        raise ValueError("manifest lacks the exact ten Syzygy CI files")
    if require_pinned_size and sum(item["bytes"] for item in entries) != TOTAL_BYTES:
        raise ValueError("Syzygy CI byte count changed")
    for item in entries:
        if not isinstance(item["bytes"], int) or item["bytes"] < 1:
            raise ValueError(f"invalid size for {item['name']}")
        if not item["hash"].startswith("sha256-"):
            raise ValueError(f"invalid hash for {item['name']}")
        if len(base64.b64decode(item["hash"][7:], validate=True)) != 32:
            raise ValueError(f"invalid digest for {item['name']}")
    return sorted(entries, key=lambda item: item["name"])


def verify_file(path: Path, item: dict) -> None:
    expected = base64.b64decode(item["hash"][7:], validate=True)
    if path.stat().st_size != item["bytes"]:
        raise ValueError(f"wrong size for {path}")
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    if digest.digest() != expected:
        raise ValueError(f"wrong SHA-256 for {path}")


def verify_dataset(directory: Path, manifest: Path = MANIFEST) -> None:
    for item in entries_from_manifest(manifest, manifest == MANIFEST):
        verify_file(directory / item["name"], item)


def fetch_dataset(directory: Path, manifest: Path = MANIFEST,
                  base_url: str = BASE_URL) -> None:
    entries = entries_from_manifest(manifest, manifest == MANIFEST)
    directory.mkdir(parents=True, exist_ok=True)
    for item in entries:
        destination = directory / item["name"]
        if destination.exists():
            verify_file(destination, item)
            print(f"verified {item['name']}")
            continue
        suffix = "3-4-5-wdl" if item["name"].endswith(".rtbw") else "3-4-5-dtz"
        url = f"{base_url.rstrip('/')}/{suffix}/{item['name']}"
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=directory,
                                             prefix=f".{item['name']}.",
                                             delete=False) as output:
                temporary = Path(output.name)
                with urlopen(url, timeout=60) as response:
                    while chunk := response.read(65536):
                        output.write(chunk)
                        if output.tell() > item["bytes"]:
                            raise ValueError(f"oversized download for {item['name']}")
            verify_file(temporary, item)
            os.replace(temporary, destination)
            print(f"fetched {item['name']} ({item['bytes']} bytes)")
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    verify_dataset(directory, manifest)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--base-url", default=BASE_URL)
    args = parser.parse_args()
    fetch_dataset(args.out_dir, args.manifest, args.base_url)


if __name__ == "__main__":
    main()
