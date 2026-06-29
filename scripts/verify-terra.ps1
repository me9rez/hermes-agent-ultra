$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "== hermes-tasks tests =="
cargo test -p hermes-tasks
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== hermes-http tests =="
cargo test -p hermes-http --lib
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== Tauri cargo check =="
cargo check -p hermes-desktop-community
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== frontend build:check =="
Push-Location apps/hermes-desktop-tauri
npm run build:check
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { exit $code }

Write-Host "Terra verify: OK"
