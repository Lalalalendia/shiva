# PDF-QA-MULTI-PRODUCER-08 — independent ReportLab corpus

**Status:** QA-only Draft. Preceding [#22](https://github.com/Lalalalendia/shiva/pull/22) (CC0 pdf-lib source) was merged as `0470b0a72432244ed8da8c8cafc06a3da140cac7` after Linux+Windows exact-head pass.

## Why this exists

The seven external CC0 PDF files from #22 were **all produced by pdf-lib**. Passing them does not prove compatibility with files from a separate PDF producer. This test adds [ReportLab](https://pypi.org/project/reportlab/4.4.10/) (BSD, Python 3.12-supported) as an **independent PDF generator**, neither the Rust `lopdf` reader under test nor pdf-lib.

QA installer pins **ReportLab 4.4.10's universal wheel by SHA-256** (`5abc815746ae2bc44e7ff25db96814f921349ca814c992c7eac3c26029bf7c24`) from [published PyPI release metadata](https://pypi.org/project/reportlab/4.4.10/). Transitive Pillow 11.3.0 and charset-normalizer 3.4.3 are pinned by version but NOT hash-locked; they only run in GitHub-hosted test environments, not in shipping products.

## Generated PDF profiles and ground-truth invariants

The fixed generator `scripts/ci/generate-reportlab-pdf-corpus.py` does not read user input files, contains no document data and writes only to a new ephemeral QA directory. All texts are synthetic ASCII markers generated from known page numbers. ReportLab `invariant=1` suppresses ambient timestamps, and the tests print SHA-256 of each newly produced document.

| Profile | Structure | Exact oracle |
| --- | --- | --- |
| `reportlab-1-page.pdf` | One A4 page, uncompressed PDF content stream, three Type1 fonts | Exactly one `RLQA-001-OF-001` |
| `reportlab-12-pages.pdf` | Twelve A4/landscape Letter pages, compressed streams (ASCII85 + Flate), multiple fonts | One marker per page, exactly 12, ordered |
| `reportlab-rotated-3-pages.pdf` | Three pages rotated 0, 90, 180 degrees, alternating size and compressed streams | Exactly three ordered markers, and independent structural checks of `/Rotate` values |

Total **16 text pages**, each with exact expected marker cardinality and global order. No oracle relies only on a successful exit or `lopdf::get_pages()` length; the latter is used solely to sanity-check the rotation fixture, while Shiva's own strict validator is used for extraction.

An automated run on the local sandbox using **ReportLab 4.4.9** and `pdftotext` confirmed all 16 markers after fixing a previously off-page placement bug. Actual **ReportLab 4.4.10** CI succeeded on native Linux and Windows, yielding **exactly the same SHA-256** for all 3 profiles: `f7aa5fdd...` (1 page), `0cccac77...` (12 pages), `6a935e0e...` (3 rotated). These three **full 64-character hashes are now enforced** by the generator source itself; any file drift fails before Rust parsing. Fresh exact-HEAD CI after the fingerprint freeze remains required.

## Acceptance and release boundaries

- Exact HEAD Windows/Linux jobs must install ReportLab 4.4.10 with the hash check and pass the complete 16-marker semantic oracle plus rotation checks. Profile generation errors and dropped/reordered text fail CI.
- Re-run the existing SHA-pinned pdf-lib corpus independently when touched. A future qpdf/PDFBox corpus must have separate provenance and explicit known-text/page invariants, not a guessed correctness label.
- No altered production parser, installed Akula files, worker allowlist or File Gate. Neither #22 nor this work qualifies PDF/DOCX for production without hostile mutation fuzzing, heterogeneous real-world documents, immutable signed Windows executable/ACL and authorization/revocation.
