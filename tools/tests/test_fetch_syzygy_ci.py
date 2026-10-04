import base64
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

TOOLS_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS_DIR))

from fetch_syzygy_ci import FILENAMES, fetch_dataset, verify_dataset  # noqa: E402


class FetchSyzygyCiTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "mirror"
        self.destination = self.root / "download"
        entries = []
        for name in sorted(FILENAMES):
            directory = self.source / ("3-4-5-wdl" if name.endswith(".rtbw")
                                       else "3-4-5-dtz")
            directory.mkdir(parents=True, exist_ok=True)
            payload = (name + "\n").encode()
            (directory / name).write_bytes(payload)
            digest = base64.b64encode(hashlib.sha256(payload).digest()).decode()
            entries.append({"name": name, "bytes": len(payload),
                            "hash": "sha256-" + digest})
        self.manifest = self.root / "manifest.json"
        self.manifest.write_text(json.dumps(entries), encoding="utf-8")

    def test_fetches_and_reuses_only_hash_verified_files(self):
        fetch_dataset(self.destination, self.manifest, self.source.as_uri())
        verify_dataset(self.destination, self.manifest)
        fetch_dataset(self.destination, self.manifest, self.source.as_uri())
        file = self.destination / sorted(FILENAMES)[0]
        file.write_bytes(b"x" * file.stat().st_size)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            fetch_dataset(self.destination, self.manifest, self.source.as_uri())

    def test_truncated_download_is_not_published(self):
        entries = json.loads(self.manifest.read_text(encoding="utf-8"))
        entries[0]["bytes"] += 1
        self.manifest.write_text(json.dumps(entries), encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "wrong size"):
            fetch_dataset(self.destination, self.manifest, self.source.as_uri())
        self.assertFalse((self.destination / entries[0]["name"]).exists())
        self.assertEqual(list(self.destination.glob(".*")), [])


if __name__ == "__main__":
    unittest.main()
