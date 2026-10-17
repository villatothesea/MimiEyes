# Vendored Zeron crates

Source: `zeronsh/zeron` (https://github.com/zeronsh/zeron.git), MIT.
Pinned: upstream `main` @ `3b2f7802` (past `v0.2.113`). Release tags are
fetched as `zeron/v*`; track `main` by syncing `zeron-upstream/main`.

## How the vendor link works

Upstream history is fetched directly into this repository under
namespaced tags (`zeron/v*`). The branch `vendor-zeron` always points at
the upstream release currently vendored — its tree **is** the upstream
tree (no wrapper), so a subtree merge places it under `vendor/zeron`.

Update flow (see `tools/vendor-sync.ps1`):

```powershell
tools/vendor-sync.ps1 v0.2.114
```

1. `git fetch zeron-upstream "+refs/tags/*:refs/tags/zeron/*"`
   (falls back to the local mirror when offline)
2. `vendor-zeron` is reset to the new tag — the branch is *pristine
   upstream*, never carries local edits
3. `git merge -X subtree=vendor/zeron vendor-zeron` on `main`

Upstream changes merge cleanly into `vendor/zeron`; conflicts only
appear where a local patch touched the same upstream lines — resolve
them using the patch list below as the spec of intent.

## Local patches (keep this list short and mechanical)

| # | File | Change | Why |
|---|------|--------|-----|
| — | — | none yet | — |

## Patch discipline

- New code goes in `hz_*.rs` files next to the upstream file — new files
  never conflict.
- Edits inside upstream files carry a `// hz:` comment marker so
  `rg "// hz:" vendor/` audits every touch point after a merge.
- Keep each upstream-file edit as small as possible; one concern per
  patch row above.
- After every sync: build, run the workspace tests, and re-verify each
  patch row still describes reality (update or drop it).
