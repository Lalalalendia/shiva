# Windows Shiva worker — P0 native AppContainer / Job preview

Status: **Windows confinement PR #6 merged with exact-HEAD CI
5/5 subprocess and 7/7 unit tests. PR #7 host-side attestation is a
separate Draft security gate until its exact-HEAD Windows CI passes.**
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

The separate parent-side attestation (PR #7) runs **after**
AssignProcessToJobObject but **while the child thread is still suspended**:
it independently queries the actual process's token (AppContainer and
zero capability groups), verifies membership in the exact Job Object
created by this launch, and confirms the system-enforced child-process
mitigation is active. Any failure kills the suspended Job/process before
the worker receives document bytes. A negative control proves an
ordinary unconfined host process cannot pass that attestation; the
existing positive E2E ensures the real worker can still parse Unicode
TXT. These attestations are useful gates but do not prove binary/package
identity, LPAC, or that additional mitigation policies are active.

## rar2 #260 donor gap — LPAC and OS mitigations

PR #8 narrows the previously proven Shiva AppContainer to **LPAC** by adding
`PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY` set to
`PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT`. It also requires the
same documented mitigation bits independently accepted in rar2 #260:
strict invalid-handle checks, Win32k system calls disabled, and legacy
extension points disabled. The host queries the *actual* suspended child
using `GetProcessMitigationPolicy` for all three and terminates the Job on
any missing flag or failed query. This is not a request to add broad
capabilities to keep libraries running.

**Deployment ACL law:** the current Shiva launcher does NOT call
`SetNamedSecurityInfo`, grant AppContainer ACLs, or chmod source paths.
Only the trusted installer/build fixture may give the exact profile SID
read+execute access to the installed, verified `shiva-worker.exe`
and narrowly necessary directory traversals. The earlier test's broad
ALL APPLICATION PACKAGES RX grant is deliberately removed. Real Windows
CI derives the fixed profile SID and creates a separate AAP-only readable
canary which LPAC must not read. A normal host positive-control read
ensures the canary exists.

`PROC_THREAD_ATTRIBUTE_JOB_LIST` is deliberately **not** changed in this
PR: Shiva still assigns its Job to the suspended child and separately
attests Job membership before calling ResumeThread. Atomic job-list
assignment from rar2 #260 is a distinct follow-up to avoid combining
two compatibility/failure causes. Exact Windows installer code signing,
protected directory ACLs, reparse-point-resistant executable identity
and real-PC acceptance remain separate release blockers.

Donor proof: https://github.com/Lalalalendia/rar2/pull/260
Microsoft API: https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer
Microsoft mitigation policies: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocessmitigationpolicy

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

## Atomic Job membership at CreateProcessW — PR #9

The Windows launcher now supplies a sixth process-creation attribute,
`PROC_THREAD_ATTRIBUTE_JOB_LIST`, pointing to its already-configured Job
handle. There is no subsequent `AssignProcessToJobObject` and **no fallback**
to the previous assignment method on unsupported Windows configurations.
The primary thread is still created suspended; the parent independently
checks actual token, zero capabilities, exact Job membership, child-process
restriction and LPAC mitigation policies before `ResumeThread` and before
transmitting untrusted bytes. On any attestation/resume failure, the parent
terminates the suspended process and Job.

The standalone Windows CI test `nested_outer_job_allows_atomic_inner_worker_launch`
spawns an isolated test host, assigns it to its own outer Job, and requires
successful real TXT extraction via the inner worker Job. It does not change
the security boundaries of other Windows tests. A fresh host-side marker
`SHIVA_WIN_NESTED_JOB_LIST=pass` appears only on success. Existing Windows
negative canaries and LPAC attestation remain required.

A failure caused by job nesting or OS incompatibility is a test failure,
not permission to skip the attribute. Real installed Windows, immutable
signed package and anti-reparse checks still block Akula integration.
