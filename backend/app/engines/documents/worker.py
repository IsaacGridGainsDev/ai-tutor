import logging
import queue
import threading
from pathlib import Path

from app.db.connection import connect
from app.engines.documents.ocr import TesseractOcr
from app.engines.documents.pipeline import process_book

logger = logging.getLogger(__name__)
_STOP = object()


class IngestWorker:
    """Processes books one at a time so OCR cannot starve the rest of the application."""

    def __init__(self, db_path: Path, tesseract_path: Path | None = None) -> None:
        self.db_path = db_path
        self.tesseract_path = tesseract_path
        self._queue: queue.Queue = queue.Queue()
        self._thread = threading.Thread(target=self._run, name="ingest-worker", daemon=True)

    def start(self) -> None:
        self._thread.start()
        conn = connect(self.db_path)
        try:
            # Anything still mid-pipeline was interrupted by the previous shutdown.
            rows = conn.execute("SELECT id FROM books WHERE status NOT IN ('ready', 'failed')").fetchall()
        finally:
            conn.close()
        for row in rows:
            self.submit(row["id"])

    def submit(self, book_id: int) -> None:
        self._queue.put(book_id)

    def join(self) -> None:
        self._queue.join()

    def stop(self) -> None:
        self._queue.put(_STOP)
        self._thread.join(timeout=10)

    def _run(self) -> None:
        while True:
            item = self._queue.get()
            try:
                if item is _STOP:
                    return
                conn = connect(self.db_path)
                try:
                    process_book(conn, item, TesseractOcr.discover(self.tesseract_path))
                finally:
                    conn.close()
            except Exception:
                logger.exception("Failed to process book %s", item)
            finally:
                self._queue.task_done()
