import sqlite3
import threading
from pathlib import Path

import faiss
import numpy as np

from app.engines.knowledge.embeddings import Embedder


class VectorStore:
    """One FAISS index per book and embedder, rebuilt whenever the book's passages change."""

    def __init__(self, directory: Path) -> None:
        self.directory = directory
        self._lock = threading.Lock()
        self._cache: dict[str, faiss.Index] = {}

    def _name(self, conn: sqlite3.Connection, book_id: int, embedder: Embedder) -> str | None:
        row = conn.execute(
            "SELECT COUNT(*) AS n, MIN(id) AS low, MAX(id) AS high FROM passages WHERE book_id = ?", (book_id,)
        ).fetchone()
        if not row["n"]:
            return None
        return f"book-{book_id}-{embedder.key}-{row['n']}-{row['low']}-{row['high']}.faiss"

    def _build(self, conn: sqlite3.Connection, book_id: int, embedder: Embedder) -> faiss.Index:
        rows = conn.execute("SELECT id, text FROM passages WHERE book_id = ? ORDER BY id", (book_id,)).fetchall()
        vectors = embedder.embed([row["text"] for row in rows])
        index = faiss.IndexIDMap2(faiss.IndexFlatIP(vectors.shape[1]))
        index.add_with_ids(vectors, np.array([row["id"] for row in rows], dtype=np.int64))
        return index

    def _ensure(self, conn: sqlite3.Connection, book_id: int, embedder: Embedder) -> faiss.Index | None:
        name = self._name(conn, book_id, embedder)
        if name is None:
            return None
        with self._lock:
            if name in self._cache:
                return self._cache[name]
            path = self.directory / name
            if path.exists():
                # Serialised through numpy because FAISS file IO fails on non-ASCII Windows paths.
                index = faiss.deserialize_index(np.frombuffer(path.read_bytes(), dtype=np.uint8))
            else:
                index = self._build(conn, book_id, embedder)
                self.directory.mkdir(parents=True, exist_ok=True)
                self._remove_files(book_id, embedder.key)
                path.write_bytes(faiss.serialize_index(index).tobytes())
            self._cache[name] = index
            return index

    def _remove_files(self, book_id: int, key: str = "*") -> None:
        for stale in self.directory.glob(f"book-{book_id}-{key}-*.faiss"):
            stale.unlink(missing_ok=True)

    def search(self, conn: sqlite3.Connection, book_id: int, embedder: Embedder, query: str, limit: int) -> list[tuple[int, float]]:
        index = self._ensure(conn, book_id, embedder)
        if index is None:
            return []
        scores, ids = index.search(embedder.embed([query]), limit)
        return [(int(i), float(s)) for i, s in zip(ids[0], scores[0]) if i != -1]

    def remove(self, book_id: int) -> None:
        with self._lock:
            for name in [n for n in self._cache if n.startswith(f"book-{book_id}-")]:
                del self._cache[name]
            if self.directory.exists():
                self._remove_files(book_id)
