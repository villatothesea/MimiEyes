//! MimiEyes (VENDORED.md #1): the boot splash's mark — the "Just Eyes"
//! logo (`assets/brand/mimieyes-icon.svg`) come alive in place of Zeron's
//! dot spinner: the jumping spider glances left and right, blinks, and
//! hops. Drawn from divs at the logo's own proportions (its 512 canvas
//! scaled down), so it stays crisp at any size.

use gpui::{Animation, AnimationExt as _, IntoElement, ParentElement as _, Styled as _, div, px};
use std::time::Duration;

/// The logo's ink (its eyes); its body wears the accent (VENDORED.md #10).
const INK: u32 = 0x111a14;
/// One loop: glance, blink, hop.
const PERIOD_MS: u64 = 2400;

/// The animated mark, `size` px square, its body in `accent` (the user's
/// accent colour, as the taskbar icon).
pub fn spider_loader(id: &'static str, size: f32, accent: gpui::Hsla) -> impl IntoElement {
    // Logo units (512 canvas) → px.
    let u = move |v: f32| v * size / 512.0;
    div().size(px(size)).relative().child(
        div().size(px(size)).absolute().with_animation(
            id,
            Animation::new(Duration::from_millis(PERIOD_MS)).repeat(),
            move |el, t| {
                let tau = std::f32::consts::TAU;
                let ink: gpui::Hsla = gpui::rgb(INK).into();
                // The gaze sweeps left → right → left over the loop.
                let look = u(20.0) * (tau * t).sin();
                // A quick blink a third of the way in.
                let blink = {
                    let d = ((t - 0.34) / 0.05).abs();
                    if d < 1.0 { 0.12 + 0.88 * d } else { 1.0 }
                };
                // A small hop near the end (a jumping spider, after all).
                let hop = if (0.72..0.92).contains(&t) {
                    u(40.0) * (std::f32::consts::PI * (t - 0.72) / 0.2).sin()
                } else {
                    0.0
                };
                let circle = |cx: f32, cy: f32, r: f32, color: gpui::Hsla, squash: f32| {
                    let h = 2.0 * r * squash;
                    div()
                        .absolute()
                        .left(px(u(cx - r)))
                        .top(px(u(cy) - u(h) / 2.0))
                        .w(px(u(2.0 * r)))
                        .h(px(u(h)))
                        .rounded_full()
                        .bg(color)
                };
                let eye = |cx: f32| {
                    let mut eye = circle(cx, 270.0, 80.0, ink, blink);
                    // Highlights only while the eye is open.
                    if blink > 0.6 {
                        eye = eye
                            .child(
                                div()
                                    .absolute()
                                    .left(px(u(80.0 - 26.0 - 20.0) + look))
                                    .top(px(u(80.0 - 26.0 - 20.0)))
                                    .size(px(u(40.0)))
                                    .rounded_full()
                                    .bg(accent),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(px(u(80.0 + 20.0 - 8.0) + look))
                                    .top(px(u(80.0 + 22.0 - 8.0)))
                                    .size(px(u(16.0)))
                                    .rounded_full()
                                    .bg(accent),
                            );
                    }
                    eye
                };
                el.top(px(-hop))
                    .rounded(px(u(115.0)))
                    .bg(accent)
                    .overflow_hidden()
                    .child(circle(66.0, 236.0, 22.0, ink, 1.0))
                    .child(circle(446.0, 236.0, 22.0, ink, 1.0))
                    .child(eye(176.0))
                    .child(eye(336.0))
            },
        ),
    )
}
