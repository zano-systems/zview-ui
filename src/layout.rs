//! Small shared layout primitives for bounded, independently scrollable screens.
use crate::MUTED;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

/// Keep supporting text on one line without losing the full explanation.
pub(super) fn note(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
) -> impl IntoElement + Styled {
    let id = id.into();
    let text = text.into();
    div()
        .id(id.clone())
        .debug_selector(move || id.to_string())
        .min_w_0()
        .flex_shrink_0()
        .text_xs()
        .text_color(rgb(MUTED))
        .truncate()
        .tooltip({
            let text = text.clone();
            move |window, cx| Tooltip::new(text.clone()).build(window, cx)
        })
        .child(text)
}
