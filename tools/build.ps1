# build.ps1 — build the MimiEyes binary (patched zeron) from the vendored tree.
param(
    [switch]$Release = $true
)

$ErrorActionPreference = 'Stop'
Set-Location (git rev-parse --show-toplevel)

$profile = if ($Release) { '--release' } else { '' }
cargo build --manifest-path vendor/zeron/Cargo.toml -p zeron $profile

$out = if ($Release) { 'vendor\zeron\target\release\zeron.exe' } else { 'vendor\zeron\target\debug\zeron.exe' }
Write-Host "built: $out"
