# P2-MFORMAT-WORKER-PDF-QA-01 — real confined PDF-to-host E2E

**Status: Draft; exact-HEAD Linux + existing Windows CI required.**
**Not a format admission.** This bridge must not be wired to installed Akula, File Gate, Tantivy, a user-facing import button, or a release worker.

## Existing process boundary reused

The Shiva worker consumes only framed, host-authorized bytes with independently checked BLAKE3, and runs behind Linux seccomp and rlimits (or Windows LPAC/Job). Its host client independently pins the executable, imposes time/output limits, verifies each source/revision/extractor frame and returns data **only** after an intact Complete terminal result. No pathname, URL, disk, network, or worker-initiated file-opening APIs are added.

## Narrow new debug-only experiment

The opt-in Cargo feature qa-pdf-worker is required **on both** shiva-worker and shiva-worker-client; both also require debug_assertions, Linux, and x86_64. Default builds, release builds (even with the feature), Windows and all other platforms reject PDF. DOCX, archives and all other additional formats remain disabled.

The dispatcher calls Shiva's actual PDF parser in the OS-contained child, after a bounded independent lopdf structural preflight. At most one text-only page and two MiB of source/page content are accepted. Unsupported graphics operators, XObjects, shading, patterns, graphics state, images, annotations, forms/actions/attachment metadata and unknown extensions cause Failed rather than a misleading Complete. A document must contain text-show operators. The preflight is intentionally conservative, not a proof of PDF visual or semantic fidelity.

Tests:
1. Generate a real in-memory PDF with two text markers and send it through the pinned host plus OS-contained worker; verify an ordered, BLAKE3/source-bound Complete frame stream.
2. Reject a blank, malformed, or mixed text-and-vector PDF with no indexable host result.
3. Reject DOCX at host and worker even under the QA PDF feature.
4. Run the ordinary worker/client tests without the feature: PDF must stay disabled.
5. Run explicit release tests with the QA feature selected: PDF still disabled.
6. Existing Windows LPAC, input safety, worker-host and denial workflows remain mandatory. Never weaken seccomp for a green test.

## Remaining integration gates

This feature does not index into Akula. It does not establish immutable File Gate snapshots, permission checks, revocation, atomic Tantivy commits, trusted signed installed-worker identity, page-accurate locators, DOCX handling or Windows PDF admission. Unknown locators are QA-only, not an indexing-quality promise.

Once CI is accepted: implement a separate Akula File Gate adapter that transmits authorized immutable bytes, stages extraction without updating the index, checks source revision and rights again before commit, atomically replaces all prior search state, and deletes/revokes old entries when access changes. Unsupported or partial must be labeled Needs review. Production formats remain closed until native Windows installed acceptance.

Related: [Shiva PR #30](https://github.com/Lalalalendia/shiva/pull/30) is independent real InDesign fidelity QA and should not be silently merged into this branch.
