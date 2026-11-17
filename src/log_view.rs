use super::{Colors, FeedbackScope, MainWindow, TEXT_PRIMARY, control, panel, scroll_area};
use crate::{
    desktop, logging,
    logging::{LogLevel, LogSource, log_level},
};
use gpui::{
    ClipboardItem, Context, Div, PathBuilder, Rgba, ScrollHandle, Stateful, canvas, div, point,
    prelude::*, px,
};

pub(super) struct LogView {
    source: LogSource,
    lines: [Vec<String>; 4],
    paused: bool,
    scroll: [ScrollHandle; 4],
}

impl LogView {
    pub(super) fn new() -> Self {
        Self {
            source: LogSource::App,
            lines: Default::default(),
            paused: false,
            scroll: std::array::from_fn(|_| {
                let scroll = ScrollHandle::new();
                scroll.scroll_to_bottom();
                scroll
            }),
        }
    }

    pub(super) fn refresh(&mut self) -> bool {
        if self.paused {
            return false;
        }
        let mut changed = false;
        for source in LogSource::ALL {
            let index = source.index();
            let lines = logging::recent_for(source);
            if lines != self.lines[index] {
                let scroll = &self.scroll[index];
                let at_bottom = -scroll.offset().y >= scroll.max_offset().height - px(2.);
                self.lines[index] = lines;
                if at_bottom {
                    scroll.scroll_to_bottom();
                }
                changed |= source == self.source;
            }
        }
        changed
    }

    fn select(&mut self, source: LogSource) {
        self.source = source;
        self.refresh();
    }

    fn toggle_paused(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.refresh();
            self.scroll[self.source.index()].scroll_to_bottom();
        }
    }

    pub(super) fn scroll_key(&self, key: &str) -> bool {
        let scroll = &self.scroll[self.source.index()];
        let mut offset = scroll.offset();
        offset.y = match key {
            "pageup" => (offset.y + scroll.bounds().size.height * 0.9).min(px(0.)),
            "pagedown" => {
                (offset.y - scroll.bounds().size.height * 0.9).max(-scroll.max_offset().height)
            }
            "home" => px(0.),
            "end" => -scroll.max_offset().height,
            _ => return false,
        };
        scroll.set_offset(offset);
        true
    }
}

fn button(label: &'static str, id: &'static str, index: isize, colors: Colors) -> Stateful<Div> {
    control(id, index, colors, false)
        .flex_shrink_0()
        .h(px(30.))
        .min_w(px(82.))
        .justify_center()
        .px(px(12.))
        .text_size(px(TEXT_PRIMARY))
        .border_color(colors.line)
        .bg(colors.panel)
        .child(label)
}

fn playback_icon(paused: bool, color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scale = bounds.size.width / px(16.);
            let p = |x, y| bounds.origin + point(px(x) * scale, px(y) * scale);
            let mut path = PathBuilder::fill();
            if paused {
                path.move_to(p(4., 2.));
                path.line_to(p(14., 8.));
                path.line_to(p(4., 14.));
                path.close();
            } else {
                for x in [3., 10.] {
                    path.move_to(p(x, 2.));
                    path.line_to(p(x + 3., 2.));
                    path.line_to(p(x + 3., 14.));
                    path.line_to(p(x, 14.));
                    path.close();
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}

impl MainWindow {
    pub(super) fn logs_page(&self, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        let source = self.logs.source;
        let mut sources = div()
            .flex()
            .flex_shrink_0()
            .gap(px(2.))
            .p(px(1.))
            .bg(colors.canvas)
            .border_1()
            .border_color(colors.line)
            .rounded(px(6.));
        for (index, item) in LogSource::ALL.into_iter().enumerate() {
            sources = sources.child(
                control(
                    ("log-source", item.index()),
                    6 + index as isize,
                    colors,
                    false,
                )
                .h(px(26.))
                .min_w(px(60.))
                .px(px(8.))
                .justify_center()
                .text_size(px(TEXT_PRIMARY))
                .when(source == item, |el| {
                    el.bg(colors.selection).text_color(colors.accent)
                })
                .child(item.label())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.logs.select(item);
                    cx.notify();
                })),
            );
        }
        let playback_colors = if self.logs.paused {
            colors
        } else {
            Colors {
                accent_fill: colors.stop_fill,
                accent_hover: colors.stop_hover,
                ..colors
            }
        };
        div()
            .id("application-logs")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(8.))
                    .child(sources)
                    .child(
                        control("pause-logs", 10, playback_colors, true)
                            .size(px(30.))
                            .flex_shrink_0()
                            .justify_center()
                            .bg(playback_colors.accent_fill)
                            .border_color(playback_colors.accent_fill)
                            .child(playback_icon(self.logs.paused, colors.on_accent))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.logs.toggle_paused();
                                cx.notify();
                            })),
                    )
                    .child(
                        button("复制日志", "copy-logs", 11, colors).on_click(cx.listener(
                            |view, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    view.logs.lines[view.logs.source.index()].join("\n"),
                                ));
                            },
                        )),
                    )
                    .child(div().flex_1())
                    .child(
                        button("打开日志目录", "open-log-folder", 12, colors).on_click(
                            cx.listener(|view, _, _, cx| {
                                if let Some(path) = logging::path()
                                    .and_then(|path| path.parent().map(std::path::Path::to_owned))
                                {
                                    Self::open_external(
                                        path,
                                        desktop::open_folder,
                                        FeedbackScope::Logs,
                                        cx,
                                    );
                                } else {
                                    logging::warn("desktop", "应用日志目录尚未初始化");
                                    view.logs.refresh();
                                    cx.notify();
                                }
                            }),
                        ),
                    ),
            )
            .child(
                panel(colors).flex_1().min_h(px(0.)).flex().child(
                    scroll_area(
                        ("log-scroll", source.index()),
                        div()
                            .id(("log-lines", source.index()))
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .p(px(12.))
                            .gap(px(6.))
                            .text_size(px(TEXT_PRIMARY))
                            .children(self.logs.lines[source.index()].iter().enumerate().map(
                                |(index, line)| {
                                    div()
                                        .id(index)
                                        .flex_shrink_0()
                                        .w_full()
                                        .whitespace_normal()
                                        .text_color(match log_level(line) {
                                            Some(LogLevel::Error) => colors.error,
                                            Some(LogLevel::Warning) => colors.warning,
                                            None => colors.text,
                                        })
                                        .child(line.clone())
                                },
                            )),
                        &self.logs.scroll[source.index()],
                        colors,
                    )
                    .flex_1(),
                ),
            )
    }
}
