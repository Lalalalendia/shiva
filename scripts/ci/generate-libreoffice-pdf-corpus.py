#!/usr/bin/env python3
"""QA-only Writer PDF corpus from synthetic OOXML DOCX documents.

Actual PDF exporter is LibreOffice Writer, but the source documents are
synthetic. This is not a corpus of real Office/Adobe customer documents.
"""
import argparse
import hashlib
import re
import subprocess
import tempfile
from pathlib import Path
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZipFile

MAX_DOCX = 128 * 1024
MAX_PDF = 2 * 1024 * 1024
PROFILES = (
    ("writer-1-page", 1, False),
    ("writer-6-pages", 6, False),
    ("writer-unicode-2-pages", 2, True),
)


def runs(text: str):
    return f'<w:r><w:t>{escape(text)}</w:t></w:r>'


def paragraph(text: str, *, new_page=False):
    props = '<w:pPr><w:pageBreakBefore/></w:pPr>' if new_page else ''
    return f'<w:p>{props}{runs(text)}</w:p>'


def document_xml(pages: int, unicode_text: bool):
    body = []
    for page in range(1, pages + 1):
        body.append(paragraph(f'LOQA-{page:03}-OF-{pages:03}', new_page=page > 1))
        body.append(paragraph(f'LOBODY-{page:03}-OF-{pages:03}-ALPHA-12345'))
        body.append(paragraph(f'LOTAIL-{page:03}-OF-{pages:03}-OMEGA-67890'))
        body.append(paragraph(
            'Тестовая строка LibreOffice'
            if unicode_text else 'WRITER-PDF-FROM-SYNTHETIC-DOCX'
        ))
    body.append(
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1000" w:bottom="1000" w:left="1200" w:right="1200"/></w:sectPr>'
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
        '<w:body>' + ''.join(body) + '</w:body></w:document>'
    )


def write_docx(path: Path, pages: int, unicode_text: bool):
    content_types = (
        '<?xml version="1.0" encoding="UTF-8"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" '
        'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        '</Types>'
    )
    relationships = (
        '<?xml version="1.0" encoding="UTF-8"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" '
        'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" '
        'Target="word/document.xml"/></Relationships>'
    )
    with ZipFile(path, 'w', compression=ZIP_DEFLATED, compresslevel=9) as archive:
        archive.writestr('[Content_Types].xml', content_types)
        archive.writestr('_rels/.rels', relationships)
        archive.writestr('word/document.xml', document_xml(pages, unicode_text))
    if not 100 <= path.stat().st_size <= MAX_DOCX:
        raise ValueError('synthetic DOCX outside QA input size budget')


def checked(command, timeout=50, max_bytes=200 * 1024):
    completed = subprocess.run(
        command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        timeout=timeout, check=False,
    )
    if (completed.returncode
            or len(completed.stdout) > max_bytes
            or len(completed.stderr) > max_bytes):
        raise RuntimeError('office/PDF QA tool failed or exceeded output bound')
    return completed.stdout.decode('utf-8', errors='replace')


def verify_pdf(pdf: Path, pages: int, unicode_text: bool):
    if pdf.is_symlink() or not pdf.is_file() or not 100 <= pdf.stat().st_size <= MAX_PDF:
        raise ValueError('LibreOffice PDF missing or outside QA output budget')
    data = pdf.read_bytes()
    if not data.startswith(b'%PDF-'):
        raise ValueError('LibreOffice PDF header invalid')
    metadata = checked(['pdfinfo', str(pdf)])
    count = re.search(r'^Pages:\s+(\d+)\s*$', metadata, re.MULTILINE)
    producer = re.search(r'^Producer:\s*(.*?)\s*$', metadata, re.MULTILINE)
    if count is None or int(count.group(1)) != pages:
        raise AssertionError('Writer emitted unexpected PDF page count')
    if producer is None or 'LibreOffice' not in producer.group(1):
        raise AssertionError('Writer PDF producer provenance mismatch')
    text = checked(['pdftotext', '-enc', 'UTF-8', '-layout', str(pdf), '-'])
    sections = text.split('\f')
    if sections and not sections[-1].strip():
        sections.pop()
    if len(sections) != pages:
        raise AssertionError('Poppler page segmentation mismatch')
    for page, section in enumerate(sections, 1):
        markers = (
            f'LOQA-{page:03}-OF-{pages:03}',
            f'LOBODY-{page:03}-OF-{pages:03}-ALPHA-12345',
            f'LOTAIL-{page:03}-OF-{pages:03}-OMEGA-67890',
            'Тестовая строка LibreOffice' if unicode_text
            else 'WRITER-PDF-FROM-SYNTHETIC-DOCX',
        )
        previous = -1
        for marker in markers:
            if section.count(marker) != 1:
                raise AssertionError('source expected-text loss/duplication')
            position = section.find(marker)
            if position <= previous:
                raise AssertionError('source expected-text order changed')
            previous = position
    return len(data), hashlib.sha256(data).hexdigest()


def generate(output: Path, binary: str):
    if output.exists():
        raise ValueError('refuse to overwrite existing QA corpus')
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    version = checked([binary, '--version'], timeout=10).strip()
    if 'LibreOffice' not in version:
        raise RuntimeError('expected actual LibreOffice binary')
    print(f'SHIVA_LO_WRITER={version}', flush=True)
    with tempfile.TemporaryDirectory(dir=output, prefix='office-sources-') as tmp:
        source_dir = Path(tmp)
        for label, pages, unicode_text in PROFILES:
            docx = source_dir / (label + '.docx')
            write_docx(docx, pages, unicode_text)
            profile = source_dir / (label + '-profile')
            profile.mkdir(mode=0o700)
            checked([
                binary, '-env:UserInstallation=' + profile.as_uri(),
                '--headless', '--convert-to', 'pdf:writer_pdf_Export',
                '--outdir', str(output), str(docx)
            ])
            size, pdf_sha = verify_pdf(output / (label + '.pdf'), pages, unicode_text)
            source_sha = hashlib.sha256(docx.read_bytes()).hexdigest()
            print(
                f'SHIVA_LO_PDF=pass;profile={label};pages={pages};'
                f'bytes={size};sha256={pdf_sha};source_docx_sha256={source_sha}',
                flush=True
            )
    print('SHIVA_LO_CORPUS=pass;profiles=3;pages=9', flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--binary', default='libreoffice')
    args = parser.parse_args()
    generate(args.out.resolve(), args.binary)


if __name__ == '__main__':
    main()
