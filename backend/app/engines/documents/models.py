from dataclasses import dataclass, field


@dataclass(slots=True)
class Line:
    """One visual line of text. Coordinates and size are in PDF points."""

    page: int  # 1-based
    text: str
    size: float
    bold: bool
    block: int
    x0: float
    y0: float
    x1: float
    y1: float


@dataclass(slots=True)
class PageText:
    number: int
    label: str | None
    height: float
    source: str  # 'text', 'ocr' or 'none'
    lines: list[Line] = field(default_factory=list)

    @property
    def text(self) -> str:
        return "\n".join(line.text for line in self.lines)


@dataclass(slots=True)
class SectionSpan:
    title: str
    start: int  # index into the flat line list


@dataclass(slots=True)
class ChapterSpan:
    title: str
    start: int
    end: int  # exclusive
    fallback_page: int
    sections: list[SectionSpan] = field(default_factory=list)


@dataclass(slots=True)
class Structure:
    method: str
    chapters: list[ChapterSpan]
    heading_lines: set[int]


@dataclass(slots=True)
class Segment:
    text: str
    start_page: int
    end_page: int
