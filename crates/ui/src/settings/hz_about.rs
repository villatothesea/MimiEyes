//! MimiEyes (VENDORED.md #14): Settings → About — what this app is, which
//! Zeron it is built on (the pin and patch count read from `VENDORED.md`,
//! embedded at build time so they never go stale), and where it keeps its
//! data, logs and executable, each one click from Explorer.

use std::path::PathBuf;

use gpui::{AnyElement, Context, SharedString, Window, div, prelude::*, px};

use crate::popover;
use crate::settings::widgets;
use crate::theme::Theme;

/// The patch log, as built.
const VENDORED: &str = include_str!("../../../../VENDORED.md");
const UPSTREAM: &str = "https://github.com/zeronsh/zeron";

/// "upstream main @ 3b2f7802 (past v0.2.113)" from the log's `Pinned:` line.
pub fn upstream_pin(log: &str) -> Option<String> {
    let start = log.find("Pinned:")? + "Pinned:".len();
    let rest = &log[start..];
    let end = rest.find(". ").or_else(|| rest.find(".\n")).unwrap_or(rest.len());
    let pin = rest[..end].replace('`', "");
    Some(pin.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// How many local patches the log lists (its numbered table rows).
pub fn patch_count(log: &str) -> usize {
    log.lines()
        .filter(|line| {
            line.strip_prefix("| ")
                .and_then(|rest| rest.split(' ').next())
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .count()
}

pub struct AboutPage {
    data_dir: PathBuf,
    scroll: widgets::PageScroll,
}

impl AboutPage {
    pub fn new(data_dir: PathBuf, _cx: &mut Context<Self>) -> Self {
        Self {
            data_dir,
            scroll: widgets::PageScroll::default(),
        }
    }

    fn on_scroll_hovered(&mut self, hovered: &bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.scroll.set_list_hovered(*hovered) {
            cx.notify();
        }
    }

    /// One row of the card: a label, its value, and an optional action.
    fn row(
        ix: usize,
        label: &str,
        value: String,
        action: Option<(&'static str, Box<dyn Fn(&mut gpui::App) + 'static>)>,
        theme: &Theme,
    ) -> AnyElement {
        div()
            .id(("about-row", ix))
            .px(px(14.0))
            .min_h(px(44.0))
            .py(px(8.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .when(ix > 0, |el| el.border_t_1().border_color(widgets::row_divider(theme)))
            .child(
                div()
                    .flex_none()
                    .w(px(120.0))
                    .text_size(crate::typography::ui_rems(13.0))
                    .text_color(theme.text)
                    .child(SharedString::from(label.to_owned())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::typography::ui_rems(12.5))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(value)),
            )
            .children(action.map(|(name, act)| {
                widgets::ghost_action(theme)
                    .id(("about-action", ix))
                    .flex_none()
                    .on_click(move |_, _, cx| act(cx))
                    .child(name)
            }))
            .into_any_element()
    }
}

impl popover::ScrollRailHost for AboutPage {
    fn rail_bar(&mut self) -> &mut popover::MenuScrollbarState {
        self.scroll.rail_bar()
    }

    fn rail_scroll(&self) -> Option<gpui::ScrollHandle> {
        self.scroll.rail_scroll()
    }
}

impl Render for AboutPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).for_settings_surface();
        let version = env!("CARGO_PKG_VERSION");
        let pin = upstream_pin(VENDORED).unwrap_or_else(|| format!("v{version}"));
        let patches = patch_count(VENDORED);
        let data = self.data_dir.clone();
        let logs = self.data_dir.join("logs");
        let exe = std::env::current_exe().ok();
        let exe_dir = exe.as_ref().and_then(|e| e.parent().map(PathBuf::from));
        let open = |path: PathBuf| -> Box<dyn Fn(&mut gpui::App)> {
            Box::new(move |cx: &mut gpui::App| cx.open_with_system(&path))
        };

        let hero = div()
            .mt(px(24.0))
            .flex()
            .items_center()
            .gap(px(18.0))
            .child(crate::hz_splash::spider_loader("about-spider", 64.0, theme.accent))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(22.0))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.text)
                            .child("MimiEyes"),
                    )
                    .child(
                        div()
                            .text_size(crate::typography::ui_rems(13.0))
                            .text_color(theme.text_muted)
                            .child(SharedString::from(format!("Version {version}"))),
                    )
                    .child(
                        div()
                            .mt(px(4.0))
                            .max_w(px(460.0))
                            .text_size(crate::typography::ui_rems(12.5))
                            .text_color(theme.text_muted)
                            .child(
                                "Zeron with panes, a project tree, agent teams, usage stats and an \
                                 activity monitor. Sessions, CLIs and accounts work as in Zeron.",
                            ),
                    ),
            );

        let mut rows = vec![
            Self::row(0, "Version", version.to_owned(), None, &theme),
            Self::row(
                1,
                "Built on",
                format!("Zeron, {pin}"),
                Some(("GitHub", Box::new(|cx: &mut gpui::App| cx.open_url(UPSTREAM)))),
                &theme,
            ),
            Self::row(
                2,
                "Changes",
                format!("{patches} local patches over Zeron (VENDORED.md)"),
                None,
                &theme,
            ),
            Self::row(3, "Data", data.display().to_string(), Some(("Open", open(data.clone()))), &theme),
            Self::row(4, "Logs", logs.display().to_string(), Some(("Open", open(logs.clone()))), &theme),
        ];
        if let (Some(exe), Some(dir)) = (exe, exe_dir) {
            rows.push(Self::row(5, "App", exe.display().to_string(), Some(("Open", open(dir))), &theme));
        }
        rows.push(Self::row(6, "License", "MIT (as Zeron)".to_owned(), None, &theme));

        let scrollbar = popover::rail(self, "about-page-scrollbar", &theme, cx);
        div()
            .id("about-page-host")
            .relative()
            .size_full()
            .on_hover(cx.listener(Self::on_scroll_hovered))
            .child(
                crate::edge_fade::edge_faded(
                    16.0,
                    true,
                    true,
                    div()
                        .id("about-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll.scroll)
                        .child(
                            widgets::page_column()
                                .child(widgets::page_header(&theme, "About", None))
                                .child(hero)
                                .child(widgets::section(
                                    &theme,
                                    "Details",
                                    div()
                                        .rounded(px(12.0))
                                        .bg(widgets::block_fill(&theme))
                                        .flex()
                                        .flex_col()
                                        .children(rows),
                                )),
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
    fn the_log_names_its_pin_and_counts_its_patches() {
        let log = "Source: x.\nPinned: upstream `main` @ `3b2f7802` (past `v0.2.113`). Release tags\n\
                   | # | Files | What |\n|---|---|---|\n| 1 | a | b |\n| 12 | c | d |\n| x | y |\n";
        assert_eq!(upstream_pin(log).as_deref(), Some("upstream main @ 3b2f7802 (past v0.2.113)"));
        assert_eq!(patch_count(log), 2);
        // The real log parses too.
        assert!(upstream_pin(VENDORED).is_some_and(|pin| pin.contains('@')));
        assert!(patch_count(VENDORED) >= 13);
    }
}
