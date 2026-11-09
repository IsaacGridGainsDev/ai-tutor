import re
from collections import Counter
from pathlib import Path

import pymupdf

from app.engines.documents.models import Line, PageText

BOLD_FLAG = 16
MIN_NATIVE_CHARS = 20
MARGIN_FRACTION = 0.09
FURNITURE_MIN_PAGES = 3
MAX_RUNNING_HEAD_CHARS = 100
TITLE_SIZE_RATIO = 1.4
_NOT_A_TITLE = re.compile(r"^(chapter|part|unit|contents|table of contents|preface|introduction|topic)\b", re.I)
_LEADING_INT = re.compile(r"^(\d{1,4})\s")
_TRAILING_INT = re.compile(r"\s(\d{1,4})$")
_PLACEHOLDER_TITLE = re.compile(r"^(untitled|microsoft word|.*\.(dvi|pdf|docx?|tex|indd|qxd))", re.I)
_PAGE_NUMBER = re.compile(r"^(page\s+)?(\d{1,4}|[ivxlc]{1,7})$", re.I)
_TEXT_FLAGS = pymupdf.TEXTFLAGS_DICT & ~pymupdf.TEXT_PRESERVE_LIGATURES & ~pymupdf.TEXT_PRESERVE_IMAGES


def clean(text: str) -> str:
    return re.sub(r"\s+", " ", text.replace("\x00", "").replace("\xad", "")).strip()


def native_lines(page: pymupdf.Page, number: int) -> list[Line]:
    """Read lines in content-stream order, which keeps multi-column text in reading order."""
    lines: list[Line] = []
    for block_index, block in enumerate(page.get_text("dict", flags=_TEXT_FLAGS)["blocks"]):
        for raw in block.get("lines", []):
            spans = [span for span in raw["spans"] if span["text"].strip()]
            text = clean("".join(span["text"] for span in raw["spans"]))
            if not spans or not text:
                continue
            widest = max(spans, key=lambda span: len(span["text"]))
            x0, y0, x1, y1 = raw["bbox"]
            lines.append(
                Line(number, text, round(widest["size"], 1), bool(widest["flags"] & BOLD_FLAG),
                     block_index, x0, y0, x1, y1)
            )
    return lines


def page_needs_ocr(page: pymupdf.Page, lines: list[Line]) -> bool:
    return sum(len(line.text) for line in lines) < MIN_NATIVE_CHARS and bool(page.get_images())


def book_metadata(doc: pymupdf.Document, file_name: str, pages: list[PageText]) -> tuple[str, str | None]:
    meta = doc.metadata or {}
    title = clean(meta.get("title") or "")
    author = clean(meta.get("author") or "") or None
    if not title or _PLACEHOLDER_TITLE.match(title):
        title = (
            title_from_first_pages(pages)
            or clean(Path(file_name).stem.replace("_", " ").replace("-", " "))
            or "Untitled document"
        )
    return title, author


def title_from_first_pages(pages: list[PageText]) -> str | None:
    """The largest text on the opening pages, for PDFs that carry no title metadata."""
    candidates = [
        line for page in pages[:3] for line in page.lines
        if len(line.text) >= 4 and not _NOT_A_TITLE.match(line.text)
    ]
    if not candidates:
        return None
    sizes = sorted(line.size for line in candidates)
    largest = max(candidates, key=lambda line: line.size)
    if largest.size < sizes[len(sizes) // 2] * TITLE_SIZE_RATIO:
        return None
    wrapped = [
        line.text for line in candidates
        if line.page == largest.page and abs(line.size - largest.size) < 0.5
    ]
    return " ".join(wrapped[:3])


def _printed_page_offset(pages: list[PageText], in_margin) -> int | None:
    """The gap between PDF page index and printed page number, if the book has a consistent one."""
    offsets: Counter[int] = Counter()
    for page in pages:
        found = set()
        for line in page.lines:
            for match in (_LEADING_INT.match(line.text), _TRAILING_INT.search(line.text)):
                if match and in_margin(page, line):
                    found.add(page.number - int(match.group(1)))
        offsets.update(found)
    if not offsets:
        return None
    offset, count = offsets.most_common(1)[0]
    return offset if count >= max(FURNITURE_MIN_PAGES, len(pages) // 4) else None


def remove_page_furniture(pages: list[PageText]) -> None:
    """Strip page numbers and running headers or footers from the top and bottom margins."""

    def in_margin(page: PageText, line: Line) -> bool:
        limit = page.height * MARGIN_FRACTION
        return line.y1 <= limit or line.y0 >= page.height - limit

    offset = _printed_page_offset(pages, in_margin)

    def carries_page_number(page: PageText, line: Line) -> bool:
        """Catches running heads that name the current section and so never repeat exactly."""
        if offset is None or len(line.text) > MAX_RUNNING_HEAD_CHARS:
            return False
        matches = (_LEADING_INT.match(line.text), _TRAILING_INT.search(line.text))
        return any(match and page.number - int(match.group(1)) == offset for match in matches)

    # Coarse size is part of the key so a chapter heading survives its smaller running head,
    # while the size jitter between OCR'd copies of one running head does not split it.
    def key(line: Line) -> tuple[str, int]:
        return re.sub(r"\d+", "#", line.text.lower()), round(line.size / 4)

    seen: Counter[tuple[str, int]] = Counter()
    for page in pages:
        seen.update({key(line) for line in page.lines if in_margin(page, line)})

    for page in pages:
        page.lines = [
            line
            for line in page.lines
            if not (
                in_margin(page, line)
                and (
                    seen[key(line)] >= FURNITURE_MIN_PAGES
                    or _PAGE_NUMBER.match(line.text)
                    or carries_page_number(page, line)
                )
            )
        ]
