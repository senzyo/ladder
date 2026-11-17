use super::{Colors, MainWindow, TEXT_PRIMARY};
use gpui::{
    Bounds, ClipboardItem, Context, Div, FocusHandle, MouseButton, Pixels, Point, ShapedLine,
    Stateful, TextRun, Window, canvas, div, fill, point, prelude::*, px, size,
};
use std::{cell::RefCell, ops::Range, rc::Rc};

struct InputLayout {
    line: ShapedLine,
    bounds: Bounds<Pixels>,
    offset: Pixels,
}

pub(super) struct ProxyInput {
    text: String,
    cursor: usize,
    anchor: usize,
    focus: FocusHandle,
    layout: Rc<RefCell<Option<InputLayout>>>,
}

impl ProxyInput {
    pub(super) fn new(text: String, window: &mut Window, cx: &mut Context<MainWindow>) -> Self {
        let cursor = text.len();
        let focus = cx.focus_handle().tab_index(18).tab_stop(true);
        cx.on_blur(&focus, window, |view, _, cx| view.save_github_proxy(cx))
            .detach();
        Self {
            text,
            cursor,
            anchor: cursor,
            focus,
            layout: Rc::default(),
        }
    }

    pub(super) fn value(&self) -> &str {
        &self.text
    }

    pub(super) fn set_value(&mut self, text: String) {
        self.cursor = text.len();
        self.anchor = self.cursor;
        self.text = text;
        self.layout.borrow_mut().take();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn move_to(&mut self, cursor: usize, selecting: bool) {
        self.cursor = cursor;
        if !selecting {
            self.anchor = cursor;
        }
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |ch| self.cursor + ch.len_utf8())
    }

    fn replace(&mut self, text: &str) {
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        let range = self.selection();
        if self.text.len() - range.len() + text.len() > 2048 {
            return;
        }
        let cursor = range.start + text.len();
        self.text.replace_range(range, &text);
        self.move_to(cursor, false);
    }

    fn click(&mut self, position: Point<Pixels>, selecting: bool) {
        let cursor = self.layout.borrow().as_ref().map(|layout| {
            layout
                .line
                .closest_index_for_x(position.x - layout.bounds.left() + layout.offset)
                .min(self.text.len())
        });
        if let Some(mut cursor) = cursor {
            while !self.text.is_char_boundary(cursor) {
                cursor -= 1;
            }
            self.move_to(cursor, selecting);
        }
    }

    pub(super) fn render(&self, colors: Colors, cx: &Context<MainWindow>) -> Stateful<Div> {
        let text = self.text.clone();
        let selection = self.selection();
        let cursor = self.cursor;
        let focus = self.focus.clone();
        let layout = self.layout.clone();
        div()
            .id("github-proxy-input")
            .track_focus(&self.focus)
            .tab_index(18)
            .flex_1()
            .min_w(px(100.))
            .h(px(30.))
            .px(px(8.))
            .py(px(5.))
            .border_1()
            .border_color(colors.line)
            .rounded(px(5.))
            .bg(colors.panel)
            .focus(|el| el.border_color(colors.accent))
            .cursor_text()
            .overflow_hidden()
            .on_mouse_down_out(cx.listener(|view, _, window, _| {
                if view.github_proxy.focus.is_focused(window) {
                    window.focus(&view.focus);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&view.github_proxy.focus);
                    view.github_proxy
                        .click(event.position, event.modifiers.shift);
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_key_down(
                cx.listener(move |view, event: &gpui::KeyDownEvent, window, cx| {
                    let input = &mut view.github_proxy;
                    if !input.focus.is_focused(window) {
                        return;
                    }
                    let key = &event.keystroke.key;
                    let modifiers = event.keystroke.modifiers;
                    if key == "tab" {
                        return;
                    }
                    if key == "enter" {
                        view.save_github_proxy(cx);
                    } else if modifiers.control {
                        match key.as_str() {
                            "a" => {
                                input.anchor = 0;
                                input.cursor = input.text.len();
                            }
                            "c" | "x" => {
                                let range = input.selection();
                                if !range.is_empty() {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        input.text[range].into(),
                                    ));
                                    if key == "x" {
                                        input.replace("");
                                    }
                                }
                            }
                            "v" => {
                                if let Some(text) =
                                    cx.read_from_clipboard().and_then(|item| item.text())
                                {
                                    input.replace(text.trim());
                                }
                            }
                            "left" => input.move_to(0, modifiers.shift),
                            "right" => input.move_to(input.text.len(), modifiers.shift),
                            _ => return,
                        }
                    } else {
                        match key.as_str() {
                            "left" => {
                                let cursor = if !modifiers.shift && !input.selection().is_empty() {
                                    input.selection().start
                                } else {
                                    input.previous()
                                };
                                input.move_to(cursor, modifiers.shift);
                            }
                            "right" => {
                                let cursor = if !modifiers.shift && !input.selection().is_empty() {
                                    input.selection().end
                                } else {
                                    input.next()
                                };
                                input.move_to(cursor, modifiers.shift);
                            }
                            "home" => input.move_to(0, modifiers.shift),
                            "end" => input.move_to(input.text.len(), modifiers.shift),
                            "backspace" => {
                                if input.selection().is_empty() {
                                    input.anchor = input.previous();
                                }
                                input.replace("");
                            }
                            "delete" => {
                                if input.selection().is_empty() {
                                    input.anchor = input.next();
                                }
                                input.replace("");
                            }
                            "escape" => {
                                input.set_value(view.snapshot.settings.github_proxy.clone());
                                window.focus(&view.focus);
                            }
                            _ => {
                                if let Some(text) = &event.keystroke.key_char {
                                    input.replace(text);
                                } else {
                                    return;
                                }
                            }
                        }
                    }
                    cx.notify();
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(
                cx.listener(|view, event: &gpui::MouseMoveEvent, window, cx| {
                    if event.dragging() && view.github_proxy.focus.is_focused(window) {
                        view.github_proxy.click(event.position, true);
                        cx.notify();
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, window, _| {
                        let run = TextRun {
                            len: text.len(),
                            font: window.text_style().font(),
                            color: colors.text.into(),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(
                            text.clone().into(),
                            px(TEXT_PRIMARY),
                            &[run],
                            None,
                        );
                        let offset =
                            (line.x_for_index(cursor) - bounds.size.width + px(3.)).max(px(0.));
                        *layout.borrow_mut() = Some(InputLayout {
                            line: line.clone(),
                            bounds,
                            offset,
                        });
                        (line, offset)
                    },
                    move |bounds, (line, offset), window, cx| {
                        let origin = bounds.origin - point(offset, px(0.));
                        if focus.is_focused(window) {
                            if selection.is_empty() {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        origin + point(line.x_for_index(cursor), px(0.)),
                                        size(px(1.), bounds.size.height),
                                    ),
                                    colors.accent,
                                ));
                            } else {
                                window.paint_quad(fill(
                                    Bounds::from_corners(
                                        origin + point(line.x_for_index(selection.start), px(0.)),
                                        origin
                                            + point(
                                                line.x_for_index(selection.end),
                                                bounds.size.height,
                                            ),
                                    ),
                                    colors.selection,
                                ));
                            }
                        }
                        let _ = line.paint(origin, bounds.size.height, window, cx);
                    },
                )
                .size_full(),
            )
    }
}
