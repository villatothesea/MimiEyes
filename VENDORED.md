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
| 1 | `apps/zeron/Cargo.toml`, window app id | binary + AppUserModelID renamed to `mimieyes` | must not merge with the real Zeron on the taskbar / in install dirs |
| 2 | `crates/ui` sidebar (new `hz_tree.rs` + small hooks) | Tree sidebar view: machines → spaces → chats, foldable | product feature #1 |
| 3 | `crates/ui` shell/transcript area (new `hz_split.rs` + small hooks) | Split-pane mode: a space's chats side by side | product feature #2 |

Patches 2-3 are planned, not yet applied — port the equivalents
(`vendor/zeron/ui/src/shell/hz_tabs.rs` and friends) from HerdrGui,
adapted to Zeron's own space/chat model instead of Herdr's
workspace/tab/pane model.

## Patch discipline

- New code goes in `hz_*.rs` files next to the upstream file — new files
  never conflict.
- Edits inside upstream files carry a `// hz:` comment marker so
  `rg "// hz:" vendor/` audits every touch point after a merge.
- Keep each upstream-file edit as small as possible; one concern per
  patch row above.
- After every sync: build, run the workspace tests, and re-verify each
  patch row still describes reality (update or drop it).
