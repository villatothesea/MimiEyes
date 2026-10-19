//! MimiEyes (VENDORED.md #7): Settings → Activity — how each MimiEyes run
//! ended, as the activity monitor booked it (`crate::hz_activity`): when it
//! started and stopped, how long it ran and slept, and why it ended.

use std::path::PathBuf;

use gpui::{AnyElement, Context, SharedString, Window, div, prelude::*, px};

use crate::hz_activity::{self, Run};
use crate::popover;
use crate::settings::widgets;
use crate::theme::Theme;

pub struct RunsPage {
    data_dir: PathBuf,
    runs: Vec<Run>,
    scroll: widgets::PageScroll,
}

impl RunsPage {
    pub fn new(data_dir: PathBuf, _cx: &mut Context<Self>) -> Self {
        let runs = hz_activity::read_runs(&data_dir);
        Self {
            data_dir,
            runs,
            scroll: widgets::PageScroll::default(),
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.runs = hz_activity::read_runs(&self.data_dir);
        cx.notify();
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }
}

/// "2h 47m", "33m", "40s".
fn span(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3_600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60),
    }
}

fn local(ms: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| t.with_timezone(&chrono::Local).format(format).to_string())
        .unwrap_or_default()
}

impl popover::ScrollRailHost for RunsPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for RunsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let rows: Vec<AnyElement> = self
            .runs
            .iter()
            .enumerate()
            .map(|(ix, run)| {
                // Green: closed or quit; amber: ended from outside or by
                // Windows; red: a crash; accent: the run going now.
                let tone = match (run.ended, run.exit_code) {
                    (None, _) if run.reason == "Running now" => theme.accent,
                    (None, _) => theme.warning,
                    (Some(_), Some(code)) if code >= 0xC000_0000 => theme.danger,
                    (Some(_), _)
                        if run.reason.starts_with("Window closed")
                            || run.reason.starts_with("Quit")
                            || run.reason.starts_with("Exited normally") =>
                    {
                        theme.success
                    }
                    _ => theme.warning,
                };
                let mut meta = vec![format!("Started {}", local(run.started, "%Y-%m-%d %H:%M"))];
                if let Some(ended) = run.ended {
                    meta.push(format!("ended {}", local(ended, "%H:%M:%S")));
                    meta.push(format!(
                        "ran {}",
                        span(((ended - run.started).max(0) / 1_000) as u64)
                    ));
                }
                if run.slept_secs > 0 {
                    meta.push(format!("PC slept {}", span(run.slept_secs)));
                }
                if let Some(code) = run.exit_code {
                    meta.push(format!("exit 0x{code:08X}"));
                }
                if !run.version.is_empty() {
                    meta.push(format!("v{}", run.version));
                }
                div()
                    .id(("hz-run", ix))
                    .flex()
                    .items_start()
                    .gap(px(12.0))
                    .rounded(px(8.0))
                    .px(px(12.0))
                    .py(px(10.0))
                    .hover(|s| s.bg(crate::theme::ink(0.03)))
                    .child(
                        div()
                            .flex_none()
                            .mt(px(5.0))
                            .size(px(8.0))
                            .rounded(px(2.0))
                            .bg(tone),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(13.5))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(theme.text)
                                    .child(SharedString::from(run.reason.clone())),
                            )
                            .child(
                                div()
                                    .text_size(crate::typography::ui_rems(12.0))
                                    .text_color(theme.text_muted)
                                    .child(SharedString::from(meta.join(" · "))),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        let body: AnyElement = if rows.is_empty() {
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
                        .child(SharedString::from("No runs recorded yet")),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(crate::typography::ui_rems(12.0))
                        .child(SharedString::from(
                            "Each run is booked from the moment it starts; how it ends shows here.",
                        )),
                )
                .into_any_element()
        } else {
            div()
                .mt(px(20.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .children(rows)
                .into_any_element()
        };

        let scrollbar = popover::rail(self, "runs-page-scrollbar", &theme, cx);
        div()
            .id("runs-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("runs-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(
                                    div()
                                        .flex()
                                        .items_end()
                                        .gap(px(16.0))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .child(widgets::page_header(
                                                    &theme,
                                                    "Activity",
                                                    (!self.runs.is_empty()).then_some(self.runs.len()),
                                                ))
                                                .child(widgets::page_subtitle(
                                                    &theme,
                                                    "How each MimiEyes run ended — booked by a small watcher that starts with the app, so closes, crashes and kills all show.",
                                                )),
                                        )
                                        .child(
                                            widgets::ghost_action(&theme)
                                                .id("runs-refresh")
                                                .flex_none()
                                                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
                                                .child("Refresh"),
                                        ),
                                )
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

    #[test]
    fn spans_read_short() {
        assert_eq!(span(40), "40s");
        assert_eq!(span(33 * 60), "33m");
        assert_eq!(span(2 * 3_600 + 47 * 60), "2h 47m");
    }
}
