# Compila e testa no Windows.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo fmt --all -- --check; if ($LASTEXITCODE -ne 0) { throw "cargo fmt" }
    cargo clippy --workspace --all-targets -- -D warnings; if ($LASTEXITCODE -ne 0) { throw "cargo clippy" }
    cargo test --workspace; if ($LASTEXITCODE -ne 0) { throw "cargo test" }
    cargo build --workspace --release; if ($LASTEXITCODE -ne 0) { throw "cargo build" }
    Write-Host "Binário: $root\target\release\lan-kvm.exe"
} finally { Pop-Location }
