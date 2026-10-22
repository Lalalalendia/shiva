#!/usr/bin/env python3
"""Explicit local-only DjVu raster QA; never emits images, OCR text, or file paths."""
from __future__ import annotations
import argparse
import gc
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1048576), b""):
            h.update(block)
    return h.hexdigest()


def assert_source(path: Path, expected: str) -> None:
    if len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected):
        raise ValueError("expected SHA-256 must be 64 lowercase hex digits")
    if not path.is_file() or sha256(path) != expected:
        raise ValueError("source missing or SHA-256 mismatch")


def worker(o: argparse.Namespace) -> None:
    import djvu_rs
    import psutil
    assert_source(o.source, o.expected_sha256)
    doc = djvu_rs.Document.open(str(o.source))
    if doc.page_count() != o.expected_pages:
        raise ValueError("DjVu page count differs from expected")
    process = psutil.Process()
    for index in range(o.start, o.end):
        t0 = time.perf_counter()
        page = doc.page(index)
        if not page.dpi or page.dpi <= 0:
            raise ValueError("invalid DjVu source DPI")
        width = math.ceil(page.width * o.dpi / page.dpi)
        height = math.ceil(page.height * o.dpi / page.dpi)
        if width <= 0 or height <= 0 or width * height > o.max_pixels:
            raise ValueError("page would exceed pixel budget")
        image = page.render(dpi=o.dpi).to_pil()
        if image.width <= 0 or image.height <= 0 or image.width * image.height > o.max_pixels:
            raise ValueError("actual page exceeds pixel budget")
        gray = image.convert("L")
        hist = gray.histogram()
        seconds = round(time.perf_counter() - t0, 4)
        if seconds > o.max_page_sec:
            raise RuntimeError("page exceeded elapsed-time budget")
        print(json.dumps({
            "page": index + 1, "pixels": [image.width, image.height],
            "render_seconds": seconds, "nonwhite_pixels": sum(hist[:255]),
            "dark_pixels": sum(hist[:128]),
            "rss_mib": round(process.memory_info().rss / 1048576, 1),
        }, separators=(",", ":")), flush=True)
        del image, gray, hist, page
        gc.collect()
    assert_source(o.source, o.expected_sha256)


def run_audit(o: argparse.Namespace) -> dict:
    assert_source(o.source, o.expected_sha256)
    if not 1 <= o.expected_pages <= 100000:
        raise ValueError("invalid expected_pages")
    if not 72 <= o.dpi <= 600:
        raise ValueError("invalid dpi")
    if not 1 <= o.batch_pages <= 50:
        raise ValueError("invalid batch_pages")
    if o.out is None or not o.out.is_absolute() or o.out.resolve() == o.source.resolve():
        raise ValueError("refusing missing, relative, or source-overwriting output")
    records = []
    began = time.perf_counter()
    for start in range(0, o.expected_pages, o.batch_pages):
        end = min(start + o.batch_pages, o.expected_pages)
        command = [
            sys.executable, str(Path(__file__).resolve()), "--worker",
            "--source", str(o.source), "--expected-sha256", o.expected_sha256,
            "--expected-pages", str(o.expected_pages),
            "--start", str(start), "--end", str(end),
            "--dpi", str(o.dpi), "--max-pixels", str(o.max_pixels),
            "--max-page-sec", str(o.max_page_sec),
        ]
        try:
            run = subprocess.run(command, capture_output=True, check=False,
                                 timeout=(end-start)*o.max_page_sec+45)
        except subprocess.TimeoutExpired as exc:
            raise RuntimeError(f"pages {start+1}-{end}: worker deadline") from exc
        if run.returncode != 0 or len(run.stdout) > 100000:
            # Never expose child stderr or source paths in CI diagnostics.
            raise RuntimeError(f"pages {start+1}-{end}: private worker failed")
        try:
            batch = [json.loads(row) for row in run.stdout.decode("utf-8").splitlines()]
        except (UnicodeError, ValueError) as exc:
            raise RuntimeError("invalid worker evidence") from exc
        if [row.get("page") for row in batch] != list(range(start+1, end+1)):
            raise RuntimeError("incomplete or duplicated page batch")
        records.extend(batch)
        assert_source(o.source, o.expected_sha256)
        print(f"DJVU_LOCAL_BATCH_PASS={start+1}-{end}", flush=True)
    times = [row["render_seconds"] for row in records]
    report = {
        "kind": "qa-only-read-only-djvu-raster-check",
        "source_sha256": o.expected_sha256,
        "source_bytes": o.source.stat().st_size,
        "pages_expected": o.expected_pages,
        "pages_rendered": len(records),
        "dpi": o.dpi,
        "median_page_sec": round(statistics.median(times), 4),
        "p95_page_sec": round(sorted(times)[min(len(times)-1, int(.95*len(times)))], 4),
        "slowest_page_sec": max(times),
        "max_sampled_worker_rss_mib": max(row["rss_mib"] for row in records),
        "nonwhite_pages": sum(row["nonwhite_pixels"] > 0 for row in records),
        "total_wall_sec": round(time.perf_counter()-began, 2),
        "source_unchanged": sha256(o.source) == o.expected_sha256,
        "no_source_bytes_images_or_ocr_text_in_output": True,
        "pages": records,
    }
    o.out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=o.out.parent, prefix=".djvu-check-",
                                     delete=False, encoding="utf-8") as f:
        json.dump(report, f, ensure_ascii=False, indent=2)
        f.flush()
        os.fsync(f.fileno())
        temporary = f.name
    os.replace(temporary, o.out)
    print("DJVU_LOCAL_RENDER_PASS", json.dumps({
        k: v for k, v in report.items() if k != "pages"
    }), flush=True)
    return report


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--source", required=True, type=Path)
    p.add_argument("--expected-sha256", required=True)
    p.add_argument("--expected-pages", type=int, required=True)
    p.add_argument("--out", type=Path)
    p.add_argument("--dpi", type=int, default=300)
    p.add_argument("--batch-pages", type=int, default=20)
    p.add_argument("--max-page-sec", type=float, default=15)
    p.add_argument("--max-pixels", type=int, default=35_000_000)
    p.add_argument("--worker", action="store_true")
    p.add_argument("--start", type=int, default=0)
    p.add_argument("--end", type=int, default=0)
    args = p.parse_args()
    if args.worker:
        if not 0 <= args.start < args.end <= args.expected_pages:
            p.error("invalid worker range")
        worker(args)
    else:
        run_audit(args)


if __name__ == "__main__":
    main()
