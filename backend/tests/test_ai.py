import json
import threading
import time
from collections.abc import Iterator
from pathlib import Path

import httpx
import numpy as np
import pytest
import uvicorn
from fastapi.testclient import TestClient

from app.api.ai import inference_options, resolve_chat_model
from app.api.settings import AppSettings
from app.config import Settings
from app.db.connection import connect
from app.engines.knowledge.embeddings import HashingEmbedder
from app.engines.knowledge.grounding import quote_appears_in, verify_segments
from app.engines.knowledge.retrieve import identify_subject, retrieve
from app.engines.knowledge.vector_store import VectorStore
from app.engines.llm.base import CloudModelRefused, LLMError, LLMUnavailable, ModelNotAvailable
from app.engines.llm.ollama import OllamaProvider
from app.main import create_app
from tests.fakes import FakeLLM, model
from tests.pdf_factory import knowledge_book, textbook

CLOUD_TAG = {"name": "remote:latest", "size": 300, "remote_host": "https://ollama.com", "details": {}, "capabilities": ["completion"]}
LOCAL_TAG = {"name": "local:1", "size": 9000, "details": {"family": "f"}, "capabilities": ["completion", "thinking"]}


def ollama(handler) -> OllamaProvider:
    return OllamaProvider(transport=httpx.MockTransport(handler))


def tags_then(handler):
    def wrapped(request: httpx.Request) -> httpx.Response:
        if request.url.path == "/api/tags":
            return httpx.Response(200, json={"models": [LOCAL_TAG, CLOUD_TAG]})
        return handler(request)

    return wrapped


@pytest.fixture
def llm() -> FakeLLM:
    return FakeLLM()


@pytest.fixture
def app_client(settings: Settings, llm: FakeLLM) -> Iterator[TestClient]:
    with TestClient(create_app(settings, llm=llm)) as test_client:
        yield test_client


@pytest.fixture
def book(app_client: TestClient, tmp_path: Path) -> dict:
    path = tmp_path / "knowledge.pdf"
    knowledge_book(path)
    with path.open("rb") as handle:
        created = app_client.post("/api/books", files={"file": ("knowledge.pdf", handle)}).json()
    app_client.app.state.worker.join()
    return app_client.get(f"/api/books/{created['id']}").json()


def ask(client: TestClient, book_id: int, question: str) -> list[dict]:
    response = client.post(f"/api/books/{book_id}/ask", json={"question": question})
    assert response.status_code == 200, response.text
    return [json.loads(line) for line in response.text.splitlines()]


def passage_label(call: dict, needle: str) -> str:
    prompt = call["messages"][1]["content"]
    for block in prompt.split("[P")[1:]:
        if needle in block:
            return "P" + block.split("]")[0]
    raise AssertionError(f"{needle!r} not in prompt: {prompt}")


# --- Model communication ---


def test_cloud_models_are_flagged_by_remote_host() -> None:
    provider = ollama(tags_then(lambda request: httpx.Response(500)))
    flags = {m.name: m.is_cloud for m in provider.list_models()}
    assert flags == {"local:1": False, "remote:latest": True}


def test_cloud_model_refused_even_without_cloud_in_its_name() -> None:
    sent = []
    def handler(request):
        sent.append(request.url.path)
        return httpx.Response(200, json={})
    provider = ollama(tags_then(handler))
    with pytest.raises(CloudModelRefused):
        list(provider.chat_stream("remote:latest", [{"role": "user", "content": "private book text"}]))
    assert "/api/chat" not in sent


def test_cloud_suffix_refused_for_chat_and_pull() -> None:
    provider = ollama(tags_then(lambda request: httpx.Response(200, json={})))
    with pytest.raises(CloudModelRefused):
        list(provider.chat_stream("x:cloud", []))
    with pytest.raises(CloudModelRefused):
        list(provider.pull("x:cloud"))


def test_missing_model_is_reported() -> None:
    provider = ollama(tags_then(lambda request: httpx.Response(200, json={})))
    with pytest.raises(ModelNotAvailable):
        list(provider.chat_stream("absent:1", []))


def test_ollama_down_is_reported() -> None:
    def handler(request):
        raise httpx.ConnectError("refused")
    with pytest.raises(LLMUnavailable):
        ollama(handler).list_models()


def test_chat_streams_and_disables_thinking() -> None:
    bodies = []
    def handler(request):
        bodies.append(json.loads(request.content))
        lines = [
            {"message": {"content": "Hel"}, "done": False},
            {"message": {"content": "lo"}, "done": False},
            {"message": {"content": ""}, "done": True, "prompt_eval_count": 7, "eval_count": 2, "eval_duration": 2_000_000_000},
        ]
        return httpx.Response(200, content="\n".join(map(json.dumps, lines)))
    chunks = list(ollama(tags_then(handler)).chat_stream("local:1", [{"role": "user", "content": "hi"}], options={"num_ctx": 8192}))
    assert "".join(c.delta for c in chunks) == "Hello"
    assert chunks[-1].done and chunks[-1].completion_tokens == 2 and chunks[-1].generation_seconds == 2.0
    assert bodies[0]["think"] is False and bodies[0]["options"] == {"num_ctx": 8192}


def test_error_during_stream_is_raised() -> None:
    def handler(request):
        return httpx.Response(200, content=json.dumps({"error": "out of memory"}))
    with pytest.raises(LLMError, match="out of memory"):
        list(ollama(tags_then(handler)).chat_stream("local:1", []))


def test_resolve_model_never_selects_cloud() -> None:
    cloud = model("remote:1", is_cloud=True)
    assert resolve_chat_model([cloud, model("local:1")], None) == "local:1"
    with pytest.raises(ModelNotAvailable):
        resolve_chat_model([cloud], "remote:1")
    with pytest.raises(ModelNotAvailable):
        resolve_chat_model([cloud], None)


def test_cpu_preference_disables_gpu_layers() -> None:
    assert inference_options(AppSettings(compute_preference="cpu"))["num_gpu"] == 0
    assert "num_gpu" not in inference_options(AppSettings(compute_preference="auto"))


# --- Grounding ---


class P:
    id, section_id, chapter_title, section_title, start_page, end_page = 9, 2, "C", "S", 41, 42
    text = "Mitochondria produce most of the chemical energy that a cell needs."


@pytest.mark.parametrize("quote,expected", [
    ("produce most of the chemical energy", True),
    ("PRODUCE most  of the chemical, energy", True),
    ("produce all of the chemical energy", False),
    ("produce most", False),
    ("", False),
])
def test_quote_matching(quote: str, expected: bool) -> None:
    assert quote_appears_in(quote, P.text) is expected


def test_valid_quote_gets_citation_from_stored_pages() -> None:
    raw = [{"kind": "supported", "text": "They make energy.", "source": "P1", "quote": "most of the chemical energy"}]
    segment = verify_segments(raw, {"P1": P})[0]
    assert segment["kind"] == "supported"
    assert segment["citation"]["start_page"] == 41 and segment["citation"]["end_page"] == 42


@pytest.mark.parametrize("raw", [
    {"kind": "supported", "text": "x", "source": "P1", "quote": "an invented sentence never in the book"},
    {"kind": "supported", "text": "x", "source": "P7", "quote": "most of the chemical energy"},
    {"kind": "supported", "text": "x", "source": None, "quote": None},
])
def test_unverifiable_support_is_downgraded(raw: dict) -> None:
    segment = verify_segments([raw], {"P1": P})[0]
    assert segment["kind"] == "interpretation"
    assert segment["downgraded"] is True
    assert segment["citation"] is None


# --- Retrieval ---


def test_retrieves_relevant_passage_with_pages(app_client: TestClient, book: dict, settings: Settings) -> None:
    conn = connect(settings.db_path)
    question = "What do mitochondria produce?"
    results = retrieve(conn, book["id"], question, identify_subject(conn, book["id"], question), app_client.app.state.vectors, HashingEmbedder())
    conn.close()
    assert "Mitochondria" in results[0].text
    assert results[0].start_page == 2 and results[0].section_title.endswith("Energy in cells")


def test_no_shared_words_retrieves_nothing(app_client: TestClient, book: dict, settings: Settings) -> None:
    conn = connect(settings.db_path)
    question = "Which volcano erupted?"
    results = retrieve(conn, book["id"], question, identify_subject(conn, book["id"], question), app_client.app.state.vectors, HashingEmbedder())
    conn.close()
    assert results == []


def test_current_section_is_preferred(app_client: TestClient, book: dict, settings: Settings) -> None:
    conn = connect(settings.db_path)
    target = conn.execute("SELECT id FROM sections WHERE title LIKE '%Glossary%'").fetchone()["id"]
    question = "gravity"
    results = retrieve(conn, book["id"], question, identify_subject(conn, book["id"], question),
                       app_client.app.state.vectors, HashingEmbedder(), section_id=target)
    conn.close()
    assert results[0].section_id == target


def test_hashing_embedder_is_normalised_and_form_tolerant() -> None:
    vectors = HashingEmbedder().embed(["recursive functions call themselves", "a recursive function", "the weather in spring"])
    assert np.allclose(np.linalg.norm(vectors, axis=1), 1.0)
    assert vectors[0] @ vectors[1] > vectors[0] @ vectors[2]


def test_vector_index_survives_restart_and_tracks_changes(tmp_path: Path, app_client: TestClient, book: dict, settings: Settings) -> None:
    conn = connect(settings.db_path)
    directory = tmp_path / "ünïcode indexes"
    embedder = HashingEmbedder()
    first = VectorStore(directory).search(conn, book["id"], embedder, "mitochondria", 3)
    files = list(directory.glob("*.faiss"))
    assert len(files) == 1
    assert VectorStore(directory).search(conn, book["id"], embedder, "mitochondria", 3) == first

    with conn:
        conn.execute("DELETE FROM passages WHERE id = (SELECT MAX(id) FROM passages)")
    VectorStore(directory).search(conn, book["id"], embedder, "mitochondria", 3)
    assert [f.name for f in directory.glob("*.faiss")] != [files[0].name]
    assert len(list(directory.glob("*.faiss"))) == 1
    conn.close()


def test_removing_book_removes_its_index(app_client: TestClient, book: dict, settings: Settings) -> None:
    conn = connect(settings.db_path)
    app_client.app.state.vectors.search(conn, book["id"], HashingEmbedder(), "energy", 2)
    conn.close()
    assert list((settings.data_dir / "indexes").glob("*.faiss"))
    app_client.delete(f"/api/books/{book['id']}")
    assert not list((settings.data_dir / "indexes").glob("*.faiss"))


# --- Answering ---


def test_answer_is_grounded_and_cites_real_pages(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    def scripted(*args, **kwargs):
        return original(*args, **kwargs)
    original = llm.chat_stream
    llm.reply = {"segments": [
        {"kind": "supported", "text": "Mitochondria make the energy.", "source": "P1", "quote": "Mitochondria produce most of the chemical energy"},
        {"kind": "background", "text": "This happens in nearly all animal cells.", "source": None, "quote": None},
    ]}
    events = ask(app_client, book["id"], "What do mitochondria produce?")
    assert [e["stage"] for e in events if e["type"] == "stage"] == ["retrieving", "generating"]
    answer = events[-1]
    assert answer["type"] == "answer" and answer["found_in_document"] is True
    assert answer["segments"][0]["citation"]["start_page"] == 2
    assert answer["segments"][1]["kind"] == "background"
    assert answer["stats"]["completion_tokens"] == 22


def test_prompt_has_only_relevant_context_not_the_book(app_client: TestClient, tmp_path: Path, llm: FakeLLM) -> None:
    path = tmp_path / "long.pdf"
    textbook(path, chapters=12, sections=4, paragraphs=8)
    with path.open("rb") as handle:
        created = app_client.post("/api/books", files={"file": ("long.pdf", handle)}).json()
    app_client.app.state.worker.join()
    book_text = sum(p["text"].__len__() for s in app_client.get(f"/api/books/{created['id']}/structure").json() for c in s["sections"] for p in app_client.get(f"/api/sections/{c['id']}/passages").json())
    ask(app_client, created["id"], "How does energy relate to motion?")
    prompt = llm.calls[0]["messages"][1]["content"]
    assert len(prompt) < 7000 < book_text


def test_invented_quote_is_not_presented_as_the_book(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    llm.reply = {"segments": [{"kind": "supported", "text": "Claim.", "source": "P1", "quote": "mitochondria were discovered in Paris during winter"}]}
    answer = ask(app_client, book["id"], "What do mitochondria produce?")[-1]
    assert answer["found_in_document"] is False
    assert answer["segments"][0]["kind"] == "interpretation" and answer["segments"][0]["citation"] is None


def test_question_unrelated_to_book_never_reaches_model(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    answer = ask(app_client, book["id"], "Which volcano erupted?")[-1]
    assert answer["found_in_document"] is False
    assert answer["segments"][0]["kind"] == "not_found"
    assert llm.calls == []


def test_model_can_say_not_found(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    llm.reply = {"segments": [{"kind": "not_found", "text": "The document does not say.", "source": None, "quote": None}]}
    answer = ask(app_client, book["id"], "What energy did Einstein describe?")[-1]
    assert answer["found_in_document"] is False and answer["segments"][0]["kind"] == "not_found"


@pytest.mark.parametrize("reply", ["not json at all", "{\"wrong\": 1}", {"segments": []}])
def test_malformed_model_output_becomes_error_event(app_client: TestClient, book: dict, llm: FakeLLM, reply) -> None:
    llm.reply = reply
    assert ask(app_client, book["id"], "What do mitochondria produce?")[-1]["type"] == "error"


def test_ollama_down_becomes_error_event(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    llm.down = True
    last = ask(app_client, book["id"], "What do mitochondria produce?")[-1]
    assert last["type"] == "error" and "Ollama is not running" in last["message"]


def test_selected_cloud_model_cannot_run(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    llm.models = [model("remote:1", is_cloud=True)]
    assert ask(app_client, book["id"], "What do mitochondria produce?")[-1]["type"] == "error"
    assert llm.calls == []


def test_settings_reject_cloud_models(app_client: TestClient) -> None:
    current = app_client.get("/api/settings").json()
    current["llm_model"] = "something:cloud"
    assert app_client.put("/api/settings", json=current).status_code == 422


def test_ask_rejects_unready_or_missing_book(app_client: TestClient, book: dict, settings: Settings) -> None:
    assert app_client.post("/api/books/999/ask", json={"question": "anything?"}).status_code == 404
    conn = connect(settings.db_path)
    with conn:
        conn.execute("UPDATE books SET status = 'indexing' WHERE id = ?", (book["id"],))
    conn.close()
    assert app_client.post(f"/api/books/{book['id']}/ask", json={"question": "anything?"}).status_code == 409


def test_disconnecting_stops_generation(settings: Settings, tmp_path: Path) -> None:
    fake = FakeLLM(reply={"segments": [{"kind": "background", "text": "x" * 4000, "source": None, "quote": None}]})
    fake.chunk_delay = 0.02
    app = create_app(settings, llm=fake)
    server = uvicorn.Server(uvicorn.Config(app, host="127.0.0.1", port=18791, log_level="warning"))
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    try:
        deadline = time.time() + 10
        while not server.started and time.time() < deadline:
            time.sleep(0.05)
        path = tmp_path / "k.pdf"
        knowledge_book(path)
        with httpx.Client(base_url="http://127.0.0.1:18791", timeout=20) as client:
            with path.open("rb") as handle:
                book_id = client.post("/api/books", files={"file": ("k.pdf", handle)}).json()["id"]
            app.state.worker.join()
            with client.stream("POST", f"/api/books/{book_id}/ask", json={"question": "What do mitochondria produce?"}) as response:
                for line in response.iter_lines():
                    if json.loads(line).get("stage") == "generating":
                        break
        deadline = time.time() + 8
        while fake.closed == 0 and time.time() < deadline:
            time.sleep(0.05)
        assert fake.closed == 1, "generator was not closed after the client disconnected"
    finally:
        server.should_exit = True
        thread.join(timeout=10)


# --- Knowledge map ---


def concepts_by_name(client: TestClient, book_id: int) -> dict[str, dict]:
    return {c["name"].lower(): c for c in client.get(f"/api/books/{book_id}/concepts").json()}


def test_glossary_entries_become_defined_concepts(app_client: TestClient, book: dict) -> None:
    concepts = concepts_by_name(app_client, book["id"])
    assert concepts["gravity"]["definition"].startswith("The attraction between masses")
    assert 3 in concepts["gravity"]["pages"]


def test_called_phrases_become_concepts(app_client: TestClient, book: dict) -> None:
    concepts = concepts_by_name(app_client, book["id"])
    assert "osmosis" in concepts
    assert "known as osmosis" in concepts["osmosis"]["definition"]


def test_section_titles_become_concepts_but_generic_ones_do_not(app_client: TestClient, book: dict) -> None:
    concepts = concepts_by_name(app_client, book["id"])
    assert "cell membranes" in concepts or "energy in cells" in concepts
    assert "glossary" not in concepts


def test_definitions_create_relations(app_client: TestClient, book: dict) -> None:
    concepts = concepts_by_name(app_client, book["id"])
    assert "gravity" in concepts["force"]["related"]


def test_enrichment_keeps_only_concepts_with_real_evidence(app_client: TestClient, book: dict, llm: FakeLLM, settings: Settings) -> None:
    llm.reply = {"concepts": [
        {"name": "chemical energy", "definition": "Energy a cell uses.", "quote": "most of the chemical energy that a cell needs"},
        {"name": "dark matter", "definition": "Made up.", "quote": "dark matter binds the galaxies together firmly"},
    ]}
    structure = app_client.get(f"/api/books/{book['id']}/structure").json()
    chapter = next(c for c in structure if "Cells" in c["title"])
    assert app_client.post(f"/api/chapters/{chapter['id']}/concepts/enrich").json() == {"added": 1}
    concepts = concepts_by_name(app_client, book["id"])
    assert "chemical energy" in concepts and "dark matter" not in concepts


def test_enrich_reports_model_failure(app_client: TestClient, book: dict, llm: FakeLLM) -> None:
    llm.down = True
    structure = app_client.get(f"/api/books/{book['id']}/structure").json()
    assert app_client.post(f"/api/chapters/{structure[0]['id']}/concepts/enrich").status_code == 503


# --- Model management API ---


def test_status_lists_local_models_and_names_excluded_cloud(app_client: TestClient, llm: FakeLLM) -> None:
    llm.models = [model("local:1"), model("remote:1", is_cloud=True), model("embed:1", ("embedding",))]
    body = app_client.get("/api/ai/status").json()
    assert body["reachable"] is True and body["active_model"] == "local:1"
    assert [m["name"] for m in body["models"]] == ["local:1", "embed:1"]
    assert body["excluded_cloud_models"] == ["remote:1"]
    assert body["running"][0]["processor"] == "cpu"
    assert body["embedding"] == {"mode": "hashing", "model": None}


def test_status_uses_embedding_model_when_selected(app_client: TestClient, llm: FakeLLM) -> None:
    llm.models = [model("local:1"), model("embed:1", ("embedding",))]
    current = app_client.get("/api/settings").json()
    current["embedding_model"] = "embed:1"
    app_client.put("/api/settings", json=current)
    assert app_client.get("/api/ai/status").json()["embedding"] == {"mode": "model", "model": "embed:1"}


def test_status_when_ollama_is_down(app_client: TestClient, llm: FakeLLM) -> None:
    llm.down = True
    body = app_client.get("/api/ai/status").json()
    assert body["reachable"] is False and "not running" in body["message"]


def test_pull_streams_progress(app_client: TestClient, llm: FakeLLM) -> None:
    response = app_client.post("/api/ai/models/pull", json={"name": "new:1"})
    events = [json.loads(line) for line in response.text.splitlines()]
    assert llm.pulled == ["new:1"]
    assert events[0]["completed"] == 50 and events[-1]["type"] == "done"


def test_delete_model(app_client: TestClient, llm: FakeLLM) -> None:
    assert app_client.delete("/api/ai/models", params={"name": "old:1"}).status_code == 204
    assert llm.deleted == ["old:1"]
