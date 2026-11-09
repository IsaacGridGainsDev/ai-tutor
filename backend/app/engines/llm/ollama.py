import json
from collections.abc import Iterator
from typing import Any

import httpx

from app.engines.llm.base import (
    ChatChunk,
    CloudModelRefused,
    LLMError,
    LLMUnavailable,
    ModelInfo,
    ModelNotAvailable,
)

DEFAULT_URL = "http://127.0.0.1:11434"
_KEEP_ALIVE = "10m"
_NOT_RUNNING = "Ollama is not running. Start Ollama and try again."


def is_cloud_entry(entry: dict[str, Any]) -> bool:
    """Cloud models appear in the local list but forward prompts to a remote host."""
    name = entry.get("name", "")
    return bool(entry.get("remote_host") or entry.get("remote_model")) or name.endswith((":cloud", "-cloud"))


def _is_cloud_name(name: str) -> bool:
    return name.endswith((":cloud", "-cloud"))


class OllamaProvider:
    def __init__(self, base_url: str = DEFAULT_URL, transport: httpx.BaseTransport | None = None) -> None:
        self._client = httpx.Client(
            base_url=base_url,
            timeout=httpx.Timeout(connect=3, read=900, write=30, pool=5),
            transport=transport,
        )

    def _get(self, path: str) -> dict[str, Any]:
        try:
            response = self._client.get(path)
        except httpx.TransportError as exc:
            raise LLMUnavailable(_NOT_RUNNING) from exc
        return self._json(response)

    @staticmethod
    def _json(response: httpx.Response) -> dict[str, Any]:
        if response.status_code == 404:
            raise ModelNotAvailable(_error_text(response))
        if response.is_error:
            raise LLMError(f"Ollama error: {_error_text(response)}")
        return response.json()

    def version(self) -> str:
        return self._get("/api/version")["version"]

    def list_models(self) -> list[ModelInfo]:
        models = []
        for entry in self._get("/api/tags").get("models", []):
            details = entry.get("details") or {}
            models.append(
                ModelInfo(
                    name=entry["name"],
                    size_bytes=entry.get("size", 0),
                    family=details.get("family") or None,
                    parameter_size=details.get("parameter_size") or None,
                    quantization=details.get("quantization_level") or None,
                    capabilities=entry.get("capabilities") or [],
                    is_cloud=is_cloud_entry(entry),
                )
            )
        return models

    def _require_local(self, model: str) -> ModelInfo:
        """Enforced on every inference call, so no setting can send book text off this machine."""
        if _is_cloud_name(model):
            raise CloudModelRefused(f"{model} is a cloud model. Documents must stay on this computer.")
        for info in self.list_models():
            if info.name == model:
                if info.is_cloud:
                    raise CloudModelRefused(f"{model} is a cloud model. Documents must stay on this computer.")
                return info
        raise ModelNotAvailable(f"The model {model} is not installed.")

    def chat_stream(
        self, model: str, messages: list[dict[str, str]], schema: dict[str, Any] | None = None,
        options: dict[str, Any] | None = None,
    ) -> Iterator[ChatChunk]:
        info = self._require_local(model)
        payload: dict[str, Any] = {
            "model": model,
            "messages": messages,
            "stream": True,
            "keep_alive": _KEEP_ALIVE,
            "options": options or {},
        }
        if schema:
            payload["format"] = schema
        if "thinking" in info.capabilities:
            payload["think"] = False
        yield from self._stream_chat(payload)

    def _stream_chat(self, payload: dict[str, Any]) -> Iterator[ChatChunk]:
        try:
            with self._client.stream("POST", "/api/chat", json=payload) as response:
                if response.is_error:
                    response.read()
                    self._json(response)
                for line in response.iter_lines():
                    if not line:
                        continue
                    event = json.loads(line)
                    if "error" in event:
                        raise LLMError(f"Ollama error: {event['error']}")
                    message = event.get("message") or {}
                    yield ChatChunk(
                        delta=message.get("content") or "",
                        thinking=bool(message.get("thinking")),
                        done=bool(event.get("done")),
                        prompt_tokens=event.get("prompt_eval_count", 0),
                        completion_tokens=event.get("eval_count", 0),
                        generation_seconds=event.get("eval_duration", 0) / 1e9,
                    )
        except httpx.TransportError as exc:
            raise LLMUnavailable(_NOT_RUNNING) from exc

    def embed(self, model: str, texts: list[str]) -> list[list[float]]:
        self._require_local(model)
        try:
            response = self._client.post("/api/embed", json={"model": model, "input": texts, "keep_alive": _KEEP_ALIVE})
        except httpx.TransportError as exc:
            raise LLMUnavailable(_NOT_RUNNING) from exc
        return self._json(response)["embeddings"]

    def pull(self, model: str) -> Iterator[dict[str, Any]]:
        if _is_cloud_name(model):
            raise CloudModelRefused(f"{model} is a cloud model. Documents must stay on this computer.")
        try:
            with self._client.stream("POST", "/api/pull", json={"model": model, "stream": True}) as response:
                if response.is_error:
                    response.read()
                    self._json(response)
                for line in response.iter_lines():
                    if not line:
                        continue
                    event = json.loads(line)
                    if "error" in event:
                        raise LLMError(f"Download failed: {event['error']}")
                    yield event
        except httpx.TransportError as exc:
            raise LLMUnavailable(_NOT_RUNNING) from exc

    def delete(self, model: str) -> None:
        try:
            response = self._client.request("DELETE", "/api/delete", json={"model": model})
        except httpx.TransportError as exc:
            raise LLMUnavailable(_NOT_RUNNING) from exc
        if response.status_code == 404:
            raise ModelNotAvailable(f"The model {model} is not installed.")
        if response.is_error:
            raise LLMError(f"Ollama error: {_error_text(response)}")

    def running(self) -> list[dict[str, Any]]:
        return self._get("/api/ps").get("models", [])


def _error_text(response: httpx.Response) -> str:
    try:
        return response.json().get("error") or response.text
    except ValueError:
        return response.text
