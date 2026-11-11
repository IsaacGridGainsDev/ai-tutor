# Freezes the Python backend and stages it, with Tesseract, for the Tauri bundler.
# Run from anywhere: powershell -File packaging\build-backend.ps1
$root = Split-Path -Parent $PSScriptRoot
$backend = Join-Path $root 'backend'
$staging = Join-Path $root 'frontend\src-tauri\binaries'
$tesseract = 'C:\Program Files\Tesseract-OCR'

if (-not (Test-Path "$tesseract\tesseract.exe")) { throw "Tesseract not found at $tesseract" }

Set-Location $backend
& .venv\Scripts\python.exe -m PyInstaller --noconfirm --clean --onedir --name tutor-backend `
    --distpath build\dist --workpath build\work --specpath build `
    --paths $backend `
    --add-data "$backend\app\db\migrations;app\db\migrations" `
    --collect-submodules uvicorn `
    --exclude-module onnxruntime --exclude-module pymupdf_layout --exclude-module pymupdf4llm `
    --exclude-module faiss --exclude-module numpy --exclude-module httpx `
    --exclude-module tkinter --exclude-module pytest --exclude-module PyInstaller `
    run_backend.py
if ($LASTEXITCODE -ne 0) { throw "PyInstaller failed with exit code $LASTEXITCODE" }

if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Force "$staging\tesseract\tessdata" | Out-Null
Copy-Item -Recurse build\dist\tutor-backend "$staging\tutor-backend"
Copy-Item "$tesseract\tesseract.exe", "$tesseract\*.dll" "$staging\tesseract"
Copy-Item "$tesseract\tessdata\eng.traineddata" "$staging\tesseract\tessdata"
# The 'tsv' output mode is defined by a config file, without which Tesseract emits nothing useful.
Copy-Item -Recurse "$tesseract\tessdata\configs" "$staging\tesseract\tessdata\configs"
Get-ChildItem "$tesseract\*" -Include 'LICENSE*', 'COPYING*', 'AUTHORS*' -ErrorAction SilentlyContinue | Copy-Item -Destination "$staging\tesseract"

foreach ($dir in 'tutor-backend', 'tesseract') {
    $mb = (Get-ChildItem -Recurse -File "$staging\$dir" | Measure-Object Length -Sum).Sum / 1MB
    "{0}: {1:N0} MB" -f $dir, $mb
}
