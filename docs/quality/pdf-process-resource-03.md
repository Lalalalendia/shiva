# PDF-QA-OS-PROCESS-03 — real Linux process resource feasibility

Status: Draft experimental proof. Depends on [#17 PDF cumulative limits](https://github.com/Lalalalendia/shiva/pull/17), which depends on merged [#16 ToUnicode](https://github.com/Lalalalendia/shiva/pull/16).

This is **not the production Shiva sandbox**. The current `shiva-worker` intentionally does not link or admit PDF.

## Boundary exercised

The GitHub-hosted Ubuntu runner builds a **separate QA-only example** with the PDF library feature. The runner then starts it as an independent OS process with external, fail-closed constraints:

- `prlimit --as=3221225472 --cpu=20` — max virtual address space 3 GiB and CPU 20 seconds for the child.
- GNU `timeout --signal=TERM --kill-after=3s 30s` — wall-clock upper bound for each scenario.
- GNU `/usr/bin/time` records peak resident KB, checked against a provisional 1-GiB RSS QA threshold. This check happens after each child returns successfully; it is an **observed metric**, not a hard kernel RSS enforcement rule.
- If any limit is hit, evidence is missing, a fixture escapes its expected behavior, or the child fails, CI fails. No permissive fallback.
- The child generates only deterministic in-memory synthetic PDF fixtures; no user's files or secrets are read. It prints only constant status values, not PDF text or parser errors.

## Two child process scenarios

1. **smoke:** a one-page Type1 PDF with a single `HelloBounded` text run must parse exactly once under the imposed OS limits.
2. **cumulative:** five separate Flate-compressed pages below 8 MiB each, but above the 16-MiB total decode budget, must return the expected fail-closed parser error while the child exits cleanly under the same OS limits.

CI requires `SHIVA_PDF_QA_OS_RESOURCE_BOUNDARY=pass` and separately reports the peak child RSS for each case. The numeric QA thresholds are intentionally generous feasibility bounds, not tuned release ceilings.

## Explicitly out of scope

This experiment does not establish a restricted Windows AppContainer PDF worker, network/filesystem denials, memory commitments for real hostile PDF corpora, DOS resistance against arbitrary nested font/CMap trees, Authenticode/pinned signer, release installer ACLs, or Akula File Gate/revocation. Do not grant PDF support in the existing Shiva worker on the strength of this evidence. Next proof requires a restricted Windows/Linux worker test with actual PDF parsing enabled only in a test-specific build, then extensive adversarial corpus and genuine end-to-end publisher/artifact permissions.
