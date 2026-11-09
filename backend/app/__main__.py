import os
import sys
import threading

import uvicorn

from app.config import Settings
from app.main import create_app


def exit_when_parent_closes_stdin() -> None:
    """Stop with the desktop shell, even if the shell crashed and never killed us."""

    def watch() -> None:
        sys.stdin.read()
        os._exit(0)

    threading.Thread(target=watch, daemon=True).start()


def main() -> None:
    if os.environ.get("TUTOR_WATCH_STDIN") == "1":
        exit_when_parent_closes_stdin()
    settings = Settings()
    uvicorn.run(create_app(settings), host=settings.host, port=settings.port, log_level="info")


if __name__ == "__main__":
    main()
