# PDF-QA-SEMANTIC-MUTATIONS-09 — deterministic structural mutation gate

**State:** Draft QA experiment, not coverage-guided fuzzing, not hostile-production PDF admission. Predecessors: merged [#22 pdf-lib corpus](https://github.com/Lalalalendia/shiva/pull/22) and [#23 ReportLab independent generator](https://github.com/Lalalalendia/shiva/pull/23).

## Threat model

A library may accept a syntactically damaged PDF by recovering its xref, page tree or content stream, **but silently lose, duplicate or reorder extracted text pages**. Successful `Transformer::parse` must not become a trustworthy `Complete` result in that case. "Does not crash" is insufficient for indexing.

## Implemented narrow experiment

`lib/tests/pdf_semantic_mutations.rs` reads **only four known-good immutable QA files** already verified by pinned provenance scripts:

- independent CC0 pdf-lib source: 1-page and 10-page `sample-*.pdf`;
- independent ReportLab 4.4.10 source: 12-page mixed-size and 3-page rotated `reportlab-*.pdf`.

Mutations are in-memory only; no PDF is written back. The same process uses bounded maximum input size **2 MiB** per source. Enumerate the first up-to-three occurrences of PDF structural key names such as `/Kids`, `/Pages`, `/Page`, `/Parent`, `/Count`, `/Contents`, `/Resources`, `/MediaBox`, `/Filter`, `/Length`, `/Root`, `/Type`, `/Rotate`, `xref` and `trailer`. Each mutated byte changes **a structural identifier**, never intended page text. Also mutate numeric `/Count` values while preserving numeric syntax.

### Semantic oracle

Before mutations, every seed must itself parse with exactly one original per-page marker in original order. For each mutated document:

- `Err` → acceptable explicit rejection.
- `Ok(Document)` → **mandatory** exact once-and-in-order original markers for **every source page**; no partial success.
- panic, abort, dropped/reordered/duplicated markers, or unexpected input corpus shrinkage → **test failure**.

The test prints one bounded machine-readable summary with mutation count, rejected count and accepted-complete count, and no PDF payload or extracted text. Minimum `80` mutated cases guards against accidentally disabled coverage. No dynamic files or arbitrary network access.

### Native CI

[Linux and Windows workflows](https://github.com/Lalalalendia/shiva/blob/quality/pdf-semantic-mutation-20261020/.github/workflows/pdf-semantic-mutation.yml) independently obtain the CC0 file corpus pinned by commit/blob SHA-1/SHA-256, install SHA-256-verified ReportLab 4.4.10, regenerate its three SHA-256-pinned profiles and execute the same Rust test via **nondefault** Cargo feature `qa-external-pdf`. Runtime and memory are bounded by CI job scope and library parse budgets; **not** a substitute for OS sandbox fuzzing.

## Important limitations

This is a **small deterministic structural mutation campaign**, not libFuzzer/AFL++ coverage-guided fuzzing, arbitrary binary spraying, a malicious external corpus, native sanitizer coverage or proof of soundness. It tests markers and source order only; silently losing body text while preserving markers would still pass. Future campaigns need whole-text digests, differential text extraction, raw font/xref/XObject mutations, format-specific valid variants, issue minimization and a long-running sandboxed fuzz target. Any mutation failure must be reproduced and investigated rather than removing a case or allowing partial success.

PDF/DOCX remain disabled in shipping Shiva worker and installed Akula. Signed immutable executable/installer ACLs and File Gate snapshot/permission/revocation integration remain independent release blockers.
