//! MimiEyes (VENDORED.md #10): Settings → Closed panes — every session no
//! pane shows. Closed panes come first, newest first, each with the project
//! and tab it left; then sessions that were never in a pane (an agent's, or
//! from before panes). Restore puts one back into its tab (the shell decides
//! where: `Shell::hz_restore_closed`) and brings it on screen; Delete asks
//! the shell's "Delete session?" confirmation, then deletes it for good.

use gpui::{AnyElement, Context, EventEmitter, SharedString, Window, div, prelude::*, px};
use zeron_proto::HarnessId;

use crate::popover;
use crate::settings::widgets;
use crate::theme::Theme;

/// One row, as the shell lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClosedItem {
    pub chat_id: String,
    pub title: String,
    /// "Project › Tab" for a closed pane, the project for the rest.
    pub place: String,
    /// When its pane closed (epoch ms); `None`: never in a pane.
    pub closed_at: Option<i64>,
    /// Its last activity (epoch ms).
    pub active_at: i64,
    pub harness: Option<HarnessId>,
}

pub enum ClosedPanesEvent {
    Restore(String),
    Delete(String),
}

pub struct ClosedPanesPage {
    items: Vec<ClosedItem>,
    scroll: widgets::PageScroll,
}

impl EventEmitter<ClosedPanesEvent> for ClosedPanesPage {}

impl ClosedPanesPage {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            items: Vec::new(),
            scroll: widgets::PageScroll::default(),
        }
    }

    /// The shell's current list (it knows the layouts).
    pub fn set_items(&mut self, items: Vec<ClosedItem>) {
        self.items = items;
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn row(&self, ix: usize, item: &ClosedItem, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let now = chrono::Utc::now();
        let ago = |ms: i64| {
            chrono::DateTime::from_timestamp_millis(ms)
                .map(|t| crate::state::format_time_ago(t, now))
                .unwrap_or_default()
        };
        let when: SharedString = match item.closed_at {
            Some(at) => format!("closed {}", ago(at)).into(),
            None => ago(item.active_at).into(),
        };
        let (glyph, tint) = crate::pickers::harness_brand_icon(item.harness.unwrap_or(HarnessId::Mock));
        let chat = item.chat_id.clone();
        let delete_chat = item.chat_id.clone();
        div()
            .id(("closed-pane-row", ix))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .rounded(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .hover(|s| s.bg(crate::theme::ink(0.03)))
            .child(
                div()
                    .flex_none()
                    .size(px(32.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        crate::icons::icon(glyph)
                            .size(px(16.0))
                            .text_color(tint.unwrap_or(theme.text_muted.opacity(0.6))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(crate::typography::ui_rems(13.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(SharedString::from(item.title.clone())),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(crate::typography::ui_rems(11.0))
                                    .text_color(theme.text_muted)
                                    .child(when),
                            ),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .min_w_0()
                            .truncate()
                            .text_size(crate::typography::ui_rems(11.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(match item.closed_at {
                                Some(_) => item.place.clone(),
                                None => format!("{} · never in a pane", item.place),
                            })),
                    ),
            )
            .child(
                div()
                    .id(("closed-pane-restore", ix))
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.border)
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .opacity(0.8)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.surface_raised).text_color(theme.text))
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ClosedPanesEvent::Restore(chat.clone()));
                    }))
                    .child(
                        crate::icons::icon(crate::icons::ARCHIVE_UP_MINIMALISTIC)
                            .size(px(14.0))
                            .text_color(theme.text_muted),
                    )
                    .child(SharedString::from("Restore")),
            )
            .child(
                div()
                    .id(("closed-pane-delete", ix))
                    .flex_none()
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme.border)
                    .text_color(theme.text_muted)
                    .opacity(0.8)
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.danger.opacity(0.12)).text_color(theme.danger))
                    .tab_index(0)
                    .role(gpui::Role::Button)
                    .aria_label("Delete session")
                    .focus_visible(|s| s.border_2().border_color(theme.accent).opacity(1.0))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ClosedPanesEvent::Delete(delete_chat.clone()));
                    }))
                    .child(crate::icons::icon(crate::icons::TRASH_BIN_MINIMALISTIC).size(px(14.0))),
            )
            .into_any_element()
    }
}

impl popover::ScrollRailHost for ClosedPanesPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for ClosedPanesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let items = self.items.clone();
        let closed: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.closed_at.is_some())
            .map(|(ix, item)| self.row(ix, item, &theme, cx))
            .collect();
        let never: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.closed_at.is_none())
            .map(|(ix, item)| self.row(ix, item, &theme, cx))
            .collect();
        let section = |title: &'static str, rows: Vec<AnyElement>| {
            div()
                .mt(px(24.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(widgets::section_label(&theme, title).pb(px(6.0)))
                .children(rows)
        };
        let body: AnyElement = if items.is_empty() {
            div()
                .mt(px(96.0))
                .flex()
                .flex_col()
                .items_center()
                .text_center()
                .text_color(theme.text_muted)
                .child(
                    crate::icons::icon(crate::icons::EVEN_PANES)
                        .size(px(28.0))
                        .text_color(theme.text_muted.opacity(0.2)),
                )
                .child(
                    div()
                        .mt(px(12.0))
                        .text_size(crate::typography::ui_rems(14.0))
                        .child(SharedString::from("No closed panes")),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .child(SharedString::from(
                            "A pane you close lands here; restore it into its tab anytime.",
                        )),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .when(!closed.is_empty(), |el| el.child(section("Closed panes", closed)))
                .when(!never.is_empty(), |el| el.child(section("Not in a pane", never)))
                .into_any_element()
        };
        let count = items.len();
        let scrollbar = popover::rail(self, "closed-panes-page-scrollbar", &theme, cx);
        div()
            .id("closed-panes-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("closed-panes-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(
                                    &theme,
                                    "Closed panes",
                                    (count > 0).then_some(count),
                                ))
                                .child(widgets::page_subtitle(
                                    &theme,
                                    "Sessions no pane shows. Restore one to put it back into the tab it left, or delete it.",
                                ))
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}
