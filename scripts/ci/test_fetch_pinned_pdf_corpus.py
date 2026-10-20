"""Offline negative controls for the pinned PDF download trust boundary."""
import hashlib
import importlib.util
from pathlib import Path
import unittest

SCRIPT = Path(__file__).with_name("fetch-pinned-pdf-corpus.py")
spec = importlib.util.spec_from_file_location("shiva_pinned_pdf", SCRIPT)
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

PDF = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n"


def record(blob):
    header = f"blob {len(blob)}\0".encode("ascii")
    return {
        "name": "sample-unit-test.pdf",
        "bytes": len(blob),
        "git_blob_sha1": hashlib.sha1(header + blob).hexdigest(),
        "sha256": hashlib.sha256(blob).hexdigest(),
        "oracle": "reject",
    }


class PinnedPdfIntegrityTests(unittest.TestCase):
    def test_verified_file_accepted(self):
        self.assertEqual(module.checked_bytes(PDF, record(PDF)), hashlib.sha256(PDF).hexdigest())

    def test_same_size_modified_blob_rejected(self):
        altered = PDF.replace(b"/Catalog", b"/CatALog")
        self.assertEqual(len(altered), len(PDF))
        with self.assertRaises(ValueError):
            module.checked_bytes(altered, record(PDF))

    def test_correct_git_blob_but_wrong_sha256_rejected(self):
        modified = record(PDF)
        modified["sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            module.checked_bytes(PDF, modified)

    def test_oversized_download_rejected(self):
        with self.assertRaises(ValueError):
            module.checked_bytes(PDF + b"x", record(PDF))

    def test_manifest_fails_without_sha256(self):
        r = record(PDF)
        del r["sha256"]
        with self.assertRaises(ValueError):
            module.validate_manifest(self._manifest(r))

    def test_manifest_rejects_traversal(self):
        r = record(PDF)
        r["name"] = "../sample.pdf"
        with self.assertRaises(ValueError):
            module.validate_manifest(self._manifest(r))

    def test_manifest_rejects_wrong_upstream(self):
        manifest = self._manifest(record(PDF))
        manifest["origin"]["repository"] = "untrusted/repository"
        with self.assertRaises(ValueError):
            module.validate_manifest(manifest)

    @staticmethod
    def _manifest(entry):
        return {
            "schema": 1,
            "origin": {
                "repository": "tinytoolkit-org/pdf-sample-files",
                "commit": "4fd733c47ead5446b06372ab910cb78d1c7da55e",
                "license": "CC0-1.0",
            },
            "fixtures": [entry],
        }


if __name__ == "__main__":
    unittest.main()
