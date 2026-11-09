import bisect
import re
import statistics
from dataclasses import dataclass, field

from app.engines.documents.models import Line, Segment

TARGET_CHARS = 900
MAX_CHARS = 1600
MIN_TAIL_CHARS = 200
_SENTENCE_END = re.compile(r"[.!?:;][\"')\]]?$")
_SENTENCE_BREAK = re.compile(r"(?<=[.!?])\s+(?=[A-Z(\[\"'])")


@dataclass(slots=True)
class _Paragraph:
    text: str = ""
    offsets: list[int] = field(default_factory=list)  # start offset of each source line
    pages: list[int] = field(default_factory=list)

    def append(self, line: Line) -> None:
        if self.text.endswith("-") and line.text[:1].islower():
            self.text = self.text[:-1]
        elif self.text:
            self.text += " "
        self.offsets.append(len(self.text))
        self.pages.append(line.page)
        self.text += line.text

    def page_at(self, offset: int) -> int:
        return self.pages[max(bisect.bisect_right(self.offsets, offset) - 1, 0)]


def _starts_paragraph(previous: Line, current: Line, line_height: float, right_edge: float) -> bool:
    ended = bool(_SENTENCE_END.search(previous.text))
    if current.page != previous.page:
        return ended
    if current.block != previous.block:
        return ended or not current.text[:1].islower()
    if current.y0 - previous.y1 > line_height * 0.9:
        return True
    return ended and (previous.x1 < right_edge * 0.85 or current.x0 > previous.x0 + 8)


def _paragraphs(lines: list[Line]) -> list[_Paragraph]:
    if not lines:
        return []
    line_height = statistics.median(max(line.y1 - line.y0, 1.0) for line in lines)
    right_edge = max(line.x1 for line in lines)
    paragraphs = [_Paragraph()]
    paragraphs[0].append(lines[0])
    for previous, current in zip(lines, lines[1:]):
        if _starts_paragraph(previous, current, line_height, right_edge):
            paragraphs.append(_Paragraph())
        paragraphs[-1].append(current)
    return paragraphs


def _split_long(paragraph: _Paragraph) -> list[Segment]:
    pieces: list[Segment] = []
    start = 0
    end = 0
    matches = [m.start() for m in _SENTENCE_BREAK.finditer(paragraph.text)] + [len(paragraph.text)]

    def emit(stop: int) -> None:
        text = paragraph.text[start:stop].strip()
        if text:
            pieces.append(Segment(text, paragraph.page_at(start), paragraph.page_at(stop - 1)))

    for boundary in matches:
        if boundary - start > MAX_CHARS and end > start:
            emit(end)
            start = end
            while start < len(paragraph.text) and paragraph.text[start].isspace():
                start += 1
        # A single unbroken "sentence" over the limit is cut hard to respect MAX_CHARS.
        while boundary - start > MAX_CHARS:
            emit(start + MAX_CHARS)
            start += MAX_CHARS
        end = boundary
    if end > start:
        emit(end)
    return pieces


def segment_lines(lines: list[Line]) -> list[Segment]:
    """Group a section's lines into passages, breaking only at paragraph or sentence boundaries."""
    segments: list[Segment] = []
    texts: list[str] = []
    first_page = last_page = 0

    def flush() -> None:
        nonlocal texts
        if texts:
            segments.append(Segment("\n\n".join(texts), first_page, last_page))
            texts = []

    for paragraph in _paragraphs(lines):
        if len(paragraph.text) > MAX_CHARS:
            flush()
            segments.extend(_split_long(paragraph))
            continue
        if texts and sum(map(len, texts)) + len(paragraph.text) > MAX_CHARS:
            flush()
        if not texts:
            first_page = paragraph.pages[0]
        texts.append(paragraph.text)
        last_page = paragraph.pages[-1]
        # A paragraph ending in a colon introduces what follows, so keep them together.
        if sum(map(len, texts)) >= TARGET_CHARS and not paragraph.text.endswith(":"):
            flush()
    flush()

    if len(segments) >= 2 and len(segments[-1].text) < MIN_TAIL_CHARS:
        tail = segments.pop()
        segments[-1].text += "\n\n" + tail.text
        segments[-1].end_page = tail.end_page
    return segments
