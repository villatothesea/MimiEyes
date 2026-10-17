# vendor-sync.ps1 <ref>   e.g.  tools/vendor-sync.ps1 v0.2.114   or   tools/vendor-sync.ps1 main
# Moves vendor-zeron to the ref and subtree-merges it into the current branch.
param(
    [Parameter(Mandatory = $true)][string]$Tag
)

$ErrorActionPreference = 'Stop'
Set-Location (git rev-parse --show-toplevel)

$tag = if ($Tag -eq 'main') { 'zeron-upstream/main' } else { "zeron/$Tag" }

# 1. Fetch namespaced upstream tags + main (github first, local mirror as fallback).
$remotes = @('zeron-upstream', 'D:\MyWords\VibeCoding\Assets\zeron')
$fetched = $false
foreach ($remote in $remotes) {
    git fetch $remote "+refs/tags/*:refs/tags/zeron/*" "main" 2>$null
    if ($LASTEXITCODE -eq 0) { $fetched = $true; break }
}
if (-not $fetched) { throw "could not fetch upstream tags from any remote" }

git rev-parse --verify --quiet $tag | Out-Null
if ($LASTEXITCODE -ne 0) { throw "tag $tag not found after fetch" }

# 2. vendor-zeron is pristine upstream: reset it to the tag.
$branch = git symbolic-ref --short HEAD
git checkout vendor-zeron
git reset --hard $tag

# 3. Back to the working branch, subtree-merge it in.
git checkout $branch
git merge -X subtree=vendor/zeron vendor-zeron -m "vendor: zeron $Tag"

Write-Host ""
Write-Host "vendored $Tag. If there were conflicts, resolve them per vendor/zeron/VENDORED.md,"
Write-Host "then run the build + tests and update the patch list."
