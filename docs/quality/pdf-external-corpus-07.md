# PDF-QA-EXTERNAL-CORPUS-07 — pinned public PDF fidelity/rejection gate

Status: **Draft and subject to exact-HEAD Linux/Windows CI.** Predecessors [#20 strict /Contents](https://github.com/Lalalalendia/shiva/pull/20) and [#21 strict /Pages](https://github.com/Lalalalendia/shiva/pull/21) have merged. Canonical P2-MFORMAT task: [Notion](https://app.notion.com/p/3fe32a84beec8138b380f9e52c5a8c1c).

## Provenance and limits

Source: [tinytoolkit-org/pdf-sample-files](https://github.com/tinytoolkit-org/pdf-sample-files), generated with pdf-lib/@cantoo/pdf-lib. The author declares [CC0-1.0](https://github.com/tinytoolkit-org/pdf-sample-files/blob/4fd733c47ead5446b06372ab910cb78d1c7da55e/LICENSE) for all samples. **Immutable source commit** `4fd733c47ead5446b06372ab910cb78d1c7da55e`. The exact filename, declared byte size and Git blob SHA-1 of each file are tracked in `scripts/ci/pinned-pdf-corpus-manifest.json`. A bounded Python downloader verifies each file's PDF header, exact length and Git blob object SHA before it reaches the Rust QA test; it records SHA-256 as an additional fingerprint. No redirect to a non-`raw.githubusercontent.com` host, unexpected filename, symlinked output, larger-than-budget download or changed file is accepted.

The files are fetched only in ephemeral GitHub Actions runners. PDF bytes are **not vendored** into Shiva and are never read or downloaded by shipping shiva-worker/Akula. The test itself is behind the opt-in Cargo `qa-external-pdf` feature, not enabled by `all`/release.

**Important:** these are genuine third-party repository **PDF binaries** from a separate PDF generator, but still *machine-generated sample documents*, not heterogeneous customer or vendor publications. They are an independent integration gate; they do **not** establish broad real-world fidelity or safety.

## Exact oracles and expected status

| Pinned fixture | Source ground truth | Acceptance |
| --- | --- | --- |
| `sample-1-page.pdf` | 1 page with selectable text | Exactly one `Page 1 of 1` marker |
| `sample-10-pages.pdf` | 10 ordered text pages | Exactly one marker for every page, in original order |
| `sample-100-pages.pdf` | 100 ordered text pages | Exactly one marker for every page, in original order |
| `sample-blank-a4.pdf` | Valid, blank 1-page document | Parse without inventing any text |
| `sample-scanned.pdf` | Image-only 3-page scan | No fabricated searchable text; explicitly report `needs_ocr` |
| `sample-corrupted.pdf` | Truncated at 55%, invalid | `Transformer::parse` must return Err |
| `sample-password-protected.pdf` | Encrypted without supplied password | `Transformer::parse` must return Err |

A `Complete` searchable-text classification must never follow from the QA label `needs_ocr`; this experiment only prints a test diagnosis. Missing AcroForm fields, encrypted permission semantics, image-based OCR, attachments, complex typography and full authorial fidelity remain separate.

Each of the two dedicated CI jobs validates immutable downloaded inputs, then runs `pdf_external_corpus` against the exact pinned bytes on Ubuntu 24.04 or Windows Server 2022. A wrongly accepted damaged PDF or missing/reordered page marker fails the job; there is no permissive pass-through for incomplete positive documents.

## Follow-on tasks

1. Study the returned diagnostics from the exact-head external corpus gate and fix any positive-fidelity issues **without** downgrading the assertions.
2. Freeze final SHA-256 fingerprints after the first verified download run and retain versioned provenance in Notion.
3. Add *different-generator* audited public samples (PDF.js/PDFium/qpdf/veraPDF), respecting original license and fixed input bytes, with independent known-text/page oracles and separate reject/review classifications.
4. Add bounded structure-aware mutation fuzzing and an adversarial corpus. Neither library acceptance nor this CI grants permission to enable PDF in shipping worker; signed Windows install/ACL and Akula File Gate/revocation remain release blockers.
