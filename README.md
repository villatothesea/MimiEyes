# MimiEyes

Zeron, kept exactly as it is — plus two UI capabilities:

1. **Tree sidebar** — an alternative left bar that shows machines →
   spaces → chats as a foldable tree (multi-level, collapsible).
2. **Split panes** — a space's chats can sit side by side in split
   panes instead of one-at-a-time.

Everything else is upstream `zeronsh/zeron`, tracked live through the
vendor branch (see `vendor/zeron/VENDORED.md`). Local changes are a
documented patch series, not a fork that drifts.

## Build

```powershell
tools/build.ps1            # release binary at vendor/zeron/target/release/
```

## Sync upstream

```powershell
tools/vendor-sync.ps1 main      # or a release tag, e.g. v0.2.114
```

Upstream merges into `vendor/zeron` as a real three-way merge; our two
features live in `hz_*`/marked regions so conflicts stay rare and small.
