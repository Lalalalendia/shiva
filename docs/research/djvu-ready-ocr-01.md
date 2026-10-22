# SHIVA-DJVU-READY-OCR-01 — ready pretrained models, no training

## Purpose

Find a reliable, optional OCR stack for Russian DjVu books. This PR is **QA tooling only**, and changes no shipping crate, worker feature flag, search index, or production format policy. The real book supplied in ChatGPT is **never put into GitHub or CI**.

## Test contract

1. Generate four original synthetic printed pages in Russian: normal print, code/mixed Latin, two-column reading order, low contrast. Ground truth is authored with the pages. No training/dataset fit occurs.
2. Encode to a four-page DjVu through external `cjb2` and `djvm` tools, no vendored GPL code. Independent reference: `ddjvu`.
3. Read via the **published MIT `djvu-rs==0.43.0` Python wheel**, render all pages at 300 DPI, compare per-page black text masks with `ddjvu`: sizes exactly equal and IoU >= 0.95, otherwise fail. The source DjVu SHA-256 must not change.
4. Fetch four **published** traineddata weights from frozen Git commits, verify byte size and exact Git blob hash before Tesseract is permitted to load them:
   - `tessdata_fast@87416418657359cb625c412a48b6e1d6d41c29bd`: `eng` 4,113,088 B (`bbef4675053b5b468cdb477053e28b1c698ba08e`); `rus` 3,861,738 B (`b146cb2263acbc6383f8e92ea0ce759537687bb8`).
   - `tessdata_best@e12c65a915945e4c28e237a9b52bc4a8f39a0cec`: `eng` 15,400,601 B (`176dc3220de7db34d3b3aecbfa42043a6038348b`); `rus` 15,301,764 B (`702e3c43c863cf3325002c9d2cc38045e01314a9`).
5. Run the **same Rust-decoded pixels** through Tesseract `rus+eng`, one process/page, one thread: system baseline, fast, best. Record CER/WER against independent synthetic truth, per-page time and OCR process RSS. Raw source images, model bytes and OCR full text are never uploaded. Any missing dependency, invalid hash, decoding mismatch or crashed OCR must fail the test.

## Known limits and decisions

- A synthetic CER/WER result **is not representative** of real book quality. It mainly proves the multiengine pipeline and verifies model pinning. The supplied user's 378-page DjVu remains private. Prior source-book baseline: system Tesseract `rus+eng` 1.274 s median, peak OCR RSS 73.7 MiB (12 pages, OCR cache agreement only); `djvu-rs` is still untested on this book.
- The previous local environment has system Tesseract but cannot install Cargo, the `djvu-rs` wheel, or official weights due to DNS restrictions. The independent CI runs against a completely **synthetic** corpus. Before production, repeat pixel fidelity and models on the same *local* real book (not uploaded to public CI), sample other books and establish reviewed real-scan CER/WER.
- The official Cyrillic PP-OCRv5 recognizer from `matyushkin/djvu-rs/docs/ocr-model-manifest.toml` has Apache-2.0 weights (`inference.onnx` 8,048,799 B, SHA-256 `5371ee1ddaa7983cc62d0818d99e982b6804638c85e4f960d59a574094e172e5`) but needs a pinned detector, dictionary and inference runtime. It is **not** part of this initial Tesseract comparison and must not be claimed tested. Its `ocr-onnx` driver is experimental.
- No loading of arbitrary downloaded weights without byte validation. All network access in the workflow is for installing QA-only tools or explicit model-fetch. No network in shipping Shiva, no self-training, no user-document uploads. Executing standalone DjVuLibre tools in ephemeral CI is not equivalent to embedding GPL code in Akula.
- Do not enable DjVu/PDF/DOCX production admission, mutate original files, or index OCR without a File Gate-authorized immutable snapshot, OS-contained worker, page/bbox/provenance, and revocation-aware index publication/read checks.

## Commands

```bash
python3 -m unittest -v scripts/ci/test_ready_ocr_lab.py
python3 scripts/ci/ready_ocr_lab.py --step generate --out /tmp/shiva-djvu-ready-ocr --require-djvu-rs
python3 scripts/ci/ready_ocr_lab.py --step fetch-models --out /tmp/shiva-djvu-ready-ocr
python3 scripts/ci/ready_ocr_lab.py --step measure --out /tmp/shiva-djvu-ready-ocr --models system,fast,best --require-djvu-rs
```

`fetch-models` is the only step that explicitly contacts a model hosting service; absent model files must not trigger a download during `measure`.

Notion: [SHIVA-DJVU-READY-OCR-01](https://app.notion.com/p/40032a84beec8122bbd8db246ed51261).