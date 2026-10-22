#!/usr/bin/env python3
"""Offline tests: no data downloads, no model training, no user's document."""
from __future__ import annotations
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

TARGET = Path(__file__).with_name('ready_ocr_lab.py')
spec = importlib.util.spec_from_file_location('ready_ocr_lab', TARGET)
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)


class ReadyOcrQa(unittest.TestCase):
    def test_blob_integrity_requires_both_size_and_content(self):
        data = b'official-model-check-bytes'
        digest = lab.git_blob_sha(data)
        self.assertEqual(lab.verified_download(data, len(data), digest),
                         hashlib.sha256(data).hexdigest())
        with self.assertRaises(ValueError):
            lab.verified_download(data, len(data) + 1, digest)
        with self.assertRaises(ValueError):
            lab.verified_download(data + b'bad', len(data) + 3, digest)
        with self.assertRaises(ValueError):
            lab.verified_download(data, len(data), '0' * 40)

    def test_ocr_cer_wer_metrics_are_about_known_truth(self):
        self.assertEqual(lab.quality('Привет, Мир!', 'привет,  МИР!')['cer'], 0)
        self.assertEqual(lab.quality('одно два три', 'одно два четыре')['wer'], 0.333333)
        self.assertGreater(lab.quality('документ', '')['cer'], 0.99)
        self.assertEqual(lab.levenshtein(['a','b','c'], ['a','x','c']), 1)
        self.assertEqual(lab.levenshtein([], []), 0)
        self.assertEqual(lab.levenshtein(['x'], []), 1)

    def test_synthetic_generator_has_no_user_book_and_rejects_missing_rust(self):
        with tempfile.TemporaryDirectory(prefix='shiva-ready-ocr-test-') as root:
            dest = Path(root)
            lab.make_fixture(dest, require_djvu=False)
            created = (dest/'fixture'/'truth.json').read_text(encoding='utf-8')
            self.assertIn('project-authored synthetic', created)
            self.assertNotIn('1.djvu', created)
            self.assertEqual(len(list((dest/'fixture').glob('page_*.png'))), 4)
            # No existing user-supplied files are accepted as implicit fixture.
            if not (dest/'fixture'/'synthetic_book.djvu').exists():
                with self.assertRaisesRegex(RuntimeError, 'synthetic DjVu missing'):
                    lab.check_djvu(dest, [], require=True, decoded=dest)


if __name__ == '__main__':
    unittest.main()