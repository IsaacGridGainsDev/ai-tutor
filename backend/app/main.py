import hmac
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
from starlette.datastructures import Headers
from starlette.types import ASGIApp, Receive, Scope, Send

from app import __version__
from app.api import health, library
from app.config import Settings
from app.db.connection import connect
from app.db.migrate import migrate
from app.engines.documents.worker import IngestWorker

API_PREFIX = "/api"
TOKEN_HEADER = "x-tutor-token"
PUBLIC_PATHS = {f"{API_PREFIX}/health"}


class TokenMiddleware:
    """Plain ASGI middleware: Starlette's BaseHTTPMiddleware hides client disconnects from streams."""

    def __init__(self, app: ASGIApp, token: str | None) -> None:
        self.app = app
        self.token = token

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if (
            self.token
            and scope["type"] == "http"
            and scope["path"] not in PUBLIC_PATHS
            and scope["method"] != "OPTIONS"
        ):
            supplied = Headers(scope=scope).get(TOKEN_HEADER, "")
            if not hmac.compare_digest(supplied, self.token):
                response = JSONResponse({"detail": "Invalid or missing API token"}, status_code=401)
                await response(scope, receive, send)
                return
        await self.app(scope, receive, send)


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings()

    @asynccontextmanager
    async def lifespan(app: FastAPI) -> AsyncIterator[None]:
        settings.data_dir.mkdir(parents=True, exist_ok=True)
        conn = connect(settings.db_path)
        try:
            migrate(conn)
        finally:
            conn.close()
        worker = IngestWorker(settings.db_path, settings.tesseract_path)
        app.state.worker = worker
        worker.start()
        yield
        worker.stop()

    app = FastAPI(title="Interactive Tutor", version=__version__, lifespan=lifespan)
    app.state.settings = settings

    app.add_middleware(TokenMiddleware, token=settings.api_token)

    # Added last so it wraps the token check and 401s still carry CORS headers.
    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.allowed_origins,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    app.include_router(health.router, prefix=API_PREFIX)
    app.include_router(library.router, prefix=API_PREFIX)
    return app
