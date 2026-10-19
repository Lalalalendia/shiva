# Shiva → Akula: versioned extractor contract v1

**Status:** experimental interface, not a worker or a released parser.
**Repository:** Lalalalendia/shiva, package `shiva-extractor-protocol` (`publish = false`).

## Separation of responsibility

- Akula owns file permissions, hash of immutable input bytes, user identity, indexing in Tantivy, revocation and deletion. It must not send arbitrary vault paths to the worker.
- A future separate Shiva worker accepts **only authorized bytes or a host-provided constrained handle**. It must have no network access, no authority to walk the vault or update user files, and OS-enforced CPU/RAM/time/decompression limits.
- Protocol frames travel over a length-prefixed JSON pipe. Each frame is ≤1 MiB and cannot be used to request file reads, URLs, commands or writes.
- Every frame includes protocol version, opaque source ID, BLAKE3 of the host-verified input snapshot, declared format, and pinned extractor version.
- Message sequence and block IDs are strictly increasing. `StreamValidator` rejects source/parser swaps, oversized text and invalid frames, and requires a final explicit `End` status.
- `Complete` on nonempty input cannot silently return zero blocks. `Partial` and `Failed` require a bounded warning. Only the host can decide whether to index a partial result; a `Failed` result must not silently replace a previous valid index projection.
- Individual content blocks have zero-based page, sheet/row/column, section, or unknown locators. Source filenames, tokens and full document text must never be copied into warning notes or logs.

## Wire framing / budgets

Wire: 4-byte little-endian length, followed by UTF-8 JSON. The decoder rejects lengths above 1 MiB **before allocation** and rejects unknown top-level fields. A batch contains 1–128 blocks, each text ≤256 KiB. The validator enforces total extracted text ≤128 MiB. Host-side process execution and overall output/time budgets are independent and mandatory.

## Scope and next safety gates

1. Build a separate `shiva-worker` subprocess that reads authorized immutable input bytes, not vault paths. Protocol crate does **not** spawn or sandbox it.
2. Explicitly disable network, dynamic font downloads, external image retrieval and ambient filesystem access using OS containment (Windows Job/Restricted Token/AppContainer as appropriate; Linux namespaces/seccomp where supported). No portable claim of sandboxing until measured.
3. Pin worker binary hash, schema version, parser feature-set and revision. Compute the source BLAKE3 in Akula, not from worker claims.
4. Add crash/truncation, malformed PDF/DOCX/archives, malicious path, resource exhaustion and partial-index replacement E2E tests. Stage index writes and publish only a validated terminal result.
5. Then implement a thin, opt-in Akula adapter; do not vendor `shiva` source into Akula and do not replace EPUB Reader or Chaptera/PUB.

**No user-facing feature is enabled by this contract alone.**
