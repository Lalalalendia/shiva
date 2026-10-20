# PDF-QA-REAL-INDESIGN-14 — verified public InDesign PDF source, discovery stage

**Status:** Discovery-only QA. A green CI run is neither PDF admission nor fidelity proof. No shipping Shiva worker/Akula changes.

## Immutable source and license

The openly licensed [Open Preservation Foundation format-corpus](https://github.com/openpreserve/format-corpus) provides a genuine Adobe InDesign flyer PDF together with an original INDD and a copyright-waiver README.

- Fixed commit: 366f068cec399d0cdfd61fa473de3ab6dc858098
- PDF: desktop-publishing/InDesign/Neddy_Flyer_HeatherRyan.pdf
- File size: 59,106 bytes; Git blob SHA-1: a60bed5ae166a191919ff35abd08ac33cb325bb9
- Original INDD blob SHA-1: 04c223cb1010570c72fe6bbc1b1b4caa2df1d701
- [Author README](https://github.com/openpreserve/format-corpus/blob/366f068cec399d0cdfd61fa473de3ab6dc858098/desktop-publishing/InDesign/Neddy_Flyer_README_HeatherRyan.md.rtf) identifies the PDF as InDesign output and waives copyright and related rights to the extent legally possible. The particular InDesign version is unknown.

## Discovery-only implementation

QA Python script downloads only the immutable fixed Git URL, checks HTTPS host and redirects, bounds to 256 KiB, requires exact size/header and Git blob SHA-1, then prints an independent full SHA-256 to be pinned in a subsequent revision. No PDF bytes are checked into the repo; everything stays on an ephemeral GitHub runner.

A separate Linux CI invokes qpdf --check, pdfinfo and pdftotext -layout with command timeouts; it logs metadata, page count and candidate text markers. No arbitrary user inputs or authorization changes. It deliberately prints SHIVA_IND_DISCOVERY_ONLY=pass;admission=not_proven, not a format-readiness pass.

## Future acceptance gate

1. Obtain SHA-256 from actual discovery CI, pin and require it every time alongside the Git blob.
2. Validate exact page count and source per-page text/order against an independent reference, recording immutable text hashes.
3. Introduce a Rust test using real Shiva Transformer, asserting marker cardinality/order and full source fidelity or rejecting a document as Needs review. Never call partial extraction Complete.
4. Verify across Linux and Windows, and retain existing sandbox/file-authority prerequisites before production.
5. Expand beyond one genuine file with licensed documents from multiple Adobe/Word/LibreOffice pipelines.

**Not production-ready:** installed Akula and shipping Shiva worker still forbid PDF/DOCX pending File Gate permissions, signed immutable Windows installer/ACL and validation on genuinely heterogeneous real documents.
