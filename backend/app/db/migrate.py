import re
import sqlite3
from pathlib import Path

MIGRATIONS_DIR = Path(__file__).parent / "migrations"
_FILENAME = re.compile(r"^(\d{4})_[a-z0-9_]+\.sql$")


def _discover() -> list[tuple[int, Path]]:
    found = []
    for path in MIGRATIONS_DIR.glob("*.sql"):
        match = _FILENAME.match(path.name)
        if not match:
            raise RuntimeError(f"Invalid migration filename: {path.name}")
        found.append((int(match.group(1)), path))
    return sorted(found)


def current_version(conn: sqlite3.Connection) -> int:
    conn.execute(
        """CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               applied_at TEXT NOT NULL DEFAULT (datetime('now'))
           )"""
    )
    row = conn.execute("SELECT MAX(version) AS v FROM schema_migrations").fetchone()
    return row["v"] or 0


def migrate(conn: sqlite3.Connection) -> int:
    """Apply pending migrations, each atomically, and return the resulting version."""
    version = current_version(conn)
    for number, path in _discover():
        if number <= version:
            continue
        name = path.name.replace("'", "''")
        script = (
            "BEGIN;\n"
            f"{path.read_text(encoding='utf-8')}\n;"
            f"INSERT INTO schema_migrations (version, name) VALUES ({number}, '{name}');\n"
            "COMMIT;"
        )
        try:
            conn.executescript(script)
        except sqlite3.Error as exc:
            conn.rollback()
            raise RuntimeError(f"Migration {path.name} failed: {exc}") from exc
        version = number
    return version
