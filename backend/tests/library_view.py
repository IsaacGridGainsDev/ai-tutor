"""Reads the library straight from the database, for tests of the code that fills it.

The window gets these views from the Rust engine (`engine/core/src/library.rs` and `search.rs`),
which has its own tests. The Python tests that remain are about processing documents and running
the model, so they look at what was written rather than at how it is served.
"""
import json
import sqlite3
from typing import Any

from fastapi.testclient import TestClient

from app.db.connection import connect

MARK_START = "\u0002"


def _conn(client: TestClient) -> sqlite3.Connection:
    return connect(client.app.state.settings.db_path)


def _rows(client: TestClient, sql: str, *params: Any) -> list[dict]:
    conn = _conn(client)
    try:
        return [dict(row) for row in conn.execute(sql, params)]
    finally:
        conn.close()


def books(client: TestClient) -> list[dict]:
    found = _rows(
        client,
        """SELECT b.*, (SELECT COUNT(*) FROM passages p WHERE p.book_id = b.id) AS passage_count
           FROM books b ORDER BY b.created_at DESC, b.id DESC""",
    )
    for book in found:
        book["needs_ocr"] = bool(book["needs_ocr"])
    return found


def book(client: TestClient, book_id: int) -> dict | None:
    return next((found for found in books(client) if found["id"] == book_id), None)


def structure(client: TestClient, book_id: int) -> list[dict]:
    chapters = _rows(client, "SELECT * FROM chapters WHERE book_id = ? ORDER BY ordinal", book_id)
    for chapter in chapters:
        chapter["sections"] = _rows(client, "SELECT * FROM sections WHERE chapter_id = ? ORDER BY ordinal", chapter["id"])
    return chapters


def passages(client: TestClient, section_id: int) -> list[dict]:
    return _rows(client, "SELECT * FROM passages WHERE section_id = ? ORDER BY ordinal", section_id)


def all_passages(client: TestClient, book_id: int) -> list[dict]:
    return [
        passage
        for chapter in structure(client, book_id)
        for section in chapter["sections"]
        for passage in passages(client, section["id"])
    ]


def page(client: TestClient, book_id: int, page_number: int) -> dict:
    return _rows(client, "SELECT * FROM pages WHERE book_id = ? AND page_number = ?", book_id, page_number)[0]


def find(client: TestClient, book_id: int, text: str) -> list[dict]:
    """Passages of a book holding every given word, to check that what was written is indexed."""
    words = " AND ".join('"' + word.replace('"', "") + '"' for word in text.split())
    return _rows(
        client,
        f"""SELECT p.start_page, p.end_page, snippet(passages_fts, 0, '{MARK_START}', '\u0003', '…', 24) AS snippet
            FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
            WHERE passages_fts MATCH ? AND p.book_id = ?""",
        words, book_id,
    )


def concepts(client: TestClient, book_id: int) -> dict[str, dict]:
    """A book's concepts by lower-case name, each with the pages it is tied to and its related names."""
    found = {}
    for row in _rows(client, "SELECT * FROM concepts WHERE book_id = ?", book_id):
        row["pages"] = [page["start_page"] for page in _rows(
            client,
            "SELECT DISTINCT p.start_page FROM passages p JOIN passage_concepts pc ON pc.passage_id = p.id WHERE pc.concept_id = ?",
            row["id"],
        )]
        row["related"] = [other["name"] for other in _rows(
            client,
            """SELECT DISTINCT c.name FROM concept_relations r
               JOIN concepts c ON c.id = CASE WHEN r.source_id = ? THEN r.target_id ELSE r.source_id END
               WHERE r.source_id = ? OR r.target_id = ?""",
            row["id"], row["id"], row["id"],
        )]
        found[row["name"].lower()] = row
    return found


def set_setting(client: TestClient, key: str, value: Any) -> None:
    """Stores a setting the way the engine stores it: one row per key, the value as JSON."""
    conn = _conn(client)
    try:
        with conn:
            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                (key, json.dumps(value)),
            )
    finally:
        conn.close()
