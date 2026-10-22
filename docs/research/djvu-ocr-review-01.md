# SHIVA-DJVU-OCR-REVIEW-01 — conservative diagram/text QA projection

**Scope:** QA-only policy experiment. No model training, no new OCR engine, no Akula indexing, no shipping parser or File Gate changes. No user-supplied DjVu/pages/OCR text uploaded to GitHub. The code processes Tesseract word TSV from a *synthetic* DjVu in GitHub Actions.

## Why

The private user's 378-page DjVu was fully rendered in SHIVA-DJVU-REAL-BOOK-01 (merged PR #38). Its page 256 contains a large Go-board illustration above real paragraphs. Tesseract can mistake the board for words: those words should not become confident search hits. Search must not silently drop the raw OCR or label an arbitrary page as `Complete`.

## QA-only algorithm

1. Parse bounded Tesseract TSV word records, preserving each token, 2D box, page/line identity and recognition confidence. Malformed/out-of-bounds words, invalid confidence, absurd image dimensions and excessive TSV lengths fail closed.
2. Find *candidate connected prose* from at least two geometrically coherent multi-word, wide textual lines. Adjoin nearby short final lines; do **not** classify tokens by confidence alone. Geometric thresholds are relative to page/line dimensions; they are a heuristic, not a general illustration detector.
3. Separate `body` candidates from `review` (any disconnected or unusually tall regions); prove a bijective partition of **all OCR lines**. Never delete the raw line/word objects. Any page with body candidates is still `Partial` because no independent transcription oracle proves completeness; otherwise `Needs review`. `Complete` is **never** produced by this QA projection.
4. Store only anonymous numeric aggregate receipts in the CI artifact. Do not publish images, TSV text, document bytes or OCR strings.

## Private real book: one page only

Local-only reproduction on page **256**, `djvu-rs 0.43.0` raster at 300 DPI → installed Tesseract `rus+eng` (official fast model hashes). **74 recognized words** in 26 lines. QA separation: **52 words in 8 connected body lines**; **22 words in 18 unverified lines** in the Go-board region. All 74 OCR words retained in memory; page status `Partial`. The 52 body words were in the visually identified lower paragraph block; no line from that block was quarantined, no figure words entered the proposed search channel. The original source file hash is unchanged. **No CER/WER claim** on the real page: no independently proofread transcript. This single-page result does not establish other-book generality.

## Author-owned synthetic figure regression

Generate one 1700×2300 page (300 DPI) with a 19×19 Go grid, circles, diagram coordinate labels and six original printed Russian sentences. Encode through independent DjVuLibre `cjb2`, decode via pinned pure-Rust `djvu-rs==0.43.0`, run installed Tesseract, produce the two-channel projection. Only the test fixture generator knows which pixel region contains the author-created prose; the classifier is not given that information.

Local direct-PNG preliminary result (before running the CI DjVu codec leg): **98 recognized words** total; 37 authored-paragraph words kept, 61 diagram-region words held for review; **0** figure-region words indexed. Raw CER against authored prose (including figure labels) **86.03%**, projected CER **0**; this drastic change is **specific to this deliberately artificial test** and must not be marketed as a real-book OCR accuracy claim. Status `Partial`.

Eight offline negative/positive unit tests cover source partition integrity, high-confidence figure labels, isolated short text, connected short paragraph endings, malformed TSV, empty page, and no false `Complete`.

## Acceptance and release blockers

- [ ] Exact-HEAD Linux GitHub Actions PASS with a *real synthetic DjVu encoded independently* and decoded by `djvu-rs`, plus Tesseract TSV and ground-truth metrics.
- [ ] 8/8 offline unit tests PASS; source text/image never leaves runner.
- [ ] Keep `Partial` on synthetic page with review region; never claim `Complete` from raw OCR without a proofread oracle.
- [ ] Do **not** connect QA projection to shipping index. Current worker-client rejects `Partial`; production would need an explicit safe staging/review policy with rechecked user permissions.
- [ ] Production DjVu blocked: current trusted worker accepts only 2 MiB (private book 5.58 MB), seccomp/LPAC worker needs separately accepted byte/CPU/RSS/model-handling changes, and Akula needs File Gate immutable snapshot, per-page locators and revocation-safe index updates.

Previous accepted work: [PR #33](https://github.com/Lalalalendia/shiva/pull/33), [PR #35](https://github.com/Lalalalendia/shiva/pull/35), [PR #38](https://github.com/Lalalalendia/shiva/pull/38). [Notion evidence](https://app.notion.com/p/40032a84beec8122bbd8db246ed51261).