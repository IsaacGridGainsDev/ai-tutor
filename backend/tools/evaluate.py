"""Evaluates retrieval and grounded answering against a real textbook.

Usage: python -m tools.evaluate <think_python.pdf> [--models name1 name2]

Without --models only retrieval is measured, which needs no language model. Questions refer to
Think Python 2e (https://greenteapress.com/thinkpython2/).
"""
import argparse
import json
import shutil
import statistics
import tempfile
import time
from pathlib import Path

from app.db.connection import connect
from app.db.migrate import migrate
from app.engines.documents.ocr import TesseractOcr
from app.engines.documents.pipeline import process_book
from app.engines.knowledge.answer import answer_question
from app.engines.knowledge.embeddings import HashingEmbedder
from app.engines.knowledge.retrieve import identify_subject, lexical_search
from app.engines.knowledge.retrieve import retrieve as hybrid_retrieve
from app.engines.knowledge.vector_store import VectorStore
from app.engines.llm.ollama import OllamaProvider

# (question, expected section title fragment)
RETRIEVAL_GOLD = [
    ("What is an assignment statement?", "Assignment statements"),
    ("Which names are not allowed for variables?", "Variable names"),
    ("In what order are mathematical operators evaluated?", "Order of operations"),
    ("How do I define a new function?", "Adding new functions"),
    ("What does the floor division operator do?", "Floor division and modulus"),
    ("What happens if a recursive function never reaches a base case?", "Infinite recursion"),
    ("How do I read input from the keyboard?", "Keyboard input"),
    ("What is a while loop?", "The while statement"),
    ("Why are strings described as immutable?", "Strings are immutable"),
    ("How do I slice part of a string?", "String slices"),
    ("What does it mean that lists are mutable?", "Lists are mutable"),
    ("What is aliasing with lists?", "Aliasing"),
    ("How does a dictionary store a mapping from keys to values?", "A dictionary is a mapping"),
    ("What is memoization?", "Memos"),
    ("What is tuple assignment?", "Tuple assignment"),
    ("How can I handle exceptions when opening a file?", "Catching exceptions"),
    ("What is a pure function?", "Pure functions"),
    ("What does the init method do?", "The init method"),
    ("How do you define a child class that inherits from a parent?", "Inheritance"),
    ("how to stop an endless loop that keeps repeating", "Debugging"),
]

# (question, expected page range or None when the book does not contain the answer)
ANSWER_GOLD = [
    ("What is a base case in recursion?", (61, 72)),
    ("What does the modulus operator return?", (61, 62)),
    ("What is the difference between a list and a tuple?", (111, 146)),
    ("Who won the football World Cup in 1998?", None),
    ("What is the boiling point of water at sea level?", None),
    ("How do I deploy a Python application to Kubernetes?", None),
]


def ingest(pdf: Path, workdir: Path):
    conn = connect(workdir / "eval.sqlite3")
    migrate(conn)
    copy = workdir / pdf.name
    shutil.copy(pdf, copy)
    with conn:
        conn.execute("INSERT INTO books (id, title, file_name, file_path, file_hash) VALUES (1, 'x', ?, ?, 'h')", (pdf.name, str(copy)))
    process_book(conn, 1, TesseractOcr.discover())
    return conn


def evaluate_retrieval(conn, vectors: VectorStore) -> None:
    embedder = HashingEmbedder()

    def section_of(passage_id: int) -> str:
        return conn.execute(
            "SELECT s.title FROM passages p JOIN sections s ON s.id = p.section_id WHERE p.id = ?", (passage_id,)
        ).fetchone()["title"]

    def rankers(question: str):
        subject = identify_subject(conn, 1, question)
        yield "keyword", lexical_search(conn, 1, subject.terms, 5)
        yield "vector", [i for i, _ in vectors.search(conn, 1, embedder, question, 5)]
        yield "hybrid", [p.id for p in hybrid_retrieve(conn, 1, question, subject, vectors, embedder)]

    results: dict[str, list[int | None]] = {}
    for question, expected in RETRIEVAL_GOLD:
        for name, ids in rankers(question):
            rank = next((i for i, passage in enumerate(ids) if expected.lower() in section_of(passage).lower()), None)
            results.setdefault(name, []).append(rank)
    print(f"retrieval over {len(RETRIEVAL_GOLD)} questions")
    for name, ranks in results.items():
        hit1 = sum(r == 0 for r in ranks) / len(ranks)
        hit5 = sum(r is not None for r in ranks) / len(ranks)
        mrr = sum(1 / (r + 1) for r in ranks if r is not None) / len(ranks)
        print(f"  {name:8s} hit@1={hit1:.2f} hit@5={hit5:.2f} mrr={mrr:.2f}")


def evaluate_model(conn, vectors: VectorStore, llm: OllamaProvider, model: str) -> None:
    options = {"num_ctx": 8192, "temperature": 0.2, "num_predict": 900}
    latencies, speeds, verified, claimed, correct, details = [], [], 0, 0, 0, []
    for question, pages in ANSWER_GOLD:
        started = time.perf_counter()
        answer = None
        for event in answer_question(conn, llm, vectors, HashingEmbedder(), 1, question, model, options):
            if event["type"] == "answer":
                answer = event
        latency = time.perf_counter() - started
        latencies.append(latency)
        segments = answer["segments"]
        if answer.get("stats", {}).get("generation_seconds"):
            speeds.append(answer["stats"]["completion_tokens"] / answer["stats"]["generation_seconds"])
        supported = [s for s in segments if s["kind"] == "supported"]
        downgraded = [s for s in segments if s["downgraded"]]
        verified += len(supported)
        claimed += len(supported) + len(downgraded)
        if pages is None:
            ok = not supported and any(s["kind"] == "not_found" for s in segments)
        else:
            ok = any(pages[0] <= s["citation"]["start_page"] <= pages[1] for s in supported)
        correct += ok
        details.append({"question": question, "ok": ok, "seconds": round(latency, 1), "kinds": [s["kind"] for s in segments],
                        "downgraded": len(downgraded), "first": segments[0]["text"][:140]})
    print(f"model {model}")
    print(f"  correct behaviour {correct}/{len(ANSWER_GOLD)}")
    print(f"  quotes verified   {verified}/{claimed}")
    print(f"  median latency    {statistics.median(latencies):.1f}s")
    print(f"  tokens per second {statistics.mean(speeds):.1f}" if speeds else "  tokens per second n/a")
    print(f"  running on        {[(m['name'], m.get('size_vram', 0)) for m in llm.running()]}")
    print(json.dumps(details, indent=1, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("pdf", type=Path)
    parser.add_argument("--models", nargs="*", default=[])
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        workdir = Path(tmp)
        conn = ingest(args.pdf, workdir)
        vectors = VectorStore(workdir / "indexes")
        print("concepts:", conn.execute("SELECT COUNT(*) AS n FROM concepts").fetchone()["n"],
              "relations:", conn.execute("SELECT COUNT(*) AS n FROM concept_relations").fetchone()["n"])
        evaluate_retrieval(conn, vectors)
        llm = OllamaProvider()
        for model in args.models:
            evaluate_model(conn, vectors, llm, model)
        conn.close()


if __name__ == "__main__":
    main()
