# Provenance

| | |
|---|---|
| Upstream | [zeronsh/zeron](https://github.com/zeronsh/zeron) @ `3b2f7802` (past `v0.2.113`), MIT |
| Local patches | documented in `VENDORED.md` |
| Exported | 2026-10-19 |

This repo is generated from a private development repository where
upstream is vendored under `vendor/zeron` and synced by subtree merge.
Each release exports the current tree, strips non-code content, and
lands here as a tagged snapshot.

## Removed from the upstream tree

- `docs/` — screenshots & media (~36 MB)
- `apps/ios`, `apps/landing`, `apps/www-redirect` — companion apps/pages
- `edge/` — Cloudflare worker (except `src/install.sh`, a build-time
  `include_str!` input of `apps/zeron`)
- per-crate `tests/` and `benches/` dirs, `.github/` (upstream CI)
- upstream localized `README.*.md`

## Build-affecting edits vs upstream

- `crates/harness/Cargo.toml`: two `[[bin]]` blocks removed (they pointed
  into the deleted `tests/fixtures/` dir)
- `crates/text/Cargo.toml`: `[[bench]]` block removed (`benches/` deleted)

No workspace members were removed; `cargo build` works as-is.
