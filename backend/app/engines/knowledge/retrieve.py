import re
import sqlite3
from dataclasses import dataclass, field

from app.engines.knowledge.embeddings import Embedder, content_words
from app.engines.knowledge.vector_store import VectorStore

_RRF_K = 60
_CANDIDATES = 30
_CURRENT_SECTION_BOOST = 0.01


def fts_query(text: str, joiner: str) -> str:
    """Quote every token so user input can never be parsed as FTS5 syntax."""
    tokens = re.findall(r"\w+", text)
    return f" {joiner} ".join(f'"{token}"' for token in tokens)


@dataclass(slots=True)
class Retrieved:
    id: int
    text: str
    chapter_id: int
    section_id: int
    chapter_title: str
    section_title: str
    start_page: int
    end_page: int
    score: float


@dataclass(slots=True)
class Subject:
    terms: list[str]
    concepts: list[sqlite3.Row] = field(default_factory=list)


def identify_subject(conn: sqlite3.Connection, book_id: int, question: str) -> Subject:
    """Work out what the question is about from its content words and the book's known concepts."""
    terms = content_words(question)
    lowered = f" {' '.join(re.findall(r'[a-z0-9]+', question.lower()))} "
    rows = conn.execute(
        "SELECT id, name, normalized_name, definition FROM concepts WHERE book_id = ?", (book_id,)
    ).fetchall()
    matched = [row for row in rows if f" {row['normalized_name']} " in lowered]
    matched.sort(key=lambda row: len(row["normalized_name"]), reverse=True)
    return Subject(terms, matched)


def lexical_search(conn: sqlite3.Connection, book_id: int, terms: list[str], limit: int) -> list[int]:
    query = " OR ".join(f'"{term}"' for term in terms)
    if not query:
        return []
    rows = conn.execute(
        """SELECT p.id FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
           WHERE passages_fts MATCH ? AND p.book_id = ? ORDER BY bm25(passages_fts) LIMIT ?""",
        (query, book_id, limit),
    ).fetchall()
    return [row["id"] for row in rows]


def _load(conn: sqlite3.Connection, scored: list[tuple[int, float]]) -> list[Retrieved]:
    if not scored:
        return []
    marks = ",".join("?" * len(scored))
    rows = conn.execute(
        f"""SELECT p.id, p.text, p.chapter_id, p.section_id, p.start_page, p.end_page,
                   c.title AS chapter_title, s.title AS section_title
            FROM passages p JOIN chapters c ON c.id = p.chapter_id JOIN sections s ON s.id = p.section_id
            WHERE p.id IN ({marks})""",
        [passage_id for passage_id, _ in scored],
    ).fetchall()
    by_id = {row["id"]: row for row in rows}
    return [Retrieved(**dict(by_id[i]), score=score) for i, score in scored if i in by_id]


def retrieve(
    conn: sqlite3.Connection, book_id: int, question: str, subject: Subject, vectors: VectorStore,
    embedder: Embedder, limit: int = 5, section_id: int | None = None,
) -> list[Retrieved]:
    """Fuse keyword and vector rankings; a passage needs at least one shared content word to qualify."""
    lexical = lexical_search(conn, book_id, subject.terms, _CANDIDATES)
    if not lexical:
        return []
    vector = [passage_id for passage_id, _ in vectors.search(conn, book_id, embedder, question, _CANDIDATES)]

    fused: dict[int, float] = {}
    for ranking in (lexical, vector):
        for rank, passage_id in enumerate(ranking):
            fused[passage_id] = fused.get(passage_id, 0.0) + 1.0 / (_RRF_K + rank + 1)

    for row in _concept_passages(conn, subject):
        if row in fused:
            fused[row] += 0.005

    results = _load(conn, sorted(fused.items(), key=lambda item: item[1], reverse=True))
    if section_id is not None:
        for item in results:
            if item.section_id == section_id:
                item.score += _CURRENT_SECTION_BOOST
        results.sort(key=lambda item: item.score, reverse=True)
    return results[:limit]


def _concept_passages(conn: sqlite3.Connection, subject: Subject) -> list[int]:
    if not subject.concepts:
        return []
    marks = ",".join("?" * len(subject.concepts))
    rows = conn.execute(
        f"SELECT DISTINCT passage_id FROM passage_concepts WHERE concept_id IN ({marks})",
        [concept["id"] for concept in subject.concepts],
    ).fetchall()
    return [row["passage_id"] for row in rows]


def related_concepts(conn: sqlite3.Connection, subject: Subject, passages: list[Retrieved], limit: int = 6) -> list[sqlite3.Row]:
    """Concepts attached to the retrieved passages, those the question names first."""
    named = {concept["id"]: concept for concept in subject.concepts}
    if passages:
        marks = ",".join("?" * len(passages))
        rows = conn.execute(
            f"""SELECT c.id, c.name, c.normalized_name, c.definition FROM concepts c
                JOIN passage_concepts pc ON pc.concept_id = c.id
                WHERE pc.passage_id IN ({marks}) AND c.definition IS NOT NULL
                GROUP BY c.id ORDER BY COUNT(*) DESC, c.importance DESC LIMIT ?""",
            [item.id for item in passages] + [limit],
        ).fetchall()
        for row in rows:
            named.setdefault(row["id"], row)
    return list(named.values())[:limit]
