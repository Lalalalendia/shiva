#!/usr/bin/env python3
"""Offline synthetic Chrome-print PDF QA fixtures; never process user files.

Chrome output may change across browser versions. Log per-run SHA-256 and
browser version without treating those values as immutable cross-run pins.
"""
import argparse
import hashlib
from html import escape
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile

MAX_PDF_BYTES = 2 * 1024 * 1024
PROFILES = (
    ("chromium-1-page.pdf", 1, False),
    ("chromium-8-pages.pdf", 8, False),
    ("chromium-unicode-2-pages.pdf", 2, True),
)


def html_for(pages: int, unicode_text: bool) -> str:
    style = """@page { size: A4; margin: 20mm 18mm; }
html, body { margin: 0; padding: 0; }
body { font: 14px Arial, 'DejaVu Sans', sans-serif; }
.sheet { box-sizing: border-box; height: 230mm; padding: 5mm 3mm; break-after: page; }
.sheet:last-child { break-after: auto; }
h1 { font-size: 17px; margin: 0 0 15mm; }
p { margin: 0 0 12mm; }
"""
    sections = []
    for index in range(1, pages + 1):
        title = f"CHQA-{index:03}-OF-{pages:03}"
        body = f"CHBODY-{index:03}-OF-{pages:03}-ALPHA-12345"
        tail = f"CHTAIL-{index:03}-OF-{pages:03}-OMEGA-67890"
        extra = "Тестовая строка кириллицы" if unicode_text else "INDEPENDENT-CHROME-PDF-WRITER"
        sections.append(
            '<section class="sheet">'
            f"<h1>{escape(title)}</h1>"
            f"<p>{escape(body)}</p>"
            f"<p>{escape(tail)}</p>"
            f"<p>{escape(extra)}</p>"
            "</section>"
        )
    return (
        '<!doctype html><html><head><meta charset="utf-8">'
        f'<style>{style}</style></head><body>{"".join(sections)}</body></html>'
    )


def print_pdf(browser: str, html: Path, pdf: Path, profile: Path) -> None:
    command = [
        browser, "--headless", "--no-sandbox", "--disable-gpu",
        "--disable-dev-shm-usage", "--disable-background-networking",
        "--disable-extensions", "--disable-component-update",
        "--no-first-run", "--no-default-browser-check",
        "--no-pdf-header-footer", f"--user-data-dir={profile}",
        f"--print-to-pdf={pdf}", html.as_uri(),
    ]
    # Renderer descendants may keep pipes open after the browser exits.
    # Avoid pipes; kill the entire process group after the wall-clock limit.
    with subprocess.Popen(
        command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True,
    ) as process:
        try:
            result = process.wait(timeout=40)
        except subprocess.TimeoutExpired as error:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise RuntimeError("Chrome printing exceeded the QA timeout") from error
    if result != 0:
        raise RuntimeError("Chrome failed to print the synthetic QA document")
    if not pdf.is_file() or pdf.is_symlink():
        raise ValueError("Chrome produced no regular PDF file")
    raw = pdf.read_bytes()
    if not (100 <= len(raw) <= MAX_PDF_BYTES) or not raw.startswith(b"%PDF-"):
        raise ValueError("Chrome PDF exceeded limits or has an invalid header")


def generate(output: Path, browser: str) -> None:
    if output.exists():
        raise ValueError("refuse to overwrite existing QA corpus directory")
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    version = subprocess.run(
        [browser, "--version"], check=True, capture_output=True,
        text=True, timeout=10,
    ).stdout.strip()
    if not ("chrome" in version.lower() or "chromium" in version.lower()):
        raise ValueError("expected a Chrome/Chromium binary")
    print(f"SHIVA_CHROMIUM_WRITER={version}", flush=True)
    with tempfile.TemporaryDirectory(prefix="chromium-html-", dir=output) as temp:
        temp_dir = Path(temp)
        for filename, count, unicode_text in PROFILES:
            html_path = temp_dir / (filename + ".html")
            html_path.write_text(html_for(count, unicode_text), encoding="utf-8")
            pdf_path = output / filename
            print_pdf(browser, html_path, pdf_path, temp_dir / (filename + ".profile"))
            data = pdf_path.read_bytes()
            print(
                f"SHIVA_CHROMIUM_GENERATED={filename};bytes={len(data)};"
                f"sha256={hashlib.sha256(data).hexdigest()};pages={count}",
                flush=True,
            )
    print("SHIVA_CHROMIUM_GENERATOR=pass;profiles=3;expected_pages=11", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--browser", default=None)
    args = parser.parse_args()
    browser = args.browser or shutil.which("google-chrome") or shutil.which("chromium")
    if not browser:
        raise RuntimeError("Chrome/Chromium executable missing from QA runner")
    generate(args.out.resolve(), browser)


if __name__ == "__main__":
    main()
