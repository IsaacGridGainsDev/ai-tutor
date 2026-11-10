import os
import shutil
import statistics
import subprocess
from pathlib import Path
from typing import Protocol

from app.engines.documents.models import Line

OCR_DPI = 200
_POINTS_PER_PIXEL = 72 / OCR_DPI
_DEFAULT_TESSERACT = Path(r"C:\Program Files\Tesseract-OCR\tesseract.exe")
_MIN_WORD_CONFIDENCE = 30
_TIMEOUT_SECONDS = 180
_CREATE_NO_WINDOW = 0x08000000


class OcrError(RuntimeError):
    pass


class OcrEngine(Protocol):
    def recognize(self, png: bytes, page_number: int) -> list[Line]: ...


def parse_tsv(tsv: str, page_number: int) -> list[Line]:
    """Convert Tesseract TSV output into lines, using its block and paragraph grouping."""
    grouped: dict[tuple[int, int, int], list[list[str]]] = {}
    for row in tsv.splitlines()[1:]:
        cols = row.split("\t")
        if len(cols) == 12 and cols[0] == "5" and cols[11].strip() and float(cols[10]) >= _MIN_WORD_CONFIDENCE:
            grouped.setdefault((int(cols[2]), int(cols[3]), int(cols[4])), []).append(cols)

    lines = []
    for (block, paragraph, _), words in grouped.items():
        left = min(int(w[6]) for w in words)
        top = min(int(w[7]) for w in words)
        right = max(int(w[6]) + int(w[8]) for w in words)
        bottom = max(int(w[7]) + int(w[9]) for w in words)
        height = statistics.median(int(w[9]) for w in words)
        lines.append(
            Line(
                page=page_number,
                text=" ".join(w[11].strip() for w in words),
                size=round(height * _POINTS_PER_PIXEL, 1),
                bold=False,
                block=block * 1000 + paragraph,
                x0=left * _POINTS_PER_PIXEL,
                y0=top * _POINTS_PER_PIXEL,
                x1=right * _POINTS_PER_PIXEL,
                y1=bottom * _POINTS_PER_PIXEL,
            )
        )
    return lines


class TesseractOcr:
    def __init__(self, executable: Path, language: str = "eng") -> None:
        # Tesseract finds its language data from its own path, which fails under the \\?\ form
        # that Tauri uses for an installed app's resource folder.
        self.executable = Path(str(executable).removeprefix("\\\\?\\"))
        self.language = language

    @classmethod
    def discover(cls, configured: Path | None = None) -> "TesseractOcr | None":
        found = shutil.which("tesseract")
        candidates = [configured, Path(found) if found else None, _DEFAULT_TESSERACT]
        for candidate in candidates:
            if candidate and candidate.exists():
                return cls(candidate)
        return None

    def recognize(self, png: bytes, page_number: int) -> list[Line]:
        try:
            result = subprocess.run(
                [str(self.executable), "stdin", "stdout", "-l", self.language, "--dpi", str(OCR_DPI), "tsv"],
                input=png,
                capture_output=True,
                timeout=_TIMEOUT_SECONDS,
                # Pages run in parallel, so each process stays single-threaded.
                env={**os.environ, "OMP_THREAD_LIMIT": "1"},
                creationflags=_CREATE_NO_WINDOW,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise OcrError(f"OCR failed on page {page_number}: {exc}") from exc
        if result.returncode != 0:
            detail = result.stderr.decode("utf-8", "replace").strip()[:300]
            raise OcrError(f"OCR failed on page {page_number}: {detail}")
        output = result.stdout.decode("utf-8", "replace")
        # A missing tessdata config still exits 0, so a blank page must be told apart from a broken install.
        if not output.startswith("level\t"):
            raise OcrError(f"OCR failed on page {page_number}: Tesseract did not produce TSV output.")
        return parse_tsv(output, page_number)
