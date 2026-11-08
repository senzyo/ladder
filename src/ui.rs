use crate::runtime::{Action, Core, Runtime, Setting, Snapshot};
use gpui::{
    Context, Div, ElementId, FocusHandle, FontWeight, Rgba, SharedString, Stateful, Timer, Window,
    WindowAppearance, div, prelude::*, px, rgb, rgba,
};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Core(Core),
    Settings,
}

pub(crate) struct MainWindow {
    runtime: Runtime,
    snapshot: Snapshot,
    page: Page,
    selected: [Option<String>; 2],
    command_error: Option<String>,
    focus: FocusHandle,
}

impl MainWindow {
    pub(crate) fn new(runtime: Runtime, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let snapshot = runtime.snapshot();
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        cx.spawn(async move |view, cx| {
            loop {
                Timer::after(Duration::from_millis(250)).await;
                if view.update(cx, |view, cx| view.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            runtime,
            snapshot,
            page: Page::Core(Core::SingBox),
            selected: [None, None],
            command_error: None,
            focus,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let next = self.runtime.snapshot();
        if next != self.snapshot {
            for core in [Core::SingBox, Core::Xray] {
                let selected = &mut self.selected[core_index(core)];
                if selected
                    .as_ref()
                    .is_some_and(|name| !next.core(core).configs.contains(name))
                {
                    *selected = None;
                }
            }
            self.snapshot = next;
            cx.notify();
        }
    }

    fn send(&mut self, action: Action, cx: &mut Context<Self>) {
        self.command_error = self.runtime.send(action).err();
        self.refresh(cx);
        cx.notify();
    }

    fn navigation(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let item = |page, label: &'static str, index| {
            let selected = self.page == page;
            control(label, index, colors)
                .w_full()
                .h(px(36.))
                .px(px(9.))
                .gap(px(9.))
                .when(selected, |el| {
                    el.bg(colors.selection).text_color(colors.accent)
                })
                .child(
                    div()
                        .w(px(3.))
                        .h(px(15.))
                        .rounded(px(2.))
                        .when(selected, |el| el.bg(colors.accent)),
                )
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.page = page;
                    view.command_error = None;
                    cx.notify();
                }))
        };
        div()
            .w(px(155.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .border_r_1()
            .border_color(colors.line)
            .px(px(10.))
            .py(px(18.))
            .gap(px(4.))
            .child(
                div()
                    .px(px(12.))
                    .mb(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(18.))
                    .child("Ladder"),
            )
            .child(item(Page::Core(Core::SingBox), "sing-box", 1))
            .child(item(Page::Core(Core::Xray), "xray", 2))
            .child(div().flex_1())
            .child(item(Page::Settings, "软件设置", 3))
    }

    fn core_page(&self, core: Core, colors: Colors, cx: &Context<Self>) -> Div {
        let state = self.snapshot.core(core);
        let (status, status_color) = if state.busy {
            ("切换中", colors.accent)
        } else if state.running {
            ("运行中", colors.running)
        } else {
            ("已停止", colors.muted)
        };
        let can_start = !state.busy
            && !state.running
            && state
                .current
                .as_ref()
                .is_some_and(|name| state.configs.contains(name));
        let can_stop = !state.busy && state.running;
        let action_button = |label: &'static str, index, enabled, start| {
            button(label, index, enabled, colors)
                .child(label)
                .when(enabled, |el| {
                    el.on_click(cx.listener(move |view, _, _, cx| {
                        view.send(
                            if start {
                                Action::Start(core)
                            } else {
                                Action::Stop(core)
                            },
                            cx,
                        );
                    }))
                })
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child("当前配置"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(
                                        state
                                            .current
                                            .clone()
                                            .unwrap_or_else(|| "未选择配置".into()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .text_size(px(12.))
                                    .text_color(status_color)
                                    .child(
                                        div().w(px(6.)).h(px(6.)).rounded_full().bg(status_color),
                                    )
                                    .child(status),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .gap(px(6.))
                    .child(action_button("启动", 4, can_start, true))
                    .child(action_button("停止", 5, can_stop, false)),
            );

        let mut files = div()
            .id(("files", core_index(core)))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(3.));
        let selected = self.selected[core_index(core)].as_deref();
        for (index, name) in state.configs.iter().enumerate() {
            let current = state.current.as_ref() == Some(name);
            let highlighted = selected == Some(name.as_str());
            let row_color = if highlighted {
                colors.selection
            } else {
                colors.panel
            };
            let file = name.clone();
            let tab_index = 10 + index as isize * 2;
            let row = control(
                SharedString::from(format!("file-{}-{name}", core.label())),
                tab_index,
                colors,
            )
            .flex_1()
            .min_w(px(0.))
            .min_h(px(37.))
            .py(px(5.))
            .px(px(9.))
            .gap(px(8.))
            .child(
                div()
                    .w(px(14.))
                    .flex_shrink_0()
                    .text_color(colors.accent)
                    .child(if current { "✓" } else { "" }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .when(!highlighted, |el| el.truncate())
                    .child(name.clone()),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                view.selected[core_index(core)] = Some(file.clone());
                cx.notify();
            }));
            let mut line = div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .pr(px(5.))
                .bg(row_color)
                .rounded(px(5.))
                .child(row);
            if highlighted && !current {
                let file = name.clone();
                line = line.child(
                    button("use-config", tab_index + 1, !state.busy, colors)
                        .text_color(if state.busy {
                            colors.disabled
                        } else {
                            colors.accent
                        })
                        .child("使用此配置")
                        .when(!state.busy, |el| {
                            el.on_click(cx.listener(move |view, _, _, cx| {
                                view.send(Action::Use(core, file.clone()), cx);
                            }))
                        }),
                );
            }
            files = files.child(line);
        }
        if state.configs.is_empty() {
            files = files.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .py(px(20.))
                    .px(px(12.))
                    .child("此目录下没有 JSON 配置文件")
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(self.runtime.config_dir(core).display().to_string()),
                    ),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(header)
            .when_some(state.error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(
                div()
                    .mt(px(5.))
                    .border_t_1()
                    .border_color(colors.line)
                    .pt(px(15.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("配置文件"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.muted)
                            .child(format!("{} 个文件", state.configs.len())),
                    ),
            )
            .child(files)
    }

    fn settings_page(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let settings = &self.snapshot.settings;
        let mut page = div().size_full().flex().flex_col().gap(px(4.)).child(
            div()
                .mb(px(14.))
                .text_size(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("软件设置"),
        );
        for (index, (label, key, value)) in [
            (
                "启动后最小化到托盘",
                Setting::MinimizeToTray,
                settings.minimize_to_tray,
            ),
            (
                "自动启动 sing-box",
                Setting::AutoSingBox,
                settings.auto_sing_box,
            ),
            ("自动启动 xray", Setting::AutoXray, settings.auto_xray),
        ]
        .into_iter()
        .enumerate()
        {
            let track = div()
                .w(px(34.))
                .h(px(19.))
                .px(px(3.))
                .rounded_full()
                .bg(if value {
                    colors.accent
                } else {
                    colors.switch_off
                })
                .flex()
                .items_center()
                .when(value, |el| el.justify_end())
                .child(div().w(px(13.)).h(px(13.)).rounded_full().bg(rgb(0xffffff)));
            page = page.child(
                control(label, 4 + index as isize, colors)
                    .h(px(48.))
                    .px(px(10.))
                    .justify_between()
                    .child(label)
                    .child(
                        div()
                            .flex()
                            .gap(px(9.))
                            .items_center()
                            .child(
                                div()
                                    .text_color(colors.muted)
                                    .text_size(px(12.))
                                    .child(if value { "开启" } else { "关闭" }),
                            )
                            .child(track),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send(Action::Set(key, !value), cx);
                    })),
            );
        }
        page
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::new(window.appearance());
        let page = match self.page {
            Page::Core(core) => self.core_page(core, colors, cx),
            Page::Settings => self.settings_page(colors, cx),
        };
        div()
            .id("ladder")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .font_family("Microsoft YaHei UI")
            .text_size(px(14.))
            .text_color(colors.text)
            .bg(colors.panel)
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    cx.stop_propagation();
                }
            })
            .child(self.navigation(colors, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .p(px(22.))
                    .gap(px(10.))
                    .when_some(
                        self.command_error
                            .clone()
                            .or_else(|| self.snapshot.error.clone()),
                        |el, message| el.child(error_message(message, colors)),
                    )
                    .child(div().flex_1().min_h(px(0.)).child(page)),
            )
    }
}

fn core_index(core: Core) -> usize {
    match core {
        Core::SingBox => 0,
        Core::Xray => 1,
    }
}

fn error_message(message: String, colors: Colors) -> Div {
    div()
        .flex_shrink_0()
        .px(px(9.))
        .py(px(6.))
        .border_l_2()
        .border_color(colors.error)
        .text_color(colors.error)
        .text_size(px(13.))
        .child(message)
}

fn control(id: impl Into<ElementId>, index: isize, colors: Colors) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(index)
        .cursor_pointer()
        .flex()
        .items_center()
        .border_1()
        .border_color(rgba(0))
        .rounded(px(5.))
        .hover(|el| el.bg(colors.hover))
        .focus(|el| el.border_color(colors.accent))
}

fn button(id: &'static str, index: isize, enabled: bool, colors: Colors) -> Stateful<Div> {
    let base = if enabled {
        control(id, index, colors)
    } else {
        div()
            .id(id)
            .flex()
            .items_center()
            .text_color(colors.disabled)
            .rounded(px(5.))
            .border_1()
    };
    base.flex_shrink_0()
        .justify_center()
        .h(px(29.))
        .min_w(px(55.))
        .px(px(9.))
        .text_size(px(13.))
        .border_color(colors.line)
        .bg(if enabled {
            colors.panel
        } else {
            colors.sidebar
        })
}

#[derive(Clone, Copy)]
struct Colors {
    panel: Rgba,
    sidebar: Rgba,
    text: Rgba,
    muted: Rgba,
    line: Rgba,
    accent: Rgba,
    running: Rgba,
    selection: Rgba,
    hover: Rgba,
    disabled: Rgba,
    switch_off: Rgba,
    error: Rgba,
}

impl Colors {
    fn new(appearance: WindowAppearance) -> Self {
        let dark = matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let tone = |light, night| rgb(if dark { night } else { light });
        Self {
            panel: tone(0xffffff, 0x202124),
            sidebar: tone(0xf4f5f7, 0x191a1d),
            text: tone(0x202326, 0xf0f1f4),
            muted: tone(0x565c64, 0xb4bac3),
            line: tone(0xd0d4da, 0x444851),
            accent: tone(0x115da7, 0x86bcff),
            running: tone(0x18744c, 0x7dd6a6),
            selection: tone(0xe7f1fd, 0x253953),
            hover: tone(0xe9edf2, 0x30353d),
            disabled: tone(0x868b93, 0x80858e),
            switch_off: tone(0x727985, 0x606774),
            error: tone(0xa32c2d, 0xff9c98),
        }
    }
}
