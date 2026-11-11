"""The concepts found in a document when it is processed, without a model."""
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from tests import library_view
from tests.pdf_factory import knowledge_book


@pytest.fixture
def concepts(client: TestClient, tmp_path: Path) -> dict[str, dict]:
    path = tmp_path / "knowledge.pdf"
    knowledge_book(path)
    with path.open("rb") as handle:
        created = client.post("/api/books", files={"file": ("knowledge.pdf", handle)}).json()
    client.app.state.worker.join()
    return library_view.concepts(client, created["id"])


def test_glossary_entries_become_defined_concepts(concepts: dict[str, dict]) -> None:
    assert concepts["gravity"]["definition"].startswith("The attraction between masses")
    assert 3 in concepts["gravity"]["pages"]


def test_called_phrases_become_concepts(concepts: dict[str, dict]) -> None:
    assert "osmosis" in concepts
    assert "known as osmosis" in concepts["osmosis"]["definition"]


def test_section_titles_become_concepts_but_generic_ones_do_not(concepts: dict[str, dict]) -> None:
    assert "cell membranes" in concepts or "energy in cells" in concepts
    assert "glossary" not in concepts


def test_definitions_create_relations(concepts: dict[str, dict]) -> None:
    assert "gravity" in concepts["force"]["related"]
