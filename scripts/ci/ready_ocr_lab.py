#!/usr/bin/env python3
"""QA-only, no-training OCR benchmark with original synthetic Cyrillic ground truth.

No user's DjVu is copied, uploaded, OCR'ed, or reported by this program.
Models are fetched only with an EXPLICIT `fetch-models` command. All network
artifacts have immutable Git commit + exact Git blob SHA-1 + byte-size receipts.
Installed app and shipping Shiva never import this module.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import random
import re
import shutil
import statistics
import subprocess
import tempfile
import time
import unicodedata
from pathlib import Path
from urllib.request import urlopen

from PIL import Image, ImageChops, ImageDraw, ImageFont

MODEL_SOURCES = {
    "fast": {
        "repo": "tessdata_fast", "commit": "87416418657359cb625c412a48b6e1d6d41c29bd",
        "eng": (4113088, "bbef4675053b5b468cdb477053e28b1c698ba08e"),
        "rus": (3861738, "b146cb2263acbc6383f8e92ea0ce759537687bb8"),
    },
    "best": {
        "repo": "tessdata_best", "commit": "e12c65a915945e4c28e237a9b52bc4a8f39a0cec",
        "eng": (15400601, "176dc3220de7db34d3b3aecbfa42043a6038348b"),
        "rus": (15301764, "702e3c43c863cf3325002c9d2cc38045e01314a9"),
    },
}

# Synthetic, authored for this benchmark. No copyrighted page text is used.
SAMPLES = [
    {"name": "russian_clean", "columns": [[
        "Лаборатория распознавания русских книг",
        "Обычный печатный текст следует читать без пропусков.",
        "Проверяем страницы, строки и порядок слов.",
        "Количество страниц указано в заголовке документа.",
        "Обработка изображений требует аккуратной проверки.",
        "Модели уже обучены, мы не изменяем их веса.",
    ]]},
    {"name": "russian_with_latin", "columns": [[
        "Заметки о коде Rust и Python 3.13",
        "Проверка памяти: RSS 64 MB, CPU 2 threads.",
        "Страница номер 42 открывается по точной ссылке.",
        "SHA256 проверяет целостность файла после чтения.",
        "API возвращает UTF-8 текст без подмены знаков.",
        "Файл config.json содержит идентификатор book_001.",
    ]]},
    {"name": "two_columns", "columns": [
        ["Левая колонка", "Старинная печатная книга", "Текст разбит на два столбца.",
         "Сначала прочитать левый столбец.", "Затем перейти к правому столбцу."],
        ["Правая колонка", "Важна правильная очерёдность.", "Каждая строка имеет свои координаты.",
         "Поиск должен помнить исходную страницу.", "Оригинал документа неизменен."],
    ]},
    {"name": "low_contrast", "columns": [[
        "Качество бумаги неодинаково",
        "Буквы немного бледные, но читаются полностью.",
        "Шум, пятна и фон ухудшают распознавание.",
        "Готовые модели должны пройти одну проверку.",
        "Нельзя выдавать ошибку распознавания за успех.",
    ]], "noise": True},
]


def git_blob_sha(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\x00" + data).hexdigest()


def verified_download(data: bytes, length: int, git_sha: str) -> str:
    if len(data) != length or git_blob_sha(data) != git_sha:
        raise ValueError("model byte size or immutable Git blob differs from pinned receipt")
    return hashlib.sha256(data).hexdigest()


def fetch_models(root: Path) -> None:
    model_root = root / "models"
    receipts = []
    for family, info in MODEL_SOURCES.items():
        directory = model_root / family
        directory.mkdir(parents=True, exist_ok=True)
        for lang in ("rus", "eng"):
            expected_size, expected_git_sha = info[lang]
            dest = directory / (lang + ".traineddata")
            if dest.exists():
                data = dest.read_bytes()
                sha256 = verified_download(data, expected_size, expected_git_sha)
                source = "verified-existing"
            else:
                url = ("https://raw.githubusercontent.com/tesseract-ocr/"
                       + info["repo"] + "/" + info["commit"] + "/" + lang + ".traineddata")
                with urlopen(url, timeout=90) as response:
                    data = response.read(expected_size + 1)
                sha256 = verified_download(data, expected_size, expected_git_sha)
                tmp = dest.with_suffix(".part")
                try:
                    tmp.write_bytes(data)
                    tmp.replace(dest)
                finally:
                    tmp.unlink(missing_ok=True)
                source = "downloaded-verified"
            receipts.append({"family": family, "language": lang, "bytes": expected_size,
                             "git_commit": info["commit"], "git_blob_sha1": expected_git_sha,
                             "sha256": sha256, "source": source})
            print(f"{family}/{lang}: {expected_size} bytes; SHA-256 {sha256}", flush=True)
    (root / "model_receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")


def font_at(size: int) -> ImageFont.FreeTypeFont:
    candidates = ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                  "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"]
    for candidate in candidates:
        if Path(candidate).is_file():
            return ImageFont.truetype(candidate, size)
    raise RuntimeError("known freely redistributable Cyrillic font is absent")


def run(args: list[str], timeout: float = 120, *, env=None) -> tuple[bytes, float, float | None]:
    start = time.perf_counter()
    try:
        import psutil
    except ImportError:
        psutil = None
    process = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               stdin=subprocess.DEVNULL, env=env)
    peak_rss = None
    # Tests use bounded output: OCR on one synthetic page at most 3 MiB of text.
    while True:
        try:
            stdout, stderr = process.communicate(timeout=0.05)
            break
        except subprocess.TimeoutExpired:
            if time.perf_counter() - start > timeout:
                process.kill()
                stdout, stderr = process.communicate(timeout=5)
                raise TimeoutError(f"timeout for {args[0]}")
            if psutil is not None:
                try:
                    current = psutil.Process(process.pid).memory_info().rss
                    peak_rss = max(peak_rss or 0, current)
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
    elapsed = time.perf_counter() - start
    if process.returncode:
        raise RuntimeError(f"{args[0]} exit={process.returncode}; stderr={stderr.decode('utf-8','replace')[-500:]}")
    if len(stdout) > 3 * 1024 * 1024:
        raise ValueError("unbounded subprocess response")
    return stdout, round(elapsed, 4), (round(peak_rss / 1048576, 1) if peak_rss else None)


def render_fixture(spec: dict, number: int, out: Path) -> dict:
    canvas = Image.new("L", (1700, 2300), color=255)
    draw = ImageDraw.Draw(canvas)
    font = font_at(31 if len(spec["columns"]) > 1 else 45)
    text_lines = []
    for column, lines in enumerate(spec["columns"]):
        x = 85 + column * 845
        y = 220
        for line in lines:
            assert draw.textbbox((0, 0), line, font=font)[2] < (800 if len(spec["columns"]) > 1 else 1550), line
            draw.text((x, y), line, font=font, fill=102 if spec.get("noise") else 0)
            y += 95
            text_lines.append(line)
    if spec.get("noise"):
        rng = random.Random(4200 + number)
        for _ in range(3000):
            draw.point((rng.randrange(70, 1650), rng.randrange(160, 1150)), fill=190)
    png = out / f"page_{number:04d}.png"
    pbm = out / f"page_{number:04d}.pbm"
    canvas.save(png)
    # DjVuLibre JB2 requires bilevel PBM. The OCR PNG is the native source;
    # DjVu source parity is checked against the encoded/decoded bitmap later.
    canvas.point(lambda v: 0 if v < 180 else 255).convert("1").save(pbm)
    return {"page": number, "name": spec["name"], "truth": " ".join(text_lines),
            "png": png.name, "pbm": pbm.name, "noise": bool(spec.get("noise")),
            "sha256_pbm": hashlib.sha256(pbm.read_bytes()).hexdigest()}


def make_fixture(root: Path, require_djvu: bool = False) -> None:
    out = root / "fixture"
    out.mkdir(parents=True, exist_ok=True)
    pages = [render_fixture(spec, idx, out) for idx, spec in enumerate(SAMPLES, 1)]
    have_tools = all(shutil.which(name) for name in ("cjb2", "djvm", "ddjvu"))
    if require_djvu and not have_tools:
        raise RuntimeError("independent DjVuLibre tools required for parity")
    djvu_generated = False
    if have_tools:
        component_paths = []
        for item in pages:
            filename = out / f"source_{item['page']:04d}.djvu"
            run(["cjb2", "-dpi", "300", str(out / item["pbm"]), str(filename)])
            component_paths.append(str(filename))
        book = out / "synthetic_book.djvu"
        run(["djvm", "-c", str(book), *component_paths])
        djvu_generated = True
    fixture = {"source": "project-authored synthetic Cyrillic; NOT the user's book",
               "license": "CC0-1.0 (synthetic authored corpus)", "dpi": 300,
               "book_generated": djvu_generated, "pages": pages}
    (out / "truth.json").write_text(json.dumps(fixture, ensure_ascii=False, indent=2), encoding="utf-8")
    print("Synthetic pages:", len(pages), "generated DjVu:", djvu_generated)


def norm(text: str) -> str:
    return re.sub(r"\s+", " ", unicodedata.normalize("NFC", text).casefold()).strip()


def levenshtein(a: list, b: list) -> int:
    # Two-row edit distance: O(min(n,m)) space.
    if len(b) > len(a):
        a, b = b, a
    previous = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        current = [i]
        for j, cb in enumerate(b, 1):
            current.append(min(current[j-1] + 1, previous[j] + 1,
                               previous[j-1] + (ca != cb)))
        previous = current
    return previous[-1]


def quality(truth: str, predicted: str) -> dict:
    a, b = norm(truth), norm(predicted)
    return {"cer": round(levenshtein(list(a), list(b)) / max(1, len(a)), 6),
            "wer": round(levenshtein(a.split(), b.split()) / max(1, len(a.split())), 6),
            "truth_characters": len(a), "recognized_characters": len(b)}


def check_djvu(root: Path, pages: list[dict], *, require: bool, decoded: Path) -> dict:
    book = root / "fixture" / "synthetic_book.djvu"
    if not book.is_file():
        if require:
            raise RuntimeError("synthetic DjVu missing: cannot claim Rust decode parity")
        return {"status": "not_run_missing_generated_DjVu"}
    try:
        import djvu_rs
    except ImportError:
        if require:
            raise RuntimeError("djvu-rs wheel not installed: cannot claim Rust decode parity")
        return {"status": "not_run_missing_djvu_rs"}
    original_sha = hashlib.sha256(book.read_bytes()).hexdigest()
    doc = djvu_rs.Document.open(str(book))
    if doc.page_count() != len(pages):
        raise AssertionError("djvu-rs decoded incorrect page count")
    stats = []
    for p in pages:
        idx = p["page"] - 1
        with tempfile.TemporaryDirectory(prefix="djvu-ocr-compare-") as tmp:
            reference = Path(tmp) / "reference.pnm"
            run(["ddjvu", f"-page={idx+1}", "-format=pnm", "-scale=100", str(book), str(reference)])
            t0 = time.perf_counter()
            image = doc.page(idx).render(dpi=300).to_pil().convert("L")
            elapsed = round(time.perf_counter() - t0, 4)
            with Image.open(reference) as ref:
                ref = ref.convert("L")
                if image.size != ref.size:
                    raise AssertionError(f"page {idx+1}: djvu-rs has incorrect dimensions {image.size} vs {ref.size}")
                image.save(decoded / f"page_{idx+1:04d}.png")
                mask_rust = image.point(lambda v: 255 if v < 128 else 0).convert("1")
                mask_libre = ref.point(lambda v: 255 if v < 128 else 0).convert("1")
                h_inter = ImageChops.logical_and(mask_rust, mask_libre).histogram()[255]
                h_union = ImageChops.logical_or(mask_rust, mask_libre).histogram()[255]
                iou = h_inter / max(1, h_union)
                stats.append({"page": idx + 1, "pixels": list(image.size),
                              "djvu_rs_render_sec": elapsed, "black_mask_iou": round(iou, 6)})
                print(f"DjVu parity page {idx+1} black-mask IoU={iou:.4f}")
                if iou < 0.95:
                    raise AssertionError(f"page {idx+1}: Rust/DjVuLibre mask IoU < 0.95 ({iou:.4f})")
    if hashlib.sha256(book.read_bytes()).hexdigest() != original_sha:
        raise AssertionError("DjVu reader unexpectedly changed source bytes")
    return {"status": "ran", "version": "djvu-rs 0.43.0",
            "input_sha256": original_sha, "observations": stats}


def tesseract(page: Path, family: str, model_root: Path) -> tuple[str, float, float | None]:
    if not shutil.which("tesseract"):
        raise RuntimeError("Tesseract executable absent")
    env = os.environ.copy()
    env["OMP_THREAD_LIMIT"] = "1"
    if family != "system":
        directory = model_root / family
        for name in ("rus.traineddata", "eng.traineddata"):
            if not (directory / name).is_file():
                raise RuntimeError(f"not installed: {directory/name}")
        info = MODEL_SOURCES[family]
        for lang in ("rus", "eng"):
            verified_download((directory / f"{lang}.traineddata").read_bytes(), *info[lang])
        env["TESSDATA_PREFIX"] = str(directory.absolute())
    data, seconds, rss = run(["tesseract", str(page), "stdout", "-l", "rus+eng",
                              "--oem", "1", "--psm", "3", "--dpi", "300"], timeout=80, env=env)
    return data.decode("utf-8", "replace"), seconds, rss


def measure(root: Path, models: list[str], require_djvu: bool) -> None:
    fixture_file = root / "fixture" / "truth.json"
    if not fixture_file.is_file():
        raise RuntimeError("first run --step generate")
    fixture = json.loads(fixture_file.read_text(encoding="utf-8"))
    pages = fixture["pages"]
    samples = []
    with tempfile.TemporaryDirectory(prefix="shiva-synthetic-djvu-") as tmp:
        decoded_dir = Path(tmp)
        djvu = check_djvu(root, pages, require=require_djvu, decoded=decoded_dir)
        input_path = "Rust-decoded DjVu pages" if djvu["status"] == "ran" else "synthetic source PNG only"
        for item in pages:
            image_path = ((decoded_dir if djvu["status"] == "ran" else root / "fixture")
                          / item["png"])
            for family in models:
                predicted, secs, rss = tesseract(image_path, family, root / "models")
                evidence = quality(item["truth"], predicted)
                samples.append({"page": item["page"], "profile": item["name"],
                                "family": family, "time_sec": secs, "peak_rss_mb": rss, **evidence})
                print(f"OCR {family:<7s} page={item['page']} CER={evidence['cer']:.3f} WER={evidence['wer']:.3f} ({secs}s)")
    summaries = {}
    for family in models:
        subset = [sample for sample in samples if sample["family"] == family]
        summaries[family] = {"cer_mean": round(statistics.mean(s["cer"] for s in subset), 5),
                             "wer_mean": round(statistics.mean(s["wer"] for s in subset), 5),
                             "time_median_sec": round(statistics.median(s["time_sec"] for s in subset), 4),
                             "rss_peak_mb": max((s["peak_rss_mb"] or 0) for s in subset),
                             "pages": len(subset)}
    report = {"independent_ground_truth": True,
              "corpus": "project-authored synthetic Cyrillic / Latin page text, NOT a real book",
              "no_training": True, "ocr_input": input_path, "tesseract_model_families": models,
              "sources": MODEL_SOURCES, "djvu_rs": djvu,
              "scores": summaries, "observations": samples,
              "no_shipping_code_changed": True}
    (root / "result.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    lines = ["# Shiva: pretrained-only DjVu/OCR experiment", "",
             "Test corpus: **synthetic printed Russian**, 4 pages; **ground truth is known**.",
             "Do not extrapolate to real scanned books. No weights have been trained.", "",
             "| Profile | CER mean | WER mean | Median OCR (s) | Peak OCR RSS (MB) |",
             "|---|---:|---:|---:|---:|"]
    for name in models:
        s = summaries[name]
        lines.append(f"| {name} | {s['cer_mean']:.4f} | {s['wer_mean']:.4f} | {s['time_median_sec']:.3f} | {s['rss_peak_mb']:.1f} |")
    lines += ["", f"djvu-rs decoding: {djvu['status']}",
              f"Tesseract input: {input_path}", "",
              "Scores are normalized Unicode case-folded CER/WER vs independent authored text.",
              "Pages include mixed Latin code, 2-column order, noise. Reporting synthetic data only."]
    (root / "summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("REPORT:", root / "result.json", root / "summary.md")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--step", choices=["generate", "fetch-models", "measure"], required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--models", default="system,fast,best")
    parser.add_argument("--require-djvu-rs", action="store_true")
    opts = parser.parse_args()
    opts.out.mkdir(parents=True, exist_ok=True)
    if opts.step == "generate":
        make_fixture(opts.out, require_djvu=opts.require_djvu_rs)
    elif opts.step == "fetch-models":
        fetch_models(opts.out)
    else:
        families = opts.models.split(",")
        if not families or len(families) != len(set(families)) or set(families) - {"system", "fast", "best"}:
            parser.error("unknown or duplicate model family")
        measure(opts.out, families, require_djvu=opts.require_djvu_rs)


if __name__ == "__main__":
    main()