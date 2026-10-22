#!/usr/bin/env python3
"""Fail-closed tests of explicit local DjVu QA parameters."""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

file = Path(__file__).with_name("private_djvu_render_audit.py")
spec = importlib.util.spec_from_file_location("private_djvu_render_audit", file)
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)


class ReadOnlyDjvuQa(unittest.TestCase):
    def test_invalid_hash_rejected_before_open(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "dummy.djvu"
            path.write_bytes(b"not djvu")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                qa.assert_source(path, "0" * 64)

    def test_source_overwrite_and_bad_dpi_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "dummy.djvu"
            path.write_bytes(b"not djvu")
            args = argparse.Namespace(
                source=path, expected_sha256=hashlib.sha256(b"not djvu").hexdigest(),
                expected_pages=1, out=path, dpi=300, batch_pages=2
            )
            with self.assertRaisesRegex(ValueError, "source-overwriting"):
                qa.run_audit(args)
            args.out = Path(tmp) / "receipt.json"
            args.dpi = 999
            with self.assertRaisesRegex(ValueError, "invalid dpi"):
                qa.run_audit(args)


if __name__ == "__main__":
    unittest.main()
