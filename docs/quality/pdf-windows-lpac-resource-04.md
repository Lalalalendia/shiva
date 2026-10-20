# PDF-QA-WINDOWS-LPAC-04 — synthetic PDF in real AppContainer+Job

Status: Draft feasibility gate. [Prior #16 ToUnicode](https://github.com/Lalalalendia/shiva/pull/16), [#17 cumulative parser budgets](https://github.com/Lalalalendia/shiva/pull/17), [#18 Linux OS process QA](https://github.com/Lalalalendia/shiva/pull/18) are merged into Shiva/main.

## Purpose and security boundary

The **existing** Windows worker-client launcher is already capable of fail-closed LPAC AppContainer creation, zero-capability token attestation, atomic Job Object attachment before first thread execution, strict child-process/Win32k/extension/handle mitigations, binary hash pinning, reparse-path denial and immediate pre-launch physical FileId check. Its Job Object enforces per-process and per-job 512-MiB memory quotas and 15 CPU seconds.

This PR adds **no PDF support to the shipping shiva-worker**. A temporary CI-only `lib/examples/pdf_resource_probe.rs` is compiled as a standalone exe under the `pdf` feature. A Windows-2022 GitHub-hosted runner grants only the exact worker AppContainer SID read/execute on that exe and ancestor directories. The same existing launcher starts the QA exe twice with the non-secret arguments `smoke` and `cumulative`, and reuses the original attestation before resume.

The probe methods are additionally gated by `debug_assertions` **and** the opt-in Cargo feature `qa-pdf-windows`. Standard and release builds of `shiva-worker-client` do not expose them. The sole integration test is marked `#[ignore]` so it cannot run inadvertently without an explicitly provisioned CI-only LPAC fixture.

## Measurements and acceptance

Both synthetic child processes must exit successfully with exactly `SHIVA_PDF_QA_CHILD_OK` on stdout. The host checks no unbounded stdout, enforces a 10-second deadline per child and queries the existing Job via `QueryInformationJobObject(JobObjectExtendedLimitInformation)`. Report `PeakJobMemoryUsed` for each case, proving it is positive and below the existing 512-MiB Job quota.

CI requires `SHIVA_WIN_JOB_AT_CREATION=verified`, pre-resume AppContainer attestation and `SHIVA_WIN_PDF_LPAC_JOB_RESOURCE_QA=pass`. Negative cumulative case (five small compressed /Contents streams >16MiB total decoded page content) must error **inside** the PDF parser while the OS child exits successfully, with no partial document emitted.

The CI must remain green alongside the existing Windows text/markdown/HTML-only worker tests; the QA exe is not installed, indexed or used to read actual files. There is no file access beyond the CI-provisioned executable and OS runtime.

## Important omissions (not a release gate)

Only two synthetic fixtures are exercised, without a hostile public/real-world corpus. Exact per-process CPU consumption is not separately reported, although the real Job has a 15-second CPU limit. An upstream Windows Store/installed Authenticode signed image and immutable installation ACL have **not** been proven. Current Akula File Gate, authorization, revocation, document snapshot/protocol, crash recovery and Tantivy indexing remain separate. The test does not prove a PDF worker safe for production; do not enable PDF in `shiva-worker` or installed Akula from this evidence alone.
