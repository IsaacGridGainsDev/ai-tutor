import hashlib
import shutil
import sqlite3
import time
import uuid
from pathlib import Path
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Query, Request, UploadFile, status
from pydantic import BaseModel, Field

from app.api.deps import get_db
from app.api.settings import load_settings
from app.engines.knowledge.retrieve import fts_query

router = APIRouter(tags=["library"])
Db = Annotated[sqlite3.Connection, Depends(get_db)]

MAX_UPLOAD_BYTES = 2 * 1024**3
_CHUNK = 1024 * 1024
_REMOVE_ATTEMPTS = 20


class Book(BaseModel):
    id: int
    title: str
    author: str | None
    file_name: str
    page_count: int
    status: str
    status_detail: str | None
    progress: float
    needs_ocr: bool
    created_at: str
    chapter_count: int
    passage_count: int
    collection_ids: list[int]


class Collection(BaseModel):
    id: int
    name: str
    book_count: int


class CollectionCreate(BaseModel):
    name: str = Field(min_length=1, max_length=100)


class BookCollections(BaseModel):
    collection_ids: list[int]


class Section(BaseModel):
    id: int
    title: str
    start_page: int
    end_page: int
    passage_count: int


class Chapter(BaseModel):
    id: int
    title: str
    start_page: int
    end_page: int
    detection_method: str
    sections: list[Section]


class Passage(BaseModel):
    id: int
    section_id: int
    ordinal: int
    text: str
    start_page: int
    end_page: int


class SearchHit(BaseModel):
    passage_id: int
    chapter_id: int
    section_id: int
    chapter_title: str
    section_title: str
    snippet: str
    start_page: int
    end_page: int


class Page(BaseModel):
    page_number: int
    label: str | None
    text: str
    source: str


_BOOK_SELECT = """
    SELECT b.*,
        (SELECT COUNT(*) FROM chapters c WHERE c.book_id = b.id) AS chapter_count,
        (SELECT COUNT(*) FROM passages p WHERE p.book_id = b.id) AS passage_count,
        (SELECT group_concat(bc.collection_id) FROM book_collections bc WHERE bc.book_id = b.id) AS collection_ids
    FROM books b
"""


def _to_book(row: sqlite3.Row) -> Book:
    ids = row["collection_ids"]
    return Book(
        **{key: row[key] for key in row.keys() if key in Book.model_fields and key != "collection_ids"},
        collection_ids=[int(value) for value in ids.split(",")] if ids else [],
    )


def _require_book(db: sqlite3.Connection, book_id: int) -> sqlite3.Row:
    row = db.execute(_BOOK_SELECT + " WHERE b.id = ?", (book_id,)).fetchone()
    if row is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Book not found")
    return row


def _storage_dir(request: Request, db: sqlite3.Connection) -> Path:
    configured = load_settings(db).document_storage_path
    directory = Path(configured) if configured else request.app.state.settings.data_dir / "documents"
    try:
        directory.mkdir(parents=True, exist_ok=True)
    except OSError as exc:
        raise HTTPException(status.HTTP_400_BAD_REQUEST, f"Cannot use the document storage folder: {exc}") from exc
    return directory


@router.post("/books", response_model=Book, status_code=status.HTTP_201_CREATED)
def upload_book(file: UploadFile, request: Request, db: Db) -> Book:
    name = Path(file.filename or "document.pdf").name
    if b"%PDF-" not in file.file.read(1024):
        raise HTTPException(status.HTTP_415_UNSUPPORTED_MEDIA_TYPE, "Only PDF documents can be uploaded.")
    file.file.seek(0)

    directory = _storage_dir(request, db)
    temporary = directory / f".upload-{uuid.uuid4().hex}"
    digest = hashlib.sha256()
    size = 0
    try:
        with temporary.open("wb") as target:
            while chunk := file.file.read(_CHUNK):
                size += len(chunk)
                if size > MAX_UPLOAD_BYTES:
                    raise HTTPException(status.HTTP_413_REQUEST_ENTITY_TOO_LARGE, "PDFs are limited to 2 GB.")
                digest.update(chunk)
                target.write(chunk)

        file_hash = digest.hexdigest()
        existing = db.execute("SELECT title FROM books WHERE file_hash = ?", (file_hash,)).fetchone()
        if existing:
            raise HTTPException(status.HTTP_409_CONFLICT, f"This document is already in your library as “{existing['title']}”.")

        destination = directory / f"{file_hash}.pdf"
        shutil.move(temporary, destination)
    finally:
        temporary.unlink(missing_ok=True)

    with db:
        book_id = db.execute(
            "INSERT INTO books (title, file_name, file_path, file_hash) VALUES (?, ?, ?, ?)",
            (Path(name).stem, name, str(destination), file_hash),
        ).lastrowid
    request.app.state.worker.submit(book_id)
    return _to_book(_require_book(db, book_id))


@router.get("/books", response_model=list[Book])
def list_books(db: Db, q: str | None = None, collection_id: int | None = None) -> list[Book]:
    clauses, params = [], []
    if q:
        escaped = q.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
        clauses.append("b.title LIKE ? ESCAPE '\\'")
        params.append(f"%{escaped}%")
    if collection_id is not None:
        clauses.append("EXISTS (SELECT 1 FROM book_collections x WHERE x.book_id = b.id AND x.collection_id = ?)")
        params.append(collection_id)
    where = f" WHERE {' AND '.join(clauses)}" if clauses else ""
    rows = db.execute(f"{_BOOK_SELECT}{where} ORDER BY b.created_at DESC, b.id DESC", params).fetchall()
    return [_to_book(row) for row in rows]


@router.get("/books/{book_id}", response_model=Book)
def get_book(book_id: int, db: Db) -> Book:
    return _to_book(_require_book(db, book_id))


def _remove_file(path: Path) -> None:
    """Delete a stored PDF, allowing for the worker briefly holding it open on Windows."""
    for _ in range(_REMOVE_ATTEMPTS):
        try:
            path.unlink(missing_ok=True)
            return
        except PermissionError:
            time.sleep(0.1)
    raise HTTPException(status.HTTP_409_CONFLICT, "The document is in use. Try removing it again in a moment.")


@router.delete("/books/{book_id}", status_code=status.HTTP_204_NO_CONTENT)
def delete_book(book_id: int, request: Request, db: Db) -> None:
    row = db.execute("SELECT file_path FROM books WHERE id = ?", (book_id,)).fetchone()
    if row is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Book not found")
    _remove_file(Path(row["file_path"]))
    with db:
        db.execute("DELETE FROM books WHERE id = ?", (book_id,))
    request.app.state.vectors.remove(book_id)


@router.put("/books/{book_id}/collections", response_model=Book)
def set_book_collections(book_id: int, body: BookCollections, db: Db) -> Book:
    _require_book(db, book_id)
    try:
        with db:
            db.execute("DELETE FROM book_collections WHERE book_id = ?", (book_id,))
            db.executemany(
                "INSERT INTO book_collections (book_id, collection_id) VALUES (?, ?)",
                [(book_id, collection_id) for collection_id in set(body.collection_ids)],
            )
    except sqlite3.IntegrityError as exc:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Collection not found") from exc
    return _to_book(_require_book(db, book_id))


@router.get("/books/{book_id}/structure", response_model=list[Chapter])
def book_structure(book_id: int, db: Db) -> list[Chapter]:
    _require_book(db, book_id)
    sections: dict[int, list[Section]] = {}
    for row in db.execute(
        """SELECT s.id, s.chapter_id, s.title, s.start_page, s.end_page,
                  (SELECT COUNT(*) FROM passages p WHERE p.section_id = s.id) AS passage_count
           FROM sections s JOIN chapters c ON c.id = s.chapter_id
           WHERE c.book_id = ? ORDER BY s.ordinal""",
        (book_id,),
    ):
        sections.setdefault(row["chapter_id"], []).append(
            Section(**{key: row[key] for key in row.keys() if key != "chapter_id"})
        )
    rows = db.execute("SELECT * FROM chapters WHERE book_id = ? ORDER BY ordinal", (book_id,)).fetchall()
    return [
        Chapter(
            id=row["id"], title=row["title"], start_page=row["start_page"], end_page=row["end_page"],
            detection_method=row["detection_method"], sections=sections.get(row["id"], []),
        )
        for row in rows
    ]


@router.get("/sections/{section_id}/passages", response_model=list[Passage])
def section_passages(section_id: int, db: Db) -> list[Passage]:
    if db.execute("SELECT 1 FROM sections WHERE id = ?", (section_id,)).fetchone() is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Section not found")
    rows = db.execute(
        "SELECT id, section_id, ordinal, text, start_page, end_page FROM passages WHERE section_id = ? ORDER BY ordinal",
        (section_id,),
    ).fetchall()
    return [Passage(**dict(row)) for row in rows]


@router.get("/books/{book_id}/pages/{page_number}", response_model=Page)
def book_page(book_id: int, page_number: int, db: Db) -> Page:
    row = db.execute(
        "SELECT page_number, label, text, source FROM pages WHERE book_id = ? AND page_number = ?",
        (book_id, page_number),
    ).fetchone()
    if row is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Page not found")
    return Page(**dict(row))


@router.get("/books/{book_id}/search", response_model=list[SearchHit])
def search_book(book_id: int, db: Db, q: Annotated[str, Query(min_length=1)], limit: int = 25) -> list[SearchHit]:
    _require_book(db, book_id)
    sql = """
        SELECT p.id AS passage_id, p.chapter_id, p.section_id, c.title AS chapter_title,
               s.title AS section_title, p.start_page, p.end_page,
               snippet(passages_fts, 0, '[[', ']]', '…', 28) AS snippet
        FROM passages_fts
        JOIN passages p ON p.id = passages_fts.rowid
        JOIN chapters c ON c.id = p.chapter_id
        JOIN sections s ON s.id = p.section_id
        WHERE passages_fts MATCH ? AND p.book_id = ?
        ORDER BY bm25(passages_fts) LIMIT ?
    """
    # All terms together first; any term only if that finds nothing.
    for joiner in ("AND", "OR"):
        query = fts_query(q, joiner)
        if not query:
            return []
        rows = db.execute(sql, (query, book_id, min(limit, 100))).fetchall()
        if rows:
            return [SearchHit(**dict(row)) for row in rows]
    return []


@router.get("/collections", response_model=list[Collection])
def list_collections(db: Db) -> list[Collection]:
    rows = db.execute(
        """SELECT c.id, c.name, COUNT(bc.book_id) AS book_count
           FROM collections c LEFT JOIN book_collections bc ON bc.collection_id = c.id
           GROUP BY c.id ORDER BY c.name COLLATE NOCASE"""
    ).fetchall()
    return [Collection(**dict(row)) for row in rows]


@router.post("/collections", response_model=Collection, status_code=status.HTTP_201_CREATED)
def create_collection(body: CollectionCreate, db: Db) -> Collection:
    name = body.name.strip()
    if not name:
        raise HTTPException(status.HTTP_422_UNPROCESSABLE_ENTITY, "Collection name cannot be blank.")
    try:
        with db:
            collection_id = db.execute("INSERT INTO collections (name) VALUES (?)", (name,)).lastrowid
    except sqlite3.IntegrityError as exc:
        raise HTTPException(status.HTTP_409_CONFLICT, "A collection with that name already exists.") from exc
    return Collection(id=collection_id, name=name, book_count=0)


@router.delete("/collections/{collection_id}", status_code=status.HTTP_204_NO_CONTENT)
def delete_collection(collection_id: int, db: Db) -> None:
    with db:
        cursor = db.execute("DELETE FROM collections WHERE id = ?", (collection_id,))
    if cursor.rowcount == 0:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Collection not found")
