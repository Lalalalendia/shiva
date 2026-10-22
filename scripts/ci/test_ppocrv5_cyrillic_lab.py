#!/usr/bin/env python3
"""Offline tests: do not download, execute models, or read user's DjVu."""
import hashlib
from pathlib import Path
import tempfile
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ppocrv5_cyrillic_lab as lab
from ready_ocr_lab import quality


class PinnedPaddleTest(unittest.TestCase):
    def test_detect_byte_size_and_digest_corruption(self):
        data = b"published weights only"
        pin = ("unused", len(data), hashlib.sha256(data).hexdigest(), "fake.onnx")
        self.assertEqual(lab.check(data, pin), pin[2])
        with self.assertRaisesRegex(ValueError, "mismatch"):
            lab.check(data + b"x", pin)
        with self.assertRaisesRegex(ValueError, "mismatch"):
            lab.check(b"x" + data[1:], pin)

    def test_missing_weights_fail_closed_without_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(FileNotFoundError, "explicit fetch-models"):
                lab.model_paths(Path(tmp))

    def test_ctc_dictionary_never_falls_back_to_chinese(self):
        import yaml
        chars = [chr(0x410 + i) for i in range(850)]
        example = yaml.safe_dump({"PostProcess": {"character_dict": chars}},
                                 allow_unicode=True).encode("utf-8")
        self.assertEqual(lab.ctc_keys(example).decode("utf-8").splitlines(), chars)
        invalid = yaml.safe_dump({"PostProcess": {"character_dict": ["x"]}}).encode()
        with self.assertRaisesRegex(ValueError, "850"):
            lab.ctc_keys(invalid)

    def test_evaluate_against_authored_text_not_tesseract_cache(self):
        self.assertEqual(quality("Русский текст и Rust", "Русский текст и Rust")["cer"], 0)
        self.assertGreater(quality("Русский текст и Rust", "совсем другой текст")["cer"], 0)


if __name__ == "__main__":
    unittest.main()
