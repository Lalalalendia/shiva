# PDF-QA-PAGE-TREE-06 — strict atomic page-tree admission

Status: **Draft implementation / dependent on Shiva PR #20**. Canonical [Notion task](https://app.notion.com/p/3ff32a84beec813ea0aefd485d078342).

## Threat

`lopdf 0.45.0`'s `get_pages()` uses a recovery-oriented `PageTreeIter` that can stop after an object-based visit limit, skip invalid child references, or hit its own depth guard **without exposing a traversal error**. Passing its returned subset to the search pipeline can therefore create a misleading Complete document.

## Implemented independent validator

`lib/src/pdf/page_tree.rs` returns an **ordered `Vec<ObjectId>` or an error**. The ordinary PDF parser now extracts strictly from that returned sequence, never from a second call to `get_pages()`. The validator reads original loaded PDF objects without modifying or repairing them.

All checks apply *before* extracting any page content:

- `/Catalog /Pages` must be an indirect reference to the root `/Pages` dictionary.
- `/Kids` members must be indirect references to `/Page` or `/Pages` dictionaries, with an explicit and matching `/Parent` for each child. Root `/Pages` cannot have `/Parent`.
- Both repeated `/Page` leaves and repeated `/Pages` branches/cycles are rejected by globally tracking unique `(object number, generation)` identities. Indirect/shared/cyclic `/Kids` arrays are rejected too.
- Positive `/Count` on every intermediate node is checked against the exact number of descendant leaves visited (post-order), **not trusted for enumeration**.
- Guardrails: maximum 1024 actual and declared pages, 4096 traversed tree nodes, 64 ancestry levels, and 16 links per indirect `/Kids` chain; checked arithmetic where totals change.
- A genuinely empty page **without /Contents** remains valid, but an entire PDF with zero pages is rejected explicitly, never published as Complete. Valid `/Resources` and `/MediaBox` inheritance is preserved.
- No calls to any repair path. No raw PDF bytes or text logged.

## Regression tests

The generated in-memory PDF corpus checks:

- Valid two-page text cardinality and original ordering.
- Valid nested page tree and inherited `/Resources`.
- Root and nested `/Count` mismatch; wrong leaf parent; missing/non-reference/wrong-type kid.
- Duplicate leaf; duplicated indirect shared branch/array; self-cycle.
- Root leaf typed `/Page` instead of `/Pages`; zero-page tree.
- Depth and declared-count limits.

The new `pdf::page_tree_tests::tree_` CI step is required in **both** Linux offline-PDF and Windows malicious-PDF workflows. The existing CMap, multi-stream, cumulative byte/text budgets, malformed FlateDecode, and LPAC resource process checks remain separate gates.

## Nonclaims

This validates the *available, pinned PDF revision's internal page tree* only; it cannot reconstruct historical pages deleted by a legitimate edit, validate every PDF operator/XObject, prove actual CPU/RSS limits for malicious files, or grant a `Complete` Akula search index. The `shiva-worker` still only admits its preexisting text/Markdown/HTML formats; installed Akula, File Gate authorization and revoked-index behavior remain untouched.

Release acceptance requires #20 merged first, exact-head green Linux/Windows and worker-safety CI on this branch after rebasing onto `main`, code review, and a separate signed-install/security/real-world corpus gate.
