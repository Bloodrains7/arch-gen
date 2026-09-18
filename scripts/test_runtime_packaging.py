import importlib.util
from pathlib import Path
import re
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("setup_python", ROOT / "scripts/setup-python-runtime.py")
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class PackagingTests(unittest.TestCase):
    def test_lock_has_only_exact_versions_and_sha256(self):
        lines = [line for line in (ROOT / "python-engine/requirements-win-x64.lock").read_text().splitlines()
                 if line.strip() and not line.startswith("#")]
        names = []
        for line in lines:
            match = re.fullmatch(r"([\w.-]+)==([\w.+-]+) --hash=sha256:([a-f0-9]{64})", line)
            self.assertIsNotNone(match, line)
            names.append(match[1].lower().replace("_", "-"))
        self.assertEqual(len(names), len(set(names)))
        self.assertGreater(len(names), 4)

    def test_archive_escape_is_rejected_before_extracting_any_file(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / "unsafe.zip"
            with zipfile.ZipFile(archive, "w") as bundle:
                bundle.writestr("safe.txt", "safe")
                bundle.writestr("../escaped.txt", "unsafe")
            with self.assertRaises(ValueError):
                setup.unpack(archive, root / "output")
            self.assertFalse((root / "escaped.txt").exists())
            self.assertFalse((root / "output/safe.txt").exists())

    def test_checksum_is_content_based(self):
        with tempfile.TemporaryDirectory() as folder:
            file = Path(folder) / "artifact"
            file.write_bytes(b"abc")
            self.assertEqual(setup.checksum(file), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
            file.write_bytes(b"tampered")
            self.assertNotEqual(setup.checksum(file), setup.PYTHON_SHA)


if __name__ == "__main__":
    unittest.main()
