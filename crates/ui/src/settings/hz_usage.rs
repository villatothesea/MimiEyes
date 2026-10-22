//! MimiEyes (VENDORED.md #3, #10): Settings → Usage — tokens your sessions
//! used. Like ccusage and the OpenAI / Cursor dashboards: headline tiles,
//! a line chart over time (per hour for Today, per day, per week for long
//! ranges) with a line per series of the first grouping, and an interactive
//! table —
//! sortable columns, rows nesting by any ordered mix of project, tab,
//! session, CLI, model and day, each top row wearing its chart colour.
//! Project, CLI and model filters narrow everything at once. Hovering the
//! line chart reads a day out; hovering a row or legend entry lifts its
//! series.
//!
//! The engine books each turn's tokens (`zeron_engine::hz_usage`, `HzUsage`);
//! this page reads them for the chosen range and groups them here. Projects
//! and titles resolve from the current chat rows; a session's tab is the view
//! tab it is in now (`set_tabs`, fed by the shell).

use std::collections::{HashMap, HashSet};

use chrono::{Datelike as _, Timelike as _};
use gpui::{
    AnyElement, Context, Entity, MouseButton, MouseDownEvent, Pixels, Point, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
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

/// A table column (the sort key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Name,
    Input,
    Output,
    Total,
    Turns,
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

/// Re-sort every level by a table column (`desc`: largest / Z first).
pub fn sort_groups(groups: &mut [Group], col: Col, desc: bool, metric: Metric) {
    groups.sort_by(|a, b| {
        let order = match col {
            Col::Name => a.label.cmp(&b.label),
            Col::Input => a.input.cmp(&b.input),
            Col::Output => a.output.cmp(&b.output),
            Col::Total => metric.of(a.input, a.output).cmp(&metric.of(b.input, b.output)),
            Col::Turns => a.turns.cmp(&b.turns),
        };
        if desc { order.reverse() } else { order }
    });
    for group in groups {
        sort_groups(&mut group.children, col, desc, metric);
    }
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

/// The categorical palette (fixed order, validated for adjacent stacks in
/// both modes — the dataviz reference instance); light and dark steps.
const SERIES_LIGHT: [u32; 8] = [
    0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x6250d6, 0xe34948,
];
const SERIES_DARK: [u32; 8] = [
    0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x008300, 0x9085e9, 0xe66767,
];
/// Named series in the chart; the rest fold into "Other".
const MAX_SERIES: usize = 6;
const OTHER: &str = "Other";
const CHART_H: f32 = 160.0;
/// The dimensions the page filters by.
const FILTERS: [Dim; 3] = [Dim::Project, Dim::Cli, Dim::Model];

/// The chart's time buckets for a range: (start of each bucket, its label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Hour,
    Day,
    Week,
}

/// Bucket starts (local) covering `range` up to `now`, and the step.
fn buckets(
    range: Range,
    first: Option<i64>,
    now: chrono::DateTime<chrono::Local>,
) -> (Vec<chrono::DateTime<chrono::Local>>, Step) {
    let midnight = |d: chrono::NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
    };
    let today = now.date_naive();
    match range {
        Range::Today => {
            let start = midnight(today);
            let hours = (0..24)
                .filter_map(|h| start.map(|s| s + chrono::Duration::hours(h)))
                .collect();
            (hours, Step::Hour)
        }
        Range::Week | Range::Month => {
            let n = if range == Range::Week { 7 } else { 30 };
            let days = (0..n)
                .rev()
                .filter_map(|back| midnight(today - chrono::Duration::days(back)))
                .collect();
            (days, Step::Day)
        }
        Range::All => {
            let first_day = first
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|t| t.with_timezone(&chrono::Local).date_naive())
                .unwrap_or(today)
                .min(today);
            let span = (today - first_day).num_days() + 1;
            if span <= 45 {
                let days = (0..span)
                    .filter_map(|d| midnight(first_day + chrono::Duration::days(d)))
                    .collect();
                (days, Step::Day)
            } else {
                let monday = |d: chrono::NaiveDate| {
                    d - chrono::Duration::days(d.weekday().num_days_from_monday() as i64)
                };
                let (mut week, last) = (monday(first_day), monday(today));
                let mut weeks = Vec::new();
                while week <= last {
                    weeks.extend(midnight(week));
                    week += chrono::Duration::days(7);
                }
                (weeks, Step::Week)
            }
        }
    }
}

/// The bucket `at` (epoch ms) falls in: the last start at or before it.
fn bucket_of(starts: &[chrono::DateTime<chrono::Local>], at: i64) -> Option<usize> {
    let ix = starts.partition_point(|s| s.timestamp_millis() <= at);
    ix.checked_sub(1)
}

fn bucket_label(start: chrono::DateTime<chrono::Local>, step: Step) -> String {
    match step {
        Step::Hour => format!("{:02}:00", start.hour()),
        Step::Day => start.format("%a %m/%d").to_string(),
        Step::Week => format!("Week of {}", start.format("%m/%d")),
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
    sort: Option<(Col, bool)>,
    /// Open groups, by their path of labels.
    open: HashSet<String>,
    /// chat id → "Project › Tab" for sessions in a view tab now.
    tabs: HashMap<String, String>,
    /// A series keeps its colour while the page lives (colour follows the
    /// entity, never its rank).
    slots: HashMap<String, usize>,
    hovered_bucket: Option<usize>,
    hovered_series: Option<String>,
    /// Project / CLI / model filters: the value kept (absent = all).
    filters: HashMap<Dim, String>,
    /// A filter's menu: which one, where it opened.
    filter_menu: Option<(Dim, Point<Pixels>)>,
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
            sort: None,
            open: HashSet::new(),
            tabs: HashMap::new(),
            slots: HashMap::new(),
            hovered_bucket: None,
            hovered_series: None,
            filters: HashMap::new(),
            filter_menu: None,
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
        self.hovered_series = None;
        cx.notify();
    }

    /// A header click: sort by it; again flips the direction; a third time
    /// returns to the natural order (by size, days by date).
    fn toggle_sort(&mut self, col: Col, cx: &mut Context<Self>) {
        self.sort = match self.sort {
            Some((c, desc)) if c == col && desc != (col == Col::Name) => Some((col, !desc)),
            Some((c, _)) if c == col => None,
            _ => Some((col, col != Col::Name)),
        };
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    fn set_filter(&mut self, dim: Dim, value: Option<String>, cx: &mut Context<Self>) {
        match value {
            Some(value) => self.filters.insert(dim, value),
            None => self.filters.remove(&dim),
        };
        self.filter_menu = None;
        self.open.clear();
        self.hovered_bucket = None;
        self.hovered_series = None;
        cx.notify();
    }

    /// A filter chip: "Project: All" / "CLI: Claude Code"; a click opens its
    /// menu.
    fn filter_chip(&self, dim: Dim, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let value = self.filters.get(&dim);
        let on = value.is_some();
        div()
            .id(SharedString::from(format!("usage-filter-{}", dim.label())))
            .max_w(px(220.0))
            .px(px(10.0))
            .h(px(26.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .border_1()
            .border_color(if on { theme.accent.opacity(0.6) } else { theme.border })
            .when(on, |el| el.bg(theme.accent.opacity(0.12)))
            .hover(|st| st.bg(theme.wash(0.05)))
            .text_size(crate::typography::ui_rems(12.5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.filter_menu = match this.filter_menu {
                        Some((open, _)) if open == dim => None,
                        _ => Some((dim, event.position)),
                    };
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme.text_muted)
                    .child(SharedString::from(format!("{}:", dim.label()))),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.text)
                    .child(SharedString::from(value.cloned().unwrap_or_else(|| "All".into()))),
            )
            .child(
                crate::icons::icon(crate::icons::ALT_ARROW_RIGHT)
                    .size(px(10.0))
                    .text_color(theme.text_faint)
                    .with_transformation(gpui::Transformation::rotate(gpui::percentage(0.25))),
            )
            .into_any_element()
    }

    /// The open filter's menu: All, then every value in range, largest first.
    fn filter_menu(&self, options: &HashMap<Dim, Vec<(String, u64)>>, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (dim, position) = self.filter_menu?;
        let theme = Theme::of(cx).for_popup();
        let current = self.filters.get(&dim).cloned();
        let mut menu = popover::popover_card(&theme)
            .w(px(260.0))
            .max_h(px(360.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.filter_menu = None;
                cx.notify();
            }))
            .flex()
            .flex_col()
            .child(
                popover::menu_row(&theme, current.is_none(), "usage-filter-all")
                    .id("usage-filter-all")
                    .on_click(cx.listener(move |this, _, _, cx| this.set_filter(dim, None, cx)))
                    .child(div().flex_1().child(SharedString::from(format!("All {}", match dim {
                        Dim::Project => "projects",
                        Dim::Cli => "CLIs",
                        _ => "models",
                    })))),
            );
        let values = options.get(&dim).cloned().unwrap_or_default();
        if !values.is_empty() {
            menu = menu.child(popover::menu_separator());
        }
        let mut list = div().id("usage-filter-values").flex().flex_col().overflow_y_scroll();
        for (ix, (value, tokens)) in values.into_iter().enumerate() {
            let pick = value.clone();
            list = list.child(
                popover::menu_row(&theme, current.as_deref() == Some(value.as_str()), format!("usage-filter-{ix}"))
                    .id(("usage-filter-value", ix))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_filter(dim, Some(pick.clone()), cx)
                    }))
                    .child(div().flex_1().min_w_0().truncate().child(SharedString::from(value)))
                    .child(
                        div()
                            .flex_none()
                            .text_size(crate::typography::ui_rems(11.5))
                            .text_color(theme.text_faint)
                            .child(SharedString::from(fmt_tokens(tokens))),
                    ),
            );
        }
        menu = menu.child(list);
        Some(popover::menu_at("usage-filter-menu", position, menu.into_any_element(), None))
    }

    /// The colour of `series` (`None`: the folded "Other", a neutral).
    fn series_color(&mut self, series: &str, theme: &Theme) -> gpui::Hsla {
        let palette = if theme.appearance.is_dark() { SERIES_DARK } else { SERIES_LIGHT };
        if series == OTHER {
            return theme.text_faint.opacity(0.55);
        }
        let slot = match self.slots.get(series) {
            Some(slot) => *slot,
            None => {
                let used: HashSet<usize> = self.slots.values().copied().collect();
                let slot = (0..palette.len()).find(|s| !used.contains(s)).unwrap_or_else(|| {
                    // Every colour is taken: start over (seen series repaint
                    // only once eight others have come and gone).
                    self.slots.clear();
                    0
                });
                self.slots.insert(series.to_owned(), slot);
                slot
            }
        };
        gpui::rgb(palette[slot]).into()
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

    /// A headline tile.
    fn tile(theme: &Theme, label: &str, value: String, sub: Option<String>) -> AnyElement {
        div()
            .flex_1()
            .min_w(px(120.0))
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(12.0))
            .bg(widgets::block_fill(theme))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(
                div()
                    .text_size(crate::typography::ui_rems(12.0))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(label.to_owned())),
            )
            .child(
                div()
                    .text_size(crate::typography::ui_rems(20.0))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                    .truncate()
                    .child(SharedString::from(value)),
            )
            .children(sub.map(|sub| {
                div()
                    .text_size(crate::typography::ui_rems(11.5))
                    .text_color(theme.text_faint)
                    .truncate()
                    .child(SharedString::from(sub))
            }))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_rows(
        &mut self,
        groups: &[Group],
        depth: usize,
        path: &str,
        parent_total: u64,
        swatches: bool,
        out: &mut Vec<AnyElement>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) {
        for group in groups {
            let key = format!("{path}/{}", group.label);
            let open = self.open.contains(&key);
            let has_children = !group.children.is_empty();
            let value = self.metric.of(group.input, group.output);
            let share = if parent_total == 0 { 0.0 } else { value as f32 / parent_total as f32 };
            let toggle = key.clone();
            let swatch = (swatches && depth == 0).then(|| {
                let color = self.series_color(&group.label, theme);
                div().flex_none().size(px(8.0)).rounded(px(2.0)).bg(color)
            });
            let hover_label = (depth == 0).then(|| group.label.clone());
            let lifted = depth == 0 && self.hovered_series.as_deref() == Some(group.label.as_str());
            let num = |text: String, w: f32, strong: bool| {
                div()
                    .flex_none()
                    .w(px(w))
                    .flex()
                    .justify_end()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(if strong { theme.text } else { theme.text_muted })
                    .when(strong, |el| el.font_weight(gpui::FontWeight::MEDIUM))
                    .child(SharedString::from(text))
            };
            out.push(
                div()
                    .id(SharedString::from(format!("usage-row-{key}")))
                    .px(px(12.0))
                    .h(px(34.0))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text)
                    .when(depth == 0 && !out.is_empty(), |el| {
                        el.border_t_1().border_color(widgets::row_divider(theme))
                    })
                    .when(lifted, |el| el.bg(theme.wash(0.05)))
                    .hover(|st| st.bg(theme.wash(0.04)))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        let next = if *hovered { hover_label.clone() } else { None };
                        if hover_label.is_some() && this.hovered_series != next {
                            this.hovered_series = next;
                            cx.notify();
                        }
                    }))
                    .when(has_children, |el| {
                        el.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                            if !this.open.remove(&toggle) {
                                this.open.insert(toggle.clone());
                            }
                            cx.notify();
                        }))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.0))
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
                            .children(swatch)
                            .child(div().min_w_0().truncate().child(SharedString::from(group.label.clone()))),
                    )
                    .child(num(fmt_tokens(group.input), 72.0, false))
                    .child(num(fmt_tokens(group.output), 72.0, false))
                    .child(num(fmt_tokens(group.input + group.output), 72.0, true))
                    .child(num(group.turns.to_string(), 52.0, false))
                    .child(
                        div()
                            .flex_none()
                            .w(px(96.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(4.0))
                                    .rounded_full()
                                    .bg(theme.wash(0.06))
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .w(gpui::relative(share.clamp(0.0, 1.0)))
                                            .bg(theme.text_muted.opacity(0.6)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(34.0))
                                    .flex()
                                    .justify_end()
                                    .text_size(crate::typography::ui_rems(11.5))
                                    .text_color(theme.text_faint)
                                    .child(SharedString::from(format!("{:.0}%", share * 100.0))),
                            ),
                    )
                    .into_any_element(),
            );
            if open && has_children {
                self.render_rows(&group.children, depth + 1, &key, value, false, out, theme, cx);
            }
        }
    }

    /// The line chart: one 2px line per series of the first grouping (the
    /// largest six, then "Other") over the range's buckets. Hovering a
    /// bucket draws a crosshair with a marker on every line and reads the
    /// bucket out above; a lifted series (hovered row or legend entry) stays
    /// strong while the rest recede.
    #[allow(clippy::too_many_arguments)]
    fn render_chart(
        &mut self,
        records: &[UsageRecord],
        series_of: &dyn Fn(usize) -> String,
        series: &[String],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = chrono::Local::now();
        let first = records.iter().map(|r| r.at).min();
        let (starts, step) = buckets(self.range, first, now);
        // values[series][bucket]
        let mut values = vec![vec![0u64; starts.len()]; series.len()];
        for (ix, record) in records.iter().enumerate() {
            let Some(bucket) = bucket_of(&starts, record.at) else {
                continue;
            };
            let name = series_of(ix);
            let slot = series
                .iter()
                .position(|s| *s == name)
                .or_else(|| series.iter().position(|s| s == OTHER));
            if let Some(slot) = slot {
                values[slot][bucket] += self.metric.of(record.input_tokens, record.output_tokens);
            }
        }
        let max = values.iter().flatten().copied().max().unwrap_or(0);
        let colors: Vec<gpui::Hsla> = series.iter().map(|s| self.series_color(s, theme)).collect();
        let lifted = self.hovered_series.clone();
        let hovered = self.hovered_bucket.filter(|b| *b < starts.len());

        // The readout: the hovered bucket, else the range's peak.
        let readout = match hovered {
            Some(bucket) => {
                let total: u64 = values.iter().map(|v| v[bucket]).sum();
                let mut parts: Vec<String> = series
                    .iter()
                    .zip(&values)
                    .filter(|(_, v)| v[bucket] > 0)
                    .map(|(s, v)| format!("{s} {}", fmt_tokens(v[bucket])))
                    .collect();
                if parts.len() > 4 {
                    parts.truncate(4);
                    parts.push("…".into());
                }
                format!(
                    "{} · {}{}",
                    bucket_label(starts[bucket], step),
                    fmt_tokens(total),
                    if parts.is_empty() { String::new() } else { format!(" — {}", parts.join(" · ")) }
                )
            }
            None => format!(
                "Peak {} per {}",
                fmt_tokens(max),
                match step {
                    Step::Hour => "hour",
                    Step::Day => "day",
                    Step::Week => "week",
                }
            ),
        };

        // The plot: recessive grid (top, middle, base), the lines, then the
        // crosshair and markers over them.
        let grid = theme.border.opacity(0.45);
        let ring = theme.surface_raised;
        let count = starts.len();
        let lines: Vec<(Vec<u64>, gpui::Hsla, bool)> = values
            .iter()
            .zip(&colors)
            .zip(series)
            .map(|((v, c), name)| {
                let dim = lifted.as_ref().is_some_and(|l| l != name);
                (v.clone(), *c, dim)
            })
            .collect();
        let plot = gpui::canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let (x0, y0) = (bounds.origin.x, bounds.origin.y);
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let x_of = |i: usize| x0 + px(w * (i as f32 + 0.5) / count.max(1) as f32);
                let y_of = |v: u64| {
                    let f = if max == 0 { 0.0 } else { v as f32 / max as f32 };
                    y0 + px(h - 2.0 - f * (h - 4.0))
                };
                for frac in [0.0f32, 0.5, 1.0] {
                    let y = y0 + px(2.0 + frac * (h - 4.0));
                    let mut path = gpui::PathBuilder::stroke(px(1.0));
                    path.move_to(gpui::point(x0, y));
                    path.line_to(gpui::point(x0 + px(w), y));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, grid);
                    }
                }
                // Receded lines first, so the lifted one draws on top.
                let mut order: Vec<usize> = (0..lines.len()).collect();
                order.sort_by_key(|ix| !lines[*ix].2);
                for ix in order {
                    let (points, color, dim) = &lines[ix];
                    if count < 2 {
                        continue;
                    }
                    let mut path = gpui::PathBuilder::stroke(px(2.0));
                    for (i, v) in points.iter().enumerate() {
                        let p = gpui::point(x_of(i), y_of(*v));
                        if i == 0 {
                            path.move_to(p);
                        } else {
                            path.line_to(p);
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, if *dim { color.opacity(0.22) } else { *color });
                    }
                }
                if let Some(bucket) = hovered {
                    let x = x_of(bucket);
                    let mut cross = gpui::PathBuilder::stroke(px(1.0));
                    cross.move_to(gpui::point(x, y0));
                    cross.line_to(gpui::point(x, y0 + px(h)));
                    if let Ok(path) = cross.build() {
                        window.paint_path(path, grid.opacity(1.0));
                    }
                    let dot = |window: &mut Window, center: gpui::Point<gpui::Pixels>, r: f32, color| {
                        let mut path = gpui::PathBuilder::fill();
                        for k in 0..=24 {
                            let a = std::f32::consts::TAU * k as f32 / 24.0;
                            let p = gpui::point(center.x + px(r * a.cos()), center.y + px(r * a.sin()));
                            if k == 0 {
                                path.move_to(p);
                            } else {
                                path.line_to(p);
                            }
                        }
                        path.close();
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                    };
                    for (points, color, dim) in &lines {
                        if *dim {
                            continue;
                        }
                        let center = gpui::point(x, y_of(points[bucket]));
                        // An 8px marker inside a 2px surface ring.
                        dot(window, center, 6.0, ring);
                        dot(window, center, 4.0, *color);
                    }
                }
            },
        )
        .size_full();

        // Hit targets: one transparent column per bucket over the plot.
        let targets: Vec<AnyElement> = (0..count)
            .map(|bucket| {
                div()
                    .id(("usage-point", bucket))
                    .flex_1()
                    .h_full()
                    .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                        let next = if *over { Some(bucket) } else { None };
                        if *over || this.hovered_bucket == Some(bucket) {
                            this.hovered_bucket = next;
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            })
            .collect();

        let edge = |ix: usize| {
            starts
                .get(ix)
                .map(|s| bucket_label(*s, step))
                .unwrap_or_default()
        };
        let x_labels = div()
            .mt(px(6.0))
            .flex()
            .justify_between()
            .text_size(crate::typography::ui_rems(11.0))
            .text_color(theme.text_faint)
            .child(SharedString::from(edge(0)))
            .when(starts.len() > 2, |el| el.child(SharedString::from(edge(starts.len() / 2))))
            .child(SharedString::from(edge(starts.len().saturating_sub(1))));

        // A legend for two or more series (identity is never colour alone).
        let legend = (series.len() >= 2).then(|| {
            div()
                .mt(px(10.0))
                .flex()
                .flex_wrap()
                .gap(px(12.0))
                .children(series.iter().zip(&colors).enumerate().map(|(ix, (name, color))| {
                    let hover_name = name.clone();
                    div()
                        .id(("usage-legend", ix))
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .text_color(theme.text_muted)
                        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                            this.hovered_series = over.then(|| hover_name.clone());
                            cx.notify();
                        }))
                        .child(div().flex_none().w(px(12.0)).h(px(2.0)).rounded_full().bg(*color))
                        .child(div().max_w(px(180.0)).truncate().child(SharedString::from(name.clone())))
                }))
        });

        div()
            .mt(px(12.0))
            .px(px(16.0))
            .py(px(14.0))
            .rounded(px(12.0))
            .bg(widgets::block_fill(theme))
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(10.0))
                    .h(px(18.0))
                    .truncate()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(if hovered.is_some() { theme.text } else { theme.text_muted })
                    .child(SharedString::from(readout)),
            )
            .child(
                div()
                    .relative()
                    .h(px(CHART_H))
                    .child(plot)
                    .child(div().absolute().inset_0().flex().children(targets)),
            )
            .child(x_labels)
            .children(legend)
            .into_any_element()
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
        let all_labels: Vec<HashMap<Dim, String>> = self
            .records
            .iter()
            .map(|r| Dim::ALL.iter().map(|d| (*d, self.label_of(r, *d, cx))).collect())
            .collect();
        // Every filter's values in range, largest first.
        let mut options: HashMap<Dim, Vec<(String, u64)>> = HashMap::new();
        for dim in FILTERS {
            let mut sums: HashMap<String, u64> = HashMap::new();
            for (record, labels) in self.records.iter().zip(&all_labels) {
                *sums.entry(labels[&dim].clone()).or_default() +=
                    self.metric.of(record.input_tokens, record.output_tokens);
            }
            let mut list: Vec<(String, u64)> = sums.into_iter().collect();
            list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            options.insert(dim, list);
        }
        let (records, labels): (Vec<UsageRecord>, Vec<HashMap<Dim, String>>) = self
            .records
            .iter()
            .cloned()
            .zip(all_labels)
            .filter(|(_, labels)| {
                self.filters
                    .iter()
                    .all(|(dim, value)| labels.get(dim) == Some(value))
            })
            .unzip();
        let refs: Vec<&UsageRecord> = records.iter().collect();
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
        let mut groups = group_records(&refs, &self.dims, self.metric, &label);
        // The chart's series: the first grouping's largest, the rest "Other"
        // (taken before the table's own sort).
        let first_dim = self.dims[0];
        let mut series: Vec<String> = groups.iter().take(MAX_SERIES).map(|g| g.label.clone()).collect();
        if first_dim == Dim::Day {
            // Days are the time axis already: draw a line per CLI.
            series.clear();
        }
        let breakdown = if first_dim == Dim::Day { Dim::Cli } else { first_dim };
        if series.is_empty() {
            let by: Vec<Group> = group_records(&refs, &[breakdown], self.metric, &label);
            series = by.iter().take(MAX_SERIES).map(|g| g.label.clone()).collect();
            if by.len() > MAX_SERIES {
                series.push(OTHER.into());
            }
        } else if groups.len() > MAX_SERIES {
            series.push(OTHER.into());
        }
        if let Some((col, desc)) = self.sort {
            sort_groups(&mut groups, col, desc, self.metric);
        }

        let input: u64 = records.iter().map(|r| r.input_tokens).sum();
        let output: u64 = records.iter().map(|r| r.output_tokens).sum();
        let total = self.metric.of(input, output);
        let sessions: HashSet<&str> = records.iter().map(|r| r.chat_id.as_str()).collect();
        let busiest = {
            let mut by_day: HashMap<String, u64> = HashMap::new();
            for r in &records {
                *by_day.entry(label(r, Dim::Day)).or_default() +=
                    self.metric.of(r.input_tokens, r.output_tokens);
            }
            by_day.into_iter().max_by_key(|(_, v)| *v)
        };

        let mut rows = Vec::new();
        let swatches = first_dim != Dim::Day;
        self.render_rows(&groups, 0, "", total, swatches, &mut rows, &theme, cx);

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
                this.hovered_bucket = None;
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

        let header_cell = |id: &'static str, label: String, col: Col, w: Option<f32>, cx: &mut Context<Self>| {
            let arrow = match self.sort {
                Some((c, desc)) if c == col => if desc { " ↓" } else { " ↑" },
                _ => "",
            };
            div()
                .id(id)
                .when_some(w, |el, w| el.flex_none().w(px(w)).flex().justify_end())
                .when(w.is_none(), |el| el.flex_1().min_w_0().pl(px(20.0)))
                .cursor_pointer()
                .text_color(if arrow.is_empty() { theme.text_faint } else { theme.text })
                .hover(|st| st.text_color(theme.text))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_sort(col, cx)))
                .child(SharedString::from(format!("{label}{arrow}")))
                .into_any_element()
        };
        let table_header = div()
            .px(px(12.0))
            .h(px(32.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .text_size(crate::typography::ui_rems(11.5))
            .child(header_cell(
                "usage-col-name",
                self.dims.iter().map(|d| d.label()).collect::<Vec<_>>().join(" › "),
                Col::Name,
                None,
                cx,
            ))
            .child(header_cell("usage-col-input", "Input".into(), Col::Input, Some(72.0), cx))
            .child(header_cell("usage-col-output", "Output".into(), Col::Output, Some(72.0), cx))
            .child(header_cell("usage-col-total", "Total".into(), Col::Total, Some(72.0), cx))
            .child(header_cell("usage-col-turns", "Turns".into(), Col::Turns, Some(52.0), cx))
            .child(
                div()
                    .flex_none()
                    .w(px(96.0))
                    .flex()
                    .justify_end()
                    .text_color(theme.text_faint)
                    .child("Share"),
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
                    crate::icons::icon(crate::icons::CHART)
                        .size(px(28.0))
                        .text_color(theme.text_muted.opacity(0.25)),
                )
                .child(
                    div()
                        .mt(px(12.0))
                        .text_size(crate::typography::ui_rems(14.0))
                        .child(SharedString::from(if self.loading.is_some() {
                            "Reading usage…"
                        } else {
                            if self.filters.is_empty() {
                                "No tokens recorded in this range yet"
                            } else {
                                "Nothing matches these filters"
                            }
                        })),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .child(SharedString::from(
                            "Every turn a session runs on this device is counted.",
                        )),
                )
                .into_any_element()
        } else {
            let series_label = |ix: usize| -> String {
                labels
                    .get(ix)
                    .and_then(|l| l.get(&breakdown).cloned())
                    .unwrap_or_default()
            };
            let chart = self.render_chart(&records, &series_label, &series, &theme, cx);
            let tiles = div()
                .mt(px(20.0))
                .flex()
                .flex_wrap()
                .gap(px(10.0))
                .child(Self::tile(
                    &theme,
                    match self.metric {
                        Metric::Total => "Tokens",
                        Metric::Input => "Input tokens",
                        Metric::Output => "Output tokens",
                    },
                    fmt_tokens(total),
                    Some(format!("{} in · {} out", fmt_tokens(input), fmt_tokens(output))),
                ))
                .child(Self::tile(&theme, "Turns", records.len().to_string(), None))
                .child(Self::tile(&theme, "Sessions", sessions.len().to_string(), None))
                .child(Self::tile(
                    &theme,
                    "Busiest day",
                    busiest.as_ref().map(|(_, v)| fmt_tokens(*v)).unwrap_or_default(),
                    busiest.as_ref().map(|(day, _)| day.clone()),
                ));
            div()
                .flex()
                .flex_col()
                .child(tiles)
                .child(widgets::section(
                    &theme,
                    format!(
                        "{} by {}",
                        match self.range {
                            Range::Today => "Tokens per hour",
                            Range::All => "Tokens over time",
                            _ => "Tokens per day",
                        },
                        breakdown.label().to_lowercase()
                    ),
                    chart,
                ))
                .child(
                    div()
                        .mt(px(32.0))
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.0))
                                .child(widgets::section_label(&theme, "Group by"))
                                .children(dims),
                        )
                        .child(
                            div()
                                .py(px(4.0))
                                .rounded(px(12.0))
                                .bg(widgets::block_fill(&theme))
                                .flex()
                                .flex_col()
                                .child(table_header)
                                .child(div().h(px(1.0)).bg(widgets::row_divider(&theme)))
                                .children(rows),
                        ),
                )
                .into_any_element()
        };

        let any_filter = !self.filters.is_empty();
        let chips: Vec<AnyElement> = FILTERS.iter().map(|dim| self.filter_chip(*dim, &theme, cx)).collect();
        let filter_row = div()
            .mt(px(10.0))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .children(chips)
            .when(any_filter, |el| {
                el.child(
                    widgets::ghost_action(&theme)
                        .id("usage-filter-clear")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.filters.clear();
                            this.open.clear();
                            cx.notify();
                        }))
                        .child("Clear filters"),
                )
            });
        let filter_menu = self.filter_menu(&options, cx);
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
                                    "Tokens your sessions used on this device. Filter by project, CLI or model; hover the chart to read a day; sort and group the table any way.",
                                ))
                                .child(
                                    div()
                                        .mt(px(20.0))
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
                                .child(filter_row)
                                .child(body),
                        ),
                )
                .fade_overflow_y(&self.scroll.scroll),
            )
            .children(scrollbar)
            .children(filter_menu)
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
    fn table_columns_sort_every_level() {
        let records = [
            rec("a", HarnessId::ClaudeCode, "opus", 100, 10),
            rec("b", HarnessId::Codex, "gpt", 500, 50),
            rec("a", HarnessId::ClaudeCode, "sonnet", 40, 400),
        ];
        let refs: Vec<&UsageRecord> = records.iter().collect();
        let label = |r: &UsageRecord, d: Dim| match d {
            Dim::Model => r.model.clone().unwrap_or_default(),
            _ => harness_name(r.harness),
        };
        let mut groups = group_records(&refs, &[Dim::Cli, Dim::Model], Metric::Total, &label);
        sort_groups(&mut groups, Col::Output, true, Metric::Total);
        assert_eq!(groups[0].label, "Claude Code");
        assert_eq!(groups[0].children[0].label, "sonnet");
        sort_groups(&mut groups, Col::Name, false, Metric::Total);
        assert_eq!(groups[0].label, "Claude Code");
        assert_eq!(groups[0].children[0].label, "opus");
        sort_groups(&mut groups, Col::Turns, false, Metric::Total);
        assert_eq!(groups[0].label, "Codex");
    }

    #[test]
    fn chart_buckets_cover_the_range_and_place_each_turn() {
        let now = chrono::Local::now();
        let (hours, step) = buckets(Range::Today, None, now);
        assert_eq!((hours.len(), step), (24, Step::Hour));
        let (days, step) = buckets(Range::Week, None, now);
        assert_eq!((days.len(), step), (7, Step::Day));
        assert_eq!(bucket_of(&days, now.timestamp_millis()), Some(6));
        assert_eq!(bucket_of(&days, days[0].timestamp_millis() - 1), None);
        let long_ago = (now - chrono::Duration::days(120)).timestamp_millis();
        let (weeks, step) = buckets(Range::All, Some(long_ago), now);
        assert_eq!(step, Step::Week);
        assert!((17..=19).contains(&weeks.len()));
        assert_eq!(bucket_of(&weeks, long_ago), Some(0));
    }

    #[test]
    fn token_counts_read_short() {
        assert_eq!(fmt_tokens(950), "950");
        assert_eq!(fmt_tokens(12_340), "12.3k");
        assert_eq!(fmt_tokens(4_560_000), "4.56M");
        assert_eq!(fmt_tokens(1_200_000_000), "1.20B");
    }
}
