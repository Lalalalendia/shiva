# PDF-QA-REAL-INDESIGN-14 — verified public InDesign PDF source, discovery stage

**Status:** Discovery-only QA, CI diagnostics in progress. A green CI run is neither PDF admission nor fidelity proof. No shipping Shiva worker/Akula changes.

## Immutable source and license

The openly licensed [Open Preservation Foundation format-corpus](https://github.com/openpreserve/format-corpus) provides a genuine Adobe InDesign flyer PDF together with an original INDD and a copyright-waiver README.

- Fixed commit: 366f068cec399d0cdfd61fa473de3ab6dc858098
- PDF: desktop-publishing/InDesign/Neddy_Flyer_HeatherRyan.pdf
- File size: 59,106 bytes; Git blob SHA-1: a60bed5ae166a191919ff35abd08ac33cb325bb9
- Exact source SHA-256 from original CI #39478250994: 6a3c9444d4905c8896a717be7c30ee7d20b3c319eb2d3d469393a0f0e3529243 (now checked at download as an immutable pin)
- Original INDD blob SHA-1: 04c223cb1010570c72fe6bbc1b1b4caa2df1d701
- [Author README](https://github.com/openpreserve/format-corpus/blob/366f068cec399d0cdfd61fa473de3ab6dc858098/desktop-publishing/InDesign/Neddy_Flyer_README_HeatherRyan.md.rtf) identifies the PDF as InDesign output and waives copyright and related rights to the extent legally possible. The particular InDesign version is unknown.

## Discovery-only implementation

QA Python script downloads only the immutable fixed Git URL, checks HTTPS host and redirects, bounds to 256 KiB, requires exact size/header and Git blob SHA-1, and enforces independently observed SHA-256 from the initial GitHub Action, now pinned in the script. No PDF bytes are checked into the repo; everything stays on an ephemeral GitHub runner.

The initial Linux CI downloaded and verified the authentic file, then failed inside qpdf --check. The initial script hid qpdf output, so the precise reason cannot be inferred. The updated CI logs bounded qpdf diagnostics and distinguishes exit 0 (clean), exit 3 (warning/recovery, **never** strict admission), and any other nonzero exit (hard failure). It then attempts pdfinfo and pdftotext -layout as discovery-only metadata and candidate page-text extraction. No arbitrary user inputs or authorization changes. It deliberately prints SHIVA_IND_DISCOVERY_ONLY=pass;admission=not_proven, not a format-readiness pass.

## Active source-derived Shiva semantic fidelity gate

The source PDF itself is a **one-page** InDesign CS 3.0 / Adobe PDF Library 6.0 export, using three different embedded /ToUnicode maps and nine ordered page content streams. The QA investigation decompressed those original streams and decoded their text with the per-font Unicode maps; fixed text fragments include:

1. The Denver
2. Puppet Guild
3. Presents:
4. Neddy Buys
5. a New Pillow
6. Monday, July 10th
7. SML Coffee
8. The Dream Job!

The new QA-only Rust test `lib/tests/pdf_indesign_corpus.rs` requires these phrases **once each and in source order**, a one-page PDF, and no replacement glyphs or phantom List from the Shiva PDF transformer. The test will fail on missing, duplicated or scrambled text, rather than treating an apparently parseable file as Complete. The test is independent of the qpdf/Poppler source inspection and runs on the exact fixed source downloaded to the runner; no arbitrary PDF path is accepted.

[Exact-HEAD CI #39581587889](https://github.com/Lalalalendia/shiva/actions/runs/39581587889) is QUEUED. Any failure remains a blocker pending diagnosis. Even a green run is **QA-only** and does not permit production worker/Akula format admission.

## Future acceptance gate

1. **Completed in code, requires re-run:** SHA-256 obtained from initial CI and enforced with the Git blob hash; verify pinned receipt on the updated exact-HEAD CI.
2. Run the fixed 8-phrase source-derived text/order oracle through real Shiva and compare against independent Poppler; if the document is incomplete or qpdf emits warnings, keep a Needs-review classification. Record immutable Poppler full-text hash once observed.
3. **Implemented (not yet accepted):** Rust test using real Shiva Transformer asserts page count and eight source-derived phrases with cardinality/order. Fix discrepancies rather than silently accepting partial extraction.
4. Verify across Linux and Windows, and retain existing sandbox/file-authority prerequisites before production.
5. Expand beyond one genuine file with licensed documents from multiple Adobe/Word/LibreOffice pipelines.

**Not production-ready:** installed Akula and shipping Shiva worker still forbid PDF/DOCX pending File Gate permissions, signed immutable Windows installer/ACL and validation on genuinely heterogeneous real documents.
