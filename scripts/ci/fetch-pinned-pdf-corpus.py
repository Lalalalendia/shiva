#!/usr/bin/env python3
"""CI-only immutable, size-bounded public PDF fixture fetcher.

No production runtime dependency. Only seven explicitly pinned CC0 samples
are fetched. The exact Git blob object identity is checked before anything
can be passed to the QA executable. Files are never embedded into Shiva
and are deleted with the ephemeral CI runner.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
from urllib.parse import quote, urlparse
from urllib.request import Request, urlopen

MAX_SINGLE_PDF = 2 * 1024 * 1024
MAX_CORPUS_BYTES = 4 * 1024 * 1024
MAX_FILES = 12
ALLOWED_ORACLES = {
    "exact-page-text", "empty-text-layer", "needs-ocr",
    "reject", "reject-without-password",
}


def validate_manifest(data):
    if data.get("schema") != 1:
        raise ValueError("unsupported PDF QA manifest version")
    origin = data["origin"]
    if (origin.get("repository") != "tinytoolkit-org/pdf-sample-files"
            or origin.get("license") != "CC0-1.0"
            or not re.fullmatch(r"[0-9a-f]{40}", origin.get("commit", ""))):
        raise ValueError("unexpected fixture source, license or git revision")
    entries = data["fixtures"]
    if not 1 <= len(entries) <= MAX_FILES:
        raise ValueError("invalid fixture count")
    total = 0
    names = set()
    for entry in entries:
        name = entry["name"]
        if not re.fullmatch(r"sample-[a-z0-9-]+\.pdf", name) or name in names:
            raise ValueError("invalid or duplicate PDF fixture basename")
        names.add(name)
        expected = entry["bytes"]
        if not isinstance(expected, int) or not 100 <= expected <= MAX_SINGLE_PDF:
            raise ValueError("PDF fixture size outside QA limits")
        total += expected
        if (not re.fullmatch(r"[0-9a-f]{40}", entry["git_blob_sha1"])
                or entry["oracle"] not in ALLOWED_ORACLES):
            raise ValueError("unverified PDF blob identity or oracle")
    if total > MAX_CORPUS_BYTES:
        raise ValueError("total PDF corpus exceeds QA fetch budget")
    return origin, entries


def checked_bytes(payload, entry):
    expected_size = entry["bytes"]
    if len(payload) != expected_size or not payload.startswith(b"%PDF-"):
        raise ValueError("PDF fixture size/header mismatch")
    blob_header = f"blob {len(payload)}\0".encode("ascii")
    git_sha = hashlib.sha1(blob_header + payload).hexdigest()
    if git_sha != entry["git_blob_sha1"]:
        raise ValueError("PDF fixture differs from pinned upstream Git blob")
    digest = hashlib.sha256(payload).hexdigest()
    pinned_sha256 = entry.get("sha256")
    if pinned_sha256 is not None and digest != pinned_sha256:
        raise ValueError("PDF SHA-256 mismatch")
    return digest


def fetch_one(origin, entry):
    url = (
        "https://raw.githubusercontent.com/"
        + origin["repository"]
        + "/" + origin["commit"]
        + "/out/" + quote(entry["name"], safe="")
    )
    if urlparse(url).hostname != "raw.githubusercontent.com":
        raise ValueError("untrusted fixture host")
    # A one-file memory ceiling keeps even a malicious server response bounded.
    last_error = None
    for _ in range(2):
        try:
            req = Request(url, headers={"User-Agent": "Shiva-Pinned-QA-Corpus/1"})
            with urlopen(req, timeout=30) as response:
                remote = urlparse(response.geturl())
                if remote.scheme != "https" or remote.hostname != "raw.githubusercontent.com":
                    raise ValueError("untrusted HTTP redirect")
                body = response.read(entry["bytes"] + 1)
            return body
        except (OSError, TimeoutError) as exc:
            last_error = exc
    raise RuntimeError("pinned fixture download failed") from last_error


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--manifest", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    args = p.parse_args()
    origin, entries = validate_manifest(
        json.loads(args.manifest.read_text(encoding="utf-8"))
    )
    if args.out.exists():
        raise ValueError("PDF QA output directory already exists")
    args.out.mkdir(mode=0o700, parents=True)
    for entry in entries:
        blob = fetch_one(origin, entry)
        sha256 = checked_bytes(blob, entry)
        path = args.out / entry["name"]
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor = os.open(path, flags, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(blob)
            output.flush()
            os.fsync(output.fileno())
        print(
            "SHIVA_QA_PINNED_FILE="
            + entry["name"]
            + ";bytes=" + str(len(blob))
            + ";sha256=" + sha256
            + ";oracle=" + entry["oracle"],
            flush=True,
        )
    print(f"SHIVA_QA_PINNED_CORPUS=pass;files={len(entries)}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        # Do not leak downloaded PDF data or document metadata in CI.
        print(f"SHIVA_QA_PINNED_CORPUS=failed;reason={type(exc).__name__}", file=sys.stderr)
        sys.exit(1)
