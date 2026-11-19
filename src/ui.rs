use crate::app_updater::{AppUpdateState, AppUpdater};
use crate::core::Core;
use crate::core_output::ConfigFailure;
use crate::desktop;
use crate::i18n::{self, Language, ui as t};
use crate::runtime::{
    Action, ConfigLatency, ConfigValidation, CoreOperation, CoreState, LatencyTarget, Runtime,
    Setting, Snapshot, ThemePreference,
};
use crate::update::{CoreVersion, UpdateChannel, UpdateProgress};
use gpui::{
    Animation, AnimationExt, App, Asset, Context, Div, ElementId, FocusHandle, FontWeight,
    ImageCacheError, Img, PathBuilder, RenderImage, Rgba, ScrollHandle, SharedString, Stateful,
    Task, Timer, Window, WindowAppearance, canvas, div, img, point, prelude::*, px, rgb, rgba,
};
use std::{
    collections::HashMap,
    future::poll_fn,
    path::{Path, PathBuf},
    sync::Arc,
    task::Poll,
    time::Duration,
};

#[path = "config_name.rs"]
mod config_name;
use config_name::ConfigName;

#[path = "content_text.rs"]
mod content_text;
use content_text::content_text;

#[path = "log_view.rs"]
mod log_view;
use log_view::LogView;

#[path = "scrollbar.rs"]
mod scrollbar;
use scrollbar::scrollbar;

#[path = "proxy_input.rs"]
mod proxy_input;
use proxy_input::{UrlField, UrlInput};

const TEXT_PRIMARY: f32 = 14.;
const TEXT_SECONDARY: f32 = 12.;

struct LadderIcon;

impl Asset for LadderIcon {
    type Source = u32;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(side: u32, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        poll_fn(move |_| Poll::Ready(render_ladder_icon(side)))
    }
}

fn render_ladder_icon(side: u32) -> Result<Arc<RenderImage>, ImageCacheError> {
    let tree = resvg::usvg::Tree::from_data(
        include_bytes!("../assets/ladder_color.svg"),
        &resvg::usvg::Options::default(),
    )?;
    let scale = side as f32 / tree.size().width();
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(side, side).ok_or(resvg::usvg::Error::InvalidSize)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // GPUI expects straight-alpha BGRA pixels for its color image atlas.
    let mut bgra = image::RgbaImage::new(side, side);
    for (output, pixel) in bgra.pixels_mut().zip(pixmap.pixels()) {
        let color = pixel.demultiply();
        *output = image::Rgba([color.blue(), color.green(), color.red(), color.alpha()]);
    }
    Ok(Arc::new(RenderImage::new([image::Frame::new(bgra)])))
}

fn ladder_icon(size: f32) -> Img {
    img(move |window: &mut Window, cx: &mut App| {
        let side = (size * window.scale_factor()).round() as u32;
        window.use_asset::<LadderIcon>(&side, cx)
    })
    .size(px(size))
    .flex_shrink_0()
}

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
    Url(UrlField),
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
    SaveUrl(UrlField),
}

pub(crate) struct MainWindow {
    runtime: Runtime,
    snapshot: Snapshot,
    page: Page,
    uptime_seconds: Option<u64>,
    config_focus: [HashMap<String, [FocusHandle; 5]>; 3],
    config_scroll: [ScrollHandle; 3],
    core_page_scroll: [ScrollHandle; 3],
    pending_theme: Option<(ThemePreference, Task<()>)>,
    pending_language: Option<Task<()>>,
    pending_startup_core: Option<(Core, Task<()>)>,
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
    url_inputs: [UrlInput; 2],
    timeout_menu: Option<u8>,
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
        let settings = runtime.snapshot().settings;
        let url_inputs = [UrlField::GithubProxy, UrlField::CustomLatency]
            .map(|field| UrlInput::new(field, &settings, window, cx));
        let mut view = Self {
            runtime,
            snapshot: Snapshot::default(),
            page: Page::Core(settings.startup_core),
            uptime_seconds: None,
            config_focus: Default::default(),
            config_scroll: Default::default(),
            core_page_scroll: Default::default(),
            pending_theme: None,
            pending_language: None,
            pending_startup_core: None,
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
            url_inputs,
            timeout_menu: None,
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
        let uptime_seconds = match self.page {
            Page::Core(core) if !self.show_versions[core_index(core)] => next
                .core(core)
                .running_since
                .map(|started| started.elapsed().as_secs()),
            _ => None,
        };
        if self.uptime_seconds != uptime_seconds {
            self.uptime_seconds = uptime_seconds;
            cx.notify();
        }
        let language = next.settings.effective_language();
        if language != i18n::current() {
            i18n::set_current(language);
            let _ = self.events.try_send(crate::TrayAction::LanguageChanged);
            cx.notify();
        }
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
                | Action::TestLatency(core, _)
                | Action::TestAllLatencies(core)
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
            UiAction::Run(
                Action::Set(..) | Action::SetLatencyTarget(_) | Action::SetLatencyTimeout(_),
            ) => FeedbackScope::Settings,
            UiAction::Run(Action::RetryDns) => FeedbackScope::Settings,
            UiAction::SaveUrl(field) => FeedbackScope::Url(*field),
            UiAction::CheckAppUpdate | UiAction::DownloadAppUpdate | UiAction::InstallAppUpdate => {
                FeedbackScope::Settings
            }
        };
        let result = match action {
            UiAction::SaveUrl(field) => return self.save_url(field, cx),
            UiAction::CheckAppUpdate => self
                .updater
                .check(self.snapshot.settings.github_request_prefix().to_owned()),
            UiAction::DownloadAppUpdate => self
                .updater
                .download(self.snapshot.settings.github_request_prefix().to_owned()),
            UiAction::InstallAppUpdate => self
                .events
                .try_send(crate::TrayAction::InstallUpdate)
                .map_err(|_| t::update_thread_closed().into()),
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

    fn selected_startup_core(&self) -> Core {
        self.pending_startup_core
            .as_ref()
            .map_or(self.snapshot.settings.startup_core, |(core, _)| *core)
    }

    fn set_startup_core(&mut self, core: Core, cx: &mut Context<Self>) {
        if self.selected_startup_core() == core {
            return;
        }
        match self.runtime.set_startup_core(core) {
            Ok(saved) => {
                self.set_command_result(FeedbackScope::Settings, Ok(()));
                let task = cx.spawn(async move |view, cx| {
                    let result = saved.recv().await;
                    let _ = view.update(cx, |view, cx| {
                        view.pending_startup_core = None;
                        if result.is_err() {
                            view.set_command_result(
                                FeedbackScope::Settings,
                                Err(t::startup_save_thread_closed().into()),
                            );
                        }
                        view.refresh(cx);
                        cx.notify();
                    });
                });
                self.pending_startup_core = Some((core, task));
            }
            Err(message) => self.set_command_result(FeedbackScope::Settings, Err(message)),
        }
        cx.notify();
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
                                Err(t::theme_save_thread_closed().into()),
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

    fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        if self.pending_language.is_some() || self.snapshot.settings.language == Some(language) {
            return;
        }
        match self.runtime.set_language(language) {
            Ok(saved) => {
                self.set_command_result(FeedbackScope::Settings, Ok(()));
                self.pending_language = Some(cx.spawn(async move |view, cx| {
                    let result = saved.recv().await;
                    let _ = view.update(cx, |view, cx| {
                        view.pending_language = None;
                        if result.is_err() {
                            view.set_command_result(
                                FeedbackScope::Settings,
                                Err(t::language_save_thread_closed().into()),
                            );
                        }
                        view.refresh(cx);
                        cx.notify();
                    });
                }));
            }
            Err(message) => self.set_command_result(FeedbackScope::Settings, Err(message)),
        }
        cx.notify();
    }

    fn save_url(&mut self, field: UrlField, cx: &mut Context<Self>) {
        let scope = FeedbackScope::Url(field);
        let input = &self.url_inputs[field as usize];
        if !input.changed(&self.snapshot.settings) {
            if self.command_error(scope).is_some() {
                self.set_command_result(scope, Ok(()));
                cx.notify();
            }
            return;
        }
        let requested = input.value().to_owned();
        let save = match field {
            UrlField::GithubProxy => self.runtime.set_github_proxy(requested.clone()),
            UrlField::CustomLatency => self.runtime.set_custom_latency_url(requested.clone()),
        };
        match save {
            Ok(saved) => {
                self.set_command_result(scope, Ok(()));
                let pending = requested.clone();
                let task = cx.spawn(async move |view, cx| {
                    let result = saved.recv().await;
                    let _ = view.update(cx, |view, cx| {
                        view.url_inputs[field as usize].pending = None;
                        if result.is_err() {
                            let message = match field {
                                UrlField::GithubProxy => t::proxy_save_thread_closed(),
                                UrlField::CustomLatency => t::latency_url_save_thread_closed(),
                            };
                            view.set_command_result(scope, Err(message.into()));
                        }
                        view.refresh(cx);
                        if let Some(message) = view.snapshot.settings_error.clone() {
                            view.set_command_result(scope, Err(message));
                        } else if result.is_ok()
                            && view.url_inputs[field as usize].value() == requested
                        {
                            view.url_inputs[field as usize]
                                .set_value(field.saved(&view.snapshot.settings).to_owned());
                        }
                        cx.notify();
                    });
                });
                self.url_inputs[field as usize].pending = Some((pending, task));
            }
            Err(message) => self.set_command_result(scope, Err(message)),
        }
        cx.notify();
    }

    fn navigation(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let item = |page, label: &'static str, index| {
            let selected = self.page == page;
            let running = matches!(page, Page::Core(core) if self.snapshot.core(core).running());
            control(("navigation", index as usize), index, colors, false)
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
                    .child(ladder_icon(24.))
                    .child("Ladder"),
            )
            .child(item(Page::Core(Core::Mihomo), "mihomo", 1))
            .child(item(Page::Core(Core::SingBox), "sing-box", 2))
            .child(item(Page::Core(Core::Xray), "Xray", 3))
            .child(div().flex_1())
            .child(item(Page::Logs, t::logs(), 4))
            .child(item(Page::Settings, t::settings(), 5))
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
        let can_start_config = can_use_config && !state.validation_busy;
        let can_validate = state.installed
            && state.scan_error.is_none()
            && !state.validation_busy
            && !matches!(
                state.operation,
                Some(CoreOperation::Starting | CoreOperation::Switching)
            );
        let can_run = can_start_config && state.installed && current_exists;
        let can_test_all = state.installed
            && !state.configs.is_empty()
            && state.scan_error.is_none()
            && !state.latency_busy();
        let connection_label = match state.operation {
            Some(CoreOperation::Switching) => t::switching(),
            _ if state.running() => t::stop_core(),
            _ => t::start_core(),
        };
        let connection_action = if state.running() {
            idle.then_some(UiAction::Run(Action::Stop(core)))
        } else {
            can_run.then_some(UiAction::Run(Action::Start(core)))
        };
        let connection_disabled = connection_action.is_none();
        let connection_colors = if state.running() {
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
        let configs = visible_configs(state);
        for (index, name) in configs.iter().copied().enumerate() {
            let current = state.current.as_ref() == Some(name);
            let target = DeleteTarget::Config(core, name.clone());
            let confirming = self.confirm_delete.as_ref() == Some(&target);
            let validation = state.validation(name);
            let latency = state.latency(name);
            let probe_validation_timed_out = matches!(
                latency,
                Some(ConfigLatency::Failed(ConfigFailure::ValidationTimeout(_)))
            );
            let latency_badge = match latency {
                Some(ConfigLatency::Queued) => {
                    Some(badge(t::latency_queued(), colors.muted, colors))
                }
                Some(ConfigLatency::Testing) => {
                    Some(badge(t::testing_latency(), colors.accent, colors))
                }
                Some(ConfigLatency::Measured(time)) => Some(badge(
                    t::latency_ms(time.as_millis()),
                    colors.accent,
                    colors,
                )),
                Some(ConfigLatency::Failed(_)) => Some(badge(
                    if probe_validation_timed_out {
                        t::validation_timed_out()
                    } else {
                        t::latency_failed()
                    },
                    colors.error,
                    colors,
                )),
                None => None,
            };
            let details = div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(ConfigName(name.clone().into()))
                .children(latency_badge)
                .children(match validation {
                    Some(ConfigValidation::Valid) => {
                        Some(badge(t::config_valid(), colors.version_stable, colors))
                    }
                    Some(ConfigValidation::Invalid(ConfigFailure::ValidationTimeout(_)))
                        if !probe_validation_timed_out =>
                    {
                        Some(badge(t::validation_timed_out(), colors.error, colors))
                    }
                    _ => None,
                });
            let mut buttons = div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(5.))
                .when(current, |el| {
                    el.child(badge(t::in_use(), colors.accent, colors))
                });
            let actions = [
                (
                    t::switch(),
                    can_start_config
                        .then(|| UiAction::Run(Action::SwitchConfig(core, name.clone()))),
                ),
                (
                    t::test_latency(),
                    (state.installed && !state.latency_busy() && state.scan_error.is_none())
                        .then(|| UiAction::Run(Action::TestLatency(core, name.clone()))),
                ),
                (t::edit(), Some(UiAction::Edit(core, name.clone()))),
                (
                    if matches!(validation, Some(ConfigValidation::Checking)) {
                        t::validating()
                    } else {
                        t::validate()
                    },
                    can_validate.then(|| UiAction::Run(Action::ValidateConfig(core, name.clone()))),
                ),
                (
                    if confirming {
                        t::confirm_delete()
                    } else {
                        t::delete()
                    },
                    can_use_config.then(|| {
                        if confirming {
                            UiAction::Run(Action::DeleteConfig(core, name.clone()))
                        } else {
                            UiAction::ConfirmDelete(target)
                        }
                    }),
                ),
            ];
            for (slot, (label, action)) in actions.into_iter().enumerate() {
                if current && matches!(slot, 0 | 4) {
                    continue;
                }
                let enabled = action.is_some();
                let tab_index = 9 + (index * 5 + slot) as isize;
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
                        .min_w(px(50.))
                        .px(px(8.))
                        .when(enabled, |el| {
                            el.text_color(match slot {
                                1 | 3 => colors.text,
                                2 => colors.version_preview,
                                4 => colors.error,
                                _ => colors.accent,
                            })
                        })
                        .when(confirming && slot == 4, |el| {
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
                            .items_center()
                            .gap(px(12.))
                            .child(details)
                            .child(buttons),
                    )
                    .children(
                        config_failure_messages(validation, latency)
                            .into_iter()
                            .flatten()
                            .map(|message| error_message(message.to_owned(), colors)),
                    ),
            );
        }
        let config_list = div()
            .id("config-scroll")
            .relative()
            // 按可用空间扩展列表，避免文件数量撑高卡片、挤走底部按钮。
            .flex_grow()
            .h(px(120.))
            .flex()
            .border_t_1()
            .border_color(colors.line)
            .rounded_b(px(8.))
            .overflow_hidden()
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
            );

        let footer_index = 9 + (configs.len() * 5) as isize;
        let core_controls = panel(colors)
            .flex_shrink_0()
            .flex()
            .flex_col()
            .px(px(16.))
            .py(px(12.))
            .gap(px(8.))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
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
                                .child(t::install_core_hint()),
                        )
                    })
                    .when(state.installed, |el| {
                        let power_button = action_button(
                            "",
                            footer_index,
                            connection_action,
                            true,
                            connection_colors,
                            cx,
                        )
                        .id("core-power")
                        .gap(px(6.))
                        .bg(connection_colors.accent_fill)
                        .border_color(connection_colors.accent_fill)
                        .text_color(colors.on_accent)
                        .child(power_icon(colors.on_accent))
                        .child(connection_label)
                        .when(connection_disabled, |el| el.opacity(0.65));
                        el.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(power_button)
                                .when_some(self.uptime_seconds, |el, seconds| {
                                    el.child(caption(format_uptime(seconds), colors))
                                }),
                        )
                    })
                    .child(
                        action_button(
                            t::manage_versions(),
                            footer_index + 1,
                            Some(UiAction::ShowVersions(core, true)),
                            !state.installed
                                && !state.versions.iter().any(|version| version.installed),
                            colors,
                            cx,
                        )
                        .id("core-versions"),
                    ),
            )
            .when_some(state.run_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .when_some(state.output_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            });
        let config_panel = panel(colors)
            .flex_1()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .px(px(16.))
                    .py(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t::config_files()),
                            )
                            .child(caption(t::config_count(state.configs.len()), colors)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                action_button(
                                    state.latency_progress().map_or_else(
                                        || t::test_all_latencies().to_owned(),
                                        |(current, total)| t::latency_progress(current, total),
                                    ),
                                    6,
                                    can_test_all
                                        .then_some(UiAction::Run(Action::TestAllLatencies(core))),
                                    false,
                                    colors,
                                    cx,
                                )
                                .tab_index(6)
                                .tab_stop(can_test_all),
                            )
                            .child(action_button(
                                t::validate_all(),
                                7,
                                (can_validate && !state.configs.is_empty())
                                    .then_some(UiAction::Run(Action::ValidateAllConfigs(core))),
                                false,
                                colors,
                                cx,
                            ))
                            .child(action_button(
                                t::open_config_folder(),
                                8,
                                Some(UiAction::OpenFolder(core)),
                                false,
                                colors,
                                cx,
                            )),
                    ),
            )
            .when_some(
                self.command_error(FeedbackScope::Config(core)).cloned(),
                |el, message| el.child(error_message(message, colors)),
            )
            .when(
                state.current.is_some() && !current_exists && state.scan_error.is_none(),
                |el| el.child(error_message(t::config_moved().into(), colors)),
            )
            .when_some(state.scan_error.clone(), |el, message| {
                el.child(error_message(message, colors))
            })
            .child(config_list);
        let content = div()
            .id(("core-content", core_index(core)))
            .size_full()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(config_panel)
            .child(core_controls);
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
        let versions = visible_versions(state, downloaded_only);
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
                UpdateChannel::Stable => (t::stable(), colors.version_stable),
                UpdateChannel::Preview => (t::preview(), colors.version_preview),
            };
            let actions = if current {
                vec![]
            } else if version.installed {
                vec![
                    (
                        t::switch(),
                        can_manage.then_some(UiAction::Run(Action::SwitchVersion(
                            core,
                            version.version.clone(),
                        ))),
                    ),
                    (
                        if confirming {
                            t::confirm_delete()
                        } else {
                            t::delete()
                        },
                        (can_manage && !state.testing_version(&version.version)).then(|| {
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
                        t::downloading()
                    } else {
                        t::download()
                    },
                    (can_manage && version.release_tag.is_some()).then_some(UiAction::Run(
                        Action::DownloadVersion(core, version.version.clone()),
                    )),
                )]
            };
            let mut buttons = div()
                .flex()
                .flex_wrap()
                .items_center()
                .max_w_full()
                .gap(px(8.));
            if current {
                buttons = buttons.child(badge(t::in_use(), colors.accent, colors));
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
                    .flex_wrap()
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
                            .flex_wrap()
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
                                        .whitespace_normal()
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
        .when(!loading, |el| el.child(refresh_icon(colors.on_accent, 0.)));
        let mut filter = div()
            .flex()
            .flex_shrink_0()
            .gap(px(2.))
            .p(px(1.))
            .bg(colors.canvas)
            .border_1()
            .border_color(colors.line)
            .rounded(px(6.));
        for (index, (label, only_downloaded)) in [(t::downloaded(), true), (t::all(), false)]
            .into_iter()
            .enumerate()
        {
            filter = filter.child(
                control(("version-filter", index), 7 + index as isize, colors, false)
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
                        Core::SingBox => t::sing_box_versions(),
                        Core::Xray => t::xray_versions(),
                        Core::Mihomo => t::mihomo_versions(),
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
                                t::open_core_folder(),
                                9,
                                Some(UiAction::OpenCoreFolder(core)),
                                false,
                                colors,
                                cx,
                            ))
                            .child(
                                action_button(
                                    t::back_to_configs(),
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
                    t::load_more(),
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

    fn url_setting(&self, field: UrlField, colors: Colors, cx: &Context<Self>) -> Div {
        let input = &self.url_inputs[field as usize];
        let changed = input.changed(&self.snapshot.settings);
        let save_index = field.tab_index() + 1;
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(12.))
            .child(
                div()
                    .flex()
                    .min_h(px(44.))
                    .gap(px(8.))
                    .items_center()
                    .when(field == UrlField::GithubProxy, |el| {
                        el.child(div().flex_shrink_0().child(t::proxy_prefix()))
                    })
                    .child(input.render(colors, cx))
                    .child(
                        action_button(
                            t::save(),
                            save_index,
                            changed.then_some(UiAction::SaveUrl(field)),
                            true,
                            colors,
                            cx,
                        )
                        .tab_index(save_index)
                        .tab_stop(changed),
                    ),
            )
            .when_some(
                self.command_error(FeedbackScope::Url(field)).cloned(),
                |el, message| el.child(div().mb(px(8.)).child(error_message(message, colors))),
            )
    }

    fn timeout_dropdown(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let seconds = self.snapshot.settings.latency_timeout_secs;
        let open = self.timeout_menu.is_some();
        let button = control("latency-timeout", 25, colors, false)
            .w(px(72.))
            .h(px(30.))
            .px(px(8.))
            .justify_between()
            .border_color(colors.line)
            .bg(colors.canvas)
            .child(t::seconds(seconds))
            .child(caption("▾", colors))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |view, _, _, cx| {
                    view.timeout_menu = (!open).then_some(seconds);
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |view, event: &gpui::KeyDownEvent, _, cx| {
                let current = view.timeout_menu.unwrap_or(seconds);
                match event.keystroke.key.as_str() {
                    "up" => view.timeout_menu = Some(current.saturating_sub(1).max(3)),
                    "down" => view.timeout_menu = Some((current + 1).min(8)),
                    "enter" | "space" => {
                        if let Some(choice) = view.timeout_menu.take() {
                            view.activate(UiAction::Run(Action::SetLatencyTimeout(choice)), cx);
                        } else {
                            view.timeout_menu = Some(seconds);
                        }
                    }
                    "tab" => {
                        view.timeout_menu = None;
                        cx.notify();
                        return;
                    }
                    "escape" => view.timeout_menu = None,
                    _ => return,
                }
                cx.notify();
                cx.stop_propagation();
            }));
        div().relative().flex_shrink_0().child(button).when_some(
            self.timeout_menu,
            |el, highlighted| {
                let menu = panel(colors)
                    .id("latency-timeout-menu")
                    .w(px(72.))
                    .p(px(2.))
                    .flex()
                    .flex_col()
                    .shadow_md()
                    .occlude()
                    .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                        view.timeout_menu = None;
                        cx.notify();
                    }))
                    .children((3..=8u8).map(|choice| {
                        control(("timeout-seconds", choice as usize), 25, colors, false)
                            .tab_stop(false)
                            .h(px(26.))
                            .px(px(6.))
                            .when(highlighted == choice, |el| {
                                el.bg(colors.selection).text_color(colors.accent)
                            })
                            .child(t::seconds(choice))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.timeout_menu = None;
                                view.activate(UiAction::Run(Action::SetLatencyTimeout(choice)), cx);
                            }))
                    }));
                el.child(gpui::deferred(
                    gpui::anchored().offset(point(px(0.), px(2.))).child(menu),
                ))
            },
        )
    }

    fn latency_settings(&self, colors: Colors, cx: &Context<Self>) -> Div {
        let target = self.snapshot.settings.latency_target;
        let choices = segmented_choices(colors).children(
            [
                (LatencyTarget::Google, "Google"),
                (LatencyTarget::Cloudflare, "Cloudflare"),
                (LatencyTarget::Custom, t::custom_url()),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (option, label))| {
                control(
                    ("latency-target", index),
                    22 + index as isize,
                    colors,
                    false,
                )
                .h(px(28.))
                .px(px(8.))
                .whitespace_nowrap()
                .when(target == option, |el| {
                    el.bg(colors.selection).text_color(colors.accent)
                })
                .child(label)
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.activate(UiAction::Run(Action::SetLatencyTarget(option)), cx);
                }))
            }),
        );
        let controls = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .px(px(12.))
            .min_h(px(44.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(t::latency_url())
                    .child(choices),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(t::latency_timeout())
                    .child(self.timeout_dropdown(colors, cx)),
            );
        let group = div().flex().flex_col().py(px(4.)).child(controls);
        match target.preset_url() {
            Some(url) => group.child(caption(url, colors).mx(px(12.)).mb(px(4.))),
            None => group.child(self.url_setting(UrlField::CustomLatency, colors, cx)),
        }
    }

    fn settings_page(&self, colors: Colors, cx: &Context<Self>) -> Stateful<Div> {
        let settings = &self.snapshot.settings;
        let mut choices = segmented_choices(colors);
        for (index, (theme, label)) in [
            (ThemePreference::System, t::system_theme()),
            (ThemePreference::Light, t::light_theme()),
            (ThemePreference::Dark, t::dark_theme()),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = self.selected_theme() == theme;
            choices = choices.child(
                control(("theme", index), 6 + index as isize, colors, false)
                    .h(px(28.))
                    .px(px(8.))
                    .when(selected, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| view.set_theme(theme, cx))),
            );
        }
        let mut languages = segmented_choices(colors);
        for (index, language) in Language::ALL.into_iter().enumerate() {
            languages = languages.child(
                control(("language", index), 9 + index as isize, colors, false)
                    .h(px(28.))
                    .px(px(8.))
                    .whitespace_nowrap()
                    .when(settings.effective_language() == language, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .when(self.pending_language.is_some(), |el| el.opacity(0.65))
                    .child(language.native_name())
                    .on_click(cx.listener(move |view, _, _, cx| view.set_language(language, cx))),
            );
        }
        let mut startup_choices = segmented_choices(colors);
        for (index, (core, label)) in [
            (Core::Mihomo, "mihomo"),
            (Core::SingBox, "sing-box"),
            (Core::Xray, "Xray"),
        ]
        .into_iter()
        .enumerate()
        {
            startup_choices = startup_choices.child(
                control(("startup-core", index), 12 + index as isize, colors, false)
                    .h(px(28.))
                    .px(px(8.))
                    .when(self.selected_startup_core() == core, |el| {
                        el.bg(colors.selection).text_color(colors.accent)
                    })
                    .child(label)
                    .on_click(cx.listener(move |view, _, _, cx| view.set_startup_core(core, cx))),
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
                    .child(t::theme())
                    .child(choices),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .min_h(px(52.))
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(12.))
                    .items_center()
                    .justify_between()
                    .child(t::language())
                    .child(languages),
            )
            .child(
                div()
                    .flex()
                    .min_h(px(52.))
                    .px(px(12.))
                    .gap(px(12.))
                    .items_center()
                    .justify_between()
                    .child(t::startup_page())
                    .child(startup_choices),
            );
        let toggle = |index: isize,
                      label: &'static str,
                      description: &'static str,
                      key: Setting,
                      value: bool| {
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
            control(("setting", index as usize), index, colors, false)
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
                        .child(caption(
                            if value { t::enabled() } else { t::disabled() },
                            colors,
                        ))
                        .child(track),
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.activate(UiAction::Run(Action::Set(key, !value)), cx);
                }))
        };
        for (index, (label, description, key, value)) in [
            (
                t::hide_on_startup(),
                t::open_from_tray(),
                Setting::MinimizeToTray,
                settings.minimize_to_tray,
            ),
            (
                t::auto_mihomo(),
                t::last_mihomo_config(),
                Setting::AutoMihomo,
                settings.auto_mihomo,
            ),
            (
                t::auto_sing_box(),
                t::last_sing_box_config(),
                Setting::AutoSingBox,
                settings.auto_sing_box,
            ),
            (
                t::auto_xray(),
                t::last_xray_config(),
                Setting::AutoXray,
                settings.auto_xray,
            ),
            (
                t::github_proxy(),
                t::github_proxy_description(),
                Setting::GithubProxyEnabled,
                settings.github_proxy_enabled,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            rows = rows.child(toggle(15 + index as isize, label, description, key, value));
        }
        rows = rows.child(self.url_setting(UrlField::GithubProxy, colors, cx));
        let network_settings = panel(colors)
            .flex_shrink_0()
            .flex()
            .flex_col()
            .p(px(4.))
            .child(self.latency_settings(colors, cx))
            .child(toggle(
                28,
                t::hijack_dns(),
                t::hijack_dns_description(),
                Setting::HijackDns,
                settings.hijack_dns,
            ));
        let update_status = match &self.app_update {
            AppUpdateState::Idle
            | AppUpdateState::Checking
            | AppUpdateState::Installing
            | AppUpdateState::Failed(_) => None,
            AppUpdateState::Latest => Some(t::latest_version().into()),
            AppUpdateState::Available(version) => Some(t::new_version(version)),
            AppUpdateState::Downloading(progress) => Some(update_progress_text(progress)),
            AppUpdateState::Ready(version) => Some(t::version_ready(version)),
        };
        let (update_label, update_action) = match self.app_update {
            AppUpdateState::Checking => (t::checking_update(), None),
            AppUpdateState::Downloading(_) => (t::downloading_update(), None),
            AppUpdateState::Installing => (t::restarting(), None),
            AppUpdateState::Available(_) => {
                (t::download_update(), Some(UiAction::DownloadAppUpdate))
            }
            AppUpdateState::Ready(_) => (t::restart_to_update(), Some(UiAction::InstallAppUpdate)),
            _ => (t::check_update(), Some(UiAction::CheckAppUpdate)),
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
                                t::restoring_dns()
                            } else {
                                t::retry_dns()
                            },
                            29,
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
            .child(network_settings)
            .child(
                panel(colors)
                    .flex_shrink_0()
                    .p(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(10.))
                            .child(ladder_icon(36.))
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
                                            .items_center()
                                            .gap(px(8.))
                                            .child(repository_link(
                                                format!("Ladder {}", env!("CARGO_PKG_VERSION")),
                                                "https://github.com/senzyo/ladder",
                                                30,
                                                colors,
                                            ))
                                            .child(caption(t::mit_license(), colors)),
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
                                                        31 + index as isize,
                                                        colors,
                                                    )
                                                },
                                            ),
                                        ),
                                    ),
                            )
                            .child(
                                div().flex_1().min_w(px(140.)).text_right().when_some(
                                    update_status,
                                    |el, status| {
                                        el.child(caption(status, colors).whitespace_normal())
                                    },
                                ),
                            )
                            .child(
                                action_button(
                                    update_label,
                                    34,
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
                            caption(t::update_restart_hint(), colors)
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
            .font_family(if i18n::current() == Language::ZhHans {
                "Microsoft YaHei UI"
            } else {
                "Segoe UI"
            })
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
                            if let Some(index) = visible_versions(
                                view.snapshot.core(core),
                                view.downloaded_only[core_index(core)],
                            )
                            .iter()
                            .position(|version| {
                                handles.get(&version.version).is_some_and(|handles| {
                                    handles.iter().any(|handle| handle.is_focused(window))
                                })
                            }) {
                                view.version_scroll[core_index(core)].scroll_to_item(index);
                                cx.notify();
                            }
                        } else {
                            let state = view.snapshot.core(core);
                            let configs = visible_configs(state);
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

fn visible_configs(state: &CoreState) -> Vec<&String> {
    let mut configs: Vec<_> = state.configs.iter().collect();
    configs.sort_by_key(|name| state.current.as_ref() != Some(*name));
    configs
}

fn visible_versions(state: &CoreState, downloaded_only: bool) -> Vec<&CoreVersion> {
    state
        .versions
        .iter()
        .filter(|version| !downloaded_only || version.installed)
        .collect()
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
        UpdateProgress::Checking => t::fetching_release().into(),
        UpdateProgress::Downloading { received, total } => {
            let downloaded = *received as f64 / 1_048_576.0;
            if *total == 0 {
                t::download_size(downloaded)
            } else {
                let percent = (*received as f64 / *total as f64 * 100.0).min(100.0);
                let size = *total as f64 / 1_048_576.0;
                t::download_progress(percent, downloaded, size)
            }
        }
        UpdateProgress::Verifying => t::verifying_download().into(),
        UpdateProgress::Extracting => t::preparing_install().into(),
        UpdateProgress::Installing => t::saving_core().into(),
    }
}

fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = seconds / 60 % 60;
    let seconds = seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
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
        .child(content_text(text))
}

fn panel(colors: Colors) -> Div {
    div()
        .bg(colors.panel)
        .border_1()
        .border_color(colors.line)
        .rounded(px(8.))
}

fn badge(label: impl Into<SharedString>, color: Rgba, colors: Colors) -> Div {
    div()
        .flex_shrink_0()
        .px(px(7.))
        .py(px(2.))
        .rounded(px(4.))
        .bg(colors.canvas)
        .text_color(color)
        .text_size(px(TEXT_SECONDARY))
        .child(label.into())
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
                .child(content_text(self.message)),
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

fn config_failure_messages<'a>(
    validation: Option<&'a ConfigValidation>,
    latency: Option<&'a ConfigLatency>,
) -> [Option<&'a str>; 2] {
    let validation = match validation {
        Some(ConfigValidation::Invalid(failure)) => Some(failure),
        _ => None,
    };
    let latency = match latency {
        Some(ConfigLatency::Failed(failure)) => Some(failure),
        _ => None,
    };
    for failure in [validation, latency].into_iter().flatten() {
        if let Some(message) = failure.core_output() {
            return [Some(message), None];
        }
    }
    let validation = validation.map(ConfigFailure::as_str);
    let latency = latency.map(ConfigFailure::as_str);
    [
        validation,
        latency.filter(|message| Some(*message) != validation),
    ]
}

fn error_message(message: String, colors: Colors) -> ErrorMessage {
    ErrorMessage { message, colors }
}

fn segmented_choices(colors: Colors) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .gap(px(2.))
        .p(px(2.))
        .bg(colors.canvas)
        .border_1()
        .border_color(colors.line)
        .rounded(px(6.))
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
    label: impl Into<SharedString>,
    index: isize,
    action: Option<UiAction>,
    primary: bool,
    colors: Colors,
    cx: &Context<MainWindow>,
) -> Stateful<Div> {
    let label = label.into();
    let enabled = action.is_some();
    let base = if enabled {
        control(("action", index as usize), index, colors, primary)
    } else {
        div()
            .id(("action", index as usize))
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
    fn uptime_shows_elapsed_hours_minutes_and_seconds() {
        assert_eq!(format_uptime(0), "00:00:00");
        assert_eq!(format_uptime(60), "00:01:00");
        assert_eq!(format_uptime(90_061), "25:01:01");
    }

    #[test]
    fn ladder_icon_preserves_color_and_antialiasing_at_display_sizes() {
        for side in [24, 30, 36, 54] {
            let icon = render_ladder_icon(side).unwrap();
            assert_eq!(icon.size(0), gpui::size(side.into(), side.into()));
            let (pixels, _) = icon.as_bytes(0).unwrap().as_chunks::<4>();
            assert_eq!(pixels[0][3], 0);
            assert!(
                pixels
                    .iter()
                    .any(|[_, _, _, alpha]| (1..255).contains(alpha))
            );
            assert!(
                pixels
                    .iter()
                    .any(|[blue, green, red, alpha]| *alpha == 255 && red > green && green > blue)
            );
        }
    }

    #[test]
    fn config_row_prefers_core_output_over_generic_failures() {
        let validation = ConfigValidation::Invalid(ConfigFailure::Core("invalid outbound".into()));
        let probe = ConfigLatency::Failed(ConfigFailure::Core("dial failed".into()));
        let timeout = ConfigFailure::ValidationTimeout("INFO core started".into());
        let silent_validation = ConfigValidation::Invalid("check failed".into());
        let silent_probe = ConfigLatency::Failed("timed out".into());
        for (validation, probe, expected) in [
            (
                &ConfigValidation::Invalid(timeout.clone()),
                &ConfigLatency::Failed(timeout),
                [Some("INFO core started"), None],
            ),
            (&validation, &silent_probe, [Some("invalid outbound"), None]),
            (&silent_validation, &probe, [Some("dial failed"), None]),
            (&validation, &probe, [Some("invalid outbound"), None]),
            (
                &silent_validation,
                &silent_probe,
                [Some("check failed"), Some("timed out")],
            ),
            (
                &silent_validation,
                &ConfigLatency::Failed("check failed".into()),
                [Some("check failed"), None],
            ),
        ] {
            assert_eq!(
                config_failure_messages(Some(validation), Some(probe)),
                expected
            );
        }
    }

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
