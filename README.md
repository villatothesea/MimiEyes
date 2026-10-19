# MimiEyes

A desktop agent console built on [Zeron](https://github.com/zeronsh/zeron) —
the same app, plus:

- **Tree sidebar** — machines → spaces → chats as a foldable tree
- **Split panes & tabs** — a space's chats sit side by side instead of one at a time
- **`panes` MCP tool** — agents can see and arrange the panes
- **Settings → Usage** — per-turn tokens grouped by project, tab, session, CLI, model
- **Settings → Activity** — when and how each run ended (quit / killed / crashed)
- **Pirust harness** — PiRustDurable port on the shared pi-rpc driver
- Spider opening animation, MimiEyes identity (own data dir & IPC port)

## Status

This repository is a **release snapshot**, not the development repo.
Source = upstream `zeronsh/zeron` @ `3b2f7802` (past `v0.2.113`, MIT)
plus the MimiEyes patch series, minus upstream's docs, tests and
companion apps. See `SOURCE.md` for provenance and `VENDORED.md` for
the full patch list.

## Build

```powershell
cargo build --release -p mimieyes     # or: tools/build.ps1
```

## License

MIT — see `LICENSE` (MimiEyes additions) and `LICENSE-zeron` (upstream,
Copyright (c) 2026 Wing). Details in `THIRD-PARTY.md`.
