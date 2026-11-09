import json
import sqlite3
import time
from collections.abc import Iterator
from dataclasses import asdict
from typing import Any

from app.engines.knowledge.embeddings import Embedder
from app.engines.knowledge.grounding import verify_segments
from app.engines.knowledge.retrieve import Retrieved, identify_subject, related_concepts, retrieve
from app.engines.knowledge.vector_store import VectorStore
from app.engines.llm import prompts
from app.engines.llm.base import LLMError, LLMProvider

MAX_CONTEXT_CHARS = 5000
MAX_PASSAGES = 4
_NOT_IN_DOCUMENT = "I could not find anything about this in the document."


def _context(passages: list[Retrieved]) -> tuple[str, dict[str, Retrieved]]:
    """Label passages for the model, keeping the prompt small enough for CPU inference."""
    parts, labelled, used = [], {}, 0
    for index, passage in enumerate(passages[:MAX_PASSAGES], 1):
        if used and used + len(passage.text) > MAX_CONTEXT_CHARS:
            break
        label = f"P{index}"
        labelled[label] = passage
        used += len(passage.text)
        parts.append(f"[{label}] ({passage.section_title}, page {passage.start_page})\n{passage.text}")
    return "\n\n".join(parts), labelled


def _concept_notes(concepts: list[sqlite3.Row]) -> str:
    lines = [f"- {row['name']}: {row['definition']}" for row in concepts if row["definition"]]
    return "Related concepts from this document:\n" + "\n".join(lines) + "\n" if lines else ""


def _passage_view(label: str, passage: Retrieved) -> dict[str, Any]:
    view = asdict(passage)
    view["label"] = label
    return view


def answer_question(
    conn: sqlite3.Connection, llm: LLMProvider, vectors: VectorStore, embedder: Embedder, book_id: int,
    question: str, model: str, options: dict[str, Any], section_id: int | None = None,
) -> Iterator[dict[str, Any]]:
    """Run the knowledge pipeline, yielding progress events and finally a verified answer."""
    yield {"type": "stage", "stage": "retrieving"}
    subject = identify_subject(conn, book_id, question)
    passages = retrieve(conn, book_id, question, subject, vectors, embedder, section_id=section_id)
    if not passages:
        # Nothing in the book shares a word with the question, so there is nothing to ground on.
        yield {
            "type": "answer",
            "found_in_document": False,
            "segments": [{"kind": "not_found", "text": _NOT_IN_DOCUMENT, "quote": None, "citation": None, "downgraded": False}],
            "passages": [],
            "concepts": [],
            "model": None,
        }
        return

    context, labelled = _context(passages)
    concepts = related_concepts(conn, subject, list(labelled.values()))
    messages = [
        {"role": "system", "content": prompts.GROUNDED_ANSWER_SYSTEM},
        {"role": "user", "content": prompts.GROUNDED_ANSWER_USER.format(
            passages=context, concepts=_concept_notes(concepts), question=question)},
    ]

    yield {"type": "stage", "stage": "generating"}
    content, tokens, started = "", 0, time.perf_counter()
    prompt_tokens = completion_tokens = 0
    generation_seconds = 0.0
    for chunk in llm.chat_stream(model, messages, prompts.GROUNDED_ANSWER_SCHEMA, options):
        content += chunk.delta
        if chunk.delta:
            tokens += 1
            # Frequent events also let a disconnected client be noticed quickly.
            if tokens % 5 == 0:
                yield {"type": "progress", "tokens": tokens}
        if chunk.done:
            prompt_tokens, completion_tokens = chunk.prompt_tokens, chunk.completion_tokens
            generation_seconds = chunk.generation_seconds

    try:
        raw = json.loads(content)["segments"]
    except (json.JSONDecodeError, KeyError, TypeError) as exc:
        raise LLMError("The model returned an answer that could not be read. Please try again.") from exc

    segments = verify_segments(raw, labelled)
    if not segments:
        raise LLMError("The model returned an empty answer. Please try again.")

    yield {
        "type": "answer",
        "found_in_document": any(segment["kind"] == "supported" for segment in segments),
        "segments": segments,
        "passages": [_passage_view(label, passage) for label, passage in labelled.items()],
        "concepts": [row["name"] for row in concepts],
        "model": model,
        "stats": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "generation_seconds": round(generation_seconds, 2),
            "total_seconds": round(time.perf_counter() - started, 2),
        },
    }
