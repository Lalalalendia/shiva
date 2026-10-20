# PDF-QA-QPDF-REWRITE-12 — metamorphic full-text QA

**Status: Draft / exact-HEAD CI pending.** This is only quality-assurance code. No PDF/DOCX format admission to shipping Shiva worker or installed Akula.

## Purpose

pdf-lib, ReportLab and Chrome/Skia originate source documents with independently checked expected page markers. qpdf is a structurally independent PDF *rewriter*, not a fourth content producer. This PR tests transformations that should preserve the document text. The qpdf manual describes structural PDF transformations including linearization and object-stream mode changes: https://qpdf.readthedocs.io/en/12.2/cli.html.

## Independent checks

- Seven source profiles: pdf-lib 1 and 10 pages; ReportLab 1 and 12 pages; Chrome 1, 8 and 2 Unicode pages. Total **35 original pages**.
- Three rewrites for every profile: linearization; generated compressed object streams; no object streams with uncompressed data. Total **21 rewritten PDFs / 105 rewritten pages**.
- Python stages each byte-distinct rewritten file in a new runner-temp QA directory. qpdf --check validates every output; qpdf --check-linearization validates linearized files. The complete Poppler `pdftotext -enc UTF-8 -layout` output after every rewrite must match the original exactly. Any tool error, warning exit status, blank text, size violation or symlink fails.
- A separate Rust QA integration test opens the original plus all 21 rewrites using the actual Shiva PDF Transformer. It requires original ordered unique page markers, all PDF page counts, exact complete extracted-document equality between original and rewritten PDF, and two Cyrillic lines on the original Chrome Unicode PDF.
- Source/rewritten files are bounded and ephemeral. The CC0 downloader is pinned by upstream Git blob and SHA-256, ReportLab is pinned by generator and wheel hashes, and Chrome output identity/version is logged per CI run. qpdf/Poppler are Ubuntu-package versions, not reproducible cross-run binary pins.

## Acceptance

[Shiva PR #27](https://github.com/Lalalalendia/shiva/pull/27) is already merged as `269d7080d8b7bf408fe66192169de271274bd301`. This experiment requires exact-HEAD Linux CI that genuinely runs qpdf, Poppler and the Rust parser. The final Rust log must emit `SHIVA_QPDF_METAMORPHIC_QA=pass;originals=7;rewrites=21;rewritten_pages=105`, and the producer script `SHIVA_QPDF_POPPLER_ORACLE=pass;originals=7;rewrites=21`. Do not skip individual failures. Every discrepancy is a real investigation candidate until shown otherwise.

## Release boundaries

Not a production release gate by itself. Does not test real Adobe/Microsoft/LibreOffice documents, hostile encrypted PDFs, immutable signed Windows installer ACL, worker permissions or Akula File Gate and revocation. The next stage is a genuinely heterogeneous licensed vendor corpus and differential comparison under pinned environments.
