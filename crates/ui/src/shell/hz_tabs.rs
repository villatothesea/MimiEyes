//! MimiEyes (VENDORED.md #3): render-level tabs and panes over stock Zeron.
//!
//! Workspace → tabs → panes → chat (the model and its invariants are in
//! `hz_layout.rs`):
//!
//! * A **workspace** is the sidebar's space filter; "Panes" in the sidebar's
//!   "..." menu gives it a layout. Switching workspace switches layouts.
//! * A **view tab** is a named group of panes — a strip in the title bar
//!   swaps which arrangement is on screen. It is not an engine concept: no
//!   workspace/tab/pane ids, no RPC verbs, nothing synced.
//! * A **pane** is a slot in the tab's split tree. A slot either holds a chat
//!   id — drawn as its own transcript + composer — or is a new session: a
//!   draft composer whose first send mints the chat (splitting, "+" and the
//!   first tab all open one, focused). A sidebar pick also lands in a draft.
//!   Right click on a pane — HerdrGui's menu: Split right / Split down / Zoom
//!   / Even out panes (every row and column) / Rename / Archive / Close pane.
//!   Panes move by dragging (HerdrGui's pane drag): a name tag or a tree
//!   session row onto a pane swaps (same tab) or moves in, onto a tab chip
//!   moves into that tab.
//! * The **selected chat** and the **focused pane** follow each other
//!   ([`Shell::hz_sync`], once per frame): selecting a chat brings its pane
//!   on screen, or drops it into the active tab; focusing a pane selects its
//!   chat; switching tab selects that tab's focused chat (none for a tab of
//!   empty panes).
//!
//! What the conversation column shows ([`HzMode`]): the pane view while the
//! workspace has tabs; the stock new-session canvas (strip still up) while a
//! session is being started; the stock single-chat view for a chat of
//! another workspace, or once the last tab is gone — upstream workflows are
//! untouched.
//!
//! Everything persists in `UiSettings::hz_layouts`, keyed by the workspace.
//! The drawing primitives come from HerdrGui's `hz_tabs.rs` (strip, tiles,
//! name tags, glow dividers, folds); all Herdr semantics (hzLayout polling,
//! Mutate calls) are stripped — ops act directly on the local tree and save.

use super::hz_layout::{
    HzClosed, HzDir, HzNode, HzPreset, HzSpaceLayout, HzSplit, HzTile, HzViewTab, drag_ratio,
    gutter_at,
};
use super::side_chats::SideChatTab;
use super::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Room the name tag needs above a pane's first message.
const TAG_GAP: f32 = 46.0;
/// Width kept for a tab's name (about eight letters).
const TAB_NAME_MIN: f32 = 35.0;
/// Closed panes remembered (Settings → Closed panes).
const HZ_CLOSED_MAX: usize = 200;

/// A preset's little diagram (16×13): its panes as outlined cells.
fn preset_glyph(preset: HzPreset, color: gpui::Hsla) -> AnyElement {
    let cell = || div().flex_1().min_w_0().min_h_0().rounded(px(1.5)).border_1().border_color(color);
    let column = |n: usize| {
        let mut col = div().flex_1().flex().flex_col().gap(px(1.5));
        for _ in 0..n {
            col = col.child(cell());
        }
        col
    };
    let frame = div().flex_none().w(px(16.0)).h(px(13.0)).flex().gap(px(1.5));
    match preset {
        HzPreset::Columns2 => frame.child(column(1)).child(column(1)),
        HzPreset::Rows2 => frame.child(column(2)),
        HzPreset::LeftOneRightTwo => frame.child(column(1)).child(column(2)),
        HzPreset::LeftTwoRightOne => frame.child(column(2)).child(column(1)),
        HzPreset::Grid4 => frame.child(column(2)).child(column(2)),
    }
    .into_any_element()
}

/// A tab's name as the strip and tree show it ("Tab 2" when unnamed).
fn hz_tab_name(tab: &HzViewTab, place: usize) -> String {
    let name = tab.name.trim();
    if name.is_empty() {
        format!("Tab {}", place + 1)
    } else {
        name.to_owned()
    }
}
/// Height of the view-tab strip under the title bar.

/// What the conversation column draws for the current workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HzMode {
    /// No layout here, or the selected chat belongs to another workspace:
    /// stock rendering, no strip.
    Off,
    /// A new session is being started: the stock canvas, under the strip.
    Canvas,
    /// The active tab's panes, under the strip.
    Panes,
}

/// One slow breath of a divider the pointer is on.
const BREATH_PERIOD: f32 = 1.6;
/// A glow comes up and dies away over these.
const GLOW_IN: f32 = 0.30;
const GLOW_OUT: f32 = 0.55;

/// A conversation mounted in a slot: its own transcript + composer (the same
/// `SideChatTab` the right pane uses). A slot with no chat yet mounts a
/// *draft*: a new-session composer whose first send mints the pane's chat.
pub(super) struct HzPane {
    /// The chat shown; `None` while the pane is a draft.
    chat: Option<String>,
    pub(super) tab: SideChatTab,
    /// The draft's send listener (it learns the minted chat id).
    _minted: Option<Subscription>,
}

/// A divider being dragged: which split, the live ratio.
struct Drag {
    split: String,
    ratio: f32,
}

/// The glow of one divider: it rises while the pointer is on it and dies away
/// smoothly when it leaves, picking up from wherever it was.
struct Glow {
    level: f32,
    since: std::time::Instant,
    rising: bool,
}

impl Glow {
    fn new() -> Self {
        Self {
            level: 0.0,
            since: std::time::Instant::now(),
            rising: true,
        }
    }

    /// The ramp (0..1) at `now`.
    fn ramp(&self, now: std::time::Instant) -> f32 {
        let dt = now.duration_since(self.since).as_secs_f32();
        if self.rising {
            (self.level + dt / GLOW_IN).min(1.0)
        } else {
            (self.level - dt / GLOW_OUT).max(0.0)
        }
    }

    fn set_rising(&mut self, rising: bool) {
        let now = std::time::Instant::now();
        self.level = self.ramp(now);
        self.since = now;
        self.rising = rising;
    }

    fn done(&self, now: std::time::Instant) -> bool {
        !self.rising && self.ramp(now) <= 0.0
    }
}

/// The breathing wave at `secs` on a clock shared by every divider (so it
/// never restarts or jumps): 0.35..1, one breath per `BREATH_PERIOD`.
fn wave(secs: f32) -> f32 {
    0.35 + 0.65 * (std::f32::consts::PI * secs / BREATH_PERIOD).sin().powi(2)
}

/// The payload of a view-tab chip being dragged (GPUI's own drag, as the
/// right-pane tab strip uses).
struct HzTabDrag {
    id: String,
    from: usize,
    title: SharedString,
}

/// What a drag carries — HerdrGui's pane drag: a pane's name tag, or a
/// session row of the tree sidebar. Onto a pane of the same tab it swaps
/// the two, onto another tab's pane, a tab chip or another session's row it
/// moves into that tab; a session in no pane yet opens where it lands.
#[derive(Debug, Clone)]
pub(super) struct HzPaneDrag {
    /// The slot it is shown in (`None`: a session no pane shows).
    slot: Option<String>,
    /// The tab that slot lives in (dropping it there moves nothing).
    tab: Option<String>,
    /// The session (`None`: a draft pane).
    chat: Option<String>,
    title: SharedString,
}

/// Where the dragged chip would land (the slide offsets animate toward it).
struct TabDragState {
    id: String,
    from: usize,
    over: usize,
    /// The slot it was over before (the slide starts from there) and a counter
    /// that restarts the slide.
    prev_over: usize,
    epoch: u64,
    /// Where the chips were when the drag began (sliding chips must not move
    /// the targets).
    chips: Vec<(f32, f32)>,
}

/// A view tab's inline name editor (double-click a chip, or Rename in its
/// menu). Enter or blur commits, Escape cancels — the same recipe as the
/// sidebar's chat rename.
struct HzRename {
    tab: String,
    /// Opened on the tree's tab row (its editor shows there, not on the chip).
    in_tree: bool,
    input: Entity<ComposerInput>,
    focus_pending: bool,
    _events: Subscription,
    _blur: Option<Subscription>,
}

/// One fold's height tween (`from`..`to` px); `epoch` restarts the animation,
/// `until` ends it.
#[derive(Clone, Copy)]
pub(super) struct FoldMotion {
    from: f32,
    to: f32,
    epoch: u64,
    until: std::time::Instant,
}

/// All of MimiEyes' view-layer state: mounted pane conversations, divider
/// drags and glows, tab drags, menus, the tab rename, and the tree sidebar's
/// folds. The persistent part lives in `UiSettings::hz_layouts`.
pub(super) struct HzUi {
    /// Conversations mounted per slot (keyed by leaf id — they survive splits
    /// and moves because the leaf keeps its id).
    pub(super) panes: HashMap<String, HzPane>,
    drag: Option<Drag>,
    /// Dividers whose glow is up or fading, by key.
    glows: HashMap<String, Glow>,
    glow_ticker: Option<Task<()>>,
    glow_clock: std::time::Instant,
    /// A tab chip being reordered in the strip.
    tab_drag: Option<TabDragState>,
    /// Where each chip is on screen: tab id -> (left, width), measured while
    /// painting.
    tab_bounds: Rc<RefCell<HashMap<String, (f32, f32)>>>,
    /// The right-click menus: (slot | tab id, where it opened).
    pub(super) pane_menu: Option<(String, Point<Pixels>)>,
    pub(super) tab_menu: Option<(String, Point<Pixels>)>,
    /// The "No project" row's menu, where it opened.
    pub(super) loose_menu: Option<Point<Pixels>>,
    /// A project row's "+" menu: (project key, where it opened).
    pub(super) plus_menu: Option<(String, Point<Pixels>)>,
    /// The next session rename opens on the pane's tag (set just before
    /// `open_rename_chat` by the pane's own rename paths).
    pub(super) rename_on_pane: bool,
    /// Chats a pane draft minted that the chat list has not delivered yet
    /// (healing must not take them out of their pane meanwhile).
    minted: std::collections::HashSet<String>,
    rename: Option<HzRename>,
    /// The leaf whose composer holds the keyboard (focus follows it).
    typing_in: Option<String>,
    /// The pane area on screen: x, y, width, height (measured while painting).
    area: Rc<Cell<(f32, f32, f32, f32)>>,
    /// Tree folds opening or closing (the 180 ms height tween), and the
    /// natural height of every fold body as last measured.
    pub(super) folds: HashMap<String, FoldMotion>,
    pub(super) fold_heights: Rc<RefCell<HashMap<String, f32>>>,
    /// The new-session canvas is up (set by the stock new-session paths,
    /// cleared once a chat is selected or a tab/pane is picked).
    composing: bool,
    /// The (project, selected chat) the last [`Shell::hz_sync`] settled — a
    /// difference is a selection to put on screen.
    seen: Option<(String, Option<String>)>,
    /// The project on screen (space id, `""` = no project): the selected
    /// session's, else the last one shown (an all-new-sessions tab selects
    /// nothing, and must not lose its project).
    pub(super) project: String,
    /// The agents' `panes` relay subscription, per engine connection
    /// (`hz_agent.rs`).
    pub(super) relay: Option<(crate::state::EngineHandle, Task<()>)>,
}

impl Default for HzUi {
    fn default() -> Self {
        Self {
            panes: HashMap::new(),
            drag: None,
            glows: HashMap::new(),
            glow_ticker: None,
            glow_clock: std::time::Instant::now(),
            tab_drag: None,
            tab_bounds: Rc::new(RefCell::new(HashMap::new())),
            pane_menu: None,
            tab_menu: None,
            loose_menu: None,
            plus_menu: None,
            rename_on_pane: false,
            minted: std::collections::HashSet::new(),
            rename: None,
            typing_in: None,
            area: Rc::new(Cell::new((0.0, 0.0, 0.0, 0.0))),
            folds: HashMap::new(),
            fold_heights: Rc::new(RefCell::new(HashMap::new())),
            composing: false,
            seen: None,
            project: String::new(),
            relay: None,
        }
    }
}

impl Shell {
    // ---- persistence ----

    /// The layout key: the project on screen (see `HzUi::project`).
    pub(super) fn hz_key(&self) -> String {
        self.hz.project.clone()
    }

    fn hz_layout(&self) -> Option<&HzSpaceLayout> {
        self.settings.hz_layouts.get(&self.hz_key())
    }

    fn hz_layout_mut(&mut self) -> &mut HzSpaceLayout {
        let key = self.hz_key();
        self.settings.hz_layouts.entry(key).or_default()
    }

    /// Push the working copy to `ui-settings.json` (debounced); an emptied
    /// layout leaves no row behind (with panes on, the next frame gives the
    /// project a fresh tab).
    fn hz_save(&mut self, cx: &mut Context<Self>) {
        let key = self.hz_key();
        self.hz_layout_mut().prune();
        if self
            .settings
            .hz_layouts
            .get(&key)
            .is_none_or(|layout| layout.tabs.is_empty())
        {
            self.settings.hz_layouts.remove(&key);
        }
        self.schedule_save(cx);
    }

    /// A fresh node id (persisted ids must not collide with later ones).
    pub(super) fn hz_id(prefix: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        format!("{}{}", prefix, &id[..8])
    }

    /// The tab on screen, cloned for a render pass (mutating accessors need
    /// `&mut self` anyway, and tabs are small).
    fn hz_active(&self) -> Option<HzViewTab> {
        self.hz_layout()
            .and_then(|layout| layout.active_tab().cloned())
    }

    /// The project (layout key) a session belongs to: its space, `""` for a
    /// session in no project. `None` while its row is not known yet.
    pub(super) fn hz_project_of(&self, chat: &str, cx: &App) -> Option<String> {
        self.state
            .read(cx)
            .chats
            .iter()
            .find(|c| c.id == chat)
            .map(|c| c.space_id.clone().unwrap_or_default())
    }

    /// Whether `chat` may sit in the panes on screen: a live, unarchived
    /// session of this project (panes belong to a tab, tabs to a project).
    /// `None` while the chat row is not known (a session minted this frame).
    fn hz_belongs(&self, chat: &str, cx: &App) -> Option<bool> {
        let state = self.state.read(cx);
        let row = state.chats.iter().find(|c| c.id == chat)?;
        Some(!row.archived && row.space_id.clone().unwrap_or_default() == self.hz.project)
    }

    /// Change the selected session (`None` = none) without a nav entry —
    /// the pane view's own moves (tab switch, close) are not history.
    fn hz_select(&mut self, chat: Option<String>, cx: &mut Context<Self>) {
        if self.state.read(cx).selected_chat != chat {
            self.state.update(cx, |s, cx| s.select_chat(chat, cx));
        }
    }

    /// The stock new-session paths call this: the canvas comes up under the
    /// strip, and the session minted there lands in its project's panes.
    pub(super) fn hz_note_new_session(&mut self) {
        self.hz.composing = true;
    }

    /// The stock open path calls this: re-opening the selected session still
    /// brings its pane on screen.
    pub(super) fn hz_note_open_chat(&mut self) {
        if let Some((_, chat)) = self.hz.seen.as_mut() {
            *chat = None;
        }
    }

    /// What the conversation column draws (see [`HzMode`]).
    pub(super) fn hz_mode(&self, cx: &App) -> HzMode {
        if !matches!(self.route, Route::Chat) || !self.settings.hz_panes {
            return HzMode::Off;
        }
        match self.state.read(cx).selected_chat.as_deref() {
            None if self.hz.composing => HzMode::Canvas,
            None => HzMode::Panes,
            // An archived session (opened from Settings) has no pane: the
            // stock view shows it.
            Some(chat) => {
                let placed = self
                    .hz_layout()
                    .is_some_and(|layout| layout.slot_of(chat).is_some());
                if placed || self.hz_belongs(chat, cx) != Some(false) {
                    HzMode::Panes
                } else {
                    HzMode::Off
                }
            }
        }
    }

    /// The once-per-frame pass that keeps project, tabs, panes and the
    /// selected chat consistent (called at the top of `Shell::render`):
    ///
    /// 1. the project on screen follows the selected session (the new-session
    ///    canvas: the project it is aimed at);
    /// 2. a project shown for the first time gets its first tab;
    /// 3. heal its layout (gone/archived/foreign/duplicate chats leave panes);
    /// 4. a newly selected session comes on screen — its pane revealed, or
    ///    the session adopted into the active tab.
    pub(super) fn hz_sync(&mut self, cx: &mut Context<Self>) {
        // Agents' `panes` requests are served whatever is on screen.
        self.hz_relay_tick(cx);
        // Until the first chats frame lands every chat looks gone: healing
        // then would wipe every saved pane.
        if !self.state.read(cx).chats_synced {
            return;
        }
        // Settings from before the switch existed: saved layouts mean on.
        if !self.settings.hz_panes && !self.settings.hz_layouts.is_empty() {
            self.settings.hz_panes = true;
        }
        let selected = self.state.read(cx).selected_chat.clone();
        if selected.is_some() {
            self.hz.composing = false;
        }
        let project = match selected.as_deref() {
            Some(chat) => self.hz_project_of(chat, cx),
            None if self.hz.composing => {
                Some(self.state.read(cx).selected_space.clone().unwrap_or_default())
            }
            None => None,
        };
        if let Some(project) = project {
            self.hz.project = project;
        }
        let key = self.hz_key();
        let prev = self.hz.seen.replace((key.clone(), selected.clone()));
        // A minted chat stays protected only until the chat list has it.
        let known: Vec<String> = {
            let state = self.state.read(cx);
            self.hz
                .minted
                .iter()
                .filter(|id| state.chats.iter().any(|c| &c.id == *id))
                .cloned()
                .collect()
        };
        for id in known {
            self.hz.minted.remove(&id);
        }
        if !self.settings.hz_panes {
            return;
        }
        self.hz_prune_closed(cx);
        if !self.settings.hz_layouts.contains_key(&key) {
            // "No project" removed (or never used) while nothing is selected:
            // show a real project instead of growing it back.
            if key.is_empty()
                && selected.is_none()
                && !self.hz.composing
                && let Some(next) = self.hz_fallback_project(cx)
            {
                self.hz.project = next;
                cx.notify();
                return;
            }
            if !self.hz.composing {
                let seed = selected
                    .clone()
                    .filter(|chat| self.hz_belongs(chat, cx) != Some(false));
                self.hz_new_tab(seed, cx);
            }
            return;
        }
        let Some(mut layout) = self.hz_layout().cloned() else {
            return;
        };
        let keep = |chat: &str| {
            self.hz_belongs(chat, cx).unwrap_or(
                selected.as_deref() == Some(chat) || self.hz.minted.contains(chat),
            )
        };
        let mut changed = layout.heal(keep);
        let fresh = prev
            .as_ref()
            .is_none_or(|(k, c)| *k != key || *c != selected);
        if fresh && let Some(chat) = &selected {
            if layout.slot_of(chat).is_some() {
                let before = layout.clone();
                layout.reveal(chat);
                changed |= layout != before;
            } else if self.hz_belongs(chat, cx) != Some(false) {
                changed |= layout.adopt(chat);
            }
        }
        if changed {
            *self.hz_layout_mut() = layout;
            self.hz_save(cx);
        }
    }

    /// Show project `key` (a tree workspace row): its tabs as they were left,
    /// the active tab's focused session selected — or with panes off, its
    /// latest session in the stock view.
    pub(super) fn hz_open_project(
        &mut self,
        key: String,
        latest: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let own = self
            .settings
            .hz_layouts
            .get(&key)
            .and_then(|layout| layout.active_tab())
            .and_then(HzViewTab::primary_chat);
        if !self.settings.hz_panes || !self.settings.hz_layouts.contains_key(&key) {
            if let Some(chat) = latest {
                self.open_chat(chat, cx);
            }
            return;
        }
        match own {
            Some(chat) => self.open_chat(chat, cx),
            // A project of new-session panes only: nothing to select.
            None => {
                self.hz.project = key;
                self.hz.composing = false;
                self.set_route(Route::Chat, cx);
                self.hz_select(None, cx);
                cx.notify();
            }
        }
    }

    /// A tree workspace row's "+" (Zeron's project "+"): a new session in
    /// that project. With panes on it is a new pane of the project's active
    /// tab, focused, ready to type in; with panes off, the stock canvas.
    pub(super) fn hz_new_session_in_project(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.settings.hz_panes {
            self.open_new_session(Some(key).filter(|k| !k.is_empty()), cx);
            return;
        }
        self.hz.project = key;
        self.hz.composing = false;
        self.set_route(Route::Chat, cx);
        if self.hz_layout().is_none_or(|layout| layout.tabs.is_empty()) {
            self.hz_new_tab(None, cx);
            return;
        }
        // Split the focused pane (a free one is reused instead).
        let Some(tab) = self.hz_active() else {
            return;
        };
        let free = tab
            .root
            .leaf_ids()
            .into_iter()
            .find(|id| tab.root.chat_of(id).is_some_and(Option::is_none))
            .map(str::to_owned);
        match free {
            Some(leaf) => {
                if let Some(tab) = self.hz_layout_mut().active_tab_mut() {
                    tab.focus(&leaf);
                }
                self.hz_select(None, cx);
                self.hz_save(cx);
            }
            None => {
                if let Some(leaf) = tab.focused.clone().or_else(|| tab.root.leaf_ids().first().map(|s| (*s).to_owned())) {
                    self.hz_split_pane(&leaf, HzDir::Right, cx);
                    self.hz_select(None, cx);
                }
            }
        }
        self.hz.typing_in = None;
        cx.notify();
    }

    /// Focus a slot: it becomes the drop target for sidebar picks, its chat
    /// the selected session, its composer the keyboard target.
    fn hz_focus_pane(&mut self, leaf: String, cx: &mut Context<Self>) {
        let chat = self
            .hz_active()
            .and_then(|tab| tab.root.chat_of(&leaf).cloned())
            .flatten();
        let moved = self
            .hz_layout_mut()
            .active_tab_mut()
            .is_some_and(|tab| tab.focus(&leaf));
        // The keyboard follows in the render pass.
        if let Some(chat) = chat {
            self.hz_select(Some(chat), cx);
        }
        if moved {
            self.hz_save(cx);
        }
        cx.notify();
    }

    /// The selection after the pane holding it went away: the active tab's
    /// focused chat, else none (the tab's pickers stay up).
    fn hz_select_active_tab(&mut self, cx: &mut Context<Self>) {
        let chat = self
            .hz_layout()
            .and_then(|layout| layout.active_tab())
            .and_then(HzViewTab::primary_chat);
        self.hz_select(chat, cx);
    }

    // ---- tab ops ----

    /// The chat menu's "Open in panes": panes come on, the session's project
    /// is shown, and the session opens beside what its active tab shows —
    /// already in a pane, that pane comes on screen; else into a free pane,
    /// else a split of the focused one ("Open in panes" never replaces one).
    pub(super) fn hz_open_in_panes(&mut self, chat_id: String, cx: &mut Context<Self>) {
        self.hz.composing = false;
        self.settings.hz_panes = true;
        if let Some(project) = self.hz_project_of(&chat_id, cx) {
            self.hz.project = project;
        }
        if self
            .hz_layout()
            .is_some_and(|layout| layout.slot_of(&chat_id).is_none())
            && let Some(tab) = self.hz_active()
            && tab.root.chats().len() == tab.root.leaf_count()
            && let Some(leaf) = tab.pick_slot()
        {
            self.hz_split_pane(&leaf, HzDir::Right, cx);
        }
        self.open_chat(chat_id, cx);
        self.schedule_save(cx);
    }

    /// The view-tab strip for the titlebar row — replaces the session title
    /// while the pane view (or the new-session canvas under it) runs; the
    /// panes name their sessions on their own tags. No chip is lit on the
    /// canvas: no tab is on screen there.
    pub(super) fn hz_titlebar_strip(
        &mut self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let lit = match self.hz_mode(cx) {
            HzMode::Off => return None,
            HzMode::Canvas => String::new(),
            HzMode::Panes => self.hz_active()?.id,
        };
        Some(self.render_hz_strip(&lit, theme, cx))
    }

    /// The sidebar menu's "Panes" switch (global: every project shows its
    /// own tabs while it is on).
    pub(super) fn hz_pane_view_enabled(&self) -> bool {
        self.settings.hz_panes
    }

    /// On: the project on screen gets its tabs (the next frame makes its
    /// first one around the selected session). Off: every project's layout —
    /// and their mounted conversations — go; the stock view returns.
    pub(super) fn hz_toggle_pane_view(&mut self, cx: &mut Context<Self>) {
        if self.settings.hz_panes {
            self.settings.hz_panes = false;
            self.settings.hz_layouts.clear();
            self.hz.panes.clear();
        } else {
            self.settings.hz_panes = true;
            self.hz.composing = false;
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// A new view tab with one pane, made active: `seed` goes in it, else it
    /// opens as a new session (its composer focused, no session selected) —
    /// `+` on the strip never pulls a session out of another tab.
    fn hz_new_tab(&mut self, seed: Option<String>, cx: &mut Context<Self>) {
        let leaf = Self::hz_id("l");
        let mut tab = HzViewTab::new(Self::hz_id("t"), leaf.clone());
        tab.focused = Some(leaf.clone());
        if let Some(chat) = &seed {
            self.hz_clear_chat(chat);
            tab.root.assign(&leaf, Some(chat.clone()));
        }
        {
            let layout = self.hz_layout_mut();
            layout.active = Some(tab.id.clone());
            layout.tabs.push(tab);
        }
        self.hz.composing = false;
        self.hz_select(seed, cx);
        self.hz_save(cx);
        cx.notify();
    }

    /// Remove `chat` from every slot of this space's tabs.
    fn hz_clear_chat(&mut self, chat: &str) {
        let layout = self.hz_layout_mut();
        for tab in &mut layout.tabs {
            let leaves: Vec<String> = tab.root.leaf_ids().into_iter().map(str::to_owned).collect();
            for leaf in leaves {
                if tab
                    .root
                    .chat_of(&leaf)
                    .is_some_and(|c| c.as_deref() == Some(chat))
                {
                    tab.root.assign(&leaf, None);
                }
            }
        }
    }

    /// Put tab `id` on screen; its focused chat becomes the session (none for
    /// a tab of empty panes). From the new-session canvas this is the way
    /// back to the panes, so the active chip responds there too.
    fn hz_select_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let same = self.hz_layout().and_then(|l| l.active.as_deref()) == Some(id);
        if same && !self.hz.composing {
            return;
        }
        self.hz.composing = false;
        self.hz_layout_mut().active = Some(id.to_owned());
        self.hz_select_active_tab(cx);
        self.hz_save(cx);
        cx.notify();
    }

    /// Close a tab and its panes; the neighbour takes over and its focused
    /// chat the selection. The project's last tab gives way to a fresh one
    /// (a new-session pane) — panes stay on until the "Panes" switch. The
    /// sessions stay in the sidebar; a click there brings one back.
    fn hz_close_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let place = self
            .hz_layout()
            .and_then(|l| l.tabs.iter().position(|t| t.id == id))
            .unwrap_or(0);
        let Some(tab) = self.hz_layout_mut().remove_tab(id) else {
            return;
        };
        let key = self.hz_key();
        self.hz_note_closed(&key, &tab.id, &hz_tab_name(&tab, place), &tab.root.chats());
        for leaf in tab.root.leaf_ids() {
            self.hz.panes.remove(leaf);
        }
        let selected = self.state.read(cx).selected_chat.clone();
        let held_selection = selected
            .as_deref()
            .is_some_and(|chat| tab.root.slot_of(chat).is_some());
        let left = self.hz_layout().is_some_and(|l| !l.tabs.is_empty());
        // With tabs left, the screen shows the new active tab; without, the
        // selection clears so the fresh tab is a new session, not the closed
        // one again.
        if left && (held_selection || selected.is_none()) {
            self.hz_select_active_tab(cx);
        } else if !left {
            self.hz_select(None, cx);
        }
        self.hz_save(cx);
        cx.notify();
    }

    fn hz_close_other_tabs(&mut self, keep: &str, cx: &mut Context<Self>) {
        let dropped: Vec<(String, String, Vec<String>)> = self
            .hz_layout()
            .map(|l| {
                l.tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.id != keep)
                    .map(|(ix, t)| (t.id.clone(), hz_tab_name(t, ix), t.root.chats()))
                    .collect()
            })
            .unwrap_or_default();
        let key = self.hz_key();
        for (tab, name, chats) in dropped {
            self.hz_note_closed(&key, &tab, &name, &chats);
        }
        {
            let layout = self.hz_layout_mut();
            layout.tabs.retain(|t| t.id == keep);
            layout.active = Some(keep.to_owned());
        }
        // The kept tab is what shows now: a selection from a closed tab gives
        // way to its focused chat. Conversations of dropped tabs unmount on
        // the next frame (the render path prunes `self.hz.panes`).
        let selected = self.state.read(cx).selected_chat.clone();
        if selected
            .as_deref()
            .is_none_or(|chat| self.hz_layout().is_none_or(|l| l.slot_of(chat).is_none()))
        {
            self.hz_select_active_tab(cx);
        }
        self.hz_save(cx);
        cx.notify();
    }

    /// Drag-reorder in the strip: put `id` at slot `to`.
    fn hz_move_tab(&mut self, id: &str, to: usize, cx: &mut Context<Self>) {
        let layout = self.hz_layout_mut();
        let Some(from) = layout.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        let to = to.min(layout.tabs.len().saturating_sub(1));
        if from != to {
            let tab = layout.tabs.remove(from);
            layout.tabs.insert(to, tab);
            self.hz_save(cx);
        }
        self.hz.tab_drag = None;
        cx.notify();
    }

    fn hz_drop_tab(&mut self, payload: &HzTabDrag, cx: &mut Context<Self>) {
        let over = self.hz.tab_drag.as_ref().map(|d| d.over);
        self.hz.tab_drag = None;
        if let Some(to) = over {
            self.hz_move_tab(&payload.id, to, cx);
        }
        cx.notify();
    }

    /// Open the tab's inline name editor (double-click a chip or its menu).
    fn hz_rename_tab(
        &mut self,
        tab: &HzViewTab,
        in_tree: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.hz.rename.is_some() {
            self.hz_finish_rename(true, cx);
        }
        let input = cx.new(|cx| {
            ComposerInput::new("Tab name", cx)
                .with_single_line()
                .with_text_metrics(13.0, 17.0)
                .with_accessibility_role(gpui::Role::TextInput)
        });
        input.update(cx, |input, cx| {
            input.set_text(tab.name.trim(), cx);
            input.select_all_text(cx);
        });
        let events = cx.subscribe(&input, |this: &mut Self, _, event, cx| {
            if matches!(event, ComposerInputEvent::Submitted) {
                this.hz_finish_rename(true, cx);
            }
        });
        self.hz.rename = Some(HzRename {
            tab: tab.id.clone(),
            in_tree,
            input,
            focus_pending: true,
            _events: events,
            _blur: None,
        });
        self.hz_focus_rename(window, cx);
        cx.notify();
    }

    /// Focus a just-opened tab rename and arm its commit-on-blur.
    fn hz_focus_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = self.hz.rename.as_mut() else {
            return;
        };
        if !std::mem::take(&mut rename.focus_pending) {
            return;
        }
        let focus = rename.input.focus_handle(cx);
        window.focus(&focus, cx);
        rename._blur = Some(cx.on_blur(&focus, window, |this, _, cx| {
            this.hz_finish_rename(true, cx);
        }));
    }

    /// Close the inline rename, keeping a non-empty name (Enter or blur).
    fn hz_finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.hz.rename.take() else {
            return;
        };
        let name = rename.input.read(cx).text().trim().to_string();
        if commit {
            let layout = self.hz_layout_mut();
            if let Some(tab) = layout.tabs.iter_mut().find(|t| t.id == rename.tab) {
                tab.name = name;
            }
            self.hz_save(cx);
        }
        cx.notify();
    }

    // ---- pane ops ----

    /// Mount the slot's conversation once: `chat`'s own transcript + composer,
    /// or for an empty slot a draft — a new-session composer homed on the
    /// workspace's project. The slot's assigned chat drives (re)mounts; a
    /// draft whose send minted the chat keeps its mount (the send is still
    /// running in its composer).
    fn hz_ensure_pane(&mut self, leaf: &str, chat: Option<&str>, cx: &mut Context<Self>) -> bool {
        // hz: a pane mounting its conversation is timed (cache hits are not).
        let _hz_timer = (!self.hz.panes.get(leaf).is_some_and(|p| p.chat.as_deref() == chat))
            .then(|| crate::hz_perf::Timer::new("pane mount", chat.unwrap_or("draft").to_owned(), 0));
        if let Some(pane) = self.hz.panes.get_mut(leaf) {
            if pane.chat.as_deref() == chat {
                return true;
            }
            if pane.chat.is_none()
                && chat.is_some()
                && pane.tab.state.read(cx).selected_chat.as_deref() == chat
            {
                pane.chat = chat.map(str::to_owned);
                return true;
            }
        }
        let (tab, minted) = match chat {
            Some(chat) => {
                let Some(chat_row) = self
                    .state
                    .read(cx)
                    .chats
                    .iter()
                    .find(|c| c.id == chat)
                    .cloned()
                else {
                    return false;
                };
                (self.make_side_chat_tab(chat_row, false, cx), None)
            }
            None => {
                let parent = self.state.clone();
                let space = Some(self.hz_key()).filter(|key| !key.is_empty());
                let state = cx.new(|cx| AppState::hz_draft_state(&parent, space, cx));
                let tab = self.wrap_side_chat_tab(state, None, cx);
                // The first send names the chat: the slot showing this
                // composer takes it (found by the composer — a swap may have
                // carried the draft to another slot meanwhile).
                let minted = cx.subscribe(&tab.composer, |this, composer, event, cx| {
                    if let ComposerEvent::Sent { chat_id, .. } = event {
                        let id = composer.entity_id();
                        let leaf = this
                            .hz
                            .panes
                            .iter()
                            .find(|(_, pane)| pane.tab.composer.entity_id() == id)
                            .map(|(leaf, _)| leaf.clone());
                        if let Some(leaf) = leaf {
                            this.hz_draft_sent(&leaf, chat_id.clone(), cx);
                        }
                    }
                });
                (tab, Some(minted))
            }
        };
        tab.transcript
            .update(cx, |t, cx| t.set_hz_top_gap(TAG_GAP, cx));
        // The window's picker has the catalogs already: no re-discovery.
        let window_pickers = self.composer.read(cx).pickers().clone();
        let pane_pickers = tab.composer.read(cx).pickers().clone();
        pane_pickers.update(cx, |p, cx| p.hz_seed_from(&window_pickers, cx));
        // A new composer asks for the keyboard at once; only the focused pane
        // may have it (the render pass hands it over). Like the main
        // composer, a pane's shows the account (plan usage) ring.
        tab.composer.update(cx, |c, cx| {
            c.focus_pending = false;
            c.set_hz_plan_usage(cx);
        });
        self.hz.typing_in = None;
        self.hz.panes.insert(
            leaf.to_owned(),
            HzPane {
                chat: chat.map(str::to_owned),
                tab,
                _minted: minted,
            },
        );
        true
    }

    /// A draft pane's first send minted `chat`: the slot holds it from now on
    /// and it becomes the selected session (the pane is focused already).
    fn hz_draft_sent(&mut self, leaf: &str, chat: String, cx: &mut Context<Self>) {
        let Some(pane) = self.hz.panes.get_mut(leaf) else {
            return;
        };
        if pane.chat.is_some() {
            return;
        }
        pane.chat = Some(chat.clone());
        self.hz.minted.insert(chat.clone());
        let keys: Vec<String> = self.settings.hz_layouts.keys().cloned().collect();
        for key in keys {
            if let Some(layout) = self.settings.hz_layouts.get_mut(&key)
                && let Some(tab) = layout
                    .tabs
                    .iter_mut()
                    .find(|t| t.root.leaf_ids().contains(&leaf))
            {
                tab.root.assign(leaf, Some(chat.clone()));
                tab.focus(leaf);
                break;
            }
        }
        self.hz_select(Some(chat), cx);
        self.hz_save(cx);
        cx.notify();
    }

    /// Close one slot: its space collapses onto the neighbour; the last slot
    /// of a tab takes the tab with it, and the project's last tab gives way
    /// to a fresh new-session tab. Closing only hides: the session stays in
    /// the sidebar, and a click there opens it in a pane again.
    fn hz_close_pane(&mut self, leaf: &str, cx: &mut Context<Self>) {
        let from = self.hz_layout().and_then(|layout| {
            layout
                .tabs
                .iter()
                .enumerate()
                .find(|(_, t)| t.root.leaf_ids().contains(&leaf))
                .map(|(ix, t)| (t.id.clone(), hz_tab_name(t, ix)))
        });
        let (closed_chat, tab_gone) = {
            let layout = self.hz_layout_mut();
            let Some(ix) = layout
                .tabs
                .iter()
                .position(|t| t.root.leaf_ids().contains(&leaf))
            else {
                return;
            };
            let chat = layout.tabs[ix].root.chat_of(leaf).and_then(|c| c.clone());
            // A lone leaf's close empties the tab — it goes with it (the
            // stock view returns when that was the last tab).
            let gone = layout.tabs[ix].close_leaf(leaf);
            if gone {
                let id = layout.tabs[ix].id.clone();
                layout.remove_tab(&id);
            }
            (chat, gone)
        };
        self.hz.panes.remove(leaf);
        if let (Some(chat), Some((tab, name))) = (&closed_chat, from) {
            let key = self.hz_key();
            self.hz_note_closed(&key, &tab, &name, std::slice::from_ref(chat));
        }
        // The pane that holds the selection went: the active tab's focused
        // chat takes over (none when its panes are empty). With no tab left
        // nothing stays selected, so the fresh tab is a new session.
        let selected = self.state.read(cx).selected_chat.clone();
        let lost = selected.is_some() && selected == closed_chat;
        let left = self.hz_layout().is_some_and(|l| !l.tabs.is_empty());
        if left && (lost || (tab_gone && selected.is_none())) {
            self.hz_select_active_tab(cx);
        } else if !left && lost {
            self.hz_select(None, cx);
        }
        self.hz_save(cx);
        cx.notify();
    }

    /// Split a slot: the pane keeps its half, a new pane gets the other — a
    /// new session, its composer focused and ready to type in.
    fn hz_split_pane(&mut self, leaf: &str, dir: HzDir, cx: &mut Context<Self>) {
        let ok = {
            let layout = self.hz_layout_mut();
            let Some(tab) = layout.active_tab_mut() else {
                return;
            };
            let new_leaf = Self::hz_id("l");
            let ok = tab
                .root
                .split_leaf(leaf, dir, Self::hz_id("s"), new_leaf.clone());
            if ok {
                tab.focus(&new_leaf);
            }
            ok
        };
        if ok {
            self.hz_save(cx);
            cx.notify();
        }
    }

    /// The pane menu's one stop for every row.
    fn hz_pane_action(&mut self, leaf: String, action: &'static str, cx: &mut Context<Self>) {
        self.hz.pane_menu = None;
        match action {
            "right" => self.hz_split_pane(&leaf, HzDir::Right, cx),
            "down" => self.hz_split_pane(&leaf, HzDir::Down, cx),
            "zoom" => {
                let layout = self.hz_layout_mut();
                if let Some(tab) = layout.active_tab_mut()
                    && tab.root.leaf_count() > 1
                {
                    tab.zoom = if tab.zoom.as_deref() == Some(leaf.as_str()) {
                        None
                    } else {
                        Some(leaf)
                    };
                }
                self.hz_save(cx);
            }
            "even" => {
                let layout = self.hz_layout_mut();
                if let Some(tab) = layout.active_tab_mut() {
                    tab.root.even();
                    tab.zoom = None;
                }
                self.hz_save(cx);
            }
            "rename" => self.hz_rename_pane(&leaf, cx),
            "pin" => {
                if let Some(chat) = self
                    .hz_active()
                    .and_then(|tab| tab.root.chat_of(&leaf).cloned())
                    .flatten()
                {
                    let pinned = self.active_sidebar_pins(cx).contains(&chat);
                    self.set_chat_pinned(chat, !pinned, cx);
                }
            }
            // As in HerdrGui: the session leaves the window with its pane
            // (Settings → Archived brings it back).
            "archive" => {
                if let Some(chat) = self
                    .hz_active()
                    .and_then(|tab| tab.root.chat_of(&leaf).cloned())
                    .flatten()
                {
                    self.hz_close_pane(&leaf, cx);
                    self.set_chat_archived(chat, true, cx);
                }
            }
            "close" => self.hz_close_pane(&leaf, cx),
            _ => {}
        }
        cx.notify();
    }

    /// Move a slot into another view tab (a drop on its chip, one of its
    /// panes or a session row of it): the leaf keeps its id, so its
    /// conversation stays mounted, and it joins the tab as a split.
    fn hz_move_pane(&mut self, leaf: String, to_tab: String, cx: &mut Context<Self>) {
        self.hz.pane_menu = None;
        let selected = self.state.read(cx).selected_chat.clone();
        {
            let layout = self.hz_layout_mut();
            let Some(src) = layout
                .tabs
                .iter()
                .position(|t| t.root.leaf_ids().contains(&leaf.as_str()))
            else {
                return;
            };
            // Check the destination before anything is taken out: a stale
            // target must not lose the pane.
            if layout.tabs[src].id == to_tab || !layout.tabs.iter().any(|t| t.id == to_tab) {
                return;
            }
            let (rest, taken) = layout.tabs[src].root.clone().take_leaf(&leaf);
            let Some(taken) = taken else {
                return;
            };
            let follows = selected
                .as_deref()
                .is_some_and(|chat| taken.slot_of(chat).is_some());
            let src_id = layout.tabs[src].id.clone();
            let src_gone = rest.is_none();
            match rest {
                Some(root) => {
                    let tab = &mut layout.tabs[src];
                    tab.root = root;
                    if tab.focused.as_deref() == Some(leaf.as_str()) {
                        tab.focused = tab.root.leaf_ids().first().map(|s| (*s).to_owned());
                    }
                    if tab.zoom.as_deref() == Some(leaf.as_str()) {
                        tab.zoom = None;
                    }
                }
                // The tab was that one leaf: it goes entirely.
                None => {
                    layout.remove_tab(&src_id);
                }
            }
            if let Some(tab) = layout.tabs.iter_mut().find(|t| t.id == to_tab) {
                tab.root.append_leaf(taken, Self::hz_id("s"));
                tab.focus(&leaf);
            }
            // The selected session's pane is followed to its new tab; an
            // emptied source tab hands the screen over too.
            if follows || src_gone {
                layout.active = Some(to_tab);
            }
            layout.prune();
        }
        self.hz_save(cx);
        cx.notify();
    }

    /// A pane tag dropped on another pane: the two slots trade chats; focus
    /// (and a zoom) follow the dragged session to its new place.
    fn hz_swap_panes(&mut self, a: String, b: String, cx: &mut Context<Self>) {
        if a == b {
            return;
        }
        {
            let layout = self.hz_layout_mut();
            if let Some(tab) = layout.tabs.iter_mut().find(|t| {
                let ids = t.root.leaf_ids();
                ids.contains(&a.as_str()) && ids.contains(&b.as_str())
            }) && tab.root.swap_chats(&a, &b)
            {
                tab.focused = Some(b.clone());
                if tab.zoom.as_deref() == Some(a.as_str()) {
                    tab.zoom = Some(b.clone());
                }
            }
        }
        // The mounted conversations follow their chats, so a swap never
        // rebuilds one (a mount landing under an emptied leaf stays inert
        // until that slot takes a chat again — then `hz_ensure_pane` remounts).
        let pa = self.hz.panes.remove(&a);
        let pb = self.hz.panes.remove(&b);
        if let Some(pb) = pb {
            self.hz.panes.insert(a.clone(), pb);
        }
        if let Some(pa) = pa {
            self.hz.panes.insert(b.clone(), pa);
        }
        self.hz_save(cx);
        cx.notify();
    }

    /// A drag dropped on pane `leaf` of the active tab: a pane of this tab
    /// swaps with it, another tab's pane moves in beside it, a session no
    /// pane shows opens in it (what it showed stays in the sidebar).
    fn hz_drop_on_pane(&mut self, drag: &HzPaneDrag, leaf: &str, cx: &mut Context<Self>) {
        let active = self.hz_active().map(|t| t.id);
        match (&drag.slot, &drag.tab) {
            (Some(slot), _) if slot == leaf => {}
            (Some(slot), tab) if *tab == active => {
                self.hz_swap_panes(slot.clone(), leaf.to_owned(), cx)
            }
            (Some(slot), _) => {
                if let Some(active) = active {
                    self.hz_move_pane(slot.clone(), active, cx);
                }
            }
            (None, _) => {
                let Some(chat) = drag.chat.clone() else {
                    return;
                };
                // Panes belong to a tab and tabs to a project.
                if self.hz_belongs(&chat, cx) == Some(false) {
                    return;
                }
                self.hz_clear_chat(&chat);
                if let Some(tab) = self.hz_layout_mut().active_tab_mut() {
                    tab.root.assign(leaf, Some(chat.clone()));
                    tab.focus(leaf);
                }
                self.hz_select(Some(chat), cx);
                self.hz_save(cx);
                cx.notify();
            }
        }
    }

    /// A drag dropped on view tab `tab` (its chip, or a session row of it):
    /// a pane moves in as a split, a session no pane shows opens as one.
    fn hz_drop_on_tab(&mut self, drag: &HzPaneDrag, tab: &str, cx: &mut Context<Self>) {
        if drag.tab.as_deref() == Some(tab) {
            return;
        }
        if let Some(slot) = &drag.slot {
            self.hz_move_pane(slot.clone(), tab.to_owned(), cx);
            return;
        }
        let Some(chat) = drag.chat.clone() else {
            return;
        };
        // Panes belong to a tab and tabs to a project.
        if self.hz_belongs(&chat, cx) == Some(false) {
            return;
        }
        self.hz_clear_chat(&chat);
        let leaf = Self::hz_id("l");
        let split = Self::hz_id("s");
        {
            let layout = self.hz_layout_mut();
            let Some(t) = layout.tabs.iter_mut().find(|t| t.id == tab) else {
                return;
            };
            t.root.append_leaf(
                HzNode::Leaf {
                    id: leaf.clone(),
                    chat: Some(chat.clone()),
                },
                split,
            );
            t.focus(&leaf);
            layout.active = Some(tab.to_owned());
        }
        self.hz_select(Some(chat), cx);
        self.hz_save(cx);
        cx.notify();
    }

    /// A drag dropped on the tree row of session `onto`: with both in one
    /// tab they swap, else the drag moves into `onto`'s tab. A row no pane
    /// shows takes nothing (HerdrGui's workspace move has no MimiEyes
    /// counterpart: a session's project is fixed).
    fn hz_drop_on_row(&mut self, drag: &HzPaneDrag, onto: &str, cx: &mut Context<Self>) {
        if drag.chat.as_deref() == Some(onto) {
            return;
        }
        let Some((tab, slot)) = self.hz_layout().and_then(|l| l.slot_of(onto)) else {
            return;
        };
        match (&drag.slot, &drag.tab) {
            (Some(own), Some(own_tab)) if *own_tab == tab => {
                self.hz_swap_panes(own.clone(), slot, cx)
            }
            _ => self.hz_drop_on_tab(drag, &tab, cx),
        }
    }

    /// The 1-based strip place of the tab whose pane shows `chat` (the tree
    /// sidebar marks on-screen sessions with it).
    /// chat id → "Project › Tab" for every session in a view tab now
    /// (Settings → Usage groups by it).
    pub(super) fn hz_usage_tabs(&self, cx: &App) -> HashMap<String, String> {
        let state = self.state.read(cx);
        let mut out = HashMap::new();
        for (key, layout) in &self.settings.hz_layouts {
            let project = state
                .space_row(key)
                .map(|s| s.display_name().to_owned())
                .unwrap_or_else(|| "No project".to_owned());
            for (ix, tab) in layout.tabs.iter().enumerate() {
                let name = tab.name.trim();
                let name = if name.is_empty() {
                    format!("Tab {}", ix + 1)
                } else {
                    name.to_owned()
                };
                for chat in tab.root.chats() {
                    out.insert(chat, format!("{project} › {name}"));
                }
            }
        }
        out
    }

    // ---- the tree's tab and pane rows ----

    /// Bring project `key`'s tab `tab` on screen, its focused session selected
    /// (none for a tab of new-session panes).
    pub(super) fn hz_tree_open_tab(&mut self, key: &str, tab: &str, cx: &mut Context<Self>) {
        self.hz.project = key.to_owned();
        self.hz.composing = false;
        if let Some(layout) = self.settings.hz_layouts.get_mut(key) {
            layout.active = Some(tab.to_owned());
        }
        self.set_route(Route::Chat, cx);
        self.hz_select_active_tab(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Focus pane `leaf` of that tab (and select its session).
    pub(super) fn hz_tree_focus_pane(&mut self, key: &str, tab: &str, leaf: &str, cx: &mut Context<Self>) {
        self.hz.project = key.to_owned();
        self.hz.composing = false;
        let mut chat = None;
        if let Some(layout) = self.settings.hz_layouts.get_mut(key) {
            layout.active = Some(tab.to_owned());
            if let Some(t) = layout.tabs.iter_mut().find(|t| t.id == tab) {
                t.focus(leaf);
                chat = t.root.chat_of(leaf).cloned().flatten();
            }
        }
        self.set_route(Route::Chat, cx);
        self.hz_select(chat, cx);
        self.hz.typing_in = None;
        self.schedule_save(cx);
        cx.notify();
    }

    /// Double-click on a tab row: rename that tab, right there.
    pub(super) fn hz_tree_rename_tab(&mut self, key: &str, tab: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.hz_tree_open_tab(key, tab, cx);
        if let Some(t) = self
            .settings
            .hz_layouts
            .get(key)
            .and_then(|l| l.tabs.iter().find(|t| t.id == tab))
            .cloned()
        {
            self.hz_rename_tab(&t, true, window, cx);
        }
    }

    /// The tab row's rename field, while one is open on it.
    pub(super) fn hz_tree_rename_input(&self, tab: &str) -> Option<Entity<ComposerInput>> {
        self.hz
            .rename
            .as_ref()
            .filter(|r| r.in_tree && r.tab == tab)
            .map(|r| r.input.clone())
    }

    /// Right-click on a tab row: the tab menu, for that tab.
    pub(super) fn hz_tree_tab_menu(&mut self, key: &str, tab: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.hz_tree_open_tab(key, tab, cx);
        self.hz.tab_menu = Some((tab.to_owned(), at));
        cx.notify();
    }

    /// Right-click on a pane row: the pane menu, for that pane.
    pub(super) fn hz_tree_pane_menu(&mut self, key: &str, tab: &str, leaf: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.hz_tree_focus_pane(key, tab, leaf, cx);
        self.hz.pane_menu = Some((leaf.to_owned(), at));
        cx.notify();
    }

    /// A tab row's "+": a new session in a new pane of that tab.
    pub(super) fn hz_tree_new_pane(&mut self, key: &str, tab: &str, cx: &mut Context<Self>) {
        self.hz_tree_open_tab(key, tab, cx);
        self.hz_new_session_in_project(key.to_owned(), cx);
    }

    /// The "+" menu's New layout: a new tab in project `key` whose panes
    /// are new sessions arranged as `preset`, the first one focused.
    pub(super) fn hz_new_layout_in_project(&mut self, key: String, preset: HzPreset, cx: &mut Context<Self>) {
        self.settings.hz_panes = true;
        self.hz.project = key;
        self.hz.composing = false;
        self.set_route(Route::Chat, cx);
        let root = preset.tree(&mut |prefix| Self::hz_id(prefix));
        let first = root.leaf_ids().first().map(|l| (*l).to_owned());
        let tab = HzViewTab {
            id: Self::hz_id("t"),
            name: String::new(),
            root,
            focused: first.clone(),
            zoom: None,
        };
        {
            let layout = self.hz_layout_mut();
            layout.active = Some(tab.id.clone());
            layout.tabs.push(tab);
        }
        self.hz_select(None, cx);
        self.hz.typing_in = first;
        self.hz_save(cx);
        cx.notify();
    }

    /// A status's colour (the dots', and the tree's tab numbers').
    pub(super) fn hz_status_color(indicator: zeron_proto::ChatIndicator, theme: &Theme) -> gpui::Hsla {
        use zeron_proto::ChatIndicator as I;
        match indicator {
            I::Idle | I::Working => theme.accent,
            I::AwaitingInput | I::Errored => theme.danger,
            I::Completed => theme.success,
        }
    }

    /// A project row's "+" menu: a new session, or a new layout of several.
    pub(super) fn render_hz_plus_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (key, position) = self.hz.plus_menu.clone()?;
        let theme = Theme::of(cx).for_popup();
        let new_key = key.clone();
        let mut menu = popover::popover_card(&theme)
            .w(px(232.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.hz.plus_menu = None;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .child(
                popover::menu_row(&theme, false, "hz-plus-menu-session")
                    .id("hz-plus-menu-session")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.hz.plus_menu = None;
                        this.hz_new_session_in_project(new_key.clone(), cx);
                    }))
                    .child(icon(icons::PLUS).size(px(15.0)).text_color(theme.text_muted))
                    .child(SharedString::from("New session")),
            )
            .child(popover::menu_separator())
            .child(
                div()
                    .px(px(10.0))
                    .pt(px(4.0))
                    .pb(px(2.0))
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .child(SharedString::from("New layout")),
            );
        for (ix, preset) in HzPreset::ALL.into_iter().enumerate() {
            let key = key.clone();
            menu = menu.child(
                popover::menu_row(&theme, false, format!("hz-plus-menu-layout-{ix}"))
                    .id(("hz-plus-menu-layout", ix))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.hz.plus_menu = None;
                        this.hz_new_layout_in_project(key.clone(), preset, cx);
                    }))
                    .child(preset_glyph(preset, theme.text_muted))
                    .child(div().flex_1().child(SharedString::from(preset.label())))
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(preset.panes().to_string())),
                    ),
            );
        }
        Some(popover::menu_at(
            "hz-plus-menu",
            position,
            menu.into_any_element(),
            None,
        ))
    }

    /// Remember closed panes (newest first) for Settings → Closed panes.
    fn hz_note_closed(&mut self, project: &str, tab: &str, tab_name: &str, chats: &[String]) {
        let at = Utc::now().timestamp_millis();
        for chat in chats {
            self.settings.hz_closed.retain(|c| &c.chat != chat);
            self.settings.hz_closed.insert(
                0,
                HzClosed {
                    chat: chat.clone(),
                    project: project.to_owned(),
                    tab: tab.to_owned(),
                    tab_name: tab_name.to_owned(),
                    at,
                },
            );
        }
        self.settings.hz_closed.truncate(HZ_CLOSED_MAX);
    }

    /// A closed pane whose session is in a pane again, archived or deleted
    /// leaves the list.
    fn hz_prune_closed(&mut self, cx: &mut Context<Self>) {
        if self.settings.hz_closed.is_empty() {
            return;
        }
        let before = self.settings.hz_closed.len();
        let placed: std::collections::HashSet<String> = self
            .settings
            .hz_layouts
            .values()
            .flat_map(|l| l.tabs.iter().flat_map(|t| t.root.chats()))
            .collect();
        let state = self.state.read(cx);
        self.settings.hz_closed.retain(|c| {
            !placed.contains(&c.chat)
                && state
                    .chats
                    .iter()
                    .any(|row| row.id == c.chat && !row.archived)
        });
        if self.settings.hz_closed.len() != before {
            self.schedule_save(cx);
        }
    }

    /// Every session no pane shows, for Settings → Closed panes: closed
    /// panes first (newest first, with the tab they left), then sessions
    /// that were never in a pane (an agent's, or from before panes).
    pub(super) fn hz_closed_items(&self, cx: &App) -> Vec<crate::settings::hz_closed::ClosedItem> {
        use crate::settings::hz_closed::ClosedItem;
        let state = self.state.read(cx);
        let placed: std::collections::HashSet<String> = self
            .settings
            .hz_layouts
            .values()
            .flat_map(|l| l.tabs.iter().flat_map(|t| t.root.chats()))
            .collect();
        let project_name = |key: &str| {
            state
                .space_row(key)
                .map(|s| s.display_name().to_owned())
                .unwrap_or_else(|| "No project".to_owned())
        };
        let item = |chat: &zeron_proto::Chat, place: String, closed_at: Option<i64>| ClosedItem {
            chat_id: chat.id.clone(),
            title: chat
                .title
                .as_deref()
                .map(transcript::single_line)
                .unwrap_or_else(|| "Untitled session".to_owned()),
            place,
            closed_at,
            active_at: chat.last_message_at.unwrap_or(chat.created_at).timestamp_millis(),
            harness: chat.config.as_ref().map(|c| c.harness),
        };
        let mut out = Vec::new();
        let mut listed = std::collections::HashSet::new();
        for closed in &self.settings.hz_closed {
            let Some(chat) = state
                .chats
                .iter()
                .find(|c| c.id == closed.chat && !c.archived)
            else {
                continue;
            };
            if placed.contains(&chat.id) || !listed.insert(chat.id.clone()) {
                continue;
            }
            let place = format!("{} › {}", project_name(&closed.project), closed.tab_name);
            out.push(item(chat, place, Some(closed.at)));
        }
        let mut never: Vec<&zeron_proto::Chat> = state
            .chats
            .iter()
            .filter(|c| {
                !c.archived
                    && c.parent_chat_id.is_none()
                    && !placed.contains(&c.id)
                    && !listed.contains(&c.id)
            })
            .collect();
        never.sort_by_key(|c| std::cmp::Reverse(c.last_message_at.unwrap_or(c.created_at)));
        for chat in never {
            let project = project_name(chat.space_id.as_deref().unwrap_or(""));
            out.push(item(chat, project, None));
        }
        out
    }

    /// Settings → Closed panes' Restore: the session goes back into the
    /// tab it left (made active), else into its project's active tab, else
    /// a new tab named as the old one — and comes on screen.
    pub(super) fn hz_restore_closed(&mut self, chat: String, cx: &mut Context<Self>) {
        let Some(key) = self.hz_project_of(&chat, cx) else {
            return;
        };
        let record = self
            .settings
            .hz_closed
            .iter()
            .position(|c| c.chat == chat)
            .map(|ix| self.settings.hz_closed.remove(ix));
        self.settings.hz_panes = true;
        self.hz.project = key.clone();
        self.hz.composing = false;
        let layout = self.settings.hz_layouts.entry(key).or_default();
        if layout.slot_of(&chat).is_none() {
            let back = record
                .as_ref()
                .filter(|r| layout.tabs.iter().any(|t| t.id == r.tab))
                .map(|r| r.tab.clone());
            if let Some(tab) = back {
                layout.active = Some(tab);
            }
            let adopted = !layout.tabs.is_empty() && layout.adopt(&chat);
            if !adopted {
                self.hz_new_tab(Some(chat.clone()), cx);
                if let Some(name) = record.map(|r| r.tab_name).filter(|n| !n.starts_with("Tab "))
                    && let Some(tab) = self.hz_layout_mut().active_tab_mut()
                {
                    tab.name = name;
                }
            }
        }
        self.set_route(Route::Chat, cx);
        self.hz_select(Some(chat), cx);
        self.hz_save(cx);
        cx.notify();
    }

    /// Rename the session in pane `leaf` on its own tag (a double click on
    /// the tag, or the pane menu's Rename…). A new session has no title yet.
    fn hz_rename_pane(&mut self, leaf: &str, cx: &mut Context<Self>) {
        let Some(chat) = self
            .hz_active()
            .and_then(|tab| tab.root.chat_of(leaf).cloned())
            .flatten()
        else {
            return;
        };
        self.hz.rename_on_pane = true;
        self.open_rename_chat(chat, cx);
    }

    /// Where the screen goes instead of "No project": the selected project,
    /// else one with tabs, else the first project. `None` without projects.
    fn hz_fallback_project(&self, cx: &App) -> Option<String> {
        let state = self.state.read(cx);
        let exists = |id: &str| state.spaces.iter().any(|s| s.id == id);
        state
            .selected_space
            .clone()
            .filter(|id| !id.is_empty() && exists(id))
            .or_else(|| {
                self.settings
                    .hz_layouts
                    .keys()
                    .find(|k| !k.is_empty() && exists(k))
                    .cloned()
            })
            .or_else(|| state.spaces.first().map(|s| s.id.clone()))
    }

    /// "Remove" on the No project row: its tabs close (sessions in no
    /// project stay in the tree, and keep the row) and, if it is on screen,
    /// another project shows.
    fn hz_remove_loose(&mut self, cx: &mut Context<Self>) {
        if let Some(layout) = self.settings.hz_layouts.remove("") {
            for (ix, tab) in layout.tabs.iter().enumerate() {
                self.hz_note_closed("", &tab.id, &hz_tab_name(tab, ix), &tab.root.chats());
                for leaf in tab.root.leaf_ids() {
                    self.hz.panes.remove(leaf);
                }
            }
        }
        if self.hz.project.is_empty() {
            let selected = self.state.read(cx).selected_chat.clone();
            if selected
                .as_deref()
                .is_some_and(|chat| self.hz_project_of(chat, cx).as_deref() == Some(""))
            {
                self.hz_select(None, cx);
            }
            if let Some(next) = self.hz_fallback_project(cx) {
                if self.settings.hz_layouts.contains_key(&next) {
                    self.hz_open_project(next, None, cx);
                } else {
                    self.hz.project = next;
                    self.hz.composing = false;
                    self.set_route(Route::Chat, cx);
                }
            }
        }
        self.schedule_save(cx);
        cx.notify();
    }

    /// The No project row's menu: a new session there, or remove the row.
    pub(super) fn render_hz_loose_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let position = self.hz.loose_menu?;
        let theme = Theme::of(cx).for_popup();
        let removable =
            self.settings.hz_layouts.contains_key("") && self.hz_fallback_project(cx).is_some();
        let has_sessions = self
            .state
            .read(cx)
            .chats
            .iter()
            .any(|c| c.space_id.is_none() && !c.archived);
        let menu = popover::popover_card(&theme)
            .w(px(180.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.hz.loose_menu = None;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .child(
                popover::menu_row(&theme, false, "hz-loose-menu-new")
                    .id("hz-loose-menu-new")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.hz.loose_menu = None;
                        this.hz_new_session_in_project(String::new(), cx);
                    }))
                    .child(SharedString::from("New session")),
            )
            .when(removable, |el| {
                el.child(popover::menu_separator()).child(
                    popover::menu_row(&theme, false, "hz-loose-menu-remove")
                        .id("hz-loose-menu-remove")
                        .text_color(theme.danger)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.hz.loose_menu = None;
                            this.hz_remove_loose(cx);
                        }))
                        .child(SharedString::from(if has_sessions {
                            "Close its tabs"
                        } else {
                            "Remove from sidebar"
                        })),
                )
            });
        Some(popover::menu_at(
            "hz-loose-menu",
            position,
            menu.into_any_element(),
            None,
        ))
    }

    /// What a tab's panes are up to, the most urgent first: needs you >
    /// error > working > done (unseen). `None` while every pane is idle — the
    /// tab chip and tree row then show no mark.
    pub(super) fn hz_tab_status(
        &self,
        tab: &HzViewTab,
        cx: &App,
    ) -> Option<zeron_proto::ChatIndicator> {
        use zeron_proto::ChatIndicator as I;
        let state = self.state.read(cx);
        let now = Utc::now();
        let rank = |i: I| match i {
            I::AwaitingInput => 4,
            I::Errored => 3,
            I::Working => 2,
            I::Completed => 1,
            I::Idle => 0,
        };
        tab.root
            .chats()
            .iter()
            .filter_map(|id| state.chats.iter().find(|c| &c.id == id))
            .map(|c| state.display_status_for(c, now))
            .filter(|i| rank(*i) > 0)
            .max_by_key(|i| rank(*i))
    }

    // ---- divider drags & glows ----

    /// The pointer entered or left a divider: its glow rises or dies away.
    fn hz_set_hover(&mut self, key: String, hovered: bool, cx: &mut Context<Self>) {
        let glow = self.hz.glows.entry(key).or_insert_with(Glow::new);
        if glow.rising != hovered {
            glow.set_rising(hovered);
        }
        self.hz_ensure_glow_ticker(cx);
        cx.notify();
    }

    /// Redraw about 60 times a second while any glow is up or fading.
    fn hz_ensure_glow_ticker(&mut self, cx: &mut Context<Self>) {
        if self.hz.glow_ticker.is_some() {
            return;
        }
        self.hz.glow_ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    let now = std::time::Instant::now();
                    this.hz.glows.retain(|_, g| !g.done(now));
                    let alive = !this.hz.glows.is_empty();
                    if alive {
                        cx.notify();
                    } else {
                        this.hz.glow_ticker = None;
                    }
                    alive
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// How strongly the divider `key` glows now (0 = not at all).
    fn glow_of(&self, key: &str) -> f32 {
        let now = std::time::Instant::now();
        let Some(glow) = self.hz.glows.get(key) else {
            return 0.0;
        };
        let secs = now.duration_since(self.hz.glow_clock).as_secs_f32();
        glow.ramp(now) * wave(secs)
    }

    fn hz_drag_move(
        &mut self,
        position: Point<Pixels>,
        pressed: Option<MouseButton>,
        cx: &mut Context<Self>,
    ) {
        if self.hz.drag.is_none() {
            return;
        }
        if pressed != Some(MouseButton::Left) {
            self.hz_drag_end(cx);
            return;
        }
        let (ox, oy, aw, ah) = self.hz.area.get();
        let Some(split) = self
            .hz
            .drag
            .as_ref()
            .and_then(|d| self.hz_splits().into_iter().find(|s| s.id == d.split))
        else {
            return;
        };
        let ratio = drag_ratio(
            &split,
            f32::from(position.x) - ox,
            f32::from(position.y) - oy,
            aw,
            ah,
        );
        let needs = self
            .hz
            .drag
            .as_mut()
            .is_some_and(|drag| (drag.ratio - ratio).abs() > 0.002);
        if needs {
            if let Some(drag) = self.hz.drag.as_mut() {
                drag.ratio = ratio;
            }
            // Local model: the tree takes the ratio at once, saved on release.
            let layout = self.hz_layout_mut();
            if let Some(tab) = layout.active_tab_mut() {
                tab.root.set_ratio(&split.id, ratio);
            }
            cx.notify();
        }
    }

    /// The splits of the active tab (cloned for a drag lookup).
    fn hz_splits(&self) -> Vec<HzSplit> {
        self.hz_active()
            .map(|tab| tab.root.tiles_and_splits().1)
            .unwrap_or_default()
    }

    /// Every row and column of the shown tab shares its space evenly again.
    fn hz_even_tab(&mut self, cx: &mut Context<Self>) {
        let layout = self.hz_layout_mut();
        if let Some(tab) = layout.active_tab_mut() {
            tab.root.even();
            tab.zoom = None;
        }
        self.hz_save(cx);
        cx.notify();
    }

    fn hz_drag_end(&mut self, cx: &mut Context<Self>) {
        if self.hz.drag.take().is_none() {
            return;
        }
        self.hz_save(cx);
        cx.notify();
    }

    // ---- shared bits the tree sidebar also draws ----

    /// The state mark of a row: the glyph spinner while the agent works, else
    /// an accent dot (idle), red (needs you / error), grey (no run). Square
    /// marks in the tree, round in the picker.
    pub(super) fn hz_mark(
        &self,
        key: String,
        indicator: zeron_proto::ChatIndicator,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use zeron_proto::ChatIndicator as I;
        let slot = div()
            .flex_none()
            .w(px(12.0))
            .flex()
            .items_center()
            .justify_center();
        if indicator == I::Working {
            return slot
                .child(crate::loaders::mini_glyph_spinner(
                    key,
                    3.0,
                    theme.glyph,
                    self.sidebar_pane.entity_id(),
                    cx,
                ))
                .into_any_element();
        }
        let color = match indicator {
            I::Idle => theme.accent,
            I::AwaitingInput | I::Errored => theme.danger,
            I::Completed => theme.success,
            _ => theme.text_faint.opacity(0.6),
        };
        slot.child(div().size(px(8.0)).bg(color).rounded(px(2.0)))
            .into_any_element()
    }

    /// Folded-state helpers shared by the tree sidebar's foldable rows;
    /// the folded set persists in `UiSettings::hz_tree_collapsed`.
    pub(super) fn hz_collapsed(&self, key: &str) -> bool {
        self.settings.hz_tree_collapsed.iter().any(|k| k == key)
    }

    pub(super) fn hz_toggle_fold(&mut self, key: &str, cx: &mut Context<Self>) {
        let was_open = !self.hz_collapsed(key);
        let natural = self
            .hz
            .fold_heights
            .borrow()
            .get(key)
            .copied()
            .unwrap_or(0.0);
        if was_open {
            self.settings.hz_tree_collapsed.push(key.to_owned());
        } else {
            self.settings.hz_tree_collapsed.retain(|k| k != key);
        }
        let (from, to) = if was_open {
            (natural, 0.0)
        } else {
            (0.0, natural)
        };
        let epoch = self.hz.folds.get(key).map_or(0, |m| m.epoch) + 1;
        let until = std::time::Instant::now() + motion::COLLAPSE.total();
        self.hz.folds.insert(
            key.to_owned(),
            FoldMotion {
                from,
                to,
                epoch,
                until,
            },
        );
        self.schedule_save(cx);
        cx.notify();
    }

    pub(super) fn hz_live_fold(&self, key: &str) -> Option<FoldMotion> {
        if self.reduced_motion {
            return None;
        }
        self.hz
            .folds
            .get(key)
            .copied()
            .filter(|m| std::time::Instant::now() < m.until)
    }

    /// The rows under a fold: always laid out (so their height is known),
    /// clipped to the tweened height.
    pub(super) fn hz_fold_body(&self, key: &str, open: bool, rows: Vec<AnyElement>) -> AnyElement {
        let heights = self.hz.fold_heights.clone();
        let measured_key = key.to_owned();
        let inner = div()
            .relative()
            .flex()
            .flex_col()
            .child(
                gpui::canvas(
                    move |b, _, _| {
                        heights
                            .borrow_mut()
                            .insert(measured_key.clone(), f32::from(b.size.height));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .children(rows);
        let frame = div().w_full().flex_none().overflow_hidden().child(inner);
        match self.hz_live_fold(key) {
            Some(m) => {
                let denominator = m.from.max(m.to).max(1.0);
                frame
                    .with_animation(
                        SharedString::from(format!("hz-fold-{key}-{}", m.epoch)),
                        motion::COLLAPSE.animation(),
                        move |el, t| {
                            let h = motion::lerp(m.from, m.to, t);
                            let reveal = (h / denominator).clamp(0.0, 1.0);
                            el.h(px(h))
                                .opacity(0.35 + 0.65 * reveal)
                                .relative()
                                .top(px(-3.0 * (1.0 - reveal)))
                        },
                    )
                    .into_any_element()
            }
            None if open => frame.into_any_element(),
            None => frame.h(px(0.0)).into_any_element(),
        }
    }

    /// The fold arrow: one glyph that turns a quarter (down when open) as the
    /// body moves.
    pub(super) fn hz_chevron(&self, key: &str, open: bool, theme: &Theme) -> AnyElement {
        let glyph = icon(icons::ALT_ARROW_RIGHT)
            .size(px(12.0))
            .text_color(theme.text_muted);
        match self.hz_live_fold(key) {
            Some(m) => {
                let denominator = m.from.max(m.to).max(1.0);
                let (from, to) = (
                    (m.from / denominator).clamp(0.0, 1.0),
                    (m.to / denominator).clamp(0.0, 1.0),
                );
                glyph
                    .with_animation(
                        SharedString::from(format!("hz-chevron-{key}-{}", m.epoch)),
                        motion::COLLAPSE.animation(),
                        move |el, t| {
                            el.with_transformation(gpui::Transformation::rotate(gpui::percentage(
                                motion::lerp(from, to, t) * 0.25,
                            )))
                        },
                    )
                    .into_any_element()
            }
            None => glyph
                .with_transformation(gpui::Transformation::rotate(gpui::percentage(if open {
                    0.25
                } else {
                    0.0
                })))
                .into_any_element(),
        }
    }

    // ---- rendering ----

    /// The hz view takes the conversation column in [`HzMode::Panes`] (the
    /// new-session canvas stays stock and mints into the focused pane on
    /// send). Called from `render_main`; `None` means stock rendering.
    pub(super) fn render_hz_view(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.hz_mode(cx) != HzMode::Panes {
            return None;
        }
        let tab = self.hz_active()?;
        // Slots that went away take their mounted conversation down; slots
        // of inactive tabs and other workspaces stay mounted (their drafts
        // keep the editor across tab and workspace switches).
        let in_layout: std::collections::HashSet<String> = self
            .settings
            .hz_layouts
            .values()
            .flat_map(|l| l.tabs.iter())
            .flat_map(|t| t.root.leaf_ids().into_iter().map(str::to_owned))
            .collect();
        self.hz.panes.retain(|leaf, _| in_layout.contains(leaf));
        let theme = Theme::of(cx).clone();

        let (tiles, splits) = match &tab.zoom {
            Some(leaf) => (
                vec![HzTile {
                    slot: leaf.clone(),
                    chat: tab.root.chat_of(leaf).cloned().flatten(),
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                }],
                Vec::new(),
            ),
            None => tab.root.tiles_and_splits(),
        };
        let (_, _, area_w, _) = self.hz.area.get();
        let focused_slot = tab.focused.clone();
        let mut panes = Vec::new();
        for tile in &tiles {
            let is_focused = focused_slot.as_deref() == Some(tile.slot.as_str());
            panes.extend(self.render_hz_pane(tile, is_focused, area_w, &theme, cx));
        }
        let dragging = self.hz.drag.as_ref().map(|d| d.split.clone());
        let gutters: Vec<AnyElement> = splits
            .iter()
            .map(|s| {
                let at = if dragging.as_deref() == Some(s.id.as_str()) {
                    if s.horizontal {
                        s.x + self.hz.drag.as_ref().map_or(s.ratio, |d| d.ratio) * s.w
                    } else {
                        s.y + self.hz.drag.as_ref().map_or(s.ratio, |d| d.ratio) * s.h
                    }
                } else {
                    gutter_at(&tiles, s)
                };
                let active = dragging.as_deref() == Some(s.id.as_str());
                let (id, ratio) = (s.id.clone(), s.ratio);
                let hover_key = format!("gutter-{}", s.id);
                let glow = self.glow_of(&hover_key);
                let base = div()
                    .id(SharedString::from(format!("hz-gutter-{}", s.id)))
                    .absolute()
                    .on_hover(cx.listener({
                        let key = hover_key.clone();
                        move |this, hovered: &bool, _, cx| {
                            this.hz_set_hover(key.clone(), *hovered, cx)
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.hz.drag = Some(Drag {
                                split: id.clone(),
                                ratio,
                            });
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    // A double click on a divider shares the tab's space
                    // evenly again.
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| {
                            if event.click_count == 2 {
                                this.hz.drag = None;
                                this.hz_even_tab(cx);
                            }
                        }),
                    );
                // Hidden until the pointer is over it; then it breathes for as
                // long as it stays.
                let line = |el: gpui::Div| -> AnyElement {
                    if active {
                        el.bg(theme.accent).into_any_element()
                    } else {
                        el.bg(theme.accent).opacity(glow).into_any_element()
                    }
                };
                if s.horizontal {
                    base.left(gpui::relative(at))
                        .ml(px(-4.0))
                        .top(gpui::relative(s.y))
                        .h(gpui::relative(s.h))
                        .w(px(9.0))
                        .cursor_col_resize()
                        .child(
                            div()
                                .absolute()
                                .left(px(4.0))
                                .w(px(1.0))
                                .h_full()
                                .bg(theme.border),
                        )
                        .child(line(div().absolute().left(px(4.0)).w(px(2.0)).h_full()))
                        .into_any_element()
                } else {
                    base.top(gpui::relative(at))
                        .mt(px(-4.0))
                        .left(gpui::relative(s.x))
                        .w(gpui::relative(s.w))
                        .h(px(9.0))
                        .cursor_row_resize()
                        .child(
                            div()
                                .absolute()
                                .top(px(4.0))
                                .h(px(1.0))
                                .w_full()
                                .bg(theme.border),
                        )
                        .child(line(div().absolute().top(px(4.0)).h(px(2.0)).w_full()))
                        .into_any_element()
                }
            })
            .collect();

        // The keyboard goes to the focused pane's composer when focus moves
        // (or its mount changed) — a fresh pane's draft included, so a split
        // is ready to type in.
        if self.hz.typing_in != focused_slot
            && let Some(leaf) = &focused_slot
            && let Some(pane) = self.hz.panes.get(leaf)
        {
            for other in self.hz.panes.values() {
                other.tab.composer.update(cx, |c, _| c.focus_pending = false);
            }
            pane.tab.composer.update(cx, |c, cx| {
                c.focus_pending = true;
                cx.notify();
            });
            self.hz.typing_in = focused_slot.clone();
        }

        // The titlebar's session-details toggle: its card floats over the
        // panes for the focused pane's session (the stock column that would
        // draw it is not on screen).
        let has_selection = self.state.read(cx).selected_chat.is_some();
        let (docked, reveal, _) = self.session_info_frame(has_selection, cx);
        let card_max = self.viewport_height - Theme::TITLEBAR_HEIGHT - 24.0;
        let session_card = self.render_session_info(docked, reveal, card_max, cx);
        let measured = self.hz.area.clone();
        Some(
            div()
                .id("hz-view")
                // The stock conversation column's focus scope, which this view
                // replaces: a click in a reply leaves the keyboard here (not
                // nowhere), so Ctrl+C copies the selected text.
                .track_focus(&self.navigation_focus.main)
                .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                    this.capture_navigation_focus(false, false, window, cx);
                }))
                .on_key_down(|event, _, cx| super::navigation_focus::copy_transcript_selection(event, cx))
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .pt(px(Theme::TITLEBAR_HEIGHT))
                .flex()
                .flex_col()
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                            this.hz_drag_move(event.position, event.pressed_button, cx)
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.hz_drag_end(cx)),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| this.hz_drag_end(cx)),
                        )
                        .child(
                            gpui::canvas(
                                move |bounds, window, _| {
                                    let next = (
                                        f32::from(bounds.origin.x),
                                        f32::from(bounds.origin.y),
                                        f32::from(bounds.size.width),
                                        f32::from(bounds.size.height),
                                    );
                                    if (measured.get().2 - next.2).abs() > 0.5 {
                                        window.request_animation_frame();
                                    }
                                    measured.set(next);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .inset_0(),
                        )
                        .children(panes)
                        .children(gutters),
                )
                // Painted after the panes, so it floats above them.
                .children(session_card)
                .into_any_element(),
        )
    }

    /// The view-tab strip under the title bar: one chip per render group —
    /// its place number, its name when it has one, a ×N badge for panes, a
    /// close — then the "+". Chips click, right-click (menu), double-click
    /// (rename) and reorder by drag; a pane tag dropped on a chip moves the
    /// slot into that tab.
    fn render_hz_strip(
        &mut self,
        active_id: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Heal drag state if the pointer was released outside the strip.
        if self.hz.tab_drag.is_some() && !cx.has_active_drag() {
            self.hz.tab_drag = None;
        }
        let tabs: Vec<HzViewTab> = self.hz_layout().map(|l| l.tabs.clone()).unwrap_or_default();
        let mut chips = Vec::new();
        for (position, t) in tabs.iter().enumerate() {
            let selected = t.id == active_id;
            let id = SharedString::from(format!("hz-tab-{}", t.id));
            let editor = self
                .hz
                .rename
                .as_ref()
                .filter(|r| r.tab == t.id && !r.in_tree)
                .map(|r| r.input.clone());
            let (for_click, for_menu, for_close) = (t.clone(), t.clone(), t.clone());
            let drag_id = t.id.clone();
            let ghost_title = SharedString::from(t.chip_text(position));
            let panes = t.root.leaf_count();
            // Its panes' state, so a tab in the background still tells.
            let status_mark = self
                .hz_tab_status(t, cx)
                .map(|status| self.hz_mark(format!("hz-tab-mark-{}", t.id), status, theme, cx));
            // `occlude`, not `block_mouse_except_scroll`: the strip lives in
            // the titlebar's drag region, and Windows' caption hit test
            // (`WM_NCHITTEST`) sees every hitbox under an except-scroll one —
            // a press on a chip then moved the window and the chip drag only
            // landed on release. The strip never scrolls, so nothing is lost.
            let menu_on = self.hz.tab_menu.as_ref().is_some_and(|(id, _)| *id == t.id);
            let mut chip = crate::surface_chrome::tab(id.clone(), selected, theme)
                .pl(px(10.0))
                .pr(px(4.0))
                .occlude()
                .relative()
                // The chip under its open right-click menu lights up.
                .when(menu_on, |el| el.bg(theme.accent.opacity(0.18)))
                .child({
                    let bounds = self.hz.tab_bounds.clone();
                    let key = t.id.clone();
                    gpui::canvas(
                        move |b, _, _| {
                            bounds.borrow_mut().insert(
                                key.clone(),
                                (f32::from(b.origin.x), f32::from(b.size.width)),
                            );
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0()
                })
                .on_drag(
                    HzTabDrag {
                        id: drag_id.clone(),
                        from: position,
                        title: ghost_title,
                    },
                    |payload, _point, _, cx| {
                        let title = payload.title.clone();
                        cx.stop_propagation();
                        cx.new(|_| SurfaceTabGhost { title })
                    },
                )
                .on_drop::<HzTabDrag>(cx.listener(move |this, payload: &HzTabDrag, _, cx| {
                    this.hz_drop_tab(payload, cx);
                }))
                .on_click(
                    cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            this.hz_rename_tab(&for_click, false, window, cx);
                        } else {
                            this.hz_select_tab(&for_click.id, cx);
                        }
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.hz_select_tab(&for_menu.id, cx);
                        this.hz.tab_menu = Some((for_menu.id.clone(), event.position));
                        cx.stop_propagation();
                        cx.notify();
                    }),
                );
            // A dragged pane tag dropped on the chip moves the pane into that tab.
            chip = Self::hz_pane_drop_target(chip, t.id.clone(), theme, cx);
            chip = match editor {
                Some(input) => chip.w(px(140.0)).child(chat_title_editor(
                    SharedString::from(format!("hz-tab-editor-{}", t.id)),
                    input,
                    theme,
                )),
                None => {
                    let (number, label) = {
                        let name = t.name.trim();
                        let shown = (position + 1).to_string();
                        if name.is_empty() || name == shown {
                            (shown, String::new())
                        } else {
                            (shown, name.to_owned())
                        }
                    };
                    chip.children(status_mark)
                        .child(SharedString::from(number))
                        .child(
                            div()
                                .min_w(px(TAB_NAME_MIN))
                                .truncate()
                                .child(SharedString::from(label)),
                        )
                        .when(panes > 1, |el| {
                            // A colour bar with xN inside: N panes.
                            el.child(
                                div()
                                    .flex_none()
                                    .h(px(15.0))
                                    .px(px(5.0))
                                    .rounded(px(4.0))
                                    .flex()
                                    .items_center()
                                    .bg(theme.accent.opacity(0.28))
                                    .text_color(theme.text)
                                    .text_size(px(10.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(SharedString::from(format!("x{panes}"))),
                            )
                        })
                        .child(
                            div()
                                .id(SharedString::from(format!("hz-tab-x-{}", t.id)))
                                .flex_none()
                                .size(px(20.0))
                                .rounded(px(6.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(theme.text_faint)
                                .hover(|st| st.bg(crate::theme::wash(0.12)).text_color(theme.text))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.hz_close_tab(&for_close.id, cx);
                                }))
                                .child(
                                    icon(icons::CLOSE)
                                        .size(px(12.0))
                                        .text_color(theme.text_muted),
                                ),
                        )
                }
            };
            // While a sibling is dragged over, the chips between slide one
            // slot (150 ms) toward the gap; the dragged chip leaves an empty
            // spacer, the ghost carries it.
            let slot = self.hz.tab_drag.as_ref().map(|d| {
                let dragged = self.hz.tab_bounds.borrow().get(&d.id).map_or(80.0, |b| b.1);
                (d.from, d.over, d.prev_over, d.epoch, dragged + 2.0)
            });
            let wrapped: AnyElement = match slot {
                Some((from, over, prev_over, epoch, slot_w)) if position != from => {
                    let target =
                        crate::terminal::panel::slide_offset(position, from, over) * slot_w;
                    let start =
                        crate::terminal::panel::slide_offset(position, from, prev_over) * slot_w;
                    div()
                        .relative()
                        .child(chip.with_animation(
                            SharedString::from(format!("hz-tab-slide-{position}-{epoch}")),
                            TAB_SLIDE.animation(),
                            move |el, t| el.left(px(motion::lerp(start, target, t))),
                        ))
                        .into_any_element()
                }
                Some((from, ..)) if position == from => {
                    let w = self.hz.tab_bounds.borrow().get(&t.id).map_or(80.0, |b| b.1);
                    // The dragged chip's spacer is no caption either.
                    div()
                        .w(px(w))
                        .h(px(24.0))
                        .flex_none()
                        .occlude()
                        .into_any_element()
                }
                _ => chip.into_any_element(),
            };
            chips.push(wrapped);
        }
        let bounds = self.hz.tab_bounds.clone();
        let order: Vec<String> = tabs.iter().map(|t| t.id.clone()).collect();
        div()
            .id("hz-tab-strip")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(2.0))
            .overflow_hidden()
            .on_drag_move::<HzTabDrag>(cx.listener(
                move |this, event: &gpui::DragMoveEvent<HzTabDrag>, _, cx| {
                    let from = event.drag(cx).from;
                    let x = f32::from(event.event.position.x);
                    let chips: Vec<(f32, f32)> = match this.hz.tab_drag.as_ref() {
                        Some(d) => d.chips.clone(),
                        None => order
                            .iter()
                            .filter_map(|id| bounds.borrow().get(id).copied())
                            .collect(),
                    };
                    let over = super::hz_layout::tab_slot(x, &chips);
                    let id = event.drag(cx).id.clone();
                    match this.hz.tab_drag.as_mut() {
                        Some(d) if d.over != over => {
                            d.prev_over = d.over;
                            d.over = over;
                            d.epoch += 1;
                            cx.notify();
                        }
                        Some(_) => {}
                        None => {
                            this.hz.tab_drag = Some(TabDragState {
                                id,
                                from,
                                over,
                                prev_over: from,
                                epoch: 0,
                                chips,
                            });
                            cx.notify();
                        }
                    }
                },
            ))
            .on_drop::<HzTabDrag>(
                cx.listener(|this, payload: &HzTabDrag, _, cx| this.hz_drop_tab(payload, cx)),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(chips)
            .child(
                crate::surface_chrome::tab("hz-tab-new", false, theme)
                    .px(px(8.0))
                    .occlude()
                    .on_click(cx.listener(move |this, _, _, cx| this.hz_new_tab(None, cx)))
                    .child(
                        icon(icons::PLUS)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    ),
            )
            .into_any_element()
    }

    fn render_hz_pane(
        &mut self,
        tile: &HzTile,
        focused: bool,
        area_w: f32,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let leaf = tile.slot.clone();
        if !self.hz_ensure_pane(&leaf, tile.chat.as_deref(), cx) {
            return None;
        }
        let pane = self.hz.panes.get(&leaf)?;
        // The pane's own chat: the slot's, or the one its draft just minted.
        let chat = pane.chat.clone();
        let (transcript, composer) = (pane.tab.transcript.clone(), pane.tab.composer.clone());
        let width = composer_target_width(area_w * tile.w, settings::transcript_width(cx), true);
        // A draft is a new session: its composer takes the new-session (hero)
        // frame — the tall two-row pill whose input sits above the actions.
        // The conversation frame (one compact row) would clip that input to
        // nothing: nothing typed would show.
        let docked = chat.is_some();
        composer.update(cx, |composer, cx| {
            composer.set_dock_frame(crate::composer_dock::DockFrame::settled(docked), cx);
            composer.set_available_width(width, cx);
        });
        let pill = transcript.read(cx).jump_button_shown().then(|| {
            div()
                .absolute()
                .bottom(px(12.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(self.jump_pill("hz-pane-jump", "hz-pane-jump-pill", transcript.clone(), cx))
        });

        // A draft, or a chat the list has not delivered yet, is a "New
        // session" until its row (and title) arrives.
        let (title, indicator) = {
            let state = self.state.read(cx);
            let row = chat
                .as_deref()
                .and_then(|id| state.chats.iter().find(|c| c.id == id));
            let title = row
                .and_then(|c| c.title.clone())
                .map(|t| transcript::single_line(&t))
                .unwrap_or_else(|| "New session".to_owned());
            let indicator = row.map_or(zeron_proto::ChatIndicator::Idle, |c| {
                state.display_status_for(c, Utc::now())
            });
            (title, indicator)
        };
        // The tag under its open right-click menu lights up too.
        let menu_on = self.hz.pane_menu.as_ref().is_some_and(|(l, _)| *l == leaf);
        let (tag_bg, tag_fg) = if focused {
            (theme.accent, theme.on_accent)
        } else if menu_on {
            (theme.accent.opacity(0.35), theme.text)
        } else {
            (crate::theme::wash(0.10), theme.text_muted)
        };
        let running = indicator == zeron_proto::ChatIndicator::Working;
        let (menu_leaf, menu_leaf2, focus_leaf, drag_leaf) =
            (leaf.clone(), leaf.clone(), leaf.clone(), leaf.clone());
        // The tag's own rename field, while one is open on it.
        let editor = chat
            .as_deref()
            .and_then(|id| self.rename_input_for(id, ChatRenameSurface::HzPane));
        let rename_leaf = leaf.clone();
        // The agent's status is a colour bar on the tag's left edge, apart from
        // the tag's own colour (which says "focused").
        // The CLI's logo; a draft shows the CLI its composer will start
        // (HerdrGui's tag).
        let (chip_icon, chip_tint) = {
            let own = chat.as_deref().and_then(|id| {
                self.state
                    .read(cx)
                    .chats
                    .iter()
                    .find(|c| c.id == id)
                    .and_then(|c| c.config.as_ref().map(|cfg| cfg.harness))
            });
            let harness = own
                .or_else(|| composer.read(cx).pickers().read(cx).resolved(cx).harness)
                .unwrap_or(zeron_proto::HarnessId::Mock);
            crate::pickers::harness_brand_icon(harness)
        };
        let tag = div()
            .id(SharedString::from(format!("hz-tag-{leaf}")))
            .debug_selector({
                let leaf = leaf.clone();
                move || format!("hz-tag-{leaf}")
            })
            .flex_none()
            .min_w_0()
            .h(px(24.0))
            .flex()
            .flex_row()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.hz.pane_menu = Some((menu_leaf.clone(), event.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            // A double click renames the session right here, on the tag.
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.hz_rename_pane(&rename_leaf, cx);
                }
            }))
            .child({
                let body = div()
                    .min_w_0()
                    .h_full()
                    .px(px(10.0))
                    .flex()
                    .items_center()
                    .rounded_br(px(10.0))
                    .bg(tag_bg)
                    .text_color(tag_fg)
                    .text_size(px(12.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    // A running agent's logo breathes (2.4 s), in its own
                    // cached view: the window is not re-rendered for it.
                    .child(if running {
                        div()
                            .flex_none()
                            .mr(px(5.0))
                            .child(crate::hz_breath::breathing(
                                format!("hz-tag-breath-{leaf}"),
                                crate::hz_breath::Breath::Icon {
                                    path: chip_icon,
                                    tint: chip_tint.unwrap_or(tag_fg),
                                    size: 14.0,
                                },
                                14.0,
                                14.0,
                            ))
                            .into_any_element()
                    } else {
                        icon(chip_icon)
                            .size(px(14.0))
                            .flex_none()
                            .mr(px(5.0))
                            .text_color(chip_tint.unwrap_or(tag_fg))
                            .into_any_element()
                    })
                    .child(match editor {
                        // The field keeps its clicks: the pane under it would
                        // move focus to the composer and end the rename.
                        Some(input) => div()
                            .w(px(200.0))
                            .flex()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(chat_title_editor(
                                SharedString::from(format!("hz-tag-editor-{leaf}")),
                                input,
                                theme,
                            ))
                            .into_any_element(),
                        None => div()
                            .min_w_0()
                            .truncate()
                            .child(SharedString::from(title))
                            .into_any_element(),
                    });
                body.into_any_element()
            });
        // The tag drags the slot: onto a sibling pane it swaps, onto a tab
        // chip it moves.
        let tag = self
            .hz_pane_tag_drag(tag, &drag_leaf, cx)
            .into_any_element();
        // The session's short id right of the tag (the first 8 characters —
        // also a handle the Zeron MCP tools accept); a click copies the full
        // id. A new session has none until its first send.
        let id_chip = chat.clone().map(|chat_id| {
            let short: String = chat_id.chars().take(8).collect();
            div()
                .id(SharedString::from(format!("hz-tag-id-{leaf}")))
                .flex_none()
                .h(px(24.0))
                .px(px(4.0))
                .flex()
                .items_center()
                .rounded(px(4.0))
                .cursor_pointer()
                .font_family(theme.font_mono.clone())
                .text_size(px(10.5))
                .text_color(theme.text_faint)
                .hover(|st| st.text_color(theme.text_muted).bg(crate::theme::wash(0.06)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(chat_id.clone()));
                    this.show_notice(crate::toast::ToastKind::Info, "Session id copied", cx);
                }))
                .child(SharedString::from(short))
        });
        let swap_over = leaf.clone();
        let swap_drop = leaf.clone();
        let swap_accent = theme.accent;
        Some(
            div()
                .id(SharedString::from(format!("hz-pane-{leaf}")))
                .absolute()
                .left(gpui::relative(tile.x))
                .top(gpui::relative(tile.y))
                .w(gpui::relative(tile.w))
                .h(gpui::relative(tile.h))
                .overflow_hidden()
                // Another pane's tag dragged here changes places with this one.
                .drag_over::<HzPaneDrag>(move |style, drag, _, _| {
                    if drag.slot.as_deref() != Some(swap_over.as_str()) {
                        style.bg(swap_accent.opacity(0.14))
                    } else {
                        style
                    }
                })
                .on_drop::<HzPaneDrag>(cx.listener(move |this, drag: &HzPaneDrag, _, cx| {
                    this.hz_drop_on_pane(drag, &swap_drop, cx);
                }))
                // Files from the file tree (or the desktop) dropped on a pane
                // attach to its composer, as in the stock conversation.
                .drag_over::<WorkspacePathDrag>({
                    let accent = theme.accent;
                    move |style, _, _, _| style.bg(accent.opacity(0.10))
                })
                .drag_over::<gpui::ExternalPaths>({
                    let accent = theme.accent;
                    move |style, _, _, _| style.bg(accent.opacity(0.10))
                })
                .on_drop::<WorkspacePathDrag>({
                    let composer = composer.clone();
                    cx.listener(move |_, payload: &WorkspacePathDrag, window, cx| {
                        composer.update(cx, |composer, cx| {
                            composer.add_workspace_path(
                                &payload.path,
                                payload.is_directory,
                                window,
                                cx,
                            )
                        });
                        cx.notify();
                    })
                })
                .on_drop::<gpui::ExternalPaths>({
                    let composer = composer.clone();
                    cx.listener(move |_, paths: &gpui::ExternalPaths, _, cx| {
                        composer.update(cx, |composer, cx| {
                            composer.add_paths(paths.paths().to_vec(), cx)
                        });
                        cx.notify();
                    })
                })
                // A click anywhere in the pane selects it, even where a child
                // handles it.
                .capture_any_mouse_down(
                    cx.listener(move |this, _, _, cx| this.hz_focus_pane(focus_leaf.clone(), cx)),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        this.hz.pane_menu = Some((menu_leaf2.clone(), event.position));
                        cx.notify();
                    }),
                )
                .child({
                    let body = div().size_full().relative().flex().flex_col();
                    let body = match &chat {
                        // A draft: nothing said yet — the composer waits in
                        // the middle, as on the new-session canvas (its own
                        // project/machine pickers head it; no extra line).
                        None => body
                            .justify_center()
                            .child(div().flex_none().child(composer)),
                        Some(_) => body
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .relative()
                                    // Text scrolled up under the name tag is gone
                                    // by the tag's lower edge and ramps in just
                                    // below it (as Zeron does under its title bar).
                                    .child(
                                        crate::edge_fade::edge_faded(
                                            Theme::TRANSCRIPT_FADE_BAND,
                                            true,
                                            true,
                                            div().size_full().child(transcript),
                                        )
                                        .inset_top(24.0)
                                        .band_top(18.0),
                                    )
                                    .children(pill),
                            )
                            .child(div().flex_none().child(composer)),
                    };
                    body.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .max_w(gpui::relative(0.9))
                                .flex()
                                .flex_row()
                                .items_start()
                                .gap(px(4.0))
                                .child(tag)
                                .children(id_chip),
                    )
                })
                .into_any_element(),
        )
    }

    /// The pane menu, over the pane that was right-clicked — HerdrGui's rows
    /// that apply here: Split right / Split down / Zoom / Even out panes /
    /// Rename… / Archive / Close pane. Rows that cannot apply to this pane
    /// (a new session has nothing to rename or archive yet) are dimmed.
    pub(super) fn render_hz_pane_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (leaf, position) = self.hz.pane_menu.clone()?;
        let theme = Theme::of(cx).for_popup();
        let tab = self.hz_active()?;
        let chat = tab.root.chat_of(&leaf).cloned().flatten();
        let draft = chat.is_none();
        let pinned = chat
            .as_ref()
            .is_some_and(|chat| self.active_sidebar_pins(cx).contains(chat));
        let zoomed = tab.zoom.as_deref() == Some(leaf.as_str());
        let can_zoom = tab.root.leaf_count() > 1;
        let glyph_color = theme.text_muted;
        let danger_color = theme.danger;
        let row = |id: &'static str,
                   label: &'static str,
                   action: &'static str,
                   danger: bool,
                   cx: &mut Context<Self>| {
            let leaf = leaf.clone();
            let tint = if danger { danger_color } else { glyph_color };
            let named = |path: &'static str| {
                icon(path)
                    .size(px(15.0))
                    .text_color(tint)
                    .into_any_element()
            };
            let glyph: AnyElement = match action {
                "right" => crate::icons::sidebar_glyph(1.0, true, 15.0, tint).into_any_element(),
                "down" => crate::icons::panel_bottom_glyph(15.0, tint).into_any_element(),
                "zoom" => named(icons::ZOOM_ARROWS),
                "even" => named(icons::EVEN_PANES),
                "rename" => named(icons::PEN),
                "pin" => named(icons::PIN),
                "archive" => named(icons::ARCHIVE_MINIMALISTIC),
                _ => named(icons::CLOSE),
            };
            let enabled = match action {
                "zoom" | "even" => can_zoom,
                "rename" | "archive" | "pin" => !draft,
                _ => true,
            };
            popover::menu_row(&theme, false, id)
                .id(id)
                .when(danger, |el| el.text_color(theme.danger))
                .when(!enabled, |el| el.opacity(0.4).cursor_default())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if enabled {
                        this.hz_pane_action(leaf.clone(), action, cx)
                    }
                }))
                .child(glyph)
                .child(SharedString::from(label))
        };
        let menu = popover::popover_card(&theme)
            .w(px(210.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.hz.pane_menu = None;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .child(row("hz-menu-split-right", "Split right", "right", false, cx))
            .child(row("hz-menu-split-down", "Split down", "down", false, cx))
            .child(row(
                "hz-menu-zoom",
                if zoomed { "Unzoom" } else { "Zoom" },
                "zoom",
                false,
                cx,
            ))
            .child(row("hz-menu-even", "Even out panes", "even", false, cx))
            .child(row("hz-menu-rename", "Rename…", "rename", false, cx))
            .child(row(
                "hz-menu-pin",
                if pinned { "Unpin" } else { "Pin" },
                "pin",
                false,
                cx,
            ))
            .child(row("hz-menu-archive", "Archive", "archive", false, cx))
            .child(popover::menu_separator())
            .child(row("hz-menu-close", "Close pane", "close", true, cx));
        Some(popover::menu_at(
            "hz-pane-menu",
            position,
            menu.into_any_element(),
            None,
        ))
    }

    /// The tab menu: rename, close others, or close.
    pub(super) fn render_hz_tab_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (tab_id, position) = self.hz.tab_menu.clone()?;
        let theme = Theme::of(cx).for_popup();
        let tab = self
            .hz_layout()
            .and_then(|l| l.tabs.iter().find(|t| t.id == tab_id).cloned())?;
        let others = self.hz_layout().is_some_and(|l| l.tabs.len() > 1);
        let menu = popover::popover_card(&theme)
            .w(px(180.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.hz.tab_menu = None;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .child(
                popover::menu_row(&theme, false, "hz-tab-menu-rename")
                    .id("hz-tab-menu-rename")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.hz.tab_menu = None;
                        this.hz_rename_tab(&tab, false, window, cx);
                    }))
                    .child(SharedString::from("Rename tab…")),
            )
            .when(others, |el| {
                let keep = tab_id.clone();
                el.child(
                    popover::menu_row(&theme, false, "hz-tab-menu-close-others")
                        .id("hz-tab-menu-close-others")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.hz.tab_menu = None;
                            this.hz_close_other_tabs(&keep, cx);
                        }))
                        .child(SharedString::from("Close other tabs")),
                )
            })
            .child(popover::menu_separator())
            .child(
                popover::menu_row(&theme, false, "hz-tab-menu-close")
                    .id("hz-tab-menu-close")
                    .text_color(theme.danger)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.hz.tab_menu = None;
                        this.hz_close_tab(&tab_id, cx);
                    }))
                    .child(SharedString::from("Close tab")),
            );
        Some(popover::menu_at(
            "hz-tab-menu",
            position,
            menu.into_any_element(),
            None,
        ))
    }

    /// The drag payload of a slot's name tag.
    fn hz_pane_drag(&self, leaf: &str, cx: &App) -> HzPaneDrag {
        let tab = self
            .hz_layout()
            .and_then(|l| l.tabs.iter().find(|t| t.root.leaf_ids().contains(&leaf)));
        let chat = tab.and_then(|t| t.root.chat_of(leaf).cloned().flatten());
        HzPaneDrag {
            slot: Some(leaf.to_owned()),
            tab: tab.map(|t| t.id.clone()),
            title: SharedString::from(self.hz_drag_title(chat.as_deref(), cx)),
            chat,
        }
    }

    /// The drag payload of a tree session row: its pane, when one shows it.
    fn hz_row_drag(&self, chat: &str, cx: &App) -> HzPaneDrag {
        let placed = self.hz_layout().and_then(|l| l.slot_of(chat));
        HzPaneDrag {
            tab: placed.as_ref().map(|(tab, _)| tab.clone()),
            slot: placed.map(|(_, slot)| slot),
            title: SharedString::from(self.hz_drag_title(Some(chat), cx)),
            chat: Some(chat.to_owned()),
        }
    }

    fn hz_drag_title(&self, chat: Option<&str>, cx: &App) -> String {
        chat.and_then(|chat| {
            self.state
                .read(cx)
                .chats
                .iter()
                .find(|c| c.id == chat)
                .and_then(|c| c.title.clone())
                .map(|t| transcript::single_line(&t))
        })
        .unwrap_or_else(|| "New session".to_owned())
    }

    /// Make a pane's name tag draggable: GPUI's own drag with the tab ghost.
    fn hz_pane_tag_drag(
        &self,
        tag: gpui::Stateful<gpui::Div>,
        leaf: &str,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        tag.on_drag(self.hz_pane_drag(leaf, cx), |payload, _point, _, cx| {
            let title = payload.title.clone();
            cx.stop_propagation();
            cx.new(|_| SurfaceTabGhost { title })
        })
    }

    /// Make a tree session row a drag source and a drop target (panes on
    /// only): it drags like a pane's tag, and a drag let go on it swaps or
    /// moves (see [`Self::hz_drop_on_row`]).
    pub(super) fn hz_tree_row_drag(
        &self,
        row: gpui::Stateful<gpui::Div>,
        chat: &str,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        if !self.hz_pane_view_enabled() {
            return row;
        }
        let (over, drop) = (chat.to_owned(), chat.to_owned());
        let accent = theme.accent;
        row.on_drag(self.hz_row_drag(chat, cx), |payload, _point, _, cx| {
            let title = payload.title.clone();
            cx.stop_propagation();
            cx.new(|_| SurfaceTabGhost { title })
        })
        .drag_over::<HzPaneDrag>(move |style, drag, _, _| {
            if drag.chat.as_deref() != Some(over.as_str()) {
                style.bg(accent.opacity(0.22))
            } else {
                style
            }
        })
        .on_drop::<HzPaneDrag>(cx.listener(move |this, drag: &HzPaneDrag, _, cx| {
            this.hz_drop_on_row(drag, &drop, cx);
        }))
    }

    /// Make `el` (a tab chip) take a drag: it lights up while one is over it
    /// that dropping would move, and the drop moves it into the tab.
    pub(super) fn hz_pane_drop_target<E: gpui::InteractiveElement>(
        el: E,
        tab: String,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> E {
        let accent = theme.accent;
        let (lit, dropped) = (tab.clone(), tab);
        el.drag_over::<HzPaneDrag>(move |style, drag, _, _| {
            if drag.tab.as_deref() != Some(lit.as_str()) {
                style.bg(accent.opacity(0.32))
            } else {
                style
            }
        })
        .on_drop::<HzPaneDrag>(cx.listener(move |this, drag: &HzPaneDrag, _, cx| {
            this.hz_drop_on_tab(drag, &dropped, cx);
        }))
    }
}
