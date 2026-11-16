use super::Colors;
use gpui::{
    Bounds, Div, ElementId, HitboxBehavior, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollHandle, ScrollWheelEvent, Stateful, canvas, div, fill, point,
    prelude::*, px, size,
};
use std::{cell::Cell, rc::Rc};

pub(super) fn scrollbar(
    id: impl Into<ElementId>,
    scroll: &ScrollHandle,
    colors: Colors,
) -> Stateful<Div> {
    let scroll = scroll.clone();
    div().id(id).w(px(16.)).flex_shrink_0().p(px(4.)).child(
        canvas(
            |bounds, window, _| {
                let drag_offset = window.with_global_id("drag".into(), |id, window| {
                    window.with_element_state(id, |state: Option<Rc<Cell<Option<Pixels>>>>, _| {
                        let state = state.unwrap_or_default();
                        (state.clone(), state)
                    })
                });
                let hitbox = window.insert_hitbox(bounds.dilate(px(4.)), HitboxBehavior::Normal);
                (hitbox, drag_offset)
            },
            move |track, (hitbox, drag_offset), window, _| {
                // Scroll metrics are current only after the content's prepaint.
                let max_offset = scroll.max_offset().height;
                if max_offset <= px(0.) || track.size.height <= px(0.) {
                    drag_offset.set(None);
                    return;
                }
                let viewport = scroll.bounds().size.height;
                let thumb_height = (track.size.height * (viewport / (viewport + max_offset)))
                    .max(px(28.))
                    .min(track.size.height);
                let travel = track.size.height - thumb_height;
                let thumb = Bounds::new(
                    point(
                        track.left(),
                        track.top() + travel * (-scroll.offset().y / max_offset).clamp(0., 1.),
                    ),
                    size(track.size.width, thumb_height),
                );
                let hovered = hitbox.is_hovered(window);
                let active = hovered || drag_offset.get().is_some();
                window.paint_quad(fill(track, colors.line).corner_radii(px(4.)));
                window.paint_quad(
                    fill(thumb, if active { colors.accent } else { colors.muted })
                        .corner_radii(px(4.)),
                );
                window.on_mouse_event({
                    let scroll = scroll.clone();
                    let drag_offset = drag_offset.clone();
                    let hitbox = hitbox.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble()
                            && event.button == MouseButton::Left
                            && hitbox.is_hovered(window)
                        {
                            if event.position.y >= thumb.top() && event.position.y <= thumb.bottom()
                            {
                                drag_offset.set(Some(event.position.y - thumb.top()));
                            } else {
                                let direction = if event.position.y < thumb.top() {
                                    1.
                                } else {
                                    -1.
                                };
                                let mut offset = scroll.offset();
                                offset.y = (offset.y + viewport * 0.9 * direction)
                                    .clamp(-max_offset, px(0.));
                                scroll.set_offset(offset);
                            }
                            window.refresh();
                            cx.stop_propagation();
                        }
                    }
                });
                window.on_mouse_event({
                    let scroll = scroll.clone();
                    let drag_offset = drag_offset.clone();
                    let hitbox = hitbox.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if !phase.capture() {
                            return;
                        }
                        if !event.dragging() {
                            if drag_offset.take().is_some() {
                                window.refresh();
                            }
                        } else if let Some(grab) = drag_offset.get()
                            && travel > px(0.)
                        {
                            let fraction =
                                ((event.position.y - track.top() - grab) / travel).clamp(0., 1.);
                            let mut offset = scroll.offset();
                            offset.y = -max_offset * fraction;
                            scroll.set_offset(offset);
                            window.refresh();
                            cx.stop_propagation();
                        }
                        if hovered != hitbox.is_hovered(window) {
                            window.refresh();
                        }
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, _| {
                    if phase.capture()
                        && event.button == MouseButton::Left
                        && drag_offset.take().is_some()
                    {
                        window.refresh();
                    }
                });
                let line_height = window.line_height();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase.bubble() && hitbox.should_handle_scroll(window) {
                        let delta = event.delta.pixel_delta(line_height);
                        let delta_y = if delta.y == px(0.) { delta.x } else { delta.y };
                        let mut offset = scroll.offset();
                        let next = (offset.y + delta_y).clamp(-max_offset, px(0.));
                        if next != offset.y {
                            offset.y = next;
                            scroll.set_offset(offset);
                            window.refresh();
                            cx.stop_propagation();
                        }
                    }
                });
            },
        )
        .size_full(),
    )
}
