import json
import sqlite3
from collections.abc import AsyncIterator, Iterator
from typing import Annotated, Any

from fastapi import APIRouter, Depends, HTTPException, Query, Request, status
from fastapi.responses import StreamingResponse
from pydantic import BaseModel, Field
from starlette.concurrency import iterate_in_threadpool

from app.api.deps import get_db
from app.api.settings import AppSettings, load_settings
from app.db.connection import connect
from app.engines.documents.concepts import enrich_chapter_concepts
from app.engines.knowledge.answer import answer_question
from app.engines.knowledge.embeddings import Embedder, HashingEmbedder, ModelEmbedder
from app.engines.llm.base import LLMError, LLMProvider, LLMUnavailable, ModelInfo, ModelNotAvailable

router = APIRouter(tags=["ai"])
Db = Annotated[sqlite3.Connection, Depends(get_db)]

# A modest context window: this model's 262144 default would claim far more memory than teaching needs.
_NUM_CTX = 8192
_MAX_ANSWER_TOKENS = 900


class ModelView(BaseModel):
    name: str
    size_bytes: int
    family: str | None
    parameter_size: str | None
    quantization: str | None
    can_chat: bool
    can_embed: bool


class RunningModel(BaseModel):
    name: str
    processor: str
    vram_bytes: int
    size_bytes: int


class EmbeddingStatus(BaseModel):
    mode: str
    model: str | None


class AiStatus(BaseModel):
    reachable: bool
    version: str | None
    message: str | None
    active_model: str | None
    models: list[ModelView]
    excluded_cloud_models: list[str]
    running: list[RunningModel]
    embedding: EmbeddingStatus


class AskRequest(BaseModel):
    question: str = Field(min_length=3, max_length=2000)
    section_id: int | None = None


class PullRequest(BaseModel):
    name: str = Field(min_length=1, max_length=200)


class ConceptView(BaseModel):
    id: int
    name: str
    definition: str | None
    importance: float
    pages: list[int]
    related: list[str]


class EnrichResult(BaseModel):
    added: int


def _view(model: ModelInfo) -> ModelView:
    return ModelView(
        name=model.name, size_bytes=model.size_bytes, family=model.family, parameter_size=model.parameter_size,
        quantization=model.quantization, can_chat=model.can_chat, can_embed=model.can_embed,
    )


def inference_options(settings: AppSettings) -> dict[str, Any]:
    options: dict[str, Any] = {"num_ctx": _NUM_CTX, "temperature": 0.2, "num_predict": _MAX_ANSWER_TOKENS}
    if settings.compute_preference == "cpu":
        options["num_gpu"] = 0
    return options


def resolve_chat_model(models: list[ModelInfo], configured: str | None) -> str:
    local = [model for model in models if not model.is_cloud and model.can_chat]
    if configured:
        if any(model.name == configured for model in local):
            return configured
        raise ModelNotAvailable(f"The selected model {configured} is not installed locally.")
    if not local:
        raise ModelNotAvailable("No local language model is installed. Add one in Settings.")
    return local[0].name


def resolve_embedder(llm: LLMProvider, models: list[ModelInfo], configured: str | None) -> Embedder:
    """Use a real embedding model when one is selected and installed, else the local hashing fallback."""
    if configured and any(model.name == configured and model.can_embed and not model.is_cloud for model in models):
        return ModelEmbedder(llm, configured)
    return HashingEmbedder()


def _processor(entry: dict[str, Any]) -> str:
    vram, size = entry.get("size_vram", 0), entry.get("size", 0)
    if vram == 0:
        return "cpu"
    return "gpu" if vram >= size else "mixed"


def _http_error(exc: LLMError) -> HTTPException:
    code = status.HTTP_503_SERVICE_UNAVAILABLE if isinstance(exc, LLMUnavailable) else status.HTTP_400_BAD_REQUEST
    return HTTPException(code, str(exc))


def _ndjson(events: Iterator[dict[str, Any]]) -> StreamingResponse:
    async def body() -> AsyncIterator[str]:
        try:
            async for event in iterate_in_threadpool(events):
                yield json.dumps(event) + "\n"
        finally:
            # Closing the chain closes the HTTP stream to Ollama, which stops generation.
            events.close()

    return StreamingResponse(body(), media_type="application/x-ndjson")


@router.get("/ai/status", response_model=AiStatus)
def ai_status(request: Request, db: Db) -> AiStatus:
    llm: LLMProvider = request.app.state.llm
    settings = load_settings(db)
    unreachable = dict(reachable=False, version=None, active_model=None, models=[], excluded_cloud_models=[],
                       running=[], embedding=EmbeddingStatus(mode="hashing", model=None))
    try:
        version = llm.version()
        models = llm.list_models()
        running = llm.running()
    except LLMError as exc:
        return AiStatus(message=str(exc), **unreachable)

    try:
        active = resolve_chat_model(models, settings.llm_model)
        message = None
    except LLMError as exc:
        active, message = None, str(exc)

    embedder = resolve_embedder(llm, models, settings.embedding_model)
    return AiStatus(
        reachable=True, version=version, message=message, active_model=active,
        models=[_view(model) for model in models if not model.is_cloud],
        excluded_cloud_models=[model.name for model in models if model.is_cloud],
        running=[
            RunningModel(name=e["name"], processor=_processor(e), vram_bytes=e.get("size_vram", 0), size_bytes=e.get("size", 0))
            for e in running
        ],
        embedding=EmbeddingStatus(
            mode="model" if isinstance(embedder, ModelEmbedder) else "hashing",
            model=embedder.model if isinstance(embedder, ModelEmbedder) else None,
        ),
    )


@router.post("/ai/models/pull")
def pull_model(body: PullRequest, request: Request) -> StreamingResponse:
    llm: LLMProvider = request.app.state.llm

    def events() -> Iterator[dict[str, Any]]:
        try:
            for event in llm.pull(body.name):
                yield {"type": "progress", "status": event.get("status", ""), "total": event.get("total", 0), "completed": event.get("completed", 0)}
            yield {"type": "done"}
        except LLMError as exc:
            yield {"type": "error", "message": str(exc)}

    return _ndjson(events())


@router.delete("/ai/models", status_code=status.HTTP_204_NO_CONTENT)
def delete_model(name: Annotated[str, Query(min_length=1)], request: Request) -> None:
    try:
        request.app.state.llm.delete(name)
    except LLMError as exc:
        raise _http_error(exc) from exc


@router.post("/books/{book_id}/ask")
def ask(book_id: int, body: AskRequest, request: Request, db: Db) -> StreamingResponse:
    book = db.execute("SELECT status FROM books WHERE id = ?", (book_id,)).fetchone()
    if book is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Book not found")
    if book["status"] != "ready":
        raise HTTPException(status.HTTP_409_CONFLICT, "This document is still being processed.")

    app_state = request.app.state

    def events() -> Iterator[dict[str, Any]]:
        conn = connect(app_state.settings.db_path)
        try:
            settings = load_settings(conn)
            models = app_state.llm.list_models()
            yield from answer_question(
                conn, app_state.llm, app_state.vectors,
                resolve_embedder(app_state.llm, models, settings.embedding_model),
                book_id, body.question, resolve_chat_model(models, settings.llm_model),
                inference_options(settings), body.section_id,
            )
        except LLMError as exc:
            yield {"type": "error", "message": str(exc)}
        finally:
            conn.close()

    return _ndjson(events())


@router.get("/books/{book_id}/concepts", response_model=list[ConceptView])
def book_concepts(book_id: int, db: Db, limit: int = 200) -> list[ConceptView]:
    if db.execute("SELECT 1 FROM books WHERE id = ?", (book_id,)).fetchone() is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Book not found")
    rows = db.execute(
        "SELECT id, name, definition, importance FROM concepts WHERE book_id = ? ORDER BY importance DESC, name LIMIT ?",
        (book_id, min(limit, 1000)),
    ).fetchall()
    views = []
    for row in rows:
        pages = db.execute(
            """SELECT DISTINCT p.start_page FROM passages p JOIN passage_concepts pc ON pc.passage_id = p.id
               WHERE pc.concept_id = ? ORDER BY p.start_page LIMIT 8""",
            (row["id"],),
        ).fetchall()
        related = db.execute(
            """SELECT DISTINCT c.name FROM concept_relations r
               JOIN concepts c ON c.id = CASE WHEN r.source_id = ? THEN r.target_id ELSE r.source_id END
               WHERE r.source_id = ? OR r.target_id = ? ORDER BY c.importance DESC LIMIT 6""",
            (row["id"], row["id"], row["id"]),
        ).fetchall()
        views.append(ConceptView(
            id=row["id"], name=row["name"], definition=row["definition"], importance=row["importance"],
            pages=[page["start_page"] for page in pages], related=[item["name"] for item in related],
        ))
    return views


@router.post("/chapters/{chapter_id}/concepts/enrich", response_model=EnrichResult)
def enrich_concepts(chapter_id: int, request: Request, db: Db) -> EnrichResult:
    if db.execute("SELECT 1 FROM chapters WHERE id = ?", (chapter_id,)).fetchone() is None:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Chapter not found")
    llm: LLMProvider = request.app.state.llm
    settings = load_settings(db)
    try:
        model = resolve_chat_model(llm.list_models(), settings.llm_model)
        return EnrichResult(added=enrich_chapter_concepts(db, llm, chapter_id, model, inference_options(settings)))
    except LLMError as exc:
        raise _http_error(exc) from exc
