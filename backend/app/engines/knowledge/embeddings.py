import re
import zlib
from typing import Protocol

import numpy as np

from app.engines.llm.base import LLMProvider

_WORD = re.compile(r"[^\W\d_]+|\d+")
STOPWORDS = frozenset(
    """a an and are as at be been but by can could did do does for from had has have how i if in into is it its
    me my of on or our should so than that the their them then there these they this to us was we were what when
    where which who why will with would you your about explain tell please mean means meant""".split()
)


def stem(word: str) -> str:
    """A deliberately small stemmer: enough to unify plurals and common verb endings."""
    for suffix in ("ing", "ies", "ed", "es", "s"):
        if word.endswith(suffix) and len(word) - len(suffix) >= 3:
            return word[: -len(suffix)] + ("y" if suffix == "ies" else "")
    return word


def content_words(text: str) -> list[str]:
    return [word for word in (m.lower() for m in _WORD.findall(text)) if word not in STOPWORDS]


class Embedder(Protocol):
    key: str

    def embed(self, texts: list[str]) -> np.ndarray: ...


class HashingEmbedder:
    """Dependency-free vectors from hashed word and character n-grams.

    This captures spelling and word-form similarity, not meaning. It is the fallback
    used until an embedding model is installed.
    """

    key = "hashing-v1"
    dimensions = 4096

    def _features(self, text: str) -> list[tuple[str, float]]:
        stems = [stem(word) for word in content_words(text)]
        features = [(word, 1.0) for word in stems]
        features += [(f"{a}_{b}", 1.5) for a, b in zip(stems, stems[1:])]
        for word in stems:
            padded = f"<{word}>"
            features += [(padded[i:i + 4], 0.2) for i in range(len(padded) - 3)]
        return features

    def embed(self, texts: list[str]) -> np.ndarray:
        matrix = np.zeros((len(texts), self.dimensions), dtype=np.float32)
        for row, text in enumerate(texts):
            for feature, weight in self._features(text):
                digest = zlib.crc32(feature.encode("utf-8"))
                sign = 1.0 if digest & 1 else -1.0
                matrix[row, (digest >> 1) % self.dimensions] += sign * weight
        # Dampen repeated terms so long passages are not dominated by a few words.
        matrix = np.sign(matrix) * np.sqrt(np.abs(matrix))
        norms = np.linalg.norm(matrix, axis=1, keepdims=True)
        norms[norms == 0] = 1.0
        return matrix / norms


class ModelEmbedder:
    """Semantic vectors from a local embedding model."""

    _BATCH = 32

    def __init__(self, provider: LLMProvider, model: str) -> None:
        self.provider = provider
        self.model = model
        self.key = "model-" + re.sub(r"[^a-zA-Z0-9.-]", "_", model)

    def embed(self, texts: list[str]) -> np.ndarray:
        vectors: list[list[float]] = []
        for start in range(0, len(texts), self._BATCH):
            vectors.extend(self.provider.embed(self.model, texts[start:start + self._BATCH]))
        matrix = np.array(vectors, dtype=np.float32)
        norms = np.linalg.norm(matrix, axis=1, keepdims=True)
        norms[norms == 0] = 1.0
        return matrix / norms
