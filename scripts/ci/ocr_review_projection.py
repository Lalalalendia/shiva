#!/usr/bin/env python3
"""QA-only, conservative projection of OCR TSV into searchable vs review-needed lines.

Not a trained model, format parser or production indexer. Keeps all words in
memory and never claims Complete when any unverified line is withheld.
"""
from __future__ import annotations

import csv
from dataclasses import dataclass
from io import StringIO
from statistics import median


@dataclass(frozen=True)
class Word:
    text: str
    x: int
    y: int
    w: int
    h: int
    conf: float
    key: tuple[int, int, int, int]


@dataclass
class OcrLine:
    words: list[Word]
    x: int
    y: int
    right: int
    bottom: int

    @property
    def span(self) -> int:
        return self.right - self.x

    @property
    def height(self) -> int:
        return self.bottom - self.y

    @property
    def characters(self) -> int:
        return sum(len(w.text) for w in self.words)

    @property
    def confidence(self) -> float:
        return median(w.conf for w in self.words)


def parse_tsv(tsv: str, width: int, height: int) -> list[OcrLine]:
    if width <= 0 or height <= 0 or width * height > 50_000_000:
        raise ValueError("invalid image dimensions")
    if len(tsv) > 16 * 1024 * 1024:
        raise ValueError("excessive TSV length")
    reader = csv.DictReader(StringIO(tsv), delimiter="\t")
    expected = {"level", "block_num", "par_num", "line_num", "word_num", "left", "top", "width", "height", "conf", "text", "page_num"}
    if not expected.issubset(reader.fieldnames or []):
        raise ValueError("missing TSV fields")
    lines: dict[tuple[int, int, int, int], list[Word]] = {}
    count = 0
    for row in reader:
        if row["level"] != "5" or not row["text"].strip():
            continue
        count += 1
        if count > 100_000:
            raise ValueError("OCR word count exceeded")
        try:
            x, y, w, h = (int(row[field]) for field in ("left", "top", "width", "height"))
            conf = float(row["conf"])
            k = tuple(int(row[field]) for field in ("page_num", "block_num", "par_num", "line_num"))
        except (ValueError, TypeError) as exc:
            raise ValueError("invalid TSV value") from exc
        if not (0 <= x < width and 0 <= y < height and w > 0 and h > 0 and x + w <= width and y + h <= height):
            raise ValueError("word outside page")
        if not 0 <= conf <= 100:
            raise ValueError("invalid OCR confidence")
        lines.setdefault(k, []).append(Word(row["text"].strip(), x, y, w, h, conf, k))
    output = []
    for words in lines.values():
        output.append(OcrLine(words=words, x=min(x.x for x in words), y=min(x.y for x in words),
                              right=max(x.x + x.w for x in words), bottom=max(x.y + x.h for x in words)))
    return sorted(output, key=lambda l: (l.y, l.x))


def classify(lines: list[OcrLine], page_width: int) -> dict:
    """Return indices of body/review OCR lines, never discarding raw words.

    Needs >=2 wide, multiword anchors. Only geometrically connected lines join
    their proven paragraph component. Every remaining line is quarantined for
    manual review; no confidence-only global cutoff and no text is silently lost.
    """
    if not lines:
        return {"status": "needs_review", "body": [], "review": [], "reason": "empty_ocr", "anchor_count": 0}
    anchors = [i for i, line in enumerate(lines)
               if len(line.words) >= 4 and line.characters >= 16
               and line.span >= page_width * 0.27
               and line.height < page_width * 0.1]
    if len(anchors) < 2:
        return {"status": "needs_review", "body": [], "review": list(range(len(lines))),
                "reason": "not_enough_paragraph_anchors", "anchor_count": len(anchors)}
    anchor_h = median(lines[i].height for i in anchors)
    max_gap = anchor_h * 3.2
    # Partition into vertical components based on a line's full extent, not its
    # first pixel. This lets short legitimate final lines join their paragraph.
    components: list[list[int]] = []
    bottom = -1
    for i, line in enumerate(lines):
        if not components or line.y - bottom > max_gap:
            components.append([])
        components[-1].append(i)
        bottom = max(bottom, line.bottom)
    bodies = []
    anchor_set = set(anchors)
    for group in components:
        group_anchor_count = sum(i in anchor_set for i in group)
        if group_anchor_count >= 2:
            bodies.extend(group)
    body = sorted({i for i in bodies if lines[i].height <= anchor_h * 2.5})
    body_set = set(body)
    review = [i for i in range(len(lines)) if i not in body_set]
    if not body:
        return {"status": "needs_review", "body": [], "review": review,
                "reason": "no_coherent_paragraph", "anchor_count": len(anchors)}
    # No independently proofread original: even a page with every word in a
    # paragraph is only an OCR candidate, never a proven Complete extraction.
    return {"status": "partial", "body": body,
            "review": review, "reason": "unverified_ocr_regions" if review else "ocr_not_proofread",
            "anchor_count": len(anchors)}


def metrics(lines: list[OcrLine], result: dict) -> dict:
    all_ids = set(range(len(lines)))
    body_ids = set(result["body"])
    review_ids = set(result["review"])
    if body_ids & review_ids or body_ids | review_ids != all_ids:
        raise AssertionError("loss or duplication of raw OCR lines")
    if result["status"] == "complete" and review_ids:
        raise AssertionError("silent non-indexing cannot claim complete")
    return {"status": result["status"], "reason": result["reason"],
            "raw_words": sum(len(l.words) for l in lines),
            "search_words": sum(len(lines[i].words) for i in body_ids),
            "review_words": sum(len(lines[i].words) for i in review_ids),
            "raw_lines": len(lines), "search_lines": len(body_ids),
            "review_lines": len(review_ids), "anchor_count": result["anchor_count"]}