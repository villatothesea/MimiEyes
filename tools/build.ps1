# build.ps1 — build the MimiEyes binary.
param(
    [switch]$Release = $true
)

$ErrorActionPreference = 'Stop'
Set-Location (git rev-parse --show-toplevel)

$profile = if ($Release) { '--release' } else { '' }
cargo build -p mimieyes $profile

$out = if ($Release) { 'target\release\mimieyes.exe' } else { 'target\debug\mimieyes.exe' }
Write-Host "built: $out"
