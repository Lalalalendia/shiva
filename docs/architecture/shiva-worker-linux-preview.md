# Shiva Worker — Linux isolation preview (P2-MFORMAT Worker-01)

Status: **Draft research gate, not an Akula connector/release**.

This crate lives in `Lalalalendia/shiva/worker`, not the Akula monorepo.
It uses the independent `shiva-extractor-protocol` (merged PR #3)
to return bounded v1 frames for **TXT, Markdown, HTML only**. PDF, DOCX,
archives and other formats are explicitly rejected. This avoids claiming
that resource-intensive third-party parsers are already production-safe.

## Request / response

Input is **stdin only**: 4-byte little-endian length (max 4096),
JSON `{"schema_version":1,"source":{"source_id":"doc-1","blake3_hex":"<64 lowercase hex>","input_bytes":12,"format":"txt"}}`, followed by exactly
`input_bytes` raw document bytes (max 2 MiB). No filename, URL,
source directory or arbitrary command is accepted as a request field.

Worker independently checks BLAKE3, parses in-memory using Shiva, and
writes `shiva-extractor-protocol` frames to stdout. Markdown and HTML
pass a **deny-all image callback**, not the upstream `disk_image_loader(".")`.
Embedded images intentionally cause a bounded `Failed` terminal response.
Blocks are chunked to <=4096 UTF-8 bytes; the host must still stage and
validate the complete stream before updating Tantivy. Crash/missing `End`
is a failed attempt, not an empty or partial successful index.

## Linux preview: fail-closed OS controls

Only **Linux x86_64** runs: the worker closes inherited FDs >=3, sets
RLIMIT_AS=512 MiB, RLIMIT_CPU=5 s, RLIMIT_FSIZE/CORE=0 and
RLIMIT_NOFILE=16, then installs `no_new_privs` and a syscall **allowlist**
via seccomp BPF *before reading untrusted input*. File-opening, socket,
process creation, and executable loading syscalls are not allowed.
A debug-only subprocess probe attempts to read a protected external file,
create a file in /tmp, and connect via TCP: each must return EPERM.
Unsupported platforms, or kernels that refuse the controls, fail closed.

**Security caveats:** this is a narrow Linux x86_64 prototype, *not*
a portable Windows Job/Restricted Token/AppContainer sandbox. Resource
limits do not replace host kill-on-timeout, process identity/binary hash
pinning, input immutability, and a stable Akula launch authority. The
worker has no API to open a vault file. Parent process still owns
permissions, BLAKE3, source revisions, and index lifecycle. Avoid
running a privileged Akula process and untrusted Shiva code together.

## Acceptance and next steps

- CI must compile and run real subprocess tests with the Linux seccomp
  filter, including file/network denial, Unicode, and framed failures.
- Add host launcher with bounded pipes, independent source BLAKE3,
  kill-on-timeout, staged index commits, and revocation handling.
- Develop a **separate Windows OS sandbox**, deny by default until it
  passes the same real-machine denial tests.
- Only then enable additional parsers in worker behind explicit limits:
  PDF/DOCX, archive expansion, size/rate/cpu budgets and malformed-file QA.

No automatic deployment, Akula integration, or Shiva release is performed.
