# PDF-QA-COVERAGE-FUZZ-10 — Linux libFuzzer smoke (not production admission)

**Status: Draft implementation, exact-HEAD CI required.** Depends on the already merged [#24 semantic graph mutations](https://github.com/Lalalalendia/shiva/pull/24) and [#25 whole-text oracle](https://github.com/Lalalalendia/shiva/pull/25). This adds no PDF support to the shipping worker.

## Why a true coverage-guided experiment

Earlier 93 deterministic mutations proved 76 explicit rejections and 17 acceptances preserving the entire original extract. Those inputs were fixed in advance, so they cannot discover new execution paths autonomously. [cargo-fuzz/libFuzzer](https://github.com/rust-fuzz/cargo-fuzz) instruments program branches and mutates input guided by new coverage. The initial program is a short **CI smoke**, not a long-running fuzzing service and not comprehensive memory safety assurance.

## Independent nightly-only workspace

- `fuzz/Cargo.toml` defines a **separate workspace** with `cargo-fuzz 0.13.2` (CI command) and `libfuzzer-sys 0.4.13`, not a dependency of the main Shiva Cargo workspace or installed worker.
- **`pdf_parse`:** consumes libFuzzer's raw bytes (max 256 KiB) and invokes the real library parser. An error is an acceptable rejection; a panic, ASan diagnostic or timeout fails the job. Its coverage is primarily parser resilience, **not** index completeness.
- **`pdf_required_graph`:** consumes a short (4–16 byte) mutation instruction, not an arbitrary document. Once at startup, it reads the exact source-verified pdf-lib (1, 10 pages) and ReportLab (12, rotated 3 pages) QA inputs, captures every expected page marker and the *entire* baseline extracted document. Instructions select a seed, a required page-graph token, an occurrence and either a one-byte key change or numeric `/Count` change. For every mutated document: **parse error or byte-for-byte identical whole extraction**. A silent partial/full-text change causes libFuzzer failure; raw document data never appears in the assertion message.
- The bounded QA script stages **five exact PDF files** for raw parser fuzzing and **72 small mutation instructions** from those independent libraries. Files exist only in temporary GitHub Actions checkout/runner and are not distributed in a release binary or committed as corpus samples.

## Operational and safety gates

The dedicated Ubuntu 24.04 workflow installs the pinned fuzz runner and immutable upstream PDF seeds, regenerates SHA-verified ReportLab PDFs, then compiles address-sanitized targets with Rust nightly. Both targets run under GNU timeout and libFuzzer input length, per-case timeout and RSS limits. The first gate requests **1000 raw-input runs**, the second **2000 semantic-instruction runs**, with exact-head coverage-guided instrumentation. Any crash/ASan issue, parser panic, memory cap, timeout or accepted semantic drift is **CI FAIL**, never Complete. Reproducers are uploaded only on CI failure and retained for seven days, then must become a standalone deterministic regression before a fix is accepted.

This workflow is **Linux only** because sanitizer/libFuzzer integration is presently best supported there; normal native Windows PDF hostile and LPAC/Job tests remain separate previously accepted gates. A successful fixed-length smoke does not replace long-running OSS-Fuzz/ClusterFuzz, differential PDF engines, corpus minimization, actual signed installed Windows image/ACL tests or Akula File Gate permissions/revocation.

## Follow-up, after actual results

1. Inspect coverage and execution count emitted by libFuzzer; if corpus fails to exercise parser deeply, add audited parser-aware seeds and mutation operators.
2. For crashes: save input's SHA, minimize with `cargo fuzz tmin`, create a fixed test that reproduces the exact panic or accepted-text drift, then repair the parser. No manual exceptions for individual crashing cases.
3. Run longer rotating fuzz campaigns with sanitizer coverage and cross-version dependency identity. Define clear CPU/RSS and wall-clock budgets for longer campaigns.
4. Keep all PDF/DOCX off the shipping `shiva-worker` allowlist and installed Akula until independent signed-installer/ACL and File Gate/authorization/revocation gates pass.
