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
| 1 | `apps/zeron` (`Cargo.toml`, `build.rs`, `main.rs`, `paths.rs`), `crates/ui/src/lib.rs` (`app_id`, window title), `crates/engine/src/instance_lock.rs` messages, `tools/build.ps1` | package/bin/app-id renamed to `mimieyes`; **own data dir** `%LOCALAPPDATA%\MimiEyes` / `~/.mimieyes` (`MIMIEYES_DATA_DIR`, not `ZERON_DATA_DIR`) and **own IPC port** 29654 (`MIMIEYES_IPC_PORT`) so it never joins a running Zeron. The `zeron://` URL scheme and `ZERON_*` env contract with harness CLIs stay upstream's; daemon/launchd+systemd identities stay `zeron` until MimiEyes ships an installer. Windows exe icon: `dist/windows/mimieyes.{rc,ico}` (new; `build.rs` compiles it instead of `zeron.rc`), drawn from `assets/brand/mimieyes-icon.svg`. Boot splash: `crates/ui/src/hz_splash.rs` (new; the logo's spider glances, blinks and hops) in place of the dot spinner in `loaders.rs::splash_overlay`, whose line reads "Setting up MimiEyes environment" | must not merge with the real Zeron on the taskbar, in install dirs, or on the data path |
| 2 | `crates/ui/src/shell/hz_tree.rs` (new) + `// hz:` hooks in `shell.rs` (sidebar branch), `shell/spaces.rs` ("Tree sidebar" Show toggle) + `UiSettings.sidebar_tree`/`hz_tree_collapsed` | Tree sidebar — only a view of how things are organized: Zeron-style uppercase labels (PINNED, LOCAL / machine names) → projects (fold, mark, Zeron's project "+", count) → with panes on, **tabs** (place + name, status, "+"; click shows it, double-click renames it there, right-click its menu, drops move into it) → **panes** in layout order (mark, CLI logo, title, short id, age; click focuses, double-click renames the session, right-click the pane menu, drags like a tag; new-session panes listed) and a folded "Other sessions"; with panes off, projects → sessions. Rows and tree lines in HerdrGui's measure | product feature #1 |
| 3 | `crates/ui/src/shell/hz_layout.rs`, `hz_tabs.rs` (new) + `// hz:` hooks in `shell.rs` (view branch, menus, field, `hz_sync` at the top of `render`, `apply_nav` back-to-canvas), `shell/tabs.rs` (`open_chat`, `open_new_session`, titlebar strip + its 2px seam inset), `shell/spaces.rs` (`land_in_space`), `shell/side_chats.rs` (`make_side_chat_tab` / `wrap_side_chat_tab`, draft-aware observer + worktree setup), `shell/hz_agent.rs` (new: the window's side of the agents' `panes` tool), `hz_agent_msg.rs` (new) + `transcript.rs::render_user_body` (an agent-sent message shows "From <title> · <short id>" instead of the raw `[Message from Zeron chat …]` line), **`crates/engine/src/hz_relay.rs`** (new; `HzRequest` / `WatchHzRequests` / `HzReply` relay, hooked in `engine/src/lib.rs` + `rpc.rs::EngineRpc`), **`crates/mcp/src/hz_panes.rs`** (new; the `panes` tool — `list` / `arrange` — hooked in `mcp/src/lib.rs` + `tools.rs` catalog/filter/dispatch), `state.rs` (`hz_draft_state`), `composer.rs` (`hz_plan_usage`: a pane's composer keeps the account/plan-usage ring), `shell/session_info.rs` (`session_info_reserve`: over panes the session-details card floats for the focused pane's session), `transcript.rs` (`hz_top_gap`), `icons.rs` (`ZOOM_ARROWS`, `EVEN_PANES`, `panel_bottom_glyph`) + `assets/icons/{zoom-arrows,even-panes}.svg` + `UiSettings.hz_panes`/`hz_layouts` | Project → view tabs (named groups) → panes (chat slots in a split tree) → chat. The project on screen is the selected session's; each project keeps its own tabs (a session never sits in another project's pane). "Panes" is one global switch. A new pane is a new session in the tab's project (Zeron's project "+": a draft composer, minted on first send). HerdrGui's pane menu (Split right/down, Zoom, Even out panes — every row and column —, Rename, Archive, Close pane; close only hides, the sidebar brings it back) and pane drag (tag or tree row onto a pane swaps or moves in, onto a tab chip moves into that tab). View-tab chips `occlude` so Windows' caption hit test never moves the window under them; selection ⇄ focused pane kept in step once per frame; persisted per project, no engine/RPC | product feature #2 |
| 4 | `proto`, `harness/pi`, `engine`, `ui` (`// hz:` regions) + `assets/icons/pirust-mark.svg` | `HarnessId::Pirust`: the PiRustDurable port as a first-class harness on the shared pi-rpc driver (`pirust` exe, `PIRUST_EXECUTABLE`, `~/.pirust/agent`, stepped-R mark) | run our Rust port side by side with upstream Pi |
| 5 | `crates/ui/src/shell.rs` (`render_titlebar_cluster`, `CLUSTER_BUTTONS_WIDTH` + its test, `nav_history_button` allowed dead) | No Back/Forward buttons in the titlebar; the shortcuts still navigate | user request: the arrows are noise |
| 6 | `crates/engine/src/hz_usage.rs` (new; hooked in `engine/src/lib.rs` (module + `init` at `assemble_with_profile_locked`), `sessions.rs::drive_run` (books each `AgentEvent::Usage`), `rpc.rs` (`HzUsage`)), `crates/harness/src/claude/{wire,normalize}.rs` (Claude's turn input counts its prompt-cache reads and writes), `crates/ui/src/settings/hz_usage.rs` (new) + `settings.rs` module + `shell.rs` (`SettingsSection::Usage`: nav, slug, title, icon, page) + `shell/hz_tabs.rs::hz_usage_tabs` | Settings → Usage: every turn's tokens booked to `hz-usage.jsonl` in the engine data dir (stock drops them), grouped on the page by any ordered mix of project, tab, session, CLI, model and day; range Today / 7 / 30 days / All; total, input or output | user request: see where tokens go |
| 7 | `apps/zeron/src/hz_monitor.rs` (new) + `main.rs` (hidden `monitor --pid` command; the headed start spawns it detached, out of the app's job), `apps/zeron/Cargo.toml` (windows-sys Threading / SystemInformation / WindowsProgramming), `apps/zeron/tests/noop_browser.rs` (binary is `mimieyes`), `crates/ui/src/hz_activity.rs` (new) + `lib.rs` (module; quit reason left at `on_app_quit` and window close), `crates/ui/src/settings/hz_runs.rs` (new) + `settings.rs` module + `shell.rs` (`SettingsSection::Activity`: nav, slug, title, icon, page) | Activity monitor: a watcher process books every run to `logs/hz-activity.jsonl` — start, end, exit code, the app's own quit reason, how long the PC slept, and whether it ended just after a wake; Settings → Activity lists them (closed / quit, killed from outside, ended by Windows, crashed) | user request: find out why the app exited |
| 8 | `crates/ui/src/lib.rs` (`HZ_ZERON_UPDATES = false` around `AppUpdate::init`; `app_update.rs` untouched), `crates/harness/src/acp/devin_models.rs` (`HZ_DEVIN_MODEL_WAIT` 30 s in `wait_for_model`, error names the cause), `crates/ui/src/shell/hz_tabs.rs` (`loose_menu`, `hz_fallback_project`, `hz_remove_loose`, `render_hz_loose_menu`; `hz_sync` shows a real project instead of regrowing an emptied No project) + `shell/hz_tree.rs` (No project right-click) + `shell.rs` (overlay) | Stock Zeron's update check and sidebar strip off (it would replace MimiEyes with stock Zeron); Devin waits 30 s, not 10 s, for its model list (slow networks failed every turn with "did not advertise requested model"); the No project row has a menu (New session, Remove from sidebar / Close its tabs) | user request |
| 9 | `crates/engine/src/model_catalogs.rs` (`hz_latest_claude`: Claude's newest saved catalog when the exact credential-keyed one is missing, + test), `crates/ui/src/pickers.rs::measure_trigger` (open down only where the card fits or below is roomier, else up), `crates/ui/src/shell.rs` (`ChatRenameSurface::HzPane`; `open_rename_chat` honours `hz.rename_on_pane`) + `shell/hz_tabs.rs` (`hz_rename_pane`, tag double-click, editor on the tag) + `shell/chat_rename_tests.rs` | Claude's model list shows at once instead of after a ~20 s cold probe (its cache key changes with every login-token refresh); the model picker opens upward where a pane leaves little room below; renaming a session happens where it was asked for: the pane's tag (double-click or pane menu) or the sidebar row | user request |

Patches 2-3 ported the *drawing* primitives of HerdrGui's `hz_tabs.rs`
(strip, tiles, name tags, glow dividers, folds) onto a local model in
`hz_layout.rs`: a tab is a named group of pane slots, a slot a chat id —
no `hzLayout` verb, no `Mutate` calls, nothing engine-side.

## Patch discipline

- New code goes in `hz_*.rs` files next to the upstream file — new files
  never conflict.
- Edits inside upstream files carry a `// hz:` comment marker so
  `rg "// hz:" vendor/` audits every touch point after a merge.
- Keep each upstream-file edit as small as possible; one concern per
  patch row above.
- After every sync: build, run the workspace tests, and re-verify each
  patch row still describes reality (update or drop it).
