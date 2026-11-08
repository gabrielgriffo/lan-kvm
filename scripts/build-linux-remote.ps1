# Copia o código para o notebook e compila lá dentro do container Ubuntu 22.04.
# Uso: .\scripts\build-linux-remote.ps1 [-Target bsn@192.168.0.18] [-Run] [-Install]
# -Install instala o binário em ~/.local/bin do notebook e reinicia o receptor na sessão gráfica.
param(
    [string]$Target = "bsn@192.168.0.18",
    [string]$RemoteDir = "lan-kvm",
    [switch]$Run,
    [switch]$Install
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

ssh $Target "mkdir -p ~/$RemoteDir"
if ($LASTEXITCODE -ne 0) { throw "ssh falhou" }

tar -C $root --exclude=./target --exclude=./.git -cf - . | ssh $Target "tar -x -C ~/$RemoteDir"
if ($LASTEXITCODE -ne 0) { throw "cópia falhou" }

ssh $Target "cd ~/$RemoteDir && bash scripts/build-linux-docker.sh"
if ($LASTEXITCODE -ne 0) { throw "build remoto falhou" }

if ($Install) {
    ssh -n $Target "cd ~/$RemoteDir && bash scripts/install-linux.sh"
    if ($LASTEXITCODE -ne 0) { throw "instalação remota falhou" }
}

if ($Run) {
    ssh $Target "~/lan-kvm-build/bin/lan-kvm"
}
