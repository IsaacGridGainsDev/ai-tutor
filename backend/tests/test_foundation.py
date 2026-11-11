import sqlite3

import pytest
from fastapi.testclient import TestClient

from app.config import Settings
from app.db.connection import connect
from app.db.migrate import current_version, migrate
from app.main import create_app

EXPECTED_TABLES = {
    "books", "chapters", "sections", "passages", "concepts", "learning_sessions",
    "conversations", "questions", "answers", "assessments", "mastery_records",
    "misconceptions", "revision_schedules", "model_configurations",
}


def table_names(conn: sqlite3.Connection) -> set[str]:
    rows = conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'").fetchall()
    return {row["name"] for row in rows}


def test_health_reports_migrated_schema(client: TestClient) -> None:
    body = client.get("/api/health").json()
    assert body["status"] == "ok"
    assert body["schema_version"] >= 1


def test_schema_contains_required_tables(settings: Settings, client: TestClient) -> None:
    conn = connect(settings.db_path)
    assert EXPECTED_TABLES <= table_names(conn)
    conn.close()


def test_migrate_is_idempotent(tmp_path) -> None:
    conn = connect(tmp_path / "db.sqlite3")
    first = migrate(conn)
    assert migrate(conn) == first == current_version(conn)
    conn.close()


def test_foreign_keys_cascade(tmp_path) -> None:
    conn = connect(tmp_path / "db.sqlite3")
    migrate(conn)
    with conn:
        conn.execute(
            "INSERT INTO books (id, title, file_name, file_path, file_hash) VALUES (1, 't', 'f', 'p', 'h')"
        )
        conn.execute(
            "INSERT INTO chapters (id, book_id, ordinal, title, start_page, end_page, detection_method) "
            "VALUES (1, 1, 1, 'c', 1, 2, 'test')"
        )
        conn.execute("DELETE FROM books WHERE id = 1")
    assert conn.execute("SELECT COUNT(*) AS n FROM chapters").fetchone()["n"] == 0
    conn.close()


def test_fts_tracks_passages(tmp_path) -> None:
    conn = connect(tmp_path / "db.sqlite3")
    migrate(conn)
    with conn:
        conn.execute(
            "INSERT INTO books (id, title, file_name, file_path, file_hash) VALUES (1, 't', 'f', 'p', 'h')"
        )
        conn.execute(
            "INSERT INTO chapters (id, book_id, ordinal, title, start_page, end_page, detection_method) "
            "VALUES (1, 1, 1, 'c', 1, 2, 'test')"
        )
        conn.execute(
            "INSERT INTO sections (id, chapter_id, ordinal, title, start_page, end_page) "
            "VALUES (1, 1, 1, 's', 1, 2)"
        )
        conn.execute(
            "INSERT INTO passages (id, book_id, chapter_id, section_id, ordinal, text, start_page, end_page) "
            "VALUES (1, 1, 1, 1, 1, 'Mitochondria produce energy', 1, 1)"
        )
    query = "SELECT rowid FROM passages_fts WHERE passages_fts MATCH ?"
    assert conn.execute(query, ("produces",)).fetchone()["rowid"] == 1  # porter stemming
    with conn:
        conn.execute("DELETE FROM passages WHERE id = 1")
    assert conn.execute(query, ("produces",)).fetchone() is None
    conn.close()


@pytest.fixture
def secured(tmp_path):
    secured_settings = Settings(data_dir=tmp_path / "data", api_token="s3cret")
    with TestClient(create_app(secured_settings)) as test_client:
        yield test_client


def test_token_required_when_configured(secured: TestClient) -> None:
    # Removing a book that does not exist: refused without the token, looked for with it.
    assert secured.delete("/api/books/999").status_code == 401
    assert secured.delete("/api/books/999", headers={"x-tutor-token": "wrong"}).status_code == 401
    assert secured.delete("/api/books/999", headers={"x-tutor-token": "s3cret"}).status_code == 404


def test_health_is_public_when_token_configured(secured: TestClient) -> None:
    assert secured.get("/api/health").status_code == 200


def test_cors_rejects_foreign_origin(client: TestClient) -> None:
    allowed = client.get("/api/health", headers={"origin": "http://tauri.localhost"})
    assert allowed.headers["access-control-allow-origin"] == "http://tauri.localhost"
    foreign = client.get("/api/health", headers={"origin": "https://evil.example"})
    assert "access-control-allow-origin" not in foreign.headers
