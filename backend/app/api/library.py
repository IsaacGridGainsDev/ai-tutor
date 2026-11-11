"""Adding and removing books: the two library actions that touch the stored PDF and the work queue.

Everything the window reads from the library is served by the Rust engine
(`engine/core/src/library.rs`). These two move with the document pipeline.
"""
import hashlib
import shutil
import sqlite3
import time
import uuid
from pathlib import Path
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Request, UploadFile, status
from pydantic import BaseModel

from app.api.deps import get_db
from app.api.settings import load_settings

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
    # The search vectors the Rust engine made for the book, and any an older version left.
    for index in (request.app.state.settings.data_dir / "indexes").glob(f"book-{book_id}-*"):
        index.unlink(missing_ok=True)
