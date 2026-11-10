# Implementation Status

The application is **not complete**. Phases 1 to 3 of 7 are done, and a Windows installer of
this version exists.

| Phase | State |
| --- | --- |
| 1. Foundation | Done, verified in the real desktop window |
| 2. Document processing | Done, verified in the real desktop window and the installed app |
| 3. Local AI | Done, verified with the real local model in the desktop window; two-model benchmark deferred |
| 4. Teaching | Not started |
| 5. Voice | Not started |
| 6. Learning memory | Not started (tables exist) |
| 7. Packaging | Installer built early (see below); offline hardening, performance and settings remain |

## Installer

- `release/Interactive Tutor_0.1.0_x64-setup.exe`, 84 MB, installs per user without admin rights.
- It includes the app, the frozen Python backend and Tesseract OCR with English data.
- It needs Ollama installed separately with a local model. Models are not bundled.
- Rebuild it with `packaging\build-installer.ps1`.

## Works today

- Desktop app that starts and stops its own backend, with an authenticated loopback connection.
- My Library: upload, processing status, title search, collections, metadata, removal and resumed
  processing. Reading by chapter and section with page references, and search inside a book.
- Scanned PDFs through Tesseract OCR.
- Ask: grounded answers from the local model. Each claim is labelled as from the book (with a
  verified quote and page), general knowledge, interpretation or not found. A question with no
  match in the book is not sent to the model. Stop cancels.
- Concepts: knowledge map built automatically at import, plus optional per-chapter AI enrichment
  that keeps only concepts with verified evidence.
- Settings: model selection with cloud models excluded, embedding model selection, model download
  and removal, compute preference, reading speed, explanation depth, assessment difficulty and
  storage folder.

## Not built yet

Learning Room, Progress Centre and Revision Centre show which phase builds them and have no controls.

## Known limitations

- **Hardware.** The RTX 5080 is reported as not present (`CM_PROB_PHANTOM`) while on battery, so
  `qwen3.8` runs on CPU and uses about 18 GB of system RAM. Answers take roughly 15 to 140 seconds.
  One automated run was killed by Claude Code because the system ran low on memory.
- **Stop.** Stop frees the interface immediately, but the model keeps working until its first
  token, up to about 2 minutes on CPU. A following question waits behind it.
- **Search.** Without an embedding model, search matches words, not meaning.
- **Benchmark.** Only one model has been benchmarked, by the owner's instruction not to download
  models yet. `backend/tools/evaluate.py --models a b` runs the comparison once more are installed.
- **Installed-app Ask.** In the installed app the backend received Ask requests and Ollama
  answered them, but a full uninterrupted Ask run was not completed: interference, time limits and
  memory pressure. The same code passed end to end in the desktop debug build with the real model.
- **Unsigned.** The installer is not code-signed, so Windows SmartScreen will warn on first run.
- **Licences.** PyMuPDF is AGPL-3.0, so distributing the installer publicly requires AGPL
  compliance or a commercial licence. Tesseract is Apache-2.0.
- **Document handling.** Code listings lose their line layout; tables and figures are not
  interpreted; OCR is English only; two-column scans are untested.
