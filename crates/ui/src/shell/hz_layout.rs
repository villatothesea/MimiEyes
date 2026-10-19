//! MimiEyes (VENDORED.md #3): the render-level tab/pane model.
//!
//! The four levels and how they hang together:
//!
//! * **Workspace** — the sidebar's space filter (`""` = All projects). It
//!   owns at most one [`HzSpaceLayout`]; having one means "panes on" there.
//! * **Tab** ([`HzViewTab`]) — a named arrangement of panes inside one
//!   workspace. Exactly one tab of a layout is active (on screen).
//! * **Pane** (a leaf of [`HzNode`]) — a slot in the tab's split tree. It
//!   holds one chat or nothing (it then draws a session picker). Slots tile by
//!   a binary split tree — `Split right/down` divides a leaf, a divider drag
//!   changes a split's ratio, closing a leaf collapses its parent split onto
//!   the sibling.
//! * **Chat** — an engine session. Inside one layout a chat sits in at most
//!   one pane, and only chats of the workspace may sit there at all
//!   ([`HzSpaceLayout::heal`] enforces both).
//!
//! The selected chat and the focused pane mirror each other: the selected
//! chat's pane is the active tab's focused one ([`HzSpaceLayout::reveal`]),
//! and a newly selected chat lands in the active tab ([`HzSpaceLayout::adopt`]).
//!
//! Everything here is local view state: no engine concepts, no RPC. The model
//! serializes straight into `UiSettings::hz_layouts`, keyed by the workspace.
//! Pure data + pure functions so the rules and the geometry are unit-testable
//! without gpui.

use serde::{Deserialize, Serialize};

/// A slot keeps at least this much of its split (divider drags clamp here).
pub const RATIO_MIN: f32 = 0.05;
/// Same bound on the pointer, so a drag can never reach a hidden pane.
const DRAG_MIN: f32 = 0.10;
const DRAG_MAX: f32 = 0.90;
const EPS: f32 = 0.002;

/// Which way a split divides its rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HzDir {
    /// Side by side (a vertical divider).
    Right,
    /// Stacked (a horizontal divider).
    Down,
}

/// The tiling tree of one view tab. Nodes carry stable ids so element ids,
/// drag state and focus survive re-renders and restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HzNode {
    Leaf {
        id: String,
        /// The chat shown in this slot; `None` draws a session picker.
        #[serde(default)]
        chat: Option<String>,
    },
    Split {
        id: String,
        dir: HzDir,
        ratio: f32,
        a: Box<HzNode>,
        b: Box<HzNode>,
    },
}

/// One view tab: a named group of pane slots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HzViewTab {
    pub id: String,
    /// User label; empty shows "Tab N" (its strip position).
    #[serde(default)]
    pub name: String,
    pub root: HzNode,
    /// The slot a click last focused (receives the next sidebar pick).
    #[serde(default)]
    pub focused: Option<String>,
    /// The zoomed slot, if any (it fills the pane area alone).
    #[serde(default)]
    pub zoom: Option<String>,
}

/// All of a space's view tabs (the `hz_layouts` value).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HzSpaceLayout {
    pub tabs: Vec<HzViewTab>,
    /// The on-screen tab's id; `None` or dangling falls back to the first tab.
    pub active: Option<String>,
}

/// A slot laid out on screen: its leaf id, its chat, and where it sits
/// (fractions 0..1 of the pane area).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HzTile {
    pub slot: String,
    pub chat: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A divider: the split, as fractions of the pane area, and where its border is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HzSplit {
    pub id: String,
    /// Panes side by side (a vertical divider).
    pub horizontal: bool,
    pub ratio: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl HzNode {
    pub fn leaf(id: impl Into<String>) -> Self {
        HzNode::Leaf {
            id: id.into(),
            chat: None,
        }
    }

    /// Every leaf id, left to right / top to bottom.
    pub fn leaf_ids(&self) -> Vec<&str> {
        match self {
            HzNode::Leaf { id, .. } => vec![id.as_str()],
            HzNode::Split { a, b, .. } => {
                let mut out = a.leaf_ids();
                out.extend(b.leaf_ids());
                out
            }
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            HzNode::Leaf { .. } => 1,
            HzNode::Split { a, b, .. } => a.leaf_count() + b.leaf_count(),
        }
    }

    /// Every chat id shown anywhere in the tree.
    pub fn chats(&self) -> Vec<String> {
        match self {
            HzNode::Leaf { chat, .. } => chat.iter().cloned().collect(),
            HzNode::Split { a, b, .. } => {
                let mut out = a.chats();
                out.extend(b.chats());
                out
            }
        }
    }

    /// The leaf holding `chat`, if any.
    pub fn slot_of(&self, chat: &str) -> Option<&str> {
        match self {
            HzNode::Leaf { id, chat: c } => (c.as_deref() == Some(chat)).then_some(id.as_str()),
            HzNode::Split { a, b, .. } => a.slot_of(chat).or_else(|| b.slot_of(chat)),
        }
    }

    /// The chat in leaf `id`.
    pub fn chat_of(&self, id: &str) -> Option<&Option<String>> {
        match self {
            HzNode::Leaf { id: leaf, chat } if leaf.as_str() == id => Some(chat),
            HzNode::Leaf { .. } => None,
            HzNode::Split { a, b, .. } => a.chat_of(id).or_else(|| b.chat_of(id)),
        }
    }

    /// Put `chat` into leaf `id` (None empties the slot again).
    pub fn assign(&mut self, id: &str, chat: Option<String>) -> bool {
        match self {
            HzNode::Leaf { id: leaf, chat: c } if leaf.as_str() == id => {
                *c = chat;
                true
            }
            HzNode::Leaf { .. } => false,
            HzNode::Split { a, b, .. } => a.assign(id, chat.clone()) || b.assign(id, chat),
        }
    }

    /// Trade the two leaves' chats (a pane-tag drop onto another pane).
    pub fn swap_chats(&mut self, a_id: &str, b_id: &str) -> bool {
        let (Some(a), Some(b)) = (self.chat_of(a_id).cloned(), self.chat_of(b_id).cloned()) else {
            return false;
        };
        self.assign(a_id, b);
        self.assign(b_id, a);
        true
    }

    /// Split leaf `id` along `dir`: the leaf keeps its chat, a fresh empty
    /// slot takes the other half.
    pub fn split_leaf(
        &mut self,
        leaf_id: &str,
        dir: HzDir,
        split_id: String,
        new_leaf_id: String,
    ) -> bool {
        match self {
            HzNode::Leaf { id, .. } if id.as_str() == leaf_id => {
                let old = std::mem::replace(self, HzNode::leaf(new_leaf_id.clone()));
                *self = HzNode::Split {
                    id: split_id,
                    dir,
                    ratio: 0.5,
                    a: Box::new(old),
                    b: Box::new(HzNode::leaf(new_leaf_id)),
                };
                true
            }
            HzNode::Leaf { .. } => false,
            HzNode::Split { a, b, .. } => {
                a.split_leaf(leaf_id, dir, split_id.clone(), new_leaf_id.clone())
                    || b.split_leaf(leaf_id, dir, split_id, new_leaf_id)
            }
        }
    }

    /// The tree with leaf `id` removed; `None` when the tree is left empty.
    /// The removed leaf's parent split collapses onto its sibling, which
    /// inherits the freed space — Herdr's "neighbour takes it".
    pub fn without_leaf(self, leaf_id: &str) -> Option<HzNode> {
        match self {
            HzNode::Leaf { id, .. } if id.as_str() == leaf_id => None,
            leaf @ HzNode::Leaf { .. } => Some(leaf),
            HzNode::Split {
                id,
                dir,
                ratio,
                a,
                b,
            } => match (a.without_leaf(leaf_id), b.without_leaf(leaf_id)) {
                (None, None) => None,
                (Some(node), None) | (None, Some(node)) => Some(node),
                (Some(a), Some(b)) => Some(HzNode::Split {
                    id,
                    dir,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
            },
        }
    }

    /// The leaf taken out (for a move between tabs): the trimmed tree and the
    /// leaf itself.
    pub fn take_leaf(self, leaf_id: &str) -> (Option<HzNode>, Option<HzNode>) {
        fn find(node: &HzNode, leaf_id: &str) -> Option<HzNode> {
            match node {
                HzNode::Leaf { id, .. } if id.as_str() == leaf_id => Some(node.clone()),
                HzNode::Leaf { .. } => None,
                HzNode::Split { a, b, .. } => find(a, leaf_id).or_else(|| find(b, leaf_id)),
            }
        }
        let leaf = find(&self, leaf_id);
        (self.without_leaf(leaf_id), leaf)
    }

    /// Append `leaf` as a split right of the whole tree, sized so every leaf
    /// keeps an even share.
    pub fn append_leaf(&mut self, leaf: HzNode, split_id: String) {
        let total = self.leaf_count() + 1;
        let ratio = (total - 1) as f32 / total as f32;
        let old = std::mem::replace(self, HzNode::leaf(String::new()));
        *self = HzNode::Split {
            id: split_id,
            dir: HzDir::Right,
            ratio,
            a: Box::new(old),
            b: Box::new(leaf),
        };
    }

    /// Move a split's border to `ratio` (a divider drag).
    pub fn set_ratio(&mut self, split_id: &str, ratio: f32) -> bool {
        match self {
            HzNode::Leaf { .. } => false,
            HzNode::Split {
                id, ratio: r, a, b, ..
            } => {
                if id.as_str() == split_id {
                    *r = ratio.clamp(RATIO_MIN, 1.0 - RATIO_MIN);
                    true
                } else {
                    a.set_ratio(split_id, ratio) || b.set_ratio(split_id, ratio)
                }
            }
        }
    }

    /// How many columns the subtree spans side by side: a right split adds
    /// its halves' columns, a down split stacks them (the wider one counts).
    pub fn columns(&self) -> usize {
        match self {
            HzNode::Leaf { .. } => 1,
            HzNode::Split { dir: HzDir::Right, a, b, .. } => a.columns() + b.columns(),
            HzNode::Split { a, b, .. } => a.columns().max(b.columns()),
        }
    }

    /// How many rows the subtree stacks (the mirror of [`Self::columns`]).
    pub fn rows(&self) -> usize {
        match self {
            HzNode::Leaf { .. } => 1,
            HzNode::Split { dir: HzDir::Down, a, b, .. } => a.rows() + b.rows(),
            HzNode::Split { a, b, .. } => a.rows().max(b.rows()),
        }
    }

    /// Even out every row and every column: each right split shares its
    /// width by the columns on either side, each down split its height by
    /// the rows — so every column of a row ends up the same width and every
    /// row of a column the same height, however the splits nest.
    pub fn even(&mut self) {
        if let HzNode::Split { dir, ratio, a, b, .. } = self {
            a.even();
            b.even();
            let (na, nb) = match dir {
                HzDir::Right => (a.columns(), b.columns()),
                HzDir::Down => (a.rows(), b.rows()),
            };
            *ratio = na as f32 / (na + nb) as f32;
        }
    }

    /// The tree laid out inside (x, y, w, h): slots become tiles, splits
    /// become dividers. Fractions of the pane area.
    pub fn layout(
        &self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tiles: &mut Vec<HzTile>,
        splits: &mut Vec<HzSplit>,
    ) {
        match self {
            HzNode::Leaf { id, chat } => tiles.push(HzTile {
                slot: id.clone(),
                chat: chat.clone(),
                x,
                y,
                w,
                h,
            }),
            HzNode::Split {
                id,
                dir,
                ratio,
                a,
                b,
            } => {
                let ratio = ratio.clamp(RATIO_MIN, 1.0 - RATIO_MIN);
                splits.push(HzSplit {
                    id: id.clone(),
                    horizontal: *dir == HzDir::Right,
                    ratio,
                    x,
                    y,
                    w,
                    h,
                });
                match dir {
                    HzDir::Right => {
                        a.layout(x, y, ratio * w, h, tiles, splits);
                        b.layout(x + ratio * w, y, (1.0 - ratio) * w, h, tiles, splits);
                    }
                    HzDir::Down => {
                        a.layout(x, y, w, ratio * h, tiles, splits);
                        b.layout(x, y + ratio * h, w, (1.0 - ratio) * h, tiles, splits);
                    }
                }
            }
        }
    }

    pub fn tiles_and_splits(&self) -> (Vec<HzTile>, Vec<HzSplit>) {
        let (mut tiles, mut splits) = (Vec::new(), Vec::new());
        self.layout(0.0, 0.0, 1.0, 1.0, &mut tiles, &mut splits);
        (tiles, splits)
    }

    /// One leaf's rectangle in the layout (the zoomed tile, a swap ghost).
    pub fn tile_of(&self, leaf_id: &str) -> Option<HzTile> {
        self.tiles_and_splits()
            .0
            .into_iter()
            .find(|t| t.slot == leaf_id)
    }
}

impl HzViewTab {
    pub fn new(id: impl Into<String>, leaf_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            root: HzNode::leaf(leaf_id),
            focused: None,
            zoom: None,
        }
    }

    /// The slot a sidebar pick or a newly selected session lands in: the
    /// focused slot when it is empty (the user pointed at it), else the first
    /// empty one (a filled slot is never traded away while one is free), else
    /// the focused, else the first.
    pub fn pick_slot(&self) -> Option<String> {
        let leaves = self.root.leaf_ids();
        let empty = |id: &str| self.root.chat_of(id).is_some_and(|c| c.is_none());
        let focused = self.focused.as_deref().filter(|id| leaves.contains(id));
        focused
            .filter(|id| empty(id))
            .or_else(|| leaves.iter().copied().find(|id| empty(id)))
            .or(focused)
            .or_else(|| leaves.first().copied())
            .map(str::to_owned)
    }

    /// The chat that speaks for the tab: the focused slot's, else the first
    /// one it shows. `None` when every slot is empty.
    pub fn primary_chat(&self) -> Option<String> {
        self.focused
            .as_deref()
            .and_then(|leaf| self.root.chat_of(leaf).cloned().flatten())
            .or_else(|| self.root.chats().into_iter().next())
    }

    /// Focus slot `leaf`; a zoom on another slot lifts so the focused pane is
    /// always on screen. `true` when anything changed.
    pub fn focus(&mut self, leaf: &str) -> bool {
        let mut changed = false;
        if self.focused.as_deref() != Some(leaf) {
            self.focused = Some(leaf.to_owned());
            changed = true;
        }
        if self.zoom.as_deref().is_some_and(|z| z != leaf) {
            self.zoom = None;
            changed = true;
        }
        changed
    }

    /// The tab after `leaf_id` closed; `true` when its last slot went — the
    /// tree cannot shrink to nothing, so the caller drops the tab itself.
    pub fn close_leaf(&mut self, leaf_id: &str) -> bool {
        let Some(root) = self.root.clone().without_leaf(leaf_id) else {
            return true;
        };
        self.root = root;
        if self.focused.as_deref() == Some(leaf_id) {
            self.focused = self.root.leaf_ids().first().map(|s| (*s).to_owned());
        }
        if self.zoom.as_deref() == Some(leaf_id) {
            self.zoom = None;
        }
        false
    }

    /// The chip's caption: its place, plus the name when it has one.
    pub fn chip_text(&self, position: usize) -> String {
        let name = self.name.trim();
        if name.is_empty() || name == (position + 1).to_string() {
            (position + 1).to_string()
        } else {
            format!("{} {}", position + 1, name)
        }
    }
}

impl HzSpaceLayout {
    /// The tab on screen (the active id, else the first — a stale id heals).
    pub fn active_tab(&self) -> Option<&HzViewTab> {
        self.tabs
            .iter()
            .find(|t| Some(&t.id) == self.active.as_ref())
            .or_else(|| self.tabs.first())
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut HzViewTab> {
        if self.active.is_none() {
            self.active = self.tabs.first().map(|t| t.id.clone());
        }
        let id = self.active.clone()?;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// Which tab holds `chat` in a slot (any tab, not just the active one).
    pub fn tab_with_chat(&self, chat: &str) -> Option<&HzViewTab> {
        self.tabs.iter().find(|t| t.root.slot_of(chat).is_some())
    }

    /// The (tab id, slot id) showing `chat`, if any.
    pub fn slot_of(&self, chat: &str) -> Option<(String, String)> {
        self.tabs.iter().find_map(|t| {
            t.root
                .slot_of(chat)
                .map(|slot| (t.id.clone(), slot.to_owned()))
        })
    }

    /// Bring `chat`'s pane on screen: its tab becomes active, its slot
    /// focused (un-zooming a sibling). `false` when no pane shows it.
    pub fn reveal(&mut self, chat: &str) -> bool {
        let Some((tab, slot)) = self.slot_of(chat) else {
            return false;
        };
        self.active = Some(tab.clone());
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) {
            t.focus(&slot);
        }
        true
    }

    /// Put `chat` (in no pane yet) into the active tab's
    /// [`HzViewTab::pick_slot`] and focus it. `false` without a tab.
    pub fn adopt(&mut self, chat: &str) -> bool {
        let Some(tab) = self.active_tab_mut() else {
            return false;
        };
        let Some(slot) = tab.pick_slot() else {
            return false;
        };
        tab.root.assign(&slot, Some(chat.to_owned()));
        tab.focus(&slot);
        true
    }

    /// Take tab `id` out; when it was active, the neighbour that slides into
    /// its place (else the one before) takes over.
    pub fn remove_tab(&mut self, id: &str) -> Option<HzViewTab> {
        let ix = self.tabs.iter().position(|t| t.id == id)?;
        let tab = self.tabs.remove(ix);
        if self.active.as_deref() == Some(id) {
            self.active = self
                .tabs
                .get(ix.min(self.tabs.len().saturating_sub(1)))
                .map(|t| t.id.clone());
        }
        Some(tab)
    }

    /// Enforce the layout's invariants: only chats `keep` accepts sit in
    /// panes (live, unarchived, of this workspace), each in one pane only
    /// (the first one wins), and focus/zoom point at real slots. `true` when
    /// anything changed.
    pub fn heal(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let mut changed = false;
        let mut seen = std::collections::HashSet::new();
        for tab in &mut self.tabs {
            let leaves: Vec<String> = tab.root.leaf_ids().into_iter().map(str::to_owned).collect();
            for leaf in &leaves {
                let Some(Some(chat)) = tab.root.chat_of(leaf).cloned() else {
                    continue;
                };
                if !keep(&chat) || !seen.insert(chat) {
                    tab.root.assign(leaf, None);
                    changed = true;
                }
            }
            if tab
                .focused
                .as_deref()
                .is_some_and(|id| !leaves.iter().any(|l| l == id))
            {
                tab.focused = leaves.first().cloned();
                changed = true;
            }
            if tab
                .zoom
                .as_deref()
                .is_some_and(|id| leaves.len() < 2 || !leaves.iter().any(|l| l == id))
            {
                tab.zoom = None;
                changed = true;
            }
        }
        let before = (self.tabs.len(), self.active.clone());
        self.prune();
        changed || before != (self.tabs.len(), self.active.clone())
    }

    /// Drop empty tabs and heal `active` (`None` once no tab is left — the
    /// caller then drops the layout and the stock single-chat view returns).
    pub fn prune(&mut self) {
        self.tabs.retain(|t| t.root.leaf_count() > 0);
        if self
            .active
            .as_ref()
            .is_some_and(|id| !self.tabs.iter().any(|t| &t.id == id))
        {
            self.active = self.tabs.first().map(|t| t.id.clone());
        }
        if self.tabs.is_empty() {
            self.active = None;
        }
    }
}

/// Where a divider is drawn: slot borders rarely meet exactly at
/// `x + ratio * w` (clamped minimums, a zoomed neighbour), so the line goes
/// in the middle of the gap between the two sides' actual edges — it never
/// cuts into a pane's tag.
pub fn gutter_at(tiles: &[HzTile], s: &HzSplit) -> f32 {
    let nominal = if s.horizontal {
        s.x + s.ratio * s.w
    } else {
        s.y + s.ratio * s.h
    };
    let eps = 0.001;
    let (mut before, mut after) = (f32::NEG_INFINITY, f32::INFINITY);
    for t in tiles {
        let (cx, cy) = (t.x + t.w / 2.0, t.y + t.h / 2.0);
        if cx < s.x - eps || cx > s.x + s.w + eps || cy < s.y - eps || cy > s.y + s.h + eps {
            continue;
        }
        let (lo, hi, mid) = if s.horizontal {
            (t.x, t.x + t.w, cx)
        } else {
            (t.y, t.y + t.h, cy)
        };
        if mid < nominal {
            before = before.max(hi);
        } else {
            after = after.min(lo);
        }
    }
    if before.is_finite() && after.is_finite() && before <= after + eps {
        let mid = (before + after) / 2.0;
        if (mid - nominal).abs() < 0.08 {
            return mid;
        }
    }
    nominal
}

/// The ratio a divider takes when the pointer is at `(px, py)` inside an area
/// of `aw` x `ah`; never so far that a pane disappears.
pub fn drag_ratio(s: &HzSplit, px: f32, py: f32, aw: f32, ah: f32) -> f32 {
    if aw < 1.0 || ah < 1.0 {
        return s.ratio;
    }
    let r = if s.horizontal {
        (px / aw - s.x) / s.w.max(EPS)
    } else {
        (py / ah - s.y) / s.h.max(EPS)
    };
    r.clamp(DRAG_MIN, DRAG_MAX)
}

/// The place in a strip a pointer at `x` is over, from the chips' (left, width).
pub fn tab_slot(x: f32, chips: &[(f32, f32)]) -> usize {
    chips
        .iter()
        .position(|(left, width)| x < left + width / 2.0)
        .unwrap_or(chips.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str) -> HzNode {
        HzNode::Leaf {
            id: id.into(),
            chat: Some(format!("chat-{id}")),
        }
    }

    #[test]
    fn a_split_gives_two_halves_and_a_divider_between_them() {
        let mut root = leaf("a");
        assert!(root.split_leaf("a", HzDir::Right, "s".into(), "b".into()));
        let (tiles, splits) = root.tiles_and_splits();
        assert_eq!(tiles.len(), 2);
        assert_eq!(splits.len(), 1);
        assert!((tiles[0].w - 0.5).abs() < EPS && (tiles[1].x - 0.5).abs() < EPS);
        assert_eq!(splits[0].ratio, 0.5);
        // The new slot is empty: it draws the picker.
        assert_eq!(root.chat_of("b"), Some(&None));
        // Down splits stack instead.
        let mut root = leaf("a");
        assert!(root.split_leaf("a", HzDir::Down, "s".into(), "b".into()));
        let (tiles, splits) = root.tiles_and_splits();
        assert!((tiles[0].h - 0.5).abs() < EPS && (tiles[1].y - 0.5).abs() < EPS);
        assert!(!splits[0].horizontal);
    }

    #[test]
    fn closing_a_leaf_collapses_its_split_onto_the_sibling() {
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s".into(), "b".into());
        root.assign("b", Some("chat-b".into()));
        let root = root.without_leaf("b").unwrap();
        let (tiles, splits) = root.tiles_and_splits();
        assert_eq!(tiles.len(), 1);
        assert!(splits.is_empty());
        assert_eq!(tiles[0].w, 1.0);
        // Removing the only leaf empties the tree.
        assert!(root.without_leaf("a").is_none());
    }

    #[test]
    fn a_nested_close_keeps_the_outer_split() {
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s1".into(), "b".into());
        root.split_leaf("b", HzDir::Down, "s2".into(), "c".into());
        let root = root.without_leaf("c").unwrap();
        let (tiles, splits) = root.tiles_and_splits();
        assert_eq!(tiles.len(), 2);
        assert_eq!(splits.len(), 1);
        // b took c's space, both under s1.
        assert!((tiles[1].y - 0.0).abs() < EPS && (tiles[1].h - 1.0).abs() < EPS);
    }

    #[test]
    fn dragging_a_divider_moves_only_its_own_border() {
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s".into(), "b".into());
        assert!(root.set_ratio("s", 0.7));
        let (tiles, _) = root.tiles_and_splits();
        assert!((tiles[0].w - 0.7).abs() < EPS && (tiles[1].x - 0.7).abs() < EPS);
        // The pointer clamps so a pane can never vanish.
        assert!(root.set_ratio("s", 0.001));
        let (tiles, _) = root.tiles_and_splits();
        assert!((tiles[0].w - RATIO_MIN).abs() < 0.001);
    }

    #[test]
    fn even_out_shares_rows_and_columns() {
        // a | (b / c): two columns, the right one stacked — columns halve,
        // the stacked rows halve.
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s1".into(), "b".into());
        root.split_leaf("b", HzDir::Down, "s2".into(), "c".into());
        root.set_ratio("s1", 0.2);
        root.set_ratio("s2", 0.9);
        root.even();
        let (tiles, _) = root.tiles_and_splits();
        assert!((tiles[0].w - 0.5).abs() < EPS);
        assert!((tiles[1].w - 0.5).abs() < EPS);
        assert!((tiles[1].h - 0.5).abs() < EPS && (tiles[2].h - 0.5).abs() < EPS);

        // a | b | c built by nesting: three equal columns.
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s1".into(), "b".into());
        root.split_leaf("b", HzDir::Right, "s2".into(), "c".into());
        root.even();
        let (tiles, _) = root.tiles_and_splits();
        for t in &tiles {
            assert!((t.w - 1.0 / 3.0).abs() < EPS);
        }

        // (a | b) over c: the top row's two columns halve, the rows halve.
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Down, "s1".into(), "c".into());
        root.split_leaf("a", HzDir::Right, "s2".into(), "b".into());
        root.set_ratio("s1", 0.8);
        root.even();
        let (tiles, _) = root.tiles_and_splits();
        assert!((tiles[0].h - 0.5).abs() < EPS && (tiles[2].h - 0.5).abs() < EPS);
        assert!((tiles[0].w - 0.5).abs() < EPS && (tiles[1].w - 0.5).abs() < EPS);
        assert!((tiles[2].w - 1.0).abs() < EPS);
    }

    #[test]
    fn the_next_pick_prefers_an_empty_then_the_focused_slot() {
        let mut tab = HzViewTab::new("t", "a");
        tab.root
            .split_leaf("a", HzDir::Right, "s".into(), "b".into());
        tab.root.assign("a", Some("chat-a".into()));
        // The only empty slot takes it, focused or not.
        assert_eq!(tab.pick_slot().as_deref(), Some("b"));
        tab.focused = Some("a".into());
        assert_eq!(tab.pick_slot().as_deref(), Some("b"));
        // Everything filled: the focused slot, then the first.
        tab.root.assign("b", Some("chat-b".into()));
        assert_eq!(tab.pick_slot().as_deref(), Some("a"));
        tab.focused = Some("gone".into());
        assert_eq!(tab.pick_slot().as_deref(), Some("a"));
        tab.focused = None;
        assert_eq!(tab.pick_slot().as_deref(), Some("a"));
        // A focused empty slot wins over an earlier empty one.
        tab.root
            .split_leaf("b", HzDir::Down, "s2".into(), "c".into());
        tab.root.assign("b", None);
        tab.focused = Some("c".into());
        assert_eq!(tab.pick_slot().as_deref(), Some("c"));
    }

    fn two_tabs() -> HzSpaceLayout {
        // t1: a | b (b empty), t2: c — chats chat-a, chat-c.
        let mut t1 = HzViewTab::new("t1", "a");
        t1.root
            .split_leaf("a", HzDir::Right, "s".into(), "b".into());
        t1.root.assign("a", Some("chat-a".into()));
        let mut t2 = HzViewTab::new("t2", "c");
        t2.root.assign("c", Some("chat-c".into()));
        HzSpaceLayout {
            tabs: vec![t1, t2],
            active: Some("t1".into()),
        }
    }

    #[test]
    fn revealing_a_chat_switches_to_its_tab_and_focuses_its_pane() {
        let mut layout = two_tabs();
        assert!(layout.reveal("chat-c"));
        assert_eq!(layout.active.as_deref(), Some("t2"));
        assert_eq!(layout.tabs[1].focused.as_deref(), Some("c"));
        // A zoom on a sibling lifts so the revealed pane is visible.
        layout.tabs[0].zoom = Some("b".into());
        assert!(layout.reveal("chat-a"));
        assert_eq!(layout.active.as_deref(), Some("t1"));
        assert!(layout.tabs[0].zoom.is_none());
        assert!(!layout.reveal("chat-x"));
    }

    #[test]
    fn adopting_fills_the_active_tabs_free_pane_before_replacing() {
        let mut layout = two_tabs();
        assert!(layout.adopt("chat-x"));
        assert_eq!(layout.tabs[0].root.chat_of("b"), Some(&Some("chat-x".into())));
        assert_eq!(layout.tabs[0].focused.as_deref(), Some("b"));
        // No free pane: the focused one takes it.
        assert!(layout.adopt("chat-y"));
        assert_eq!(layout.tabs[0].root.chat_of("b"), Some(&Some("chat-y".into())));
        assert_eq!(layout.tabs[0].root.chat_of("a"), Some(&Some("chat-a".into())));
        assert!(!HzSpaceLayout::default().adopt("chat-z"));
    }

    #[test]
    fn healing_drops_foreign_and_duplicate_chats_and_stale_refs() {
        let mut layout = two_tabs();
        layout.tabs[0].root.assign("b", Some("chat-c".into()));
        layout.tabs[0].focused = Some("gone".into());
        layout.tabs[1].zoom = Some("c".into());
        assert!(layout.heal(|chat| chat != "chat-a"));
        // chat-a is not this workspace's; chat-c stays in its first pane only.
        assert_eq!(layout.tabs[0].root.chats(), vec!["chat-c".to_string()]);
        assert!(layout.tabs[1].root.chats().is_empty());
        assert_eq!(layout.tabs[0].focused.as_deref(), Some("a"));
        // A lone pane cannot stay zoomed.
        assert!(layout.tabs[1].zoom.is_none());
        // A healed layout is stable.
        assert!(!layout.heal(|chat| chat != "chat-a"));
    }

    #[test]
    fn removing_the_active_tab_activates_its_neighbour() {
        let mut layout = two_tabs();
        assert!(layout.remove_tab("t1").is_some());
        assert_eq!(layout.active.as_deref(), Some("t2"));
        let mut layout = two_tabs();
        layout.active = Some("t2".into());
        layout.remove_tab("t2");
        assert_eq!(layout.active.as_deref(), Some("t1"));
        // Removing an inactive tab keeps the active one.
        let mut layout = two_tabs();
        layout.remove_tab("t2");
        assert_eq!(layout.active.as_deref(), Some("t1"));
        assert!(layout.remove_tab("nope").is_none());
    }

    #[test]
    fn a_tabs_primary_chat_is_its_focused_one_else_its_first() {
        let mut layout = two_tabs();
        let tab = &mut layout.tabs[0];
        assert_eq!(tab.primary_chat().as_deref(), Some("chat-a"));
        tab.focused = Some("b".into());
        assert_eq!(tab.primary_chat().as_deref(), Some("chat-a"));
        tab.root.assign("a", None);
        assert!(tab.primary_chat().is_none());
    }

    #[test]
    fn closing_a_tab_empties_the_layout() {
        let mut layout = HzSpaceLayout::default();
        layout.tabs.push(HzViewTab::new("t1", "a"));
        layout.tabs.push(HzViewTab::new("t2", "b"));
        layout.active = Some("t2".into());
        // A lone leaf's close empties the tab — its caller drops it.
        assert!(layout.active_tab_mut().unwrap().close_leaf("b"));
        layout.tabs.remove(1);
        layout.prune();
        assert_eq!(layout.tabs.len(), 1);
        assert_eq!(layout.active.as_deref(), Some("t1"));
        assert!(layout.active_tab_mut().unwrap().close_leaf("a"));
        layout.tabs.remove(0);
        layout.prune();
        assert!(layout.tabs.is_empty());
        assert!(layout.active.is_none());
    }

    #[test]
    fn a_leaf_moves_between_tabs() {
        let mut root = leaf("a");
        root.split_leaf("a", HzDir::Right, "s".into(), "b".into());
        let (rest, taken) = root.take_leaf("b");
        let taken = taken.unwrap();
        let mut other = leaf("c");
        other.append_leaf(taken, "s2".into());
        let (tiles, _) = other.tiles_and_splits();
        assert_eq!(tiles.len(), 2);
        assert!(rest.is_some());
        // The appended leaf gets an even share (half of two).
        assert!((tiles[1].w - 0.5).abs() < EPS);
    }

    #[test]
    fn layout_round_trips_through_json() {
        let mut tab = HzViewTab::new("t", "a");
        tab.root
            .split_leaf("a", HzDir::Down, "s".into(), "b".into());
        tab.root.assign("a", Some("chat-a".into()));
        tab.name = "watch".into();
        let layout = HzSpaceLayout {
            tabs: vec![tab],
            active: Some("t".into()),
        };
        let text = serde_json::to_string(&layout).unwrap();
        let back: HzSpaceLayout = serde_json::from_str(&text).unwrap();
        assert_eq!(back, layout);
    }
}
