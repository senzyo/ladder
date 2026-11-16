use crate::desktop;
use crate::runtime::{Action, Core, CoreOperation, Runtime, Setting, Snapshot, ThemePreference};
use crate::update::{UpdateChannel, UpdateProgress};
use gpui::{
    Context, Div, ElementId, FocusHandle, FontWeight, Rgba, ScrollHandle, SharedString, Stateful,
    Task, Timer, Window, WindowAppearance, div, prelude::*, px, rgb, rgba,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "log_view.rs"]
mod log_view;
use log_view::LogView;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Core(Core),
    Logs,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum FeedbackScope {
    Config(Core),
    Versions(Core),
    Settings,
    Logs,
}

#[derive(Clone)]
enum UiAction {
    Run(Action),
    OpenFolder(Core),
    Edit(Core, String),
    ShowVersions(Core, bool),
    ConfirmDelete(Core, String),
    CancelDelete,
}

pub(crate) struct MainWindow {
    runtime: Runtime,
    snapshot: Snapshot,
    page: Page,
    selected: [Option<String>; 2],
    config_focus: [HashMap<String, FocusHandle>; 2],
    config_scroll: [ScrollHandle; 2],
    pending_theme: Option<(ThemePreference, Task<()>)>,
    show_versions: [bool; 2],
    version_focus: [HashMap<String, [FocusHandle; 2]>; 2],
    version_scroll: [ScrollHandle; 2],
    confirm_delete: Option<(Core, String)>,
    titlebar_dark: Option<bool>,
    command_errors: Vec<(FeedbackScope, String)>,
    focus: FocusHandle,
    logs: LogView,
}

impl MainWindow {
    pub(crate) fn new(runtime: Runtime, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.observe_window_appearance(window, |view, _, cx| {
            // GPUI reapplies the system title bar when Windows changes appearance.
            view.titlebar_dark = None;
            cx.notify();
        })
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
        let mut view = Self {
            runtime,
            snapshot: Snapshot::default(),
            page: Page::Core(Core::SingBox),
            selected: [None, None],
            config_focus: Default::default(),
            config_scroll: Default::default(),
            pending_theme: None,
            show_versions: [false; 2],
            version_focus: Default::default(),
            version_scroll: Default::default(),
            confirm_delete: None,
            titlebar_dark: None,
            command_errors: Vec::new(),
            focus,
            logs: LogView::new(),
        };
        view.refresh(cx);
        if view.snapshot.settings_error.is_some()
            || view.snapshot.dns_recovery_error.is_some()
            || view.snapshot.dns_task_warning.is_some()
        {
            view.page = Page::Settings;
        }
        view
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.page == Page::Logs && self.logs.refresh() {
            cx.notify();
        }
        let next = self.runtime.snapshot();
        if next != self.snapshot {
            for core in Core::ALL {
                let handles = &mut self.version_focus[core_index(core)];
                *handles = next
                    .core(core)
                    .versions
                    .iter()
                    .map(|version| {
                        handles.remove_entry(&version.version).unwrap_or_else(|| {
                            (
                                version.version.clone(),
                                std::array::from_fn(|_| cx.focus_handle()),
                            )
                        })
                    })
                    .collect();
                let files = &next.core(core).configs;
                if *files == self.snapshot.core(core).configs {
                    continue;
                }
                let handles = &mut self.config_focus[core_index(core)];
                *handles = files
                    .iter()
                    .map(|name| {
                        handles
                            .remove_entry(name)
                            .unwrap_or_else(|| (name.clone(), cx.focus_handle()))
                    })
                    .collect();
                let selected = &mut self.selected[core_index(core)];
                if selected
                    .as_ref()
                    .is_some_and(|name| !handles.contains_key(name))
                {
                    *selected = None;
                }
            }
            self.snapshot = next;
            cx.notify();
        }
    }

    fn activate(&mut self, action: UiAction, cx: &mut Context<Self>) {
        let scope = match &action {
            UiAction::Run(
                Action::Start(core)
                | Action::Stop(core)
                | Action::Restart(core)
                | Action::Use(core, _),
            )
            | UiAction::OpenFolder(core)
            | UiAction::Edit(core, _) => FeedbackScope::Config(*core),
            UiAction::Run(
                Action::RefreshVersions(core)
                | Action::LoadMoreVersions(core)
                | Action::DownloadVersion(core, _)
                | Action::SwitchVersion(core, _)
                | Action::DeleteVersion(core, _),
            )
            | UiAction::ShowVersions(core, _)
            | UiAction::ConfirmDelete(core, _) => FeedbackScope::Versions(*core),
            UiAction::Run(Action::Set(..)) => FeedbackScope::Settings,
            UiAction::Run(Action::RetryDns) => FeedbackScope::Settings,
            UiAction::CancelDelete => {
                self.confirm_delete = None;
                cx.notify();
                return;
            }
        };
        let result = match action {
            UiAction::ShowVersions(core, show) => {
                self.show_versions[core_index(core)] = show;
                self.confirm_delete = None;
                let state = self.snapshot.core(core);
                if show && !state.versions_loaded && !state.versions_loading {
                    self.runtime.send(Action::RefreshVersions(core))
                } else {
                    cx.notify();
                    return;
                }
            }
            UiAction::ConfirmDelete(core, version) => {
                self.confirm_delete = Some((core, version));
                cx.notify();
                return;
            }
            UiAction::CancelDelete => unreachable!(),
            UiAction::Run(action) => {
                self.confirm_delete = None;
                self.runtime.send(action)
            }
            UiAction::OpenFolder(core) => {
                return Self::open_external(
                    self.runtime.config_dir(core),
                    desktop::open_folder,
                    scope,
                    cx,
                );
            }
            UiAction::Edit(core, name) => {
                return Self::open_external(
                    self.runtime.config_dir(core).join(name),
                    desktop::edit_config,
                    scope,
                    cx,
                );
            }
        };
        self.set_command_result(scope, result);
        self.refresh(cx);
        cx.notify();
    }

    fn open_external(
        path: PathBuf,
        open: fn(&Path) -> Result<(), String>,
        scope: FeedbackScope,
        cx: &mut Context<Self>,
    ) {
        // ShellExecute 会泵 Windows 消息，执行时不能持有 GPUI 的 App 或实体借用。
        cx.spawn(async move |view, cx| {
            let result = open(&path);
            let _ = view.update(cx, |view, cx| {
                view.set_command_result(scope, result);
                view.refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn selected_theme(&self) -> ThemePreference {
        self.pending_theme
            .as_ref()
            .map_or(self.snapshot.settings.theme, |(theme, _)| *theme)
    }

    fn command_error(&self, scope: FeedbackScope) -> Option<&String> {
        self.command_errors
            .iter()
            .find(|(source, _)| *source == scope)
            .map(|(_, message)| message)
    }

    fn set_command_result(&mut self, scope: FeedbackScope, result: Result<(), String>) {
        self.command_errors.retain(|(source, _)| *source != scope);
        if let Err(message) = result {
            crate::logging::error("desktop", &message);
            self.command_errors.push((scope, message));
        }
    }

    fn set_theme(&mut self, theme: ThemePreference, cx: &mut Context<Self>) {
        if self.selected_theme() == theme {
            return;
        }
        match self.runtime.set_theme(theme) {
            Ok(saved) => {
                self.set_command_result(FeedbackScope::Settings, Ok(()));
                let task = cx.spawn(async move |view, cx| {
                    let result = saved.recv().await;
                    let _ = view.update(cx, |view, cx| {
                        view.pending_theme = None;
                        if result.is_err() {
                            view.set_command_result(
                                FeedbackScope::Settings,
                                Err("主题设置未保存，管理线程已关闭。".into()),
                            );
                        }
                        view.refresh(cx);
                        cx.notify();
                    });
                });
                // Replacing the task cancels the previous preview's completion handler.
                self.pending_theme = Some((theme, task));
            }
            Err(message) => self.set_command_result(FeedbackScope::Settings, Err(message)),
        }
        cx.notify();
    }

    fn navigation(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let item = |page, label: &'static str, index| {
            let selected = self.page == page;
            control(label, index, colors, false)
                .w_full()
                .h(px(38.))
                .px(px(10.))
                .gap(px(10.))
                .when(selected, |el| {
                    el.bg(colors.selection)
                        .text_color(colors.accent)
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .child(
                    div()
                        .w(px(3.))
                        .h(px(16.))
                        .rounded(px(2.))
                        .when(selected, |el| el.bg(colors.accent)),
                )
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.page = page;
                    view.confirm_delete = None;
                    view.refresh(cx);
                    cx.notify();
                }))
        };
        div()
            .w(px(148.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .border_r_1()
            .border_color(colors.line)
            .px(px(10.))
            .py(px(20.))
            .gap(px(4.))
            .child(
                div()
                    .px(px(12.))
                    .mb(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(20.))
                            .child("Ladder"),
                    )
                    .child(caption("核心启动器", colors)),
            )
            .child(item(Page::Core(Core::SingBox), "sing-box", 1))
            .child(item(Page::Core(Core::Xray), "Xray", 2))
            .child(item(Page::Logs, "应用日志", 3))
            .child(div().flex_1())
            .child(item(Page::Settings, "软件设置", 4))
            .child(
                caption(format!("v{}", env!("CARGO_PKG_VERSION")), colors)
                    .px(px(12.))
                    .mt(px(8.)),
            )
    }

    fn core_page(&self, core: Core, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        if self.show_versions[core_index(core)] {
            return self.versions_page(core, colors, cx);
        }
        let state = self.snapshot.core(core);
        let idle = state.operation.is_none();
        let current_exists = state
            .current
            .as_ref()
            .is_some_and(|name| state.configs.contains(name));
        let can_use_config = idle && state.scan_error.is_none();
        let can_run = can_use_config && state.installed && current_exists;
        let (status, status_color) = match state.operation {
            Some(CoreOperation::Starting) => ("启动中", colors.accent),
            Some(CoreOperation::Stopping) => ("停止中", colors.accent),
            Some(CoreOperation::Switching) => ("切换中", colors.accent),
            Some(CoreOperation::Restarting) => ("重启中", colors.accent),
            Some(CoreOperation::Updating) | None if state.running => ("运行中", colors.running),
            Some(CoreOperation::Updating) | None if !state.installed => ("未安装", colors.muted),
            Some(CoreOperation::Updating) | None => ("已停止", colors.muted),
        };
        let current = state
            .current
            .clone()
            .unwrap_or_else(|| "尚未选择配置".into());
        let current_card = panel(colors)
            .flex_shrink_0()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(caption("当前配置", colors))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(current),
                            ),
                    )
                    .child(badge(status, status_color, colors)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(action_button(
                        "启动",
                        8,
                        (can_run && !state.running).then_some(UiAction::Run(Action::Start(core))),
                        true,
                        colors,
                        cx,
                    ))
                    .child(action_button(
                        "停止",
                        9,
                        (idle && state.running).then_some(UiAction::Run(Action::Stop(core))),
                        false,
                        colors,
                        cx,
                    ))
                    .child(action_button(
                        "重启",
                        10,
                        (can_run && state.running).then_some(UiAction::Run(Action::Restart(core))),
                        false,
                        colors,
                        cx,
                    )),
            );

        let mut files = div()
            .id(("files", core_index(core)))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.config_scroll[core_index(core)])
            .flex()
            .flex_col()
            .p(px(4.))
            .gap(px(2.));
        let selected = self.selected[core_index(core)].as_deref();
        for (index, name) in state.configs.iter().enumerate() {
            let current = state.current.as_ref() == Some(name);
            let highlighted = selected == Some(name.as_str());
            let file = name.clone();
            let tab_index = 13 + index as isize;
            // GPUI uses the tracked handle's tab order when a control supplies its own handle.
            let focus = self.config_focus[core_index(core)][name]
                .clone()
                .tab_stop(true)
                .tab_index(tab_index);
            let row = control(
                SharedString::from(format!("file-{}-{name}", core.label())),
                tab_index,
                colors,
                false,
            )
            .track_focus(&focus)
            .w_full()
            .min_h(px(36.))
            .flex_shrink_0()
            .px(px(10.))
            .py(px(6.))
            .gap(px(8.))
            .when(highlighted, |el| el.bg(colors.selection))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .when(!highlighted, |el| el.truncate())
                    .child(name.clone()),
            )
            .when(current, |el| el.child(badge("当前", colors.accent, colors)))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.selected[core_index(core)] = Some(file.clone());
                view.config_scroll[core_index(core)].scroll_to_item(index);
                view.set_command_result(FeedbackScope::Config(core), Ok(()));
                cx.notify();
            }));
            files = files.child(row);
        }
        if state.configs.is_empty() {
            files = files.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .items_center()
                    .gap(px(6.))
                    .p(px(16.))
                    .child("还没有配置文件")
                    .child(caption("打开目录，放入你的 JSON 配置。", colors)),
            );
        }

        let footer_index = 13 + state.configs.len() as isize;
        let edit = selected.map(|name| UiAction::Edit(core, name.to_owned()));
        let apply = selected
            .filter(|name| can_use_config && state.current.as_deref() != Some(*name))
            .map(|name| UiAction::Run(Action::Use(core, name.to_owned())));
        let selection_hint = match selected {
            Some(name) if state.current.as_deref() == Some(name) => {
                if state.running {
                    "当前配置 · 编辑后重启生效"
                } else {
                    "当前配置 · 编辑后启动生效"
                }
            }
            Some(_) => "已选中 · 使用后启动此配置",
            None => "选择配置后可编辑或使用",
        };
        let config_list = panel(colors)
            .flex_1()
            .min_h(px(120.))
            .flex()
            .flex_col()
            .child(files)
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .p(px(10.))
                    .border_t_1()
                    .border_color(colors.line)
                    .child(caption(selection_hint, colors))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(action_button("编辑", footer_index, edit, false, colors, cx))
                            .child(action_button(
                                "使用此配置",
                                footer_index + 1,
                                apply,
                                false,
                                colors,
                                cx,
                            )),
                    ),
            );

        div()
            .id(("core-content", core_index(core)))
            .overflow_y_scroll()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(page_title(if core == Core::Xray {
                                "Xray"
                            } else {
                                "sing-box"
                            }))
                            .child(caption(
                                match &state.version {
                                    Some(version) => format!("当前版本 v{version}"),
                                    None if !state.installed => "尚未安装核心".into(),
                                    None => "当前版本未知".into(),
                                },
                                colors,
                            )),
                    )
                    .child(action_button(
                        "管理版本",
                        7,
                        Some(UiAction::ShowVersions(core, true)),
                        !state.installed,
                        colors,
                        cx,
                    )),
            )
            .when_some(
                self.command_error(FeedbackScope::Config(core)).cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .child(current_card)
            .when(
                state.current.is_some() && !current_exists && state.scan_error.is_none(),
                |el| {
                    el.child(error_message(
                        "当前配置文件已移走，请重新选择配置。".into(),
                        colors,
                    ))
                },
            )
            .when_some(state.run_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("配置文件"))
                            .child(caption(format!("{} 个", state.configs.len()), colors)),
                    )
                    .child(action_button(
                        "打开目录",
                        11,
                        Some(UiAction::OpenFolder(core)),
                        false,
                        colors,
                        cx,
                    )),
            )
            .when_some(state.scan_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(config_list)
    }

    fn versions_page(&self, core: Core, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        let state = self.snapshot.core(core);
        let can_manage = state.operation.is_none() && state.update.is_none();
        let mut rows = div()
            .id(("version-list", core_index(core)))
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.version_scroll[core_index(core)])
            .flex()
            .flex_col();
        for (index, version) in state.versions.iter().enumerate() {
            let current = state.selected_version.as_ref() == Some(&version.version);
            let downloading = state.update.is_some()
                && state.downloading_version.as_ref() == Some(&version.version);
            let confirming = self
                .confirm_delete
                .as_ref()
                .is_some_and(|(target, name)| *target == core && name == &version.version);
            let (label, color) = match version.channel {
                UpdateChannel::Stable => ("稳定版", colors.running),
                UpdateChannel::Preview => ("预览版", colors.warning),
            };
            let actions = if confirming {
                vec![
                    (
                        "确认删除",
                        can_manage.then_some(UiAction::Run(Action::DeleteVersion(
                            core,
                            version.version.clone(),
                        ))),
                    ),
                    ("取消", Some(UiAction::CancelDelete)),
                ]
            } else if version.installed {
                vec![
                    (
                        if current { "当前使用" } else { "切换" },
                        (can_manage && !current).then_some(UiAction::Run(Action::SwitchVersion(
                            core,
                            version.version.clone(),
                        ))),
                    ),
                    (
                        "删除",
                        (can_manage && !current)
                            .then_some(UiAction::ConfirmDelete(core, version.version.clone())),
                    ),
                ]
            } else {
                vec![(
                    if downloading { "下载中" } else { "下载" },
                    (can_manage && version.release_tag.is_some()).then_some(UiAction::Run(
                        Action::DownloadVersion(core, version.version.clone()),
                    )),
                )]
            };
            let mut buttons = div().flex().flex_shrink_0().gap(px(8.));
            for (slot, (label, action)) in actions.into_iter().enumerate() {
                let tab_index = 8 + (index * 2 + slot) as isize;
                let focus = self.version_focus[core_index(core)][&version.version][slot]
                    .clone()
                    .tab_stop(action.is_some())
                    .tab_index(tab_index);
                buttons = buttons.child(
                    action_button(label, tab_index, action, false, colors, cx)
                        .id(SharedString::from(format!(
                            "version-{}-{}-{slot}",
                            core.label(),
                            version.version
                        )))
                        .track_focus(&focus)
                        .when(confirming && slot == 0, |el| el.text_color(colors.error)),
                );
            }
            rows = rows.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(12.))
                    .when(index > 0, |el| el.border_t_1().border_color(colors.line))
                    .when(current, |el| el.bg(colors.selection))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(100.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("v{}", version.version)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(badge(label, color, colors))
                                    .when(version.installed && !current, |el| {
                                        el.child(caption("已下载", colors))
                                    })
                                    .when(confirming, |el| {
                                        el.child(
                                            caption("删除此本地版本？", colors)
                                                .text_color(colors.error),
                                        )
                                    }),
                            ),
                    )
                    .child(buttons),
            );
        }
        if state.versions.is_empty() {
            rows = rows.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(24.))
                    .child(caption(
                        if state.versions_loading {
                            "正在获取版本列表…"
                        } else {
                            "暂无可用版本，请刷新列表。"
                        },
                        colors,
                    )),
            );
        }
        let footer_index = 8 + (state.versions.len() * 2) as isize;
        div()
            .id(("versions-content", core_index(core)))
            .size_full()
            .overflow_y_scroll()
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
                    .child(page_title(if core == Core::Xray {
                        "Xray 版本"
                    } else {
                        "sing-box 版本"
                    }))
                    .child(action_button(
                        "返回配置",
                        5,
                        Some(UiAction::ShowVersions(core, false)),
                        false,
                        colors,
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(caption(
                        match &state.selected_version {
                            Some(version) => format!("当前使用 v{version}"),
                            None => "下载后选择一个版本使用".into(),
                        },
                        colors,
                    ))
                    .child(action_button(
                        if state.versions_loading {
                            "获取中"
                        } else {
                            "刷新列表"
                        },
                        6,
                        (!state.versions_loading)
                            .then_some(UiAction::Run(Action::RefreshVersions(core))),
                        false,
                        colors,
                        cx,
                    )),
            )
            .when_some(state.update.as_ref(), |el, progress| {
                let version = state.downloading_version.as_deref().unwrap_or("");
                el.child(
                    caption(
                        format!("v{version} · {}", update_progress_text(progress)),
                        colors,
                    )
                    .flex_shrink_0()
                    .text_color(colors.accent),
                )
            })
            .when_some(state.update_message.clone(), |el, message| {
                el.child(caption(message, colors).flex_shrink_0())
            })
            .when_some(state.update_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .when_some(
                self.command_error(FeedbackScope::Versions(core)).cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when_some(
                state
                    .versions_error
                    .as_ref()
                    .or(state.local_versions_error.as_ref())
                    .cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when_some(state.version_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(
                panel(colors)
                    .flex_1()
                    .min_h(px(120.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(rows),
            )
            .when(state.versions_has_more, |el| {
                el.child(action_button(
                    "加载更多",
                    footer_index,
                    (!state.versions_loading)
                        .then_some(UiAction::Run(Action::LoadMoreVersions(core))),
                    false,
                    colors,
                    cx,
                ))
            })
    }

    fn settings_page(&self, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        let settings = &self.snapshot.settings;
        let mut choices = div()
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .bg(colors.canvas)
            .border_1()
            .border_color(colors.line)
            .rounded(px(6.));
        for (index, (theme, label)) in [
            (ThemePreference::System, "跟随系统"),
            (ThemePreference::Light, "浅色"),
            (ThemePreference::Dark, "深色"),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.selected_theme() == theme;
            choices = choices.child(
                control(label, 5 + index as isize, colors, false)
                    .h(px(28.))
                    .px(px(8.))
                    .when(selected, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| view.set_theme(theme, cx))),
            );
        }
        let mut rows = panel(colors).flex().flex_col().p(px(4.)).child(
            div()
                .flex()
                .min_h(px(52.))
                .px(px(12.))
                .gap(px(12.))
                .items_center()
                .justify_between()
                .child("主题")
                .child(choices),
        );
        for (index, (label, description, key, value)) in [
            (
                "启动时隐藏主窗口",
                "从系统托盘打开 Ladder",
                Setting::MinimizeToTray,
                settings.minimize_to_tray,
            ),
            (
                "随 Ladder 启动 sing-box",
                "使用上次选择的 sing-box 配置",
                Setting::AutoSingBox,
                settings.auto_sing_box,
            ),
            (
                "随 Ladder 启动 Xray",
                "使用上次选择的 Xray 配置",
                Setting::AutoXray,
                settings.auto_xray,
            ),
            (
                "劫持DNS到本地回环",
                "下次启动任一核心时，将物理网卡 DNS 设为 127.0.0.1 和 ::1",
                Setting::HijackDns,
                settings.hijack_dns,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let track = div()
                .w(px(32.))
                .h(px(18.))
                .px(px(3.))
                .rounded_full()
                .bg(if value {
                    colors.accent_fill
                } else {
                    colors.switch_off
                })
                .flex()
                .items_center()
                .when(value, |el| el.justify_end())
                .child(
                    div()
                        .w(px(12.))
                        .h(px(12.))
                        .rounded_full()
                        .bg(colors.on_accent),
                );
            rows = rows.child(
                control(label, 8 + index as isize, colors, false)
                    .min_h(px(68.))
                    .px(px(12.))
                    .py(px(10.))
                    .gap(px(12.))
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(label)
                            .child(caption(description, colors)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(caption(if value { "开启" } else { "关闭" }, colors))
                            .child(track),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.activate(UiAction::Run(Action::Set(key, !value)), cx);
                    })),
            );
        }
        div()
            .id("settings-content")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(page_title("软件设置"))
            .child(caption("设置更改后自动保存", colors))
            .when_some(
                self.command_error(FeedbackScope::Settings)
                    .or(self.snapshot.settings_error.as_ref())
                    .cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when_some(self.snapshot.dns_recovery_error.clone(), |el, message| {
                let busy = self.snapshot.dns_recovery_busy;
                el.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(error_message(message, colors)),
                        )
                        .child(action_button(
                            if busy {
                                "正在恢复 DNS"
                            } else {
                                "重试恢复 DNS"
                            },
                            12,
                            (!busy).then_some(UiAction::Run(Action::RetryDns)),
                            false,
                            colors,
                            cx,
                        )),
                )
            })
            .when_some(self.snapshot.dns_task_warning.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(rows)
            .child(caption("关闭窗口后，Ladder 将继续在系统托盘运行。", colors))
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.selected_theme().is_dark(window.appearance());
        if self.titlebar_dark != Some(dark) {
            desktop::set_titlebar_dark(window, dark);
            self.titlebar_dark = Some(dark);
        }
        let colors = Colors::new(dark);
        let page = match self.page {
            Page::Core(core) => self.core_page(core, colors, cx),
            Page::Logs => self.logs_page(colors, cx),
            Page::Settings => self.settings_page(colors, cx),
        };
        div()
            .id("ladder")
            .track_focus(&self.focus)
            .tab_group()
            .size_full()
            .flex()
            .font_family("Microsoft YaHei UI")
            .text_size(px(14.))
            .text_color(colors.text)
            .bg(colors.canvas)
            .on_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" {
                    if event.keystroke.modifiers.shift {
                        window.focus_prev();
                    } else {
                        window.focus_next();
                    }
                    if let Page::Core(core) = view.page {
                        if view.show_versions[core_index(core)] {
                            let handles = &view.version_focus[core_index(core)];
                            if let Some(index) =
                                view.snapshot
                                    .core(core)
                                    .versions
                                    .iter()
                                    .position(|version| {
                                        handles.get(&version.version).is_some_and(|handles| {
                                            handles.iter().any(|handle| handle.is_focused(window))
                                        })
                                    })
                            {
                                view.version_scroll[core_index(core)].scroll_to_item(index);
                                cx.notify();
                            }
                        }
                        let handles = &view.config_focus[core_index(core)];
                        if let Some(index) =
                            view.snapshot.core(core).configs.iter().position(|name| {
                                handles
                                    .get(name)
                                    .is_some_and(|handle| handle.is_focused(window))
                            })
                        {
                            view.config_scroll[core_index(core)].scroll_to_item(index);
                            cx.notify();
                        }
                    }
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" && view.confirm_delete.is_some() {
                    view.confirm_delete = None;
                    cx.notify();
                    cx.stop_propagation();
                } else if view.page == Page::Logs && view.logs.scroll_key(&event.keystroke.key) {
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .child(self.navigation(colors, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .p(px(20.))
                    .gap(px(10.))
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

fn update_progress_text(progress: &UpdateProgress) -> String {
    match progress {
        UpdateProgress::Checking => "正在获取官方版本信息…".into(),
        UpdateProgress::Downloading { received, total } => {
            let downloaded = *received as f64 / 1_048_576.0;
            if *total == 0 {
                format!("正在下载 · {downloaded:.1} MiB")
            } else {
                let percent = (*received as f64 / *total as f64 * 100.0).min(100.0);
                let size = *total as f64 / 1_048_576.0;
                format!("正在下载 · {percent:.0}% · {downloaded:.1} / {size:.1} MiB")
            }
        }
        UpdateProgress::Verifying => "正在校验下载文件…".into(),
        UpdateProgress::Extracting => "正在准备安装文件…".into(),
        UpdateProgress::Installing => "正在保存核心版本…".into(),
    }
}

fn page_title(label: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .text_size(px(22.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn caption(text: impl Into<SharedString>, colors: Colors) -> Div {
    div()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(text.into())
}

fn panel(colors: Colors) -> Div {
    div()
        .bg(colors.panel)
        .border_1()
        .border_color(colors.line)
        .rounded(px(8.))
}

fn badge(label: &'static str, color: Rgba, colors: Colors) -> Div {
    div()
        .flex_shrink_0()
        .px(px(7.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(colors.canvas)
        .text_color(color)
        .text_size(px(12.))
        .child(label)
}

fn error_message(message: String, colors: Colors) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("error-{message}")))
        .flex_shrink_0()
        .max_h(px(64.))
        .overflow_y_scroll()
        .px(px(10.))
        .py(px(8.))
        .border_l_2()
        .border_color(colors.error)
        .bg(colors.error_bg)
        .text_color(colors.error)
        .text_size(px(12.))
        .child(message)
}

fn control(id: impl Into<ElementId>, index: isize, colors: Colors, primary: bool) -> Stateful<Div> {
    let (hover, pressed, focus) = if primary {
        (colors.accent_hover, colors.accent_fill, colors.text)
    } else {
        (colors.hover, colors.selection, colors.accent)
    };
    div()
        .id(id)
        .tab_index(index)
        .cursor_pointer()
        .flex()
        .items_center()
        .border_1()
        .border_color(rgba(0))
        .rounded(px(5.))
        .hover(|el| el.bg(hover))
        .active(|el| el.bg(pressed))
        .focus(|el| el.border_color(focus))
}

fn action_button(
    label: &'static str,
    index: isize,
    action: Option<UiAction>,
    primary: bool,
    colors: Colors,
    cx: &Context<MainWindow>,
) -> Stateful<Div> {
    let enabled = action.is_some();
    let base = if enabled {
        control(label, index, colors, primary)
    } else {
        div()
            .id(label)
            .flex()
            .items_center()
            .rounded(px(5.))
            .border_1()
    };
    base.flex_shrink_0()
        .justify_center()
        .h(px(30.))
        .min_w(px(58.))
        .px(px(12.))
        .text_size(px(13.))
        .border_color(colors.line)
        .text_color(if enabled {
            colors.text
        } else {
            colors.disabled
        })
        .bg(if enabled { colors.panel } else { colors.canvas })
        .when(primary && enabled, |el| {
            el.bg(colors.accent_fill)
                .border_color(colors.accent_fill)
                .text_color(colors.on_accent)
        })
        .child(label)
        .when_some(action, |el, action| {
            el.on_click(cx.listener(move |view, _, _, cx| view.activate(action.clone(), cx)))
        })
}

#[derive(Clone, Copy)]
struct Colors {
    canvas: Rgba,
    panel: Rgba,
    sidebar: Rgba,
    text: Rgba,
    muted: Rgba,
    line: Rgba,
    accent: Rgba,
    accent_fill: Rgba,
    accent_hover: Rgba,
    on_accent: Rgba,
    running: Rgba,
    selection: Rgba,
    hover: Rgba,
    disabled: Rgba,
    switch_off: Rgba,
    warning: Rgba,
    error: Rgba,
    error_bg: Rgba,
}

impl ThemePreference {
    fn is_dark(self, system: WindowAppearance) -> bool {
        match self {
            Self::System => matches!(
                system,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

impl Colors {
    fn new(dark: bool) -> Self {
        let tone = |light, night| rgb(if dark { night } else { light });
        Self {
            canvas: tone(0xf8f9fb, 0x1c1e22),
            panel: tone(0xffffff, 0x24272c),
            sidebar: tone(0xf0f2f5, 0x202226),
            text: tone(0x202631, 0xeff1f5),
            muted: tone(0x586174, 0xb3bccb),
            line: tone(0xd9dee7, 0x424852),
            accent: tone(0x185bb8, 0x9bc3ff),
            accent_fill: tone(0x245fba, 0x9bc3ff),
            accent_hover: tone(0x1c4e9d, 0xb7d3ff),
            on_accent: tone(0xffffff, 0x172841),
            running: tone(0x18704a, 0x83d9b1),
            selection: tone(0xe3edfc, 0x2c3b51),
            hover: tone(0xe9edf3, 0x303740),
            disabled: tone(0x858c98, 0x7f8998),
            switch_off: tone(0x737c8b, 0x657184),
            warning: tone(0x8a6500, 0xf2cc60),
            error: tone(0xa62a36, 0xffadb5),
            error_bg: tone(0xfff0f1, 0x3c292e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_system_theme_tracks_changes_in_window_appearance() {
        for (appearance, dark) in [
            (WindowAppearance::Light, false),
            (WindowAppearance::Dark, true),
            (WindowAppearance::VibrantLight, false),
            (WindowAppearance::VibrantDark, true),
        ] {
            assert_eq!(ThemePreference::System.is_dark(appearance), dark);
            assert!(!ThemePreference::Light.is_dark(appearance));
            assert!(ThemePreference::Dark.is_dark(appearance));
        }
    }
}
