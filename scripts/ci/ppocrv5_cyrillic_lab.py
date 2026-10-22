#!/usr/bin/env python3
"""Pretrained-only Cyrillic PP-OCRv5 vs Tesseract on identical Rust-decoded DjVu.

QA only: synthetic authored corpus, never the user's book. No network access
outside the explicit fetch-models action, no training, no shipping imports.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import statistics
import time
from urllib.request import urlopen

import ready_ocr_lab as base

# Immutable publisher artifacts from djvu-rs 0.43.0 model manifest.
# The detection model is PP-OCRv4 DBNet, recognition is Cyrillic PP-OCRv5 CTC.
PINS = {
    "det": ("https://huggingface.co/SWHL/RapidOCR/resolve/1cfba2e90fc938db55889873735088de210cc173/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx",
            4745517, "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9", "det.onnx"),
    "rec": ("https://huggingface.co/PaddlePaddle/cyrillic_PP-OCRv5_mobile_rec_onnx/resolve/2cef88145434beb8afa9dd82d77d799eb1ad7b29/inference.onnx",
            8048799, "5371ee1ddaa7983cc62d0818d99e982b6804638c85e4f960d59a574094e172e5", "rec.onnx"),
    "config": ("https://huggingface.co/PaddlePaddle/cyrillic_PP-OCRv5_mobile_rec_onnx/resolve/2cef88145434beb8afa9dd82d77d799eb1ad7b29/inference.yml",
               6991, "5c76cc91fa98410178a09f498db10050d0ec1634a660053d3005ab7be581f501", "rec.yml"),
}


def check(data: bytes, pin: tuple) -> str:
    if len(data) != pin[1] or hashlib.sha256(data).hexdigest() != pin[2]:
        raise ValueError("pretrained model size/SHA-256 mismatch")
    return pin[2]


def ctc_keys(data: bytes) -> bytes:
    import yaml
    obj = yaml.safe_load(data.decode("utf-8"))
    chars = obj["PostProcess"]["character_dict"]
    if not isinstance(chars, list) or len(chars) != 850:
        raise ValueError("CTC dictionary must contain 850 published characters")
    if any(not isinstance(ch, str) or not ch or "\n" in ch for ch in chars):
        raise ValueError("invalid CTC character")
    if " " in chars or len(set(chars)) != len(chars):
        raise ValueError("duplicated characters or space in source dictionary")
    # RapidOCR inserts CTC blank at position 0 and a trailing space itself.
    return ("\n".join(chars) + "\n").encode("utf-8")


def model_paths(root: Path) -> dict:
    folder = root / "ppocr_models"
    paths = {}
    for name, pin in PINS.items():
        p = folder / pin[3]
        if not p.is_file():
            raise FileNotFoundError("pretrained model missing: explicit fetch-models required")
        check(p.read_bytes(), pin)
        paths[name] = p
    keys = folder / "rec_keys.txt"
    if not keys.is_file() or keys.read_bytes() != ctc_keys(paths["config"].read_bytes()):
        raise ValueError("verified Cyrillic CTC dictionary absent or changed")
    paths["keys"] = keys
    return paths


def fetch_models(root: Path):
    folder = root / "ppocr_models"
    folder.mkdir(parents=True, exist_ok=True)
    receipts = {}
    for name, pin in PINS.items():
        p = folder / pin[3]
        if p.is_file():
            sha = check(p.read_bytes(), pin)
        else:
            with urlopen(pin[0], timeout=90) as response:
                payload = response.read(pin[1] + 1)
            sha = check(payload, pin)
            tmp = p.with_suffix(p.suffix + ".part")
            try:
                tmp.write_bytes(payload)
                tmp.replace(p)
            finally:
                tmp.unlink(missing_ok=True)
        receipts[name] = {"bytes": pin[1], "sha256": sha, "license": "Apache-2.0"}
        print(f"Published {name} verified: {pin[1]} bytes, sha256={sha}", flush=True)
    dictionary = ctc_keys((folder / "rec.yml").read_bytes())
    (folder / "rec_keys.txt").write_bytes(dictionary)
    model_paths(root)
    receipts["ctc"] = {"source": "pinned official inference.yml", "characters": 850,
                       "sha256": hashlib.sha256(dictionary).hexdigest()}
    (root / "ppocr_receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")


def engine(root: Path):
    # All required external files must pass verification before model runtime
    # imports. Explicit local model paths prevent automatic model downloads.
    models = model_paths(root)
    from rapidocr import RapidOCR
    from rapidocr.utils.typings import EngineType, OCRVersion, ModelType, LangRec
    params = {
        "Global.use_det": True,
        "Global.use_cls": False,
        "Global.use_rec": True,
        "Global.use_preprocess_img": False,
        "Global.use_vertical_padding": False,
        "Global.text_score": 0.0,
        "Global.log_level": "warning",
        "EngineConfig.onnxruntime.intra_op_num_threads": 1,
        "EngineConfig.onnxruntime.inter_op_num_threads": 1,
        "Det.engine_type": EngineType.ONNXRUNTIME,
        "Det.ocr_version": OCRVersion.PPOCRV4,
        "Det.model_type": ModelType.MOBILE,
        "Det.model_path": str(models["det"]),
        "Det.limit_side_len": 2000,
        "Det.limit_type": "max",
        "Rec.engine_type": EngineType.ONNXRUNTIME,
        "Rec.ocr_version": OCRVersion.PPOCRV5,
        "Rec.model_type": ModelType.MOBILE,
        "Rec.lang_type": LangRec.CYRILLIC,
        "Rec.model_path": str(models["rec"]),
        "Rec.rec_keys_path": str(models["keys"]),
        "Rec.rec_batch_num": 6,
    }
    return RapidOCR(params=params)


def column_reading_order(texts, boxes, page_width: int):
    """Conservative layout-only reading order for two spatially separate columns.

    Detect two clusters of LEFT edges, not OCR contents or known fixture text.
    Require >=2 boxes on each side, a large horizontal gutter and overlapping
    vertical ranges. Otherwise return the original order untouched.
    """
    original = list(texts)
    if boxes is None or len(original) < 4 or len(boxes) != len(original) or page_width <= 0:
        return original, 1
    geometry = []
    for i, quad in enumerate(boxes):
        if len(quad) != 4:
            return original, 1
        try:
            xx = [float(p[0]) for p in quad]
            yy = [float(p[1]) for p in quad]
        except (IndexError, TypeError, ValueError):
            return original, 1
        geometry.append((min(xx), min(yy), max(yy), i))
    by_left = sorted(geometry)
    gaps = [(by_left[i + 1][0] - by_left[i][0], i)
            for i in range(len(by_left) - 1)]
    gap, split = max(gaps)
    if gap < page_width * 0.25:
        return original, 1
    left, right = by_left[:split + 1], by_left[split + 1:]
    if len(left) < 2 or len(right) < 2:
        return original, 1
    # Two columns must overlap in the vertical dimension, not be two
    # differently indented paragraphs on different sections of a page.
    if max(min(p[1] for p in left), min(p[1] for p in right)) >= min(
        max(p[2] for p in left), max(p[2] for p in right)
    ):
        return original, 1
    def top_to_bottom(group):
        return [original[p[3]] for p in sorted(group, key=lambda p: (p[1], p[0]))]
    return top_to_bottom(left) + top_to_bottom(right), 2


def measure(root: Path):
    import os
    import tempfile
    import psutil
    os.environ["OMP_NUM_THREADS"] = "1"
    os.environ["OPENBLAS_NUM_THREADS"] = "1"
    if not (root / "fixture" / "synthetic_book.djvu").is_file():
        raise RuntimeError("synthetic DjVu required; refusing an arbitrary or user file")
    fixture = json.loads((root / "fixture" / "truth.json").read_text(encoding="utf-8"))
    pages = fixture["pages"]
    if len(pages) != 4 or "synthetic" not in fixture.get("source", ""):
        raise RuntimeError("only the four authored synthetic pages are permitted")
    model_paths(root)
    with tempfile.TemporaryDirectory(prefix="shiva-ppocr-v5-only-") as scratch:
        images = Path(scratch)
        djvu = base.check_djvu(root, pages, require=True, decoded=images)
        if djvu["status"] != "ran" or len(djvu["observations"]) != 4:
            raise RuntimeError("missing actual djvu-rs pixel parity")
        start = time.perf_counter()
        ocr = engine(root)
        init_sec = round(time.perf_counter() - start, 4)
        pp = []
        tess = []
        peak_process_rss = psutil.Process().memory_info().rss
        for page in pages:
            src = images / page["png"]
            t0 = time.perf_counter()
            result = ocr(str(src))
            warm_time = round(time.perf_counter() - t0, 4)
            lines = list(result.txts or [])
            raw_score = base.quality(page["truth"], " ".join(lines))
            from PIL import Image
            with Image.open(src) as check_image:
                image_width = check_image.width
            corrected_lines, detected_columns = column_reading_order(
                lines, getattr(result, "boxes", None), image_width
            )
            score = base.quality(page["truth"], " ".join(corrected_lines))
            count = len(lines)
            peak_process_rss = max(peak_process_rss, psutil.Process().memory_info().rss)
            pp.append({"page": page["page"], "lines": count, "warm_sec": warm_time,
                       "columns_detected": detected_columns,
                       "raw_cer": raw_score["cer"], "raw_wer": raw_score["wer"],
                       **score})
            for fam in ("fast", "best"):
                text, secs, rss = base.tesseract(src, fam, root / "models")
                tess.append({"page": page["page"], "family": fam,
                             "cold_sec": secs, "rss_mb": rss, **base.quality(page["truth"], text)})
            print(f"page={page['page']} PP raw CER={raw_score['cer']:.3f} "
                  f"ordered CER={score['cer']:.3f} columns={detected_columns}, lines={count} "
                  f"fast CER={tess[-2]['cer']:.3f} best CER={tess[-1]['cer']:.3f}", flush=True)
        if sum(x["lines"] > 0 for x in pp) < 3:
            raise AssertionError("PP-OCRv5 failed to detect text on most test pages")
    summary = {}
    for family, subset, time_key in [
        ("fast", [x for x in tess if x["family"] == "fast"], "cold_sec"),
        ("best", [x for x in tess if x["family"] == "best"], "cold_sec"),
        ("ppocr-v5-cyrillic", pp, "warm_sec"),
    ]:
        summary[family] = {
            "pages": len(subset),
            "cer_mean": round(statistics.mean(x["cer"] for x in subset), 6),
            "wer_mean": round(statistics.mean(x["wer"] for x in subset), 6),
            "time_median_sec": round(statistics.median(x[time_key] for x in subset), 4),
            "timing_method": "persistent model, warm page" if family.startswith("ppocr") else "new OS process per page",
        }
    output = {
        "status": "executed", "no_training": True,
        "ppocr_raw_cer_mean": round(statistics.mean(x["raw_cer"] for x in pp), 6),
        "ppocr_raw_wer_mean": round(statistics.mean(x["raw_wer"] for x in pp), 6),
        "layout_method": "conservative two-column left-edge spatial clustering, no model changes",
        "synthetic_only": True, "no_user_book": True,
        "decoder": djvu,
        "pipeline": "PP-OCRv4 mobile DBNet detector + Cyrillic PP-OCRv5 CTC recognizer",
        "rapidocr_version": "3.9.2",
        "onnxruntime_version": "1.23.2",
        "pp_init_sec": init_sec,
        "pp_peak_python_process_rss_mb": round(peak_process_rss / 1048576, 1),
        "pp_memory_includes_python_and_decoder": True,
        "summary": summary, "tesseract": tess, "ppocr": pp,
        "notes": "Timing is not apples-to-apples: PP warm persistent vs Tesseract cold CLI; "
                 "two-column reading order affects CER/WER. Four synthetic pages only. "
                 "No images, OCR text or models are stored in uploaded report.",
    }
    (root / "ppocr_result.json").write_text(json.dumps(output, ensure_ascii=False, indent=2), encoding="utf-8")
    lines = ["# Shiva: pretrained Cyrillic PP-OCRv5 benchmark", "",
             "Four original synthetic DjVu pages only; known text ground truth.",
             "No training, no user's book. All inputs are Rust-decoded and parity-checked.", "",
             "| Engine | CER mean | WER mean | Median OCR (s) |",
             "|---|---:|---:|---:|"]
    for name, v in summary.items():
        lines.append(f"| {name} | {v['cer_mean']:.4f} | {v['wer_mean']:.4f} | {v['time_median_sec']:.3f} |")
    lines += ["", f"PP-OCR cold initialization: {init_sec:.3f} s",
              f"PP Python process RSS (includes interpreter and decoder): {output['pp_peak_python_process_rss_mb']:.1f} MB",
              "", "Not comparable latency: PP warmed once; Tesseract launched per page.",
              "PP raw mean CER: " + str(output["ppocr_raw_cer_mean"]),
              "PP raw mean WER: " + str(output["ppocr_raw_wer_mean"]),
              "Layout correction: conservative two-column spatial order from OCR boxes; model unchanged.",
              "No production DjVu admission, no real-book accuracy claim."]
    (root / "ppocr_summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--step", choices=("fetch-models", "measure"), required=True)
    cli.add_argument("--out", type=Path, required=True)
    args = cli.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    if args.step == "fetch-models":
        fetch_models(args.out)
    else:
        measure(args.out)


if __name__ == "__main__":
    main()
