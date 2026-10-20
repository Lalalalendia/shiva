# PDF-QA-LIBREOFFICE-13 — genuine Writer PDF export, synthetic DOCX ground truth

**Status: Draft implementation, CI on exact HEAD required.** This does not admit PDF/DOCX to the shipping Shiva worker or installed Akula.

## What is tested

The previous accepted PDF corpus used pdf-lib, ReportLab, and Chrome/Skia plus qpdf structural rewrites. None involved an office-document import/export pipeline. This test builds minimal deterministic OOXML DOCX inputs using only the Python standard library, opens each with **real LibreOffice Writer**, and exports via the official **pdf:writer_pdf_Export** filter. The PDF producer must identify LibreOffice, as independently reported by Poppler's pdfinfo. This proves a real office PDF writer was involved, not that Microsoft Word or Adobe Acrobat produced the file.

LibreOffice's export CLI is documented at https://help.libreoffice.org/latest/en-US/text/shared/guide/pdf_params.html.

## Profiles and ground truth

- writer-1-page.pdf: one A4 page, four known lines.
- writer-6-pages.pdf: explicit DOCX page breaks, 6 A4 pages, four known lines per page.
- writer-unicode-2-pages.pdf: two pages containing Cyrillic; four known lines per page.

Each page has unique LOQA, LOBODY, LOTAIL markers and one ordinary repeated body line. **9 source pages and 36 expected source text lines.**

The CI-only generator:
1. Creates minimal DOCX in a fresh temporary location, without user content or remote objects.
2. Launches LibreOffice headless with a separate temporary profile and bounded subprocess timeout.
3. Validates PDF header/size, LibreOffice Producer, exact page count via Poppler pdfinfo, and four source-derived lines on each corresponding page through Poppler pdftotext -layout.
4. Emits source DOCX and exported PDF SHA-256 plus LibreOffice version **per run**. No claim of bit-for-bit reproducibility across Office versions.
5. Refuses preexisting corpus paths, oversized output, renderer failure, and missing lines.

The Rust qa-external-pdf integration separately opens the generated PDF in the actual Shiva Transformer; it asserts page counts, unique per-page marker multiplicity, source order of **all 36 original lines**, and Cyrillic. CI must emit:
- SHIVA_LO_CORPUS=pass;profiles=3;pages=9
- SHIVA_LO_FIDELITY=pass;profiles=3;pages=9;source_lines=36

Any discrepancy must fail CI, not be silently ignored.

## Evidence and nonclaims

The local prototype (LibreOffice 25.2.3.2, Poppler pdfinfo/pdftotext) successfully exported 1+6+2 pages, verified original text order and Cyrillic, and rendered both Unicode PDF pages. This **does not** establish Shiva Rust behavior; exact-HEAD GitHub CI is required before merge.

The documents are authored by this test as synthetic DOCX, **not** genuinely externally sourced Microsoft Office or Adobe artifacts. A license-reviewed heterogeneous real-world corpus with immutable SHA-256 and provenance remains pending, along with signed immutable Windows installer/ACL and Akula File Gate authorization/revocation. Windows native acceptance is a separate follow-up; this remains Linux-only QA.
