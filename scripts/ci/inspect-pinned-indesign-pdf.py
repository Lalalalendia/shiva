#!/usr/bin/env python3
"""Inspect one immutable, openly licensed Adobe InDesign PDF for Shiva QA.

Discovery-only: pin a SHA-256 and build a source-text oracle before permitting
merge. The fixed public corpus URL is the only network input.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import subprocess
from urllib.parse import urlparse
from urllib.request import Request, urlopen

SOURCE_REPO = "openpreserve/format-corpus"
SOURCE_COMMIT = "366f068cec399d0cdfd61fa473de3ab6dc858098"
SOURCE_PATH = "desktop-publishing/InDesign/Neddy_Flyer_HeatherRyan.pdf"
SOURCE_BLOB_SHA1 = "a60bed5ae166a191919ff35abd08ac33cb325bb9"
SOURCE_SHA256 = "6a3c9444d4905c8896a717be7c30ee7d20b3c319eb2d3d469393a0f0e3529243"
SOURCE_BYTES = 59106
MAX_SOURCE_BYTES = 256 * 1024
MAX_TOOL_OUTPUT = 256 * 1024
SOURCE_URL = (
    f"https://raw.githubusercontent.com/{SOURCE_REPO}/{SOURCE_COMMIT}/{SOURCE_PATH}"
)


def verify_bytes(data: bytes) -> str:
    if len(data) != SOURCE_BYTES or len(data) > MAX_SOURCE_BYTES:
        raise ValueError("external InDesign sample size differs from pinned Git blob")
    if not data.startswith(b"%PDF-"):
        raise ValueError("external InDesign sample lacks a PDF header")
    git_header = f"blob {len(data)}\0".encode("ascii")
    blob_sha1 = hashlib.sha1(git_header + data).hexdigest()
    if blob_sha1 != SOURCE_BLOB_SHA1:
        raise ValueError("external InDesign sample differs from immutable Git blob")
    actual_sha256 = hashlib.sha256(data).hexdigest()
    if actual_sha256 != SOURCE_SHA256:
        raise ValueError("external InDesign sample SHA-256 differs from CI-pinned receipt")
    return actual_sha256


def fetch_fixed_pdf() -> bytes:
    request = Request(SOURCE_URL, headers={"User-Agent": "Shiva-Source-QA/1"})
    with urlopen(request, timeout=30) as response:
        redirect = urlparse(response.geturl())
        if redirect.scheme != "https" or redirect.hostname != "raw.githubusercontent.com":
            raise ValueError("unexpected download redirect/host for PDF corpus")
        payload = response.read(MAX_SOURCE_BYTES + 1)
    return payload


def checked(command: list[str], timeout: int = 20) -> bytes:
    result = subprocess.run(
        command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if len(result.stdout) > MAX_TOOL_OUTPUT or len(result.stderr) > MAX_TOOL_OUTPUT:
        raise RuntimeError("PDF inspection tool exceeded output limit")
    if result.returncode:
        diagnostic = result.stderr.decode("utf-8", errors="replace")[:2400]
        raise RuntimeError(
            f"PDF inspector {command[0]} failed, exit={result.returncode};"
            f" diagnostic={diagnostic!r}"
        )
    return result.stdout


def qpdf_diagnosis(pdf: Path) -> str:
    # qpdf 3 means recoverable warnings; this discovery task may inspect
    # with Poppler but must NEVER assert that warnings imply a valid PDF.
    result = subprocess.run(
        ["qpdf", "--check", str(pdf)],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=20, check=False,
    )
    if len(result.stdout) > MAX_TOOL_OUTPUT or len(result.stderr) > MAX_TOOL_OUTPUT:
        raise RuntimeError("qpdf diagnostic exceeded output budget")
    diagnostic = (
        result.stdout + b"\\n" + result.stderr
    ).decode("utf-8", errors="replace")[:6000]
    print(
        f"SHIVA_IND_QPDF_DIAG=exit_{result.returncode};"
        f"message={diagnostic!r}",
        flush=True,
    )
    if result.returncode == 0:
        return "clean"
    if result.returncode == 3:
        return "warnings_not_admitted"
    raise RuntimeError(f"qpdf rejected real source: exit={result.returncode}")


def inspect(pdf: Path) -> None:
    structural_status = qpdf_diagnosis(pdf)
    info = checked(["pdfinfo", str(pdf)]).decode("utf-8", errors="replace")
    page_match = re.search(r"^Pages:\s*(\d+)\s*$", info, re.MULTILINE)
    if page_match is None:
        raise ValueError("independent PDF page count missing")
    pages = int(page_match.group(1))
    if not 1 <= pages <= 64:
        raise ValueError("unexpected external PDF page count")
    metadata = {}
    for field in ("Creator", "Producer"):
        match = re.search(r"^" + field + r":\s*(.*?)\s*$", info, re.MULTILINE)
        metadata[field.lower()] = (match.group(1) if match else "unavailable")[:140]
    text = checked(["pdftotext", "-enc", "UTF-8", "-layout", str(pdf), "-"])
    decoded = text.decode("utf-8", errors="strict")
    segments = decoded.split("\f")
    if segments and not segments[-1].strip():
        segments.pop()
    if len(segments) != pages:
        raise AssertionError("independent PDF text segmentation differs from page count")
    print(f"SHIVA_IND_REAL_PDF=receipt_only;structural_status={structural_status};pages={pages};text_bytes={len(text)};"
          f"creator={metadata['creator']!r};producer={metadata['producer']!r}", flush=True)
    for page_number, segment in enumerate(segments, 1):
        lines = [line.strip() for line in segment.splitlines() if line.strip()]
        print(f"SHIVA_IND_PAGE={page_number};nonempty_lines={len(lines)};"
              f"utf8_sha256={hashlib.sha256(segment.encode('utf-8')).hexdigest()}", flush=True)
        for line in lines[:16]:
            # Public CC0-authorized fixture only, never a user document.
            print("SHIVA_IND_CANDIDATE_ORACLE=" + repr(line[:120]), flush=True)
    print("SHIVA_IND_ADMISSION=blocked_pending_fixed_text_oracle_and_cross_platform_Shiva", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    output = args.out.resolve()
    if output.exists():
        raise ValueError("refuse to overwrite external QA output directory")
    data = fetch_fixed_pdf()
    sha256 = verify_bytes(data)
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    path = output / "indesign-neddy-flyer.pdf"
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(data)
    print(f"SHIVA_IND_SOURCE_SHA=verified_blob;bytes={len(data)};"
          f"git_blob_sha1={SOURCE_BLOB_SHA1};sha256={sha256}", flush=True)
    inspect(path)


if __name__ == "__main__":
    main()
