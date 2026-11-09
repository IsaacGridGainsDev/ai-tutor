import sqlite3
from typing import Annotated

from fastapi import APIRouter, Depends
from pydantic import BaseModel

from app import __version__
from app.api.deps import get_db
from app.db.migrate import current_version

router = APIRouter(tags=["health"])


class HealthResponse(BaseModel):
    status: str
    version: str
    schema_version: int


@router.get("/health", response_model=HealthResponse)
def health(db: Annotated[sqlite3.Connection, Depends(get_db)]) -> HealthResponse:
    return HealthResponse(status="ok", version=__version__, schema_version=current_version(db))
