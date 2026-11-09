import os
from pathlib import Path

from pydantic import Field
from pydantic_settings import BaseSettings, SettingsConfigDict


def default_data_dir() -> Path:
    base = os.environ.get("LOCALAPPDATA") or str(Path.home() / ".local" / "share")
    return Path(base) / "InteractiveTutor"


class Settings(BaseSettings):
    """Process-level configuration, supplied by the desktop shell through TUTOR_* variables."""

    model_config = SettingsConfigDict(env_prefix="TUTOR_")

    data_dir: Path = Field(default_factory=default_data_dir)
    host: str = "127.0.0.1"
    port: int = 8765
    # Set by the desktop shell on each launch. When present, every request except
    # the health probe must present it, so web pages cannot reach the local API.
    api_token: str | None = None
    tesseract_path: Path | None = None
    ollama_url: str = "http://127.0.0.1:11434"
    allowed_origins: list[str] = [
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "http://tauri.localhost",
        "https://tauri.localhost",
        "tauri://localhost",
    ]

    @property
    def db_path(self) -> Path:
        return self.data_dir / "tutor.sqlite3"
