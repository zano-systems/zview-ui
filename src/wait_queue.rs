//! One visual vocabulary for thread wait queues in overviews and details.
use crate::{MUTED, PURPLE};
use gpui_kit::component::{Icon, scroll::ScrollableElement, tag::Tag};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

#[derive(Clone, Copy)]
pub(super) enum Layout {
    Vertical,
    Wrap,
}

pub(super) fn empty_message(names: Option<&[String]>) -> Option<&'static str> {
    if names.is_some() {
        None
    } else {
        Some("Wait queue unavailable")
    }
}

pub(super) fn threads(
    prefix: impl Into<String>,
    names: Option<&[String]>,
    layout: Layout,
) -> impl IntoElement {
    let prefix = prefix.into();
    let entries = match names {
        Some(names) if !names.is_empty() => names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let selector = format!("{prefix}-waiter-{index}");
                div()
                    .debug_selector(move || selector.clone())
                    .min_w_0()
                    .max_w_full()
                    .when(matches!(layout, Layout::Vertical), |el| el.w_full())
                    .child(
                        Tag::custom(
                            rgb(0x352b4d).into(),
                            rgb(PURPLE).into(),
                            rgb(0x55416f).into(),
                        )
                        .min_w_0()
                        .max_w_full()
                        .rounded_full()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    div()
                                        .debug_selector(|| "wait-thread-icon".into())
                                        .flex_shrink_0()
                                        .child(Icon::new(assets::IconName::Spool).size(px(14.))),
                                )
                                .child(
                                    crate::layout::note(
                                        format!("{prefix}-waiter-label-{index}"),
                                        name.clone(),
                                    )
                                    .flex_1()
                                    .text_sm(),
                                ),
                        ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>(),
        _ => empty_message(names)
            .into_iter()
            .map(|message| {
                let selector = format!("{prefix}-wait-queue-empty");
                div()
                    .debug_selector(move || selector.clone())
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child(message)
                    .into_any_element()
            })
            .collect(),
    };
    let scroll_id = SharedString::from(format!("{prefix}-wait-queue-scroll"));
    let content = div()
        .debug_selector(move || format!("{prefix}-waiter-tags"))
        .flex()
        .flex_col()
        .items_start()
        .min_w_0()
        .gap_2()
        .when(matches!(layout, Layout::Wrap), |el| {
            el.flex_row().flex_wrap()
        })
        .children(entries);
    match layout {
        Layout::Wrap => content.into_any_element(),
        Layout::Vertical => div()
            .id(scroll_id.clone())
            .debug_selector({
                let scroll_id = scroll_id.clone();
                move || scroll_id.to_string()
            })
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .id(scroll_id)
            .child(content)
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::empty_message;
    #[test]
    fn empty_queues_have_no_message_but_unknown_queues_remain_explicit() {
        assert_eq!(empty_message(Some(&[])), None);
        assert_eq!(empty_message(None), Some("Wait queue unavailable"));
    }
}
