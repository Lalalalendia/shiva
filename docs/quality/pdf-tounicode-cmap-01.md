# PDF-QA-CMAP-01 — ToUnicode/CID extraction experiment

This is **library-only QA** for [Akula P2-MFORMAT](https://app.notion.com/p/3fe32a84beec8138b380f9e52c5a8c1c). It is NOT permission to enable PDF in the bounded Shiva worker or the installed Akula app.

## Problem

The previous Shiva PDF parser pinned `lopdf 0.32.0`, used `get_font_encoding()` and a string-only `decode_text` fallback, and did not resolve font-owned ToUnicode CMaps. Composite Type0 Identity-H text uses multi-byte character **codes**, not UTF-8 bytes; a CMap is required to convert these codes to searchable Unicode. Upstream lopdf's changelog records ToUnicode support in 0.34.0 and decompression-bomb limiting APIs in 0.44.x.

## Bounded experimental implementation

- Pin `lopdf = "=0.45.0"` with default features disabled (the new API has a minimum Rust version of 1.88).
- Use `LoadOptions::with_max_decompressed_size` to cap eagerly decoded xref/object streams at 8 MiB.
- Cap document input to 64 MiB and parsed pages to 1024; these are experiment admission thresholds, not global CPU/memory guarantees.
- Decode each page with `get_page_content_with_limit(page_id, 8 MiB)` and font CMaps with `get_font_encoding_with_limit(doc, 1 MiB)`.
- Composite Type0 fonts without a usable `UnicodeMapEncoding` fail, rather than silently indexing Latin fallback bytes.
- Unresolved glyph codes producing Unicode replacement U+FFFD fail. Missing text font fails. No guessed UTF-8 fallback.
- Preserve the existing Shiva `Document` structure and operator parsing; no opt-in PDF worker feature is enabled.

## Tests / acceptance

`cid_identity_h_fixture()` produces a minimal **actual PDF** with a Type0/Identity-H font and Unicode CMap containing both `beginbfchar` and `beginbfrange`. It maps glyph codes to the exact mixed Cyrillic/CJK text `Пр世丗`. The test requires exact single occurrence, no replacement characters. A second fixture omits ToUnicode and must fail closed. Existing two-stream page regression and offline PDF generation regressions must continue to pass.

### Important open gates

Passing these two tests does **not** establish general PDF correctness. Still needed: surrogate/non-BMP targets; >2-byte variable codes; missing/overlapping bfrange; content streams with images and special text operators; structured page locators; verified hostile corpus and resource limits; installed Windows ACL/signing/revoke; dedicated sandbox allowlist and Akula File Gate adapter. The standalone library `Document` output does not yet express `complete=false`, so failure propagates as an error rather than publishing a misleading partial document.
