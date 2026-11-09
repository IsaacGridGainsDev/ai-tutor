import json
import sqlite3
from typing import Annotated, Literal

from fastapi import APIRouter, Depends, HTTPException, status
from pydantic import BaseModel, Field

from app.api.deps import get_db

router = APIRouter(prefix="/settings", tags=["settings"])


class AppSettings(BaseModel):
    llm_model: str | None = None
    embedding_model: str | None = None
    speech_model: str | None = None
    voice: str | None = None
    reading_speed: float = Field(default=1.0, ge=0.5, le=2.0)
    explanation_depth: Literal["brief", "standard", "deep"] = "standard"
    assessment_difficulty: Literal["gentle", "standard", "rigorous"] = "standard"
    compute_preference: Literal["auto", "gpu", "cpu"] = "auto"
    document_storage_path: str | None = None


def load_settings(db: sqlite3.Connection) -> AppSettings:
    rows = db.execute("SELECT key, value FROM app_settings").fetchall()
    stored = {row["key"]: json.loads(row["value"]) for row in rows}
    known = {key: value for key, value in stored.items() if key in AppSettings.model_fields}
    return AppSettings(**known)


@router.get("", response_model=AppSettings)
def get_settings(db: Annotated[sqlite3.Connection, Depends(get_db)]) -> AppSettings:
    return load_settings(db)


@router.put("", response_model=AppSettings)
def put_settings(
    settings: AppSettings, db: Annotated[sqlite3.Connection, Depends(get_db)]
) -> AppSettings:
    for model in (settings.llm_model, settings.embedding_model):
        if model and model.endswith((":cloud", "-cloud")):
            raise HTTPException(status.HTTP_422_UNPROCESSABLE_ENTITY, "Cloud models cannot be selected.")
    with db:
        db.executemany(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) "
            "ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [(key, json.dumps(value)) for key, value in settings.model_dump().items()],
        )
    return load_settings(db)
