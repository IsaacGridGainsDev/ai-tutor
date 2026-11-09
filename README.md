# Interactive Tutor

An offline-first Windows desktop tutor for PDFs. See `IMPLEMENTATION_STATUS.md` for what works today,
`ARCHITECTURE.md` for the design and `DEVELOPMENT_LOG.md` for the build history.

## Prerequisites

- Windows 11 with WebView2
- Node 24 and npm
- Rust (MSVC toolchain) and Visual Studio Build Tools with the C++ workload
- [uv](https://docs.astral.sh/uv/) for Python 3.12
- Tesseract OCR, for scanned PDFs (`winget install UB-Mannheim.TesseractOCR`). Set
  `TUTOR_TESSERACT_PATH` if it is not in the default location. Without it, scanned pages have no text
  and the app says so.
- Ollama, for later phases

## Setup

```powershell
cd backend
uv venv --python 3.12 .venv
uv pip install --python .venv\Scripts\python.exe -e ".[dev]"

cd ..\frontend
npm install
```

## Run

```powershell
cd frontend
npx tauri dev
```

The shell starts the Python backend itself. Application data is stored in
`%LOCALAPPDATA%\InteractiveTutor`.

## Test

```powershell
cd backend;  .venv\Scripts\python.exe -m pytest
cd frontend; npm test
```

## Licensing note

PyMuPDF is licensed under AGPL-3.0 (or a commercial licence from Artifex). Distributing this
application publicly requires complying with the AGPL or buying that licence.
