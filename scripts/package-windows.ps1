# Gera o instalador do Windows (Inno Setup) com a versão do Cargo.toml.
# Uso: .\scripts\package-windows.ps1 [-NoBuild] [-Install]
# -Install roda o instalador em modo silencioso nesta máquina (usado no desenvolvimento).
param(
    [switch]$NoBuild,
    [switch]$Install
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.+)"$' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "versão não encontrada no Cargo.toml" }

if (-not $NoBuild) {
    Push-Location $root
    try {
        cargo build --release -p lan-kvm
        if ($LASTEXITCODE -ne 0) { throw "cargo build" }
    } finally { Pop-Location }
}
$exe = Join-Path $root "target\release\lan-kvm.exe"
if (-not (Test-Path $exe)) { throw "binário não encontrado: $exe" }

$iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $iscc) { throw "Inno Setup 6 não encontrado (winget install JRSoftware.InnoSetup)" }

$out = Join-Path $root "target\installer"
& $iscc /Q "/DAppVersion=$version" "/DSourceExe=$exe" "/DOutputDir=$out" (Join-Path $root "packaging\windows\lan-kvm.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC falhou" }
$setup = Join-Path $out "lan-kvm-$version-windows-setup.exe"
Write-Host "instalador: $setup"

if ($Install) {
    $p = Start-Process $setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -PassThru
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) { throw "instalador saiu com código $($p.ExitCode)" }
    Write-Host "lan-kvm $version instalado"
}
