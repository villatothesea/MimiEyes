//! MimiEyes (VENDORED.md #3): Settings → Usage — tokens your sessions used,
//! grouped any way: pick dimensions in order (project, tab, session, CLI,
//! model, day) and every level nests under the one before.
//!
//! The engine books each turn's tokens (`zeron_engine::hz_usage`, `HzUsage`);
//! this page reads them for the chosen time range and groups them here.
//! Projects and titles resolve from the current chat rows; a session's tab is
//! the view tab it is in now (`set_tabs`, fed by the shell).

use std::collections::{HashMap, HashSet};

use gpui::{AnyElement, Context, Entity, SharedString, Subscription, Task, Window, div, prelude::*, px};
use serde::Deserialize;
use zeron_proto::HarnessId;

use crate::popover;
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

const HZ_USAGE: &str = "HzUsage";

/// One turn's tokens, as the engine books them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub at: i64,
    pub chat_id: String,
    #[serde(default)]
    pub device_id: String,
    pub harness: HarnessId,
    #[serde(default)]
    pub model: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A way to group the records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dim {
    Project,
    Tab,
    Session,
    Cli,
    Model,
    Day,
}

impl Dim {
    const ALL: [Dim; 6] = [Dim::Project, Dim::Tab, Dim::Session, Dim::Cli, Dim::Model, Dim::Day];

    fn label(self) -> &'static str {
        match self {
            Dim::Project => "Project",
            Dim::Tab => "Tab",
            Dim::Session => "Session",
            Dim::Cli => "CLI",
            Dim::Model => "Model",
            Dim::Day => "Day",
        }
    }
}

/// Which tokens to rank and draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    Total,
    Input,
    Output,
}

impl Metric {
    fn of(self, input: u64, output: u64) -> u64 {
        match self {
            Metric::Total => input + output,
            Metric::Input => input,
            Metric::Output => output,
        }
    }
}

/// The time range read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Today,
    Week,
    Month,
    All,
}

impl Range {
    fn label(self) -> &'static str {
        match self {
            Range::Today => "Today",
            Range::Week => "7 days",
            Range::Month => "30 days",
            Range::All => "All",
        }
    }

    /// The first epoch ms the range covers (`None` = everything).
    fn since(self, now: chrono::DateTime<chrono::Local>) -> Option<i64> {
        let days = match self {
            Range::Today => 0,
            Range::Week => 6,
            Range::Month => 29,
            Range::All => return None,
        };
        let start = (now - chrono::Duration::days(days))
            .date_naive()
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(chrono::Local)
            .earliest()?;
        Some(start.timestamp_millis())
    }
}

/// One group: its label, its tokens, its turns, its sub-groups.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub label: String,
    pub input: u64,
    pub output: u64,
    pub turns: usize,
    pub children: Vec<Group>,
}

/// Group `records` by `dims` in order (`label` names a record's value on a
/// dimension); every level is sorted by `metric`, largest first.
pub fn group_records(
    records: &[&UsageRecord],
    dims: &[Dim],
    metric: Metric,
    label: &dyn Fn(&UsageRecord, Dim) -> String,
) -> Vec<Group> {
    let Some((&dim, rest)) = dims.split_first() else {
        return Vec::new();
    };
    let mut buckets: Vec<(String, Vec<&UsageRecord>)> = Vec::new();
    for record in records {
        let key = label(record, dim);
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(record),
            None => buckets.push((key, vec![record])),
        }
    }
    let mut groups: Vec<Group> = buckets
        .into_iter()
        .map(|(label_text, list)| Group {
            input: list.iter().map(|r| r.input_tokens).sum(),
            output: list.iter().map(|r| r.output_tokens).sum(),
            turns: list.len(),
            children: group_records(&list, rest, metric, label),
            label: label_text,
        })
        .collect();
    groups.sort_by(|a, b| {
        if dim == Dim::Day {
            // Days read newest first, not by size.
            b.label.cmp(&a.label)
        } else {
            metric
                .of(b.input, b.output)
                .cmp(&metric.of(a.input, a.output))
                .then_with(|| a.label.cmp(&b.label))
        }
    });
    groups
}

/// "950", "12.3k", "4.56M", "1.2B".
pub fn fmt_tokens(n: u64) -> String {
    let n = n as f64;
    if n < 1_000.0 {
        format!("{n}")
    } else if n < 1_000_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else if n < 1_000_000_000.0 {
        format!("{:.2}M", n / 1_000_000.0)
    } else {
        format!("{:.2}B", n / 1_000_000_000.0)
    }
}

fn harness_name(harness: HarnessId) -> String {
    match harness {
        HarnessId::ClaudeCode => "Claude Code".into(),
        HarnessId::Codex => "Codex".into(),
        HarnessId::Cursor => "Cursor".into(),
        HarnessId::Opencode => "OpenCode".into(),
        HarnessId::Pi => "Pi".into(),
        HarnessId::Pirust => "Pi (Rust)".into(),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "Other".into()),
    }
}

pub struct UsagePage {
    state: Entity<AppState>,
    scroll: widgets::PageScroll,
    records: Vec<UsageRecord>,
    loaded_range: Option<Range>,
    loading: Option<Task<()>>,
    error: Option<SharedString>,
    range: Range,
    metric: Metric,
    dims: Vec<Dim>,
    /// Open groups, by their path of labels.
    open: HashSet<String>,
    /// chat id → "Project › Tab" for sessions in a view tab now.
    tabs: HashMap<String, String>,
    _observe: Subscription,
}

impl UsagePage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            scroll: widgets::PageScroll::default(),
            records: Vec::new(),
            loaded_range: None,
            loading: None,
            error: None,
            range: Range::Week,
            metric: Metric::Total,
            dims: vec![Dim::Project, Dim::Session],
            open: HashSet::new(),
            tabs: HashMap::new(),
            _observe: observe,
        }
    }

    /// The view tab each session is in now (the shell knows the layouts).
    pub fn set_tabs(&mut self, tabs: HashMap<String, String>) {
        self.tabs = tabs;
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            self.error = Some("Engine not connected".into());
            return;
        };
        let range = self.range;
        let since = range.since(chrono::Local::now());
        self.loaded_range = Some(range);
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = engine
                .client()
                .call(HZ_USAGE, serde_json::json!({ "since": since }))
                .await;
            this.update(cx, |page, cx| {
                page.loading = None;
                match result {
                    Ok(value) => {
                        page.records = value
                            .get("records")
                            .cloned()
                            .and_then(|r| serde_json::from_value(r).ok())
                            .unwrap_or_default();
                        page.error = None;
                    }
                    Err(err) => {
                        page.error = Some(format!("Couldn't read usage: {err}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn label_of(&self, record: &UsageRecord, dim: Dim, cx: &gpui::App) -> String {
        let state = self.state.read(cx);
        let chat = state.chats.iter().find(|c| c.id == record.chat_id);
        match dim {
            Dim::Project => match chat {
                None => "Deleted sessions".into(),
                Some(c) => c
                    .space_id
                    .as_deref()
                    .and_then(|id| state.space_row(id))
                    .map(|s| s.display_name().to_owned())
                    .unwrap_or_else(|| "No project".into()),
            },
            Dim::Tab => self
                .tabs
                .get(&record.chat_id)
                .cloned()
                .unwrap_or_else(|| "Not in a tab".into()),
            Dim::Session => chat
                .and_then(|c| c.title.clone())
                .map(|t| crate::transcript::single_line(&t))
                .unwrap_or_else(|| {
                    format!("Session {}", record.chat_id.chars().take(8).collect::<String>())
                }),
            Dim::Cli => harness_name(record.harness),
            Dim::Model => record
                .model
                .clone()
                .unwrap_or_else(|| "Default model".into()),
            Dim::Day => chrono::DateTime::from_timestamp_millis(record.at)
                .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %a").to_string())
                .unwrap_or_default(),
        }
    }

    fn toggle_dim(&mut self, dim: Dim, cx: &mut Context<Self>) {
        if let Some(ix) = self.dims.iter().position(|d| *d == dim) {
            // Keep at least one level.
            if self.dims.len() > 1 {
                self.dims.remove(ix);
            }
        } else {
            self.dims.push(dim);
        }
        self.open.clear();
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    /// A segmented control's button.
    fn seg(
        &self,
        id: SharedString,
        label: &'static str,
        on: bool,
        theme: &Theme,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px(px(10.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(crate::typography::ui_rems(12.5))
            .text_color(if on { theme.text } else { theme.text_muted })
            .when(on, |el| el.bg(theme.wash(0.10)))
            .when(!on, |el| el.hover(|st| st.bg(theme.wash(0.05))))
            .child(SharedString::from(label))
    }

    fn render_rows(
        &self,
        groups: &[Group],
        depth: usize,
        path: &str,
        max: u64,
        out: &mut Vec<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) {
        for group in groups {
            let key = format!("{path}/{}", group.label);
            let open = self.open.contains(&key);
            let has_children = !group.children.is_empty();
            let value = self.metric.of(group.input, group.output);
            let share = if max == 0 { 0.0 } else { value as f32 / max as f32 };
            let toggle = key.clone();
            out.push(
                div()
                    .id(SharedString::from(format!("usage-row-{key}")))
                    .mx(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(7.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text)
                    .when(has_children, |el| {
                        el.cursor_pointer()
                            .hover(|st| st.bg(theme.wash(0.04)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.open.remove(&toggle) {
                                    this.open.insert(toggle.clone());
                                }
                                cx.notify();
                            }))
                    })
                    .child(div().flex_none().w(px(16.0 * depth as f32)))
                    .child(
                        div()
                            .flex_none()
                            .w(px(12.0))
                            .flex()
                            .justify_center()
                            .when(has_children, |el| {
                                el.child(
                                    crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                                        .size(px(11.0))
                                        .text_color(theme.text_muted)
                                        .with_transformation(gpui::Transformation::rotate(
                                            gpui::percentage(if open { 0.25 } else { 0.0 }),
                                        )),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(div().truncate().child(SharedString::from(group.label.clone())))
                            .child(
                                div()
                                    .h(px(4.0))
                                    .w_full()
                                    .rounded_full()
                                    .bg(theme.wash(0.06))
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .w(gpui::relative(share.clamp(0.0, 1.0)))
                                            .bg(theme.accent.opacity(0.75)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(150.0))
                            .flex()
                            .justify_end()
                            .gap(px(10.0))
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(format!("↑{}", fmt_tokens(group.input))))
                            .child(SharedString::from(format!("↓{}", fmt_tokens(group.output)))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(64.0))
                            .flex()
                            .justify_end()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(SharedString::from(fmt_tokens(value))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(56.0))
                            .flex()
                            .justify_end()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(format!(
                                "{} turn{}",
                                group.turns,
                                if group.turns == 1 { "" } else { "s" }
                            ))),
                    )
                    .into_any_element(),
            );
            if open && has_children {
                let child_max = group
                    .children
                    .iter()
                    .map(|g| self.metric.of(g.input, g.output))
                    .max()
                    .unwrap_or(0);
                self.render_rows(&group.children, depth + 1, &key, child_max, out, theme, cx);
            }
        }
    }
}

impl popover::ScrollRailHost for UsagePage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for UsagePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::toast::take_error(&mut self.error, cx);
        if self.loaded_range != Some(self.range) && self.loading.is_none() {
            self.load(cx);
        }
        let theme = Theme::of(cx).for_settings_surface();
        let records: Vec<UsageRecord> = self.records.clone();
        let refs: Vec<&UsageRecord> = records.iter().collect();
        let labels: Vec<HashMap<Dim, String>> = records
            .iter()
            .map(|r| Dim::ALL.iter().map(|d| (*d, self.label_of(r, *d, cx))).collect())
            .collect();
        let index: HashMap<*const UsageRecord, usize> = records
            .iter()
            .enumerate()
            .map(|(ix, r)| (r as *const UsageRecord, ix))
            .collect();
        let label = |r: &UsageRecord, d: Dim| -> String {
            index
                .get(&(r as *const UsageRecord))
                .and_then(|ix| labels[*ix].get(&d).cloned())
                .unwrap_or_default()
        };
        let groups = group_records(&refs, &self.dims, self.metric, &label);
        let input: u64 = records.iter().map(|r| r.input_tokens).sum();
        let output: u64 = records.iter().map(|r| r.output_tokens).sum();
        let max = groups
            .iter()
            .map(|g| self.metric.of(g.input, g.output))
            .max()
            .unwrap_or(0);
        let mut rows = Vec::new();
        self.render_rows(&groups, 0, "", max, &mut rows, &theme, cx);

        // Range and measure.
        let ranges = [Range::Today, Range::Week, Range::Month, Range::All].map(|range| {
            self.seg(
                SharedString::from(format!("usage-range-{}", range.label())),
                range.label(),
                self.range == range,
                &theme,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.range = range;
                this.open.clear();
                cx.notify();
            }))
            .into_any_element()
        });
        let metrics = [(Metric::Total, "Total"), (Metric::Input, "Input"), (Metric::Output, "Output")].map(
            |(metric, name)| {
                self.seg(
                    SharedString::from(format!("usage-metric-{name}")),
                    name,
                    self.metric == metric,
                    &theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.metric = metric;
                    cx.notify();
                }))
                .into_any_element()
            },
        );
        // Group by: chips in the order they nest.
        let dims: Vec<AnyElement> = Dim::ALL
            .iter()
            .map(|&dim| {
                let place = self.dims.iter().position(|d| *d == dim);
                let on = place.is_some();
                div()
                    .id(SharedString::from(format!("usage-dim-{}", dim.label())))
                    .px(px(10.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_1()
                    .border_color(if on { theme.accent.opacity(0.6) } else { theme.border })
                    .when(on, |el| el.bg(theme.accent.opacity(0.12)))
                    .hover(|st| st.bg(theme.wash(0.05)))
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(if on { theme.text } else { theme.text_muted })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_dim(dim, cx)))
                    .when_some(place, |el, place| {
                        el.child(
                            div()
                                .text_size(crate::typography::ui_rems(11.0))
                                .text_color(theme.accent)
                                .child(SharedString::from((place + 1).to_string())),
                        )
                    })
                    .child(SharedString::from(dim.label()))
                    .into_any_element()
            })
            .collect();

        let summary = div()
            .mt(px(20.0))
            .px(px(16.0))
            .flex()
            .items_baseline()
            .gap(px(18.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(22.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .child(SharedString::from(fmt_tokens(self.metric.of(input, output)))),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!(
                        "{} input · {} output · {} turns",
                        fmt_tokens(input),
                        fmt_tokens(output),
                        records.len()
                    ))),
            );

        let body: AnyElement = if records.is_empty() {
            div()
                .mt(px(48.0))
                .flex()
                .flex_col()
                .items_center()
                .text_center()
                .text_color(theme.text_muted)
                .child(
                    div()
                        .text_size(crate::typography::ui_rems(14.0))
                        .child(SharedString::from(if self.loading.is_some() {
                            "Reading usage…"
                        } else {
                            "No tokens booked in this range yet"
                        })),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .child(SharedString::from(
                            "Every turn a session runs on this device is counted from now on.",
                        )),
                )
                .into_any_element()
        } else {
            div()
                .mt(px(12.0))
                .py(px(6.0))
                .rounded(px(12.0))
                .bg(widgets::block_fill(&theme))
                .flex()
                .flex_col()
                .children(rows)
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "usage-page-scrollbar", &theme, cx);
        div()
            .id("usage-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("usage-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "Usage", None))
                                .child(widgets::page_subtitle(
                                    &theme,
                                    "Tokens your sessions used on this device. Pick how to group them; each level nests under the one before.",
                                ))
                                .child(
                                    div()
                                        .mt(px(20.0))
                                        .px(px(8.0))
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(16.0))
                                        .child(div().flex().gap(px(2.0)).children(ranges))
                                        .child(div().flex().gap(px(2.0)).children(metrics))
                                        .child(div().flex_1())
                                        .child(
                                            widgets::ghost_action(&theme)
                                                .id("usage-refresh")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.loaded_range = None;
                                                    cx.notify();
                                                }))
                                                .child("Refresh"),
                                        ),
                                )
                                .child(
                                    div()
                                        .mt(px(14.0))
                                        .px(px(8.0))
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(
                                            div()
                                                .mr(px(4.0))
                                                .text_size(crate::typography::ui_rems(12.0))
                                                .text_color(theme.text_muted)
                                                .child("Group by"),
                                        )
                                        .children(dims),
                                )
                                .child(summary)
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(chat: &str, harness: HarnessId, model: &str, input: u64, output: u64) -> UsageRecord {
        UsageRecord {
            at: 0,
            chat_id: chat.into(),
            device_id: "d".into(),
            harness,
            model: Some(model.into()),
            input_tokens: input,
            output_tokens: output,
        }
    }

    #[test]
    fn records_nest_by_the_chosen_dimensions_largest_first() {
        let records = [
            rec("a", HarnessId::ClaudeCode, "opus", 100, 10),
            rec("b", HarnessId::Codex, "gpt", 500, 50),
            rec("a", HarnessId::ClaudeCode, "sonnet", 40, 4),
        ];
        let refs: Vec<&UsageRecord> = records.iter().collect();
        let label = |r: &UsageRecord, d: Dim| match d {
            Dim::Session => r.chat_id.clone(),
            Dim::Model => r.model.clone().unwrap_or_default(),
            Dim::Cli => harness_name(r.harness),
            _ => String::new(),
        };
        let groups = group_records(&refs, &[Dim::Cli, Dim::Model], Metric::Total, &label);
        assert_eq!(groups[0].label, "Codex");
        assert_eq!(groups[0].input + groups[0].output, 550);
        let claude = &groups[1];
        assert_eq!((claude.input, claude.output, claude.turns), (140, 14, 2));
        assert_eq!(claude.children.len(), 2);
        assert_eq!(claude.children[0].label, "opus");
        // By output alone the order holds; one level stays flat.
        let flat = group_records(&refs, &[Dim::Session], Metric::Output, &label);
        assert_eq!(flat[0].label, "b");
        assert!(flat[0].children.is_empty());
    }

    #[test]
    fn token_counts_read_short() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(12_340), "12.3k");
        assert_eq!(fmt_tokens(4_560_000), "4.56M");
        assert_eq!(fmt_tokens(1_200_000_000), "1.20B");
    }
}
