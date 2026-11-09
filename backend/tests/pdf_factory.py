"""Generates test PDFs. Every paragraph carries a marker naming the page it was drawn on."""
import textwrap
from pathlib import Path

import pymupdf

WIDTH, HEIGHT = 612, 792
LEFT, TOP, BOTTOM = 72, 90, 700
LINE = 15


def marker(page: int) -> str:
    return f"zqmark{page}q"


class _Writer:
    def __init__(self, doc: pymupdf.Document, running_head: bool) -> None:
        self.doc = doc
        self.running_head = running_head
        self.y = TOP
        self.head = "Sample Textbook"
        self.page = self.new_page()

    def new_page(self) -> pymupdf.Page:
        page = self.doc.new_page(width=WIDTH, height=HEIGHT)
        self.y = TOP
        if self.running_head:
            page.insert_text((LEFT, 40), self.head, fontsize=9)
            page.insert_text((WIDTH - 100, 760), str(page.number + 1), fontsize=9)
        self.page = page
        return page

    def ensure(self, needed: float) -> None:
        if self.y + needed > BOTTOM:
            self.new_page()

    def heading(self, text: str, size: float, font: str = "hebo") -> None:
        self.ensure(size * 3)
        self.page.insert_text((LEFT, self.y), text, fontsize=size, fontname=font)
        self.y += size * 1.8

    def paragraph(self, sentences: int = 7) -> None:
        self.ensure(LINE * 3)
        number = self.page.number + 1
        body = " ".join(
            f"The concept discussed in sentence {i} relates energy to motion in a measurable way." for i in range(sentences)
        )
        for line in textwrap.wrap(f"{marker(number)} {body}", 86):
            self.ensure(LINE)
            self.page.insert_text((LEFT, self.y), line, fontsize=11)
            self.y += LINE
        self.y += LINE


def textbook(
    path: Path, chapters: int = 3, sections: int = 2, paragraphs: int = 5,
    outline: bool = False, title: str = "Sample Textbook", author: str = "A. Author", running_head: bool = True,
) -> list[list]:
    doc = pymupdf.open()
    writer = _Writer(doc, running_head)
    writer.heading(title, 30)
    writer.new_page()
    toc: list[list] = []
    for c in range(1, chapters + 1):
        writer.new_page()
        writer.head = f"Chapter {c}. Topic {c}"
        toc.append([1, f"Topic {c}", writer.page.number + 1])
        writer.heading(f"Chapter {c}", 26)
        writer.heading(f"Topic {c}", 22)
        writer.paragraph(3)
        for s in range(1, sections + 1):
            writer.ensure(80)
            toc.append([2, f"Subject {c} {s}", writer.page.number + 1])
            writer.heading(f"{c}.{s} Subject {c} {s}", 14)
            for _ in range(paragraphs):
                writer.paragraph()
    if outline:
        doc.set_toc(toc)
    doc.set_metadata({"title": title, "author": author})
    doc.save(path)
    doc.close()
    return toc


KNOWLEDGE_CHAPTERS = [
    ("Cells", [
        ("Energy in cells", [
            "Mitochondria produce most of the chemical energy that a cell needs to function.",
            "The energy molecule made by mitochondria is called adenosine triphosphate.",
        ]),
        ("Cell membranes", [
            "A cell membrane is a thin barrier that controls which substances enter the cell.",
            "Movement of water across a membrane is known as osmosis.",
        ]),
    ]),
    ("Motion", [
        ("Forces", [
            "Gravity pulls every object toward the centre of the planet at a steady acceleration.",
            "A push or a pull acting on a body is called a force.",
        ]),
        ("Glossary", [
            "gravity: The attraction between masses that pulls objects toward the planet.",
            "force: A push or pull that changes the motion of a body, as gravity does.",
        ]),
    ]),
]


def knowledge_book(path: Path) -> None:
    """A short book with known facts on known pages, for retrieval and grounding tests."""
    doc = pymupdf.open()
    writer = _Writer(doc, running_head=False)
    writer.heading("Knowledge Book", 30)
    for c, (chapter, sections) in enumerate(KNOWLEDGE_CHAPTERS, 1):
        writer.new_page()
        writer.heading(f"Chapter {c}", 26)
        writer.heading(chapter, 22)
        for s, (title, sentences) in enumerate(sections, 1):
            writer.heading(f"{c}.{s} {title}", 14)
            for sentence in sentences:
                for line in textwrap.wrap(sentence, 86):
                    writer.page.insert_text((LEFT, writer.y), line, fontsize=11)
                    writer.y += LINE
                writer.y += LINE
    doc.set_metadata({"title": "Knowledge Book", "author": "T. Tester"})
    doc.save(path)
    doc.close()


def scanned_copy(source: Path, target: Path, dpi: int = 200) -> None:
    """Rasterise a PDF so that no page contains any selectable text."""
    with pymupdf.open(source) as original, pymupdf.open() as scanned:
        for page in original:
            pix = page.get_pixmap(dpi=dpi, colorspace=pymupdf.csGRAY)
            new = scanned.new_page(width=page.rect.width, height=page.rect.height)
            new.insert_image(new.rect, pixmap=pix)
        scanned.save(target)


def two_column(path: Path) -> None:
    doc = pymupdf.open()
    page = doc.new_page(width=WIDTH, height=HEIGHT)
    for column, x in enumerate((72, 330)):
        y = 90
        for n in range(20):
            page.insert_text((x, y), f"column{column}word{n} continues here,", fontsize=10)
            y += 14
    doc.save(path)
    doc.close()
