//! One visual vocabulary for thread wait queues in overviews and details.
use crate::{MUTED, PURPLE};
use gpui_kit::component::{Icon, tag::Tag};
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
                    .max_w_full()
                    .child(
                        Tag::custom(
                            rgb(0x352b4d).into(),
                            rgb(PURPLE).into(),
                            rgb(0x55416f).into(),
                        )
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
                                        .child(Icon::new(assets::IconName::Spool).size(px(14.))),
                                )
                                .child(div().min_w_0().child(name.clone())),
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
    div()
        .debug_selector(move || format!("{prefix}-waiter-tags"))
        .flex()
        .flex_col()
        .items_start()
        .min_w_0()
        .gap_2()
        .when(matches!(layout, Layout::Wrap), |el| {
            el.flex_row().flex_wrap()
        })
        .children(entries)
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
