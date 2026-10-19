# Shiva Worker Host v1 — staged-only supervision

**Status:** Linux-only proof, not an Akula connector. This crate lives in
`Lalalalendia/shiva/worker-client`; no parser code is copied into Akula.

The host accepts an already authorized in-memory input snapshot and a
host-assigned opaque source ID. It calculates BLAKE3 itself, selects a
trusted pinned Shiva worker executable (checked via full executable BLAKE3)
and starts it with empty environment, neutral cwd and only stdio pipes.
The worker must independently activate its Linux seccomp allowlist.

## Mandatory budgets and fail-closed results

- Input <=2 MiB; output <=16 MiB, guarded streaming pipe read.
- Independent parent deadline (50ms–30s) with forced process kill and
  wait; worker itself also has Linux RLIMIT_CPU/AS and seccomp.
- Decode exact `shiva-extractor-protocol` v1 frames, verify all
  sequence IDs, worker version, source ID/BLAKE3, Complete terminal and
  successful child exit before returning **any** blocks.
- `Partial`, `Failed`, missing/forged End, truncated/oversized output,
  binary mismatch, process crash or nonzero exit all produce errors.
  No Tantivy write or source modification takes place in this crate.
- Tests use a real worker binary with pinned hash, blocked image asset,
  unsupported PDF and an intentional stalled child killed by the parent.

## Not production safe by itself

- Executable path hashing before `Command::spawn` has an inherent TOCTOU
  window. Deployment must keep a trusted immutable installed worker,
  verify code signing/packaging authority and ideally execute from a
  pinned file identity. The worker hash configured by an attacker is
  **not** an independent trust anchor.
- This is **Linux x86_64 only**. Both client and worker must reject
  unconfined execution. Windows requires native AppContainer/Restricted
  Token + Job Object containment and actual denial tests. Job Objects
  alone **do not** prevent file or network access.
- Production index adapter must own source revision transactions,
  permission revocation, immutable file snapshots, atomic index
  replacement and separate records for partial/error outcomes.
- Real PDF/DOCX/archives are not enabled; malformed, fuzz, large input,
  memory and shutdown matrices are separate release gates.

No publication, Akula integration, Windows deployment, or background
watcher is performed.
