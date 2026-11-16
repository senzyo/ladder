use super::{
    Colors, FeedbackScope, MainWindow, caption, control, error_message, page_title, panel,
    scroll_area,
};
use crate::{desktop, logging};
use gpui::{ClipboardItem, Context, Div, ScrollHandle, Stateful, div, prelude::*, px};

pub(super) struct LogView {
    lines: Vec<String>,
    warning: Option<String>,
    paused: bool,
    copied: bool,
    scroll: ScrollHandle,
}

impl LogView {
    pub(super) fn new() -> Self {
        let scroll = ScrollHandle::new();
        scroll.scroll_to_bottom();
        Self {
            lines: Vec::new(),
            warning: None,
            paused: false,
            copied: false,
            scroll,
        }
    }

    pub(super) fn refresh(&mut self) -> bool {
        let warning = logging::warning();
        let mut changed = warning != self.warning;
        self.warning = warning;
        if !self.paused {
            let lines = logging::recent();
            if lines != self.lines {
                let at_bottom = -self.scroll.offset().y >= self.scroll.max_offset().height - px(2.);
                self.lines = lines;
                self.copied = false;
                if at_bottom {
                    self.scroll.scroll_to_bottom();
                }
                changed = true;
            }
        }
        changed
    }

    pub(super) fn scroll_key(&self, key: &str) -> bool {
        let mut offset = self.scroll.offset();
        offset.y = match key {
            "pageup" => (offset.y + self.scroll.bounds().size.height * 0.9).min(px(0.)),
            "pagedown" => (offset.y - self.scroll.bounds().size.height * 0.9)
                .max(-self.scroll.max_offset().height),
            "home" => px(0.),
            "end" => -self.scroll.max_offset().height,
            _ => return false,
        };
        self.scroll.set_offset(offset);
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
        .text_size(px(13.))
        .border_color(colors.line)
        .bg(colors.panel)
        .child(label)
}

impl MainWindow {
    pub(super) fn logs_page(&self, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
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
                    .justify_between()
                    .gap(px(12.))
                    .child(page_title("应用日志"))
                    .child(caption(
                        format!("最近 {} 条", self.logs.lines.len()),
                        colors,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap(px(8.))
                    .child(
                        button(
                            if self.logs.paused {
                                "继续刷新"
                            } else {
                                "暂停刷新"
                            },
                            "pause-logs",
                            5,
                            colors,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.logs.paused = !view.logs.paused;
                            if !view.logs.paused {
                                view.logs.refresh();
                                view.logs.scroll.scroll_to_bottom();
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            if self.logs.copied {
                                "已复制"
                            } else {
                                "复制日志"
                            },
                            "copy-logs",
                            6,
                            colors,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                view.logs.lines.join("\n"),
                            ));
                            view.logs.copied = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        button("打开目录", "open-log-folder", 7, colors).on_click(cx.listener(
                            |view, _, _, cx| {
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
                                    view.set_command_result(
                                        FeedbackScope::Logs,
                                        Err("应用日志目录尚未初始化".into()),
                                    );
                                    cx.notify();
                                }
                            },
                        )),
                    ),
            )
            .when_some(
                self.command_error(FeedbackScope::Logs).cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when_some(
                self.logs
                    .warning
                    .as_ref()
                    .filter(|warning| Some(*warning) != self.command_error(FeedbackScope::Logs))
                    .cloned(),
                |el, warning| el.child(error_message(warning, colors)),
            )
            .child(
                panel(colors).flex_1().min_h(px(0.)).flex().child(
                    scroll_area(
                        "log-scroll",
                        div()
                            .id("log-lines")
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .h_full()
                            .flex()
                            .flex_col()
                            .p(px(12.))
                            .gap(px(6.))
                            .text_size(px(12.))
                            .when(self.logs.lines.is_empty(), |el| {
                                el.child(caption("暂无应用日志", colors))
                            })
                            .children(self.logs.lines.iter().enumerate().map(|(index, line)| {
                                let level = line
                                    .split_once(" [")
                                    .and_then(|(_, rest)| rest.split_once(']'))
                                    .map(|(level, _)| level);
                                div()
                                    .id(index)
                                    .flex_shrink_0()
                                    .w_full()
                                    .whitespace_normal()
                                    .text_color(match level {
                                        Some("ERROR") => colors.error,
                                        Some("WARN") => colors.warning,
                                        _ => colors.text,
                                    })
                                    .child(line.clone())
                            })),
                        &self.logs.scroll,
                        colors,
                    )
                    .flex_1(),
                ),
            )
    }
}
