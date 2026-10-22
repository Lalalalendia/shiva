#!/usr/bin/env python3
"""Create PDF text-operator and Form XObject QA corpus; stdlib only."""
import argparse
import hashlib
from pathlib import Path


def stream(data, attrs=b""):
    return b"<< /Length " + str(len(data)).encode() + b" " + attrs + b">>\nstream\n" + data + b"\nendstream"


def package(objects):
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for i, data in enumerate(objects, 1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode() + data + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objects)+1}\n".encode() + b"0000000000 65535 f \n"
    for offset in offsets[1:]:
        out += f"{offset:010d} 00000 n \n".encode()
    out += (
        b"trailer\n<< /Size " + str(len(objects)+1).encode()
        + b" /Root 1 0 R >>\nstartxref\n" + str(xref).encode()
        + b"\n%%EOF\n"
    )
    assert 100 < len(out) < 32768
    return bytes(out)


def operators():
    commands = (
        b"BT /F1 15 Tf 16 TL 72 720 Td "
        b"(ALPHA_FIRST) Tj "
        b"0 -22 Td (BETA_SECOND) Tj "
        b"T* (GAMMA_THIRD) Tj "
        b"(DELTA_QUOTE) ' "
        b"0 0 (EPSILON_DOUBLE) \" "
        b"0 -18 TD (ZETA_TD) Tj ET\n"
    )
    return package([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(commands),
    ])


def form():
    content = (
        b"BT /F1 16 Tf 72 700 Td (PAGE_NATIVE_123) Tj ET\n"
        b"q 1 0 0 1 72 540 cm /Form1 Do Q\n"
    )
    form_content = b"BT /F1 16 Tf 10 30 Td (FORM_INSIDE_451) Tj ET\n"
    form_attrs = (
        b"/Type /XObject /Subtype /Form /FormType 1 "
        b"/BBox [0 0 350 80] /Resources << /Font << /F1 4 0 R >> >> "
    )
    return package([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Form1 5 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(form_content, form_attrs),
        stream(content),
    ])



def nested_form():
    return package([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Outer 5 0 R >> >> /Contents 7 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(b"BT /F1 16 Tf 10 42 Td (OUTER_NESTED) Tj ET\n/Inner Do\n",
               b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 350 100] /Resources << /Font << /F1 4 0 R >> /XObject << /Inner 6 0 R >> >> "),
        stream(b"BT /F1 16 Tf 5 17 Td (INNER_NESTED) Tj ET\n",
               b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 350 100] /Resources << /Font << /F1 4 0 R >> >> "),
        stream(b"BT /F1 16 Tf 72 700 Td (PAGE_NESTED) Tj ET\nq 1 0 0 1 72 540 cm /Outer Do Q\n"),
    ])


def cyclic_form():
    return package([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Cycle 5 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(b"BT /F1 16 Tf 10 40 Td (CYCLE_SAFE) Tj ET\n/Loop Do\n",
               b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 350 80] /Resources << /Font << /F1 4 0 R >> /XObject << /Loop 5 0 R >> >> "),
        stream(b"/Cycle Do\n"),
    ])


def repeated_form():
    return package([
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Repeat 5 0 R >> >> /Contents 6 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        stream(b"BT /F1 16 Tf 10 40 Td (REPEAT_SAFE) Tj ET\n",
               b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 350 80] /Resources << /Font << /F1 4 0 R >> >> "),
        stream(b"/Repeat Do\n" * 150),
    ])


def deeply_nested_form():
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /First 5 0 R >> >> /Contents 11 0 R >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    for depth in range(6):
        refs = (b" /XObject << /Next " + str(6 + depth).encode() + b" 0 R >>") if depth < 5 else b""
        attrs = (b"/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 350 80] "
                 b"/Resources << /Font << /F1 4 0 R >>" + refs + b" >> ")
        content = b"BT /F1 16 Tf 10 40 Td (DEPTH_" + str(depth).encode() + b") Tj ET\n"
        objects.append(stream(content + (b"/Next Do\n" if depth < 5 else b""), attrs))
    objects.append(stream(b"/First Do\n"))
    assert len(objects) == 11
    return package(objects)



def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    args.out.mkdir(parents=True, mode=0o700, exist_ok=False)
    for name, create in (
        ("text_operators.pdf", operators),
        ("form_xobject.pdf", form),
        ("form_nested.pdf", nested_form),
        ("form_cycle.pdf", cyclic_form),
        ("form_repeat.pdf", repeated_form),
        ("form_deep.pdf", deeply_nested_form),
    ):
        data = create()
        (args.out / name).write_bytes(data)
        print(
            f"SHIVA_OPERATORS={name};bytes={len(data)};"
            f"sha256={hashlib.sha256(data).hexdigest()}"
        )
    print("SHIVA_OPERATOR_GENERATOR=pass;source=stdlib;files=6")


if __name__ == "__main__":
    main()
