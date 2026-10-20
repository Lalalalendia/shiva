# PDF-QA-CHROMIUM-11 — browser-origin third-producer fidelity QA

**Status:** Draft, exact-HEAD CI required. No production admission of PDF/DOCX.

## Provenance and goal

pdf-lib and ReportLab do not exercise Chrome's HTML-to-PDF print pipeline. This QA-only experiment prints three deterministic local HTML inputs via Google Chrome on GitHub-hosted Ubuntu 24.04, which has Chrome preinstalled. The resulting /Producer field must identify Skia/PDF, not a relabeled ReportLab source.

PDF.js and PDFium are primarily reading/rendering engines. qpdf rewrites PDF structure without originating document contents. They are different future comparison/transformation axes, not third original generators.

The Python generator uses only local file:// input, no remote images/scripts/fonts or user files, ephemeral Chrome profile, per-profile timeout, and no overwrite of prior corpus directories. The QA-only --no-sandbox option does not constitute browser isolation or authorize hostile HTML inputs.

## Acceptance profiles

| File | Expected pages | Independent source oracle |
| --- | ---: | --- |
| chromium-1-page.pdf | 1 | CHQA/CHBODY/CHTAIL markers each once |
| chromium-8-pages.pdf | 8 | 24 distinct, ordered markers |
| chromium-unicode-2-pages.pdf | 2 | Six ordered markers plus two Cyrillic lines |

Rust tests require an actual Skia/PDF Producer, exact page count, and Shiva's ordered extraction of each generated marker and remaining per-page source text (nine ASCII producer lines and two Cyrillic lines). Any drop, duplicate, reordering, generation failure, or parsing rejection is a failing CI gate, never a skipped test.

Browser binary versions and fonts drift across runner revisions. Generator logs version, sizes and SHA-256 per run, but these are NOT immutable cross-run pins. This tests a third synthetic producer only, not actual arbitrary vendor PDFs, File Gate, revoked index permissions, signed Windows installer/ACL, production memory limits or release readiness.

Follow-up: pin a Chrome image digest for reproducibility; then add genuine Microsoft/Adobe/LibreOffice sources with provenance and separate legal rights before production admission.
