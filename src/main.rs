//! 程序入口与模块装配。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod app_error;
mod app_state;
mod com_guard;
mod commands;
mod config_selection;
mod core;
mod dns;
mod emergency;
mod logging;
mod process;
mod scheduler;
mod settings;
mod single_instance_guard;
mod toast;
mod tray;
mod update;
mod windows_strings;

fn main() {
    app::run();
}
