# Builds the Windows installer. Output: frontend\src-tauri\target\release\bundle\nsis\
# Pass -E2E to build a test-only version that exposes the WebView debug port.
param([switch]$E2E)

$root = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

& powershell -NoProfile -File (Join-Path $PSScriptRoot 'build-backend.ps1')
if ($LASTEXITCODE -ne 0) { throw "Backend build failed" }

Set-Location (Join-Path $root 'frontend')
$arguments = @('tauri', 'build')
if ($E2E) { $arguments += @('--features', 'e2e') }
& npx @arguments
if ($LASTEXITCODE -ne 0) { throw "Tauri build failed with exit code $LASTEXITCODE" }
