import re
import subprocess
from pathlib import Path

import pymupdf
import pytest
from fastapi.testclient import TestClient

from app.engines.documents.ocr import OcrError, TesseractOcr, parse_tsv
from app.engines.documents.segment import MAX_CHARS, segment_lines
from app.engines.documents.models import Line
from tests.pdf_factory import marker, scanned_copy, textbook, two_column

ocr_available = TesseractOcr.discover() is not None


def upload(client: TestClient, path: Path, wait: bool = True) -> dict:
    with path.open("rb") as handle:
        response = client.post("/api/books", files={"file": (path.name, handle, "application/pdf")})
    assert response.status_code == 201, response.text
    if wait:
        client.app.state.worker.join()
    return client.get(f"/api/books/{response.json()['id']}").json()


def all_passages(client: TestClient, book_id: int) -> list[dict]:
    structure = client.get(f"/api/books/{book_id}/structure").json()
    return [
        passage
        for chapter in structure
        for section in chapter["sections"]
        for passage in client.get(f"/api/sections/{section['id']}/passages").json()
    ]


def assert_page_references_hold(passages: list[dict]) -> int:
    """Every paragraph marker must fall inside the page range of the passage containing it."""
    checked = 0
    for passage in passages:
        for page in map(int, re.findall(r"zqmark(\d+)q", passage["text"])):
            assert passage["start_page"] <= page <= passage["end_page"], passage
            checked += 1
    return checked


def test_extracts_metadata_and_pages(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "book.pdf"
    textbook(path, title="Physics for Tutors", author="N. Newton")
    book = upload(client, path)
    assert book["status"] == "ready", book["status_detail"]
    assert book["title"] == "Physics for Tutors"
    assert book["author"] == "N. Newton"
    with pymupdf.open(path) as doc:
        assert book["page_count"] == doc.page_count


def test_title_falls_back_to_file_name(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "cell_biology-notes.pdf"
    textbook(path, title="", author="")
    assert upload(client, path)["title"] == "cell biology notes"


def test_detects_chapters_from_outline(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "outlined.pdf"
    toc = textbook(path, chapters=4, outline=True)
    book = upload(client, path)
    structure = client.get(f"/api/books/{book['id']}/structure").json()
    chapters = [c for c in structure if c["title"] != "Front matter"]
    assert [c["title"] for c in chapters] == [f"Topic {n}" for n in range(1, 5)]
    assert all(c["detection_method"] == "outline" for c in structure)
    assert [c["start_page"] for c in chapters] == [e[2] for e in toc if e[0] == 1]
    titles = [s["title"] for s in chapters[0]["sections"]]
    assert "Subject 1 1" in titles and "Subject 1 2" in titles


def test_detects_chapters_without_outline(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "plain.pdf"
    toc = textbook(path, chapters=4, outline=False)
    book = upload(client, path)
    structure = client.get(f"/api/books/{book['id']}/structure").json()
    chapters = [c for c in structure if c["title"] != "Front matter"]
    assert [c["title"] for c in chapters] == [f"Chapter {n}: Topic {n}" for n in range(1, 5)]
    assert chapters[0]["detection_method"] == "heading_pattern"
    assert [c["start_page"] for c in chapters] == [e[2] for e in toc if e[0] == 1]
    assert [s["title"] for s in chapters[1]["sections"]][-2:] == ["2.1 Subject 2 1", "2.2 Subject 2 2"]


def test_contents_page_is_not_mistaken_for_chapters(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "with_contents.pdf"
    toc = textbook(path, chapters=3)
    with pymupdf.open(path) as doc:
        contents = doc.new_page(pno=1, width=612, height=792)
        for n in range(1, 7):
            contents.insert_text((72, 100 + n * 20), f"Chapter {n} Topic {n} ........ {n * 10}", fontsize=11)
        fixed = tmp_path / "with_contents_final.pdf"
        doc.save(fixed)
    book = upload(client, fixed)
    structure = client.get(f"/api/books/{book['id']}/structure").json()
    expected = [entry[2] + 1 for entry in toc if entry[0] == 1]
    assert [c["start_page"] for c in structure if c["title"].startswith("Chapter")] == expected
    text = "\n".join(p["text"] for p in all_passages(client, book["id"]))
    assert "........" not in text, "contents pages are not teachable passages"


def test_running_head_naming_a_section_is_removed(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "section_heads.pdf"
    doc = pymupdf.open()
    for n in range(1, 13):
        page = doc.new_page(width=612, height=792)
        # Each head is unique and the printed number runs 4 behind the PDF page.
        if n > 4:
            page.insert_text((72, 40), f"{n // 2}.{n} Unique heading number {n} {n - 4}", fontsize=9)
        page.insert_text((72, 120), f"Body text on page {n} of the document, which is an ordinary sentence.", fontsize=11)
    doc.save(path)
    book = upload(client, path)
    assert "Unique heading" not in "\n".join(p["text"] for p in all_passages(client, book["id"]))


def test_title_is_read_from_title_page_when_metadata_is_empty(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "file_name_only.pdf"
    doc = pymupdf.open()
    page = doc.new_page()
    page.insert_text((72, 200), "Quantum Field Notes", fontsize=32)
    for n in range(10):
        page.insert_text((72, 300 + n * 16), f"Ordinary sentence number {n} on the opening page.", fontsize=11)
    doc.save(path)
    assert upload(client, path)["title"] == "Quantum Field Notes"


def test_unstructured_document_gets_honest_fallback(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "flat.pdf"
    doc = pymupdf.open()
    for n in range(3):
        doc.new_page().insert_text((72, 100), f"Plain body text on page {n}, with nothing to mark it out.", fontsize=11)
    doc.save(path)
    book = upload(client, path)
    structure = client.get(f"/api/books/{book['id']}/structure").json()
    assert [c["title"] for c in structure] == ["Full document"]


@pytest.mark.parametrize("outline", [True, False])
def test_page_references_are_preserved(client: TestClient, tmp_path: Path, outline: bool) -> None:
    path = tmp_path / "pages.pdf"
    textbook(path, chapters=3, sections=3, paragraphs=6, outline=outline)
    book = upload(client, path)
    passages = all_passages(client, book["id"])
    assert assert_page_references_hold(passages) > 20
    # Text must also be readable page by page.
    page = client.get(f"/api/books/{book['id']}/pages/3").json()
    assert marker(3) in page["text"]
    assert page["source"] == "text"


def test_running_heads_and_page_numbers_are_removed(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "heads.pdf"
    textbook(path, chapters=2, paragraphs=8)
    book = upload(client, path)
    text = "\n".join(p["text"] for p in all_passages(client, book["id"]))
    assert "Chapter 1. Topic 1" not in text


def test_passages_follow_paragraphs_and_size_limits(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "chunks.pdf"
    textbook(path, chapters=2, sections=2, paragraphs=10)
    book = upload(client, path)
    passages = all_passages(client, book["id"])
    assert all(len(p["text"]) <= MAX_CHARS * 1.25 for p in passages)
    body = [p for p in passages if p["text"] != "Sample Textbook"]  # the title page
    assert len(body) == len(passages) - 1
    assert all(p["text"].rstrip().endswith(".") for p in body), "passages must end at a sentence"
    assert len({len(p["text"]) for p in passages}) > 1, "chunks must not be uniform lengths"


def test_long_paragraph_splits_at_sentences_and_tracks_pages() -> None:
    lines = []
    for page in (1, 2):
        for n in range(30):
            lines.append(Line(page, f"Sentence number {n} on page {page} keeps going for a while here.", 11, False, 0, 72, 100 + n * 2, 500, 101 + n * 2))
    segments = segment_lines(lines)
    assert len(segments) >= 2
    assert all(len(s.text) <= MAX_CHARS for s in segments)
    assert segments[0].start_page == 1 and segments[-1].end_page == 2


def test_hyphenated_line_breaks_are_rejoined() -> None:
    lines = [
        Line(1, "The mitochon-", 11, False, 0, 72, 100, 500, 112),
        Line(1, "dria produce energy.", 11, False, 0, 72, 113, 300, 125),
    ]
    assert segment_lines(lines)[0].text == "The mitochondria produce energy."


def test_two_column_text_stays_in_reading_order(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "columns.pdf"
    two_column(path)
    book = upload(client, path)
    text = " ".join(p["text"] for p in all_passages(client, book["id"]))
    assert text.index("column0word19") < text.index("column1word0")


def test_search_returns_page_references(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "search.pdf"
    textbook(path, chapters=2, paragraphs=6)
    book = upload(client, path)
    hits = client.get(f"/api/books/{book['id']}/search", params={"q": marker(4)}).json()
    assert hits and all(h["start_page"] <= 4 <= h["end_page"] for h in hits)
    assert "[[" in hits[0]["snippet"]


@pytest.mark.parametrize("query", ['"', "AND OR NOT", "energy* NEAR(", "col:umn"])
def test_search_survives_fts_syntax(client: TestClient, tmp_path: Path, query: str) -> None:
    path = tmp_path / "syntax.pdf"
    textbook(path, chapters=2)
    book = upload(client, path)
    assert client.get(f"/api/books/{book['id']}/search", params={"q": query}).status_code == 200


def test_rejects_non_pdf(client: TestClient) -> None:
    response = client.post("/api/books", files={"file": ("notes.pdf", b"hello world", "application/pdf")})
    assert response.status_code == 415


def test_rejects_duplicate(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "dup.pdf"
    textbook(path, chapters=2)
    upload(client, path)
    with path.open("rb") as handle:
        assert client.post("/api/books", files={"file": ("again.pdf", handle)}).status_code == 409


def test_password_protected_pdf_fails_clearly(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "locked.pdf"
    doc = pymupdf.open()
    doc.new_page().insert_text((72, 100), "secret text")
    doc.save(path, encryption=pymupdf.PDF_ENCRYPT_AES_256, user_pw="pw", owner_pw="pw")
    book = upload(client, path)
    assert book["status"] == "failed"
    assert "password" in book["status_detail"].lower()


def test_corrupt_pdf_fails_clearly(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "corrupt.pdf"
    path.write_bytes(b"%PDF-1.7\n" + b"\x00garbage" * 100)
    book = upload(client, path)
    assert book["status"] == "failed"


def test_scan_without_ocr_engine_fails_clearly(client: TestClient, tmp_path: Path, monkeypatch) -> None:
    source, scan = tmp_path / "s.pdf", tmp_path / "scan.pdf"
    textbook(source, chapters=1, sections=1, paragraphs=1)
    scanned_copy(source, scan, dpi=60)
    monkeypatch.setattr(TesseractOcr, "discover", classmethod(lambda cls, configured=None: None))
    book = upload(client, scan)
    assert book["status"] == "failed"
    assert "OCR engine not installed" in book["status_detail"]


def test_remove_deletes_rows_and_file(client: TestClient, tmp_path: Path, settings) -> None:
    path = tmp_path / "gone.pdf"
    textbook(path, chapters=2)
    book = upload(client, path)
    stored = list((settings.data_dir / "documents").glob("*.pdf"))
    assert len(stored) == 1
    assert client.delete(f"/api/books/{book['id']}").status_code == 204
    assert client.get(f"/api/books/{book['id']}").status_code == 404
    assert not stored[0].exists()


def test_remove_during_processing_does_not_fail(client: TestClient, tmp_path: Path) -> None:
    path = tmp_path / "racing.pdf"
    textbook(path, chapters=8, sections=3, paragraphs=8)
    book = upload(client, path, wait=False)
    assert client.delete(f"/api/books/{book['id']}").status_code == 204
    client.app.state.worker.join()
    assert client.get("/api/books").json() == []


def test_collections_and_title_search(client: TestClient, tmp_path: Path) -> None:
    first, second = tmp_path / "a.pdf", tmp_path / "b.pdf"
    textbook(first, chapters=2, title="Organic Chemistry", paragraphs=2)
    textbook(second, chapters=2, title="Linear Algebra", paragraphs=3)
    a, b = upload(client, first), upload(client, second)

    science = client.post("/api/collections", json={"name": "Science"}).json()
    assert client.post("/api/collections", json={"name": "Science"}).status_code == 409
    client.put(f"/api/books/{a['id']}/collections", json={"collection_ids": [science["id"]]})

    in_collection = client.get("/api/books", params={"collection_id": science["id"]}).json()
    assert [book["id"] for book in in_collection] == [a["id"]]
    found = client.get("/api/books", params={"q": "algebra"}).json()
    assert [book["id"] for book in found] == [b["id"]]
    assert client.get("/api/books", params={"q": "%"}).json() == []

    assert client.delete(f"/api/collections/{science['id']}").status_code == 204
    assert client.get(f"/api/books/{a['id']}").json()["collection_ids"] == []


def test_interrupted_processing_resumes_on_restart(settings, tmp_path: Path) -> None:
    from app.db.connection import connect
    from app.main import create_app

    path = tmp_path / "resume.pdf"
    textbook(path, chapters=2)
    with TestClient(create_app(settings)) as first:
        book = upload(first, path)
    conn = connect(settings.db_path)
    with conn:
        conn.execute("UPDATE books SET status = 'extracting', progress = 0.2 WHERE id = ?", (book["id"],))
    conn.close()
    with TestClient(create_app(settings)) as second:
        second.app.state.worker.join()
        restored = second.get(f"/api/books/{book['id']}").json()
        assert restored["status"] == "ready"
        assert restored["passage_count"] == book["passage_count"]


def test_custom_storage_location_is_used(client: TestClient, tmp_path: Path) -> None:
    custom = tmp_path / "my books"
    current = client.get("/api/settings").json()
    current["document_storage_path"] = str(custom)
    client.put("/api/settings", json=current)
    path = tmp_path / "stored.pdf"
    textbook(path, chapters=2)
    upload(client, path)
    assert len(list(custom.glob("*.pdf"))) == 1


def test_parse_tsv_groups_lines() -> None:
    tsv = "\n".join([
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext",
        "5\t1\t1\t1\t1\t1\t100\t200\t50\t20\t96\tHello",
        "5\t1\t1\t1\t1\t2\t160\t200\t60\t20\t95\tworld",
        "5\t1\t1\t1\t1\t3\t230\t200\t60\t20\t5\tnoise",
        "5\t1\t2\t1\t1\t1\t100\t300\t60\t20\t90\tNext",
    ])
    lines = parse_tsv(tsv, 7)
    assert [line.text for line in lines] == ["Hello world", "Next"]
    assert lines[0].page == 7
    assert lines[0].block != lines[1].block


def test_tesseract_without_tsv_output_is_an_error(tmp_path: Path, monkeypatch) -> None:
    completed = subprocess.CompletedProcess(args=[], returncode=0, stdout=b"", stderr=b"")
    monkeypatch.setattr(subprocess, "run", lambda *args, **kwargs: completed)
    with pytest.raises(OcrError):
        TesseractOcr(tmp_path / "tesseract.exe").recognize(b"png", 3)


@pytest.mark.skipif(not ocr_available, reason="Tesseract is not installed")
def test_scanned_document_is_read_with_ocr(client: TestClient, tmp_path: Path) -> None:
    source, scan = tmp_path / "source.pdf", tmp_path / "scanned.pdf"
    textbook(source, chapters=2, sections=1, paragraphs=2, running_head=False)
    scanned_copy(source, scan)
    with pymupdf.open(scan) as doc:
        assert all(not page.get_text().strip() for page in doc)

    book = upload(client, scan)
    assert book["status"] == "ready", book["status_detail"]
    assert book["needs_ocr"] is True

    structure = client.get(f"/api/books/{book['id']}/structure").json()
    assert [c["title"] for c in structure if c["title"] != "Front matter"] == [
        "Chapter 1: Topic 1", "Chapter 2: Topic 2"
    ]
    passages = all_passages(client, book["id"])
    assert assert_page_references_hold(passages) >= 2
    assert client.get(f"/api/books/{book['id']}/pages/3").json()["source"] == "ocr"
    hits = client.get(f"/api/books/{book['id']}/search", params={"q": "measurable energy"}).json()
    assert hits
