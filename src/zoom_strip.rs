//! Shared navigation and chrome for strips over different coordinate systems.
//! Heap offsets remain integers; time views retain their original floating timestamps.
use crate::PURPLE;
use gpui_kit::component::{
    Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
};
use gpui_kit::*;
use std::{cell::Cell, rc::Rc};

pub(super) trait Viewport: Copy + Default {
    fn zoom(&mut self, factor: f64, anchor: f64);
    fn fit(&mut self);
    fn pan_fraction(&mut self, fraction: f64);
    fn step(&mut self, forward: bool) {
        self.pan_fraction(if forward { 0.1 } else { -0.1 });
    }
    fn edge(&mut self, end: bool);
    fn can_zoom_in(&self) -> bool;
    fn can_zoom_out(&self) -> bool;
}

#[derive(Default)]
pub(super) struct Navigation<V: Viewport> {
    pub viewport: V,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<(Point<Pixels>, V)>,
}

impl<V: Viewport> Navigation<V> {
    pub fn fraction(&self, position: Point<Pixels>) -> f64 {
        let bounds = self.bounds.get();
        ((position.x - bounds.left()).as_f32() / bounds.size.width.as_f32().max(1.)) as f64
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None;
    }
    pub fn begin_drag(&mut self, position: Point<Pixels>) {
        self.drag = Some((position, self.viewport));
    }
    #[cfg(test)]
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }
    fn pan_pixels(&mut self, pixels: Pixels) {
        self.viewport.pan_fraction(
            -f64::from(pixels.as_f32()) / f64::from(self.bounds.get().size.width.as_f32().max(1.)),
        );
    }
}

pub(super) trait State: 'static {
    type Viewport: Viewport;
    fn navigation(&self) -> &Navigation<Self::Viewport>;
    fn navigation_mut(&mut self) -> &mut Navigation<Self::Viewport>;
    fn select(&mut self, _position: Point<Pixels>) {}
}

pub(super) struct Controls {
    pub zoom_in: &'static str,
    pub zoom_out: &'static str,
    pub fit: &'static str,
    pub name: &'static str,
    pub fit_label: &'static str,
}

pub(super) fn controls<S: State>(
    state: &Entity<S>,
    controls: Controls,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let viewport = state.read(cx).navigation().viewport;
    let view = window.current_view();
    let mut buttons = Vec::new();
    for (id, icon, label, factor, disabled) in [
        (
            controls.zoom_out,
            IconName::Minus,
            format!("Zoom out {}", controls.name),
            0.5,
            !enabled || !viewport.can_zoom_out(),
        ),
        (
            controls.zoom_in,
            IconName::Plus,
            format!("Zoom in {}", controls.name),
            2.,
            !enabled || !viewport.can_zoom_in(),
        ),
        (
            controls.fit,
            IconName::Maximize,
            controls.fit_label.to_string(),
            0.,
            !enabled,
        ),
    ] {
        let state = state.clone();
        buttons.push(
            div().debug_selector(move || id.into()).child(
                Button::new(id)
                    .small()
                    .ghost()
                    .icon(icon)
                    .tooltip(label.clone())
                    .accessibility_label(label)
                    .disabled(disabled)
                    .on_click(move |_, _, cx| {
                        state.update(cx, |s, _| {
                            let v = &mut s.navigation_mut().viewport;
                            if factor == 0. {
                                v.fit();
                            } else {
                                v.zoom(factor, 0.5);
                            }
                        });
                        cx.notify(view);
                    }),
            ),
        );
    }
    div().flex().items_center().gap_2().children(buttons)
}

pub(super) fn surface<S: State>(
    state: &Entity<S>,
    id: &'static str,
    plot: impl IntoElement,
    window: &mut Window,
) -> impl IntoElement {
    let view = window.current_view();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .w_full()
        .h(px(42.))
        .min_w_0()
        .border_1()
        .border_color(rgb(0x625675))
        .focusable()
        .tab_stop(true)
        .cursor_pointer()
        .focus_visible(|s| s.border_color(rgb(PURPLE)))
        .on_scroll_wheel({
            let state = state.clone();
            move |event, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                state.update(cx, |s, _| {
                    let n = s.navigation_mut();
                    if delta.x != px(0.) {
                        n.pan_pixels(delta.x);
                    } else if event.modifiers.shift {
                        n.pan_pixels(delta.y);
                    } else {
                        let anchor = n.fraction(event.position);
                        n.viewport.zoom(
                            (f64::from(delta.y.as_f32()) * 0.01).clamp(-2., 2.).exp(),
                            anchor,
                        );
                    }
                });
                cx.stop_propagation();
                cx.notify(view);
            }
        })
        .on_mouse_down(MouseButton::Left, {
            let state = state.clone();
            move |event, _, cx| {
                state.update(cx, |s, _| s.navigation_mut().begin_drag(event.position));
            }
        })
        .on_mouse_move({
            let state = state.clone();
            move |event, _, cx| {
                if event.dragging() {
                    state.update(cx, |s, _| {
                        let n = s.navigation_mut();
                        if let Some((origin, viewport)) = n.drag {
                            n.viewport = viewport;
                            n.pan_pixels(event.position.x - origin.x);
                        }
                    });
                    cx.notify(view);
                }
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let state = state.clone();
            move |event, _, cx| {
                state.update(cx, |s, _| {
                    if let Some((origin, _)) = s.navigation_mut().drag.take()
                        && (event.position.x - origin.x).abs() < px(3.)
                        && (event.position.y - origin.y).abs() < px(3.)
                    {
                        s.select(event.position);
                    }
                });
                cx.notify(view);
            }
        })
        .on_mouse_up_out(MouseButton::Left, {
            let state = state.clone();
            move |_, _, cx| {
                state.update(cx, |s, _| s.navigation_mut().cancel_drag());
            }
        })
        .on_key_down({
            let state = state.clone();
            move |event, _, cx| {
                let key = event.keystroke.key.as_str();
                if ["left", "right", "home", "end"].contains(&key) {
                    state.update(cx, |s, _| {
                        let v = &mut s.navigation_mut().viewport;
                        match key {
                            "left" => v.step(false),
                            "right" => v.step(true),
                            "home" => v.edge(false),
                            "end" => v.edge(true),
                            _ => {}
                        }
                    });
                    cx.stop_propagation();
                    cx.notify(view);
                }
            }
        })
        .child(plot)
}

pub(super) fn label_origin(bounds: Bounds<Pixels>, width: Pixels) -> Option<Point<Pixels>> {
    (bounds.size.width >= width + px(8.) && bounds.size.height >= px(14.)).then(|| {
        bounds.origin
            + point(
                (bounds.size.width - width) / 2.,
                (bounds.size.height - px(14.)) / 2.,
            )
    })
}
