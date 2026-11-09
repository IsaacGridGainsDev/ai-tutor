from collections.abc import Iterator
from dataclasses import dataclass, field
from typing import Any, Protocol


class LLMError(Exception):
    """A model failure with a message suitable for showing to the learner."""


class LLMUnavailable(LLMError):
    pass


class ModelNotAvailable(LLMError):
    pass


class CloudModelRefused(LLMError):
    pass


@dataclass(slots=True)
class ModelInfo:
    name: str
    size_bytes: int
    family: str | None
    parameter_size: str | None
    quantization: str | None
    capabilities: list[str] = field(default_factory=list)
    is_cloud: bool = False

    @property
    def can_chat(self) -> bool:
        return "completion" in self.capabilities

    @property
    def can_embed(self) -> bool:
        return "embedding" in self.capabilities


@dataclass(slots=True)
class ChatChunk:
    delta: str = ""
    thinking: bool = False
    done: bool = False
    prompt_tokens: int = 0
    completion_tokens: int = 0
    generation_seconds: float = 0.0


class LLMProvider(Protocol):
    """Everything the engines need from a model runtime, so the runtime stays interchangeable."""

    def version(self) -> str: ...

    def list_models(self) -> list[ModelInfo]: ...

    def chat_stream(
        self, model: str, messages: list[dict[str, str]], schema: dict[str, Any] | None = None,
        options: dict[str, Any] | None = None,
    ) -> Iterator[ChatChunk]: ...

    def embed(self, model: str, texts: list[str]) -> list[list[float]]: ...

    def pull(self, model: str) -> Iterator[dict[str, Any]]: ...

    def delete(self, model: str) -> None: ...

    def running(self) -> list[dict[str, Any]]: ...
