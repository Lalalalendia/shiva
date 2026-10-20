# PDF-QA-TOTAL-BUDGET-02 — bounded cumulative PDF extraction

Status: **stacked Draft**, dependent on [Shiva ToUnicode PR #16](https://github.com/Lalalalendia/shiva/pull/16). The standalone library parser is still not an approved Akula extractor; Shiva worker continues to admit only text, Markdown and HTML.

Canonical task: [PDF-QA-TOTAL-BUDGET-02 in Notion](https://app.notion.com/p/3ff32a84beec81348bc9cf141456b6f3).

## First implementation slice

- The existing per-page 8-MiB decompression limit remains in effect.
- A single `PdfBudget` shared by all pages rejects cumulative uncompressed page content above 16 MiB with checked integer arithmetic.
- A cumulative decoded text limit of 8 MiB includes decoded glyph UTF-8 bytes and inserted separators; at most 100,000 emitted element structures and 200,000 decoded content operators are accepted. The parser rejects arrays nested more than 16 levels, and pages with more than 64 font entries.
- `Text` ownership is transferred using `std::mem::take`; paragraph/list vectors are extended in-place, avoiding clones of all previously decoded elements on each `Tm`.
- Raw `Object::String` and operand values are no longer written to debug logs. Only non-sensitive aggregate diagnostics should be recorded in the future.
- Every budget error aborts the complete library parse; there is no partial `Document` returned as if it were complete.

## Exact acceptance tests

`pdf::tests::total_budget_*` provides:
1. Checked budget overflow guard for text/decoded pages/elements.
2. **Five actual Flate-compressed PDF pages**, each below the per-page 8-MiB limit but together above the cumulative 16-MiB decoded-page allowance.
3. Two actual compressed content pages, each containing 5 MiB of extractable text, which must fail the 8-MiB cumulative Unicode-output ceiling.
4. 1,500 alternating `Tj` / `Tm` operations preserving exact count and order without `Vec<Element>` cloning.

The existing Type0 Identity-H / ToUnicode success and negative cases, PDF multi-stream cardinality, single-page Flate overrun rejection, and real offline PDF generation also remain required.

## Not a production claim

These are library-level counters; they do **not** prove a hard RSS limit, CPU or execution-time bound. Before adding PDF to the sandboxed worker, separately validate memory/time through a real restricted OS process, hostile/invalid PDF corpora, zero raw-document telemetry, immutable signed worker install, and Akula File Gate permission/revocation semantics. The existing 64-MiB input and 1024-page limits are admission bounds, not allocations, and parser-owned font/CMap memory may still require additional aggregated budgeting.

Do not modify installed Akula, publish a partial/incorrect index as Complete, or enable PDF in the worker as part of this PR.
