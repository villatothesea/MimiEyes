//! MimiEyes (VENDORED.md #17): a breathing mark in its own cached view, built
//! like Zeron's mini spinner. A repeating `with_animation` inside the shell
//! asks the shell — the whole window — to re-render every frame for as long
//! as it is on screen; here only this small view repaints, on the shared
//! pulse clock (15 Hz, parked when nothing breathes, still under reduced
//! motion).

use gpui::{App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div, prelude::*, px};

/// What breathes.
#[derive(Clone, PartialEq)]
pub enum Breath {
    /// A label, right-aligned (an unread session's age).
    Text {
        text: SharedString,
        color: gpui::Hsla,
        size: f32,
        bold: bool,
    },
    /// A number on a rounded square (a working tab's place).
    Square {
        text: SharedString,
        bg: gpui::Hsla,
        fg: gpui::Hsla,
    },
    /// A glyph (a running pane's CLI logo).
    Icon {
        path: &'static str,
        tint: gpui::Hsla,
        size: f32,
    },
}

/// `breath` in a `w`×`h` px box that pulses on its own.
pub fn breathing(key: impl Into<SharedString>, breath: Breath, w: f32, h: f32) -> impl IntoElement {
    Breathing {
        key: key.into(),
        breath,
        w,
        h,
    }
}

#[derive(IntoElement)]
struct Breathing {
    key: SharedString,
    breath: Breath,
    w: f32,
    h: f32,
}

impl RenderOnce for Breathing {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let view = window.with_global_id(self.key.into(), |id, window| {
            window.with_element_state(id, |previous: Option<gpui::Entity<BreathView>>, _| {
                let view = previous.unwrap_or_else(|| {
                    cx.new(|_| BreathView {
                        breath: self.breath.clone(),
                    })
                });
                view.update(cx, |view, cx| {
                    if view.breath != self.breath {
                        view.breath = self.breath.clone();
                        cx.notify();
                    }
                });
                (view.clone(), view)
            })
        });
        // `flex_none`: never squeezed by its row (a tab number overhangs the
        // 12px dots' column on both sides, like the static square).
        view.cached(
            gpui::StyleRefinement::default()
                .flex_none()
                .w(px(self.w))
                .h(px(self.h)),
        )
    }
}

struct BreathView {
    breath: Breath,
}

impl Render for BreathView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity_id();
        // Reduced motion: a steady mark, no clock.
        let opacity = if cx.reduce_motion() {
            1.0
        } else {
            let t = crate::motion::pulse_delta_slow(&crate::motion::ZERON_PULSE, view, cx);
            0.45 + 0.55 * (std::f32::consts::PI * t).sin().powi(2)
        };
        let frame = div().size_full().flex().items_center().opacity(opacity);
        match &self.breath {
            Breath::Text {
                text,
                color,
                size,
                bold,
            } => frame
                .justify_end()
                .text_size(px(*size))
                .text_color(*color)
                .when(*bold, |el| el.font_weight(gpui::FontWeight::BOLD))
                .child(text.clone()),
            Breath::Square { text, bg, fg } => frame
                .justify_center()
                .rounded(px(4.0))
                .bg(*bg)
                .text_color(*fg)
                .text_size(px(10.5))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(text.clone()),
            Breath::Icon { path, tint, size } => frame
                .justify_center()
                .child(crate::icons::icon(path).size(px(*size)).text_color(*tint)),
        }
    }
}
