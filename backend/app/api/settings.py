"""The learner's settings, as the code still in Python needs to read them.

The window reads and saves settings through the Rust engine (`engine/core/src/settings.rs`), which
also checks them. This side only reads: the chosen model, and where documents are stored.
"""
import json
import sqlite3
from typing import Literal

from pydantic import BaseModel


class AppSettings(BaseModel):
    llm_model: str | None = None
    embedding_model: str | None = None
    speech_model: str | None = None
    voice: str | None = None
    reading_speed: float = 1.0
    explanation_depth: Literal["brief", "standard", "deep"] = "standard"
    assessment_difficulty: Literal["gentle", "standard", "rigorous"] = "standard"
    compute_preference: Literal["auto", "gpu", "cpu"] = "auto"
    document_storage_path: str | None = None


def load_settings(db: sqlite3.Connection) -> AppSettings:
    rows = db.execute("SELECT key, value FROM app_settings").fetchall()
    stored = {row["key"]: json.loads(row["value"]) for row in rows}
    known = {key: value for key, value in stored.items() if key in AppSettings.model_fields}
    return AppSettings(**known)
