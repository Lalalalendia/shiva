# SHIVA-DJVU-CYRILLIC-PP-OCRV5-01 — independent pretrained Cyrillic OCR

Purpose: compare published Cyrillic PP-OCRv5 with previously measured Tesseract fast/best on exactly the same synthetic Rust-decoded DjVu pages, without training. This is a QA-only experiment, not a production extraction feature.

## Reproducibility and pinned weights

The three artifacts below use immutable revisions and expected byte sizes/SHA-256. Both recognition and CTC dictionary must match or the harness fails closed.

- PP-OCRv4 mobile text detector (DBNet): source SWHL/RapidOCR, revision 1cfba2e90fc938db55889873735088de210cc173; 4,745,517 bytes; SHA-256 d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9.
- Cyrillic PP-OCRv5 mobile recognizer (CTC): source PaddlePaddle/cyrillic_PP-OCRv5_mobile_rec_onnx, revision 2cef88145434beb8afa9dd82d77d799eb1ad7b29; 8,048,799 bytes; SHA-256 5371ee1ddaa7983cc62d0818d99e982b6804638c85e4f960d59a574094e172e5.
- Official config containing CTC dictionary: same PaddlePaddle revision; 6,991 bytes; SHA-256 5c76cc91fa98410178a09f498db10050d0ec1634a660053d3005ab7be581f501.

All model weights are published under Apache-2.0. SHA pins were taken from [djvu-rs OCR model manifest](https://github.com/matyushkin/djvu-rs/blob/main/docs/ocr-model-manifest.toml). All downloads happen only in an explicit fetch-models step. Missing files, an unexpected dictionary or SHA mismatch terminate the test; no weights live in the repository.

## Pipeline

1. Generate four authored synthetic Russian/Latin page images with independent ground truth. DjVuLibre produces a DjVu; the accepted Rust djvu-rs 0.43.0 reads it and verifies 4-page black-mask parity with DjVuLibre.
2. Explicitly fetch and verify Tesseract tessdata_fast, tessdata_best and three ONNX pipeline artifacts.
3. RapidOCR 3.9.2 uses local PP-OCRv4 DBNet detector plus Cyrillic PP-OCRv5 recognizer and dictionary, with ONNX Runtime 1.23.2 CPU, one thread, orientation classifier disabled. No external recognizer is substituted silently.
4. Compare recognized whole-page strings against authors' original independent text. Preserve reading-order penalty on a two-column page. Report CER, WER, line counts, measured runtime and process memory.
5. Output JSON and Markdown aggregate metrics only, no OCR text, image data, user book, or model bytes.

**Measurement warning:** RapidOCR model is loaded persistently, then its per-page inference time is measured warm. Tesseract executable starts once per page. These timings are not directly comparable. Python-process memory includes Python runtime and earlier decoding; it is not a pure PP-OCR model RSS measurement.

This is the *mixed PP-OCRv4 detector + PP-OCRv5 Cyrillic recognizer* pipeline, not a claim about the official full PP-OCRv5 end-to-end bundle.


## Layout-result gate (observed in exact CI before postprocessing)

The real pinned ONNX pipeline PASSed in CI #39790155300; all 4 samples recognized. Raw two-column page OCR has **CER 0.600, WER 0.756757**, while Tesseract fast/best have CER/WER 0 on that page. Because the recognizer returned 10 lines with total output text length equal to authored text length (285 characters), a large part may be ordering rather than missing characters. The four-page raw PP-OCR mean CER was 0.159259 / WER 0.226689; Tesseract fast 0.007408 / 0.01875, best 0.000926 / 0.0125. RapidOCR cold init 0.4333s, PP warmed per-page median 1.1706s and Python process peak RSS 250.3MB; compare memory/timings cautiously (different measurement boundaries).

A new QA-only, **no-training** attempt uses a conservative two-column geometric sort on RapidOCR returned line boxes: split on a large gap in left edges (>=25% page width), at least two text blocks in each column, and overlap in vertical spans; order each column from top to bottom and put left before right. Preserve original order for one column and malformed geometry. Always report *both raw and reordered* CER/WER. This is not assumed successful until the new exact-head workflow passes. A real page with mixed headings, images, or 3+ columns needs further validation and cannot be auto-marked Complete.


## Acceptance

- [ ] Pinned models loaded from exact byte hashes; no implicit downloads.
- [ ] Real ONNX inference produces text on at least 3 of 4 synthetic pages.
- [ ] All 4 pages have independent ground-truth CER/WER and proper decoder parity evidence.
- [ ] Exact-head GitHub Actions PASS with JSON evidence that contains no user file or OCR content.
- [ ] One follow-on local-only check of the provided 378-page book once the Rust decoder and models can run locally, without uploading the book anywhere.
- [ ] Real scanned-book fidelity is separately judged against proofread pages; synthetic accuracy must not be reported as real-book quality.

Out of scope: training, fine-tuning, new default Akula model dependencies, user source file uploads, shipping DjVu format admission. Production Shiva/Akula requires authorized File Gate byte snapshots, sandboxing and revocation-safe indexed citations.

Previous [PR #33 — accepted baseline](https://github.com/Lalalalendia/shiva/pull/33). [Notion Shiva OCR evidence](https://app.notion.com/p/40032a84beec8122bbd8db246ed51261).
