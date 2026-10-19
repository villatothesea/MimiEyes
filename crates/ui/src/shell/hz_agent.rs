//! MimiEyes (VENDORED.md #3): the window's side of the agents' `panes` tool.
//!
//! The engine relays an agent's request (`WatchHzRequests`); the window —
//! which owns every project's tabs and panes — serves it and answers
//! (`HzReply`):
//!
//! * `list`: the calling chat's project — its tabs and their panes;
//! * `arrange`: lay out one tab of that project (or create one) as a split
//!   tree of its sessions, optionally renaming it and bringing it on screen.
//!
//! Only sessions of the caller's project can be placed: panes belong to a
//! tab, tabs to a project.

use super::hz_layout::{HzDir, HzNode, HzSpaceLayout, HzViewTab};
use super::*;
use serde_json::{Value, json};

const WATCH_HZ_REQUESTS: &str = "WatchHzRequests";
/// `zeron_engine::hz_relay::HZ_RELAY_CAPABILITY`.
const HZ_RELAY_CAPABILITY: &str = "mimieyes.hzRelay.v1";
const HZ_REPLY: &str = "HzReply";

/// A pane tree as an agent writes it.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Spec {
    /// A session: "self", a chat id, a unique id prefix, or an exact title.
    Chat(String),
    Split {
        dir: HzDir,
        ratio: Option<f32>,
        a: Box<Spec>,
        b: Box<Spec>,
    },
}

/// Read `{"chat": ..}` / `{"split": "right"|"down", "ratio"?, "a", "b"}`.
pub(super) fn parse_spec(value: &Value) -> Result<Spec, String> {
    if let Some(chat) = value.get("chat") {
        let chat = chat
            .as_str()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .ok_or("a pane's \"chat\" must be a non-empty string")?;
        return Ok(Spec::Chat(chat.to_owned()));
    }
    let dir = match value.get("split").and_then(Value::as_str) {
        Some("right") => HzDir::Right,
        Some("down") => HzDir::Down,
        Some(other) => return Err(format!("unknown split {other:?} (use \"right\" or \"down\")")),
        None => return Err("a node needs \"chat\" or \"split\"".into()),
    };
    let ratio = match value.get("ratio") {
        None | Some(Value::Null) => None,
        Some(r) => Some(
            r.as_f64()
                .ok_or("\"ratio\" must be a number")?
                .clamp(0.1, 0.9) as f32,
        ),
    };
    let side = |key: &str| {
        value
            .get(key)
            .ok_or(format!("a split needs \"{key}\""))
            .and_then(parse_spec)
    };
    Ok(Spec::Split {
        dir,
        ratio,
        a: Box::new(side("a")?),
        b: Box::new(side("b")?),
    })
}

/// Every session a spec names, left to right / top to bottom.
fn spec_chats(spec: &Spec) -> Vec<&str> {
    match spec {
        Spec::Chat(c) => vec![c.as_str()],
        Spec::Split { a, b, .. } => {
            let mut out = spec_chats(a);
            out.extend(spec_chats(b));
            out
        }
    }
}

/// Resolve `key` among `(id, title)` candidates: full id, unique id
/// prefix, or exact (case-insensitive) title.
pub(super) fn resolve_among(candidates: &[(String, Option<String>)], key: &str) -> Result<String, String> {
    if let Some((id, _)) = candidates.iter().find(|(id, _)| id == key) {
        return Ok(id.clone());
    }
    let by_prefix: Vec<&String> = candidates
        .iter()
        .filter(|(id, _)| key.len() >= 4 && id.starts_with(key))
        .map(|(id, _)| id)
        .collect();
    if by_prefix.len() == 1 {
        return Ok(by_prefix[0].clone());
    }
    let by_title: Vec<&String> = candidates
        .iter()
        .filter(|(_, title)| {
            title
                .as_deref()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case(key.trim()))
        })
        .map(|(id, _)| id)
        .collect();
    match (by_prefix.len(), by_title.len()) {
        (_, 1) => Ok(by_title[0].clone()),
        (0, 0) => Err(format!(
            "no session of this project matches {key:?} (id, id prefix, or exact title)"
        )),
        _ => Err(format!("{key:?} matches several sessions; use its id")),
    }
}

/// The tree a spec builds, its leaves keyed by `leaf_for(chat)`; splits
/// without a ratio share evenly by the rows/columns on either side.
fn build(spec: &Spec, leaf_for: &mut dyn FnMut(&str) -> (String, String), split_id: &mut dyn FnMut() -> String) -> HzNode {
    match spec {
        Spec::Chat(key) => {
            let (leaf, chat) = leaf_for(key);
            HzNode::Leaf {
                id: leaf,
                chat: Some(chat),
            }
        }
        Spec::Split { dir, ratio, a, b } => {
            let a = build(a, leaf_for, split_id);
            let b = build(b, leaf_for, split_id);
            let ratio = ratio.unwrap_or_else(|| {
                let (na, nb) = match dir {
                    HzDir::Right => (a.columns(), b.columns()),
                    HzDir::Down => (a.rows(), b.rows()),
                };
                na as f32 / (na + nb) as f32
            });
            HzNode::Split {
                id: split_id(),
                dir: *dir,
                ratio,
                a: Box::new(a),
                b: Box::new(b),
            }
        }
    }
}

/// A tab's layout tree in the tool's own spec shape (for `list`).
fn spec_json(node: &HzNode) -> Value {
    match node {
        HzNode::Leaf { chat, .. } => json!({ "chat": chat }),
        HzNode::Split { dir, ratio, a, b, .. } => json!({
            "split": if *dir == HzDir::Right { "right" } else { "down" },
            "ratio": (ratio * 100.0).round() / 100.0,
            "a": spec_json(a),
            "b": spec_json(b),
        }),
    }
}

impl Shell {
    /// Keep one relay subscription per engine connection (re-made after a
    /// reconnect). Called once per frame from `hz_sync`.
    pub(super) fn hz_relay_tick(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        // Only an engine with the relay (an older or stock one has none).
        if !engine.engine_info().supports(HZ_RELAY_CAPABILITY) {
            return;
        }
        if self
            .hz
            .relay
            .as_ref()
            .is_some_and(|(current, _)| current.same_connection(&engine))
        {
            return;
        }
        let handle = engine.clone();
        let task = cx.spawn(async move |this, cx| {
            const RETRY: std::time::Duration = std::time::Duration::from_secs(3);
            loop {
                let mut rx = match handle.client().subscribe(WATCH_HZ_REQUESTS, json!({})).await {
                    Ok(rx) => rx,
                    Err(_) => {
                        // An engine without the relay (older) or a dropped
                        // link: try again later, quietly.
                        if this.update(cx, |_, _| {}).is_err() {
                            return;
                        }
                        cx.background_executor().timer(RETRY).await;
                        continue;
                    }
                };
                while let Some(item) = rx.recv().await {
                    let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
                        continue;
                    };
                    let params = item.get("params").cloned().unwrap_or(Value::Null);
                    let Ok(answer) = this.update(cx, |shell, cx| shell.hz_serve(&params, cx)) else {
                        return;
                    };
                    let reply = match answer {
                        Ok(result) => json!({ "id": id, "result": result }),
                        Err(error) => json!({ "id": id, "error": error }),
                    };
                    let _ = handle.client().call(HZ_REPLY, reply).await;
                }
                if this.update(cx, |_, _| {}).is_err() {
                    return;
                }
                cx.background_executor().timer(RETRY).await;
            }
        });
        self.hz.relay = Some((engine, task));
    }

    /// One agent request: `{ from, request: { op, tab?, name?, layout?, focus? } }`.
    fn hz_serve(&mut self, params: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let from = params
            .get("from")
            .and_then(Value::as_str)
            .ok_or("the request has no calling chat")?
            .to_owned();
        let request = params.get("request").cloned().unwrap_or(Value::Null);
        let key = self
            .hz_project_of(&from, cx)
            .ok_or("the calling chat is not known to this window yet")?;
        match request.get("op").and_then(Value::as_str) {
            Some("list") => Ok(self.hz_describe(&key, &from, cx)),
            Some("arrange") => self.hz_arrange(&key, &from, &request, cx),
            Some(other) => Err(format!("unknown op {other:?} (use \"list\" or \"arrange\")")),
            None => Err("\"op\" is required".into()),
        }
    }

    /// The project's sessions a pane may hold: `(id, title)`.
    fn hz_project_candidates(&self, key: &str, cx: &App) -> Vec<(String, Option<String>)> {
        self.state
            .read(cx)
            .chats
            .iter()
            .filter(|c| !c.archived && c.space_id.clone().unwrap_or_default() == key)
            .map(|c| (c.id.clone(), c.title.clone()))
            .collect()
    }

    /// `list`: the project's tabs, panes and layout trees.
    fn hz_describe(&self, key: &str, from: &str, cx: &App) -> Value {
        let state = self.state.read(cx);
        let project = state
            .space_row(key)
            .map(|s| s.display_name().to_owned())
            .unwrap_or_else(|| "No project".to_owned());
        let Some(layout) = self.settings.hz_layouts.get(key) else {
            return json!({ "project": project, "panes_on": self.settings.hz_panes, "tabs": [] });
        };
        let active = layout.active_tab().map(|t| t.id.clone());
        let tabs: Vec<Value> = layout
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, tab)| {
                let panes: Vec<Value> = tab
                    .root
                    .leaf_ids()
                    .into_iter()
                    .map(|leaf| {
                        let chat = tab.root.chat_of(leaf).cloned().flatten();
                        let title = chat.as_deref().and_then(|id| {
                            state.chats.iter().find(|c| c.id == id).and_then(|c| c.title.clone())
                        });
                        json!({
                            "chat": chat,
                            "shortId": chat.as_deref().map(|id| id.chars().take(8).collect::<String>()),
                            "title": title.unwrap_or_else(|| "New session".to_owned()),
                            "you": chat.as_deref() == Some(from),
                            "focused": tab.focused.as_deref() == Some(leaf),
                        })
                    })
                    .collect();
                json!({
                    "id": tab.id,
                    "place": ix + 1,
                    "name": tab.name,
                    "active": active.as_deref() == Some(tab.id.as_str()),
                    "panes": panes,
                    "layout": spec_json(&tab.root),
                })
            })
            .collect();
        json!({ "project": project, "panes_on": self.settings.hz_panes, "tabs": tabs })
    }

    /// `arrange`: lay out (or create) one tab of the caller's project.
    fn hz_arrange(&mut self, key: &str, from: &str, request: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let spec = match request.get("layout") {
            None | Some(Value::Null) => None,
            Some(layout) => Some(parse_spec(layout)?),
        };
        let name = request
            .get("name")
            .and_then(Value::as_str)
            .map(|n| n.trim().to_owned());
        let focus = request.get("focus").and_then(Value::as_bool).unwrap_or(false);
        let tab_key = request
            .get("tab")
            .and_then(Value::as_str)
            .unwrap_or("current")
            .trim()
            .to_owned();

        // Resolve every named session first: nothing changes on an error.
        let candidates = self.hz_project_candidates(key, cx);
        let mut chats: Vec<String> = Vec::new();
        if let Some(spec) = &spec {
            for name in spec_chats(spec) {
                let id = if name.eq_ignore_ascii_case("self") {
                    from.to_owned()
                } else {
                    resolve_among(&candidates, name)?
                };
                if chats.contains(&id) {
                    return Err(format!("{name:?} appears twice in the layout"));
                }
                chats.push(id);
            }
        }

        let mut layout = self.settings.hz_layouts.get(key).cloned().unwrap_or_default();
        // The tab to lay out.
        let target = match tab_key.as_str() {
            "new" => None,
            "current" | "" => layout
                .tab_with_chat(from)
                .or_else(|| layout.active_tab())
                .map(|t| t.id.clone()),
            other => Some(
                layout
                    .tabs
                    .iter()
                    .find(|t| t.id == other || t.name.trim().eq_ignore_ascii_case(other))
                    .map(|t| t.id.clone())
                    .ok_or(format!("no tab {other:?} in this project"))?,
            ),
        };
        if target.is_none() && spec.is_none() {
            return Err("a new tab needs a \"layout\"".into());
        }

        // Leaves keep their ids (and so their mounted conversations) when
        // a placed session moves: take each one out of wherever it is.
        let mut leaf_of: std::collections::HashMap<String, String> = Default::default();
        if spec.is_some() {
            for chat in &chats {
                if let Some((tab, leaf)) = layout.slot_of(chat) {
                    leaf_of.insert(chat.clone(), leaf.clone());
                    if Some(&tab) != target.as_ref()
                        && let Some(t) = layout.tabs.iter_mut().find(|t| t.id == tab)
                        && t.close_leaf(&leaf)
                    {
                        layout.remove_tab(&tab);
                    }
                }
            }
        }
        let tab_id = match target {
            Some(id) => id,
            None => {
                let id = Self::hz_id("t");
                layout.tabs.push(HzViewTab::new(id.clone(), Self::hz_id("l")));
                id
            }
        };
        let place = layout.tabs.iter().position(|t| t.id == tab_id).map_or(0, |ix| ix + 1);
        let Some(tab) = layout.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return Err("the tab went away".into());
        };
        if let Some(spec) = &spec {
            let mut names = chats.clone().into_iter();
            let mut leaf_for = |_: &str| {
                let chat = names.next().unwrap_or_default();
                let leaf = leaf_of.get(&chat).cloned().unwrap_or_else(|| Self::hz_id("l"));
                (leaf, chat)
            };
            let mut split_id = || Self::hz_id("s");
            tab.root = build(spec, &mut leaf_for, &mut split_id);
            tab.zoom = None;
            let first = tab.root.leaf_ids().first().map(|s| (*s).to_owned());
            tab.focused = tab.root.slot_of(from).map(str::to_owned).or(first);
        }
        if let Some(name) = name {
            tab.name = name;
        }
        let on_screen = key == self.hz.project
            && layout.active_tab().is_some_and(|t| t.id == tab_id);
        if focus || on_screen || layout.tabs.len() == 1 {
            layout.active = Some(tab_id.clone());
        }
        layout.prune();
        // The window shows panes from now on (the agent asked for them).
        self.settings.hz_panes = true;
        self.settings.hz_layouts.insert(key.to_owned(), layout);
        if focus && key != self.hz.project {
            let primary = self
                .settings
                .hz_layouts
                .get(key)
                .and_then(HzSpaceLayout::active_tab)
                .and_then(HzViewTab::primary_chat);
            self.hz_open_project(key.to_owned(), primary, cx);
        }
        self.schedule_save(cx);
        cx.notify();
        let mut answer = self.hz_describe(key, from, cx);
        if let Some(obj) = answer.as_object_mut() {
            obj.insert("arranged".into(), json!({ "tab": tab_id, "place": place }));
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_spec_reads_splits_and_panes() {
        let spec = parse_spec(&json!({
            "split": "right",
            "a": { "split": "down", "a": { "chat": "self" }, "b": { "chat": "Researcher" } },
            "b": { "chat": "Worker" }
        }))
        .unwrap();
        assert_eq!(spec_chats(&spec), vec!["self", "Researcher", "Worker"]);
        assert!(parse_spec(&json!({ "split": "left", "a": {}, "b": {} })).is_err());
        assert!(parse_spec(&json!({ "split": "down", "a": { "chat": "x" } })).is_err());
        assert!(parse_spec(&json!({ "chat": "" })).is_err());
        // Ratios clamp so no pane vanishes.
        let Spec::Split { ratio, .. } =
            parse_spec(&json!({ "split": "down", "ratio": 0.99, "a": { "chat": "a" }, "b": { "chat": "b" } })).unwrap()
        else {
            panic!()
        };
        assert_eq!(ratio, Some(0.9));
    }

    #[test]
    fn sessions_resolve_by_id_prefix_or_title() {
        let candidates = vec![
            ("a1b2c3d4-0000".to_owned(), Some("Worker".to_owned())),
            ("a1b2ffff-0000".to_owned(), Some("Researcher".to_owned())),
            ("9f8e7d6c-0000".to_owned(), None),
        ];
        assert_eq!(resolve_among(&candidates, "9f8e7d6c-0000").unwrap(), "9f8e7d6c-0000");
        assert_eq!(resolve_among(&candidates, "9f8e").unwrap(), "9f8e7d6c-0000");
        assert_eq!(resolve_among(&candidates, "worker").unwrap(), "a1b2c3d4-0000");
        // An ambiguous prefix, and nothing at all.
        assert!(resolve_among(&candidates, "a1b2").is_err());
        assert!(resolve_among(&candidates, "Planner").is_err());
    }

    #[test]
    fn a_built_tree_evens_unratioed_splits_and_keeps_leaf_ids() {
        let spec = parse_spec(&json!({
            "split": "right",
            "a": { "split": "down", "a": { "chat": "p" }, "b": { "chat": "r" } },
            "b": { "chat": "w" }
        }))
        .unwrap();
        let mut n = 0;
        let mut leaf_for = |chat: &str| (format!("leaf-{chat}"), chat.to_owned());
        let mut split_id = || {
            n += 1;
            format!("s{n}")
        };
        let root = build(&spec, &mut leaf_for, &mut split_id);
        let (tiles, _) = root.tiles_and_splits();
        assert_eq!(tiles.len(), 3);
        // Two columns: the left one stacked, the right one whole.
        assert!((tiles[0].w - 0.5).abs() < 0.01 && (tiles[0].h - 0.5).abs() < 0.01);
        assert!((tiles[2].w - 0.5).abs() < 0.01 && (tiles[2].h - 1.0).abs() < 0.01);
        assert_eq!(root.slot_of("w"), Some("leaf-w"));
    }
}
