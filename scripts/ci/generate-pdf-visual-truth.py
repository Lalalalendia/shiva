#!/usr/bin/env python3
"""QA-only deterministic PDF visual-truth corpus; Python stdlib only."""
import argparse
import hashlib
from pathlib import Path

LABEL = "IMAGE LABEL"
GLYPHS = {
    " ": ["000"] * 7,
    "A": ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
    "B": ["11110", "10001", "10001", "11110", "10001", "10001", "11110"],
    "E": ["11111", "10000", "10000", "11110", "10000", "10000", "11111"],
    "G": ["01111", "10000", "10000", "10111", "10001", "10001", "01111"],
    "I": ["11111", "00100", "00100", "00100", "00100", "00100", "11111"],
    "L": ["10000", "10000", "10000", "10000", "10000", "10000", "11111"],
    "M": ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
}


def raster_label():
    rows = ["" for _ in range(7)]
    for char in LABEL:
        glyph = GLYPHS[char]
        for n in range(7):
            rows[n] += glyph[n] + "00"
    width = max(map(len, rows))
    assert width < 128
    return width, 7, bytes(
        0 if pixel == "1" else 255
        for row in rows
        for pixel in row.ljust(width, "0")
    )


def cmap(broken):
    chars = sorted(set("HELLO SHIVA"))
    changed = {"H": "X", "S": "Z"} if broken else {}
    bfchar = "\n".join(
        f"<{ord(char):02X}> <{ord(changed.get(char, char)):04X}>"
        for char in chars
    )
    return (
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n"
        "/CMapName /Shiva-Visual-Truth def\n/CMapType 2 def\n"
        "1 begincodespacerange\n<00> <FF>\nendcodespacerange\n"
        f"{len(chars)} beginbfchar\n{bfchar}\nendbfchar\n"
        "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
    ).encode("ascii")


def stream(data, attrs=b""):
    return (
        b"<< /Length " + str(len(data)).encode() + b" " + attrs
        + b">>\nstream\n" + data + b"\nendstream"
    )


def pdf(style):
    assert style in {"healthy", "broken", "image_only", "mixed", "invisible"}
    use_cmap = style in {"healthy", "broken"}
    has_image = style in {"image_only", "mixed"}
    objects = []

    def add(blob):
        objects.append(blob)
        return len(objects)

    assert add(b"<< /Type /Catalog /Pages 2 0 R >>") == 1
    assert add(b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>") == 2
    page = add(b"")
    font = add(b"") if style != "image_only" else None
    unicode = add(stream(cmap(style == "broken"))) if use_cmap else None
    image = None
    if has_image:
        width, height, pixels = raster_label()
        attrs = (
            b"/Type /XObject /Subtype /Image /Width " + str(width).encode()
            + b" /Height " + str(height).encode()
            + b" /ColorSpace /DeviceGray /BitsPerComponent 8 "
        )
        image = add(stream(pixels, attrs))

    if use_cmap:
        operations = b"BT /F1 26 Tf 72 702 Td (HELLO SHIVA) Tj ET\n"
    elif style == "mixed":
        operations = b"BT /F1 26 Tf 72 702 Td (NATIVE HEADER) Tj ET\n"
    elif style == "invisible":
        operations = (
            b"BT /F1 26 Tf 72 702 Td (PUBLIC MESSAGE) Tj\n"
            b"3 Tr 0 -48 Td (HIDDEN MEMO) Tj ET\n"
        )
    else:
        operations = b""
    if has_image:
        operations += b"q 225 0 0 25 72 610 cm /Im1 Do Q\n"
    contents = add(stream(operations))
    resources = []
    if font is not None:
        resources.append(b"/Font << /F1 " + str(font).encode() + b" 0 R >>")
        # Omit /Encoding deliberately: lopdf 0.45 reads ToUnicode in this case.
        objects[font - 1] = (
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica "
            + (b"/ToUnicode " + str(unicode).encode() + b" 0 R " if unicode else b"")
            + b">>"
        )
    if image is not None:
        resources.append(b"/XObject << /Im1 " + str(image).encode() + b" 0 R >>")
    objects[page - 1] = (
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
        b"/Resources << " + b" ".join(resources) + b" >> "
        b"/Contents " + str(contents).encode() + b" 0 R >>"
    )

    result = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for i, blob in enumerate(objects, 1):
        offsets.append(len(result))
        result += f"{i} 0 obj\n".encode() + blob + b"\nendobj\n"
    xref = len(result)
    result += f"xref\n0 {len(objects) + 1}\n".encode()
    result += b"0000000000 65535 f \n"
    for offset in offsets[1:]:
        result += f"{offset:010d} 00000 n \n".encode()
    result += (
        b"trailer\n<< /Size " + str(len(objects) + 1).encode()
        + b" /Root 1 0 R >>\nstartxref\n" + str(xref).encode()
        + b"\n%%EOF\n"
    )
    assert 100 < len(result) < 512 * 1024
    return bytes(result)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, mode=0o700, exist_ok=False)
    for name in ("healthy", "broken", "image_only", "mixed", "invisible"):
        data = pdf(name)
        (args.out / f"{name}.pdf").write_bytes(data)
        print(
            f"SHIVA_VISUAL_TRUTH={name};bytes={len(data)};"
            f"sha256={hashlib.sha256(data).hexdigest()}"
        )
    print("SHIVA_VISUAL_TRUTH_GENERATOR=pass;profiles=5;dependencies=stdlib")


if __name__ == "__main__":
    main()
