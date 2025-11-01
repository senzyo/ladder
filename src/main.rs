//! 程序入口与模块装配。
//!
//! 标准库不提供本项目所需的 Win32 托盘、GDI、COM、IP Helper 及原生进程接口,
//! 当前 Windows 绑定仍要求调用方使用 unsafe 并保证句柄和缓冲区有效。
//! 业务流程使用安全 Rust; 原始句柄、指针与缓冲区的安全义务集中在各叶模块的
//! Windows 边界, 由局部 SAFETY 注释和 unsafe 接口契约说明。

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
