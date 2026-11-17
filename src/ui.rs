use crate::app_updater::{AppUpdateState, AppUpdater};
use crate::desktop;
use crate::runtime::{
    Action, ConfigValidation, Core, CoreOperation, Runtime, Setting, Snapshot, ThemePreference,
};
use crate::update::{UpdateChannel, UpdateProgress};
use gpui::{
    Animation, AnimationExt, App, Context, Div, ElementId, FocusHandle, FontWeight, Image,
    ImageFormat, PathBuilder, Rgba, ScrollHandle, SharedString, Stateful, Task, Timer, Window,
    WindowAppearance, canvas, div, img, point, prelude::*, px, rgb, rgba,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "log_view.rs"]
mod log_view;
use log_view::LogView;

#[path = "scrollbar.rs"]
mod scrollbar;
use scrollbar::scrollbar;

#[path = "proxy_input.rs"]
mod proxy_input;
use proxy_input::ProxyInput;

const TEXT_PRIMARY: f32 = 14.;
const TEXT_SECONDARY: f32 = 12.;

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
    GithubProxy,
    Logs,
}

#[derive(Clone, PartialEq)]
enum DeleteTarget {
    Config(Core, String),
    Version(Core, String),
}

#[derive(Clone)]
enum UiAction {
    Run(Action),
    OpenFolder(Core),
    OpenCoreFolder(Core),
    Edit(Core, String),
    ShowVersions(Core, bool),
    SetDownloadedOnly(Core, bool),
    ConfirmDelete(DeleteTarget),
    CheckAppUpdate,
    DownloadAppUpdate,
    InstallAppUpdate,
}

pub(crate) struct MainWindow {
    runtime: Runtime,
    snapshot: Snapshot,
    page: Page,
    config_focus: [HashMap<String, [FocusHandle; 4]>; 3],
    config_scroll: [ScrollHandle; 3],
    core_page_scroll: [ScrollHandle; 3],
    pending_theme: Option<(ThemePreference, Task<()>)>,
    show_versions: [bool; 3],
    versions_auto_requested: [bool; 3],
    downloaded_only: [bool; 3],
    version_focus: [HashMap<String, [FocusHandle; 2]>; 3],
    version_scroll: [ScrollHandle; 3],
    version_page_scroll: [ScrollHandle; 3],
    settings_scroll: ScrollHandle,
    confirm_delete: Option<DeleteTarget>,
    titlebar_dark: Option<bool>,
    command_errors: Vec<(FeedbackScope, String)>,
    focus: FocusHandle,
    logs: LogView,
    icon: Arc<Image>,
    github_proxy: ProxyInput,
    pending_github_proxy: Option<Task<()>>,
    updater: AppUpdater,
    app_update: AppUpdateState,
    events: async_channel::Sender<crate::TrayAction>,
}

impl MainWindow {
    pub(crate) fn new(
        runtime: Runtime,
        updater: AppUpdater,
        events: async_channel::Sender<crate::TrayAction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let github_proxy = ProxyInput::new(runtime.snapshot().settings.github_proxy, cx);
        let mut view = Self {
            runtime,
            snapshot: Snapshot::default(),
            page: Page::Core(Core::Mihomo),
            config_focus: Default::default(),
            config_scroll: Default::default(),
            core_page_scroll: Default::default(),
            pending_theme: None,
            show_versions: [false; 3],
            versions_auto_requested: [false; 3],
            downloaded_only: [false; 3],
            version_focus: Default::default(),
            version_scroll: Default::default(),
            version_page_scroll: Default::default(),
            settings_scroll: Default::default(),
            confirm_delete: None,
            titlebar_dark: None,
            command_errors: Vec::new(),
            focus,
            logs: LogView::new(),
            icon: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../assets/ladder.png").to_vec(),
            )),
            github_proxy,
            pending_github_proxy: None,
            updater,
            app_update: AppUpdateState::Idle,
            events,
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
        let app_update = self.updater.snapshot();
        if app_update != self.app_update {
            self.app_update = app_update;
            cx.notify();
        }
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
                        handles.remove_entry(name).unwrap_or_else(|| {
                            (name.clone(), std::array::from_fn(|_| cx.focus_handle()))
                        })
                    })
                    .collect();
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
                | Action::SwitchConfig(core, _)
                | Action::ValidateConfig(core, _)
                | Action::ValidateAllConfigs(core)
                | Action::DeleteConfig(core, _),
            )
            | UiAction::OpenFolder(core)
            | UiAction::Edit(core, _)
            | UiAction::ConfirmDelete(DeleteTarget::Config(core, _)) => {
                FeedbackScope::Config(*core)
            }
            UiAction::Run(
                Action::RefreshVersions(core)
                | Action::LoadMoreVersions(core)
                | Action::DownloadVersion(core, _)
                | Action::SwitchVersion(core, _)
                | Action::DeleteVersion(core, _),
            )
            | UiAction::ShowVersions(core, _)
            | UiAction::OpenCoreFolder(core)
            | UiAction::SetDownloadedOnly(core, _)
            | UiAction::ConfirmDelete(DeleteTarget::Version(core, _)) => {
                FeedbackScope::Versions(*core)
            }
            UiAction::Run(Action::Set(..)) => FeedbackScope::Settings,
            UiAction::Run(Action::RetryDns) => FeedbackScope::Settings,
            UiAction::CheckAppUpdate | UiAction::DownloadAppUpdate | UiAction::InstallAppUpdate => {
                FeedbackScope::Settings
            }
        };
        let result = match action {
            UiAction::CheckAppUpdate => self
                .updater
                .check(self.snapshot.settings.github_request_prefix().to_owned()),
            UiAction::DownloadAppUpdate => self
                .updater
                .download(self.snapshot.settings.github_request_prefix().to_owned()),
            UiAction::InstallAppUpdate => self
                .events
                .try_send(crate::TrayAction::InstallUpdate)
                .map_err(|_| "无法开始软件更新, 托盘管理线程已关闭。".into()),
            UiAction::ShowVersions(core, show) => {
                let index = core_index(core);
                self.show_versions[index] = show;
                self.confirm_delete = None;
                if !show || self.versions_auto_requested[index] {
                    self.refresh(cx);
                    cx.notify();
                    return;
                }
                self.versions_auto_requested[index] = true;
                if !self.snapshot.core(core).versions_loading {
                    self.runtime.send(Action::RefreshVersions(core))
                } else {
                    Ok(())
                }
            }
            UiAction::SetDownloadedOnly(core, downloaded_only) => {
                let index = core_index(core);
                if self.downloaded_only[index] == downloaded_only {
                    return;
                }
                self.downloaded_only[index] = downloaded_only;
                self.confirm_delete = None;
                self.version_scroll[index].set_offset(gpui::point(px(0.), px(0.)));
                cx.notify();
                return;
            }
            UiAction::ConfirmDelete(target) => {
                self.confirm_delete = Some(target);
                cx.notify();
                return;
            }
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
            UiAction::OpenCoreFolder(core) => {
                return Self::open_external(
                    self.runtime.core_dir(core),
                    desktop::open_folder,
                    scope,
                    cx,
                );
            }
            UiAction::Edit(core, name) => {
                self.confirm_delete = None;
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
        // ShellExecute 会泵 Windows 消息, 执行时不能持有 GPUI 的 App 或实体借用。
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
                                Err("主题设置未保存, 管理线程已关闭。".into()),
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

    fn save_github_proxy(&mut self, cx: &mut Context<Self>) {
        let scope = FeedbackScope::GithubProxy;
        let requested = self.github_proxy.value().to_owned();
        match self.runtime.set_github_proxy(requested.clone()) {
            Ok(saved) => {
                self.set_command_result(scope, Ok(()));
                self.pending_github_proxy = Some(cx.spawn(async move |view, cx| {
                    let result = saved.recv().await;
                    let _ = view.update(cx, |view, cx| {
                        view.pending_github_proxy = None;
                        if result.is_err() {
                            view.set_command_result(
                                scope,
                                Err("GitHub 加速前缀未保存, 管理线程已关闭。".into()),
                            );
                        }
                        view.refresh(cx);
                        if let Some(message) = view.snapshot.settings_error.clone() {
                            view.set_command_result(scope, Err(message));
                        } else if view.github_proxy.value() == requested {
                            view.github_proxy
                                .set_value(view.snapshot.settings.github_proxy.clone());
                        }
                        cx.notify();
                    });
                }));
            }
            Err(message) => self.set_command_result(scope, Err(message)),
        }
        cx.notify();
    }

    fn navigation(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let item = |page, label: &'static str, index| {
            let selected = self.page == page;
            let running = matches!(page, Page::Core(core) if self.snapshot.core(core).running);
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
                .child(div().flex_1().child(label))
                .when(running, |el| {
                    el.child(
                        div()
                            .size(px(8.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(colors.running_dot),
                    )
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    let entering = view.page != page;
                    view.page = page;
                    view.confirm_delete = None;
                    view.refresh(cx);
                    if let Page::Core(core) = page
                        && entering
                        && view.show_versions[core_index(core)]
                    {
                        view.activate(UiAction::ShowVersions(core, true), cx);
                    }
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
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .mb(px(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(18.))
                    .child(img(self.icon.clone()).size(px(24.)).flex_shrink_0())
                    .child("Ladder"),
            )
            .child(item(Page::Core(Core::Mihomo), "mihomo", 1))
            .child(item(Page::Core(Core::SingBox), "sing-box", 2))
            .child(item(Page::Core(Core::Xray), "Xray", 3))
            .child(div().flex_1())
            .child(item(Page::Logs, "日志", 4))
            .child(item(Page::Settings, "设置", 5))
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
        let connection_label = match state.operation {
            Some(CoreOperation::Switching) => "切换中",
            _ if state.running => "停止核心",
            _ => "启动核心",
        };
        let connection_action = if state.running {
            idle.then_some(UiAction::Run(Action::Stop(core)))
        } else {
            can_run.then_some(UiAction::Run(Action::Start(core)))
        };
        let connection_colors = if state.running {
            Colors {
                accent_fill: colors.stop_fill,
                accent_hover: colors.stop_hover,
                ..colors
            }
        } else {
            colors
        };

        let mut files = div()
            .id(("files", core_index(core)))
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col();
        let mut configs: Vec<_> = state.configs.iter().collect();
        configs.sort_by_key(|name| state.current.as_ref() != Some(*name));
        for (index, name) in configs.iter().copied().enumerate() {
            let current = state.current.as_ref() == Some(name);
            let target = DeleteTarget::Config(core, name.clone());
            let confirming = self.confirm_delete.as_ref() == Some(&target);
            let mut buttons = div().flex().items_center().flex_shrink_0().gap(px(8.));
            let validation = state.validation.get(name);
            let checking = matches!(validation, Some(ConfigValidation::Checking));
            if matches!(validation, Some(ConfigValidation::Valid)) {
                buttons = buttons.child(badge("配置合法", colors.version_stable, colors));
            }
            let validate = (
                if checking { "验证中" } else { "验证" },
                (!state.validation_busy && state.installed && state.scan_error.is_none())
                    .then(|| UiAction::Run(Action::ValidateConfig(core, name.clone()))),
            );
            let edit = (2, ("编辑", Some(UiAction::Edit(core, name.clone()))));
            let actions;
            if current {
                buttons = buttons.child(div().child("当前使用"));
                actions = vec![(1, validate), edit];
            } else {
                actions = vec![
                    (
                        0,
                        (
                            "切换",
                            can_use_config
                                .then(|| UiAction::Run(Action::SwitchConfig(core, name.clone()))),
                        ),
                    ),
                    (1, validate),
                    edit,
                    (
                        3,
                        (
                            if confirming { "确认删除" } else { "删除" },
                            can_use_config.then(|| {
                                if confirming {
                                    UiAction::Run(Action::DeleteConfig(core, name.clone()))
                                } else {
                                    UiAction::ConfirmDelete(target.clone())
                                }
                            }),
                        ),
                    ),
                ];
            }
            for (slot, (label, action)) in actions {
                let enabled = action.is_some();
                let tab_index = 10 + (index * 4 + slot) as isize;
                let focus = self.config_focus[core_index(core)][name][slot]
                    .clone()
                    .tab_stop(action.is_some())
                    .tab_index(tab_index);
                buttons = buttons.child(
                    action_button(label, tab_index, action, false, colors, cx)
                        .id(SharedString::from(format!(
                            "config-{}-{name}-{slot}",
                            core.label()
                        )))
                        .track_focus(&focus)
                        .when(enabled, |el| {
                            el.text_color(match slot {
                                1 => colors.text,
                                2 => colors.version_preview,
                                3 => colors.error,
                                _ => colors.accent,
                            })
                        })
                        .when(confirming && slot == 3, |el| {
                            el.on_mouse_down_out(cx.listener(|view, _, _, cx| {
                                view.confirm_delete = None;
                                cx.notify();
                            }))
                        }),
                );
            }
            files = files.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pl(px(12.))
                    .pr(px(28.))
                    .py(px(12.))
                    .when(index > 0, |el| el.border_t_1().border_color(colors.line))
                    .when(current, |el| el.bg(colors.selection))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(div().flex_1().min_w(px(100.)).child(name.clone()))
                            .child(buttons),
                    )
                    .when_some(validation, |el, result| match result {
                        ConfigValidation::Invalid(message) => {
                            el.child(error_message(message.clone(), colors))
                        }
                        _ => el,
                    }),
            );
        }
        let config_list = panel(colors)
            .flex_1()
            .min_h(px(120.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .id("config-scroll")
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        files
                            .min_w(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.config_scroll[core_index(core)]),
                    )
                    .child(
                        scrollbar("scrollbar", &self.config_scroll[core_index(core)], colors)
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0(),
                    ),
            );

        let content = div()
            .id(("core-content", core_index(core)))
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
                    .when(!state.installed, |el| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(TEXT_PRIMARY))
                                .font_weight(FontWeight::BOLD)
                                .text_color(colors.error)
                                .child("点击右侧“管理核心版本”安装核心 -->"),
                        )
                    })
                    .when(state.installed, |el| {
                        el.child(
                            action_button("", 6, connection_action, true, connection_colors, cx)
                                .id("core-power")
                                .gap(px(6.))
                                .bg(connection_colors.accent_fill)
                                .border_color(connection_colors.accent_fill)
                                .text_color(colors.on_accent)
                                .child(power_icon(colors.on_accent))
                                .child(connection_label)
                                .when(!idle, |el| el.opacity(0.65)),
                        )
                    })
                    .child(action_button(
                        "管理核心版本",
                        7,
                        Some(UiAction::ShowVersions(core, true)),
                        true,
                        colors,
                        cx,
                    )),
            )
            .when_some(
                self.command_error(FeedbackScope::Config(core)).cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when(
                state.current.is_some() && !current_exists && state.scan_error.is_none(),
                |el| {
                    el.child(error_message(
                        "当前配置文件已移走, 请重新选择配置。".into(),
                        colors,
                    ))
                },
            )
            .when_some(state.run_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .when_some(state.output_error.clone(), |el, message| {
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(action_button(
                                "验证全部",
                                8,
                                (state.installed
                                    && !state.configs.is_empty()
                                    && state.scan_error.is_none()
                                    && !state.validation_busy)
                                    .then_some(UiAction::Run(Action::ValidateAllConfigs(core))),
                                false,
                                colors,
                                cx,
                            ))
                            .child(action_button(
                                "打开配置目录",
                                9,
                                Some(UiAction::OpenFolder(core)),
                                false,
                                colors,
                                cx,
                            )),
                    ),
            )
            .when_some(state.scan_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(config_list);
        scroll_area(
            ("core-scroll", core_index(core)),
            content,
            &self.core_page_scroll[core_index(core)],
            colors,
        )
        .size_full()
    }

    fn versions_page(&self, core: Core, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        let state = self.snapshot.core(core);
        let can_manage = state.operation.is_none() && state.update.is_none();
        let downloaded_only = self.downloaded_only[core_index(core)];
        let versions: Vec<_> = state
            .versions
            .iter()
            .filter(|version| !downloaded_only || version.installed)
            .collect();
        let mut rows = div()
            .id(("version-list", core_index(core)))
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col();
        for (index, version) in versions.iter().enumerate() {
            let current =
                version.installed && state.selected_version.as_ref() == Some(&version.version);
            let progress = state
                .update
                .as_ref()
                .filter(|_| state.downloading_version.as_ref() == Some(&version.version));
            let confirming = self.confirm_delete.as_ref()
                == Some(&DeleteTarget::Version(core, version.version.clone()));
            let (label, color) = match version.channel {
                UpdateChannel::Stable => ("稳定版", colors.version_stable),
                UpdateChannel::Preview => ("预览版", colors.version_preview),
            };
            let actions = if current {
                vec![]
            } else if version.installed {
                vec![
                    (
                        "切换",
                        can_manage.then_some(UiAction::Run(Action::SwitchVersion(
                            core,
                            version.version.clone(),
                        ))),
                    ),
                    (
                        if confirming { "确认删除" } else { "删除" },
                        can_manage.then(|| {
                            if confirming {
                                UiAction::Run(Action::DeleteVersion(core, version.version.clone()))
                            } else {
                                UiAction::ConfirmDelete(DeleteTarget::Version(
                                    core,
                                    version.version.clone(),
                                ))
                            }
                        }),
                    ),
                ]
            } else {
                vec![(
                    if progress.is_some() {
                        "下载中"
                    } else {
                        "下载"
                    },
                    (can_manage && version.release_tag.is_some()).then_some(UiAction::Run(
                        Action::DownloadVersion(core, version.version.clone()),
                    )),
                )]
            };
            let mut buttons = div().flex().items_center().flex_shrink_0().gap(px(8.));
            if current {
                buttons = buttons.child(div().child("当前使用"));
            }
            for (slot, (label, action)) in actions.into_iter().enumerate() {
                let tab_index = 14 + (index * 2 + slot) as isize;
                let enabled = action.is_some();
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
                        .when(enabled, |el| {
                            el.text_color(if slot == 1 {
                                colors.error
                            } else {
                                colors.accent
                            })
                        })
                        .when(confirming && slot == 1, |el| {
                            el.on_mouse_down_out(cx.listener(|view, _, _, cx| {
                                view.confirm_delete = None;
                                cx.notify();
                            }))
                        }),
                );
            }
            rows = rows.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .whitespace_nowrap()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .pl(px(12.))
                    .pr(px(28.))
                    .py(px(12.))
                    .when(index > 0, |el| el.border_t_1().border_color(colors.line))
                    .when(current, |el| el.bg(colors.selection))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if version.version.starts_with("alpha-") {
                                        version.version.clone()
                                    } else {
                                        format!("v{}", version.version)
                                    }),
                            )
                            .child(badge(label, color, colors).bg(colors.version_badge))
                            .when_some(progress, |el, progress| {
                                el.child(
                                    caption(update_progress_text(progress), colors)
                                        .whitespace_nowrap()
                                        .flex_shrink_0()
                                        .text_color(colors.accent),
                                )
                            }),
                    )
                    .child(buttons),
            );
        }
        let footer_index = 14 + (versions.len() * 2) as isize;
        let loading = state.versions_loading;
        let refresh = action_button(
            "",
            6,
            (!loading).then_some(UiAction::Run(Action::RefreshVersions(core))),
            true,
            colors,
            cx,
        )
        .id("refresh-versions")
        .size(px(24.))
        .min_w(px(24.))
        .px(px(0.))
        .bg(colors.accent_fill)
        .border_color(colors.accent_fill)
        .when(loading, |el| {
            el.child(refresh_icon(colors.on_accent, 0.).with_animation(
                "refresh-spin",
                Animation::new(Duration::from_secs(1)).repeat(),
                move |_, delta| refresh_icon(colors.on_accent, delta * std::f32::consts::TAU),
            ))
        })
        .when(!loading, |el| el.child(refresh_icon(colors.on_accent, 0.)))
        .tooltip(move |_, cx| cx.new(|_| RefreshTooltip { loading, colors }).into());
        let mut filter = div()
            .flex()
            .flex_shrink_0()
            .gap(px(2.))
            .p(px(1.))
            .bg(colors.canvas)
            .border_1()
            .border_color(colors.line)
            .rounded(px(6.));
        for (index, (label, only_downloaded)) in
            [("已下载", true), ("全部", false)].into_iter().enumerate()
        {
            filter = filter.child(
                control(label, 7 + index as isize, colors, false)
                    .h(px(26.))
                    .min_w(px(60.))
                    .px(px(8.))
                    .justify_center()
                    .text_size(px(TEXT_PRIMARY))
                    .when(downloaded_only == only_downloaded, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.activate(UiAction::SetDownloadedOnly(core, only_downloaded), cx);
                    })),
            );
        }
        let content = div()
            .id(("versions-content", core_index(core)))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap(px(12.))
                    .child(page_title(match core {
                        Core::SingBox => "sing-box 版本",
                        Core::Xray => "Xray 版本",
                        Core::Mihomo => "mihomo 版本",
                    }))
                    .child(refresh),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(filter)
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(8.))
                            .child(action_button(
                                "打开核心目录",
                                9,
                                Some(UiAction::OpenCoreFolder(core)),
                                false,
                                colors,
                                cx,
                            ))
                            .child(
                                action_button(
                                    "返回配置页面",
                                    10,
                                    Some(UiAction::ShowVersions(core, false)),
                                    true,
                                    colors,
                                    cx,
                                )
                                // 蓝底上下各内收 1px, 平衡旁边浅色描边按钮的视觉高度。
                                .h(px(28.)),
                            ),
                    ),
            )
            .when_some(self.snapshot.settings_error.clone(), |el, message| {
                el.child(error_message(message, colors))
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
                    .child(
                        div()
                            .id("version-scroll")
                            .relative()
                            .flex()
                            .flex_1()
                            .min_h(px(0.))
                            .child(
                                rows.min_w(px(0.))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.version_scroll[core_index(core)]),
                            )
                            .child(
                                scrollbar(
                                    "scrollbar",
                                    &self.version_scroll[core_index(core)],
                                    colors,
                                )
                                .absolute()
                                .right_0()
                                .top_0()
                                .bottom_0(),
                            ),
                    ),
            )
            .when(!downloaded_only && state.versions_has_more, |el| {
                el.child(action_button(
                    "加载更多",
                    footer_index,
                    (!state.versions_loading)
                        .then_some(UiAction::Run(Action::LoadMoreVersions(core))),
                    false,
                    colors,
                    cx,
                ))
            });
        scroll_area(
            ("versions-scroll", core_index(core)),
            content,
            &self.version_page_scroll[core_index(core)],
            colors,
        )
        .size_full()
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
                control(label, 6 + index as isize, colors, false)
                    .h(px(28.))
                    .px(px(8.))
                    .when(selected, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| view.set_theme(theme, cx))),
            );
        }
        let mut rows = panel(colors)
            .flex_shrink_0()
            .flex()
            .flex_col()
            .p(px(4.))
            .child(
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
                "随 Ladder 启动 mihomo",
                "使用上次选择的 mihomo 配置",
                Setting::AutoMihomo,
                settings.auto_mihomo,
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
                "下次启动任一核心时, 将物理网卡 DNS 设为 127.0.0.1 和 ::1。不要和 mihomo 的 strict-route、sing-box 的 strict_route、Xray 的 autoSystemWfpBlockLeak 同时开启。",
                Setting::HijackDns,
                settings.hijack_dns,
            ),
            (
                "GitHub 加速",
                "用于核心和软件的版本检查与下载",
                Setting::GithubProxyEnabled,
                settings.github_proxy_enabled,
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
                control(label, 9 + index as isize, colors, false)
                    .flex_shrink_0()
                    .min_h(px(68.))
                    .px(px(12.))
                    .py(px(10.))
                    .gap(px(12.))
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .whitespace_normal()
                            .gap(px(4.))
                            .child(label)
                            .child(caption(description, colors).w_full().whitespace_normal()),
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
        rows = rows
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .min_h(px(52.))
                    .px(px(12.))
                    .gap(px(8.))
                    .items_center()
                    .child(div().flex_shrink_0().child("加速前缀"))
                    .child(self.github_proxy.render(colors, cx))
                    .child(action_button("保存", 16, None, false, colors, cx).when(
                        self.pending_github_proxy.is_none(),
                        |el| {
                            el.tab_index(16)
                                .cursor_pointer()
                                .text_color(colors.text)
                                .bg(colors.panel)
                                .hover(|el| el.bg(colors.hover))
                                .focus(|el| el.border_color(colors.accent))
                                .active(|el| el.bg(colors.selection))
                                .on_click(cx.listener(|view, _, _, cx| view.save_github_proxy(cx)))
                        },
                    )),
            )
            .when_some(
                self.command_error(FeedbackScope::GithubProxy).cloned(),
                |el, message| {
                    el.child(
                        div()
                            .mx(px(12.))
                            .mb(px(8.))
                            .child(error_message(message, colors)),
                    )
                },
            );
        let update_status = match &self.app_update {
            AppUpdateState::Idle
            | AppUpdateState::Checking
            | AppUpdateState::Installing
            | AppUpdateState::Failed(_) => None,
            AppUpdateState::Latest => Some("已是最新版本".into()),
            AppUpdateState::Available(version) => Some(format!("新版本 {version}")),
            AppUpdateState::Downloading(progress) => Some(update_progress_text(progress)),
            AppUpdateState::Ready(version) => Some(format!("{version} 已准备好")),
        };
        let (update_label, update_action) = match self.app_update {
            AppUpdateState::Checking => ("检查中…", None),
            AppUpdateState::Downloading(_) => ("下载中…", None),
            AppUpdateState::Installing => ("重启中…", None),
            AppUpdateState::Available(_) => ("下载更新", Some(UiAction::DownloadAppUpdate)),
            AppUpdateState::Ready(_) => ("重启以更新", Some(UiAction::InstallAppUpdate)),
            _ => ("检查更新", Some(UiAction::CheckAppUpdate)),
        };
        let content = div()
            .id("settings-content")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(16.))
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
                            17,
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
            .child(
                panel(colors)
                    .flex_shrink_0()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(page_title("关于"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(img(self.icon.clone()).size(px(36.)).flex_shrink_0())
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .whitespace_normal()
                                    .gap(px(2.))
                                    .items_start()
                                    .child(
                                        div()
                                            .flex()
                                            .items_baseline()
                                            .gap(px(8.))
                                            .child(repository_link(
                                                format!("Ladder {}", env!("CARGO_PKG_VERSION")),
                                                "https://github.com/senzyo/ladder",
                                                18,
                                                colors,
                                            ))
                                            .child(caption("MIT License", colors)),
                                    )
                                    .child(
                                        div().flex().flex_wrap().gap(px(8.)).children(
                                            [
                                                ("mihomo", "https://github.com/MetaCubeX/mihomo"),
                                                (
                                                    "sing-box",
                                                    "https://github.com/SagerNet/sing-box",
                                                ),
                                                ("Xray", "https://github.com/XTLS/Xray-core"),
                                            ]
                                            .into_iter()
                                            .enumerate()
                                            .map(
                                                |(index, (label, url))| {
                                                    repository_link(
                                                        label,
                                                        url,
                                                        19 + index as isize,
                                                        colors,
                                                    )
                                                },
                                            ),
                                        ),
                                    ),
                            )
                            .child(
                                div().flex_1().min_w(px(0.)).text_right().when_some(
                                    update_status,
                                    |el, status| {
                                        el.child(caption(status, colors).whitespace_normal())
                                    },
                                ),
                            )
                            .child(
                                action_button(
                                    update_label,
                                    22,
                                    update_action,
                                    matches!(
                                        self.app_update,
                                        AppUpdateState::Available(_) | AppUpdateState::Ready(_)
                                    ),
                                    colors,
                                    cx,
                                )
                                .id("app-update")
                                .min_w(px(100.)),
                            ),
                    )
                    .when(matches!(self.app_update, AppUpdateState::Ready(_)), |el| {
                        el.child(
                            caption("更新时将停止所有核心, 完成后重新打开 Ladder。", colors)
                                .w_full()
                                .whitespace_normal(),
                        )
                    })
                    .when_some(
                        match &self.app_update {
                            AppUpdateState::Failed(message) => Some(message.clone()),
                            _ => None,
                        },
                        |el, message| el.child(error_message(message, colors)),
                    ),
            );
        scroll_area("settings-scroll", content, &self.settings_scroll, colors).size_full()
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
            .text_size(px(TEXT_PRIMARY))
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
                            if let Some(index) = view
                                .snapshot
                                .core(core)
                                .versions
                                .iter()
                                .filter(|version| {
                                    !view.downloaded_only[core_index(core)] || version.installed
                                })
                                .position(|version| {
                                    handles.get(&version.version).is_some_and(|handles| {
                                        handles.iter().any(|handle| handle.is_focused(window))
                                    })
                                })
                            {
                                view.version_scroll[core_index(core)].scroll_to_item(index);
                                cx.notify();
                            }
                        } else {
                            let state = view.snapshot.core(core);
                            let mut configs: Vec<_> = state.configs.iter().collect();
                            configs.sort_by_key(|name| state.current.as_ref() != Some(*name));
                            let handles = &view.config_focus[core_index(core)];
                            if let Some(index) = configs.iter().position(|name| {
                                handles.get(*name).is_some_and(|handles| {
                                    handles.iter().any(|handle| handle.is_focused(window))
                                })
                            }) {
                                view.config_scroll[core_index(core)].scroll_to_item(index);
                                cx.notify();
                            }
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
        Core::Mihomo => 2,
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

fn power_icon(color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scale = bounds.size.width / px(18.);
            let p = |x, y| bounds.origin + point(px(x) * scale, px(y) * scale);
            let mut path = PathBuilder::stroke(px(1.6) * scale);
            path.move_to(p(9., 2.));
            path.line_to(p(9., 9.));
            path.move_to(p(4.4, 4.4));
            path.cubic_bezier_to(p(2.5, 9.), p(3.1, 5.6), p(2.5, 7.2));
            path.cubic_bezier_to(p(9., 15.5), p(2.5, 12.6), p(5.4, 15.5));
            path.cubic_bezier_to(p(15.5, 9.), p(12.6, 15.5), p(15.5, 12.6));
            path.cubic_bezier_to(p(13.6, 4.4), p(15.5, 7.2), p(14.9, 5.6));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}

fn refresh_icon(color: Rgba, angle: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scale = bounds.size.width / px(18.);
            let (sin, cos) = angle.sin_cos();
            let p = |x: f32, y: f32| {
                let (x, y) = (x - 9., y - 9.);
                bounds.origin
                    + point(
                        px(9. + x * cos - y * sin) * scale,
                        px(9. + x * sin + y * cos) * scale,
                    )
            };
            let mut path = PathBuilder::stroke(px(1.6) * scale);
            path.move_to(p(3., 7.));
            path.cubic_bezier_to(p(14., 5.), p(4., 1.), p(11., 1.));
            path.line_to(p(16., 7.));
            path.move_to(p(16., 2.));
            path.line_to(p(16., 7.));
            path.line_to(p(11., 7.));
            path.move_to(p(15., 11.));
            path.cubic_bezier_to(p(4., 13.), p(14., 17.), p(7., 17.));
            path.line_to(p(2., 11.));
            path.move_to(p(2., 16.));
            path.line_to(p(2., 11.));
            path.line_to(p(7., 11.));
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(16.))
}

struct RefreshTooltip {
    loading: bool,
    colors: Colors,
}

impl Render for RefreshTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        panel(self.colors)
            .px(px(8.))
            .py(px(4.))
            .text_size(px(TEXT_SECONDARY))
            .text_color(self.colors.text)
            .child(if self.loading {
                "正在获取版本列表…"
            } else {
                "获取版本列表"
            })
    }
}

fn page_title(label: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .text_size(px(TEXT_PRIMARY))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn caption(text: impl Into<SharedString>, colors: Colors) -> Div {
    div()
        .text_size(px(TEXT_SECONDARY))
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
        .text_size(px(TEXT_SECONDARY))
        .child(label)
}

fn scroll_area(
    id: impl Into<ElementId>,
    content: Stateful<Div>,
    scroll: &ScrollHandle,
    colors: Colors,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(
            content
                .flex_1()
                .min_w(px(0.))
                .mr(px(16.))
                .overflow_y_scroll()
                .track_scroll(scroll),
        )
        .child(
            scrollbar("scrollbar", scroll, colors)
                .absolute()
                .right_0()
                .top_0()
                .bottom_0(),
        )
}

#[derive(IntoElement)]
struct ErrorMessage {
    message: String,
    colors: Colors,
}

impl RenderOnce for ErrorMessage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("error-{}", self.message));
        let scroll = window.use_keyed_state(id.clone(), cx, |_, _| ScrollHandle::new());
        scroll_area(
            id,
            div()
                .id("error-text")
                .max_h(px(64.))
                .px(px(10.))
                .py(px(8.))
                .child(self.message),
            scroll.read(cx),
            self.colors,
        )
        .flex_shrink_0()
        .max_h(px(64.))
        .border_l_2()
        .border_color(self.colors.error)
        .bg(self.colors.error_bg)
        .text_color(self.colors.error)
        .text_size(px(TEXT_SECONDARY))
    }
}

fn error_message(message: String, colors: Colors) -> ErrorMessage {
    ErrorMessage { message, colors }
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

fn repository_link(
    label: impl Into<SharedString>,
    url: &'static str,
    index: isize,
    colors: Colors,
) -> Stateful<Div> {
    control(url, index, colors, false)
        .text_color(colors.accent)
        .child(label.into())
        .on_click(move |_, _, cx| cx.open_url(url))
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
        .text_size(px(TEXT_PRIMARY))
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
        .when(!label.is_empty(), |el| el.child(label))
        .when_some(action, |el, action| {
            el.on_click(cx.listener(move |view, _, window, cx| {
                if matches!(
                    &action,
                    UiAction::Run(
                        Action::SwitchVersion(..)
                            | Action::DeleteVersion(..)
                            | Action::SwitchConfig(..)
                            | Action::DeleteConfig(..)
                    ) | UiAction::ShowVersions(..)
                ) {
                    window.focus(&view.focus);
                }
                view.activate(action.clone(), cx);
            }))
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
    stop_fill: Rgba,
    stop_hover: Rgba,
    running_dot: Rgba,
    version_stable: Rgba,
    version_preview: Rgba,
    version_badge: Rgba,
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
            accent_fill: tone(0x245fba, 0x2865bd),
            accent_hover: tone(0x1c4e9d, 0x2256a3),
            on_accent: rgb(0xffffff),
            stop_fill: tone(0xb52f3d, 0xb83240),
            stop_hover: tone(0x942633, 0x9d2935),
            running_dot: tone(0x3fb950, 0x3fb950),
            version_stable: tone(0x3fb950, 0x3fb950),
            version_preview: tone(0xd29922, 0xd29922),
            version_badge: tone(0xf6f8fa, 0x1c1e22),
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
