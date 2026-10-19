# Windows Shiva worker — P0 native AppContainer / Job preview

Status: **Draft until exact-HEAD Windows real API tests complete**.
Repository: [Lalalalendia/shiva](https://github.com/Lalalalendia/shiva).
Existing Linux work: PR #4 and #5. This is not an Akula release or a
permission to index untrusted PDF/DOCX.

## Security model

The trusted host pins a worker BLAKE3 from an independently configured
versioned package, passes document bytes only over an anonymous stdin
pipe, and verifies every result frame before accepting any block. It uses
Win32 CreateAppContainerProfile/DeriveAppContainerSidFromAppContainerName
and CreateProcessW with PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
zero capabilities (including **no network capability**), an explicit
PROC_THREAD_ATTRIBUTE_HANDLE_LIST (only stdin, stdout and NUL stderr),
and PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY.

The process starts **CREATE_SUSPENDED** and is assigned to a fresh
Job Object with JOB_OBJECT_LIMIT_ACTIVE_PROCESS=1, process/job memory
512 MiB, CPU 15 seconds, KILL_ON_JOB_CLOSE and no breakaway flags.
Only after successful AssignProcessToJobObject does the host
ResumeThread. The worker checks TokenIsAppContainer,
TokenCapabilities group count = 0, and IsProcessInJob **before reading
any untrusted byte**.

There is no ordinary std::process::Command fallback on Windows.
Unsupported/unavailable AppContainer or Job APIs cause a hard failure.
The old Linux seccomp implementation stays independent.

The Windows host holds an open executable handle disallowing FILE_SHARE_WRITE
and FILE_SHARE_DELETE between hash check and CreateProcessW. This
protects the binary's path against ordinary replacement of that file,
but **is not proof that mutable ancestor directories/reparse points,
packaging ACL, signed binary origin or image mapping identity are safe**.
The Akula installer must provision an immutable trusted program location,
reject reparse points, verify signature and physical file identity,
and separately accept a real installed Windows build.

## Real negative controls required in CI

- Host checks current worker binary BLAKE3, runs TXT/Markdown/HTML
  Unicode text and validates the COMPLETE terminal protocol.
- Debug-only contained worker attempts to read a private sentinel,
  create a sibling file, connect to a real host loopback listener and
  spawn cmd.exe. The test must prove denies in a **real AppContainer
  process already assigned to its Job**, not infer them from source code.
- Corrupted binary hash, unsupported PDF and external document image
  references must fail without returning indexable text.
- Missing AppContainer/Job configuration MUST cause the test to fail:
  the Windows test is not optional/skipped.
- CI provisionally gives read/execute access to ALL APPLICATION PACKAGES
  for the compiled **worker executable and its parent directory
  traversals only** on an ephemeral GitHub runner. It does *not* grant
  the AppContainer or application packages access to the sentinel,
  user vault or temp input directory.

## Remaining P0 gates

- Host and worker shipped inside Akula's trusted, immutable, code-signed
  Windows installation; physical path identity and anti-reparse review.
- Actual Windows 10/11 installed-machine denial acceptance, DLL search
  hardening, no inherited privileged handles, isolated profile and job
  cleanup policy, malicious archive/parser fuzzing and budget evidence.
- Reconcile policy and ACL with installed-system file structure.
- Then a separate, opt-in Akula -> Shiva adapter and atomic Tantivy
  revocation-safe projection. No source code vendoring into Akula.
- DOCX/PDF and ZIP decoding remain explicitly disabled here.

API sources:
- https://learn.microsoft.com/windows/win32/secauthz/implementing-an-appcontainer
- https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute
- https://learn.microsoft.com/windows/win32/procthread/job-objects
