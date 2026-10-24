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
    /// Normalized viewport start/end within the full domain.
    fn overview_range(&self) -> (f64, f64);
    /// Move the start within the full domain, preserving the zoom span.
    fn set_overview_start(&mut self, fraction: f64);
}

#[derive(Default)]
pub(super) struct Navigation<V: Viewport> {
    pub viewport: V,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    drag: Option<(Point<Pixels>, V)>,
    overview_bounds: Rc<Cell<Bounds<Pixels>>>,
    overview_grab: Option<f64>,
}

impl<V: Viewport> Navigation<V> {
    pub fn fraction(&self, position: Point<Pixels>) -> f64 {
        let bounds = self.bounds.get();
        ((position.x - bounds.left()).as_f32() / bounds.size.width.as_f32().max(1.)) as f64
    }
    pub fn cancel_drag(&mut self) {
        self.drag = None;
        self.overview_grab = None;
    }
    pub fn begin_drag(&mut self, position: Point<Pixels>) {
        self.cancel_drag();
        self.drag = Some((position, self.viewport));
    }
    fn overview_fraction(&self, position: Point<Pixels>) -> f64 {
        let bounds = self.overview_bounds.get();
        f64::from((position.x - bounds.left()) / bounds.size.width.max(px(1.)))
    }
    fn begin_overview_drag(&mut self, position: Point<Pixels>) {
        self.cancel_drag();
        let fraction = self.overview_fraction(position).clamp(0., 1.);
        let (start, end) = self.viewport.overview_range();
        let width = f64::from(self.overview_bounds.get().size.width.as_f32()).max(1.);
        let (left, right) = overview_pixels((start, end), width);
        // The minimum-width handle must also be grabbable at extreme zoom.
        if fraction * width < left || fraction * width > right {
            self.viewport
                .set_overview_start(fraction - (end - start) / 2.);
        }
        self.overview_grab = Some(fraction - self.viewport.overview_range().0);
    }
    fn drag_overview(&mut self, position: Point<Pixels>) {
        if let Some(grab) = self.overview_grab {
            self.viewport
                .set_overview_start(self.overview_fraction(position) - grab);
        }
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
) -> impl IntoElement + use<S> {
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

/// A full-domain preview supplied by the caller, with shared viewport chrome
/// and navigation. Its bounds are separate from the detailed strip's bounds.
pub(super) fn minimap<S: State>(
    state: &Entity<S>,
    id: &'static str,
    name: &'static str,
    plot: impl IntoElement,
    window: &mut Window,
    cx: &App,
) -> impl IntoElement {
    let n = state.read(cx).navigation();
    let range = n.viewport.overview_range();
    let bounds = n.overview_bounds.clone();
    let view = window.current_view();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .relative()
        .w(px(120.))
        .h(px(10.))
        .flex_shrink_0()
        .min_w_0()
        .border_1()
        .border_color(rgb(0x625675))
        .focusable()
        .tab_stop(true)
        .aria_label(format!(
            "{name} overview. Click or drag to pan; arrow keys, Home and End move the viewport."
        ))
        .cursor_pointer()
        .focus_visible(|s| s.border_color(rgb(PURPLE)))
        .on_mouse_down(MouseButton::Left, {
            let state = state.clone();
            move |event, _, cx| {
                state.update(cx, |s, _| {
                    s.navigation_mut().begin_overview_drag(event.position)
                });
                cx.notify(view);
            }
        })
        .on_mouse_move({
            let state = state.clone();
            move |event, _, cx| {
                if event.dragging() {
                    state.update(cx, |s, _| s.navigation_mut().drag_overview(event.position));
                    cx.notify(view);
                }
            }
        })
        .on_mouse_up(MouseButton::Left, {
            let state = state.clone();
            move |_, _, cx| {
                state.update(cx, |s, _| s.navigation_mut().overview_grab = None);
            }
        })
        .on_mouse_up_out(MouseButton::Left, {
            let state = state.clone();
            move |_, _, cx| {
                state.update(cx, |s, _| s.navigation_mut().overview_grab = None);
            }
        })
        .on_key_down({
            let state = state.clone();
            move |event, _, cx| {
                let handled = state.update(cx, |s, _| {
                    navigate_key(&mut s.navigation_mut().viewport, &event.keystroke.key)
                });
                if handled {
                    cx.stop_propagation();
                    cx.notify(view);
                }
            }
        })
        .child(plot)
        .child(
            div().absolute().inset_0().child(
                canvas(
                    move |rect, _, _| bounds.set(rect),
                    move |rect, _, window, _| {
                        let (left, right) = overview_pixels(range, rect.size.width.as_f32() as f64);
                        let left = rect.left() + px(left as f32);
                        let right = rect.left() + px(right as f32);
                        // Dim the context, without replacing its original colors.
                        for (a, b) in [(rect.left(), left), (right, rect.right())] {
                            window.paint_quad(fill(
                                Bounds::new(point(a, rect.top()), size(b - a, rect.size.height)),
                                rgba(0x141120aa),
                            ));
                        }
                        for edge in [
                            Bounds::new(point(left, rect.top()), size(right - left, px(1.))),
                            Bounds::new(
                                point(left, rect.bottom() - px(1.)),
                                size(right - left, px(1.)),
                            ),
                            Bounds::new(point(left, rect.top()), size(px(1.), rect.size.height)),
                            Bounds::new(
                                point(right - px(1.), rect.top()),
                                size(px(1.), rect.size.height),
                            ),
                        ] {
                            window.paint_quad(fill(edge, rgb(0xe6dfef)));
                        }
                    },
                )
                .size_full(),
            ),
        )
}

/// Keep a subpixel viewport visible, without changing the actual zoom span.
fn overview_pixels(range: (f64, f64), width: f64) -> (f64, f64) {
    let width = width.max(0.);
    let left = range.0.clamp(0., 1.) * width;
    let right = range.1.clamp(range.0.clamp(0., 1.), 1.) * width;
    let span = (right - left).max(2.).min(width);
    let left = ((left + right - span) / 2.).clamp(0., width - span);
    (left, left + span)
}

fn navigate_key<V: Viewport>(viewport: &mut V, key: &str) -> bool {
    match key {
        "left" => viewport.step(false),
        "right" => viewport.step(true),
        "home" => viewport.edge(false),
        "end" => viewport.edge(true),
        _ => return false,
    }
    true
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
                state.update(cx, |s, _| s.navigation_mut().drag = None);
            }
        })
        .on_key_down({
            let state = state.clone();
            move |event, _, cx| {
                let handled = state.update(cx, |s, _| {
                    navigate_key(&mut s.navigation_mut().viewport, &event.keystroke.key)
                });
                if handled {
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

#[cfg(test)]
mod tests {
    use super::overview_pixels;

    #[test]
    fn overview_highlight_tracks_the_range_and_keeps_subpixel_windows_visible() {
        assert_eq!(overview_pixels((0.25, 0.75), 100.), (25., 75.));
        assert_eq!(overview_pixels((0., 1.), 100.), (0., 100.));
        assert_eq!(overview_pixels((0., 1e-12), 100.), (0., 2.));
        assert_eq!(overview_pixels((1. - 1e-12, 1.), 100.), (98., 100.));
        assert_eq!(overview_pixels((0.5, 0.5), 100.), (49., 51.));
        assert_eq!(overview_pixels((0.25, 0.75), 0.), (0., 0.));
        assert_eq!(overview_pixels((0., 1.), 1.), (0., 1.));
    }
}
