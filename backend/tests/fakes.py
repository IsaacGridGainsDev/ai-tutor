import json
from collections.abc import Iterator
from typing import Any

from app.engines.llm.base import ChatChunk, LLMUnavailable, ModelInfo


def model(name: str = "local:1", capabilities: tuple[str, ...] = ("completion",), is_cloud: bool = False) -> ModelInfo:
    return ModelInfo(name, 1000, "fam", "1B", "Q4", list(capabilities), is_cloud)


class FakeLLM:
    """Scripted provider recording what the engines send to it."""

    def __init__(self, reply: Any = None, models: list[ModelInfo] | None = None) -> None:
        self.reply = reply if reply is not None else {"segments": []}
        self.models = models if models is not None else [model()]
        self.calls: list[dict[str, Any]] = []
        self.down = False
        self.closed = 0
        self.pulled: list[str] = []
        self.deleted: list[str] = []
        self.chunk_delay = 0.0

    def _check(self) -> None:
        if self.down:
            raise LLMUnavailable("Ollama is not running. Start Ollama and try again.")

    def version(self) -> str:
        self._check()
        return "9.9.9"

    def list_models(self) -> list[ModelInfo]:
        self._check()
        return self.models

    def running(self) -> list[dict[str, Any]]:
        self._check()
        return [{"name": "local:1", "size": 100, "size_vram": 0}]

    def chat_stream(self, model: str, messages, schema=None, options=None) -> Iterator[ChatChunk]:
        import time

        self._check()
        self.calls.append({"model": model, "messages": messages, "schema": schema, "options": options})
        text = self.reply if isinstance(self.reply, str) else json.dumps(self.reply)
        try:
            for start in range(0, len(text), 8):
                if self.chunk_delay:
                    time.sleep(self.chunk_delay)
                yield ChatChunk(delta=text[start:start + 8])
            yield ChatChunk(done=True, prompt_tokens=11, completion_tokens=22, generation_seconds=1.5)
        finally:
            self.closed += 1

    def embed(self, model: str, texts: list[str]) -> list[list[float]]:
        return [[float(len(text)), 1.0, 0.5] for text in texts]

    def pull(self, name: str) -> Iterator[dict[str, Any]]:
        self.pulled.append(name)
        yield {"status": "pulling", "total": 100, "completed": 50}
        yield {"status": "success"}

    def delete(self, name: str) -> None:
        self.deleted.append(name)
