import json
import math
import re
import sqlite3
from typing import Any

from app.engines.knowledge.grounding import quote_appears_in
from app.engines.llm import prompts
from app.engines.llm.base import LLMError, LLMProvider

_GLOSSARY_SECTION = re.compile(r"glossary|key terms|definitions|vocabulary", re.I)
_GENERIC_TITLE = re.compile(
    r"^(introduction|overview|summary|exercises?|glossary|debugging|references?|preface|contents|index|"
    r"acknowledg|conclusion|front matter|further reading|appendix|problems?|review|notes?|full document|pages )",
    re.I,
)
_GLOSSARY_ENTRY = re.compile(r"^([A-Za-z][A-Za-z0-9 '\-]{1,40}):\s+(\S.{10,})$", re.S)
_CALLED = re.compile(
    r"\b(?:is|are)\s+(?:called|known as|referred to as)\s+(?:an?\s+|the\s+)?[\"“]?"
    r"([a-zA-Z][a-zA-Z\-]*(?: [a-zA-Z\-]+){0,3}?)[\"”]?\s*[.,;)]"
)
_SENTENCE = re.compile(r"[^.!?]+[.!?]")
_LEADING_NUMBER = re.compile(r"^[\d.]+\s+")
_MAX_LINKS = 40
_MIN_SHARED_PASSAGES = 3
_MAX_EXCERPT_CHARS = 6000

_WEIGHTS = {"glossary": 1.0, "called": 0.7, "section": 0.4, "model": 0.8}


def normalize_name(name: str) -> str:
    words = re.findall(r"[a-z0-9]+", name.lower())
    if words and words[0] in ("a", "an", "the"):
        words = words[1:]
    return " ".join(words)


def _phrase_matches(conn: sqlite3.Connection, book_id: int, normalized: str) -> list[int]:
    phrase = '"' + normalized.replace('"', "") + '"'
    rows = conn.execute(
        """SELECT p.id FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
           WHERE passages_fts MATCH ? AND p.book_id = ? LIMIT ?""",
        (phrase, book_id, _MAX_LINKS),
    ).fetchall()
    return [row["id"] for row in rows]


def _candidates(conn: sqlite3.Connection, book_id: int) -> dict[str, dict[str, Any]]:
    """Find concepts the document itself marks out, strongest evidence first."""
    found: dict[str, dict[str, Any]] = {}

    def add(name: str, definition: str | None, passage_id: int | None, source: str) -> None:
        name = name.strip()
        key = normalize_name(name)
        if len(key) < 3 or key.isdigit():
            return
        current = found.get(key)
        if current is None or _WEIGHTS[source] > _WEIGHTS[current["source"]]:
            found[key] = {"name": name, "definition": definition, "passage_id": passage_id, "source": source}

    rows = conn.execute(
        """SELECT p.id, p.text, s.title AS section_title, c.title AS chapter_title
           FROM passages p JOIN sections s ON s.id = p.section_id JOIN chapters c ON c.id = p.chapter_id
           WHERE p.book_id = ? ORDER BY p.id""",
        (book_id,),
    ).fetchall()
    for row in rows:
        if _GLOSSARY_SECTION.search(row["section_title"]):
            for paragraph in row["text"].split("\n\n"):
                entry = _GLOSSARY_ENTRY.match(paragraph.strip())
                if entry:
                    add(entry.group(1), entry.group(2).strip(), row["id"], "glossary")
            continue
        for sentence in _SENTENCE.findall(row["text"]):
            called = _CALLED.search(sentence)
            if called:
                add(called.group(1), sentence.strip(), row["id"], "called")

    titles = conn.execute(
        """SELECT DISTINCT s.title, c.title AS chapter_title FROM sections s JOIN chapters c ON c.id = s.chapter_id
           WHERE c.book_id = ?""",
        (book_id,),
    ).fetchall()
    for row in titles:
        title = _LEADING_NUMBER.sub("", row["title"]).strip()
        if title != row["chapter_title"] and len(title.split()) <= 6 and not _GENERIC_TITLE.match(title):
            add(title, None, None, "section")
    return found


def extract_concepts(conn: sqlite3.Connection, book_id: int) -> int:
    """Build the deterministic knowledge map. Runs inside the caller's transaction, with no model."""
    conn.execute("DELETE FROM concepts WHERE book_id = ?", (book_id,))
    candidates = _candidates(conn, book_id)
    ids: dict[str, int] = {}
    links: list[tuple[int, int]] = []
    for key, item in candidates.items():
        passages = set(_phrase_matches(conn, book_id, key))
        if item["passage_id"]:
            passages.add(item["passage_id"])
        if not passages:
            continue
        importance = _WEIGHTS[item["source"]] + math.log1p(len(passages)) / math.log1p(_MAX_LINKS)
        concept_id = conn.execute(
            "INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, ?, ?, ?, ?)",
            (book_id, item["name"], key, item["definition"], round(importance, 3)),
        ).lastrowid
        ids[key] = concept_id
        links += [(passage_id, concept_id) for passage_id in passages]
    conn.executemany("INSERT OR IGNORE INTO passage_concepts (passage_id, concept_id) VALUES (?, ?)", links)

    relations: list[tuple[int, int, str]] = []
    for key, item in candidates.items():
        definition = item["definition"]
        if key in ids and definition:
            text = f" {' '.join(re.findall(r'[a-z0-9]+', definition.lower()))} "
            relations += [
                (ids[key], other_id, "defined_using")
                for other, other_id in ids.items()
                if other != key and f" {other} " in text
            ]
    conn.executemany("INSERT OR IGNORE INTO concept_relations (source_id, target_id, relation) VALUES (?, ?, ?)", relations)
    conn.execute(
        """INSERT OR IGNORE INTO concept_relations (source_id, target_id, relation)
           SELECT a.concept_id, b.concept_id, 'co_occurs'
           FROM passage_concepts a JOIN passage_concepts b
             ON a.passage_id = b.passage_id AND a.concept_id < b.concept_id
           JOIN concepts c ON c.id = a.concept_id
           WHERE c.book_id = ?
           GROUP BY a.concept_id, b.concept_id HAVING COUNT(*) >= ?""",
        (book_id, _MIN_SHARED_PASSAGES),
    )
    return len(ids)


def enrich_chapter_concepts(
    conn: sqlite3.Connection, llm: LLMProvider, chapter_id: int, model: str, options: dict[str, Any]
) -> int:
    """Ask the model for further concepts, keeping only those whose quoted evidence really exists."""
    chapter = conn.execute("SELECT id, book_id, title FROM chapters WHERE id = ?", (chapter_id,)).fetchone()
    passages = conn.execute(
        "SELECT id, text FROM passages WHERE chapter_id = ? ORDER BY id", (chapter_id,)
    ).fetchall()
    if chapter is None or not passages:
        return 0

    excerpt, included = "", []
    for passage in passages:
        if len(excerpt) + len(passage["text"]) > _MAX_EXCERPT_CHARS and excerpt:
            break
        excerpt += passage["text"] + "\n\n"
        included.append(passage)

    messages = [
        {"role": "system", "content": prompts.CONCEPT_EXTRACTION_SYSTEM},
        {"role": "user", "content": prompts.CONCEPT_EXTRACTION_USER.format(title=chapter["title"], excerpt=excerpt)},
    ]
    content = "".join(chunk.delta for chunk in llm.chat_stream(model, messages, prompts.CONCEPT_EXTRACTION_SCHEMA, options))
    try:
        items = json.loads(content)["concepts"]
    except (json.JSONDecodeError, KeyError, TypeError) as exc:
        raise LLMError("The model returned concepts that could not be read. Please try again.") from exc

    added = 0
    with conn:
        for item in items:
            name, definition, quote = (str(item.get(field) or "").strip() for field in ("name", "definition", "quote"))
            key = normalize_name(name)
            source = next((p for p in included if quote_appears_in(quote, p["text"])), None)
            if len(key) < 3 or source is None:
                continue
            existing = conn.execute(
                "SELECT id, definition FROM concepts WHERE book_id = ? AND normalized_name = ?", (chapter["book_id"], key)
            ).fetchone()
            if existing:
                concept_id = existing["id"]
                if not existing["definition"] and definition:
                    conn.execute("UPDATE concepts SET definition = ? WHERE id = ?", (definition, concept_id))
            else:
                concept_id = conn.execute(
                    "INSERT INTO concepts (book_id, name, normalized_name, definition, importance) VALUES (?, ?, ?, ?, ?)",
                    (chapter["book_id"], name, key, definition or None, _WEIGHTS["model"]),
                ).lastrowid
                added += 1
            conn.execute("INSERT OR IGNORE INTO passage_concepts (passage_id, concept_id) VALUES (?, ?)", (source["id"], concept_id))
    return added
