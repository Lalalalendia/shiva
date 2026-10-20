# Shiva trusted installed worker: image path preflight (IMAGE-01)

Status: **isolated security increment / Draft until exact-HEAD Windows CI**.
This is NOT a signature check, installer authorization or complete anti-TOCTOU
solution. The installed Akula application still has no Shiva adapter.

Canonical task: [P2-MFORMAT-WIN-IMAGE-01](https://app.notion.com/p/3fe32a84beec81a2bc29f096fe8bd7a7).
Baseline: PR #8 (LPAC + mitigations); JOB_LIST follow-up #9 is independent.

## New preflight law

Before reading or hashing the configured worker path on Windows, the launcher:

- Requires a local absolute drive path (normal or verbatim drive prefix).
  UNC, device-namespace, relative and drive-relative paths are rejected.
- Rejects explicit parent-directory traversal.
- Calls symlink_metadata on the worker path and *every* ancestor and refuses
  FILE_ATTRIBUTE_REPARSE_POINT, including directory junctions and file links.
- Preserves the held-open, hash-checked executable and all existing LPAC/Job
  restrictions. It never broadens filesystem ACLs or enables a fallback worker.

Windows native E2E creates a junction pointing to the actual worker binary
directory, asserts the alias resolves to a genuine PE file, then ensures
extraction returns InvalidConfiguration rather than launching that path.
A separate test rejects UNC and parent-traversal worker paths without remote
filesystem access. These tests run in the existing Windows-2022 CI.

## Deliberately NOT claimed

Checking path attributes does NOT prevent an attacker from replacing a
mutable ancestor *after* admission. The held-open EXE cannot prove the image
mapped by CreateProcessW came from the same physical object if a parent
directory is redirectable. Full installed-package trust requires separate:

1. Volume/file identity and launch-time revalidation on a pinned handle.
2. An immutable installer-controlled root and audited ACL inheritance,
   with only the exact package SID granted worker read/execute.
3. Authenticode verification and a **pinned publisher identity**; a generic
   successful WinVerifyTrust against any trusted signer is insufficient.
4. Full BLAKE3 binary pinning in addition to signature validation.
5. Real installed Windows 10/11 negative tests (mutation, junction swap,
   signature tamper, malicious files, user-mode ACL bypass) and independent
   review of early-load PE imports/DLL search.

If any remaining gate fails, do not enable Shiva for user documents.

API docs:
- https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfinalpathnamebyhandlew
- https://learn.microsoft.com/en-us/windows/desktop/api/winbase/nf-winbase-getfileinformationbyhandleex
- https://learn.microsoft.com/en-us/windows/win32/api/Wintrust/nf-wintrust-winverifytrust
- https://learn.microsoft.com/en-us/windows/win32/dlls/dynamic-link-library-search-order


## IMAGE-02: bound physical file identity immediately before launch (stacked PR)

The previous IMAGE-01 preflight only detects reparse points present at inspection.
Now the client holds the BLAKE3-validated image `File` across launch and passes
that same handle to `windows_launch::spawn`. Just before `CreateProcessW` the
host checks all ancestors for reparse points **again**, opens the pathname
separately, and obtains `FILE_ID_INFO` with `GetFileInformationByHandleEx`
for both still-open handles. Both `VolumeSerialNumber` and all 128 bits of
`FileId` must match. A failed query or mismatch blocks launch without
falling back to an unverified process. The debug-only exact-head Windows CI
requires `SHIVA_WIN_IMAGE_FILE_ID=pass`.

A dedicated native Windows unit negative creates two distinct files with
*identical contents*. It verifies that reopening the original path matches the
pinned handle while reopening the duplicate does not. A file hash alone would
accept both copies; physical FileId prevents this particular substitution.

**Security limit:** the comparison is still a *pre-launch* path re-open, not
an identity query on the actual mapped image section. An attacker who can
rename/junction-swap mutable ancestor directories in the remaining interval
might still alter resolution by `CreateProcessW`. Only a trusted immutable
installation directory, audited ancestry ACL and verified, pinned signed
package can satisfy the production gate. Do not describe IMAGE-02 as a full
TOCTOU fix.

Source: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info


## IMAGE-03a: protected-root feasibility on hosted Windows CI only

A separate native Windows CI step, after the ordinary LPAC acceptance suite,
copies the already-built `shiva-worker.exe` into a unique temporary fixture
under `%ProgramFiles%` on an ephemeral GitHub-hosted Windows runner.
It disables inherited ACLs at the fixture root and executable, explicitly
allows only Local System, Administrators and the **exact** worker AppContainer
SID (read/execute). The script structurally audits these allow ACEs and
refuses mutation permissions on the exact profile SID. It compares source and
copied SHA-256, then runs the existing real host+LPAC negative and junction
E2E from the protected image location. It prints
`SHIVA_WIN_PROTECTED_ROOT_CI=pass` only when both succeed.

This fixture deliberately does **not** change the real Akula installer,
any vault ACL, any user's installed application, or Shiva's runtime
permissions. It is constrained to `RUNNER_ENVIRONMENT=github-hosted`,
`RUNNER_OS=Windows`, and a unique CI-run ID path. Cleanup is attempted
in a `finally` block.

**Evidence limit:** ACL inspection by an administrator CI user is not the
same as a negative execution from an independently authenticated non-admin
Windows account. It does not establish signer provenance or protected
updates on Windows 10/11. Akula's current per-user NSIS still installs its
GUI in `%LOCALAPPDATA%\\Akula`, without a protected Shiva worker.
Treat this stage as a feasibility test for a *separate* trusted worker
deployment, not installer acceptance. Required follow-ups remain a
real signed package, verified file/root owner and effective non-admin
write/rename/reparse denial, pinned Authenticode publisher and installed
Windows 10/11 acceptance.

Packaging evidence:
- https://github.com/Lalalalendia/Akula/blob/main/.github/workflows/windows-beta.yml
- https://github.com/Lalalalendia/Akula/blob/main/scripts/test-windows-beta.ps1
- https://github.com/Lalalalendia/Akula/blob/main/aquilum-app/native/packaging/akula-windows.json
