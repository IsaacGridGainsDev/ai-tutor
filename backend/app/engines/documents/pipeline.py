import os
import sqlite3
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pymupdf

from app.engines.documents import extract
from app.engines.documents.concepts import extract_concepts
from app.engines.documents.models import Line, PageText, Structure
from app.engines.documents.ocr import OCR_DPI, OcrEngine, OcrError
from app.engines.documents.segment import segment_lines
from app.engines.documents.structure import detect_structure

_OCR_WORKERS = max(2, (os.cpu_count() or 4) // 2)
_PROGRESS_EVERY = 10


class ProcessingError(Exception):
    """A failure with a message suitable for showing to the learner."""


class BookRemoved(Exception):
    """The book was deleted from the library while it was being processed."""


class _Progress:
    def __init__(self, conn: sqlite3.Connection, book_id: int) -> None:
        self.conn = conn
        self.book_id = book_id

    def __call__(self, status: str, progress: float, detail: str | None = None) -> None:
        with self.conn:
            cursor = self.conn.execute(
                "UPDATE books SET status = ?, progress = ?, status_detail = ? WHERE id = ?",
                (status, progress, detail, self.book_id),
            )
        if cursor.rowcount == 0:
            raise BookRemoved


def _read_pages(doc: pymupdf.Document, report: Callable[..., None]) -> tuple[list[PageText], list[int]]:
    pages: list[PageText] = []
    needing_ocr: list[int] = []
    for index, page in enumerate(doc):
        lines = extract.native_lines(page, index + 1)
        needs_ocr = extract.page_needs_ocr(page, lines)
        if needs_ocr:
            needing_ocr.append(index)
        pages.append(PageText(index + 1, page.get_label() or None, page.rect.height,
                              "none" if needs_ocr or not lines else "text", lines))
        if index % _PROGRESS_EVERY == 0:
            report("extracting", 0.3 * index / doc.page_count, f"Reading page {index + 1} of {doc.page_count}")
    return pages, needing_ocr


def _run_ocr(
    doc: pymupdf.Document, pages: list[PageText], needing: list[int], ocr: OcrEngine, report: Callable[..., None]
) -> list[int]:
    """OCR pages in small batches to bound memory; rendering stays on this thread."""
    failed: list[int] = []
    batch_size = _OCR_WORKERS * 2

    def recognize(item: tuple[int, bytes]) -> tuple[int, list[Line] | None]:
        index, png = item
        try:
            return index, ocr.recognize(png, index + 1)
        except OcrError:
            return index, None

    with ThreadPoolExecutor(_OCR_WORKERS) as pool:
        for start in range(0, len(needing), batch_size):
            report("ocr", 0.3 + 0.5 * start / len(needing),
                   f"Recognising scanned page {start + 1} of {len(needing)}")
            batch = [
                (index, doc[index].get_pixmap(dpi=OCR_DPI, colorspace=pymupdf.csGRAY).tobytes("png"))
                for index in needing[start:start + batch_size]
            ]
            for index, lines in pool.map(recognize, batch):
                if lines is None:
                    failed.append(index + 1)
                elif lines:
                    pages[index].lines = lines
                    pages[index].source = "ocr"
    return failed


def _store(
    conn: sqlite3.Connection, book_id: int, pages: list[PageText], flat: list[Line],
    structure: Structure, title: str, author: str | None, page_count: int, used_ocr: bool, detail: str | None,
) -> None:
    with conn:
        conn.execute("DELETE FROM pages WHERE book_id = ?", (book_id,))
        conn.execute("DELETE FROM chapters WHERE book_id = ?", (book_id,))
        conn.executemany(
            "INSERT INTO pages (book_id, page_number, label, text, source) VALUES (?, ?, ?, ?, ?)",
            [(book_id, p.number, p.label, p.text, p.source) for p in pages],
        )
        for chapter_ordinal, chapter in enumerate(structure.chapters, 1):
            chapter_lines = flat[chapter.start:chapter.end]
            start_page = chapter_lines[0].page if chapter_lines else chapter.fallback_page
            end_page = chapter_lines[-1].page if chapter_lines else chapter.fallback_page
            chapter_id = conn.execute(
                "INSERT INTO chapters (book_id, ordinal, title, start_page, end_page, detection_method) "
                "VALUES (?, ?, ?, ?, ?, ?)",
                (book_id, chapter_ordinal, chapter.title, start_page, end_page, structure.method),
            ).lastrowid

            for section_ordinal, section in enumerate(chapter.sections, 1):
                following = chapter.sections[section_ordinal:]
                stop = following[0].start if following else chapter.end
                body = [
                    flat[i] for i in range(section.start, stop) if i not in structure.heading_lines
                ]
                span = flat[section.start:stop]
                section_id = conn.execute(
                    "INSERT INTO sections (chapter_id, ordinal, title, start_page, end_page) VALUES (?, ?, ?, ?, ?)",
                    (chapter_id, section_ordinal, section.title,
                     span[0].page if span else start_page, span[-1].page if span else end_page),
                ).lastrowid
                conn.executemany(
                    "INSERT INTO passages (book_id, chapter_id, section_id, ordinal, text, start_page, end_page) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?)",
                    [
                        (book_id, chapter_id, section_id, ordinal, seg.text, seg.start_page, seg.end_page)
                        for ordinal, seg in enumerate(segment_lines(body), 1)
                    ],
                )
        extract_concepts(conn, book_id)
        cursor = conn.execute(
            "UPDATE books SET title = ?, author = ?, page_count = ?, needs_ocr = ?, "
            "status = 'ready', progress = 1, status_detail = ? WHERE id = ?",
            (title, author, page_count, int(used_ocr), detail, book_id),
        )
        if cursor.rowcount == 0:
            raise BookRemoved


def process_book(conn: sqlite3.Connection, book_id: int, ocr: OcrEngine | None) -> None:
    """Extract, OCR, structure and index one book, recording any failure on its row."""
    report = _Progress(conn, book_id)
    row = conn.execute("SELECT file_path, file_name FROM books WHERE id = ?", (book_id,)).fetchone()
    if row is None:
        return
    try:
        report("extracting", 0.0, "Opening document")
        try:
            # Read into memory so Windows does not lock the file against removal.
            doc = pymupdf.open(stream=Path(row["file_path"]).read_bytes(), filetype="pdf")
        except (OSError, RuntimeError, ValueError) as exc:
            raise ProcessingError(f"This file could not be opened as a PDF: {exc}") from exc
        with doc:
            if doc.needs_pass:
                raise ProcessingError("This PDF is password protected. Remove the password and upload it again.")
            pages, needing_ocr = _read_pages(doc, report)

            detail = None
            failed: list[int] = []
            if needing_ocr and ocr:
                failed = _run_ocr(doc, pages, needing_ocr, ocr, report)
            elif needing_ocr:
                detail = f"OCR engine not installed; {len(needing_ocr)} scanned pages have no text."
            if failed:
                detail = f"OCR failed on pages: {', '.join(map(str, failed[:20]))}" + ("…" if len(failed) > 20 else "")

            report("structuring", 0.85, "Finding chapters and sections")
            title, author = extract.book_metadata(doc, row["file_name"], pages)
            extract.remove_page_furniture(pages)
            flat = [line for page in pages for line in page.lines]
            if not flat:
                raise ProcessingError(detail or "No readable text was found in this document.")
            structure = detect_structure(flat, doc.get_toc(simple=True), doc.page_count)

            report("indexing", 0.95, "Building the search index")
            _store(conn, book_id, pages, flat, structure, title, author, doc.page_count,
                   any(page.source == "ocr" for page in pages), detail)
    except BookRemoved:
        return
    except sqlite3.IntegrityError:
        # The row vanished between the last progress update and the final write.
        conn.rollback()
        if conn.execute("SELECT 1 FROM books WHERE id = ?", (book_id,)).fetchone() is not None:
            raise
    except ProcessingError as exc:
        _fail(conn, book_id, str(exc))
    except Exception as exc:
        _fail(conn, book_id, f"Unexpected processing error: {exc}")
        raise


def _fail(conn: sqlite3.Connection, book_id: int, message: str) -> None:
    conn.rollback()
    with conn:
        conn.execute("UPDATE books SET status = 'failed', status_detail = ? WHERE id = ?", (message, book_id))
