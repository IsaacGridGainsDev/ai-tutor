import sqlite3
from collections.abc import Iterator

from fastapi import Request

from app.db.connection import connect


def get_db(request: Request) -> Iterator[sqlite3.Connection]:
    conn = connect(request.app.state.settings.db_path)
    try:
        yield conn
    finally:
        conn.close()
