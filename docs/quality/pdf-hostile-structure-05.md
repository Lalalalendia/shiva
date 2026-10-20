# PDF-QA-HOSTILE-STRUCTURE-05 — deterministic fail-closed input corpus

This is a security regression **experiment for the standalone Shiva PDF library**. The input corpus consists entirely of generated, in-memory files, not unreviewed third-party downloads. These tests do not admit PDF into `shiva-worker` and do not modify the installed Akula.

## Dependencies and scope

- [#16 — correct Unicode Type0/Identity-H/CMap](https://github.com/Lalalalendia/shiva/pull/16) (merged).
- [#17 — cumulative byte/text/operator limits](https://github.com/Lalalalendia/shiva/pull/17) (merged).
- [#18 — independent Linux process resource QA](https://github.com/Lalalalendia/shiva/pull/18) (merged).
- [#19 — synthetic PDF in Windows LPAC/Job](https://github.com/Lalalalendia/shiva/pull/19) (merged).

## Corpus matrix

| Case | Expected parser behavior | Purpose |
| --- | --- | --- |
| Valid Type1 baseline | A single exact token | Test that valid fixtures work |
| Valid PDF with empty `/Filter` list | Exact text retained | A strict decoder must not reject legal uncompressed streams |
| Type0 ToUnicode inverted bfrange | Error, not Complete | Reject invalid CID source ranges |
| Flate-compressed CMap above decoded 1 MiB | Error, not Complete | Enforce CMap decompression bound |
| 17+-deep nested text arrays | Explicit nesting-budget error | Stop recursion bombs |
| `Tf` operand is an integer, not a named font | Error, not Complete | No silent font fallback |
| Text uses undeclared font `F1` | Error, not Complete | No implicit encoding |
| Page `/Contents` references missing object | Error, not Complete | No silently empty index on broken page |
| Unsupported `/Contents` stream compression filter | Error, not Complete | Prevent raw-compressed-byte fallback |
| Declared FlateDecode with invalid compressed bytes | Error, not Complete | Never parse raw fallback bytes |
| `/Contents` array containing a non-reference | Error, not Complete | No silent partial page extraction |
| Valid blank PDF page with absent `/Contents` | Successful empty page | Strict checks do not reject legitimate blanks |
| Truncated PDF without valid xref/page tree | Error, not Complete | Invalid input should not appear indexable |

All fixtures are built from a valid PDF document/page/catalog, except the explicitly truncated case. **Current matrix: 3 positive controls and 10 fail-closed negatives.** They contain synthetic text only, no external resources or executable content. For each negative case, a returned `Ok(Document)` is a **test failure**. Panic or unbounded execution also fails CI. Assertions deliberately avoid printing the document's extracted text or bytes into CI logs.

Linux `Shiva offline PDF` runs `pdf::hostile_tests::` and continues to require ToUnicode, decompression, total-budget and generation regressions. A new Windows-2022 workflow runs the same corpus in a native Rust PDF library build.

## Outstanding release gates

This small adversarial matrix is **not a comprehensive hostile corpus** and is not a fuzzing result. Remaining: overlapping CMap mappings, multi-byte/non-BMP mappings, page-tree cycles, huge xref/objects/fonts and CMap global allocation, malformed encrypted PDFs, image/XObject decoders, real-world broken vendor PDFs, mutation fuzzing with sanitizers, and hard CPU/memory bounds in the actual installed process. Neither PDF nor DOCX should be enabled in the shipping worker until signed installation/ACL trust, File Gate/permission/revocation, and wider fidelity tests are satisfied.
