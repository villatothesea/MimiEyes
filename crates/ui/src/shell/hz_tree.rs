//! MimiEyes (VENDORED.md #2): the tree sidebar.
//!
//! It only shows how things are organized, drawn from the SAME data the
//! stock list uses (`AppState::sidebar_chats`) plus the pane view's tabs:
//!
//! ```text
//! PINNED                              section labels: Zeron's uppercase text
//!   ■ ✳ Release notes          2h
//! LOCAL                               one label per machine
//! ▾ ■ MimiReader            +  4      project: fold, mark, Zeron's "+", count
//!   ├▾ 1  Team             + ●        tab (panes on): place + name, status
//!   │  ├ ■ ✳ Planner  a1b2c3d4 now     pane: mark, CLI logo, title, id, age
//!   │  └ ■ ◆ Worker            2m
//!   └▸ ⋯ Other sessions (12)          sessions no pane shows
//! ```
//!
//! Every row acts on what it is — click, double-click (rename), right click
//! (its menu), drops and "+" — through the stock or pane-view paths. With
//! panes off the tab level drops out: machine → project → sessions.
//! Rows and tree lines follow HerdrGui's tree measure for measure.

use super::hz_layout::HzViewTab;
use super::*;
use std::collections::HashMap;

/// A machine label and the projects under it.
struct TreeDev {
    id: String,
    name: String,
    online: bool,
    local: bool,
    spaces: Vec<TreeSpace>,
    /// Sessions with no project (key `""`), under "No project".
    loose: Vec<TreeChat>,
}

struct TreeSpace {
    id: String,
    name: String,
    status: zeron_proto::ChatIndicator,
    chats: Vec<TreeChat>,
}

#[derive(Clone)]
struct TreeChat {
    id: String,
    title: String,
    status: zeron_proto::ChatIndicator,
    harness: zeron_proto::HarnessId,
    /// When it last spoke (its creation, before the first message).
    at: chrono::DateTime<Utc>,
}

/// Aggregate marks prefer attention: needs-you > error > working > done > idle.
fn rank(indicator: zeron_proto::ChatIndicator) -> u8 {
    use zeron_proto::ChatIndicator as I;
    match indicator {
        I::AwaitingInput => 5,
        I::Errored => 4,
        I::Working => 3,
        I::Completed => 2,
        I::Idle => 1,
    }
}

fn best(
    a: zeron_proto::ChatIndicator,
    b: zeron_proto::ChatIndicator,
) -> zeron_proto::ChatIndicator {
    if rank(a) >= rank(b) { a } else { b }
}

fn aggregate(chats: &[TreeChat]) -> zeron_proto::ChatIndicator {
    chats
        .iter()
        .fold(zeron_proto::ChatIndicator::Idle, |acc, c| best(acc, c.status))
}

/// HerdrGui's compact age: "now", "5m", "3h", "2d", "6w".
fn age(at: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let secs = (now - at).num_seconds().max(0);
    match secs {
        0..60 => "now".to_owned(),
        60..3_600 => format!("{}m", secs / 60),
        3_600..86_400 => format!("{}h", secs / 3_600),
        86_400..604_800 => format!("{}d", secs / 86_400),
        _ => format!("{}w", secs / 604_800),
    }
}

/// Row geometry (HerdrGui's tree).
const ROW_H: f32 = 30.0;
const PANE_ROW_H: f32 = 26.0;
/// One level of the tree: its line runs at `LINE_X` inside the column.
const LEVEL_W: f32 = 24.0;
const LINE_X: f32 = 15.5;

/// One row of the tree's body: its ancestors' guide lines (`true` where an
/// ancestor still has siblings below), then its own branch (`last` ends the
/// line at the tick), then the row.
fn branch(guides: &[bool], last: bool, row: AnyElement, line: gpui::Hsla) -> AnyElement {
    let column = |full: bool, tick: bool| {
        let mut col = div().flex_none().relative().w(px(LEVEL_W)).h_full();
        if full || tick {
            col = col.child(
                div()
                    .absolute()
                    .left(px(LINE_X))
                    .top_0()
                    .w(px(1.0))
                    .when(tick && !full, |el| el.h(px(13.0)))
                    .when(full, |el| el.bottom_0())
                    .bg(line),
            );
        }
        if tick {
            col = col.child(
                div()
                    .absolute()
                    .left(px(LINE_X + 0.5))
                    .top(px(13.0))
                    .w(px(8.0))
                    .h(px(1.0))
                    .bg(line),
            );
        }
        col
    };
    div()
        .flex()
        .items_start()
        .h(px(PANE_ROW_H + 1.0))
        .children(guides.iter().map(|g| column(*g, false)))
        .child(column(!last, true))
        .child(row)
        .into_any_element()
}

impl Shell {
    /// The tree swaps in for the stock session list (same column chrome:
    /// space filter, banners, footer — only the list body differs).
    pub(super) fn render_hz_tree(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = Utc::now();
        self.hz_tree_reveal_rename(cx);
        let pin_order = self.active_sidebar_pins(cx);
        let panes_on = self.hz_pane_view_enabled();
        let panes_project = panes_on.then(|| self.hz_key());
        let filter = self.settings.space_filter.clone();
        // Everything the rows need, under one state borrow.
        let (devs, pinned, all_chats, selected, current_key) = {
            let state = self.state.read(cx);
            let row_of = |chat: &zeron_proto::Chat, indicator| TreeChat {
                id: chat.id.clone(),
                title: transcript::single_line(
                    &chat.title.clone().unwrap_or_else(|| chat.id.clone()),
                ),
                status: indicator,
                harness: chat
                    .config
                    .as_ref()
                    .map(|c| c.harness)
                    .unwrap_or(zeron_proto::HarnessId::ClaudeCode),
                at: chat.last_message_at.unwrap_or(chat.created_at),
            };
            // Every live session (side chats included: panes may hold them).
            let all_chats: HashMap<String, TreeChat> = state
                .chats
                .iter()
                .filter(|c| !c.archived)
                .map(|c| (c.id.clone(), row_of(c, state.display_status_for(c, now))))
                .collect();
            let mut by_space: HashMap<String, Vec<TreeChat>> = HashMap::new();
            let mut by_dev_loose: HashMap<String, Vec<TreeChat>> = HashMap::new();
            let mut pinned: Vec<TreeChat> = Vec::new();
            for (indicator, chat) in state.sidebar_chats(now, filter.as_deref()) {
                let row = row_of(chat, indicator);
                // A pinned session lists under "Pinned" only, as in the
                // stock list.
                if pin_order.contains(&chat.id) {
                    pinned.push(row);
                    continue;
                }
                match &chat.space_id {
                    Some(space) => by_space.entry(space.clone()).or_default().push(row),
                    None => by_dev_loose
                        .entry(chat.device_id.clone())
                        .or_default()
                        .push(row),
                }
            }
            let mut spaces_by_dev: HashMap<String, Vec<TreeSpace>> = HashMap::new();
            for space in &state.spaces {
                let chats = by_space.remove(&space.id).unwrap_or_default();
                // A project shows when it has sessions — or, with panes on,
                // tabs (a tab of new sessions only still belongs on the tree).
                let has_tabs = panes_on
                    && self.settings.hz_layouts.contains_key(&space.id)
                    && filter.as_deref().is_none_or(|f| f == space.id);
                if chats.is_empty() && !has_tabs {
                    continue;
                }
                spaces_by_dev
                    .entry(space.device_id.clone())
                    .or_default()
                    .push(TreeSpace {
                        id: space.id.clone(),
                        name: space.display_name().to_owned(),
                        status: aggregate(&chats),
                        chats,
                    });
            }
            let local = state.local_device_id.clone();
            let mut devs: Vec<TreeDev> = state
                .devices
                .iter()
                .filter_map(|dev| {
                    let mut spaces = spaces_by_dev.remove(&dev.id).unwrap_or_default();
                    spaces.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                    let loose = by_dev_loose.remove(&dev.id).unwrap_or_default();
                    let is_local = local.as_deref() == Some(dev.id.as_str());
                    let loose_tabs = is_local
                        && panes_on
                        && filter.is_none()
                        && self.settings.hz_layouts.contains_key("");
                    if spaces.is_empty() && loose.is_empty() && !loose_tabs {
                        return None;
                    }
                    Some(TreeDev {
                        id: dev.id.clone(),
                        name: if is_local {
                            "Local".to_owned()
                        } else {
                            dev.name.clone()
                        },
                        online: state.device_online(&dev.id, now),
                        local: is_local,
                        spaces,
                        loose,
                    })
                })
                .collect();
            devs.sort_by(|a, b| {
                b.local
                    .cmp(&a.local)
                    .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            // Sessions on devices the window does not know still show.
            if !by_dev_loose.is_empty() || !spaces_by_dev.is_empty() {
                let mut spaces: Vec<TreeSpace> = spaces_by_dev.into_values().flatten().collect();
                spaces.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                devs.push(TreeDev {
                    id: "\0other".into(),
                    name: "Other devices".into(),
                    online: false,
                    local: false,
                    spaces,
                    loose: by_dev_loose.into_values().flatten().collect(),
                });
            }
            pinned.sort_by_key(|c| pin_order.iter().position(|id| *id == c.id));
            let selected = state.selected_chat.clone();
            let current_key = panes_project.or_else(|| {
                selected
                    .as_deref()
                    .and_then(|id| state.chats.iter().find(|c| c.id == id))
                    .map(|c| c.space_id.clone().unwrap_or_default())
            });
            (devs, pinned, all_chats, selected, current_key)
        };

        let line = theme.text_faint.opacity(0.45);
        let mut rows: Vec<AnyElement> = Vec::new();
        if !pinned.is_empty() {
            rows.push(self.hz_tree_label("pinned", "Pinned", None, theme, cx));
            let last = pinned.len().saturating_sub(1);
            let body: Vec<AnyElement> = pinned
                .iter()
                .enumerate()
                .map(|(ix, chat)| {
                    let row = self.hz_tree_chat_row(chat, &selected, theme, cx);
                    branch(&[], ix == last, row, line)
                })
                .collect();
            rows.push(self.hz_fold_body("pinned", !self.hz_collapsed("pinned"), body));
        }
        for dev in &devs {
            let key = format!("dev:{}", dev.id);
            rows.push(self.hz_tree_label(&key, &dev.name, Some(dev.online || dev.local), theme, cx));
            let mut body = Vec::new();
            for space in &dev.spaces {
                body.push(self.hz_tree_project(
                    &space.id,
                    &space.name,
                    space.status,
                    &space.chats,
                    current_key.as_deref() == Some(space.id.as_str()),
                    &all_chats,
                    &selected,
                    theme,
                    cx,
                ));
            }
            // Sessions in no project; the "" tabs belong to this machine.
            let loose_tabs = dev.local
                && panes_on
                && filter.is_none()
                && self.settings.hz_layouts.contains_key("");
            if !dev.loose.is_empty() || loose_tabs {
                let key = if dev.local { String::new() } else { format!("\0loose:{}", dev.id) };
                body.push(self.hz_tree_project(
                    &key,
                    "No project",
                    aggregate(&dev.loose),
                    &dev.loose,
                    current_key.as_deref() == Some(key.as_str()),
                    &all_chats,
                    &selected,
                    theme,
                    cx,
                ));
            }
            let open = !self.hz_collapsed(&key);
            rows.push(self.hz_fold_body(&key, open, body));
        }
        if devs.is_empty() && pinned.is_empty() {
            rows.push(
                div()
                    .px(px(Theme::SPACE_SM))
                    .py(px(6.0))
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_faint)
                    .child("No sessions yet")
                    .into_any_element(),
            );
        }

        // Same chrome as the stock column: filter row on top, scrollable list
        // in an edge-fade scope, banners, then the footer.
        let filter_row = self.render_spaces_filter(theme, cx);
        let lists = crate::edge_fade::edge_faded(
            SIDEBAR_GLASS_FADE_BAND,
            true,
            true,
            div().relative().flex_1().min_h_0().child(
                div()
                    .id("hz-tree")
                    .relative()
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.sidebar_scroll)
                    .px(px(Theme::SPACE_SM))
                    .flex()
                    .flex_col()
                    .pt(px(SIDEBAR_LIST_PAD_TOP))
                    .pb(px(Theme::SPACE_SM))
                    .children(rows),
            ),
        )
        .fade_overflow_y(&self.sidebar_scroll);

        let update_strip = self.render_update_strip(theme, cx);
        let github_star_banner = self.render_github_star_banner(update_strip.is_some(), theme, cx);
        div()
            .w(px(self.sidebar_content_width()))
            .h_full()
            .flex()
            .flex_col()
            .child(filter_row)
            .child(lists)
            .when_some(self.render_connection_pill(theme, cx), |el, pill| {
                el.child(pill)
            })
            .when_some(github_star_banner, |el, banner| el.child(banner))
            .when_some(update_strip, |el, strip| el.child(strip))
            .child(
                div()
                    .p(px(Theme::SPACE_SM))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(self.render_sidebar_footer_row(theme, cx)),
            )
            .into_any_element()
    }

    /// A rename must see its row: unfold the ancestors of the chat it is
    /// open on (the stock path unfolds its own groups; the tree's folds are
    /// ours).
    fn hz_tree_reveal_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = self
            .chat_rename
            .as_ref()
            .filter(|r| r.surface == ChatRenameSurface::Sidebar)
        else {
            return;
        };
        let Some(chat) = self
            .state
            .read(cx)
            .chats
            .iter()
            .find(|c| c.id == rename.chat_id)
            .cloned()
        else {
            return;
        };
        let mut keys = vec![format!("dev:{}", chat.device_id), "pinned".to_owned()];
        let project = chat.space_id.clone().unwrap_or_default();
        keys.push(format!("sp:{project}"));
        keys.push(format!("other:{project}"));
        if let Some(layout) = self.settings.hz_layouts.get(&project)
            && let Some(tab) = layout.tab_with_chat(&chat.id)
        {
            keys.push(format!("tab:{}", tab.id));
        }
        let before = self.settings.hz_tree_collapsed.len();
        self.settings
            .hz_tree_collapsed
            .retain(|k| !keys.contains(k));
        if self.settings.hz_tree_collapsed.len() != before {
            self.schedule_save(cx);
        }
    }

    /// A section label in Zeron's style — uppercase text, nothing else
    /// ("PINNED", "LOCAL", a machine's name). A click folds what is under it;
    /// an offline machine carries a grey dot.
    fn hz_tree_label(
        &mut self,
        key: &str,
        text: &str,
        online: Option<bool>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let fold = key.to_owned();
        div()
            .id(SharedString::from(format!("hz-tree-label-{key}")))
            .mt(px(6.0))
            .px(px(8.0))
            .h(px(24.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(theme.text_faint)
            .hover(|st| st.text_color(theme.text_muted))
            .on_click(cx.listener(move |this, _, _, cx| this.hz_toggle_fold(&fold, cx)))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(text.to_uppercase())),
            )
            .when(online == Some(false), |el| {
                el.child(
                    div()
                        .flex_none()
                        .size(px(6.0))
                        .rounded_full()
                        .bg(theme.text_faint.opacity(0.5)),
                )
            })
            .into_any_element()
    }

    /// The session-count pill of a group row.
    fn hz_tree_pill(count: usize, theme: &Theme) -> gpui::Div {
        div()
            .flex_none()
            .px(px(6.0))
            .rounded_full()
            .bg(crate::theme::wash(0.08))
            .text_size(px(11.0))
            .text_color(theme.text_muted)
            .child(SharedString::from(count.to_string()))
    }

    /// A row's fold button (its own hit area, so the row's click stays
    /// "show it").
    fn hz_tree_fold_button(&self, key: &str, open: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let fold = key.to_owned();
        div()
            .id(SharedString::from(format!("hz-tree-fold-{key}")))
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .hover(|st| st.bg(crate::theme::wash(0.10)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.hz_toggle_fold(&fold, cx);
            }))
            .child(self.hz_chevron(key, open, theme))
            .into_any_element()
    }

    /// A hover-only "+" button.
    fn hz_tree_plus(
        id: SharedString,
        group: SharedString,
        theme: &Theme,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(id)
            .flex_none()
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.0))
            .opacity(0.0)
            .group_hover(group, |st| st.opacity(1.0))
            .hover(|st| st.bg(crate::theme::wash(0.10)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                on_click(this, cx);
            }))
            .child(icon(icons::PLUS).size(px(13.0)).text_color(theme.text_muted))
            .into_any_element()
    }

    /// A project row and, folded under it: its tabs and their panes (panes
    /// on), then "Other sessions"; with panes off, its sessions. `project` is
    /// its key (`""`: sessions in no project).
    #[allow(clippy::too_many_arguments)]
    fn hz_tree_project(
        &mut self,
        project: &str,
        name: &str,
        status: zeron_proto::ChatIndicator,
        chats: &[TreeChat],
        current: bool,
        all_chats: &HashMap<String, TreeChat>,
        selected: &Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = format!("sp:{project}");
        let open = !self.hz_collapsed(&key);
        let mark = self.hz_mark(format!("hz-tree-mark-{key}"), status, theme, cx);
        // A remote machine's "No project" group has no tabs or menu.
        let real = !project.starts_with('\0');
        let latest = chats.first().map(|c| c.id.clone());
        let (open_project, new_project) = (project.to_owned(), project.to_owned());
        let menu_space = Some(project.to_owned()).filter(|p| real && !p.is_empty());
        // This machine's "No project" has its own menu (new session, remove).
        let loose_menu = project.is_empty();
        let group = SharedString::from(format!("hz-tree-ws-group-{project}"));
        let plus = real.then(|| {
            Self::hz_tree_plus(
                SharedString::from(format!("hz-tree-new-{project}")),
                group.clone(),
                theme,
                move |this, cx| this.hz_new_session_in_project(new_project.clone(), cx),
                cx,
            )
        });
        let row = div()
            .id(SharedString::from(format!("hz-tree-ws-{project}")))
            .group(group)
            .mb(px(1.0))
            .px(px(8.0))
            .h(px(ROW_H))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(if project.is_empty() || !real {
                theme.text_muted
            } else {
                theme.text
            })
            .when(current, |el| el.bg(crate::theme::wash(0.04)))
            .hover(|st| st.bg(crate::theme::wash(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.hz_open_project(open_project.clone(), latest.clone(), cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if let Some(space) = &menu_space {
                        this.space_menu.open((space.clone(), event.position));
                        cx.stop_propagation();
                        cx.notify();
                    } else if loose_menu {
                        this.hz.loose_menu = Some(event.position);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .child(self.hz_tree_fold_button(&key, open, theme, cx))
            .child(mark)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(name.to_owned())),
            )
            .children(plus)
            .child(Self::hz_tree_pill(chats.len(), theme));

        let line = theme.text_faint.opacity(0.45);
        let tabs: Vec<HzViewTab> = if real && self.hz_pane_view_enabled() {
            self.settings
                .hz_layouts
                .get(project)
                .map(|l| l.tabs.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut body: Vec<AnyElement> = Vec::new();
        if tabs.is_empty() {
            // Panes off (or none here): the project's sessions.
            let last = chats.len().saturating_sub(1);
            for (ix, chat) in chats.iter().enumerate() {
                let row = self.hz_tree_chat_row(chat, selected, theme, cx);
                body.push(branch(&[], ix == last, row, line));
            }
        } else {
            let placed: std::collections::HashSet<String> =
                tabs.iter().flat_map(|t| t.root.chats()).collect();
            let others: Vec<TreeChat> = chats
                .iter()
                .filter(|c| !placed.contains(&c.id))
                .cloned()
                .collect();
            let active = self
                .settings
                .hz_layouts
                .get(project)
                .and_then(|l| l.active_tab())
                .map(|t| t.id.clone());
            let tab_count = tabs.len();
            for (ix, tab) in tabs.iter().enumerate() {
                let last = ix + 1 == tab_count && others.is_empty();
                let on_screen = current && active.as_deref() == Some(tab.id.as_str());
                body.extend(self.hz_tree_tab(project, ix, tab, last, on_screen, all_chats, theme, cx));
            }
            if !others.is_empty() {
                body.extend(self.hz_tree_others(project, &others, selected, theme, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .child(row)
            .child(self.hz_fold_body(&key, open, body))
            .into_any_element()
    }

    /// A tab row (on the project's line) and, folded under it, its panes in
    /// layout order (left to right, top to bottom).
    #[allow(clippy::too_many_arguments)]
    fn hz_tree_tab(
        &mut self,
        project: &str,
        place: usize,
        tab: &HzViewTab,
        last: bool,
        on_screen: bool,
        all_chats: &HashMap<String, TreeChat>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let line = theme.text_faint.opacity(0.45);
        let key = format!("tab:{}", tab.id);
        let open = !self.hz_collapsed(&key);
        let status = self.hz_tab_status(tab, cx);
        // The tab's columns line up with its panes' rows below: what its
        // panes are up to in the dots' 12px slot (kept empty while all are
        // idle), the place number in the CLI logos' 14px slot, then the name
        // where the session titles start.
        let status_mark = match status {
            Some(status) => self.hz_mark(format!("hz-tree-tabmark-{}", tab.id), status, theme, cx),
            None => div().flex_none().w(px(12.0)).into_any_element(),
        };
        let number = div()
            .flex_none()
            .w(px(14.0))
            .flex()
            .justify_center()
            .text_size(px(11.0))
            .text_color(theme.text_faint)
            .child(SharedString::from((place + 1).to_string()));
        let group = SharedString::from(format!("hz-tree-tab-group-{}", tab.id));
        let ids = (project.to_owned(), tab.id.clone());
        let (click, dbl, menu, plus_ids) = (ids.clone(), ids.clone(), ids.clone(), ids.clone());
        let plus = Self::hz_tree_plus(
            SharedString::from(format!("hz-tree-tab-new-{}", tab.id)),
            group.clone(),
            theme,
            move |this, cx| this.hz_tree_new_pane(&plus_ids.0, &plus_ids.1, cx),
            cx,
        );
        let label: AnyElement = match self.hz_tree_rename_input(&tab.id) {
            Some(input) => chat_title_editor(
                SharedString::from(format!("hz-tree-tab-editor-{}", tab.id)),
                input,
                theme,
            ),
            None => {
                let name = tab.name.trim();
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(SharedString::from(if name.is_empty() {
                        format!("Tab {}", place + 1)
                    } else {
                        name.to_owned()
                    }))
                    .into_any_element()
            }
        };
        let mut row = div()
            .id(SharedString::from(format!("hz-tree-tab-{}", tab.id)))
            .group(group)
            .h(px(PANE_ROW_H))
            .flex_1()
            .min_w_0()
            .mb(px(1.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(theme.text)
            .when(on_screen, |el| el.bg(crate::theme::wash(0.07)))
            .hover(|st| st.bg(crate::theme::wash(0.05)))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.hz_tree_rename_tab(&dbl.0, &dbl.1, window, cx);
                } else {
                    this.hz_tree_open_tab(&click.0, &click.1, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.hz_tree_tab_menu(&menu.0, &menu.1, event.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(self.hz_tree_fold_button(&key, open, theme, cx))
            .child(status_mark)
            .child(number)
            .child(label)
            .child(plus)
            .child(Self::hz_tree_pill(tab.root.leaf_count(), theme));
        // A drag let go on the row moves it into this tab (the tabs on
        // screen only: drops act on the project shown).
        if project == self.hz_key() {
            row = Self::hz_pane_drop_target(row, tab.id.clone(), theme, cx);
        }
        let mut out = vec![branch(&[], last, row.into_any_element(), line)];

        let leaves: Vec<String> = tab.root.leaf_ids().into_iter().map(str::to_owned).collect();
        let pane_count = leaves.len();
        let mut panes = Vec::new();
        for (ix, leaf) in leaves.iter().enumerate() {
            let chat = tab.root.chat_of(leaf).cloned().flatten();
            let focused = on_screen && tab.focused.as_deref() == Some(leaf.as_str());
            let row = self.hz_tree_pane_row(project, &tab.id, leaf, chat.as_deref(), focused, all_chats, theme, cx);
            panes.push(branch(&[!last], ix + 1 == pane_count, row, line));
        }
        out.push(self.hz_fold_body(&key, open, panes));
        out
    }

    /// A pane row: the session's mark, CLI logo, title, short id (on
    /// hover) and age — or "New session" for a pane that has none yet. A
    /// click focuses that pane, a double click renames its session, a right
    /// click opens the pane menu; it drags like the pane's own tag.
    #[allow(clippy::too_many_arguments)]
    fn hz_tree_pane_row(
        &mut self,
        project: &str,
        tab: &str,
        leaf: &str,
        chat: Option<&str>,
        focused: bool,
        all_chats: &HashMap<String, TreeChat>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row_chat = chat.and_then(|id| all_chats.get(id)).cloned();
        let ids = (project.to_owned(), tab.to_owned(), leaf.to_owned());
        let (click, menu) = (ids.clone(), ids.clone());
        let rename = chat.map(str::to_owned);
        let group = SharedString::from(format!("hz-tree-pane-group-{leaf}"));
        let editor = chat.and_then(|id| self.rename_input_for(id, ChatRenameSurface::Sidebar));
        let (mark, logo) = match &row_chat {
            Some(c) => {
                let (harness, tint) = crate::pickers::harness_brand_icon(c.harness);
                (
                    self.hz_mark(format!("hz-tree-pane-mark-{leaf}"), c.status, theme, cx),
                    icon(harness)
                        .size(px(14.0))
                        .flex_none()
                        .text_color(tint.unwrap_or(theme.text_muted))
                        .into_any_element(),
                )
            }
            None => (
                div()
                    .flex_none()
                    .w(px(12.0))
                    .flex()
                    .justify_center()
                    .child(
                        icon(icons::PLUS)
                            .size(px(12.0))
                            .text_color(theme.text_faint),
                    )
                    .into_any_element(),
                div().flex_none().w(px(14.0)).into_any_element(),
            ),
        };
        let label: AnyElement = match (editor, &row_chat) {
            (Some(input), _) => chat_title_editor(
                SharedString::from(format!("hz-tree-pane-editor-{leaf}")),
                input,
                theme,
            ),
            (None, Some(c)) => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(c.title.clone()))
                .into_any_element(),
            (None, None) => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(theme.text_faint)
                .child(if chat.is_some() { "Starting…" } else { "New session" })
                .into_any_element(),
        };
        let short = chat.map(|id| id.chars().take(8).collect::<String>());
        let row = div()
            .id(SharedString::from(format!("hz-tree-pane-{leaf}")))
            .group(group.clone())
            .h(px(PANE_ROW_H))
            .flex_1()
            .min_w_0()
            .mb(px(1.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(theme.text)
            .when(focused, |el| el.bg(crate::theme::wash(0.09)))
            .hover(|st| st.bg(crate::theme::wash(0.05)))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() >= 2
                    && let Some(chat) = &rename
                {
                    this.open_rename_chat(chat.clone(), cx);
                } else {
                    this.hz_tree_focus_pane(&click.0, &click.1, &click.2, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.hz_tree_pane_menu(&menu.0, &menu.1, &menu.2, event.position, cx);
                    cx.stop_propagation();
                }),
            )
            .child(mark)
            .child(logo)
            .child(label)
            .children(short.map(|short| {
                div()
                    .flex_none()
                    .opacity(0.0)
                    .group_hover(group, |st| st.opacity(1.0))
                    .font_family(theme.font_mono.clone())
                    .text_size(px(10.5))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(short))
            }))
            .children(row_chat.as_ref().map(|c| {
                div()
                    .flex_none()
                    .ml(px(2.0))
                    .text_size(px(11.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(age(c.at, Utc::now())))
            }));
        let row = match chat {
            Some(chat) => self.hz_tree_row_drag(row, chat, theme, cx),
            None => row,
        };
        row.into_any_element()
    }

    /// "Other sessions (n)": the project's sessions no pane shows, folded
    /// by default (the tabs stay compact; the rest is one click away).
    fn hz_tree_others(
        &mut self,
        project: &str,
        chats: &[TreeChat],
        selected: &Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let line = theme.text_faint.opacity(0.45);
        let key = format!("other:{project}");
        // Folded unless the user opened it (the opened set is stored inverted).
        let opened_key = format!("open:{key}");
        let open = self.hz_collapsed(&opened_key);
        let toggle = opened_key.clone();
        let row = div()
            .id(SharedString::from(format!("hz-tree-others-{project}")))
            .h(px(PANE_ROW_H))
            .flex_1()
            .min_w_0()
            .mb(px(1.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(12.5))
            .text_color(theme.text_muted)
            .hover(|st| st.bg(crate::theme::wash(0.05)))
            .on_click(cx.listener(move |this, _, _, cx| this.hz_toggle_fold(&toggle, cx)))
            .child(
                div()
                    .flex_none()
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.hz_chevron(&opened_key, open, theme)),
            )
            .child(div().flex_1().min_w_0().truncate().child("Other sessions"))
            .child(Self::hz_tree_pill(chats.len(), theme))
            .into_any_element();
        let mut out = vec![branch(&[], true, row, line)];
        let last = chats.len().saturating_sub(1);
        let rows: Vec<AnyElement> = chats
            .iter()
            .enumerate()
            .map(|(ix, chat)| {
                let row = self.hz_tree_chat_row(chat, selected, theme, cx);
                branch(&[false], ix == last, row, line)
            })
            .collect();
        out.push(self.hz_fold_body(&opened_key, open, rows));
        out
    }

    /// A session row: mark, CLI logo, title, age. Same actions as a stock
    /// sidebar row: click opens, double click renames in place, right click
    /// menus; with panes on it drags like a pane's name tag.
    fn hz_tree_chat_row(
        &mut self,
        chat: &TreeChat,
        selected: &Option<String>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = chat.id.clone();
        let is_selected = selected.as_deref() == Some(id.as_str());
        let editor = self.rename_input_for(&id, ChatRenameSurface::Sidebar);
        let mark = self.hz_mark(format!("hz-tree-mark-{id}"), chat.status, theme, cx);
        let (harness, tint) = crate::pickers::harness_brand_icon(chat.harness);
        let (open_id, menu_id, sel_id) = (id.clone(), id.clone(), id.clone());
        let label = match editor {
            Some(input) => chat_title_editor(
                SharedString::from(format!("hz-tree-editor-{id}")),
                input,
                theme,
            ),
            None => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(SharedString::from(chat.title.clone()))
                .into_any_element(),
        };
        let row = div()
            .id(SharedString::from(format!("hz-tree-chat-{id}")))
            .debug_selector(move || format!("session-row-{sel_id}"))
            .h(px(PANE_ROW_H))
            .flex_1()
            .min_w_0()
            .mb(px(1.0))
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(13.0))
            .text_color(theme.text)
            .when(is_selected, |el| el.bg(crate::theme::wash(0.09)))
            .hover(|st| st.bg(crate::theme::wash(0.05)))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.open_rename_chat(open_id.clone(), cx);
                } else {
                    this.open_chat(open_id.clone(), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.chat_menu.open(ChatMenuState {
                        tab: None,
                        chat_id: menu_id.clone(),
                        position: event.position,
                        page: ChatMenuPage::Root,
                    });
                    cx.notify();
                }),
            )
            .child(mark)
            .child(
                icon(harness)
                    .size(px(14.0))
                    .flex_none()
                    .text_color(tint.unwrap_or(theme.text_muted)),
            )
            .child(label)
            .child(
                div()
                    .flex_none()
                    .ml(px(6.0))
                    .text_size(px(11.0))
                    .text_color(theme.text_faint)
                    .child(SharedString::from(age(chat.at, Utc::now()))),
            );
        // HerdrGui's pane drag: the row drags like the pane's name tag, and a
        // drag let go on it swaps or moves into its tab.
        self.hz_tree_row_drag(row, &id, theme, cx).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_like_herdrgui() {
        let now = Utc::now();
        let ago = |secs: i64| age(now - chrono::Duration::seconds(secs), now);
        assert_eq!(ago(5), "now");
        assert_eq!(ago(32 * 60), "32m");
        assert_eq!(ago(2 * 3_600 + 10), "2h");
        assert_eq!(ago(3 * 86_400), "3d");
        assert_eq!(ago(15 * 86_400), "2w");
        // A clock skewed into the future still reads "now".
        assert_eq!(ago(-30), "now");
    }
}
