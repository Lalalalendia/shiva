#!/usr/bin/env python3
"""CI-only qpdf structural rewrites checked against independent Poppler text.

Inputs are strictly allowlisted QA sources, never user PDFs.
"""
import argparse
import hashlib
import subprocess
from pathlib import Path

MAX_SOURCE = 2 * 1024 * 1024
MAX_OUTPUT = 4 * 1024 * 1024
MAX_TEXT = 512 * 1024
SOURCES = (
    ("cc0-1", "pdf_lib", "sample-1-page.pdf"),
    ("cc0-10", "pdf_lib", "sample-10-pages.pdf"),
    ("rl-1", "reportlab", "reportlab-1-page.pdf"),
    ("rl-12", "reportlab", "reportlab-12-pages.pdf"),
    ("ch-1", "chromium", "chromium-1-page.pdf"),
    ("ch-8", "chromium", "chromium-8-pages.pdf"),
    ("ch-uni-2", "chromium", "chromium-unicode-2-pages.pdf"),
)
VARIANTS = (
    ("linearized", ("--linearize",)),
    ("objstreams", ("--object-streams=generate", "--stream-data=compress")),
    ("uncompressed", ("--object-streams=disable", "--stream-data=uncompress")),
)


def checked(command, timeout=20):
    proc = subprocess.run(
        command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if proc.returncode:
        raise RuntimeError("independent PDF QA tool failed or reported warnings")
    if len(proc.stdout) > MAX_TEXT:
        raise ValueError("independent PDF QA tool output exceeded limit")
    return proc.stdout


def pdf_bytes(path, limit):
    if path.is_symlink() or not path.is_file():
        raise ValueError("PDF source missing or symlinked")
    if not 100 <= path.stat().st_size <= limit:
        raise ValueError("PDF outside allowed size")
    data = path.read_bytes()
    if len(data) > limit or not data.startswith(b"%PDF-"):
        raise ValueError("PDF header or size invalid")
    return data


def poppler(path):
    output = checked(["pdftotext", "-enc", "UTF-8", "-layout", str(path), "-"])
    if not output.strip():
        raise ValueError("nonempty QA PDF yielded empty Poppler text")
    return output


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--pdf-lib", type=Path, required=True)
    p.add_argument("--reportlab", type=Path, required=True)
    p.add_argument("--chromium", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    args = p.parse_args()
    roots = {"pdf_lib": args.pdf_lib, "reportlab": args.reportlab, "chromium": args.chromium}
    for root in roots.values():
        if not root.is_absolute() or root.is_symlink() or not root.is_dir():
            raise ValueError("QA source root must be an absolute existing directory")
    if args.out.exists():
        raise ValueError("refuse to overwrite existing QA results")
    args.out.mkdir(mode=0o700, parents=True, exist_ok=False)

    print("SHIVA_QPDF_VERSION=" + checked(["qpdf", "--version"]).decode().splitlines()[0])
    for label, origin, filename in SOURCES:
        source_bytes = pdf_bytes(roots[origin] / filename, MAX_SOURCE)
        directory = args.out / label
        directory.mkdir(mode=0o700)
        original_path = directory / "original.pdf"
        original_path.write_bytes(source_bytes)
        checked(["qpdf", "--check", str(original_path)])
        baseline = poppler(original_path)
        for variant, flags in VARIANTS:
            rewritten_path = directory / (variant + ".pdf")
            checked(["qpdf", *flags, str(original_path), str(rewritten_path)])
            modified = pdf_bytes(rewritten_path, MAX_OUTPUT)
            if modified == source_bytes:
                raise ValueError("structural rewrite was not byte-distinct")
            checked(["qpdf", "--check", str(rewritten_path)])
            if variant == "linearized":
                checked(["qpdf", "--check-linearization", str(rewritten_path)])
            if poppler(rewritten_path) != baseline:
                raise AssertionError("Poppler full-text drift after qpdf rewrite")
            print(
                "SHIVA_QPDF_PREPARED=pass;profile=" + label
                + ";variant=" + variant
                + ";bytes=" + str(len(modified))
                + ";sha256=" + hashlib.sha256(modified).hexdigest(),
                flush=True,
            )
    print("SHIVA_QPDF_POPPLER_ORACLE=pass;originals=7;rewrites=21", flush=True)


if __name__ == "__main__":
    main()
