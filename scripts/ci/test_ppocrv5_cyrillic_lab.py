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


    def test_two_columns_recover_top_to_bottom_per_column_without_training(self):
        labels = ["Л1", "П1", "Л2", "П2", "Л3", "П3"]
        def rectangle(x, y):
            return [[x, y], [x + 180, y], [x + 180, y + 30], [x, y + 30]]
        boxes = [rectangle(100 if i % 2 == 0 else 1030,
                           100 + (i // 2) * 100) for i in range(6)]
        restored, columns = lab.column_reading_order(labels, boxes, 1700)
        self.assertEqual(columns, 2)
        self.assertEqual(restored, ["Л1", "Л2", "Л3", "П1", "П2", "П3"])

    def test_single_column_and_sequential_sections_not_reordered(self):
        labels = ["Первая строка", "Вторая строка", "Третья строка", "Четвертая строка"]
        def rectangle(x, y):
            return [[x, y], [x + 180, y], [x + 180, y + 30], [x, y + 30]]
        boxes = [rectangle(100, 90 + i * 100) for i in range(4)]
        self.assertEqual(lab.column_reading_order(labels, boxes, 1700), (labels, 1))
        # Very different indents but no vertical overlap between sections.
        boxes = [rectangle(100, 100), rectangle(100, 200),
                 rectangle(1100, 500), rectangle(1100, 600)]
        self.assertEqual(lab.column_reading_order(labels, boxes, 1700), (labels, 1))

    def test_layout_missing_or_invalid_boxes_fail_closed(self):
        labels = ["один", "два", "три", "четыре"]
        self.assertEqual(lab.column_reading_order(labels, None, 1700), (labels, 1))
        self.assertEqual(lab.column_reading_order(labels, [], 1700), (labels, 1))


if __name__ == "__main__":
    unittest.main()
